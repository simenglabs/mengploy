//! Mesin verifikasi 3 langkah untuk server baru: koneksi SSH (dengan TOFU
//! host key) → Docker → registry (opsional). Urutan dan isi tiap langkah
//! mengikuti `docs/plan.md` "Verifikasi tiga langkah" dan
//! `docs/design/tambah-server.md` §4.2.
//!
//! Progres langkah 1-2 dipancarkan lewat `crate::events::EventRegistry`
//! (SSE), hasil akhir dipersist lewat `servers::repo`. Langkah 3 (registry)
//! TIDAK memakai SSE — form-nya sinkron per `docs/design/tambah-server.md`
//! §4.3 poin 2 ("Dikelola secara sinkron oleh browser").
//!
//! **Desain TOFU dan pause**: kalau fingerprint belum tersimpan, alur ini
//! BERHENTI setelah menampilkan fingerprint — TIDAK menahan sesi SSH tetap
//! terbuka menunggu konfirmasi pengguna yang bisa datang kapan saja (itu
//! akan jadi task berumur tak tentu, kelas masalah lifetime yang sama yang
//! PRD tandai krusial untuk streaming log Fase 3). Konfirmasi
//! (`POST /servers/{id}/hostkey/konfirmasi`, sub-blok 3f) memanggil
//! [`konfirmasi_hostkey_dan_lanjutkan`], yang mengambil ulang fingerprint
//! (`ssh::fetch_fingerprint_via_keyscan` — dibuat untuk kebutuhan ini
//! persis, lihat dokumentasinya) lalu membangun ulang koneksi dari awal
//! dalam mode `Strict`. Biayanya satu handshake SSH tambahan, ditukar
//! dengan tidak ada resource yang tergantung lintas request HTTP.

use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;

use super::repo;
use crate::docker;
use crate::events::VerificationEvent;
use crate::registries;
use crate::servers::model::{LangkahStatus, LangkahVerifikasi};
use crate::ssh::{self, HostKeyMode, SshSession};
use crate::state::AppState;

/// Interval polling normal untuk server yang berhasil online — satu
/// sumber kebenaran, dipakai ulang worker sub-blok 3e.
pub const NORMAL_POLL_INTERVAL_SECS: i64 = 60;

/// "Perintah remote pendek (`uname`, `docker version`, cek grup): 15 detik"
/// — tabel timeout `docs/plan.md`.
const DOCKER_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// Sama seperti timeout koneksi awal — reprobe fingerprint saat konfirmasi
/// memakai batas yang sama (`docs/plan.md`: "Bangun koneksi SSH: 10 detik").
const HOSTKEY_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

// pub(crate) — dipakai ulang routes/servers.rs untuk membangun snapshot
// checklist awal dari `ServerRingkas.status` (satu sumber kebenaran nama
// langkah, bukan diduplikasi sebagai literal string di dua tempat).
pub(crate) const NAMA_KONEKSI: &str = "Membangun Koneksi SSH";
pub(crate) const NAMA_DOCKER: &str = "Pemeriksaan Lingkungan Docker";
pub(crate) const NAMA_REGISTRY: &str = "Pemeriksaan Akses Registry";

fn langkah(nama: &str, status: LangkahStatus, pesan: Option<String>) -> LangkahVerifikasi {
    LangkahVerifikasi {
        nama: nama.to_string(),
        status,
        pesan,
    }
}

/// Kategori kegagalan verifikasi — dipetakan ke `servers.last_error_kind`
/// (`kind()`) dan pesan Bahasa Indonesia final `docs/design/tambah-server.md`
/// §4.2 poin 4 (`pesan()`). Variannya persis 5 kategori wajib PRD (A-E) plus
/// satu tampungan (`Lain`) untuk kegagalan tak terduga.
#[derive(Debug)]
pub(crate) enum LangkahKegagalan {
    /// Kategori A.
    Unreachable,
    /// Kategori B.
    AuthRejected,
    /// Kategori C.
    DockerTidakAda,
    /// Kategori D.
    AksesDockerDitolak,
    /// Kategori E.
    HostKeyBerubah {
        #[allow(dead_code)] // ditampilkan lewat pesan(), field disimpan untuk kejelasan struktur
        lama: String,
        #[allow(dead_code)]
        baru: String,
    },
    Lain(String),
}

