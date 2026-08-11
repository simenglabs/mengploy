//! Persistensi `servers` — `sqlx::query!` compile-time checked. Tulisan
//! lewat pool tulis (`AppState.db_write`), bacaan lewat pool baca
//! (`AppState.db_read`). Tidak ada logika verifikasi di sini — itu
//! `servers::verify`.

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;
use sqlx::SqlitePool;

use super::model::{ServerRingkas, StatusServer};

/// Panjang id server — token buram kripto-aman, bukan auto-increment
/// (`migrations/0002_servers.sql`), sama pola dengan token sesi
/// (`auth/session.rs`).
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

/// Baris `servers` mentah TERMASUK `ssh_key_encrypted` — HANYA dipakai
/// internal (`servers::verify`, worker sub-blok 3e). TIDAK PERNAH
/// diekspor ke `src/web/` (invariant 7); lihat `ServerRingkas` untuk
/// view-model yang aman dirender.
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub ssh_user: String,
    pub ssh_key_encrypted: String,
    pub status: String,
    pub host_key_fingerprint: Option<String>,
    pub last_error_kind: Option<String>,
    pub last_error_message: Option<String>,
    pub consecutive_failures: i64,
    pub next_poll_at: i64,
    pub last_seen_at: Option<i64>,
    pub docker_version: Option<String>,
    pub os_info: Option<String>,
}

pub struct NewServer<'a> {
    pub name: &'a str,
    pub host: &'a str,
    pub port: i64,
    pub ssh_user: &'a str,
    pub ssh_key_encrypted: &'a str,
}

/// Simpan server baru dengan status `pending`, kembalikan id yang
/// dihasilkan. Verifikasi (`servers::verify::mulai_verifikasi`) dijalankan
/// TERPISAH setelah baris ini ter-commit.
pub async fn insert_pending(pool: &SqlitePool, new: NewServer<'_>) -> Result<String> {
    let id = generate_id();
    let now = now_epoch();

    sqlx::query!(
        "INSERT INTO servers
            (id, name, host, port, ssh_user, ssh_key_encrypted, status,
             consecutive_failures, next_poll_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', 0, 0, ?, ?)",
        id,
        new.name,
        new.host,
        new.port,
        new.ssh_user,
        new.ssh_key_encrypted,
        now,
        now,
    )
    .execute(pool)
    .await
    .context("simpan server baru")?;

    Ok(id)
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<ServerRow>> {
    let row = sqlx::query!(
        r#"SELECT id as "id!", name, host, port, ssh_user, ssh_key_encrypted, status,
                  host_key_fingerprint, last_error_kind, last_error_message,
                  consecutive_failures, next_poll_at, last_seen_at, docker_version, os_info
           FROM servers WHERE id = ?"#,
        id
    )
    .fetch_optional(pool)
    .await
    .context("baca server")?;

    Ok(row.map(|r| ServerRow {
        id: r.id,
        name: r.name,
        host: r.host,
        port: r.port,
        ssh_user: r.ssh_user,
        ssh_key_encrypted: r.ssh_key_encrypted,
        status: r.status,
        host_key_fingerprint: r.host_key_fingerprint,
        last_error_kind: r.last_error_kind,
        last_error_message: r.last_error_message,
        consecutive_failures: r.consecutive_failures,
        next_poll_at: r.next_poll_at,
        last_seen_at: r.last_seen_at,
        docker_version: r.docker_version,
        os_info: r.os_info,
    }))
}

/// Fleet overview/strip: seluruh server, diurutkan nama.
pub async fn list_ringkas(pool: &SqlitePool) -> Result<Vec<ServerRingkas>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", name, host, port, ssh_user, status, last_seen_at,
                  docker_version, os_info, host_key_fingerprint, consecutive_failures,
                  last_error_kind, last_error_message
           FROM servers ORDER BY name ASC"#
    )
    .fetch_all(pool)
    .await
    .context("baca daftar server")?;

    Ok(rows
        .into_iter()
        .map(|r| ServerRingkas {
            id: r.id,
            name: r.name,
            host: r.host,
            port: r.port,
            ssh_user: r.ssh_user,
            status: StatusServer::from_db_str(&r.status),
            last_seen_at: r.last_seen_at,
            docker_version: r.docker_version,
            os_info: r.os_info,
            host_key_fingerprint: r.host_key_fingerprint,
            consecutive_failures: r.consecutive_failures,
            last_error_kind: r.last_error_kind,
            last_error_message: r.last_error_message,
        })
        .collect())
}

