//! Sapuan retensi log deploy — `docs/plan.md` Fase 3, tabel "Angka yang
//! dikunci": retensi 30 hari (dari `Config::log_retention_days`, tidak
//! pernah hardcode), batas 500 file per sapuan, satu sapuan dibatasi 60
//! detik.
//!
//! INVARIANT §3 NO.1 (paling mudah kesenggol di modul ini): retensi WAJIB
//! melewati deployment yang statusnya belum `selesai()`, apa pun umurnya.
//! Ini ditegakkan di [`pilih_korban`] — fungsi MURNI, tanpa I/O, tanpa
//! database — supaya invariant ini bisa diuji langsung tanpa disk/db, sesuai
//! `docs/plan.md`: "pemilih korban (fungsi murni) + eksekusi".
//!
//! INVARIANT §3 NO.10: satu transaksi per BATCH ([`repo::hapus_batch`]),
//! bukan satu transaksi per file.
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use super::reader;
use super::repo::{self, KandidatRetensi};
use super::writer;
use crate::deployments::model::StatusDeployment;

/// Batas jumlah file yang dihapus dalam satu sapuan (`docs/plan.md`, "Angka
/// yang dikunci"). Sisanya ditunda ke sapuan berikutnya (24 jam lagi) —
/// menghindari satu transaksi/satu sapuan raksasa.
pub const BATAS_FILE_PER_SAPUAN: usize = 500;

/// Batas waktu satu sapuan (`docs/plan.md`, "Timeout per tahap"). Terlampaui
/// bukan error fatal — sisa kandidat yang belum sempat diproses menunggu
/// sapuan berikutnya secara alami (baris metadatanya masih ada di db).
pub const BATAS_WAKTU_SAPUAN: Duration = Duration::from_secs(60);

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// Ringkasan satu sapuan — dipakai `tracing` di pemanggil, bukan dikembalikan
/// ke klien mana pun.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RingkasanSapuan {
    pub dihapus: usize,
    pub gagal_hapus_file: usize,
}

/// Pilih deployment mana yang layak disapu retensi. **Fungsi murni** — tidak
/// menyentuh disk, tidak menyentuh database, tidak memanggil jam sistem
/// (`now` diberikan pemanggil) — supaya invariant §3 no.1 bisa diuji tanpa
/// db/disk sama sekali.
///
/// Urutan `kandidat` diasumsikan sudah terurut dari yang paling tua
/// ([`repo::list_kandidat_retensi`] mengurutkan `created_at ASC`) — `take`
/// di sini berarti "ambil yang paling tua dulu" saat kandidat melebihi
/// `batas_jumlah`.
pub fn pilih_korban(
    kandidat: &[KandidatRetensi],
    now: i64,
    retention_days: u32,
    batas_jumlah: usize,
) -> Vec<String> {
    let ambang = now - i64::from(retention_days) * 86_400;
    kandidat
        .iter()
        // INVARIANT §3 NO.1: deployment yang belum `selesai()` TIDAK PERNAH
        // dipilih, apa pun umurnya — dicek sebelum umur supaya niatnya jelas
        // dibaca reviewer.
        .filter(|k| StatusDeployment::from_db_str(&k.status_db).selesai())
        .filter(|k| k.created_at < ambang)
        .take(batas_jumlah)
        .map(|k| k.deployment_id.clone())
        .collect()
}

/// Hapus file log satu deployment. File yang sudah tidak ada di disk BUKAN
/// error — metadata-nya tetap layak dihapus. Kegagalan lain (mis. izin)
/// dicatat `tracing::warn!` dan deployment itu DILEWATI di sapuan ini
/// (baris metadatanya tetap ada, dicoba lagi sapuan berikutnya) — loop
/// retensi tidak boleh mati karena satu file bermasalah.
///
/// `deployment_id` di sini berasal dari database, bukan dari klien, jadi
/// secara teori sudah alfanumerik (`deployments::repo::generate_id`). Tetap
/// dilewatkan [`reader::nama_file_aman`] sebelum path dibentuk: ini
/// satu-satunya jalur di seluruh program yang MENGHAPUS file berdasarkan
/// nilai kolom, dan gerbang yang sama dipakai semua jalur baca
/// (`docs/plan.md` "Anti path traversal"). Kalau id tidak lolos pola, file
/// TIDAK disentuh dan metadata TIDAK dihapus — baris aneh dibiarkan hidup
/// supaya bisa diselidiki manusia, bukan disapu diam-diam.
async fn hapus_file_log(log_dir: &Path, deployment_id: &str) -> bool {
    if reader::nama_file_aman(deployment_id).is_err() {
        tracing::warn!(
            deployment_id = %deployment_id,
            "deployment_id di deployment_logs tidak lolos pola nama file aman; \
             file log TIDAK dihapus dan metadata dibiarkan untuk diselidiki"
        );
        return false;
    }
    let path = writer::path_log(log_dir, deployment_id);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => {
            tracing::warn!(
                deployment_id = %deployment_id,
                error = %err,
                "gagal menghapus file log saat sapuan retensi — dilewati, dicoba lagi sapuan berikutnya"
            );
            false
        }
    }
}

