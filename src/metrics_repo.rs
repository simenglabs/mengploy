//! Repository SQLite metrik Fase 6. Semua tulisan satu siklus dan rollup
//! dibungkus transaksi tunggal.

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::metrics::{
    AlertSummary, AlertWrite, ContainerMetricPoint, ContainerMetricWrite, DeploymentMarker,
    HostMetricPoint, HostMetricWrite, MetricDashboard, RETENSI_HOUR_SECS, RETENSI_MIN_SECS,
    RETENSI_RAW_SECS,
};

pub async fn insert_cycle(
    pool: &SqlitePool,
    now: i64,
    server_ids: &[&str],
    hosts: &[HostMetricWrite<'_>],
    containers: &[ContainerMetricWrite<'_>],
    alerts: &[AlertWrite<'_>],
) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi simpan metrik")?;
    for host in hosts {
        sqlx::query!(
            "INSERT OR REPLACE INTO metrics_host
             (res, ts, server_id, cpu_avg, cpu_max, mem_used, mem_max, mem_total,
              load1, disk_used, disk_total, source)
             VALUES ('raw', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'ssh')",
            now,
            host.server_id,
            host.sample.cpu_percent,
            host.sample.cpu_percent,
            host.sample.mem_used,
            host.sample.mem_used,
            host.sample.mem_total,
            host.sample.load1,
            host.sample.disk_used,
            host.sample.disk_total,
        )
        .execute(&mut *tx)
        .await
        .context("simpan metrik host")?;
    }
    for container in containers {
        sqlx::query!(
            "INSERT OR REPLACE INTO metrics_container
             (res, ts, server_id, container_id, app_id, cpu_avg, cpu_max, mem_bytes,
              mem_max, mem_limit, net_rx, net_tx, restart_count, source)
             VALUES ('raw', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'docker')",
            now,
            container.server_id,
            container.container_id,
            container.app_id,
            container.sample.cpu_percent,
            container.sample.cpu_percent,
            container.sample.mem_bytes,
            container.sample.mem_max,
            container.sample.mem_limit,
            container.sample.net_rx,
            container.sample.net_tx,
            container.sample.restart_count,
        )
        .execute(&mut *tx)
        .await
        .context("simpan metrik container")?;
    }
    for alert in alerts {
        let alert_id = format!(
            "{}:{}:{}",
            alert.server_id,
            alert.kind.as_db_str(),
            alert.target
        );
        sqlx::query!(
            "INSERT INTO metric_alerts
             (id, server_id, app_id, container_id, deployment_id, kind, severity, target,
              message, status, first_seen_at, last_seen_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
             ON CONFLICT (server_id, kind, target) DO UPDATE SET
                app_id = excluded.app_id, container_id = excluded.container_id,
                deployment_id = excluded.deployment_id, severity = excluded.severity,
                message = excluded.message, status = 'active', last_seen_at = excluded.last_seen_at,
                resolved_at = NULL",
            alert_id,
            alert.server_id,
            alert.app_id,
            alert.container_id,
            alert.deployment_id,
            alert.kind.as_db_str(),
            alert.severity,
            alert.target,
            alert.message,
            now,
            now,
        )
        .execute(&mut *tx)
        .await
        .context("simpan alert metrik")?;
    }
    for server_id in server_ids {
        sqlx::query!(
            "UPDATE metric_alerts SET status = 'resolved', resolved_at = ?
             WHERE server_id = ? AND status = 'active' AND last_seen_at < ?",
            now,
            server_id,
            now,
        )
        .execute(&mut *tx)
        .await
        .context("pulihkan alert metrik yang sudah reda")?;
    }
    tx.commit()
        .await
        .context("commit transaksi simpan metrik")?;
    Ok(())
}

pub async fn rollup_and_retain(pool: &SqlitePool, now: i64) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi rollup metrik")?;
    sqlx::query!(
        "INSERT OR REPLACE INTO metrics_host
         (res, ts, server_id, cpu_avg, cpu_max, mem_used, mem_max, mem_total, load1,
          disk_used, disk_total, source)
         SELECT 'min', (ts / 60) * 60, server_id, AVG(cpu_avg), MAX(cpu_max), AVG(mem_used),
                MAX(mem_max), MAX(mem_total), AVG(load1), AVG(disk_used), MAX(disk_total), 'rollup'
         FROM metrics_host WHERE res = 'raw' GROUP BY (ts / 60), server_id"
    )
    .execute(&mut *tx)
    .await
    .context("rollup metrik host menit")?;
    sqlx::query!(
        "INSERT OR REPLACE INTO metrics_container
         (res, ts, server_id, container_id, app_id, cpu_avg, cpu_max, mem_bytes, mem_max,
          mem_limit, net_rx, net_tx, restart_count, source)
         SELECT 'min', (ts / 60) * 60, server_id, container_id, MAX(app_id), AVG(cpu_avg),
                MAX(cpu_max), AVG(mem_bytes), MAX(mem_max), MAX(mem_limit), MAX(net_rx),
                MAX(net_tx), MAX(restart_count), 'rollup'
         FROM metrics_container WHERE res = 'raw'
         GROUP BY (ts / 60), server_id, container_id"
    )
    .execute(&mut *tx)
    .await
    .context("rollup metrik container menit")?;
    sqlx::query!(
        "INSERT OR REPLACE INTO metrics_host
         (res, ts, server_id, cpu_avg, cpu_max, mem_used, mem_max, mem_total, load1,
          disk_used, disk_total, source)
         SELECT 'hour', (ts / 3600) * 3600, server_id, AVG(cpu_avg), MAX(cpu_max), AVG(mem_used),
                MAX(mem_used), MAX(mem_total), AVG(load1), AVG(disk_used), MAX(disk_total), 'rollup'
         FROM metrics_host WHERE res = 'min' GROUP BY (ts / 3600), server_id"
    )
    .execute(&mut *tx)
    .await
    .context("rollup metrik host jam")?;
    sqlx::query!(
        "INSERT OR REPLACE INTO metrics_container
         (res, ts, server_id, container_id, app_id, cpu_avg, cpu_max, mem_bytes, mem_max,
          mem_limit, net_rx, net_tx, restart_count, source)
         SELECT 'hour', (ts / 3600) * 3600, server_id, container_id, MAX(app_id), AVG(cpu_avg),
                MAX(cpu_max), AVG(mem_bytes), MAX(mem_max), MAX(mem_limit), MAX(net_rx),
                MAX(net_tx), MAX(restart_count), 'rollup'
         FROM metrics_container WHERE res = 'min'
         GROUP BY (ts / 3600), server_id, container_id"
    )
    .execute(&mut *tx)
    .await
    .context("rollup metrik container jam")?;

    sqlx::query!(
        "DELETE FROM metrics_host WHERE res = 'raw' AND ts < ?",
        now - RETENSI_RAW_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik host mentah")?;
    sqlx::query!(
        "DELETE FROM metrics_host WHERE res = 'min' AND ts < ?",
        now - RETENSI_MIN_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik host menit")?;
    sqlx::query!(
        "DELETE FROM metrics_host WHERE res = 'hour' AND ts < ?",
        now - RETENSI_HOUR_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik host jam")?;
    sqlx::query!(
        "DELETE FROM metrics_container WHERE res = 'raw' AND ts < ?",
        now - RETENSI_RAW_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik container mentah")?;
    sqlx::query!(
        "DELETE FROM metrics_container WHERE res = 'min' AND ts < ?",
        now - RETENSI_MIN_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik container menit")?;
    sqlx::query!(
        "DELETE FROM metrics_container WHERE res = 'hour' AND ts < ?",
        now - RETENSI_HOUR_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik container jam")?;
    sqlx::query!(
        "DELETE FROM metrics_container_legacy WHERE res = 'raw' AND ts < ?",
        now - RETENSI_RAW_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik container legacy mentah")?;
    sqlx::query!(
        "DELETE FROM metrics_container_legacy WHERE res = 'min' AND ts < ?",
        now - RETENSI_MIN_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik container legacy menit")?;
    sqlx::query!(
        "DELETE FROM metrics_container_legacy WHERE res = 'hour' AND ts < ?",
        now - RETENSI_HOUR_SECS
    )
    .execute(&mut *tx)
    .await
    .context("retensi metrik container legacy jam")?;
    tx.commit()
        .await
        .context("commit transaksi rollup metrik")?;
    Ok(())
}

pub async fn baseline_before_deployment(
    pool: &SqlitePool,
    app_id: &str,
    deployment_started_at: i64,
) -> Result<Option<(f64, f64)>> {
    let since = deployment_started_at - 10 * 60;
    let row = sqlx::query!(
        "SELECT AVG(cpu_avg) as cpu_avg, AVG(mem_bytes) as mem_bytes
         FROM metrics_container
         WHERE app_id = ? AND res = 'raw' AND ts >= ? AND ts < ?",
        app_id,
        since,
        deployment_started_at,
    )
    .fetch_one(pool)
    .await
    .context("baca baseline resource sebelum deployment")?;
    Ok(row.cpu_avg.zip(row.mem_bytes.map(|value| value as f64)))
}

pub async fn dashboard(pool: &SqlitePool, server_id: &str, since: i64) -> Result<MetricDashboard> {
    let host_rows = sqlx::query!(
        "SELECT ts, cpu_avg, cpu_max, mem_used, mem_total, load1, disk_used, disk_total
         FROM metrics_host WHERE server_id = ? AND res = 'min' AND ts >= ? ORDER BY ts ASC",
        server_id,
        since,
    )
    .fetch_all(pool)
    .await
    .context("baca metrik host untuk dashboard")?;
    let container_rows = sqlx::query!(
        "SELECT ts, server_id, container_id, app_id, cpu_avg, cpu_max, mem_bytes, mem_max,
                mem_limit, net_rx, net_tx, restart_count
         FROM metrics_container WHERE server_id = ? AND res = 'min' AND ts >= ?
         ORDER BY ts ASC",
        server_id,
        since,
    )
    .fetch_all(pool)
    .await
    .context("baca metrik container untuk dashboard")?;
    let deployments = sqlx::query!(
        "SELECT COALESCE(d.started_at, d.created_at) as ts, a.name as app_name
         FROM deployments d JOIN apps a ON a.id = d.app_id
         WHERE a.server_id = ? AND COALESCE(d.started_at, d.created_at) >= ?
         ORDER BY ts ASC",
        server_id,
        since,
    )
    .fetch_all(pool)
    .await
    .context("baca penanda deployment untuk dashboard")?;
    let alerts = sqlx::query!(
        "SELECT kind, severity, target, message FROM metric_alerts
         WHERE server_id = ? AND status = 'active' ORDER BY last_seen_at DESC",
        server_id,
    )
    .fetch_all(pool)
    .await
    .context("baca alert metrik untuk dashboard")?;

    Ok(MetricDashboard {
        host: host_rows
            .into_iter()
            .map(|row| HostMetricPoint {
                ts: row.ts,
                cpu_avg: row.cpu_avg,
                cpu_max: row.cpu_max,
                mem_used: row.mem_used,
                mem_total: row.mem_total,
                load1: row.load1,
                disk_used: row.disk_used,
                disk_total: row.disk_total,
            })
            .collect(),
        containers: container_rows
            .into_iter()
            .map(|row| ContainerMetricPoint {
                ts: row.ts,
                server_id: row.server_id,
                container_id: row.container_id,
                app_id: row.app_id,
                cpu_avg: row.cpu_avg,
                cpu_max: row.cpu_max,
                mem_bytes: row.mem_bytes,
                mem_max: row.mem_max,
                mem_limit: row.mem_limit,
                net_rx: row.net_rx,
                net_tx: row.net_tx,
                restart_count: row.restart_count,
            })
            .collect(),
        deployments: deployments
            .into_iter()
            .map(|row| DeploymentMarker {
                ts: row.ts,
                label: row.app_name,
            })
            .collect(),
        alerts: alerts
            .into_iter()
            .map(|row| AlertSummary {
                kind: row.kind,
                severity: row.severity,
                target: row.target,
                message: row.message,
            })
            .collect(),
    })
}