/// Ringkasan satu server (view-model AMAN, tanpa `ssh_key_encrypted`) —
/// dipakai `GET /servers/{id}` (detail). Beda dari `find_by_id`
/// (`ServerRow`, dipakai internal `servers::verify`/`worker`) supaya
/// handler route tidak pernah punya kesempatan meneruskan baris mentah ke
/// `src/web/` (invariant 7).
pub async fn find_ringkas_by_id(pool: &SqlitePool, id: &str) -> Result<Option<ServerRingkas>> {
    let row = sqlx::query!(
        r#"SELECT id as "id!", name, host, port, ssh_user, status, last_seen_at,
                  docker_version, os_info, host_key_fingerprint, consecutive_failures,
                  last_error_kind, last_error_message
           FROM servers WHERE id = ?"#,
        id
    )
    .fetch_optional(pool)
    .await
    .context("baca ringkasan server")?;

    Ok(row.map(|r| ServerRingkas {
        id: r.id,
        name: r.name,
        host: r.host,
        port: r.port,
        ssh_user: r.ssh_user,
        status: StatusServer::from_db_str(&r.status),
        last_seen_at: r.last_seen_at,
        docker_version: r.docker_version,
        os_info: r.os_info,
        host_key_fingerprint: r.host_key_fingerprint,
        consecutive_failures: r.consecutive_failures,
        last_error_kind: r.last_error_kind,
        last_error_message: r.last_error_message,
    }))
}

pub async fn set_status_verifying(pool: &SqlitePool, id: &str) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "UPDATE servers SET status = 'verifying', updated_at = ? WHERE id = ?",
        now,
        id
    )
    .execute(pool)
    .await
    .context("set status verifying")?;
    Ok(())
}

pub async fn set_host_key_fingerprint(
    pool: &SqlitePool,
    id: &str,
    fingerprint: &str,
) -> Result<()> {
    let now = now_epoch();
    sqlx::query!(
        "UPDATE servers SET host_key_fingerprint = ?, updated_at = ? WHERE id = ?",
        fingerprint,
        now,
        id
    )
    .execute(pool)
    .await
    .context("simpan fingerprint host key")?;
    Ok(())
}

/// Verifikasi sukses penuh (langkah 1+2; registry di langkah 3 opsional,
/// tidak menggerbangi ini): server siap dipakai, masuk jadwal polling
/// normal worker (sub-blok 3e — `next_poll_at` dihitung dari
/// `poll_interval_secs` yang dioper pemanggil, bukan konstanta di sini,
/// supaya kebijakan interval tetap satu sumber kebenaran di `servers::verify`).
pub async fn mark_online(
    pool: &SqlitePool,
    id: &str,
    docker_version: &str,
    os_info: &str,
    poll_interval_secs: i64,
) -> Result<()> {
    let now = now_epoch();
    let next_poll_at = now + poll_interval_secs;
    sqlx::query!(
        "UPDATE servers
         SET status = 'online', docker_version = ?, os_info = ?, last_seen_at = ?,
             consecutive_failures = 0, next_poll_at = ?, last_error_kind = NULL,
             last_error_message = NULL, updated_at = ?
         WHERE id = ?",
        docker_version,
        os_info,
        now,
        next_poll_at,
        now,
        id
    )
    .execute(pool)
    .await
    .context("tandai server online")?;
    Ok(())
}

