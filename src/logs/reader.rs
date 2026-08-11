//! Pembaca file log deploy: validasi nama file (anti path traversal), tail
//! N baris terakhir, dan pencarian dalam satu file.
//!
//! INVARIANT §3 NO.9: modul ini hanya membaca file di disk, tidak pernah
//! SQLite — metadata (`size_bytes`, `truncated`, dst.) tetap di `logs::repo`.
//!
//! Path file **tidak pernah** dibentuk dari input klien secara langsung.
//! Gerbang tunggal adalah [`nama_file_aman`]: menolak apa pun yang tidak
//! cocok `^[A-Za-z0-9]{1,64}$` SEBELUM path dirangkai lewat
//! `logs::writer::path_log` (`docs/plan.md` "Anti path traversal").

use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Timeout baca tail file histori — tabel "Timeout per tahap" `docs/plan.md`.
const TAIL_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout pencarian dalam file — sama tabel.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Default/maksimum baris tail log deploy — tabel "Angka yang dikunci".
pub const TAIL_DEFAULT: usize = 500;
pub const TAIL_MAX: usize = 5000;

/// Maksimum baris hasil pencarian yang dikembalikan — sama tabel.
pub const SEARCH_MAX_RESULTS: usize = 500;

/// Ukuran blok baca dari ekor file, dibesarkan bertahap sampai cukup baris
/// terkumpul atau awal file tercapai. 64 KiB cukup untuk ~ratusan baris
/// tipikal tanpa harus memuat seluruh file 8 MiB ke memori.
const CHUNK_SIZE: u64 = 64 * 1024;

/// Satu baris log siap-render. `src/web/**` hanya menerima ini — sudah
/// dipisah nomor urut (untuk gutter) dan teks mentah (di-escape HTML oleh
/// pemanggil, BUKAN di sini: byte log diteruskan apa adanya, termasuk escape
/// ANSI — `docs/plan.md` Q1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Nomor baris 1-based di file asli — dipakai viewer untuk gutter/anchor.
    pub nomor: u64,
    pub teks: String,
}

/// Hasil tail: baris-baris yang berhasil dibaca. Kosong (file tidak ada,
/// kosong, atau nol baris) bukan error — pemanggil merender state kosong.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HasilTail {
    pub baris: Vec<LogLine>,
}

/// Hasil pencarian: baris yang cocok, plus penanda kalau dipotong karena
/// melebihi [`SEARCH_MAX_RESULTS`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HasilCari {
    pub baris: Vec<LogLine>,
    pub dipotong: bool,
}

/// Kegagalan membaca/mencari file log. Dipisah dari `anyhow::Error` karena
/// pemanggil (`routes::logs`) harus memetakan timeout pencarian ke 504 TANPA
/// mem-parsing string pesan — pola sama `docker::client::LogFollowError`.
#[derive(Debug)]
pub enum LogReadError {
    /// `deployment_id` tidak lolos `^[A-Za-z0-9]{1,64}$` — pemanggil memetakan
    /// ke 404, sama seperti id yang tidak dikenal di db.
    IdTidakValid,
    /// Operasi melewati batas waktu tahap (baca tail 5 detik / cari 5 detik).
    Timeout,
    /// Kegagalan I/O lain (bukan "file tidak ada" — itu ditangani sebagai
    /// hasil kosong, bukan error). Detail HANYA untuk `tracing`, tidak pernah
    /// ke klien.
    Io,
}

/// Validasi `deployment_id` sebelum path dibentuk. Satu-satunya gerbang —
/// `docs/plan.md` "Anti path traversal": menolak apa pun yang tidak cocok
/// `^[A-Za-z0-9]{1,64}$`. Id hasil `deployments::repo::generate_id` adalah
/// alfanumerik, jadi pola ini tidak membuang kasus sah.
pub fn nama_file_aman(deployment_id: &str) -> Result<String, LogReadError> {
    let valid = !deployment_id.is_empty()
        && deployment_id.len() <= 64
        && deployment_id.bytes().all(|b| b.is_ascii_alphanumeric());

    if valid {
        Ok(deployment_id.to_string())
    } else {
        Err(LogReadError::IdTidakValid)
    }
}