impl LangkahKegagalan {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Unreachable => "host_unreachable",
            Self::AuthRejected => "auth_rejected",
            Self::DockerTidakAda => "docker_tidak_ada",
            Self::AksesDockerDitolak => "akses_docker_ditolak",
            Self::HostKeyBerubah { .. } => "host_key_berubah",
            Self::Lain(_) => "lain",
        }
    }

    pub(crate) fn pesan(&self) -> String {
        match self {
            Self::Unreachable => "Gagal terhubung ke host target dalam batas waktu 10 detik. \
                Langkah perbaikan: Periksa apakah IP/Host sudah benar, port SSH terbuka, dan \
                firewall memperbolehkan koneksi masuk."
                .to_string(),
            Self::AuthRejected => "Kunci privat ditolak oleh server target. Langkah perbaikan: \
                Pastikan public key yang sesuai telah didaftarkan pada file \
                '~/.ssh/authorized_keys' pengguna SSH di server target."
                .to_string(),
            Self::DockerTidakAda => "Binary Docker tidak ditemukan di server target. Langkah \
                perbaikan: Masuk ke server Anda via terminal luar dan jalankan instalasi Docker \
                Engine terlebih dahulu."
                .to_string(),
            Self::AksesDockerDitolak => "Pengguna SSH tidak memiliki izin untuk mengakses Unix \
                socket Docker. Langkah perbaikan: Tambahkan pengguna SSH tersebut ke dalam grup \
                OS 'docker' di server target dengan perintah 'usermod -aG docker <user>', lalu \
                verifikasi ulang."
                .to_string(),
            Self::HostKeyBerubah { .. } => "PERINGATAN KEAMANAN: Sidik jari host key yang \
                ditawarkan server berbeda dengan yang telah disimpan sebelumnya! Langkah \
                perbaikan: Jika Anda sengaja mengganti/menginstal ulang server target, Anda \
                harus mendaftarkannya kembali sebagai server baru dengan nama berbeda. Aplikasi \
                menolak menimpa sidik jari tersimpan demi mencegah serangan Man-in-the-Middle."
                .to_string(),
            Self::Lain(detail) => format!(
                "Verifikasi gagal karena kesalahan tak terduga: {detail}. Langkah perbaikan: \
                 Coba lagi; kalau berulang, periksa log aplikasi di server control plane."
            ),
        }
    }
}

pub(crate) fn classify_connect_error(err: ssh::SshConnectError) -> LangkahKegagalan {
    match err {
        ssh::SshConnectError::Unreachable => LangkahKegagalan::Unreachable,
        ssh::SshConnectError::AuthRejected => LangkahKegagalan::AuthRejected,
        ssh::SshConnectError::HostKeyMismatch { expected, offered } => {
            LangkahKegagalan::HostKeyBerubah {
                lama: expected,
                baru: offered,
            }
        }
        ssh::SshConnectError::Other(msg) => LangkahKegagalan::Lain(msg),
    }
}

pub(crate) fn classify_exec_error(err: ssh::SshExecError) -> LangkahKegagalan {
    match err {
        ssh::SshExecError::Timeout | ssh::SshExecError::Disconnected => {
            LangkahKegagalan::Unreachable
        }
        ssh::SshExecError::Other(msg) => LangkahKegagalan::Lain(msg),
    }
}

/// Klasifikasi murni hasil `docker version --format '{{.Server.Version}}'`
/// — tidak menyentuh jaringan, dites langsung dengan `ExecResult` buatan.
pub(crate) fn classify_docker_exec(result: &ssh::ExecResult) -> Result<String, LangkahKegagalan> {
    if result.success() {
        return Ok(result.stdout.trim().to_string());
    }
    if result.code == 127 {
        return Err(LangkahKegagalan::DockerTidakAda);
    }
    if result.stderr.to_lowercase().contains("permission denied") {
        return Err(LangkahKegagalan::AksesDockerDitolak);
    }
    Err(LangkahKegagalan::Lain(format!(
        "perintah `docker version` di server target keluar dengan kode {}",
        result.code
    )))
}

