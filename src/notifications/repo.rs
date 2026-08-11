use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::crypto::CryptoKey;

use super::model::NotificationSettings;

const KEY_ENABLED: &str = "notification_webhook_enabled";
const KEY_URL: &str = "notification_webhook_url_encrypted";
const KEY_SECRET: &str = "notification_webhook_secret_encrypted";
const KEY_EVENTS: &str = "notification_webhook_events";

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

async fn baca(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row = sqlx::query!("SELECT value FROM settings WHERE key = ?", key)
        .fetch_optional(pool)
        .await
        .context("baca pengaturan notifikasi")?;
    Ok(row.map(|row| row.value))
}

pub async fn load_settings(pool: &SqlitePool, crypto: &CryptoKey) -> Result<NotificationSettings> {
    let enabled = baca(pool, KEY_ENABLED).await?.as_deref() == Some("1");
    let url = match baca(pool, KEY_URL).await? {
        Some(value) => Some(crypto.decrypt(&value).context("dekripsi URL webhook")?),
        None => None,
    };
    let secret = match baca(pool, KEY_SECRET).await? {
        Some(value) => Some(crypto.decrypt(&value).context("dekripsi secret webhook")?),
        None => None,
    };
    let events = baca(pool, KEY_EVENTS)
        .await?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .context("baca event webhook")?
        .unwrap_or_default();
    Ok(NotificationSettings {
        enabled,
        url,
        secret,
        events,
    })
}

