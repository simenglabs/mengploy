//! Persistensi `registries` dan `server_registries` (join). Sama pola dua
//! pool seperti `servers::repo`.

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

/// Baris registry LENGKAP termasuk `token_encrypted` — HANYA dipakai
/// internal (`servers::verify`, sebelum `docker login`). Dekripsi terjadi
/// di pemanggil lewat `CryptoKey`, bukan di sini. TIDAK PERNAH diekspor ke
/// `src/web/` (invariant 7); lihat `RegistryRingkas` untuk view-model aman.
pub struct RegistryRow {
    pub id: String,
    pub host: String,
    pub username: String,
    pub token_encrypted: String,
}

pub async fn find_by_id(pool: &SqlitePool, registry_id: &str) -> Result<Option<RegistryRow>> {
    let row = sqlx::query!(
        r#"SELECT id as "id!", host, username, token_encrypted FROM registries WHERE id = ?"#,
        registry_id
    )
    .fetch_optional(pool)
    .await
    .context("baca registry")?;

    Ok(row.map(|r| RegistryRow {
        id: r.id,
        host: r.host,
        username: r.username,
        token_encrypted: r.token_encrypted,
    }))
}

/// Registry yang login-nya sudah tercatat di `server_id` DAN host-nya cocok
/// `image_host` — dipakai `deployments::engine` untuk resolusi kredensial
/// pull image OTOMATIS (server target sudah `docker login` di Fase 1,
/// engine tidak menebak kredensial mana yang cocok, ia mencocokkan lewat
/// `server_registries`).
pub async fn find_for_server_by_host(
    pool: &SqlitePool,
    server_id: &str,
    image_host: &str,
) -> Result<Option<RegistryRow>> {
    let row = sqlx::query!(
        r#"SELECT r.id as "id!", r.host, r.username, r.token_encrypted
           FROM registries r
           INNER JOIN server_registries sr ON sr.registry_id = r.id
           WHERE sr.server_id = ? AND r.host = ?
           LIMIT 1"#,
        server_id,
        image_host,
    )
    .fetch_optional(pool)
    .await
    .context("cari registry terpasang untuk host image")?;

    Ok(row.map(|r| RegistryRow {
        id: r.id,
        host: r.host,
        username: r.username,
        token_encrypted: r.token_encrypted,
    }))
}

/// Ringkasan registry — host+username saja, TIDAK PERNAH `token_encrypted`
/// (invariant 7). Dipakai daftar "registry tersimpan untuk dipakai ulang"
/// (wizard langkah 3) dan "registry tertaut" (detail server).
pub struct RegistryRingkas {
    pub id: String,
    pub host: String,
    pub username: String,
}

/// Seluruh registry tersimpan — dipakai wizard langkah 3 untuk daftar
/// "pakai ulang registry yang sudah ada".
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<RegistryRingkas>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", host, username FROM registries ORDER BY host ASC, username ASC"#
    )
    .fetch_all(pool)
    .await
    .context("baca daftar registry")?;

    Ok(rows
        .into_iter()
        .map(|r| RegistryRingkas {
            id: r.id,
            host: r.host,
            username: r.username,
        })
        .collect())
}

/// Registry yang tertaut ke satu server (`server_registries`) — dipakai
/// halaman detail server.
pub async fn list_linked(pool: &SqlitePool, server_id: &str) -> Result<Vec<RegistryRingkas>> {
    let rows = sqlx::query!(
        r#"SELECT r.id as "id!", r.host, r.username
           FROM registries r
           INNER JOIN server_registries sr ON sr.registry_id = r.id
           WHERE sr.server_id = ?
           ORDER BY r.host ASC, r.username ASC"#,
        server_id
    )
    .fetch_all(pool)
    .await
    .context("baca registry tertaut server")?;

    Ok(rows
        .into_iter()
        .map(|r| RegistryRingkas {
            id: r.id,
            host: r.host,
            username: r.username,
        })
        .collect())
}

/// Catat `docker login` berhasil di `server_id` untuk `registry_id`.
/// `INSERT ... ON CONFLICT` idempoten terhadap PK gabungan
/// `server_registries`.
pub async fn record_login_success(
    pool: &SqlitePool,
    server_id: &str,
    registry_id: &str,
) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "INSERT INTO server_registries (server_id, registry_id, last_login_at)
         VALUES (?, ?, ?)
         ON CONFLICT (server_id, registry_id) DO UPDATE SET last_login_at = excluded.last_login_at",
        server_id,
        registry_id,
        now,
    )
    .execute(pool)
    .await
    .context("catat login registry berhasil")?;
    Ok(())
}

/// `upsert` + `record_login_success` dalam SATU transaksi (invariant 10 —
/// `docs/plan.md`: "setiap tulisan ke SQLite dalam satu siklus dibungkus
/// satu transaksi"). Dipakai `servers::verify::tautkan_registry` untuk
/// jalur registry BARU — tanpa ini, crash tepat di antara dua statement
/// bisa menyisakan baris `registries` tanpa `server_registries` yang
/// menautkannya.
pub async fn upsert_dan_catat_login(
    pool: &SqlitePool,
    host: &str,
    username: &str,
    token_encrypted: &str,
    server_id: &str,
) -> Result<String> {
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi simpan registry baru")?;

    let id = generate_id();
    let row = sqlx::query!(
        r#"INSERT INTO registries (id, host, username, token_encrypted)
           VALUES (?, ?, ?, ?)
           ON CONFLICT (host, username) DO UPDATE SET token_encrypted = excluded.token_encrypted
           RETURNING id as "id!""#,
        id,
        host,
        username,
        token_encrypted,
    )
    .fetch_one(&mut *tx)
    .await
    .context("upsert registry")?;

    let now = now_epoch();
    sqlx::query!(
        "INSERT INTO server_registries (server_id, registry_id, last_login_at)
         VALUES (?, ?, ?)
         ON CONFLICT (server_id, registry_id) DO UPDATE SET last_login_at = excluded.last_login_at",
        server_id,
        row.id,
        now,
    )
    .execute(&mut *tx)
    .await
    .context("catat login registry berhasil")?;

    tx.commit()
        .await
        .context("commit transaksi simpan registry baru")?;

    Ok(row.id)
}