/// Mulai verifikasi server baru. Dipanggil route `GET /servers/{id}/verifikasi`
/// (sub-blok 3f) sebagai `tokio::spawn`, biasanya sesaat setelah
/// `POST /servers` sukses. Modul ini sendiri TIDAK melakukan locking
/// terhadap pemicu ganda — Fase 1 satu worker in-process, `docs/plan.md`
/// risiko baris 12 mencatat lock db belum dibutuhkan di sini; penolakan
/// pemicu ganda (`docs/design/tambah-server.md` §4.2 poin 5) adalah
/// tanggung jawab route (mis. cek `status = verifying` sebelum spawn).
pub async fn mulai_verifikasi(state: AppState, server_id: String) {
    let Some(row) = fetch_row(&state.db_read, &server_id).await else {
        return;
    };

    if let Err(err) = repo::set_status_verifying(&state.db_write, &server_id).await {
        tracing::warn!(error = %err, server_id, "gagal set status verifying");
        return;
    }

    let plaintext_key = match state.crypto.decrypt(&row.ssh_key_encrypted) {
        Ok(key) => key,
        Err(err) => {
            tracing::warn!(error = %err, server_id, "gagal dekripsi kunci ssh server");
            selesai_gagal(
                &state,
                &server_id,
                false,
                &LangkahKegagalan::Lain("kunci SSH tersimpan tidak bisa didekripsi".to_string()),
            )
            .await;
            return;
        }
    };

    let mode = match &row.host_key_fingerprint {
        Some(expected) => HostKeyMode::Strict {
            expected_fingerprint: expected.clone(),
        },
        None => HostKeyMode::Tofu,
    };

    state.events.publish(
        &server_id,
        VerificationEvent {
            langkah: vec![
                langkah(NAMA_KONEKSI, LangkahStatus::Berjalan, None),
                langkah(NAMA_DOCKER, LangkahStatus::Menunggu, None),
                langkah(NAMA_REGISTRY, LangkahStatus::Menunggu, None),
            ],
            tofu_pending_fingerprint: None,
        },
    );

    let outcome = ssh::connect(
        &row.host,
        row.port as u16,
        &row.ssh_user,
        &plaintext_key,
        &state.config.runtime_dir,
        mode,
    )
    .await;

    match outcome {
        Ok(ssh::ConnectOutcome::TofuPending { session, probe }) => {
            let fingerprint = probe.fingerprint.clone();
            let _ = session.close().await;

            state.events.publish(
                &server_id,
                VerificationEvent {
                    langkah: vec![
                        langkah(NAMA_KONEKSI, LangkahStatus::Berjalan, None),
                        langkah(NAMA_DOCKER, LangkahStatus::Menunggu, None),
                        langkah(NAMA_REGISTRY, LangkahStatus::Menunggu, None),
                    ],
                    tofu_pending_fingerprint: Some(fingerprint),
                },
            );
            // Status db tetap `verifying`, menunggu
            // `konfirmasi_hostkey_dan_lanjutkan` — tidak dipersist lebih
            // jauh di titik ini.
        }
        Ok(ssh::ConnectOutcome::Established(session)) => {
            jalankan_docker_dan_selesai(&state, &server_id, session).await;
        }
        Err(err) => {
            let kegagalan = classify_connect_error(err);
            selesai_gagal(&state, &server_id, false, &kegagalan).await;
        }
    }
}

/// Dipanggil route `POST /servers/{id}/hostkey/konfirmasi` (sub-blok 3f)
/// setelah pengguna klik "Ya, Terima & Simpan". Mengambil ulang fingerprint
/// (lihat catatan modul), menyimpannya, lalu melanjutkan verifikasi
/// (koneksi Strict + Docker) sebagai task latar terpisah — pemanggil boleh
/// langsung redirect setelah `Ok(())` tanpa menunggu.
/// `fingerprint_disetujui` adalah nilai yang dikirim klien (field
/// tersembunyi form konfirmasi, diisi dari fingerprint yang ditampilkan
/// SSE) — divalidasi terhadap hasil probe ULANG, bukan dipercaya begitu
/// saja (`docs/api-contract.md` "tidak cocok → tolak, ini yang mencegah
/// pengguna menyetujui fingerprint lama saat kunci sudah berganti").
pub async fn konfirmasi_hostkey_dan_lanjutkan(
    state: AppState,
    server_id: String,
    fingerprint_disetujui: &str,
) -> Result<(), KonfirmasiHostkeyError> {
    let row = repo::find_by_id(&state.db_read, &server_id)
        .await
        .map_err(|err| KonfirmasiHostkeyError::Lain(err.to_string()))?
        .ok_or(KonfirmasiHostkeyError::ServerTidakDitemukan)?;

    let probe = ssh::fetch_fingerprint_via_keyscan(
        &row.host,
        row.port as u16,
        &state.config.runtime_dir,
        HOSTKEY_PROBE_TIMEOUT,
    )
    .await
    .map_err(|_| {
        KonfirmasiHostkeyError::Lain(
            "gagal mengambil ulang fingerprint host key saat konfirmasi".to_string(),
        )
    })?;

    if probe.fingerprint != fingerprint_disetujui {
        return Err(KonfirmasiHostkeyError::FingerprintTidakCocok);
    }

    if let Some(existing) = &row.host_key_fingerprint
        && existing != &probe.fingerprint
    {
        // Belum ada endpoint untuk mengganti fingerprint tersimpan (Q6
        // `docs/plan.md` belum diputuskan) — hanya mengisi yang masih
        // kosong, tidak pernah menimpa.
        return Err(KonfirmasiHostkeyError::FingerprintSudahTersimpanBerbeda);
    }

    ssh::confirm_and_store(&state.config.runtime_dir, &probe).map_err(|_| {
        KonfirmasiHostkeyError::Lain(
            "gagal menyimpan fingerprint host key yang dikonfirmasi".to_string(),
        )
    })?;

    repo::set_host_key_fingerprint(&state.db_write, &server_id, &probe.fingerprint)
        .await
        .map_err(|err| KonfirmasiHostkeyError::Lain(err.to_string()))?;

    tokio::spawn(async move {
        lanjutkan_setelah_konfirmasi(state, server_id).await;
    });

    Ok(())
}