/// Baca N baris terakhir dari `path` tanpa memuat seluruh file ke memori.
///
/// `tail_lines` dijepit ke `[1, TAIL_MAX]` — di luar rentang bukan error,
/// dijepit ke batas terdekat (kenyamanan baca, bukan perintah destruktif,
/// `docs/api-contract.md`). `0` dijepit ke `TAIL_DEFAULT` (dianggap "tidak
/// diminta").
///
/// File tidak ada / kosong → `Ok(HasilTail::default())`, bukan error.
pub async fn tail(path: &Path, tail_lines: usize) -> Result<HasilTail, LogReadError> {
    let n = jepit_tail(tail_lines);

    let hasil = tokio::time::timeout(TAIL_READ_TIMEOUT, baca_tail(path, n)).await;
    match hasil {
        Ok(inner) => inner,
        Err(_) => Err(LogReadError::Timeout),
    }
}

fn jepit_tail(n: usize) -> usize {
    if n == 0 {
        TAIL_DEFAULT
    } else {
        n.min(TAIL_MAX)
    }
}

async fn baca_tail(path: &Path, n: usize) -> Result<HasilTail, LogReadError> {
    let Some((mut file, ukuran)) = buka_untuk_baca(path).await? else {
        return Ok(HasilTail::default());
    };

    if ukuran == 0 {
        return Ok(HasilTail::default());
    }

    // Baca blok dari ekor, membesar bertahap, sampai terkumpul n+1 baris
    // (newline) atau awal file tercapai. n+1 supaya baris pertama yang
    // terpotong bisa dibuang (bukan baris utuh dari newline sebelumnya).
    let mut ambil = CHUNK_SIZE.min(ukuran);
    let mut buf: Vec<u8>;
    loop {
        let mulai = ukuran.saturating_sub(ambil);
        buf = baca_blok(&mut file, mulai, ambil).await?;

        let jumlah_newline = buf.iter().filter(|&&b| b == b'\n').count();
        if jumlah_newline > n || ambil >= ukuran {
            break;
        }
        ambil = (ambil * 2).min(ukuran);
    }

    let teks = String::from_utf8_lossy(&buf);
    let mut baris_mentah: Vec<&str> = teks.split('\n').collect();
    // Baris terakhir dari split adalah "" kalau file berakhir dengan newline
    // (kasus normal writer) — buang supaya tidak jadi baris kosong palsu.
    if baris_mentah.last() == Some(&"") {
        baris_mentah.pop();
    }
    // Kalau blok yang diambil bukan dari awal file, baris pertama hasil
    // split kemungkinan terpotong (mulai di tengah baris) — buang, KECUALI
    // kita memang sudah membaca dari byte 0.
    let mulai_dari_awal = ambil >= ukuran;
    if !mulai_dari_awal && !baris_mentah.is_empty() {
        baris_mentah.remove(0);
    }

    let total_baris_terbaca = baris_mentah.len();
    let ambil_n = total_baris_terbaca.min(n);
    let mulai_idx = total_baris_terbaca - ambil_n;

    // Nomor baris 1-based dihitung relatif terhadap potongan yang terbaca
    // (bukan nomor absolut di seluruh file) — cukup untuk gutter viewer,
    // dan menghindari perlu menghitung total baris file secara terpisah.
    let nomor_awal = (mulai_idx as u64) + 1;
    let baris = baris_mentah[mulai_idx..]
        .iter()
        .enumerate()
        .map(|(i, teks)| LogLine {
            nomor: nomor_awal + i as u64,
            teks: (*teks).to_string(),
        })
        .collect();

    Ok(HasilTail { baris })
}

