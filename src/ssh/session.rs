//! Bangun koneksi SSH ControlMaster (native-mux) ke satu host, dengan
//! timeout 10 detik WAJIB untuk keseluruhan proses (TCP + handshake +
//! auth) dan mode TOFU vs ketat untuk host key.
//!
//! Kredensial diterima sebagai private key PLAINTEXT (`&str`) — pemanggil
//! (`servers/verify.rs`, sub-blok 3d) sudah mendekripsi lebih dulu lewat
//! `crate::crypto::CryptoKey`. Modul ini tidak menyentuh db maupun `age`.
//!
//! **Keputusan desain host key**: alih-alih bergantung pada mekanisme
//! known_hosts bawaan `openssh` (`KnownHosts::Strict`/`Add`, yang rapuh
//! terhadap format entri dan file yang dipakai), fingerprint diverifikasi
//! sendiri di layer aplikasi lewat `hostkey::probe` (`ssh-keyscan` +
//! `ssh-keygen -lf`) SEBELUM autentikasi kunci dicoba sama sekali. Setelah
//! fingerprint cocok (atau ini percobaan TOFU pertama), koneksi aktual
//! memakai `KnownHosts::Accept` karena pemeriksaan sesungguhnya sudah
//! selesai — openssh tidak diberi kesempatan menolak/menerima berdasarkan
//! file known_hosts-nya sendiri. Ini membuat perbandingan fingerprint jadi
//! satu jalur kode yang sepenuhnya bisa dites (`hostkey.rs`), bukan
//! bergantung pada parsing pesan `ssh` yang berbeda-beda antar versi.

use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use openssh::{KnownHosts, SessionBuilder};

use super::hostkey::{self, FingerprintProbe, HostKeyError};

/// Batas waktu WAJIB untuk membangun koneksi SSH (TCP + handshake + auth).
/// Angka ini eksplisit dari PRD (`docs/plan.md`, tabel "Timeout per
/// tahap") — bukan usulan yang boleh diubah tanpa alasan tertulis.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Sesi SSH ControlMaster yang sudah terbuka. Handle tipis di atas
/// `openssh::Session` — tidak memegang private key (kunci hanya hidup
/// sebentar di file sementara selama proses connect, lalu dihapus).
pub struct SshSession {
    pub(super) inner: openssh::Session,
}

/// Mode verifikasi host key untuk satu percobaan koneksi.
pub enum HostKeyMode {
    /// Belum ada fingerprint tersimpan untuk server ini. Fingerprint yang
    /// ditawarkan dikembalikan ke pemanggil untuk ditampilkan dan
    /// dikonfirmasi pengguna — TIDAK ditulis permanen ke known_hosts
    /// aplikasi di sini (lihat `hostkey::confirm_and_store`).
    Tofu,
    /// Fingerprint sudah tersimpan sebelumnya
    /// (`servers.host_key_fingerprint`). Kalau fingerprint yang ditawarkan
    /// sekarang tidak cocok persis, koneksi ditolak KERAS sebelum
    /// autentikasi kunci dicoba sama sekali — mencegah private key
    /// terkirim ke host yang berpotensi spoofed (Q6: tidak ada tindakan
    /// otomatis, tidak ada override).
    Strict { expected_fingerprint: String },
}

/// Hasil koneksi yang berhasil.
pub enum ConnectOutcome {
    /// Mode ketat, fingerprint cocok, sesi siap dipakai.
    Established(SshSession),
    /// Mode TOFU, sesi siap dipakai TAPI fingerprint belum dikonfirmasi
    /// pengguna. Pemanggil menampilkan `probe.fingerprint`, dan kalau
    /// pengguna setuju memanggil `hostkey::confirm_and_store` dengan
    /// `probe.known_hosts_entry`.
    TofuPending {
        session: SshSession,
        probe: FingerprintProbe,
    },
}

/// Kategori kegagalan koneksi. Dibedakan supaya pemanggil bisa memilih
/// pesan Bahasa Indonesia yang tepat sesuai `docs/design/tambah-server.md`
/// §4.2 (kategori A, B, E) TANPA meneruskan pesan mentah `openssh`.
#[derive(Debug)]
pub enum SshConnectError {
    /// Kategori A — host tidak terjangkau (resolusi gagal, TCP ditolak,
    /// atau batas waktu 10 detik tercapai).
    Unreachable,
    /// Kategori B — autentikasi kunci ditolak oleh server target.
    AuthRejected,
    /// Kategori E — fingerprint host key yang ditawarkan sekarang berbeda
    /// dari yang sudah tersimpan. Gagal keras, TANPA override otomatis
    /// (Q6, `docs/plan.md`).
    HostKeyMismatch { expected: String, offered: String },
    /// Kegagalan lain yang tidak masuk tiga kategori di atas (mis. gagal
    /// menulis file kunci sementara, `ssh`/`ssh-keyscan`/`ssh-keygen`
    /// tidak ada di PATH). Pesan sudah generik — detail asli hanya ke
    /// `tracing`.
    Other(String),
}

