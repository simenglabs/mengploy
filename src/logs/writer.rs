//! Penulis log deploy: file di `<log_dir>/deploy/{deployment_id}.log` mode
//! 0600, plus siaran ke `LogRegistry` untuk viewer langsung.
//!
//! Dua tujuan, satu panggilan `tulis`: **file** (persisten, bisa di-tail dan
//! diunduh) dan **broadcast channel** (aliran langsung ke browser). Keduanya
//! sengaja tidak saling menggagalkan — file penuh tidak menghentikan siaran,
//! dan tidak adanya subscriber tidak menghentikan penulisan file.
//!
//! INVARIANT §3 NO.9: nol baris log menyentuh SQLite. Yang masuk db hanyalah
//! angka metadata lewat `logs::repo` (`size_bytes`, `line_count`,
//! `truncated`).
//!
//! INVARIANT §3 NO.1: kegagalan tidak boleh memperburuk keadaan. Log adalah
//! pengamatan, bukan kontrol — batas ukuran terlampaui, disk error, atau
//! registry penuh TIDAK PERNAH membatalkan deploy. Semua jalur gagal di modul
//! ini berakhir di `tracing::warn!`, bukan di `Err` yang merambat ke engine.
//!
//! Control plane tidak pernah menulis secretnya sendiri ke sini — private key
//! SSH, token registry, token deploy, isi kunci `age`. Itu tanggung jawab
//! pemanggil (`deployments::engine`) saat menyusun teks baris; modul ini tidak
//! bisa menebak mana yang secret. Kebijakan untuk secret yang dicetak aplikasi
//! PENGGUNA adalah Q2 (`docs/plan.md`), milik security.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};

use super::registry::{LogRegistry, LogSession};
use super::{LogEvent, repo};

/// Batas ukuran satu file log deploy — tabel angka `docs/plan.md`.
pub const MAX_LOG_BYTES: u64 = 8_388_608;

/// Batas panjang satu baris. Lebih panjang dipotong + sisipan penanda.
pub const MAX_LINE_BYTES: usize = 8 * 1024;

/// Sisipan saat satu baris melewati [`MAX_LINE_BYTES`].
const PENANDA_BARIS_DIPOTONG: &str = "…[baris dipotong]";

/// Baris penutup saat [`MAX_LOG_BYTES`] terlampaui. Ditulis TEPAT SEKALI.
const PENANDA_LOG_DIPOTONG: &str =
    "--- log dipotong pada batas 8 MiB; sisa keluaran tidak disimpan ---";

/// Flush buffer tiap 200 ms ATAU 64 KiB, mana yang lebih dulu.
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const FLUSH_BYTES: usize = 64 * 1024;

/// Metadata di-UPDATE paling sering sekali per 5 detik — pool tulis
/// `max_connections(1)` tidak boleh dihajar per baris.
const METADATA_INTERVAL: Duration = Duration::from_secs(5);

/// Sesi penulisan log satu deployment.
///
/// Dibuka [`mulai`], ditulis [`LogWriter::tulis`], ditutup
/// [`LogWriter::tutup`]. Bentuk buka/tulis/tutup eksplisit (bukan `Drop`)
/// dipilih karena penutupan butuh `await`: flush terakhir, UPDATE metadata
/// final, dan pengiriman [`LogEvent::Selesai`] semuanya asinkron, dan `Drop`
/// tidak bisa menunggu. Engine memanggil `tutup` di titik yang sama dengan
/// `deployment_events.remove` supaya tidak ada jalur keluar yang melewatkannya.
pub struct LogWriter {
    deployment_id: String,
    file: Option<BufWriter<tokio::fs::File>>,
    /// `None` kalau `LogRegistry::mulai` menolak (batas 64 sesi tercapai).
    /// Log TETAP ditulis ke file — hanya siaran langsungnya yang hilang.
    session: Option<Arc<LogSession>>,
    size_bytes: u64,
    line_count: i64,
    truncated: bool,
    /// Menjamin penanda "log dipotong" dan `tracing::warn!` terjadi sekali
    /// saja, bukan per baris — kalau tidak, log control plane sendiri meledak.
    penanda_potong_ditulis: bool,
    buffer_bytes: usize,
    flush_terakhir: Instant,
    metadata_terakhir: Instant,
}

/// Path file log satu deployment. Selalu dirangkai dari `log_dir` + nama file
/// hasil [`repo::nama_file`] — tidak pernah dari nilai kolom `path` mentah dan
/// tidak pernah dari input klien.
pub fn path_log(log_dir: &Path, deployment_id: &str) -> PathBuf {
    log_dir.join("deploy").join(repo::nama_file(deployment_id))
}