#[derive(Debug)]
pub enum KonfirmasiHostkeyError {
    ServerTidakDitemukan,
    /// Fingerprint yang disetujui klien tidak cocok dengan hasil probe
    /// ulang — host mungkin berubah di antara wizard menampilkan
    /// fingerprint dan pengguna mengklik konfirmasi.
    FingerprintTidakCocok,
    /// Server sudah punya fingerprint tersimpan yang BERBEDA — endpoint
    /// ini tidak pernah menimpa (`docs/api-contract.md` 409).
    FingerprintSudahTersimpanBerbeda,
    Lain(String),
}

async fn lanjutkan_setelah_konfirmasi(state: AppState, server_id: String) {
    let Some(row) = fetch_row(&state.db_read, &server_id).await else {
        return;
    };
    let Some(fingerprint) = row.host_key_fingerprint.clone() else {
        tracing::warn!(
            server_id,
            "lanjut verifikasi tanpa fingerprint tersimpan — seharusnya tidak terjadi"
        );
        return;
    };

    let plaintext_key = match state.crypto.decrypt(&row.ssh_key_encrypted) {
        Ok(key) => key,
        Err(err) => {
            tracing::warn!(error = %err, server_id, "gagal dekripsi kunci ssh (lanjutan konfirmasi)");
            selesai_gagal(
                &state,
                &server_id,
                false,
                &LangkahKegagalan::Lain("kunci SSH tersimpan tidak bisa didekripsi".to_string()),
            )
            .await;
            return;
        }
    };

    state.events.publish(
        &server_id,
        VerificationEvent {
            langkah: vec![
                langkah(NAMA_KONEKSI, LangkahStatus::Sukses, None),
                langkah(NAMA_DOCKER, LangkahStatus::Berjalan, None),
                langkah(NAMA_REGISTRY, LangkahStatus::Menunggu, None),
            ],
            tofu_pending_fingerprint: None,
        },
    );

    let outcome = ssh::connect(
        &row.host,
        row.port as u16,
        &row.ssh_user,
        &plaintext_key,
        &state.config.runtime_dir,
        HostKeyMode::Strict {
            expected_fingerprint: fingerprint,
        },
    )
    .await;

    match outcome {
        Ok(ssh::ConnectOutcome::Established(session)) => {
            jalankan_docker_dan_selesai(&state, &server_id, session).await;
        }
        Ok(ssh::ConnectOutcome::TofuPending { session, .. }) => {
            // Tidak mungkin secara normal (fingerprint baru saja disimpan)
            // — ditangani tetap tanpa panik, bukan diasumsikan mustahil.
            let _ = session.close().await;
            selesai_gagal(
                &state,
                &server_id,
                false,
                &LangkahKegagalan::Lain(
                    "status host key tidak konsisten setelah konfirmasi".to_string(),
                ),
            )
            .await;
        }
        Err(err) => {
            let kegagalan = classify_connect_error(err);
            selesai_gagal(&state, &server_id, false, &kegagalan).await;
        }
    }
}

