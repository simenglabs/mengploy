//! Persistensi metadata `deployment_logs` — `sqlx::query!` compile-time
//! checked.
//!
//! INVARIANT §3 NO.9: modul ini menyimpan **path dan angka saja**. Tidak ada
//! satu pun query di sini yang mem-bind isi log. Kolom `path` menyimpan NAMA
//! FILE saja (`{deployment_id}.log`), relatif terhadap `<log_dir>/deploy/` —
//! bukan path absolut, supaya `MENGDEP_LOG_DIR` bisa berubah tanpa membuat
//! baris lama salah, dan supaya tidak ada path absolut yang bisa bocor ke
//! klien lewat pesan error.

use anyhow::{Context, Result};
use sqlx::SqlitePool;

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// Metadata satu file log deploy. Sengaja TIDAK punya field isi log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMeta {
    pub deployment_id: String,
    /// Nama file saja, bukan path absolut.
    pub path: String,
    pub size_bytes: i64,
    pub line_count: i64,
    pub truncated: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Nama file log untuk satu deployment. Satu-satunya tempat bentuk nama ini
/// ditentukan — reader dan writer memakainya, tidak merangkai sendiri.
pub fn nama_file(deployment_id: &str) -> String {
    format!("{deployment_id}.log")
}

/// Catat file log baru saat sesi dibuka (pool TULIS). `INSERT OR REPLACE`
/// tidak dipakai: satu deployment tepat satu file log, dan deploy ulang
/// menghasilkan deployment baru — id yang sama muncul dua kali berarti bug
/// pemanggil, dan lebih baik gagal keras daripada diam-diam menimpa metadata
/// log yang mungkin masih dibaca.
pub async fn insert(pool: &SqlitePool, deployment_id: &str) -> Result<()> {
    let now = now_epoch();
    let path = nama_file(deployment_id);

    sqlx::query!(
        "INSERT INTO deployment_logs
            (deployment_id, path, size_bytes, line_count, truncated, created_at, updated_at)
         VALUES (?, ?, 0, 0, 0, ?, ?)",
        deployment_id,
        path,
        now,
        now,
    )
    .execute(pool)
    .await
    .context("simpan metadata log deploy baru")?;

    Ok(())
}

/// Perbarui metadata (pool TULIS). Dipanggil BERKALA oleh writer — paling
/// sering sekali per 5 detik, plus sekali saat sesi ditutup. Memanggilnya per
/// baris akan menghajar pool tulis `max_connections(1)`; throttle-nya ada di
/// `logs::writer`, bukan di sini.
pub async fn update_metadata(
    pool: &SqlitePool,
    deployment_id: &str,
    size_bytes: i64,
    line_count: i64,
    truncated: bool,
) -> Result<()> {
    let now = now_epoch();
    let truncated_int = i64::from(truncated);

    sqlx::query!(
        "UPDATE deployment_logs
            SET size_bytes = ?, line_count = ?, truncated = ?, updated_at = ?
          WHERE deployment_id = ?",
        size_bytes,
        line_count,
        truncated_int,
        now,
        deployment_id,
    )
    .execute(pool)
    .await
    .context("perbarui metadata log deploy")?;

    Ok(())
}