/// Cari baris yang mengandung `query` (case-sensitive, substring sederhana)
/// di seluruh file. Hasil dibatasi [`SEARCH_MAX_RESULTS`]; selebihnya
/// dipotong dengan `dipotong = true` — tidak pernah diam-diam terpotong.
///
/// `query` kosong berarti tanpa filter — dipakai handler untuk "Batal cari",
/// TAPI itu keputusan pemanggil (routes::logs), bukan default di sini: kalau
/// dipanggil dengan string kosong, semua baris dianggap cocok.
pub async fn cari(path: &Path, query: &str) -> Result<HasilCari, LogReadError> {
    let hasil = tokio::time::timeout(SEARCH_TIMEOUT, cari_dalam_file(path, query)).await;
    match hasil {
        Ok(inner) => inner,
        Err(_) => Err(LogReadError::Timeout),
    }
}

async fn cari_dalam_file(path: &Path, query: &str) -> Result<HasilCari, LogReadError> {
    let Some((mut file, ukuran)) = buka_untuk_baca(path).await? else {
        return Ok(HasilCari::default());
    };
    if ukuran == 0 {
        return Ok(HasilCari::default());
    }

    let mut isi = Vec::with_capacity(ukuran as usize);
    file.read_to_end(&mut isi)
        .await
        .map_err(|err| log_io_error(path, err))?;
    let teks = String::from_utf8_lossy(&isi);

    let mut cocok = Vec::new();
    let mut dipotong = false;
    for (i, baris_teks) in teks.split('\n').enumerate() {
        if baris_teks.is_empty() && i as u64 == teks.matches('\n').count() as u64 {
            // baris kosong terakhir akibat trailing newline — bukan baris data
            continue;
        }
        if query.is_empty() || baris_teks.contains(query) {
            if cocok.len() >= SEARCH_MAX_RESULTS {
                dipotong = true;
                break;
            }
            cocok.push(LogLine {
                nomor: (i as u64) + 1,
                teks: baris_teks.to_string(),
            });
        }
    }

    Ok(HasilCari {
        baris: cocok,
        dipotong,
    })
}

