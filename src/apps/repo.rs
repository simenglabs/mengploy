//! Persistensi `apps` dan `domains` — `sqlx::query!` compile-time checked.
//! Lock per app (invariant §3 no.12) tinggal di sini juga — kolomnya di
//! `apps`, bukan tabel terpisah.

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;
use sqlx::SqlitePool;
use std::fmt;

use super::model::{AppRingkas, DeployTokenRingkas, DomainRingkas, EnvVersionRingkas};

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

pub struct NewApp<'a> {
    pub server_id: &'a str,
    pub name: &'a str,
    pub health_path: &'a str,
    pub health_grace_secs: i64,
    pub port: i64,
    pub restart_policy: &'a str,
}

/// Penanda aman yang bisa dipetakan handler menjadi 409 tanpa membocorkan
/// detail SQLite. Prune memegang lock server sampai seluruh image selesai
/// diproses, sehingga app baru tidak boleh dibuat di tengah operasi itu.
#[derive(Debug)]
pub struct ServerLocked;

impl fmt::Display for ServerLocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("server sedang dikunci operasi armada")
    }
}

impl std::error::Error for ServerLocked {}

pub async fn insert(pool: &SqlitePool, new: NewApp<'_>) -> Result<String> {
    let id = generate_id();
    let now = now_epoch();
    let mut tx = pool.begin().await.context("mulai transaksi simpan app")?;

    let locked = sqlx::query!(
        "SELECT 1 as locked FROM fleet_server_locks WHERE server_id = ? AND expires_at > ?",
        new.server_id,
        now,
    )
    .fetch_optional(&mut *tx)
    .await
    .context("cek lock server sebelum simpan app")?;
    if locked.is_some() {
        return Err(ServerLocked.into());
    }

    sqlx::query!(
        "INSERT INTO apps
            (id, server_id, name, health_path, health_grace_secs, port, restart_policy,
             created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        id,
        new.server_id,
        new.name,
        new.health_path,
        new.health_grace_secs,
        new.port,
        new.restart_policy,
        now,
        now,
    )
    .execute(&mut *tx)
    .await
    .context("simpan app baru")?;
    tx.commit().await.context("commit transaksi simpan app")?;

    Ok(id)
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<AppRingkas>> {
    let row = sqlx::query_as!(
        AppRingkas,
        r#"SELECT id as "id!", server_id, name, health_path, health_grace_secs, port,
                  restart_policy, created_at, updated_at
           FROM apps WHERE id = ?"#,
        id
    )
    .fetch_optional(pool)
    .await
    .context("baca app")?;

    Ok(row)
}

/// Dipakai `POST /api/v1/deploy` — CI mengirim nama app, bukan id buram.
pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<AppRingkas>> {
    let row = sqlx::query_as!(
        AppRingkas,
        r#"SELECT id as "id!", server_id, name, health_path, health_grace_secs, port,
                  restart_policy, created_at, updated_at
           FROM apps WHERE name = ?"#,
        name
    )
    .fetch_optional(pool)
    .await
    .context("baca app berdasarkan nama")?;

    Ok(row)
}

pub async fn list_ringkas(pool: &SqlitePool) -> Result<Vec<AppRingkas>> {
    sqlx::query_as!(
        AppRingkas,
        r#"SELECT id as "id!", server_id, name, health_path, health_grace_secs, port,
                  restart_policy, created_at, updated_at
           FROM apps ORDER BY name ASC"#
    )
    .fetch_all(pool)
    .await
    .context("baca daftar app")
}

pub async fn list_by_server(pool: &SqlitePool, server_id: &str) -> Result<Vec<AppRingkas>> {
    sqlx::query_as!(
        AppRingkas,
        r#"SELECT id as "id!", server_id, name, health_path, health_grace_secs, port,
                  restart_policy, created_at, updated_at
           FROM apps WHERE server_id = ? ORDER BY name ASC"#,
        server_id
    )
    .fetch_all(pool)
    .await
    .context("baca daftar app per server")
}

pub async fn add_domain(pool: &SqlitePool, app_id: &str, host: &str) -> Result<String> {
    let id = generate_id();
    sqlx::query!(
        "INSERT INTO domains (id, app_id, host, tls_enabled) VALUES (?, ?, ?, 1)",
        id,
        app_id,
        host,
    )
    .execute(pool)
    .await
    .context("simpan domain baru")?;
    Ok(id)
}

pub async fn list_domains(pool: &SqlitePool, app_id: &str) -> Result<Vec<DomainRingkas>> {
    sqlx::query_as!(
        DomainRingkas,
        r#"SELECT id as "id!", app_id, host, tls_enabled as "tls_enabled: bool"
           FROM domains WHERE app_id = ? ORDER BY host ASC"#,
        app_id
    )
    .fetch_all(pool)
    .await
    .context("baca daftar domain")
}

/// Simpan hash token deploy baru — plaintext-nya SUDAH ditampilkan
/// pemanggil sekali (`auth::deploy_token::generate`), tidak pernah sampai
/// ke sini.
pub async fn insert_deploy_token(
    pool: &SqlitePool,
    app_id: &str,
    name: &str,
    token_hash: &str,
) -> Result<String> {
    let id = generate_id();
    let now = now_epoch();
    sqlx::query!(
        "INSERT INTO deploy_tokens (id, app_id, name, token_hash, created_at)
         VALUES (?, ?, ?, ?, ?)",
        id,
        app_id,
        name,
        token_hash,
        now,
    )
    .execute(pool)
    .await
    .context("simpan token deploy baru")?;
    Ok(id)
}

/// `(id, token_hash)` semua token milik `app_id` — dipakai
/// `routes::deploy_api` mencocokkan `Authorization: Bearer` terhadap token
/// APP INI SAJA (bukan scan semua token semua app; nama app sudah
/// ditentukan body request sebelum verifikasi token dijalankan).
pub async fn list_deploy_token_hashes(
    pool: &SqlitePool,
    app_id: &str,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", token_hash FROM deploy_tokens WHERE app_id = ?"#,
        app_id
    )
    .fetch_all(pool)
    .await
    .context("baca token deploy per app")?;
    Ok(rows.into_iter().map(|r| (r.id, r.token_hash)).collect())
}

pub async fn touch_deploy_token_last_used(pool: &SqlitePool, id: &str) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "UPDATE deploy_tokens SET last_used_at = ? WHERE id = ?",
        now,
        id
    )
    .execute(pool)
    .await
    .context("perbarui last_used_at token deploy")?;
    Ok(())
}

