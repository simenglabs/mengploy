//! Pembuatan pool koneksi SQLite dan migrasi.
//!
//! Pola dua pool (AGENTS.md): pool tulis `max_connections(1)` untuk
//! INSERT/UPDATE/DELETE, pool baca banyak koneksi untuk SELECT. Pragma
//! `busy_timeout`, `foreign_keys`, `synchronous` diset per-koneksi lewat
//! `SqliteConnectOptions` karena tidak persisten lewat migration runner
//! (lihat catatan di `migrations/0001_init.sql`).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

/// Sepasang pool: satu untuk tulis (satu koneksi), satu untuk baca (banyak
/// koneksi). Menulis lewat pool baca adalah bug (AGENTS.md).
pub struct DbPools {
    pub write: SqlitePool,
    pub read: SqlitePool,
}

/// Jumlah koneksi maksimum pool baca. Nilai kecil cukup untuk 3-8 VPS,
/// pengguna tunggal — bukan beban tinggi.
const READ_POOL_MAX_CONNECTIONS: u32 = 5;

/// Timeout SQLite `busy_timeout` dalam milidetik.
const BUSY_TIMEOUT_MS: u64 = 5000;

/// Bangun dua pool dan jalankan migrasi. Membuat file db (dan direktori
/// induknya) kalau belum ada, dengan mode `0600`.
pub async fn connect_and_migrate(db_path: &Path) -> Result<DbPools> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("bikin direktori db {}", parent.display()))?;

        // Set mode 0700 pada direktori data — baik yang BARU dibuat maupun
        // yang SUDAH ADA sebelumnya (mis. dari sebelum perbaikan ini, mode
        // default create_dir_all biasanya 0755). File sampingan SQLite yang
        // tidak di-chmod eksplisit satu-satu (-journal, file temp VACUUM)
        // tetap lahir privat lewat umask proses (lihat main.rs), tapi
        // direktori itu sendiri harus diperketat di sini karena umask tidak
        // berlaku surut ke direktori lama (temuan security Fase 0 HARUS-2).
        set_mode(parent, 0o700)
            .with_context(|| format!("set mode 0700 pada direktori db {}", parent.display()))?;
    }

    let db_existed_before = db_path.exists();

    let write_options = connect_options(db_path).create_if_missing(true);
    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(write_options)
        .await
        .context("buka pool tulis")?;

    // File db harus sudah ada sekarang (baru dibuat kalau belum ada
    // sebelumnya); set mode 0600 sebelum pool baca dibuka supaya tidak ada
    // celah world-readable.
    if !db_existed_before {
        set_file_mode_0600(db_path)
            .with_context(|| format!("set mode 0600 pada {}", db_path.display()))?;
    }

    // File `-wal` dan `-shm` lahir setelah koneksi WAL pertama terjadi, jadi
    // baru bisa ada setelah pool tulis di atas terbuka — dan bisa lahir
    // KAPAN SAJA (termasuk pada db lama yang sudah lama ada, misal setelah
    // checkpoint menghapus lalu SQLite menulis lagi). Karena itu pengecekan
    // ini TIDAK bersyarat pada `db_existed_before` — dijalankan setiap
    // startup, bukan hanya saat db baru dibuat (temuan security Fase 0 #1).
    set_file_mode_0600_if_exists(&wal_path(db_path))
        .with_context(|| format!("set mode 0600 pada {}", wal_path(db_path).display()))?;
    set_file_mode_0600_if_exists(&shm_path(db_path))
        .with_context(|| format!("set mode 0600 pada {}", shm_path(db_path).display()))?;

    let read_options = connect_options(db_path).create_if_missing(false);
    let read = SqlitePoolOptions::new()
        .max_connections(READ_POOL_MAX_CONNECTIONS)
        .connect_with(read_options)
        .await
        .context("buka pool baca")?;

    sqlx::migrate!("./migrations")
        .run(&write)
        .await
        .context("jalankan migrasi")?;

    // Migrasi menulis ke db; -wal/-shm bisa baru lahir di titik ini kalau
    // belum lahir saat pool tulis dibuka. Cek ulang supaya tidak ada celah.
    set_file_mode_0600_if_exists(&wal_path(db_path))
        .with_context(|| format!("set mode 0600 pada {}", wal_path(db_path).display()))?;
    set_file_mode_0600_if_exists(&shm_path(db_path))
        .with_context(|| format!("set mode 0600 pada {}", shm_path(db_path).display()))?;

    Ok(DbPools { write, read })
}

/// Opsi koneksi dasar dengan pragma yang wajib diset per-koneksi.
fn connect_options(db_path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(db_path)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .foreign_keys(true)
        .synchronous(SqliteSynchronous::Normal)
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

fn set_file_mode_0600(path: &Path) -> Result<()> {
    set_mode(path, 0o600)
}

/// Sama seperti `set_file_mode_0600`, tapi tidak error kalau file belum ada
/// (file `-wal`/`-shm` bisa saja belum lahir, mis. db baru belum ditulis).
fn set_file_mode_0600_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    set_file_mode_0600(path)
}

/// Path artefak WAL SQLite untuk file db tertentu (`{db}-wal`).
fn wal_path(db_path: &Path) -> std::path::PathBuf {
    sibling_with_suffix(db_path, "-wal")
}

/// Path artefak shared-memory SQLite untuk file db tertentu (`{db}-shm`).
fn shm_path(db_path: &Path) -> std::path::PathBuf {
    sibling_with_suffix(db_path, "-shm")
}

