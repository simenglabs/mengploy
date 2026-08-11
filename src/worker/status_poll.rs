//! Satu siklus polling status armada: SSH connect (Strict — fingerprint
//! sudah tersimpan dari verifikasi awal) + forward socket Docker + panggilan
//! Bollard read-only (`ping`, `version`, `info`). Scanner periodik tidak lagi
//! hanya menjalankan perintah shell `docker version`; jalur ini benar-benar
//! memverifikasi daemon Docker melalui socket target. Backoff eksponensial
//! saat gagal, `unreachable` setelah 3 kegagalan berturut-turut
//! (`docs/plan.md` "Polling status dan backoff").

use tokio::task::JoinSet;

use crate::docker;
use crate::servers::model::StatusServer;
use crate::servers::repo::{self, PollWrite, PollWriteGagal, PollWriteSukses, ServerRow};
use crate::servers::verify::{self, LangkahKegagalan, NORMAL_POLL_INTERVAL_SECS};
use crate::ssh::{self, HostKeyMode};
use crate::state::AppState;

/// Batas konkurensi poll per siklus — `docs/plan.md`: "8 VPS tidak
/// membuka 8 koneksi sekaligus".
const MAX_CONCURRENT_POLLS: usize = 4;

/// Batas tahap Docker saat scanner periodik berbicara lewat socket forward.
/// Timeout per panggilan dipelihara di `docker::client` (5 detik), sedangkan
/// pembukaan forward punya timeout tahap sendiri (10 detik).
///
/// Menit backoff berurutan (1-based, index 0 = kegagalan ke-1) sebelum
/// jatuh ke plateau `BACKOFF_CEILING_MINUTES` — `docs/plan.md`:
/// "1, 2, 4, 8, lalu tetap 15 menit".
const BACKOFF_STEPS_MINUTES: [i64; 4] = [1, 2, 4, 8];
const BACKOFF_CEILING_MINUTES: i64 = 15;

/// `consecutive_failures >= ambang ini` → status `unreachable`
/// (`docs/plan.md`, `docs/prd.md:242`).
const UNREACHABLE_THRESHOLD: i64 = 3;

/// Backoff murni (detik) untuk kegagalan ke-`n` (1-based, `n` SUDAH
/// termasuk kegagalan yang baru saja terjadi). Fungsi murni supaya bisa
/// dites tanpa menunggu waktu nyata (`docs/plan.md`).
pub fn backoff_secs(n: i64) -> i64 {
    let menit = match usize::try_from(n.saturating_sub(1)) {
        Ok(idx) if idx < BACKOFF_STEPS_MINUTES.len() => BACKOFF_STEPS_MINUTES[idx],
        _ => BACKOFF_CEILING_MINUTES,
    };
    menit * 60
}

/// Hitung status dan jadwal poll berikutnya setelah SATU kegagalan baru,
/// murni dari status+`consecutive_failures` SEBELUM kegagalan ini. Fungsi
/// murni, dites langsung tanpa I/O.
///
/// **Penting**: di bawah ambang, status SEBELUMNYA dipertahankan apa
/// adanya — TIDAK dipaksa jadi `Online`. `list_due_for_poll` hanya memilih
/// server yang sudah pernah lolos verifikasi awal (fingerprint tersimpan),
/// jadi status masuk normalnya memang `Online`/`Unreachable`. Memaksa
/// `Online` di sini pernah jadi bug nyata: server yang sempat kepilih poll
/// sebelum verifikasi awal selesai (`status` masih `pending`/`verifying`)
/// bisa disulap jadi `Online` walau belum pernah benar-benar online.
fn hitung_setelah_gagal(
    status_sebelum: StatusServer,
    consecutive_failures_sebelum: i64,
    now: i64,
) -> (StatusServer, i64, i64) {
    let gagal_ke = consecutive_failures_sebelum + 1;
    let status = if gagal_ke >= UNREACHABLE_THRESHOLD {
        StatusServer::Unreachable
    } else {
        status_sebelum
    };
    (status, gagal_ke, now + backoff_secs(gagal_ke))
}

struct PollOutcomeSukses {
    docker_version: String,
    os_info: String,
}