pub async fn save_settings(
    pool: &SqlitePool,
    crypto: &CryptoKey,
    settings: &NotificationSettings,
) -> Result<()> {
    let url = settings
        .url
        .as_deref()
        .map(|value| crypto.encrypt(value))
        .transpose()
        .context("enkripsi URL webhook")?;
    let secret = settings
        .secret
        .as_deref()
        .map(|value| crypto.encrypt(value))
        .transpose()
        .context("enkripsi secret webhook")?;
    let events = serde_json::to_string(&settings.events).context("serialisasi event webhook")?;
    for (key, value) in [
        (
            KEY_ENABLED,
            Some(if settings.enabled { "1" } else { "0" }.to_string()),
        ),
        (KEY_URL, url),
        (KEY_SECRET, secret),
        (KEY_EVENTS, Some(events)),
    ] {
        if let Some(value) = value {
            sqlx::query!(
                "INSERT INTO settings (key, value) VALUES (?, ?)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                key,
                value,
            )
            .execute(pool)
            .await
            .context("simpan pengaturan notifikasi")?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Delivery {
    pub id: String,
    pub event_type: String,
    pub payload_json: String,
    pub attempts: i64,
}

pub async fn claim_next(pool: &SqlitePool) -> Result<Option<Delivery>> {
    let now = now_epoch();
    let mut tx = pool.begin().await.context("mulai klaim delivery webhook")?;
    let row = sqlx::query!(
        r#"SELECT id as "id!", event_type, payload_json, attempts
           FROM notification_deliveries
           WHERE status = 'queued' AND next_attempt_at <= ?
           ORDER BY next_attempt_at ASC, created_at ASC LIMIT 1"#,
        now,
    )
    .fetch_optional(&mut *tx)
    .await
    .context("cari delivery webhook jatuh tempo")?;
    let Some(row) = row else {
        tx.commit().await.context("commit klaim kosong")?;
        return Ok(None);
    };
    let attempts = row.attempts + 1;
    let updated = sqlx::query!(
        "UPDATE notification_deliveries SET status = 'sending', attempts = ?
         WHERE id = ? AND status = 'queued'",
        attempts,
        row.id,
    )
    .execute(&mut *tx)
    .await
    .context("tandai delivery webhook sedang dikirim")?;
    tx.commit().await.context("commit klaim delivery webhook")?;
    if updated.rows_affected() != 1 {
        return Ok(None);
    }
    Ok(Some(Delivery {
        id: row.id,
        event_type: row.event_type,
        payload_json: row.payload_json,
        attempts,
    }))
}

pub fn retry_delay_secs(attempts: i64) -> i64 {
    2_i64.saturating_pow(attempts.clamp(1, 8) as u32).min(3600)
}

pub async fn mark_retry(
    pool: &SqlitePool,
    id: &str,
    error_kind: &str,
    attempts: i64,
) -> Result<()> {
    let next = now_epoch() + retry_delay_secs(attempts);
    sqlx::query!(
        "UPDATE notification_deliveries
         SET status = 'queued', next_attempt_at = ?, last_error_kind = ?
         WHERE id = ? AND status = 'sending'",
        next,
        error_kind,
        id,
    )
    .execute(pool)
    .await
    .context("jadwalkan ulang delivery webhook")?;
    Ok(())
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: &str,
    error_kind: &str,
    _retryable: bool,
) -> Result<()> {
    sqlx::query!(
        "UPDATE notification_deliveries
         SET status = 'failed', last_error_kind = ?
         WHERE id = ? AND status = 'sending'",
        error_kind,
        id,
    )
    .execute(pool)
    .await
    .context("tandai delivery webhook gagal")?;
    Ok(())
}

pub async fn mark_delivered(pool: &SqlitePool, id: &str, status_code: i64) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "UPDATE notification_deliveries
         SET status = 'delivered', delivered_at = ?, last_status_code = ?
         WHERE id = ? AND status = 'sending'",
        now,
        status_code,
        id,
    )
    .execute(pool)
    .await
    .context("tandai delivery webhook berhasil")?;
    Ok(())
}

pub async fn enqueue(
    pool: &SqlitePool,
    id: &str,
    event_id: &str,
    event_type: &str,
    app_id: Option<&str>,
    payload_json: &str,
) -> Result<bool> {
    if !super::ALLOWED_EVENTS.contains(&event_type) {
        anyhow::bail!("event notifikasi tidak diizinkan");
    }
    validate_payload(event_type, payload_json)?;
    let now = now_epoch();
    let result = sqlx::query!(
        "INSERT INTO notification_deliveries
            (id, event_id, event_type, app_id, payload_json, status, attempts,
             next_attempt_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'queued', 0, ?, ?)
         ON CONFLICT (event_id, event_type) DO NOTHING",
        id,
        event_id,
        event_type,
        app_id,
        payload_json,
        now,
        now,
    )
    .execute(pool)
    .await
    .context("masukkan delivery notifikasi")?;
    Ok(result.rows_affected() == 1)
}

fn validate_payload(event_type: &str, payload_json: &str) -> Result<()> {
    if payload_json.len() > 256 * 1024 {
        anyhow::bail!("payload notifikasi terlalu besar");
    }
    let value: serde_json::Value =
        serde_json::from_str(payload_json).context("parse payload notifikasi")?;
    let object = value
        .as_object()
        .context("payload notifikasi harus berupa object")?;
    if object.get("event_type").and_then(serde_json::Value::as_str) != Some(event_type)
        || object
            .get("event_id")
            .and_then(serde_json::Value::as_str)
            .is_none()
        || object
            .get("occurred_at")
            .and_then(serde_json::Value::as_i64)
            .is_none()
        || !object.contains_key("data")
    {
        anyhow::bail!("payload notifikasi tidak sesuai envelope");
    }
    let data = object
        .get("data")
        .and_then(serde_json::Value::as_object)
        .context("data payload notifikasi harus berupa object")?;
    let allowed = match event_type {
        "deployment.failed" | "deployment.recovered" => {
            ["deployment_id", "app_id", "status", "error_kind"].as_slice()
        }
        "reconciliation.drift_detected" => ["server_id", "app_id", "kind", "observed"].as_slice(),
        "reconciliation.drift_resolved" => ["server_id", "app_id", "kind", "observed"].as_slice(),
        _ => &[],
    };
    if data.keys().any(|key| !allowed.contains(&key.as_str())) {
        anyhow::bail!("field data notifikasi tidak diizinkan");
    }
    if data
        .get("observed")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|observed| {
            observed
                .keys()
                .any(|key| !["container_id", "digest", "resolved_count"].contains(&key.as_str()))
        })
    {
        anyhow::bail!("metadata observed notifikasi tidak diizinkan");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn event_id_tidak_mengandung_nilai_rahasia() {
        let event_id = "deployment-opaque:failed";
        assert!(!event_id.contains("password"));
        assert!(!event_id.contains("secret"));
    }
}