/// Buka file untuk dibaca. `Ok(None)` kalau file tidak ada — bukan error,
/// pemanggil merender state kosong (log belum dibuat / sudah tersapu
/// retensi).
async fn buka_untuk_baca(path: &Path) -> Result<Option<(tokio::fs::File, u64)>, LogReadError> {
    match tokio::fs::File::open(path).await {
        Ok(file) => {
            let ukuran = file
                .metadata()
                .await
                .map_err(|err| log_io_error(path, err))?
                .len();
            Ok(Some((file, ukuran)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(log_io_error(path, err)),
    }
}

async fn baca_blok(
    file: &mut tokio::fs::File,
    mulai: u64,
    panjang: u64,
) -> Result<Vec<u8>, LogReadError> {
    file.seek(SeekFrom::Start(mulai))
        .await
        .map_err(log_io_error_tanpa_path)?;
    let mut buf = vec![0u8; panjang as usize];
    file.read_exact(&mut buf)
        .await
        .map_err(log_io_error_tanpa_path)?;
    Ok(buf)
}

/// Catat error I/O ke `tracing` TANPA menyertakan path ke pemanggil —
/// path file tidak pernah boleh sampai ke klien (`docs/api-contract.md`
/// "Tidak dikembalikan").
fn log_io_error(path: &Path, err: std::io::Error) -> LogReadError {
    tracing::warn!(path = %path.display(), error = %err, "gagal membaca file log");
    LogReadError::Io
}

fn log_io_error_tanpa_path(err: std::io::Error) -> LogReadError {
    tracing::warn!(error = %err, "gagal membaca blok file log");
    LogReadError::Io
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_uji(nama: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-logreader-{nama}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("bikin dir uji harus sukses");
        dir
    }

    fn tulis_file(dir: &Path, nama: &str, isi: &str) -> std::path::PathBuf {
        let path = dir.join(nama);
        std::fs::write(&path, isi).expect("tulis file uji harus sukses");
        path
    }

    // --- nama_file_aman: anti path traversal ---

    #[test]
    fn nama_file_aman_menerima_id_alfanumerik_24_karakter() {
        let id = "abcDEF0123456789ghijKLMN";
        assert_eq!(id.len(), 24);
        assert!(nama_file_aman(id).is_ok());
    }

    #[test]
    fn nama_file_aman_menolak_titik_dua_kali() {
        assert!(matches!(
            nama_file_aman(".."),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_traversal_path_lengkap() {
        assert!(matches!(
            nama_file_aman("../../etc/passwd"),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_pemisah_direktori() {
        assert!(matches!(
            nama_file_aman("a/b"),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_string_kosong() {
        assert!(matches!(
            nama_file_aman(""),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_65_karakter() {
        let id = "a".repeat(65);
        assert!(matches!(
            nama_file_aman(&id),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menerima_tepat_64_karakter() {
        let id = "a".repeat(64);
        assert!(nama_file_aman(&id).is_ok());
    }

    #[test]
    fn nama_file_aman_menolak_percent_encoded_traversal() {
        assert!(matches!(
            nama_file_aman("%2e%2e"),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_null_byte() {
        assert!(matches!(
            nama_file_aman("abc\0def"),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_karakter_unicode() {
        assert!(matches!(
            nama_file_aman("dep_日本語"),
            Err(LogReadError::IdTidakValid)
        ));
        assert!(matches!(
            nama_file_aman("café"),
            Err(LogReadError::IdTidakValid)
        ));
    }

    #[test]
    fn nama_file_aman_menolak_underscore_dan_titik() {
        // Bukan hanya traversal — pola diperketat ke alfanumerik murni,
        // sesuai id `deployments::repo::generate_id`.
        assert!(matches!(
            nama_file_aman("dep_123"),
            Err(LogReadError::IdTidakValid)
        ));
        assert!(matches!(
            nama_file_aman("dep.log"),
            Err(LogReadError::IdTidakValid)
        ));
    }

    // --- tail ---

    #[tokio::test]
    async fn tail_file_tidak_ada_mengembalikan_kosong_bukan_error() {
        let dir = dir_uji("tail-tidak-ada");
        let path = dir.join("tidak-ada.log");

        let hasil = tail(&path, 500).await.expect("tail harus sukses");
        assert!(hasil.baris.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_file_kosong_mengembalikan_kosong() {
        let dir = dir_uji("tail-kosong");
        let path = tulis_file(&dir, "dep.log", "");

        let hasil = tail(&path, 500).await.expect("tail harus sukses");
        assert!(hasil.baris.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_mengembalikan_n_baris_terakhir_dalam_urutan_benar() {
        let dir = dir_uji("tail-normal");
        let isi: String = (1..=10).map(|i| format!("baris-{i}\n")).collect();
        let path = tulis_file(&dir, "dep.log", &isi);

        let hasil = tail(&path, 3).await.expect("tail harus sukses");
        let teks: Vec<&str> = hasil.baris.iter().map(|b| b.teks.as_str()).collect();
        assert_eq!(teks, vec!["baris-8", "baris-9", "baris-10"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_meminta_lebih_banyak_dari_isi_file_mengembalikan_semua_baris() {
        let dir = dir_uji("tail-lebih");
        let isi = "a\nb\nc\n";
        let path = tulis_file(&dir, "dep.log", isi);

        let hasil = tail(&path, 100).await.expect("tail harus sukses");
        let teks: Vec<&str> = hasil.baris.iter().map(|b| b.teks.as_str()).collect();
        assert_eq!(teks, vec!["a", "b", "c"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_nol_dijepit_ke_default_bukan_error() {
        let dir = dir_uji("tail-nol");
        let isi: String = (1..=10).map(|i| format!("baris-{i}\n")).collect();
        let path = tulis_file(&dir, "dep.log", &isi);

        let hasil = tail(&path, 0).await.expect("tail harus sukses");
        assert_eq!(hasil.baris.len(), 10, "0 dijepit ke default, bukan 0 baris");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_melebihi_maksimum_dijepit_bukan_error() {
        let dir = dir_uji("tail-maks");
        let isi: String = (1..=5).map(|i| format!("baris-{i}\n")).collect();
        let path = tulis_file(&dir, "dep.log", &isi);

        // Meminta lebih dari TAIL_MAX tidak boleh gagal — dijepit, lalu tetap
        // mengembalikan seluruh isi file (yang jauh lebih pendek dari batas).
        let hasil = tail(&path, TAIL_MAX + 1000)
            .await
            .expect("tail harus sukses, bukan 400");
        assert_eq!(hasil.baris.len(), 5);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_bekerja_pada_file_besar_multi_chunk() {
        // Melebihi CHUNK_SIZE (64 KiB) supaya jalur pembesaran blok teruji,
        // bukan hanya jalur baca-sekali-cukup.
        let dir = dir_uji("tail-besar");
        let baris_panjang = "x".repeat(200);
        let mut isi = String::new();
        for i in 0..2000 {
            isi.push_str(&format!("{i}-{baris_panjang}\n"));
        }
        let path = tulis_file(&dir, "dep.log", &isi);

        let hasil = tail(&path, 10).await.expect("tail harus sukses");
        assert_eq!(hasil.baris.len(), 10);
        assert!(hasil.baris[9].teks.starts_with("1999-"));
        assert!(hasil.baris[0].teks.starts_with("1990-"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tail_file_tanpa_trailing_newline_tetap_benar() {
        let dir = dir_uji("tail-no-newline");
        let path = tulis_file(&dir, "dep.log", "a\nb\nc");

        let hasil = tail(&path, 100).await.expect("tail harus sukses");
        let teks: Vec<&str> = hasil.baris.iter().map(|b| b.teks.as_str()).collect();
        assert_eq!(teks, vec!["a", "b", "c"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- cari ---

    #[tokio::test]
    async fn cari_file_tidak_ada_mengembalikan_kosong() {
        let dir = dir_uji("cari-tidak-ada");
        let path = dir.join("tidak-ada.log");

        let hasil = cari(&path, "apa saja").await.expect("cari harus sukses");
        assert!(hasil.baris.is_empty());
        assert!(!hasil.dipotong);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cari_mengembalikan_baris_yang_cocok_saja() {
        let dir = dir_uji("cari-cocok");
        let isi = "info: mulai\nerror: gagal koneksi\ninfo: selesai\n";
        let path = tulis_file(&dir, "dep.log", isi);

        let hasil = cari(&path, "error").await.expect("cari harus sukses");
        assert_eq!(hasil.baris.len(), 1);
        assert_eq!(hasil.baris[0].teks, "error: gagal koneksi");
        assert!(!hasil.dipotong);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cari_query_kosong_mengembalikan_semua_baris() {
        let dir = dir_uji("cari-kosong-query");
        let isi = "a\nb\nc\n";
        let path = tulis_file(&dir, "dep.log", isi);

        let hasil = cari(&path, "").await.expect("cari harus sukses");
        assert_eq!(hasil.baris.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cari_melebihi_batas_hasil_menandai_dipotong() {
        let dir = dir_uji("cari-dipotong");
        let isi: String = (0..600).map(|i| format!("cocok-{i}\n")).collect();
        let path = tulis_file(&dir, "dep.log", &isi);

        let hasil = cari(&path, "cocok").await.expect("cari harus sukses");
        assert_eq!(hasil.baris.len(), SEARCH_MAX_RESULTS);
        assert!(
            hasil.dipotong,
            "hasil yang melebihi batas wajib ditandai, tidak boleh diam-diam terpotong"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cari_tidak_melebihi_batas_tidak_menandai_dipotong() {
        let dir = dir_uji("cari-tidak-dipotong");
        let isi: String = (0..10).map(|i| format!("cocok-{i}\n")).collect();
        let path = tulis_file(&dir, "dep.log", &isi);

        let hasil = cari(&path, "cocok").await.expect("cari harus sukses");
        assert_eq!(hasil.baris.len(), 10);
        assert!(!hasil.dipotong);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