/// Ambil metadata satu deployment (pool BACA). `None` kalau deployment itu
/// tidak punya file log — pemanggil merender state kosong, bukan 500.
pub async fn find(pool: &SqlitePool, deployment_id: &str) -> Result<Option<LogMeta>> {
    // `deployment_id` adalah PK TEXT; SQLite mengizinkan PK TEXT NULL secara
    // historis, jadi sqlx menyimpulkannya `Option<String>`. `as "deployment_id!"`
    // menegaskan non-null — nilainya selalu terisi karena kolom itu FK ke
    // `deployments.id` dan hanya diisi lewat `insert` di atas.
    let baris = sqlx::query!(
        r#"SELECT deployment_id AS "deployment_id!", path, size_bytes, line_count,
                  truncated, created_at, updated_at
             FROM deployment_logs
            WHERE deployment_id = ?"#,
        deployment_id,
    )
    .fetch_optional(pool)
    .await
    .context("ambil metadata log deploy")?;

    Ok(baris.map(|r| LogMeta {
        deployment_id: r.deployment_id,
        path: r.path,
        size_bytes: r.size_bytes,
        line_count: r.line_count,
        truncated: r.truncated != 0,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// Satu baris kandidat untuk sapuan retensi (`logs::retention`). Sengaja
/// TIDAK memuat `path` — retensi selalu membentuk path lewat
/// `writer::path_log`, tidak pernah dari kolom ini.
///
/// `status_db` dibiarkan string mentah (bukan `StatusDeployment`) supaya
/// modul ini tidak perlu bergantung pada `crate::deployments::model` —
/// pemanggil (`logs::retention`) yang menafsirkannya, menjaga modul ini
/// tetap murni metadata `deployment_logs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KandidatRetensi {
    pub deployment_id: String,
    pub created_at: i64,
    pub status_db: String,
}

/// Ambil seluruh baris `deployment_logs` berikut status deployment induknya
/// (pool BACA) — dipakai `logs::retention::jalankan_sapuan` untuk memilih
/// korban. Diurutkan dari yang paling tua supaya batas 500/sapuan
/// (`docs/plan.md`) mengambil kandidat tertua lebih dulu.
///
/// Tidak difilter umur di SQL: filter umur DAN status "belum selesai"
/// keduanya wajib lewat fungsi murni `logs::retention::pilih_korban` supaya
/// invariant §3 no.1 (jangan hapus log deployment yang masih berjalan) bisa
/// diuji tanpa database.
pub async fn list_kandidat_retensi(pool: &SqlitePool) -> Result<Vec<KandidatRetensi>> {
    let baris = sqlx::query!(
        r#"SELECT dl.deployment_id AS "deployment_id!", dl.created_at AS "created_at!",
                  d.status AS "status_db!"
             FROM deployment_logs dl
             JOIN deployments d ON d.id = dl.deployment_id
            ORDER BY dl.created_at ASC"#
    )
    .fetch_all(pool)
    .await
    .context("ambil kandidat sapuan retensi log")?;

    Ok(baris
        .into_iter()
        .map(|r| KandidatRetensi {
            deployment_id: r.deployment_id,
            created_at: r.created_at,
            status_db: r.status_db,
        })
        .collect())
}

/// Hapus baris `deployment_logs` untuk sekumpulan id (pool TULIS), SATU
/// transaksi untuk seluruh batch (invariant §3 no.10) — bukan satu transaksi
/// per file. `ids` kosong adalah no-op, bukan error.
pub async fn hapus_batch(pool: &SqlitePool, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool
        .begin()
        .await
        .context("mulai transaksi hapus batch metadata log retensi")?;
    for id in ids {
        sqlx::query!("DELETE FROM deployment_logs WHERE deployment_id = ?", id)
            .execute(&mut *tx)
            .await
            .context("hapus baris metadata log deploy saat retensi")?;
    }
    tx.commit()
        .await
        .context("commit transaksi hapus batch metadata log retensi")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_uji(nama: &str) -> (crate::db::DbPools, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-logrepo-{nama}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let pools = crate::db::connect_and_migrate(&dir.join("test.db"))
            .await
            .expect("connect_and_migrate harus sukses");
        (pools, dir)
    }

    /// `deployment_logs.deployment_id` adalah FK ke `deployments.id`, jadi
    /// baris induknya harus ada sebelum metadata log bisa disimpan.
    async fn buat_deployment(pool: &SqlitePool, id: &str) {
        sqlx::query!("INSERT INTO servers (id, name, host, port, ssh_user, ssh_key_encrypted, status, consecutive_failures, created_at, updated_at) VALUES ('srv1', 'srv', 'h', 22, 'u', 'k', 'pending', 0, 0, 0)")
            .execute(pool)
            .await
            .ok();
        sqlx::query!("INSERT INTO apps (id, server_id, name, port, health_path, created_at, updated_at) VALUES ('app1', 'srv1', 'app', 80, '/', 0, 0)")
            .execute(pool)
            .await
            .ok();
        sqlx::query!(
            "INSERT INTO deployments (id, app_id, commit_sha, image_digest, status, trigger_source, created_at)
             VALUES (?, 'app1', 'abc', 'sha256:x', 'queued', 'api', 0)",
            id,
        )
        .execute(pool)
        .await
        .expect("insert deployment induk harus sukses");
    }

    #[tokio::test]
    async fn insert_lalu_find_mengembalikan_metadata_awal_nol() {
        let (pools, dir) = pool_uji("insert").await;
        buat_deployment(&pools.write, "dep1").await;

        insert(&pools.write, "dep1").await.expect("insert sukses");
        let meta = find(&pools.read, "dep1")
            .await
            .expect("find sukses")
            .expect("metadata harus ada");

        assert_eq!(meta.path, "dep1.log", "path harus NAMA FILE saja");
        assert!(
            !meta.path.contains('/'),
            "path tidak boleh memuat pemisah direktori — path absolut bisa bocor ke klien"
        );
        assert_eq!(meta.size_bytes, 0);
        assert_eq!(meta.line_count, 0);
        assert!(!meta.truncated);

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn find_untuk_deployment_tanpa_log_mengembalikan_none() {
        let (pools, dir) = pool_uji("kosong").await;

        let meta = find(&pools.read, "tidak-ada").await.expect("find sukses");
        assert!(
            meta.is_none(),
            "deployment tanpa file log harus None, bukan error — pemanggil merender state kosong"
        );

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn update_metadata_menyimpan_truncated_dan_angka() {
        let (pools, dir) = pool_uji("update").await;
        buat_deployment(&pools.write, "dep2").await;
        insert(&pools.write, "dep2").await.expect("insert sukses");

        update_metadata(&pools.write, "dep2", 8_388_608, 4242, true)
            .await
            .expect("update sukses");

        let meta = find(&pools.read, "dep2")
            .await
            .expect("find sukses")
            .expect("metadata harus ada");
        assert_eq!(meta.size_bytes, 8_388_608);
        assert_eq!(meta.line_count, 4242);
        assert!(meta.truncated, "penanda terpotong harus tersimpan");

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn insert_dua_kali_untuk_id_sama_gagal_bukan_menimpa_diam_diam() {
        let (pools, dir) = pool_uji("dobel").await;
        buat_deployment(&pools.write, "dep3").await;

        insert(&pools.write, "dep3")
            .await
            .expect("insert pertama sukses");
        let kedua = insert(&pools.write, "dep3").await;
        assert!(
            kedua.is_err(),
            "id yang sama dua kali berarti bug pemanggil — harus gagal keras, \
             bukan menimpa metadata log yang mungkin masih dibaca"
        );

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