async fn jalankan_docker_dan_selesai(state: &AppState, server_id: &str, session: SshSession) {
    let hasil = jalankan_langkah_docker(&session, &state.config.runtime_dir, server_id).await;
    let _ = session.close().await;

    match hasil {
        Ok((docker_version, os_info)) => {
            if let Err(err) = repo::mark_online(
                &state.db_write,
                server_id,
                &docker_version,
                &os_info,
                NORMAL_POLL_INTERVAL_SECS,
            )
            .await
            {
                tracing::warn!(error = %err, server_id, "gagal tandai server online");
            }

            state.events.publish(
                server_id,
                VerificationEvent {
                    langkah: vec![
                        langkah(NAMA_KONEKSI, LangkahStatus::Sukses, None),
                        langkah(NAMA_DOCKER, LangkahStatus::Sukses, None),
                        langkah(NAMA_REGISTRY, LangkahStatus::Menunggu, None),
                    ],
                    tofu_pending_fingerprint: None,
                },
            );
            state.events.remove(server_id);
        }
        Err(kegagalan) => {
            selesai_gagal(state, server_id, true, &kegagalan).await;
        }
    }
}

async fn jalankan_langkah_docker(
    session: &SshSession,
    runtime_dir: &Path,
    server_id: &str,
) -> Result<(String, String), LangkahKegagalan> {
    let exec_result = ssh::exec(
        session,
        "docker",
        &["version", "--format", "{{.Server.Version}}"],
        DOCKER_COMMAND_TIMEOUT,
    )
    .await
    .map_err(classify_exec_error)?;

    let docker_version = classify_docker_exec(&exec_result)?;

    let forward = docker::establish(session, runtime_dir, server_id)
        .await
        .map_err(|err| match err {
            docker::DockerForwardError::Timeout => LangkahKegagalan::Lain(
                "membuka forward socket docker melebihi batas waktu".to_string(),
            ),
            docker::DockerForwardError::Other(msg) => LangkahKegagalan::Lain(msg),
        })?;

    let hasil_bollard = jalankan_bollard(&forward).await;
    docker::close(session, forward).await;

    let os_info = hasil_bollard?;
    Ok((docker_version, os_info))
}

async fn jalankan_bollard(forward: &docker::DockerForward) -> Result<String, LangkahKegagalan> {
    let client = docker::connect(forward.socket_path()).map_err(|_| {
        LangkahKegagalan::Lain("gagal menyambung ke docker lewat socket forward".to_string())
    })?;

    docker::ping(&client)
        .await
        .map_err(|_| LangkahKegagalan::Lain("bollard ping ke docker gagal".to_string()))?;

    docker::os_info(&client)
        .await
        .map_err(|_| LangkahKegagalan::Lain("gagal membaca info sistem docker".to_string()))
}

async fn selesai_gagal(
    state: &AppState,
    server_id: &str,
    di_langkah_docker: bool,
    kegagalan: &LangkahKegagalan,
) {
    if let Err(err) = repo::mark_verification_failed(
        &state.db_write,
        server_id,
        kegagalan.kind(),
        &kegagalan.pesan(),
    )
    .await
    {
        tracing::warn!(error = %err, server_id, "gagal tandai verifikasi server gagal");
    }

    let pesan = kegagalan.pesan();
    let (koneksi, docker_langkah) = if di_langkah_docker {
        (
            langkah(NAMA_KONEKSI, LangkahStatus::Sukses, None),
            langkah(NAMA_DOCKER, LangkahStatus::Gagal, Some(pesan)),
        )
    } else {
        (
            langkah(NAMA_KONEKSI, LangkahStatus::Gagal, Some(pesan)),
            langkah(NAMA_DOCKER, LangkahStatus::Menunggu, None),
        )
    };

    state.events.publish(
        server_id,
        VerificationEvent {
            langkah: vec![
                koneksi,
                docker_langkah,
                langkah(NAMA_REGISTRY, LangkahStatus::Menunggu, None),
            ],
            tofu_pending_fingerprint: None,
        },
    );
    state.events.remove(server_id);
}

async fn fetch_row(pool: &SqlitePool, server_id: &str) -> Option<repo::ServerRow> {
    match repo::find_by_id(pool, server_id).await {
        Ok(Some(row)) => Some(row),
        Ok(None) => {
            tracing::warn!(server_id, "server tidak ditemukan saat verifikasi");
            None
        }
        Err(err) => {
            tracing::warn!(error = %err, server_id, "gagal baca server saat verifikasi");
            None
        }
    }
}

