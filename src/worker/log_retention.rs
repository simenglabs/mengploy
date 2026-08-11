//! Orkestrasi sapuan retensi log harian, ditumpangkan ke tick worker yang
//! sudah ada (`worker::spawn`, tick 30 detik) — BUKAN worker ketiga
//! (`docs/plan.md` Fase 3: "Job retensi log menumpang tick ini").
//!
//! Penanda "kapan terakhir jalan" disimpan `settings.log_retention_last_run_at`
//! (pola key-value yang sama dipakai `settings.password_hash`,
//! `src/main.rs::seed_initial_password`) — bukan mekanisme penjadwalan baru.
//!
//! Dua syarat harus DUA-duanya terpenuhi sebelum sapuan dijalankan:
//! 1. Sudah [`DELAY_PERTAMA`] (60 detik) sejak proses ini boot — supaya
//!    startup tidak langsung dihajar sapuan sebelum apa pun lain siap.
//! 2. Sudah [`INTERVAL_SAPUAN`] (24 jam) sejak sapuan TERAKHIR yang
//!    benar-benar sukses (dari `settings`, bertahan lintas restart).
//!
//! Kegagalan satu sapuan dicatat `tracing::warn!` dan TIDAK memperbarui
//! penanda — sapuan berikutnya (tick 30 detik lagi) akan mencoba ulang,
//! bukan menunggu 24 jam lagi. Loop worker tidak pernah mati karena ini
//! (AGENTS.md: "loop latar belakang tidak boleh mati karena satu error").
use std::time::{Duration, Instant};

use crate::logs::retention;
use crate::state::AppState;

/// Jeda sejak boot sebelum sapuan pertama boleh berjalan (`docs/plan.md`,
/// "Angka yang dikunci": "sapuan pertama 60 detik setelah boot").
const DELAY_PERTAMA: Duration = Duration::from_secs(60);

/// Interval antar sapuan sukses (`docs/plan.md`, "Angka yang dikunci": 24 jam).
const INTERVAL_SAPUAN_SECS: i64 = 24 * 60 * 60;

const SETTINGS_KEY: &str = "log_retention_last_run_at";

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

async fn baca_last_run_at(state: &AppState) -> anyhow::Result<Option<i64>> {
    let row = sqlx::query!("SELECT value FROM settings WHERE key = ?", SETTINGS_KEY)
        .fetch_optional(&state.db_read)
        .await?;
    Ok(row.and_then(|r| r.value.parse::<i64>().ok()))
}

async fn tulis_last_run_at(state: &AppState, now: i64) -> anyhow::Result<()> {
    let now_str = now.to_string();
    sqlx::query!(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        SETTINGS_KEY,
        now_str,
    )
    .execute(&state.db_write)
    .await?;
    Ok(())
}

/// Dipanggil tiap tick worker (`worker::spawn`). `boot` adalah `Instant`
/// yang diambil SEKALI saat worker mulai — dipakai murni untuk syarat (1) di
/// atas, tidak pernah dipersist (restart proses = jeda 60 detik lagi, yang
/// memang diinginkan: tidak menghajar startup).
pub async fn jalankan_jika_jatuh_tempo(state: &AppState, boot: Instant) {
    if boot.elapsed() < DELAY_PERTAMA {
        return;
    }

    let now = now_epoch();
    match baca_last_run_at(state).await {
        Ok(Some(last_run)) if now - last_run < INTERVAL_SAPUAN_SECS => return,
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(
                error = %err,
                "gagal baca penanda sapuan retensi log terakhir — dicoba lagi tick berikutnya"
            );
            return;
        }
    }

    match retention::jalankan_sapuan(
        &state.db_read,
        &state.db_write,
        &state.config.log_dir,
        state.config.log_retention_days,
    )
    .await
    {
        Ok(ringkasan) => {
            tracing::info!(
                dihapus = ringkasan.dihapus,
                gagal_hapus_file = ringkasan.gagal_hapus_file,
                "sapuan retensi log selesai"
            );
            if let Err(err) = tulis_last_run_at(state, now).await {
                tracing::warn!(
                    error = %err,
                    "gagal menulis penanda sapuan retensi log — sapuan berikutnya bisa \
                     berjalan lebih cepat dari 24 jam, bukan masalah keamanan"
                );
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "sapuan retensi log gagal — dicoba lagi tick berikutnya, tidak menjatuhkan worker"
            );
        }
    }
}
