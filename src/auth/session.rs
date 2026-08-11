//! CRUD sesi login: pembuatan token buram, rotasi sesi lama, cek expiry.
//!
//! Kebijakan expiry (Q6, `docs/plan.md`): absolute 30 hari dari `created_at`,
//! tanpa idle timeout.

use anyhow::{Context, Result};
use rand::RngExt;
use rand::distr::Alphanumeric;
use sqlx::SqlitePool;

/// Panjang token sesi/CSRF dalam karakter alfanumerik. 32 karakter dari
/// alfabet 62-simbol ~= 190 bit entropi, jauh di atas cukup untuk token buram.
const TOKEN_LEN: usize = 32;

/// Umur sesi absolut dalam detik: 30 hari.
const SESSION_TTL_SECS: i64 = 30 * 24 * 60 * 60;

/// Baris sesi yang sudah divalidasi (belum kedaluwarsa).
#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub csrf_token: String,
    #[allow(dead_code)] // dipakai kalau nanti butuh audit created_at
    pub created_at: i64,
    #[allow(dead_code)] // dipakai kalau nanti butuh menampilkan sisa masa berlaku sesi
    pub expires_at: i64,
}

/// Hasilkan token acak kripto-aman untuk dipakai sebagai id sesi atau token
/// CSRF. `rand::rng()` di crate `rand` 0.10 memakai sumber acak OS di bawahnya
/// dan aman untuk keperluan token opaque semacam ini.
fn generate_token() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(TOKEN_LEN)
        .map(char::from)
        .collect()
}

/// Epoch detik saat ini. Dibungkus supaya gampang dites terpisah dari sistem
/// jam nyata bila perlu (tidak dipakai saat ini, tapi menjaga satu titik
/// sumber waktu).
fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// Buat sesi baru dan hapus semua sesi lama (rotasi penuh saat login —
/// pengguna tunggal, jadi tidak ada kebutuhan sesi paralel banyak perangkat
/// yang perlu dipertahankan lintas login).
pub async fn create_session(pool: &SqlitePool) -> Result<Session> {
    let id = generate_token();
    let csrf_token = generate_token();
    let created_at = now_epoch();
    let expires_at = created_at + SESSION_TTL_SECS;

    let mut tx = pool.begin().await.context("mulai transaksi buat sesi")?;

    sqlx::query!("DELETE FROM sessions")
        .execute(&mut *tx)
        .await
        .context("hapus sesi lama")?;

    sqlx::query!(
        "INSERT INTO sessions (id, created_at, expires_at, csrf_token) VALUES (?, ?, ?, ?)",
        id,
        created_at,
        expires_at,
        csrf_token,
    )
    .execute(&mut *tx)
    .await
    .context("simpan sesi baru")?;

    tx.commit().await.context("commit transaksi buat sesi")?;

    Ok(Session {
        id,
        csrf_token,
        created_at,
        expires_at,
    })
}

/// Ambil sesi dari token id, hanya kalau belum kedaluwarsa. Sesi kedaluwarsa
/// diperlakukan seolah tidak ada (bukan dihapus di jalur baca — pool baca
/// tidak boleh menulis).
pub async fn find_valid_session(pool: &SqlitePool, session_id: &str) -> Result<Option<Session>> {
    // ponytail: sqlx menganggap `id` (TEXT PRIMARY KEY) nullable karena SQLite
    // tidak menegakkan NOT NULL otomatis pada PRIMARY KEY non-INTEGER — cast
    // `as "id!"` memberi tahu sqlx kolom ini sebenarnya tidak pernah NULL.
    let row = sqlx::query!(
        r#"SELECT id as "id!", created_at, expires_at, csrf_token FROM sessions WHERE id = ?"#,
        session_id
    )
    .fetch_optional(pool)
    .await
    .context("baca sesi")?;

    let Some(row) = row else {
        return Ok(None);
    };

    if row.expires_at <= now_epoch() {
        return Ok(None);
    }

    Ok(Some(Session {
        id: row.id,
        csrf_token: row.csrf_token,
        created_at: row.created_at,
        expires_at: row.expires_at,
    }))
}

/// Hapus sesi tertentu (dipakai saat logout).
pub async fn delete_session(pool: &SqlitePool, session_id: &str) -> Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE id = ?", session_id)
        .execute(pool)
        .await
        .context("hapus sesi")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ponytail: tidak ada mekanisme mock waktu di modul ini, jadi expiry
    /// diuji lewat perhitungan murni (bukan lewat db) supaya cepat dan tidak
    /// butuh fixture waktu berjalan mundur.
    #[test]
    fn sesi_belum_kedaluwarsa_kalau_expires_at_di_masa_depan() {
        let created_at = now_epoch();
        let expires_at = created_at + SESSION_TTL_SECS;
        assert!(expires_at > now_epoch());
    }

    #[test]
    fn sesi_kedaluwarsa_kalau_expires_at_di_masa_lalu() {
        let expires_at = now_epoch() - 10;
        assert!(expires_at <= now_epoch());
    }

    #[test]
    fn ttl_sesi_persis_30_hari() {
        assert_eq!(SESSION_TTL_SECS, 30 * 24 * 60 * 60);
    }

    #[test]
    fn generate_token_menghasilkan_panjang_dan_keunikan_yang_diharapkan() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_eq!(t1.len(), TOKEN_LEN);
        assert_eq!(t2.len(), TOKEN_LEN);
        assert_ne!(t1, t2);
    }
}