/// Input langkah 3 (opsional): login registry di server target.
/// `docs/api-contract.md`: `registry_id` diisi → pakai ulang registry
/// tersimpan (field host/username/token diabaikan); kosong → registry baru.
pub enum RegistryStepInput<'a> {
    Baru {
        registry_host: &'a str,
        username: &'a str,
        password: &'a str,
    },
    PakaiUlang {
        registry_id: &'a str,
    },
}

/// Kegagalan langkah 3 — jauh lebih sederhana dari `LangkahKegagalan`
/// karena `docs/design/tambah-server.md` §4.3 poin 4 hanya meminta dua
/// pesan spesifik (kredensial ditolak, timeout jaringan) plus fallback.
#[derive(Debug)]
pub enum RegistryStepError {
    ServerTidakDitemukan,
    RegistryTidakDitemukan,
    /// Server belum pernah lolos verifikasi koneksi (belum ada
    /// fingerprint tersimpan) — langkah 3 butuh koneksi Strict yang sudah
    /// terverifikasi, tidak mengulang TOFU.
    KoneksiGagal,
    Ditolak {
        detail: String,
    },
    Timeout,
    Lain(String),
}

/// Login registry di server target lewat sesi SSH yang dibangun ulang
/// (Strict, fingerprint sudah tersimpan dari langkah 1). Sinkron —
/// dipanggil langsung dari handler route, TIDAK lewat SSE
/// (`docs/design/tambah-server.md` §4.3 poin 2).
pub async fn tautkan_registry(
    state: &AppState,
    server_id: &str,
    input: RegistryStepInput<'_>,
) -> Result<(), RegistryStepError> {
    let row = repo::find_by_id(&state.db_read, server_id)
        .await
        .map_err(|err| RegistryStepError::Lain(err.to_string()))?
        .ok_or(RegistryStepError::ServerTidakDitemukan)?;

    // Selesaikan kredensial SEBELUM menyentuh jaringan — kalau `registry_id`
    // yang dipakai ulang tidak dikenal, gagal cepat dengan 404, bukan
    // setelah membuka koneksi SSH sia-sia.
    let kredensial = match &input {
        RegistryStepInput::Baru {
            registry_host,
            username,
            password,
        } => KredensialRegistry {
            host: (*registry_host).to_string(),
            username: (*username).to_string(),
            password: (*password).to_string(),
            registry_id_tersimpan: None,
        },
        RegistryStepInput::PakaiUlang { registry_id } => {
            let existing = registries::repo::find_by_id(&state.db_read, registry_id)
                .await
                .map_err(|err| RegistryStepError::Lain(err.to_string()))?
                .ok_or(RegistryStepError::RegistryTidakDitemukan)?;

            let password = state
                .crypto
                .decrypt(&existing.token_encrypted)
                .map_err(|err| RegistryStepError::Lain(err.to_string()))?;

            KredensialRegistry {
                host: existing.host,
                username: existing.username,
                password,
                registry_id_tersimpan: Some(existing.id),
            }
        }
    };

    let Some(fingerprint) = row.host_key_fingerprint.clone() else {
        return Err(RegistryStepError::KoneksiGagal);
    };

    let plaintext_key = state
        .crypto
        .decrypt(&row.ssh_key_encrypted)
        .map_err(|err| RegistryStepError::Lain(err.to_string()))?;

    let session = match ssh::connect(
        &row.host,
        row.port as u16,
        &row.ssh_user,
        &plaintext_key,
        &state.config.runtime_dir,
        HostKeyMode::Strict {
            expected_fingerprint: fingerprint,
        },
    )
    .await
    {
        Ok(ssh::ConnectOutcome::Established(session)) => session,
        Ok(ssh::ConnectOutcome::TofuPending { session, .. }) => {
            let _ = session.close().await;
            return Err(RegistryStepError::KoneksiGagal);
        }
        Err(_) => return Err(RegistryStepError::KoneksiGagal),
    };

    let login_result = docker::registry_login::login(
        &session,
        &kredensial.host,
        &kredensial.username,
        &kredensial.password,
    )
    .await;

    let _ = session.close().await;

    match login_result {
        Ok(()) => {
            match kredensial.registry_id_tersimpan {
                Some(registry_id) => {
                    registries::repo::record_login_success(
                        &state.db_write,
                        server_id,
                        &registry_id,
                    )
                    .await
                    .map_err(|err| RegistryStepError::Lain(err.to_string()))?;
                }
                None => {
                    let token_encrypted = state
                        .crypto
                        .encrypt(&kredensial.password)
                        .map_err(|err| RegistryStepError::Lain(err.to_string()))?;
                    // Upsert registry BARU + catat link server-registry dalam SATU
                    // transaksi (invariant 10) — lihat dokumentasi
                    // `upsert_dan_catat_login`.
                    registries::repo::upsert_dan_catat_login(
                        &state.db_write,
                        &kredensial.host,
                        &kredensial.username,
                        &token_encrypted,
                        server_id,
                    )
                    .await
                    .map_err(|err| RegistryStepError::Lain(err.to_string()))?;
                }
            }

            Ok(())
        }
        Err(docker::RegistryLoginError::Rejected { detail }) => {
            Err(RegistryStepError::Ditolak { detail })
        }
        Err(docker::RegistryLoginError::Timeout) => Err(RegistryStepError::Timeout),
        Err(docker::RegistryLoginError::Disconnected) => Err(RegistryStepError::KoneksiGagal),
        Err(docker::RegistryLoginError::Other(msg)) => Err(RegistryStepError::Lain(msg)),
    }
}

