//! Persistensi `deployments` — `sqlx::query!` compile-time checked.

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;
use sqlx::SqlitePool;

use super::model::{DeploymentRingkas, StatusDeployment};

const ID_LEN: usize = 24;

/// `pub` (beda dari `servers`/`apps` repo yang generate id internal) —
/// `deployments::engine` butuh id SEBELUM baris dibuat, dipakai sekaligus
/// sebagai `apps.lock_token` (satu deployment = satu pemegang lock,
/// idnya sendiri sudah unik, tidak perlu token terpisah).
pub fn generate_id() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(ID_LEN)
        .map(char::from)
        .collect()
}

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

pub struct NewDeployment<'a> {
    pub app_id: &'a str,
    pub commit_sha: &'a str,
    pub git_ref: Option<&'a str>,
    pub image_digest: &'a str,
    /// `'api'` (CI lewat `POST /api/v1/deploy`) atau `'env'` (redeploy
    /// dipicu simpan env, `docs/plan.md` Fase 4). Kolom tanpa `CHECK` di
    /// skema (`migrations/0003_deploy.sql`), disiapkan eksplisit untuk
    /// nilai selain `'api'` sejak awal.
    pub trigger_source: &'a str,
    /// Versi env AKTIF app ini saat deployment dibuat — Fase 4 mengisi ini
    /// untuk KEDUA jalur (CI maupun env-save), `None` hanya kalau app
    /// belum pernah punya env sama sekali.
    pub env_version_id: Option<&'a str>,
}

/// Simpan deployment `queued` + job deploy dalam SATU transaksi
/// (`docs/plan.md` kontrak `POST /api/v1/deploy`: "INSERT deployments +
/// INSERT jobs, satu transaksi") — kegagalan salah satu tidak boleh
/// meninggalkan deployment yatim tanpa job yang menjalankannya, atau job
/// yang menunjuk deployment yang tidak pernah ada. `id` (deployment) dan
/// `job_id` SUDAH ditentukan pemanggil — `id` dipakai sekaligus sebagai
/// `apps.lock_token` (lihat `generate_id`).
///
/// `tx` opsional (Fase 4): kalau `Some`, pemanggil (`routes/apps.rs`
/// env_submit) sudah membuka transaksi yang JUGA memuat INSERT
/// `env_versions` — satu transaksi mencakup env+deployment+job sekaligus
/// (invariant §3 no.10). Kalau `None`, fungsi membuka transaksinya sendiri
/// seperti sebelumnya (jalur `POST /api/v1/deploy`, tidak menyentuh env).
pub async fn insert_queued_dengan_job(
    pool: &SqlitePool,
    id: &str,
    new: NewDeployment<'_>,
    job_id: &str,
    job_payload_json: &str,
) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi insert deployment+job")?;
    insert_queued_dengan_job_tx(&mut tx, id, new, job_id, job_payload_json).await?;
    tx.commit()
        .await
        .context("commit transaksi insert deployment+job")?;
    Ok(())
}

/// Inti `insert_queued_dengan_job`, menerima transaksi TERBUKA dari
/// pemanggil — dipakai langsung `routes/apps.rs` env_submit supaya INSERT
/// `env_versions` + `deployments` + `jobs` jadi satu transaksi.
pub async fn insert_queued_dengan_job_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    new: NewDeployment<'_>,
    job_id: &str,
    job_payload_json: &str,
) -> Result<()> {
    let now = now_epoch();

    sqlx::query!(
        "INSERT INTO deployments
            (id, app_id, commit_sha, git_ref, image_digest, status, trigger_source,
             env_version_id, heartbeat_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?)",
        id,
        new.app_id,
        new.commit_sha,
        new.git_ref,
        new.image_digest,
        new.trigger_source,
        new.env_version_id,
        now,
        now,
    )
    .execute(&mut **tx)
    .await
    .context("simpan deployment baru")?;

    sqlx::query!(
        "INSERT INTO jobs (id, kind, payload_json, status, run_at, attempts, created_at)
         VALUES (?, 'deploy', ?, 'queued', ?, 0, ?)",
        job_id,
        job_payload_json,
        now,
        now,
    )
    .execute(&mut **tx)
    .await
    .context("masukkan job deploy")?;

    Ok(())
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<DeploymentRingkas>> {
    let row = sqlx::query!(
        r#"SELECT id as "id!", app_id, commit_sha, git_ref, image_digest, status,
                  container_id, env_version_id, error_kind, error_detail, started_at, finished_at,
                  created_at
           FROM deployments WHERE id = ?"#,
        id
    )
    .fetch_optional(pool)
    .await
    .context("baca deployment")?;

    Ok(row.map(|r| DeploymentRingkas {
        id: r.id,
        app_id: r.app_id,
        commit_sha: r.commit_sha,
        git_ref: r.git_ref,
        image_digest: r.image_digest,
        status: StatusDeployment::from_db_str(&r.status),
        container_id: r.container_id,
        env_version_id: r.env_version_id,
        error_kind: r.error_kind,
        error_detail: r.error_detail,
        started_at: r.started_at,
        finished_at: r.finished_at,
        created_at: r.created_at,
    }))
}

pub async fn list_by_app(pool: &SqlitePool, app_id: &str) -> Result<Vec<DeploymentRingkas>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", app_id, commit_sha, git_ref, image_digest, status,
                  container_id, env_version_id, error_kind, error_detail, started_at, finished_at,
                  created_at
           FROM deployments WHERE app_id = ? ORDER BY created_at DESC"#,
        app_id
    )
    .fetch_all(pool)
    .await
    .context("baca riwayat deployment")?;

    Ok(rows
        .into_iter()
        .map(|r| DeploymentRingkas {
            id: r.id,
            app_id: r.app_id,
            commit_sha: r.commit_sha,
            git_ref: r.git_ref,
            image_digest: r.image_digest,
            status: StatusDeployment::from_db_str(&r.status),
            container_id: r.container_id,
            env_version_id: r.env_version_id,
            error_kind: r.error_kind,
            error_detail: r.error_detail,
            started_at: r.started_at,
            finished_at: r.finished_at,
            created_at: r.created_at,
        })
        .collect())
}