pub async fn list_deploy_tokens_ringkas(
    pool: &SqlitePool,
    app_id: &str,
) -> Result<Vec<DeployTokenRingkas>> {
    sqlx::query_as!(
        DeployTokenRingkas,
        r#"SELECT id as "id!", name, created_at, last_used_at
           FROM deploy_tokens WHERE app_id = ? ORDER BY created_at DESC"#,
        app_id
    )
    .fetch_all(pool)
    .await
    .context("baca daftar token deploy")
}

/// Ambil lock deploy untuk `app_id` — invariant §3 no.12 (WAJIB kedaluwarsa).
/// `WHERE lock_expires_at IS NULL OR lock_expires_at < now` — lock lama yang
/// sudah kedaluwarsa TIDAK menghalangi lock baru, terlepas dari apakah
/// pemegang lock lama pernah melepaskannya (worker crash tidak mengunci app
/// selamanya). Mengembalikan `true` kalau berhasil diambil (1 baris
/// terupdate), `false` kalau app sedang terkunci app lain yang masih aktif.
pub async fn acquire_lock(
    pool: &SqlitePool,
    app_id: &str,
    lock_token: &str,
    expires_at: i64,
) -> Result<bool> {
    let now = now_epoch();
    let server_locked = sqlx::query!(
        "SELECT 1 as locked FROM fleet_server_locks l JOIN apps a ON a.server_id = l.server_id
         WHERE a.id = ? AND l.expires_at > ?",
        app_id,
        now,
    )
    .fetch_optional(pool)
    .await
    .context("cek lock server sebelum lock app")?;
    if server_locked.is_some() {
        return Ok(false);
    }
    let hasil = sqlx::query!(
        "UPDATE apps SET lock_token = ?, lock_expires_at = ?, updated_at = ?
         WHERE id = ? AND (lock_expires_at IS NULL OR lock_expires_at < ?)",
        lock_token,
        expires_at,
        now,
        app_id,
        now,
    )
    .execute(pool)
    .await
    .context("ambil lock deploy app")?;

    Ok(hasil.rows_affected() == 1)
}