/// Buka sesi log: buat file mode 0600, INSERT metadata, daftarkan sesi
/// broadcast.
///
/// Mengembalikan `Err` HANYA kalau file atau baris metadata tidak bisa dibuat
/// sama sekali. Pemanggil (`deployments::engine`) tetap tidak boleh
/// membatalkan deploy karenanya — log adalah pengamatan, bukan kontrol.
pub async fn mulai(
    pool_tulis: &SqlitePool,
    registry: &Arc<LogRegistry>,
    log_dir: &Path,
    deployment_id: &str,
) -> Result<LogWriter> {
    let path = path_log(log_dir, deployment_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("bikin direktori log deploy")?;
    }

    // Mode 0600 diset SAAT file dibuat, bukan sesudahnya: chmod setelah
    // create meninggalkan celah singkat di mana file bisa terbaca proses lain.
    // `mode` di sini adalah method inheren `tokio::fs::OpenOptions` (unix),
    // bukan `std::os::unix::fs::OpenOptionsExt`.
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .await
        .context("buka file log deploy")?;

    repo::insert(pool_tulis, deployment_id).await?;

    // `mulai` bisa mengembalikan None saat batas 64 sesi tercapai. Itu BUKAN
    // kegagalan membuka sesi log: file tetap ditulis, hanya siaran langsung
    // yang tidak tersedia. Menukar fitur nyaman dengan jaminan memori, sesuai
    // aturan 5 `docs/plan.md`.
    let session = registry.mulai(deployment_id);
    if session.is_none() {
        tracing::warn!(
            deployment_id,
            "registry log penuh; log deploy tetap ditulis ke file tanpa siaran langsung"
        );
    }

    let sekarang = Instant::now();
    Ok(LogWriter {
        deployment_id: deployment_id.to_string(),
        file: Some(BufWriter::new(file)),
        session,
        size_bytes: 0,
        line_count: 0,
        truncated: false,
        penanda_potong_ditulis: false,
        buffer_bytes: 0,
        flush_terakhir: sekarang,
        metadata_terakhir: sekarang,
    })
}

impl LogWriter {
    /// Tulis satu baris: ke file (kalau belum penuh) dan ke broadcast channel
    /// (selalu).
    ///
    /// Tidak pernah mengembalikan `Err` — kegagalan I/O dicatat `tracing::warn!`
    /// dan penulisan file dihentikan, tapi deploy berjalan terus dan siaran
    /// langsung tetap mengalir.
    pub async fn tulis(&mut self, pool_tulis: &SqlitePool, baris: &str) {
        let baris = potong_baris(baris);

        // Siaran SELALU jalan, bahkan setelah file penuh: pengguna yang sedang
        // menonton tetap melihat aliran langsung; hanya persistensinya yang
        // berhenti (aturan eksplisit `docs/plan.md`).
        if let Some(session) = &self.session {
            session.kirim(LogEvent::Baris(Arc::from(baris.as_str())));
        }

        if self.file.is_none() {
            return;
        }

        if self.size_bytes >= MAX_LOG_BYTES {
            self.tandai_terpotong(pool_tulis).await;
            return;
        }

        let payload = format!("{baris}\n");
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(err) = file.write_all(payload.as_bytes()).await {
            tracing::warn!(
                deployment_id = %self.deployment_id,
                error = %err,
                "gagal menulis baris log deploy; penulisan file dihentikan, deploy lanjut"
            );
            self.file = None;
            return;
        }

        self.size_bytes += payload.len() as u64;
        self.line_count += 1;
        self.buffer_bytes += payload.len();

        if self.buffer_bytes >= FLUSH_BYTES || self.flush_terakhir.elapsed() >= FLUSH_INTERVAL {
            self.flush().await;
        }

        if self.metadata_terakhir.elapsed() >= METADATA_INTERVAL {
            self.simpan_metadata(pool_tulis).await;
        }

        if self.size_bytes >= MAX_LOG_BYTES {
            self.tandai_terpotong(pool_tulis).await;
        }
    }

    /// Tutup sesi: flush terakhir → UPDATE metadata final → kirim
    /// [`LogEvent::Selesai`] → lepas `Arc` sesi.
    ///
    /// Urutannya mengikat: `Selesai` harus terkirim SEBELUM `Arc` dilepas,
    /// kalau tidak handler SSE menggantung menunggu event yang tidak akan
    /// pernah datang (aturan 3 `docs/plan.md`).
    pub async fn tutup(mut self, pool_tulis: &SqlitePool) {
        self.flush().await;
        if let Some(mut file) = self.file.take() {
            let _ = file.shutdown().await;
        }
        self.simpan_metadata_paksa(pool_tulis).await;

        if let Some(session) = self.session.take() {
            session.kirim(LogEvent::Selesai);
            drop(session);
        }
    }