fn sibling_with_suffix(db_path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut os_string = db_path.as_os_str().to_owned();
    os_string.push(suffix);
    std::path::PathBuf::from(os_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_and_migrate_membuat_db_baru_dengan_mode_0600() {
        let dir = std::env::temp_dir().join(format!("mengdep-test-db-{}", std::process::id()));
        let db_path = dir.join("test.db");

        let pools = connect_and_migrate(&db_path)
            .await
            .expect("connect_and_migrate harus sukses");

        let metadata = std::fs::metadata(&db_path).expect("db path harus ada");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wal_dan_shm_bermode_0600_setelah_tulisan() {
        let dir = std::env::temp_dir().join(format!("mengdep-test-db-wal-{}", std::process::id()));
        let db_path = dir.join("test.db");

        let pools = connect_and_migrate(&db_path)
            .await
            .expect("connect_and_migrate harus sukses");

        // Migrasi sudah menulis (tabel dibuat), jadi -wal seharusnya sudah
        // ada. Paksa tulisan tambahan supaya lebih pasti file WAL tercipta
        // di semua konfigurasi SQLite.
        sqlx::query!("INSERT INTO settings (key, value) VALUES ('test_key', 'test_value')")
            .execute(&pools.write)
            .await
            .expect("tulis baris uji harus sukses");

        let wal = wal_path(&db_path);
        assert!(wal.exists(), "file -wal harus tercipta setelah tulisan");
        let wal_mode = std::fs::metadata(&wal)
            .expect("metadata -wal harus terbaca")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(wal_mode, 0o600, "-wal harus bermode 0600");

        let shm = shm_path(&db_path);
        if shm.exists() {
            let shm_mode = std::fs::metadata(&shm)
                .expect("metadata -shm harus terbaca")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(shm_mode, 0o600, "-shm harus bermode 0600 kalau ada");
        }

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wal_diperbaiki_mode_walau_db_lama_sudah_ada() {
        let dir =
            std::env::temp_dir().join(format!("mengdep-test-db-wal-lama-{}", std::process::id()));
        let db_path = dir.join("test.db");

        // Siklus pertama: buat db, tulis sesuatu supaya -wal lahir.
        let pools1 = connect_and_migrate(&db_path)
            .await
            .expect("connect_and_migrate pertama harus sukses");
        sqlx::query!("INSERT INTO settings (key, value) VALUES ('test_key', 'test_value')")
            .execute(&pools1.write)
            .await
            .expect("tulis baris uji harus sukses");
        pools1.write.close().await;
        pools1.read.close().await;

        // Sengaja longgarkan mode -wal untuk mensimulasikan file lama yang
        // world-readable dari sebelum perbaikan ini ada.
        let wal = wal_path(&db_path);
        if wal.exists() {
            let mut permissions = std::fs::metadata(&wal)
                .expect("metadata -wal harus terbaca")
                .permissions();
            permissions.set_mode(0o644);
            std::fs::set_permissions(&wal, permissions).expect("set mode longgar harus sukses");
        }

        // Siklus kedua: connect_and_migrate lagi ke db yang SUDAH ADA
        // (db_existed_before = true) — mode -wal harus tetap diperbaiki.
        let pools2 = connect_and_migrate(&db_path)
            .await
            .expect("connect_and_migrate kedua harus sukses");

        let wal_mode = std::fs::metadata(&wal)
            .expect("metadata -wal harus terbaca")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            wal_mode, 0o600,
            "-wal pada db lama harus diperbaiki ke 0600, bukan hanya db baru"
        );

        pools2.write.close().await;
        pools2.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn direktori_data_bermode_0700_baik_baru_maupun_sudah_ada() {
        let dir = std::env::temp_dir().join(format!("mengdep-test-db-dir-{}", std::process::id()));
        let db_path = dir.join("test.db");
        let _ = std::fs::remove_dir_all(&dir);

        // Kasus 1: direktori belum ada sama sekali.
        let pools1 = connect_and_migrate(&db_path)
            .await
            .expect("connect_and_migrate pertama harus sukses");
        let mode = std::fs::metadata(&dir)
            .expect("direktori db harus ada")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "direktori db baru harus bermode 0700");
        pools1.write.close().await;
        pools1.read.close().await;

        // Kasus 2: direktori SUDAH ADA dengan mode longgar (simulasi direktori
        // lama dari sebelum perbaikan HARUS-2) — harus diperketat ulang.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("longgarkan mode direktori harus sukses");

        let pools2 = connect_and_migrate(&db_path)
            .await
            .expect("connect_and_migrate kedua harus sukses");
        let mode2 = std::fs::metadata(&dir)
            .expect("direktori db harus ada")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode2, 0o700,
            "direktori db lama yang longgar harus diperketat ke 0700"
        );

        pools2.write.close().await;
        pools2.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn migrasi_idempoten_jalan_dua_kali_tanpa_error() {
        let dir =
            std::env::temp_dir().join(format!("mengdep-test-db-idempoten-{}", std::process::id()));
        let db_path = dir.join("test.db");

        let pools1 = connect_and_migrate(&db_path)
            .await
            .expect("migrasi pertama harus sukses");
        pools1.write.close().await;
        pools1.read.close().await;

        let pools2 = connect_and_migrate(&db_path)
            .await
            .expect("migrasi kedua (idempoten) harus sukses");
        pools2.write.close().await;
        pools2.read.close().await;

        let _ = std::fs::remove_dir_all(&dir);
    }
}