/// Jalankan satu siklus: ambil server jatuh tempo, periksa dengan batas
/// konkurensi, tulis semua hasil dalam satu transaksi. Tidak pernah panik
/// karena satu server gagal — kegagalan per server ditangkap sebagai nilai,
/// bukan `Err` yang menghentikan siklus (`AGENTS.md`: "loop tidak boleh
/// mati karena satu error").
pub async fn jalankan_satu_siklus(state: &AppState) {
    let now = now_epoch();

    let due = match repo::list_due_for_poll(&state.db_read, now).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "gagal baca daftar server jatuh tempo poll");
            return;
        }
    };

    if due.is_empty() {
        return;
    }

    let mut set: JoinSet<(ServerRow, Result<PollOutcomeSukses, LangkahKegagalan>)> = JoinSet::new();
    let mut writes = Vec::with_capacity(due.len());
    let mut sisa = due.into_iter();

    for row in sisa.by_ref().take(MAX_CONCURRENT_POLLS) {
        spawn_periksa(&mut set, state.clone(), row);
    }

    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((row, hasil)) => writes.push(bangun_poll_write(&row, hasil, now)),
            Err(err) => tracing::warn!(error = %err, "task poll server panik/dibatalkan"),
        }

        if let Some(row) = sisa.next() {
            spawn_periksa(&mut set, state.clone(), row);
        }
    }

    if let Err(err) =
        repo::apply_poll_batch(&state.db_write, &writes, now, NORMAL_POLL_INTERVAL_SECS).await
    {
        tracing::warn!(error = %err, "gagal menyimpan hasil satu siklus polling");
    }
}

fn spawn_periksa(
    set: &mut JoinSet<(ServerRow, Result<PollOutcomeSukses, LangkahKegagalan>)>,
    state: AppState,
    row: ServerRow,
) {
    set.spawn(async move {
        let hasil = periksa_server(&state, &row).await;
        (row, hasil)
    });
}

fn bangun_poll_write(
    row: &ServerRow,
    hasil: Result<PollOutcomeSukses, LangkahKegagalan>,
    now: i64,
) -> PollWrite {
    match hasil {
        Ok(sukses) => PollWrite::Sukses(PollWriteSukses {
            server_id: row.id.clone(),
            docker_version: sukses.docker_version,
            os_info: sukses.os_info,
        }),
        Err(kegagalan) => {
            let status_sebelum = StatusServer::from_db_str(&row.status);
            let (status, consecutive_failures, next_poll_at) =
                hitung_setelah_gagal(status_sebelum, row.consecutive_failures, now);
            PollWrite::Gagal(PollWriteGagal {
                server_id: row.id.clone(),
                status,
                consecutive_failures,
                next_poll_at,
                error_kind: kegagalan.kind().to_string(),
                error_message: kegagalan.pesan(),
            })
        }
    }
}