/// Transisi status murni (queued→pulling→starting→checking) — TIDAK
/// mengisi `finished_at` (hanya `mark_live`/`mark_failed` yang
/// menyelesaikan deployment). Selalu mengisi `heartbeat_at` dan
/// `started_at` kalau belum ada, supaya rekonsiliasi boot bisa mendeteksi
/// deployment aktif yang macet.
pub async fn set_status(pool: &SqlitePool, id: &str, status: StatusDeployment) -> Result<()> {
    let now = now_epoch();
    let status_str = status.as_db_str();
    sqlx::query!(
        "UPDATE deployments
         SET status = ?, heartbeat_at = ?, started_at = COALESCE(started_at, ?)
         WHERE id = ?",
        status_str,
        now,
        now,
        id,
    )
    .execute(pool)
    .await
    .context("set status deployment")?;
    Ok(())
}

pub async fn set_container_id(pool: &SqlitePool, id: &str, container_id: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE deployments SET container_id = ? WHERE id = ?",
        container_id,
        id,
    )
    .execute(pool)
    .await
    .context("simpan container_id deployment")?;
    Ok(())
}

/// Perbarui heartbeat dan perpanjang lease lock app dalam satu transaksi.
/// Tanpa perpanjangan lease, deployment yang sah bisa melewati TTL lalu
/// dianggap bebas oleh prune sementara worker-nya masih aktif.
pub async fn heartbeat(pool: &SqlitePool, deployment_id: &str) -> Result<bool> {
    const HEARTBEAT_LOCK_EXTENSION_SECS: i64 = 900;
    let now = now_epoch();
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi heartbeat deployment")?;
    let lock = sqlx::query!(
        "UPDATE apps SET lock_expires_at = MAX(lock_expires_at, ?)
         WHERE id = (SELECT app_id FROM deployments WHERE id = ?)
           AND lock_token = ?
           AND lock_expires_at IS NOT NULL
           AND lock_expires_at > ?",
        now + HEARTBEAT_LOCK_EXTENSION_SECS,
        deployment_id,
        deployment_id,
        now,
    )
    .execute(&mut *tx)
    .await
    .context("perpanjang lease lock deployment")?;
    if lock.rows_affected() != 1 {
        tx.rollback()
            .await
            .context("rollback heartbeat tanpa lock aktif")?;
        return Ok(false);
    }
    let heartbeat = sqlx::query!(
        "UPDATE deployments
         SET heartbeat_at = ?
         WHERE id = ?
           AND status IN ('queued', 'pulling', 'starting', 'checking')
           AND EXISTS (
               SELECT 1 FROM apps
               WHERE apps.id = deployments.app_id
                 AND apps.lock_token = deployments.id
                 AND apps.lock_expires_at > ?
           )",
        now,
        deployment_id,
        now,
    )
    .execute(&mut *tx)
    .await
    .context("perbarui heartbeat deployment")?;
    if heartbeat.rows_affected() != 1 {
        tx.rollback()
            .await
            .context("rollback heartbeat deployment tidak aktif")?;
        return Ok(false);
    }
    tx.commit()
        .await
        .context("commit heartbeat dan lease deployment")?;
    Ok(true)
}