/// Lepas lock — HANYA kalau `lock_token` masih cocok (pemegang lock yang
/// sama). Kalau sudah kedaluwarsa dan diambil alih pihak lain, pelepasan
/// dari pemegang lama TIDAK PERNAH menimpa lock baru itu.
pub async fn release_lock(pool: &SqlitePool, app_id: &str, lock_token: &str) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "UPDATE apps SET lock_token = NULL, lock_expires_at = NULL, updated_at = ?
         WHERE id = ? AND lock_token = ?",
        now,
        app_id,
        lock_token,
    )
    .execute(pool)
    .await
    .context("lepas lock deploy app")?;
    Ok(())
}

/// Ambil lock seluruh app pada satu server secara atomik. Prune memakai ini
/// sebelum membaca/menghapus image agar deployment baru tidak menyisip di
/// antara pemeriksaan status dan tindakan destruktif.
pub async fn acquire_server_locks(
    pool: &SqlitePool,
    server_id: &str,
    lock_token: &str,
    expires_at: i64,
) -> Result<bool> {
    let now = now_epoch();
    let mut tx = pool.begin().await.context("mulai transaksi lock server")?;
    let existing = sqlx::query!(
        "SELECT operation_id FROM fleet_server_locks WHERE server_id = ? AND expires_at > ?",
        server_id,
        now,
    )
    .fetch_optional(&mut *tx)
    .await
    .context("cek lock server yang masih aktif")?;
    if existing.is_some() {
        tx.rollback().await.context("rollback lock server aktif")?;
        return Ok(false);
    }
    sqlx::query!(
        "DELETE FROM fleet_server_locks WHERE server_id = ?",
        server_id,
    )
    .execute(&mut *tx)
    .await
    .context("hapus lock server kedaluwarsa")?;
    let count = sqlx::query!(
        "SELECT COUNT(*) as count FROM apps WHERE server_id = ?",
        server_id,
    )
    .fetch_one(&mut *tx)
    .await
    .context("hitung app untuk lock server")?;
    let updated = sqlx::query!(
        "UPDATE apps SET lock_token = ?, lock_expires_at = ?, updated_at = ?
         WHERE server_id = ? AND (lock_expires_at IS NULL OR lock_expires_at < ?)",
        lock_token,
        expires_at,
        now,
        server_id,
        now,
    )
    .execute(&mut *tx)
    .await
    .context("ambil lock seluruh app server")?;

    let expected = u64::try_from(count.count).context("jumlah app lock server tidak valid")?;
    if updated.rows_affected() != expected {
        tx.rollback()
            .await
            .context("rollback lock server yang gagal")?;
        return Ok(false);
    }
    sqlx::query!(
        "INSERT INTO fleet_server_locks (server_id, operation_id, expires_at)
         VALUES (?, ?, ?)",
        server_id,
        lock_token,
        expires_at,
    )
    .execute(&mut *tx)
    .await
    .context("simpan lock server")?;
    tx.commit()
        .await
        .context("commit lock seluruh app server")?;
    Ok(true)
}