async fn periksa_server(
    state: &AppState,
    row: &ServerRow,
) -> Result<PollOutcomeSukses, LangkahKegagalan> {
    let Some(fingerprint) = row.host_key_fingerprint.clone() else {
        // Tidak seharusnya terjadi — `next_poll_at` hanya diset setelah
        // verifikasi awal sukses (`servers::repo::mark_online`), yang
        // selalu mensyaratkan fingerprint tersimpan. Ditangani tanpa
        // panik, bukan diasumsikan mustahil.
        return Err(LangkahKegagalan::Lain(
            "server jatuh tempo poll tanpa fingerprint host key tersimpan".to_string(),
        ));
    };

    let plaintext_key = state
        .crypto
        .decrypt(&row.ssh_key_encrypted)
        .map_err(|err| LangkahKegagalan::Lain(err.to_string()))?;

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
            return Err(LangkahKegagalan::Lain(
                "status host key tidak konsisten saat polling".to_string(),
            ));
        }
        Err(err) => return Err(verify::classify_connect_error(err)),
    };

    let forward = match docker::establish(&session, &state.config.runtime_dir, &row.id).await {
        Ok(forward) => forward,
        Err(err) => {
            let _ = session.close().await;
            return Err(LangkahKegagalan::Lain(format!(
                "forward Docker gagal: {err:?}"
            )));
        }
    };
    let hasil = async {
        let client = docker::connect(forward.socket_path())
            .map_err(|err| LangkahKegagalan::Lain(format!("koneksi Docker gagal: {err:?}")))?;
        docker::ping(&client)
            .await
            .map_err(|err| LangkahKegagalan::Lain(format!("ping Docker gagal: {err:?}")))?;
        let docker_version = docker::version(&client)
            .await
            .map_err(|err| LangkahKegagalan::Lain(format!("versi Docker gagal: {err:?}")))?;
        let os_info = docker::os_info(&client)
            .await
            .map_err(|err| LangkahKegagalan::Lain(format!("informasi OS Docker gagal: {err:?}")))?;
        Ok(PollOutcomeSukses {
            docker_version,
            os_info,
        })
    }
    .await;
    docker::close(&session, forward).await;
    let _ = session.close().await;
    hasil
}

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_secs_mengikuti_urutan_1_2_4_8_lalu_plateau_15_menit() {
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(2), 120);
        assert_eq!(backoff_secs(3), 240);
        assert_eq!(backoff_secs(4), 480);
        assert_eq!(backoff_secs(5), 15 * 60);
        assert_eq!(backoff_secs(6), 15 * 60);
        assert_eq!(backoff_secs(100), 15 * 60);
    }

    #[test]
    fn backoff_secs_benar_benar_melambat_setiap_kegagalan_berikutnya() {
        let nilai: Vec<i64> = (1..=6).map(backoff_secs).collect();
        for pasangan in nilai.windows(2) {
            assert!(
                pasangan[1] >= pasangan[0],
                "backoff tidak boleh mengecil: {pasangan:?}"
            );
        }
    }

    #[test]
    fn hitung_setelah_gagal_status_dipertahankan_sebelum_ambang_tiga() {
        let (status, gagal_ke, _) = hitung_setelah_gagal(StatusServer::Online, 0, 1000);
        assert_eq!(status, StatusServer::Online);
        assert_eq!(gagal_ke, 1);

        let (status, gagal_ke, _) = hitung_setelah_gagal(StatusServer::Online, 1, 1000);
        assert_eq!(status, StatusServer::Online);
        assert_eq!(gagal_ke, 2);
    }

    #[test]
    fn hitung_setelah_gagal_unreachable_persis_di_kegagalan_ketiga() {
        let (status, gagal_ke, _) = hitung_setelah_gagal(StatusServer::Online, 2, 1000);
        assert_eq!(status, StatusServer::Unreachable);
        assert_eq!(gagal_ke, 3);
    }

    #[test]
    fn hitung_setelah_gagal_tetap_unreachable_dan_terus_di_poll_setelah_ambang() {
        let (status, gagal_ke, next_poll_at) =
            hitung_setelah_gagal(StatusServer::Unreachable, 10, 1000);
        assert_eq!(status, StatusServer::Unreachable);
        assert_eq!(gagal_ke, 11);
        assert_eq!(next_poll_at, 1000 + 15 * 60);
    }

    #[test]
    fn hitung_setelah_gagal_next_poll_at_konsisten_dengan_backoff_secs() {
        let now = 5000;
        let (_, gagal_ke, next_poll_at) = hitung_setelah_gagal(StatusServer::Online, 0, now);
        assert_eq!(next_poll_at, now + backoff_secs(gagal_ke));
    }

    /// Regresi: server yang belum pernah lolos verifikasi awal (status
    /// masih `pending`/`verifying`) TIDAK PERNAH disulap jadi `Online`
    /// hanya karena kebetulan terpilih worker sebelum ambang `unreachable`
    /// tercapai — bug nyata yang ditemukan smoke test manual sebelum
    /// perbaikan ini (`list_due_for_poll` seharusnya sudah menyaring lewat
    /// `host_key_fingerprint IS NOT NULL`, tapi fungsi ini sendiri tidak
    /// boleh ikut mengasumsikan status masuk selalu `Online`).
    #[test]
    fn hitung_setelah_gagal_tidak_memaksa_online_untuk_server_yang_belum_pernah_online() {
        let (status, _, _) = hitung_setelah_gagal(StatusServer::Pending, 0, 1000);
        assert_eq!(status, StatusServer::Pending);

        let (status, _, _) = hitung_setelah_gagal(StatusServer::Verifying, 1, 1000);
        assert_eq!(status, StatusServer::Verifying);
    }
}