/// Verifikasi gagal di langkah mana pun: KEMBALI ke `pending` (BUKAN
/// `unreachable` — status itu milik worker polling 3-strikes sub-blok 3e,
/// bukan percobaan pertama), simpan kategori+pesan pendek supaya wizard
/// bisa menampilkan kegagalannya (invariant 1: tidak ada tindakan
/// destruktif, baris server tetap ada apa adanya).
pub async fn mark_verification_failed(
    pool: &SqlitePool,
    id: &str,
    error_kind: &str,
    error_message: &str,
) -> Result<()> {
    let now = now_epoch();
    let error_message = truncate_error_message(error_message);
    sqlx::query!(
        "UPDATE servers SET status = 'pending', last_error_kind = ?, last_error_message = ?, updated_at = ?
         WHERE id = ?",
        error_kind,
        error_message,
        now,
        id
    )
    .execute(pool)
    .await
    .context("tandai verifikasi server gagal")?;
    Ok(())
}

/// Server dengan `next_poll_at <= now`, diurutkan supaya yang paling lama
/// menunggu diproses lebih dulu. Dipakai `worker::status_poll` tiap siklus.
///
/// **`host_key_fingerprint IS NOT NULL` WAJIB** — poll ringan
/// (`worker::status_poll::periksa_server`) memakai mode `Strict` yang
/// mensyaratkan fingerprint tersimpan; tanpa filter ini, server yang baru
/// dibuat (`status='pending'`, `next_poll_at=0` dari `insert_pending`)
/// bisa terpilih poll SEBELUM verifikasi awal selesai. Ini bug nyata yang
/// pernah terjadi: server belum-pernah-online yang gagal di-poll bisa
/// disulap statusnya jadi `online`/`unreachable` padahal belum pernah lolos
/// verifikasi sama sekali — ditemukan lewat smoke test manual, bukan test
/// otomatis (unit test `worker::status_poll` memakai input status buatan,
/// tidak pernah menguji jalur "baru dibuat, next_poll_at masih 0").
pub async fn list_due_for_poll(pool: &SqlitePool, now: i64) -> Result<Vec<ServerRow>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", name, host, port, ssh_user, ssh_key_encrypted, status,
                  host_key_fingerprint, last_error_kind, last_error_message,
                  consecutive_failures, next_poll_at, last_seen_at, docker_version, os_info
           FROM servers
           WHERE next_poll_at <= ? AND host_key_fingerprint IS NOT NULL
           ORDER BY next_poll_at ASC"#,
        now
    )
    .fetch_all(pool)
    .await
    .context("baca server yang jatuh tempo poll")?;

    Ok(rows
        .into_iter()
        .map(|r| ServerRow {
            id: r.id,
            name: r.name,
            host: r.host,
            port: r.port,
            ssh_user: r.ssh_user,
            ssh_key_encrypted: r.ssh_key_encrypted,
            status: r.status,
            host_key_fingerprint: r.host_key_fingerprint,
            last_error_kind: r.last_error_kind,
            last_error_message: r.last_error_message,
            consecutive_failures: r.consecutive_failures,
            next_poll_at: r.next_poll_at,
            last_seen_at: r.last_seen_at,
            docker_version: r.docker_version,
            os_info: r.os_info,
        })
        .collect())
}

/// Server online yang siap dipindai worker metrik. Worker metrik memiliki
/// interval sendiri dan tidak boleh memakai jadwal backoff status poll;
/// status `online` tetap menjadi pagar agar server pending/unreachable tidak
/// dipindai tanpa verifikasi sukses.
pub async fn list_online_for_metrics(pool: &SqlitePool) -> Result<Vec<ServerRow>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", name, host, port, ssh_user, ssh_key_encrypted, status,
                  host_key_fingerprint, last_error_kind, last_error_message,
                  consecutive_failures, next_poll_at, last_seen_at, docker_version, os_info
           FROM servers
           WHERE status = 'online' AND host_key_fingerprint IS NOT NULL
           ORDER BY name ASC"#
    )
    .fetch_all(pool)
    .await
    .context("baca server online untuk worker metrik")?;

    Ok(rows
        .into_iter()
        .map(|r| ServerRow {
            id: r.id,
            name: r.name,
            host: r.host,
            port: r.port,
            ssh_user: r.ssh_user,
            ssh_key_encrypted: r.ssh_key_encrypted,
            status: r.status,
            host_key_fingerprint: r.host_key_fingerprint,
            last_error_kind: r.last_error_kind,
            last_error_message: r.last_error_message,
            consecutive_failures: r.consecutive_failures,
            next_poll_at: r.next_poll_at,
            last_seen_at: r.last_seen_at,
            docker_version: r.docker_version,
            os_info: r.os_info,
        })
        .collect())
}