/// Perpanjang lease lock server sebelum tahap destruktif berikutnya.
/// `false` berarti lock sudah hilang atau kedaluwarsa, sehingga pemanggil
/// wajib berhenti tanpa menghapus image berikutnya.
pub async fn renew_server_lock(
    pool: &SqlitePool,
    server_id: &str,
    lock_token: &str,
    expires_at: i64,
) -> Result<bool> {
    let now = now_epoch();
    let result = sqlx::query!(
        "UPDATE fleet_server_locks SET expires_at = ?
         WHERE server_id = ? AND operation_id = ? AND expires_at > ?",
        expires_at,
        server_id,
        lock_token,
        now,
    )
    .execute(pool)
    .await
    .context("perpanjang lock server")?;
    Ok(result.rows_affected() == 1)
}

/// Lepas semua lock milik operasi prune pada satu server.
pub async fn release_server_locks(
    pool: &SqlitePool,
    server_id: &str,
    lock_token: &str,
) -> Result<()> {
    let now = now_epoch();
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi lepas lock server")?;
    sqlx::query!(
        "DELETE FROM fleet_server_locks WHERE server_id = ? AND operation_id = ?",
        server_id,
        lock_token,
    )
    .execute(&mut *tx)
    .await
    .context("hapus lock server")?;
    sqlx::query!(
        "UPDATE apps SET lock_token = NULL, lock_expires_at = NULL, updated_at = ?
         WHERE server_id = ? AND lock_token = ?",
        now,
        server_id,
        lock_token,
    )
    .execute(&mut *tx)
    .await
    .context("lepas lock seluruh app server")?;
    tx.commit()
        .await
        .context("commit transaksi lepas lock server")?;
    Ok(())
}

/// Simpan/perbarui satu env var. `is_secret` HANYA dipakai saat baris
/// BELUM ada (`ditentukan sekali saat dibuat`, `docs/plan.md` "Desain
/// teknis") — update berikutnya untuk key yang sama tidak mengubahnya,
/// terlepas dari argumen yang dikirim pemanggil.
pub async fn upsert_env_var(
    pool: &SqlitePool,
    app_id: &str,
    key: &str,
    value_encrypted: &str,
    is_secret: bool,
) -> Result<()> {
    let now = now_epoch();
    let id = generate_id();
    let is_secret_int = i64::from(is_secret);
    sqlx::query!(
        "INSERT INTO env_vars (id, app_id, key, value_encrypted, is_secret, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (app_id, key) DO UPDATE SET
             value_encrypted = excluded.value_encrypted,
             updated_at = excluded.updated_at",
        id,
        app_id,
        key,
        value_encrypted,
        is_secret_int,
        now,
    )
    .execute(pool)
    .await
    .context("simpan env var")?;
    Ok(())
}

pub async fn upsert_env_var_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    app_id: &str,
    key: &str,
    value_encrypted: &str,
    is_secret: bool,
) -> Result<()> {
    let now = now_epoch();
    let id = generate_id();
    let is_secret_int = i64::from(is_secret);
    sqlx::query!(
        "INSERT INTO env_vars (id, app_id, key, value_encrypted, is_secret, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (app_id, key) DO UPDATE SET
             value_encrypted = excluded.value_encrypted,
             updated_at = excluded.updated_at",
        id,
        app_id,
        key,
        value_encrypted,
        is_secret_int,
        now,
    )
    .execute(&mut **tx)
    .await
    .context("simpan env var dalam transaksi")?;
    Ok(())
}

pub async fn delete_env_var(pool: &SqlitePool, app_id: &str, key: &str) -> Result<()> {
    sqlx::query!(
        "DELETE FROM env_vars WHERE app_id = ? AND key = ?",
        app_id,
        key
    )
    .execute(pool)
    .await
    .context("hapus env var")?;
    Ok(())
}

pub async fn delete_env_var_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    app_id: &str,
    key: &str,
) -> Result<()> {
    sqlx::query!(
        "DELETE FROM env_vars WHERE app_id = ? AND key = ?",
        app_id,
        key
    )
    .execute(&mut **tx)
    .await
    .context("hapus env var dalam transaksi")?;
    Ok(())
}