/// Bangun koneksi SSH ke `host:port` sebagai `ssh_user`, memakai
/// `private_key` (plaintext OpenSSH, TANPA passphrase — Q2 `docs/plan.md`).
/// `runtime_dir` adalah direktori privat aplikasi (dibuat pemanggil, mis.
/// `{data_dir}/runtime`) tempat file kunci sementara dan file bantu
/// `ssh-keyscan` ditulis mode `0600` lalu dihapus setelah dipakai.
pub async fn connect(
    host: &str,
    port: u16,
    ssh_user: &str,
    private_key: &str,
    runtime_dir: &Path,
    mode: HostKeyMode,
) -> Result<ConnectOutcome, SshConnectError> {
    match tokio::time::timeout(
        CONNECT_TIMEOUT,
        connect_inner(host, port, ssh_user, private_key, runtime_dir, mode),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(SshConnectError::Unreachable),
    }
}

async fn connect_inner(
    host: &str,
    port: u16,
    ssh_user: &str,
    private_key: &str,
    runtime_dir: &Path,
    mode: HostKeyMode,
) -> Result<ConnectOutcome, SshConnectError> {
    let probe = hostkey::probe(host, port, runtime_dir, CONNECT_TIMEOUT)
        .await
        .map_err(map_hostkey_error)?;

    if let HostKeyMode::Strict {
        ref expected_fingerprint,
    } = mode
        && &probe.fingerprint != expected_fingerprint
    {
        return Err(SshConnectError::HostKeyMismatch {
            expected: expected_fingerprint.clone(),
            offered: probe.fingerprint,
        });
    }

    let key_file = TempFile::write(runtime_dir, "key", private_key.as_bytes()).map_err(|err| {
        SshConnectError::Other(format!("gagal menulis file kunci sementara: {err}"))
    })?;

    let mut builder = SessionBuilder::default();
    builder
        .user(ssh_user.to_string())
        .port(port)
        .keyfile(key_file.path())
        .known_hosts_check(KnownHosts::Accept)
        .connect_timeout(CONNECT_TIMEOUT);

    let result = builder.connect_mux(host).await;

    // File kunci privat WAJIB terhapus setelah dipakai, baik sesi berhasil
    // maupun gagal. `Drop` pada `TempFile` sudah menjamin ini di semua
    // jalur keluar termasuk lewat `?` di bawah — `drop` eksplisit di sini
    // membuat kapan file dihapus terlihat jelas di kode, bukan hanya
    // implisit lewat scope.
    drop(key_file);

    let session = result.map_err(map_openssh_error)?;

    match mode {
        HostKeyMode::Tofu => Ok(ConnectOutcome::TofuPending {
            session: SshSession { inner: session },
            probe,
        }),
        HostKeyMode::Strict { .. } => {
            Ok(ConnectOutcome::Established(SshSession { inner: session }))
        }
    }
}

impl SshSession {
    /// Buka local port forward: soket unix lokal `listen_socket` diteruskan
    /// lewat sesi SSH ini ke soket unix `connect_socket` di server target.
    /// Dipakai `docker/forward.rs` untuk menjangkau socket Docker TANPA
    /// pernah membuka port TCP (invariant 13) — `ssh` yang membuat dan
    /// mendengarkan `listen_socket` secara lokal, bukan kode aplikasi ini.
    pub async fn forward_unix_local(
        &self,
        listen_socket: &Path,
        connect_socket: &Path,
    ) -> Result<(), openssh::Error> {
        self.inner
            .request_port_forward(openssh::ForwardType::Local, listen_socket, connect_socket)
            .await
    }

    /// Tutup forward yang dibuka [`SshSession::forward_unix_local`]. Argumen
    /// harus persis sama dengan yang dipakai saat membuka.
    pub async fn close_unix_local_forward(
        &self,
        listen_socket: &Path,
        connect_socket: &Path,
    ) -> Result<(), openssh::Error> {
        self.inner
            .close_port_forward(openssh::ForwardType::Local, listen_socket, connect_socket)
            .await
    }

    /// Hentikan proses ControlMaster SSH milik sesi ini. WAJIB dipanggil
    /// eksplisit oleh pemanggil (`servers::verify`) di semua jalur keluar —
    /// `Drop` biasa TIDAK menjamin proses `ssh` lokal ikut berhenti bersih.
    pub async fn close(self) -> Result<(), openssh::Error> {
        self.inner.close().await
    }
}