struct KredensialRegistry {
    host: String,
    username: String,
    password: String,
    registry_id_tersimpan: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec_result(code: i32, stdout: &str, stderr: &str) -> ssh::ExecResult {
        ssh::ExecResult {
            code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn classify_docker_exec_sukses_mengembalikan_versi_trim() {
        let hasil = classify_docker_exec(&exec_result(0, "24.0.7\n", ""));
        assert_eq!(hasil.unwrap(), "24.0.7");
    }

    #[test]
    fn classify_docker_exec_127_berarti_docker_tidak_ada() {
        let hasil = classify_docker_exec(&exec_result(127, "", "sh: docker: not found"));
        assert!(matches!(hasil, Err(LangkahKegagalan::DockerTidakAda)));
    }

    #[test]
    fn classify_docker_exec_permission_denied_berarti_akses_ditolak() {
        let hasil = classify_docker_exec(&exec_result(
            1,
            "",
            "Got permission denied while trying to connect to the Docker daemon socket",
        ));
        assert!(matches!(hasil, Err(LangkahKegagalan::AksesDockerDitolak)));
    }

    #[test]
    fn classify_docker_exec_kegagalan_lain_jatuh_ke_lain() {
        let hasil =
            classify_docker_exec(&exec_result(1, "", "Cannot connect to the Docker daemon"));
        assert!(matches!(hasil, Err(LangkahKegagalan::Lain(_))));
    }

    #[test]
    fn classify_connect_error_memetakan_kategori_dengan_benar() {
        assert!(matches!(
            classify_connect_error(ssh::SshConnectError::Unreachable),
            LangkahKegagalan::Unreachable
        ));
        assert!(matches!(
            classify_connect_error(ssh::SshConnectError::AuthRejected),
            LangkahKegagalan::AuthRejected
        ));
        assert!(matches!(
            classify_connect_error(ssh::SshConnectError::HostKeyMismatch {
                expected: "SHA256:lama".to_string(),
                offered: "SHA256:baru".to_string(),
            }),
            LangkahKegagalan::HostKeyBerubah { .. }
        ));
    }

    #[test]
    fn setiap_kategori_kegagalan_punya_kind_dan_pesan_tidak_kosong() {
        let kegagalan = [
            LangkahKegagalan::Unreachable,
            LangkahKegagalan::AuthRejected,
            LangkahKegagalan::DockerTidakAda,
            LangkahKegagalan::AksesDockerDitolak,
            LangkahKegagalan::HostKeyBerubah {
                lama: "a".to_string(),
                baru: "b".to_string(),
            },
            LangkahKegagalan::Lain("detail".to_string()),
        ];

        for k in &kegagalan {
            assert!(!k.kind().is_empty());
            assert!(!k.pesan().is_empty());
            // Pesan simpan ke `servers.last_error_message` yang CHECK
            // <=500 karakter — pastikan tabel pesan finalnya sendiri tidak
            // pernah melebihi itu (truncation di repo.rs adalah jaring
            // pengaman kedua, bukan yang pertama).
            assert!(k.pesan().chars().count() <= 500);
        }
    }
}