    /// Jumlah baris yang sudah ditulis ke file (bukan yang disiarkan).
    pub fn line_count(&self) -> i64 {
        self.line_count
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    async fn flush(&mut self) {
        if let Some(file) = self.file.as_mut()
            && let Err(err) = file.flush().await
        {
            tracing::warn!(
                deployment_id = %self.deployment_id,
                error = %err,
                "gagal flush buffer log deploy"
            );
        }
        self.buffer_bytes = 0;
        self.flush_terakhir = Instant::now();
    }

    async fn simpan_metadata(&mut self, pool_tulis: &SqlitePool) {
        self.metadata_terakhir = Instant::now();
        self.simpan_metadata_paksa(pool_tulis).await;
    }

    async fn simpan_metadata_paksa(&self, pool_tulis: &SqlitePool) {
        if let Err(err) = repo::update_metadata(
            pool_tulis,
            &self.deployment_id,
            self.size_bytes as i64,
            self.line_count,
            self.truncated,
        )
        .await
        {
            tracing::warn!(
                deployment_id = %self.deployment_id,
                error = %err,
                "gagal memperbarui metadata log deploy"
            );
        }
    }

    /// Batas 8 MiB terlampaui: tulis SATU baris penutup, set `truncated = 1`,
    /// `tracing::warn!` SEKALI, lalu berhenti menulis ke file. Deploy tidak
    /// dibatalkan.
    async fn tandai_terpotong(&mut self, pool_tulis: &SqlitePool) {
        if self.penanda_potong_ditulis {
            return;
        }
        self.penanda_potong_ditulis = true;
        self.truncated = true;

        if let Some(file) = self.file.as_mut() {
            let payload = format!("{PENANDA_LOG_DIPOTONG}\n");
            let _ = file.write_all(payload.as_bytes()).await;
            let _ = file.flush().await;
            self.size_bytes += payload.len() as u64;
            self.line_count += 1;
        }

        tracing::warn!(
            deployment_id = %self.deployment_id,
            batas_bytes = MAX_LOG_BYTES,
            "log deploy melewati batas ukuran; penulisan file dihentikan, deploy tetap berjalan"
        );

        // Berhenti menulis ke file — siaran langsung tetap jalan karena
        // `session` sengaja TIDAK dilepas di sini.
        self.file = None;
        self.simpan_metadata_paksa(pool_tulis).await;
    }
}

/// Potong baris yang melewati [`MAX_LINE_BYTES`] dan tambahkan penanda.
/// Pemotongan menghormati batas karakter UTF-8 — memotong di tengah rangkaian
/// byte menghasilkan `String` tidak valid.
fn potong_baris(baris: &str) -> String {
    if baris.len() <= MAX_LINE_BYTES {
        return baris.to_string();
    }
    let mut batas = MAX_LINE_BYTES;
    while batas > 0 && !baris.is_char_boundary(batas) {
        batas -= 1;
    }
    format!("{}{PENANDA_BARIS_DIPOTONG}", &baris[..batas])
}

/// Ukuran file log saat ini, untuk pemanggil yang perlu tahu tanpa membuka
/// writer. Dipakai test dan (nanti) sapuan retensi.
pub async fn ukuran_file(path: &Path) -> Result<u64> {
    let mut file = tokio::fs::File::open(path)
        .await
        .context("buka file log untuk mengukur")?;
    let ukuran = file
        .seek(SeekFrom::End(0))
        .await
        .context("ukur panjang file log")?;
    Ok(ukuran)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    struct Uji {
        pools: crate::db::DbPools,
        registry: Arc<LogRegistry>,
        dir: PathBuf,
    }

    impl Uji {
        async fn baru(nama: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mengdep-test-logwriter-{nama}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let pools = crate::db::connect_and_migrate(&dir.join("db").join("test.db"))
                .await
                .expect("connect_and_migrate harus sukses");
            Self {
                pools,
                registry: Arc::new(LogRegistry::new()),
                dir,
            }
        }

        fn log_dir(&self) -> PathBuf {
            self.dir.join("logs")
        }

        async fn siapkan_deployment(&self, id: &str) {
            sqlx::query!("INSERT OR IGNORE INTO servers (id, name, host, port, ssh_user, ssh_key_encrypted, status, consecutive_failures, created_at, updated_at) VALUES ('srv1', 'srv', 'h', 22, 'u', 'k', 'pending', 0, 0, 0)")
                .execute(&self.pools.write)
                .await
                .expect("insert server uji");
            sqlx::query!("INSERT OR IGNORE INTO apps (id, server_id, name, port, health_path, created_at, updated_at) VALUES ('app1', 'srv1', 'app', 80, '/', 0, 0)")
                .execute(&self.pools.write)
                .await
                .expect("insert app uji");
            sqlx::query!(
                "INSERT INTO deployments (id, app_id, commit_sha, image_digest, status, trigger_source, created_at)
                 VALUES (?, 'app1', 'abc', 'sha256:x', 'queued', 'api', 0)",
                id,
            )
            .execute(&self.pools.write)
            .await
            .expect("insert deployment uji");
        }

        async fn selesai(self) {
            self.pools.write.close().await;
            self.pools.read.close().await;
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn file_log_dibuat_dengan_mode_0600() {
        let uji = Uji::baru("mode").await;
        uji.siapkan_deployment("dep1").await;

        let writer = mulai(&uji.pools.write, &uji.registry, &uji.log_dir(), "dep1")
            .await
            .expect("mulai sesi log harus sukses");
        writer.tutup(&uji.pools.write).await;

        let path = path_log(&uji.log_dir(), "dep1");
        let mode = std::fs::metadata(&path)
            .expect("file log harus ada")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "file log wajib 0600, bukan default umask");

        uji.selesai().await;
    }

    #[tokio::test]
    async fn baris_lebih_panjang_dari_batas_dipotong_dan_diberi_penanda() {
        let panjang = "x".repeat(MAX_LINE_BYTES * 2);
        let hasil = potong_baris(&panjang);

        assert!(
            hasil.ends_with(PENANDA_BARIS_DIPOTONG),
            "baris terpotong wajib diberi penanda supaya pembaca tahu ada yang hilang"
        );
        assert!(
            hasil.len() <= MAX_LINE_BYTES + PENANDA_BARIS_DIPOTONG.len(),
            "hasil potong tidak boleh melebihi batas + penanda"
        );

        let pendek = "baris biasa";
        assert_eq!(
            potong_baris(pendek),
            pendek,
            "baris di bawah batas tidak boleh disentuh"
        );
    }

    #[tokio::test]
    async fn potong_baris_tidak_merusak_utf8_multibyte() {
        // Memotong tepat di MAX_LINE_BYTES bisa jatuh di tengah rangkaian byte
        // karakter multibyte — hasilnya String tidak valid kalau tidak
        // dihormati batas karakternya.
        let panjang = "é".repeat(MAX_LINE_BYTES);
        let hasil = potong_baris(&panjang);
        assert!(hasil.ends_with(PENANDA_BARIS_DIPOTONG));
        assert!(
            hasil.is_char_boundary(hasil.len() - PENANDA_BARIS_DIPOTONG.len()),
            "pemotongan harus jatuh di batas karakter"
        );
    }

    #[tokio::test]
    async fn batas_ukuran_terlampaui_menandai_truncated_dan_menghentikan_penulisan() {
        let uji = Uji::baru("batas").await;
        uji.siapkan_deployment("dep2").await;

        let mut writer = mulai(&uji.pools.write, &uji.registry, &uji.log_dir(), "dep2")
            .await
            .expect("mulai sesi log harus sukses");

        // Lewati batas dengan baris besar; MAX_LINE_BYTES membatasi per baris,
        // jadi butuh banyak baris untuk menembus 8 MiB.
        let baris = "y".repeat(MAX_LINE_BYTES - 1);
        let perlu = (MAX_LOG_BYTES / MAX_LINE_BYTES as u64) + 2;
        for _ in 0..perlu {
            writer.tulis(&uji.pools.write, &baris).await;
        }

        assert!(
            writer.truncated(),
            "melewati 8 MiB wajib menandai truncated"
        );
        let baris_saat_terpotong = writer.line_count();

        // Menulis lagi setelah terpotong tidak boleh menambah isi file.
        writer.tulis(&uji.pools.write, "baris setelah penuh").await;
        assert_eq!(
            writer.line_count(),
            baris_saat_terpotong,
            "penulisan file harus berhenti setelah batas terlampaui"
        );

        writer.tutup(&uji.pools.write).await;

        let path = path_log(&uji.log_dir(), "dep2");
        let isi = tokio::fs::read_to_string(&path)
            .await
            .expect("file log harus terbaca");
        assert_eq!(
            isi.matches(PENANDA_LOG_DIPOTONG).count(),
            1,
            "baris penutup wajib ada TEPAT SATU, bukan per baris"
        );
        assert!(
            !isi.contains("baris setelah penuh"),
            "baris setelah batas tidak boleh dipersistensi"
        );

        let meta = repo::find(&uji.pools.read, "dep2")
            .await
            .expect("find sukses")
            .expect("metadata harus ada");
        assert!(meta.truncated, "truncated=1 wajib tersimpan di metadata");

        uji.selesai().await;
    }

    #[tokio::test]
    async fn siaran_tetap_jalan_setelah_file_penuh() {
        let uji = Uji::baru("siaran").await;
        uji.siapkan_deployment("dep3").await;

        let mut writer = mulai(&uji.pools.write, &uji.registry, &uji.log_dir(), "dep3")
            .await
            .expect("mulai sesi log harus sukses");
        let sesi = uji.registry.ikut("dep3").expect("sesi harus aktif");
        let mut rx = sesi.subscribe();

        let baris = "z".repeat(MAX_LINE_BYTES - 1);
        let perlu = (MAX_LOG_BYTES / MAX_LINE_BYTES as u64) + 2;
        for _ in 0..perlu {
            writer.tulis(&uji.pools.write, &baris).await;
        }
        assert!(writer.truncated());

        // Kuras channel. `try_recv` mengembalikan Err(Lagged) saat subscriber
        // tertinggal — itu BUKAN tanda channel kosong, jadi loop harus
        // menelannya dan lanjut, bukan berhenti di situ.
        loop {
            match rx.try_recv() {
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }

        writer.tulis(&uji.pools.write, "masih disiarkan").await;
        let diterima = rx
            .try_recv()
            .expect("baris setelah file penuh tetap disiarkan");
        match diterima {
            LogEvent::Baris(b) => assert_eq!(
                &*b, "masih disiarkan",
                "pengguna yang sedang menonton tetap melihat aliran langsung"
            ),
            lain => panic!("event tak terduga: {lain:?}"),
        }

        writer.tutup(&uji.pools.write).await;
        uji.selesai().await;
    }

    #[tokio::test]
    async fn metadata_tidak_diupdate_per_baris() {
        let uji = Uji::baru("throttle").await;
        uji.siapkan_deployment("dep4").await;

        let mut writer = mulai(&uji.pools.write, &uji.registry, &uji.log_dir(), "dep4")
            .await
            .expect("mulai sesi log harus sukses");

        for i in 0..200 {
            writer.tulis(&uji.pools.write, &format!("baris {i}")).await;
        }

        // Throttle 5 detik belum lewat, jadi metadata di db masih nilai INSERT
        // awal (nol) meski 200 baris sudah ditulis ke file. Kalau writer
        // meng-UPDATE per baris, angka ini sudah 200 dan test merah.
        let meta = repo::find(&uji.pools.read, "dep4")
            .await
            .expect("find sukses")
            .expect("metadata harus ada");
        assert_eq!(
            meta.line_count, 0,
            "metadata tidak boleh di-UPDATE per baris — pool tulis max_connections(1)"
        );

        writer.tutup(&uji.pools.write).await;

        // Penutupan memaksa UPDATE final, jadi sekarang angkanya benar.
        let meta_akhir = repo::find(&uji.pools.read, "dep4")
            .await
            .expect("find sukses")
            .expect("metadata harus ada");
        assert_eq!(
            meta_akhir.line_count, 200,
            "penutupan sesi wajib menyimpan metadata final"
        );

        uji.selesai().await;
    }

    #[tokio::test]
    async fn tutup_mengirim_event_selesai_lalu_melepas_sesi() {
        let uji = Uji::baru("selesai").await;
        uji.siapkan_deployment("dep5").await;

        let mut writer = mulai(&uji.pools.write, &uji.registry, &uji.log_dir(), "dep5")
            .await
            .expect("mulai sesi log harus sukses");
        let sesi = uji.registry.ikut("dep5").expect("sesi harus aktif");
        let mut rx = sesi.subscribe();

        writer.tulis(&uji.pools.write, "halo").await;
        writer.tutup(&uji.pools.write).await;

        let mut lihat_selesai = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, LogEvent::Selesai) {
                lihat_selesai = true;
            }
        }
        assert!(
            lihat_selesai,
            "tanpa event Selesai, handler SSE menggantung menunggu yang tak akan datang"
        );

        drop(rx);
        drop(sesi);
        assert_eq!(
            uji.registry.jumlah_sesi(),
            0,
            "sesi harus lenyap dari registry setelah writer dan subscriber dilepas"
        );

        uji.selesai().await;
    }
}