/// Tandai deployment live hanya bila worker masih memegang lease app.
/// Commit ini adalah linearization point handoff: setelah berhasil, container
/// baru sudah menjadi sumber kebenaran dan cleanup tidak boleh membatalkannya.
pub async fn mark_live_if_owned(pool: &SqlitePool, id: &str) -> Result<bool> {
    let now = now_epoch();
    let result = sqlx::query!(
        "UPDATE deployments
         SET status = 'live', heartbeat_at = ?, finished_at = ?
         WHERE id = ?
           AND status IN ('queued', 'pulling', 'starting', 'checking')
           AND EXISTS (
               SELECT 1 FROM apps
               WHERE apps.id = deployments.app_id
                 AND apps.lock_token = deployments.id
                 AND apps.lock_expires_at > ?
           )",
        now,
        now,
        id,
        now,
    )
    .execute(pool)
    .await
    .context("tandai deployment live dengan lease aktif")?;
    Ok(result.rows_affected() == 1)
}

/// Batas `CHECK (length(error_detail) <= 500)` di skema — dipotong di sini
/// supaya pemanggil tidak perlu tahu detail constraint db.
fn truncate_error_detail(detail: &str) -> String {
    detail.chars().take(500).collect()
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: &str,
    error_kind: &str,
    error_detail: &str,
) -> Result<()> {
    let now = now_epoch();
    let error_detail = truncate_error_detail(error_detail);
    sqlx::query!(
        "UPDATE deployments
         SET status = 'failed', heartbeat_at = ?, finished_at = ?, error_kind = ?, error_detail = ?
         WHERE id = ?",
        now,
        now,
        error_kind,
        error_detail,
        id,
    )
    .execute(pool)
    .await
    .context("tandai deployment gagal")?;
    Ok(())
}

/// Deployment dengan status AKTIF (belum selesai) tapi heartbeat lebih tua
/// dari `staleness_secs` — kandidat `unknown` saat boot (`docs/plan.md`
/// Fase 2: "control plane restart di tengah deployment... status jadi
/// unknown, BUKAN ditebak").
pub async fn list_stale_active(
    pool: &SqlitePool,
    now: i64,
    staleness_secs: i64,
) -> Result<Vec<DeploymentRingkas>> {
    let ambang = now - staleness_secs;
    let rows = sqlx::query!(
        r#"SELECT id as "id!", app_id, commit_sha, git_ref, image_digest, status,
                  container_id, env_version_id, error_kind, error_detail, started_at, finished_at,
                  created_at
           FROM deployments
           WHERE status IN ('queued', 'pulling', 'starting', 'checking')
             AND (heartbeat_at IS NULL OR heartbeat_at < ?)"#,
        ambang
    )
    .fetch_all(pool)
    .await
    .context("baca deployment aktif dengan heartbeat basi")?;

    Ok(rows
        .into_iter()
        .map(|r| DeploymentRingkas {
            id: r.id,
            app_id: r.app_id,
            commit_sha: r.commit_sha,
            git_ref: r.git_ref,
            image_digest: r.image_digest,
            status: StatusDeployment::from_db_str(&r.status),
            container_id: r.container_id,
            env_version_id: r.env_version_id,
            error_kind: r.error_kind,
            error_detail: r.error_detail,
            started_at: r.started_at,
            finished_at: r.finished_at,
            created_at: r.created_at,
        })
        .collect())
}

/// Deployment `live` TERAKHIR untuk `app_id`, KECUALI `exclude_id` (deployment
/// yang sedang berjalan sekarang) — ini "container lama" yang di-drain
/// setelah container baru terbukti sehat.
pub async fn find_current_live(
    pool: &SqlitePool,
    app_id: &str,
    exclude_id: &str,
) -> Result<Option<DeploymentRingkas>> {
    let row = sqlx::query!(
        r#"SELECT id as "id!", app_id, commit_sha, git_ref, image_digest, status,
                  container_id, env_version_id, error_kind, error_detail, started_at, finished_at,
                  created_at
           FROM deployments
           WHERE app_id = ? AND status = 'live' AND id != ?
           ORDER BY created_at DESC LIMIT 1"#,
        app_id,
        exclude_id,
    )
    .fetch_optional(pool)
    .await
    .context("cari deployment live sebelumnya")?;

    Ok(row.map(|r| DeploymentRingkas {
        id: r.id,
        app_id: r.app_id,
        commit_sha: r.commit_sha,
        git_ref: r.git_ref,
        image_digest: r.image_digest,
        status: StatusDeployment::from_db_str(&r.status),
        container_id: r.container_id,
        env_version_id: r.env_version_id,
        error_kind: r.error_kind,
        error_detail: r.error_detail,
        started_at: r.started_at,
        finished_at: r.finished_at,
        created_at: r.created_at,
    }))
}

pub async fn mark_unknown(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query!("UPDATE deployments SET status = 'unknown' WHERE id = ?", id)
        .execute(pool)
        .await
        .context("tandai deployment unknown")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_error_detail_memotong_ke_500_karakter() {
        let panjang = "x".repeat(1000);
        assert_eq!(truncate_error_detail(&panjang).chars().count(), 500);
    }
}