/// Hasil satu server dalam satu siklus polling — dibangun `worker::status_poll`
/// (I/O jaringan sudah selesai, ini murni instruksi tulis) dan diterapkan
/// lewat [`apply_poll_batch`].
pub struct PollWriteSukses {
    pub server_id: String,
    pub docker_version: String,
    pub os_info: String,
}

pub struct PollWriteGagal {
    pub server_id: String,
    pub status: StatusServer,
    pub consecutive_failures: i64,
    pub next_poll_at: i64,
    pub error_kind: String,
    pub error_message: String,
}

pub enum PollWrite {
    Sukses(PollWriteSukses),
    Gagal(PollWriteGagal),
}

/// Terapkan seluruh hasil satu siklus polling dalam SATU transaksi
/// (invariant 10 — `docs/plan.md`: "Semua tulisan satu siklus dibungkus
/// satu transaksi"). `poll_interval_secs` dioper pemanggil (kebijakan
/// interval tetap satu sumber kebenaran di `servers::verify`).
pub async fn apply_poll_batch(
    pool: &SqlitePool,
    writes: &[PollWrite],
    now: i64,
    poll_interval_secs: i64,
) -> Result<()> {
    let mut tx = pool.begin().await.context("mulai transaksi hasil poll")?;

    for write in writes {
        match write {
            PollWrite::Sukses(s) => {
                let next_poll_at = now + poll_interval_secs;
                sqlx::query!(
                    "UPDATE servers
                     SET status = 'online', docker_version = ?, os_info = ?, last_seen_at = ?,
                         consecutive_failures = 0, next_poll_at = ?, last_error_kind = NULL,
                         last_error_message = NULL, updated_at = ?
                     WHERE id = ?",
                    s.docker_version,
                    s.os_info,
                    now,
                    next_poll_at,
                    now,
                    s.server_id
                )
                .execute(&mut *tx)
                .await
                .context("tulis hasil poll sukses")?;
            }
            PollWrite::Gagal(g) => {
                let status = g.status.as_db_str();
                let error_message = truncate_error_message(&g.error_message);
                sqlx::query!(
                    "UPDATE servers
                     SET status = ?, consecutive_failures = ?, next_poll_at = ?,
                         last_error_kind = ?, last_error_message = ?, updated_at = ?
                     WHERE id = ?",
                    status,
                    g.consecutive_failures,
                    g.next_poll_at,
                    g.error_kind,
                    error_message,
                    now,
                    g.server_id
                )
                .execute(&mut *tx)
                .await
                .context("tulis hasil poll gagal")?;
            }
        }
    }

    tx.commit().await.context("commit transaksi hasil poll")?;
    Ok(())
}

/// Batas `CHECK (length(last_error_message) <= 500)` di skema — dipotong
/// di sini supaya pemanggil tidak perlu tahu detail constraint db.
fn truncate_error_message(message: &str) -> String {
    message.chars().take(500).collect()
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

    #[test]
    fn truncate_error_message_memotong_ke_500_karakter() {
        let panjang = "x".repeat(1000);
        assert_eq!(truncate_error_message(&panjang).chars().count(), 500);
    }

    #[test]
    fn truncate_error_message_tidak_mengubah_pesan_pendek() {
        assert_eq!(
            truncate_error_message("host tidak terjangkau"),
            "host tidak terjangkau"
        );
    }
}