/// Jalankan satu sapuan retensi: pilih korban, hapus file, hapus metadata
/// (satu transaksi untuk seluruh batch). Dibatasi [`BATAS_WAKTU_SAPUAN`] —
/// terlampaui berarti sisa kandidat menunggu sapuan berikutnya, BUKAN error
/// yang menjatuhkan worker (pemanggil di `worker::log_retention` tetap harus
/// menangkap `Err` dan lanjut tick berikutnya, sesuai konvensi AGENTS.md).
pub async fn jalankan_sapuan(
    pool_read: &SqlitePool,
    pool_write: &SqlitePool,
    log_dir: &Path,
    retention_days: u32,
) -> Result<RingkasanSapuan> {
    let sapuan = async {
        let kandidat = repo::list_kandidat_retensi(pool_read)
            .await
            .context("ambil kandidat sapuan retensi log")?;
        let now = now_epoch();
        let korban_id = pilih_korban(&kandidat, now, retention_days, BATAS_FILE_PER_SAPUAN);

        let mut berhasil = Vec::with_capacity(korban_id.len());
        let mut gagal_hapus_file = 0usize;
        for id in &korban_id {
            if hapus_file_log(log_dir, id).await {
                berhasil.push(id.clone());
            } else {
                gagal_hapus_file += 1;
            }
        }

        repo::hapus_batch(pool_write, &berhasil)
            .await
            .context("hapus batch metadata log saat retensi")?;

        Ok::<RingkasanSapuan, anyhow::Error>(RingkasanSapuan {
            dihapus: berhasil.len(),
            gagal_hapus_file,
        })
    };

    tokio::time::timeout(BATAS_WAKTU_SAPUAN, sapuan)
        .await
        .context("sapuan retensi log melewati batas 60 detik")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kandidat(id: &str, created_at: i64, status_db: &str) -> KandidatRetensi {
        KandidatRetensi {
            deployment_id: id.to_string(),
            created_at,
            status_db: status_db.to_string(),
        }
    }

    const SATU_HARI: i64 = 86_400;

    #[test]
    fn tidak_pernah_pilih_deployment_yang_belum_selesai_walau_sangat_tua() {
        let now = 1_000_000_000;
        // Umur jauh melewati retensi (365 hari lebih tua dari batas 30 hari)
        // tapi statusnya masih 'pulling' — invariant §3 no.1.
        let kandidat = vec![kandidat("d1", now - 365 * SATU_HARI, "pulling")];

        let korban = pilih_korban(&kandidat, now, 30, 500);

        assert!(
            korban.is_empty(),
            "deployment yang belum selesai tidak boleh dipilih apa pun umurnya"
        );
    }

    #[test]
    fn pilih_deployment_selesai_yang_lebih_tua_dari_batas() {
        let now = 1_000_000_000;
        let kandidat = vec![kandidat("d2", now - 31 * SATU_HARI, "live")];

        let korban = pilih_korban(&kandidat, now, 30, 500);

        assert_eq!(korban, vec!["d2".to_string()]);
    }

    #[test]
    fn tidak_pilih_deployment_selesai_yang_lebih_muda_dari_batas() {
        let now = 1_000_000_000;
        let kandidat = vec![kandidat("d3", now - 29 * SATU_HARI, "failed")];

        let korban = pilih_korban(&kandidat, now, 30, 500);

        assert!(
            korban.is_empty(),
            "deployment selesai yang masih di dalam masa retensi tidak boleh dihapus"
        );
    }

    #[test]
    fn semua_status_akhir_layak_dipilih_kalau_cukup_tua() {
        let now = 1_000_000_000;
        for status in ["live", "failed", "cancelled", "unknown"] {
            let kandidat = vec![kandidat("d4", now - 31 * SATU_HARI, status)];
            let korban = pilih_korban(&kandidat, now, 30, 500);
            assert_eq!(
                korban,
                vec!["d4".to_string()],
                "status '{status}' harus dipilih"
            );
        }
    }

    #[test]
    fn batas_jumlah_per_sapuan_benar_benar_membatasi() {
        let now = 1_000_000_000;
        let kandidat: Vec<KandidatRetensi> = (0..501)
            .map(|i| kandidat(&format!("d{i}"), now - 31 * SATU_HARI, "live"))
            .collect();

        let korban = pilih_korban(&kandidat, now, 30, BATAS_FILE_PER_SAPUAN);

        assert_eq!(
            korban.len(),
            BATAS_FILE_PER_SAPUAN,
            "501 kandidat memenuhi syarat tapi batas 500/sapuan harus tetap berlaku"
        );
    }

    #[test]
    fn batas_jumlah_mengambil_yang_paling_tua_dulu() {
        let now = 1_000_000_000;
        // Kandidat sudah terurut created_at ASC (kontrak repo::list_kandidat_retensi).
        let kandidat = vec![
            kandidat("tertua", now - 40 * SATU_HARI, "live"),
            kandidat("termuda", now - 31 * SATU_HARI, "live"),
        ];

        let korban = pilih_korban(&kandidat, now, 30, 1);

        assert_eq!(
            korban,
            vec!["tertua".to_string()],
            "dengan batas 1, yang paling tua harus diprioritaskan"
        );
    }

    // --- integrasi: jalankan_sapuan (db + disk sungguhan) ---

    async fn pool_uji(nama: &str) -> (crate::db::DbPools, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-logretention-{nama}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let pools = crate::db::connect_and_migrate(&dir.join("test.db"))
            .await
            .expect("connect_and_migrate harus sukses");
        (pools, dir)
    }

    async fn buat_deployment(pool: &SqlitePool, id: &str, status: &str, created_at: i64) {
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
             VALUES (?, 'app1', 'abc', 'sha256:x', ?, 'api', ?)",
            id,
            status,
            created_at,
        )
        .execute(pool)
        .await
        .expect("insert deployment induk harus sukses");
    }

    #[tokio::test]
    async fn jalankan_sapuan_menghapus_file_dan_metadata_deployment_selesai_yang_tua() {
        let (pools, dir) = pool_uji("hapus").await;
        let now = now_epoch();
        let tua = now - 31 * SATU_HARI;
        buat_deployment(&pools.write, "lama", "live", tua).await;
        repo::insert(&pools.write, "lama")
            .await
            .expect("insert metadata log");

        let log_dir = dir.join("logs");
        let deploy_dir = log_dir.join("deploy");
        tokio::fs::create_dir_all(&deploy_dir)
            .await
            .expect("buat direktori log");
        tokio::fs::write(deploy_dir.join("lama.log"), b"halo")
            .await
            .expect("tulis file log uji");

        // created_at metadata log diisi `insert()` dengan waktu SEKARANG, jadi
        // untuk menguji retensi umur file log (bukan umur deployment), samakan
        // manual ke waktu yang tua.
        sqlx::query!(
            "UPDATE deployment_logs SET created_at = ? WHERE deployment_id = 'lama'",
            tua,
        )
        .execute(&pools.write)
        .await
        .expect("set created_at metadata log uji");

        let ringkasan = jalankan_sapuan(&pools.read, &pools.write, &log_dir, 30)
            .await
            .expect("sapuan harus sukses");

        assert_eq!(ringkasan.dihapus, 1);
        assert_eq!(ringkasan.gagal_hapus_file, 0);
        assert!(
            !deploy_dir.join("lama.log").exists(),
            "file log harus terhapus"
        );
        let meta = repo::find(&pools.read, "lama")
            .await
            .expect("query find sukses");
        assert!(meta.is_none(), "baris metadata harus terhapus");

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn jalankan_sapuan_melewati_deployment_yang_belum_selesai() {
        let (pools, dir) = pool_uji("lewati").await;
        let now = now_epoch();
        let tua = now - 365 * SATU_HARI;
        buat_deployment(&pools.write, "berjalan", "pulling", tua).await;
        repo::insert(&pools.write, "berjalan")
            .await
            .expect("insert metadata log");
        sqlx::query!(
            "UPDATE deployment_logs SET created_at = ? WHERE deployment_id = 'berjalan'",
            tua,
        )
        .execute(&pools.write)
        .await
        .expect("set created_at metadata log uji");

        let log_dir = dir.join("logs");
        tokio::fs::create_dir_all(log_dir.join("deploy"))
            .await
            .expect("buat direktori log");
        tokio::fs::write(log_dir.join("deploy").join("berjalan.log"), b"halo")
            .await
            .expect("tulis file log uji");

        let ringkasan = jalankan_sapuan(&pools.read, &pools.write, &log_dir, 30)
            .await
            .expect("sapuan harus sukses");

        assert_eq!(
            ringkasan.dihapus, 0,
            "deployment yang belum selesai tidak boleh dihapus walau sangat tua"
        );
        assert!(
            log_dir.join("deploy").join("berjalan.log").exists(),
            "file log deployment yang masih berjalan tidak boleh disentuh"
        );
        let meta = repo::find(&pools.read, "berjalan")
            .await
            .expect("query find sukses");
        assert!(
            meta.is_some(),
            "baris metadata deployment yang masih berjalan tidak boleh terhapus"
        );

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn jalankan_sapuan_file_sudah_hilang_di_disk_tetap_hapus_metadata() {
        let (pools, dir) = pool_uji("filehilang").await;
        let now = now_epoch();
        let tua = now - 31 * SATU_HARI;
        buat_deployment(&pools.write, "yatim", "failed", tua).await;
        repo::insert(&pools.write, "yatim")
            .await
            .expect("insert metadata log");
        sqlx::query!(
            "UPDATE deployment_logs SET created_at = ? WHERE deployment_id = 'yatim'",
            tua,
        )
        .execute(&pools.write)
        .await
        .expect("set created_at metadata log uji");

        // Sengaja TIDAK menulis file — mensimulasikan file yang sudah hilang
        // di disk tapi barisnya masih ada di db.
        let log_dir = dir.join("logs");

        let ringkasan = jalankan_sapuan(&pools.read, &pools.write, &log_dir, 30)
            .await
            .expect("sapuan harus sukses walau file tidak ada");

        assert_eq!(ringkasan.dihapus, 1);
        assert_eq!(ringkasan.gagal_hapus_file, 0);
        let meta = repo::find(&pools.read, "yatim")
            .await
            .expect("query find sukses");
        assert!(
            meta.is_none(),
            "file hilang di disk BUKAN error — metadata tetap harus terhapus"
        );

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sapuan retensi adalah SATU-SATUNYA jalur di program ini yang menghapus
    /// file berdasarkan nilai kolom database. Kalau kolom itu pernah tercemar
    /// (bug pemanggil, restore db manual, injeksi lewat jalur lain), path
    /// relatif seperti `../../` bisa membawa `remove_file` keluar dari
    /// `<log_dir>/deploy/`. Gerbang `reader::nama_file_aman` menutupnya.
    #[tokio::test]
    async fn jalankan_sapuan_menolak_deployment_id_yang_tidak_lolos_pola_nama_aman() {
        let (pools, dir) = pool_uji("traversal").await;
        let now = now_epoch();
        let tua = now - 31 * SATU_HARI;
        let id_jahat = "../../korban";
        buat_deployment(&pools.write, id_jahat, "failed", tua).await;
        repo::insert(&pools.write, id_jahat)
            .await
            .expect("insert metadata log");
        sqlx::query!(
            "UPDATE deployment_logs SET created_at = ? WHERE deployment_id = ?",
            tua,
            id_jahat,
        )
        .execute(&pools.write)
        .await
        .expect("set created_at metadata log uji");

        let log_dir = dir.join("logs");
        tokio::fs::create_dir_all(log_dir.join("deploy"))
            .await
            .expect("bikin direktori log deploy uji");
        // File di LUAR <log_dir>/deploy/ yang akan jadi korban kalau gerbangnya
        // tidak ada: `log_dir/deploy/../../korban.log` == `dir/korban.log`.
        let korban = dir.join("korban.log");
        tokio::fs::write(&korban, b"jangan sentuh")
            .await
            .expect("tulis file korban uji");

        let ringkasan = jalankan_sapuan(&pools.read, &pools.write, &log_dir, 30)
            .await
            .expect("sapuan tidak boleh gagal keras karena satu id aneh");

        assert!(
            korban.exists(),
            "file di luar <log_dir>/deploy/ TIDAK BOLEH tersentuh sapuan retensi"
        );
        assert_eq!(ringkasan.dihapus, 0);
        assert_eq!(ringkasan.gagal_hapus_file, 1);
        let meta = repo::find(&pools.read, id_jahat)
            .await
            .expect("query find sukses");
        assert!(
            meta.is_some(),
            "baris dengan id aneh dibiarkan hidup untuk diselidiki, bukan disapu diam-diam"
        );

        pools.write.close().await;
        pools.read.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