fn map_hostkey_error(err: HostKeyError) -> SshConnectError {
    match err {
        HostKeyError::Timeout | HostKeyError::Unreachable => SshConnectError::Unreachable,
        HostKeyError::ParseFailed | HostKeyError::Io(_) => {
            SshConnectError::Other("gagal membaca host key server target".to_string())
        }
    }
}

fn map_openssh_error(err: openssh::Error) -> SshConnectError {
    // Detail asli TIDAK PERNAH memuat isi private key — `openssh` hanya
    // pernah mencetak PATH file kunci ke error/stderr-nya, bukan isinya,
    // dan path itu sendiri bukan secret. Aman dicatat ke tracing di sini
    // (invariant 7 tetap terjaga: yang dikembalikan ke pemanggil di bawah
    // sudah kategori generik, bukan `err` mentah).
    tracing::warn!(error = %err, "koneksi ssh gagal");

    match err {
        openssh::Error::Connect(io_err) => match io_err.kind() {
            io::ErrorKind::PermissionDenied => SshConnectError::AuthRejected,
            _ => SshConnectError::Unreachable,
        },
        openssh::Error::Master(_) | openssh::Error::SshMux(_) => {
            SshConnectError::Other("gagal memulai proses ssh lokal".to_string())
        }
        _ => SshConnectError::Other("kegagalan ssh yang tidak dikenali".to_string()),
    }
}

/// File sementara mode `0600` di dalam `runtime_dir`, dihapus otomatis
/// lewat `Drop` — dipakai untuk file kunci privat SSH (harus ada sebagai
/// path fisik; `openssh` butuh `IdentityFile`, tidak bisa terima string
/// kunci langsung) maupun output mentah `ssh-keyscan` (`hostkey.rs`).
/// Dihapus pada SEMUA jalur keluar (sukses maupun error) karena
/// pembersihan ada di `Drop`, bukan ditulis manual di tiap `return`.
pub(super) struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub(super) fn write(runtime_dir: &Path, prefix: &str, content: &[u8]) -> io::Result<Self> {
        let dir = runtime_dir.join("ssh-tmp");
        std::fs::create_dir_all(&dir)?;
        set_mode(&dir, 0o700)?;

        let path = dir.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        use io::Write;
        file.write_all(content)?;
        file.sync_all()?;

        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // Kegagalan hapus tidak fatal (mis. sudah terhapus manual), tapi
        // dicatat supaya tidak diam-diam menumpuk file kunci di disk.
        if let Err(err) = std::fs::remove_file(&self.path)
            && err.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(error = %err, "gagal menghapus file sementara ssh");
        }
    }
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_file_dibuat_mode_0600_dan_terhapus_saat_drop() {
        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-ssh-tempfile-{}-{}",
            std::process::id(),
            unique_suffix()
        ));

        let path_setelah_ditulis;
        {
            let temp = TempFile::write(&dir, "test-key", b"isi-kunci-dummy")
                .expect("tulis file sementara harus sukses");

            let metadata =
                std::fs::metadata(temp.path()).expect("metadata file sementara harus terbaca");
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "file kunci sementara harus bermode 0600");

            let isi = std::fs::read(temp.path()).expect("baca file sementara harus sukses");
            assert_eq!(isi, b"isi-kunci-dummy");

            path_setelah_ditulis = temp.path().to_path_buf();
            assert!(path_setelah_ditulis.exists(), "file harus ada saat dipakai");
        } // `temp` di-drop di sini — file WAJIB terhapus setelahnya.

        assert!(
            !path_setelah_ditulis.exists(),
            "file kunci sementara harus terhapus otomatis setelah Drop"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn temp_file_terhapus_walau_dipakai_lewat_jalur_yang_dianggap_gagal() {
        // Simulasikan pola "dipakai lalu error di jalur pemanggil" — Drop
        // tetap wajib menghapus file terlepas dari bagaimana pemanggil
        // memperlakukan hasilnya (RAII tidak peduli sukses/gagal).
        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-ssh-tempfile-gagal-{}-{}",
            std::process::id(),
            unique_suffix()
        ));

        let hasil: Result<(), &str> = (|| {
            let temp =
                TempFile::write(&dir, "test-key", b"isi-kunci-dummy").map_err(|_| "tulis gagal")?;
            let _ = temp.path();
            Err("simulasi kegagalan setelah file dipakai")
        })();

        assert!(hasil.is_err());

        let sisa: Vec<_> = std::fs::read_dir(dir.join("ssh-tmp"))
            .map(|entries| entries.filter_map(Result::ok).collect())
            .unwrap_or_default();
        assert!(
            sisa.is_empty(),
            "tidak boleh ada file kunci sementara tersisa setelah jalur gagal"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