/// `(key, value_encrypted, is_secret)` seluruh env app — SATU-SATUNYA
/// fungsi listing env (tidak ada varian "ringkas tanpa value" terpisah):
/// dipakai membangun snapshot `env_versions` (dekripsi tiap value, gabung
/// dengan perubahan dari form, enkripsi ulang SEKALI sebagai satu blob
/// JSON) DAN dipakai `routes/apps.rs` merender tab Environment (dekripsi
/// HANYA baris yang `is_secret=false` sebelum diteruskan ke `src/web/**` —
/// keputusan "tampilkan atau topengi" ada di pemanggil, bukan di sini,
/// supaya `apps::repo` tidak perlu tahu soal `CryptoKey`).
pub async fn list_env_vars_encrypted(
    pool: &SqlitePool,
    app_id: &str,
) -> Result<Vec<(String, String, bool)>> {
    let rows = sqlx::query!(
        r#"SELECT key, value_encrypted, is_secret as "is_secret: bool"
           FROM env_vars WHERE app_id = ? ORDER BY key ASC"#,
        app_id
    )
    .fetch_all(pool)
    .await
    .context("baca env var terenkripsi")?;
    Ok(rows
        .into_iter()
        .map(|r| (r.key, r.value_encrypted, r.is_secret))
        .collect())
}

/// Simpan snapshot env baru dalam transaksi YANG SAMA dengan pemanggil
/// (`tx`, bukan `pool`) — dipakai bersamaan dengan INSERT deployment+job
/// (invariant §3 no.10, satu transaksi per siklus tulis).
pub async fn insert_env_version_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    app_id: &str,
    version: i64,
    snapshot_encrypted: &str,
    note: Option<&str>,
) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "INSERT INTO env_versions (id, app_id, version, snapshot_encrypted, note, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        id,
        app_id,
        version,
        snapshot_encrypted,
        note,
        now,
    )
    .execute(&mut **tx)
    .await
    .context("simpan snapshot env version")?;
    Ok(())
}

pub async fn find_latest_env_version(
    pool: &SqlitePool,
    app_id: &str,
) -> Result<Option<EnvVersionRingkas>> {
    sqlx::query_as!(
        EnvVersionRingkas,
        r#"SELECT id as "id!", version, note, created_at
           FROM env_versions WHERE app_id = ? ORDER BY version DESC LIMIT 1"#,
        app_id
    )
    .fetch_optional(pool)
    .await
    .context("cari versi env terbaru")
}

/// `snapshot_encrypted` mentah untuk SATU `env_version_id` — dipakai
/// `deployments::engine` menulis file env ke server target. Dekripsi
/// terjadi di pemanggil (yang punya `CryptoKey` lewat `AppState`), bukan
/// di sini (`apps::repo` tidak menyentuh `crypto`).
pub async fn env_version_belongs_to_app(
    pool: &SqlitePool,
    env_version_id: &str,
    app_id: &str,
) -> Result<bool> {
    let row = sqlx::query!(
        "SELECT 1 as exists_flag FROM env_versions WHERE id = ? AND app_id = ?",
        env_version_id,
        app_id,
    )
    .fetch_optional(pool)
    .await
    .context("verifikasi kepemilikan env version")?;
    Ok(row.is_some())
}

pub async fn find_env_version_snapshot(
    pool: &SqlitePool,
    env_version_id: &str,
) -> Result<Option<String>> {
    let row = sqlx::query!(
        "SELECT snapshot_encrypted FROM env_versions WHERE id = ?",
        env_version_id
    )
    .fetch_optional(pool)
    .await
    .context("baca snapshot env version")?;
    Ok(row.map(|r| r.snapshot_encrypted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_menghasilkan_panjang_dan_keunikan_yang_diharapkan() {
        let a = generate_id();
        let b = generate_id();
        assert_eq!(a.len(), ID_LEN);
        assert_ne!(a, b);
    }
}
