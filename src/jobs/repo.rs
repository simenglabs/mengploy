//! Antrean `jobs` — tabel SQLite polos, tanpa crate queue eksternal
//! (CLAUDE.md §4: "~80 baris"). Satu jenis job Fase 2: `"deploy"`.

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;
use sqlx::SqlitePool;

const ID_LEN: usize = 24;

fn generate_id() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(ID_LEN)
        .map(char::from)
        .collect()
}

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

pub struct Job {
    pub id: String,
    pub kind: String,
    pub payload_json: String,
    pub attempts: i64,
}

/// Masukkan job baru, siap diklaim segera (`run_at = now`). Fase 2 tidak
/// butuh delay/retry terjadwal — kolom `run_at` disiapkan untuk itu di masa
/// depan, tidak dipakai selain "sekarang" di fase ini.
pub async fn enqueue(pool: &SqlitePool, kind: &str, payload_json: &str) -> Result<String> {
    let id = generate_id();
    let now = now_epoch();

    sqlx::query!(
        "INSERT INTO jobs (id, kind, payload_json, status, run_at, attempts, created_at)
         VALUES (?, ?, ?, 'queued', ?, 0, ?)",
        id,
        kind,
        payload_json,
        now,
        now,
    )
    .execute(pool)
    .await
    .context("masukkan job baru")?;

    Ok(id)
}

/// Klaim SATU job `kind` tertua yang jatuh tempo (`run_at <= now`) dan
/// masih `queued`. Transaksi tunggal (select + update) — pool tulis satu
/// koneksi sudah menyerialkan ini secara alami, transaksi di sini murni
/// kebenaran eksplisit, bukan penambal race (Fase 2 hanya satu worker
/// deploy in-process).
pub async fn claim_next(pool: &SqlitePool, kind: &str) -> Result<Option<Job>> {
    let now = now_epoch();
    let mut tx = pool.begin().await.context("mulai transaksi klaim job")?;

    let kandidat = sqlx::query!(
        r#"SELECT id as "id!", payload_json, attempts FROM jobs
           WHERE kind = ? AND status = 'queued' AND run_at <= ?
           ORDER BY run_at ASC LIMIT 1"#,
        kind,
        now,
    )
    .fetch_optional(&mut *tx)
    .await
    .context("cari job jatuh tempo")?;

    let Some(kandidat) = kandidat else {
        tx.commit()
            .await
            .context("commit transaksi klaim job (kosong)")?;
        return Ok(None);
    };

    let attempts_baru = kandidat.attempts + 1;
    sqlx::query!(
        "UPDATE jobs SET status = 'running', started_at = ?, attempts = ?
         WHERE id = ? AND status = 'queued'",
        now,
        attempts_baru,
        kandidat.id,
    )
    .execute(&mut *tx)
    .await
    .context("tandai job running")?;

    tx.commit().await.context("commit transaksi klaim job")?;

    Ok(Some(Job {
        id: kandidat.id,
        kind: kind.to_string(),
        payload_json: kandidat.payload_json,
        attempts: attempts_baru,
    }))
}

pub async fn mark_done(pool: &SqlitePool, job_id: &str) -> Result<()> {
    sqlx::query!("UPDATE jobs SET status = 'done' WHERE id = ?", job_id)
        .execute(pool)
        .await
        .context("tandai job selesai")?;
    Ok(())
}

pub async fn mark_failed(pool: &SqlitePool, job_id: &str, error: &str) -> Result<()> {
    let error_pendek: String = error.chars().take(500).collect();
    sqlx::query!(
        "UPDATE jobs SET status = 'failed', last_error = ? WHERE id = ?",
        error_pendek,
        job_id,
    )
    .execute(pool)
    .await
    .context("tandai job gagal")?;
    Ok(())
}
