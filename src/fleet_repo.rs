//! Persistensi metadata operasi armada. Baris output tidak pernah masuk
//! SQLite; hanya path file privat dan metadata hasil yang disimpan.

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;
use sqlx::SqlitePool;
use std::path::Path;

use crate::fleet::{
    DiskSummary, FleetOperationKind, FleetOperationResultSummary, FleetOperationSummary,
    FleetResultStatus,
};

const ID_LEN: usize = 24;

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

pub async fn insert_operation(
    pool: &SqlitePool,
    operation_id: &str,
    kind: FleetOperationKind,
    targets_json: &str,
    payload_json: &str,
) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "INSERT INTO fleet_operations (id, kind, targets, status, created_at, payload_json)
         VALUES (?, ?, ?, 'queued', ?, ?)",
        operation_id,
        kind.as_db_str(),
        targets_json,
        now,
        payload_json,
    )
    .execute(pool)
    .await
    .context("simpan operasi armada")?;
    Ok(())
}

pub async fn insert_operation_with_results(
    pool: &SqlitePool,
    operation_id: &str,
    kind: FleetOperationKind,
    targets_json: &str,
    payload_json: &str,
) -> Result<()> {
    insert_operation(pool, operation_id, kind, targets_json, payload_json).await
}

pub async fn set_status(pool: &SqlitePool, operation_id: &str, status: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE fleet_operations SET status = ? WHERE id = ?",
        status,
        operation_id,
    )
    .execute(pool)
    .await
    .context("perbarui status operasi armada")?;
    Ok(())
}

/// Simpan hasil hanya jika path berada di direktori operasi pada root
/// konfigurasi aktual. Validasi ini sengaja berada di boundary persistensi,
/// bukan hanya di handler HTTP.
pub async fn insert_result(
    pool: &SqlitePool,
    output_root: &Path,
    operation_id: &str,
    server_id: &str,
    exit_code: Option<i64>,
    output_path: Option<&str>,
    status: FleetResultStatus,
) -> Result<()> {
    if let Some(path) = output_path {
        let candidate = Path::new(path);
        let operation_dir = output_root.join(operation_id);
        if !candidate.is_absolute()
            || candidate
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            || !candidate.starts_with(&operation_dir)
            || candidate.file_name().is_none()
        {
            return Err(anyhow::anyhow!("path output operasi tidak aman"));
        }
    }
    simpan_result_query(
        pool,
        operation_id,
        server_id,
        exit_code,
        output_path,
        status,
    )
    .await
}

async fn simpan_result_query(
    pool: &SqlitePool,
    operation_id: &str,
    server_id: &str,
    exit_code: Option<i64>,
    output_path: Option<&str>,
    status: FleetResultStatus,
) -> Result<()> {
    sqlx::query!(
        "INSERT OR REPLACE INTO fleet_operation_results
         (operation_id, server_id, exit_code, output_path, status)
         VALUES (?, ?, ?, ?, ?)",
        operation_id,
        server_id,
        exit_code,
        output_path,
        status.as_db_str(),
    )
    .execute(pool)
    .await
    .context("simpan hasil operasi per server")?;
    Ok(())
}

pub async fn list_operations(pool: &SqlitePool) -> Result<Vec<FleetOperationSummary>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", kind, targets, status, created_at
           FROM fleet_operations ORDER BY created_at DESC LIMIT 50"#
    )
    .fetch_all(pool)
    .await
    .context("baca riwayat operasi armada")?;

    rows.into_iter()
        .map(|row| {
            let targets =
                serde_json::from_str(&row.targets).context("target operasi armada rusak")?;
            Ok(FleetOperationSummary {
                id: row.id,
                kind: row.kind,
                status: row.status,
                targets,
                created_at: row.created_at,
            })
        })
        .collect::<Result<Vec<_>>>()
}

pub async fn find_operation(
    pool: &SqlitePool,
    operation_id: &str,
) -> Result<Option<FleetOperationSummary>> {
    let row = sqlx::query!(
        r#"SELECT id as "id!", kind, targets, status, created_at
           FROM fleet_operations WHERE id = ?"#,
        operation_id,
    )
    .fetch_optional(pool)
    .await
    .context("baca operasi armada")?;
    let Some(row) = row else {
        return Ok(None);
    };
    let targets = serde_json::from_str(&row.targets).context("target operasi armada rusak")?;
    Ok(Some(FleetOperationSummary {
        id: row.id,
        kind: row.kind,
        targets,
        status: row.status,
        created_at: row.created_at,
    }))
}

pub async fn list_results(
    pool: &SqlitePool,
    operation_id: &str,
) -> Result<Vec<FleetOperationResultSummary>> {
    let rows = sqlx::query!(
        r#"SELECT operation_id, server_id, exit_code, output_path, status
           FROM fleet_operation_results WHERE operation_id = ? ORDER BY server_id ASC"#,
        operation_id,
    )
    .fetch_all(pool)
    .await
    .context("baca hasil operasi armada")?;
    Ok(rows
        .into_iter()
        .map(|row| FleetOperationResultSummary {
            operation_id: row.operation_id,
            server_id: row.server_id,
            exit_code: row.exit_code,
            output_path: row.output_path,
            status: row.status,
        })
        .collect())
}

pub async fn list_disk(pool: &SqlitePool) -> Result<Vec<DiskSummary>> {
    let rows = sqlx::query!(
        r#"SELECT s.id as "server_id!", s.name as "server_name!", s.status,
                  m.disk_used, m.disk_total, m.ts
           FROM servers s
           LEFT JOIN metrics_host m ON m.server_id = s.id AND m.res = 'min'
             AND m.ts = (SELECT MAX(m2.ts) FROM metrics_host m2
                         WHERE m2.server_id = s.id AND m2.res = 'min')
           ORDER BY s.name ASC"#
    )
    .fetch_all(pool)
    .await
    .context("baca ringkasan disk armada")?;
    Ok(rows
        .into_iter()
        .map(|row| DiskSummary {
            server_id: row.server_id,
            server_name: row.server_name,
            status: row.status,
            used_bytes: row.disk_used,
            total_bytes: row.disk_total,
            sampled_at: row.ts,
        })
        .collect())
}
