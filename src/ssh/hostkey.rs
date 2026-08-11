//! Known_hosts MILIK APLIKASI (bukan `~/.ssh/known_hosts` pengguna sistem),
//! pengambilan fingerprint host key, dan penulisan entri setelah pengguna
//! mengonfirmasi TOFU (`docs/design/tambah-server.md` §4.2 poin 6).
//!
//! Fingerprint host key BUKAN secret (`docs/api-contract.md`, Fase 1) —
//! boleh dikembalikan ke klien dan ditampilkan. Yang tidak boleh pernah
//! ikut adalah isi mentah entri known_hosts (kunci publik penuh) di luar
//! kebutuhan penulisan file itu sendiri.
//!
//! Pengambilan fingerprint memakai `ssh-keyscan` (ambil baris host key
//! mentah) lalu `ssh-keygen -lf` (hitung fingerprint SHA256 darinya) —
//! dua binary sistem yang sama-sama dipakai `openssh` secara tidak
//! langsung, dijalankan lewat `tokio::process::Command` supaya bisa
//! diberi timeout dan dibatalkan (`kill_on_drop`) kalau macet.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::session::TempFile;

/// Nama file known_hosts milik aplikasi di dalam `runtime_dir` (bukan
/// `~/.ssh/known_hosts`). Satu file untuk semua server — setiap baris
/// diawali alamat `[host]:port` sehingga entri antar server tidak
/// bertabrakan.
const KNOWN_HOSTS_FILE_NAME: &str = "known_hosts";

/// Hasil pengambilan fingerprint host key, belum tersimpan permanen.
pub struct FingerprintProbe {
    /// Format tampilan standar `ssh-keygen -l`: `SHA256:base64...`. Aman
    /// ditampilkan ke pengguna — bukan secret.
    pub fingerprint: String,
    /// Baris known_hosts mentah (`[host]:port key-type key-data`) yang
    /// ditulis ke known_hosts aplikasi HANYA setelah pengguna konfirmasi
    /// lewat `confirm_and_store`. Bukan private key — ini kunci PUBLIK
    /// host, aman disimpan tapi tetap tidak perlu ditampilkan ke klien.
    known_hosts_entry: String,
}

/// Alias yang dipakai `session.rs` — nama publik modul ini tetap
/// `FingerprintProbe` untuk konsumen lain lewat re-export `mod.rs`.
pub type HostKeyProbe = FingerprintProbe;

/// Kegagalan saat mengambil fingerprint host key.
#[derive(Debug)]
pub enum HostKeyError {
    /// Batas waktu tercapai sebelum host key terbaca.
    Timeout,
    /// `ssh-keyscan` tidak mengembalikan baris host key sama sekali
    /// (host tidak terjangkau, port SSH tertutup, dsb).
    Unreachable,
    /// Output `ssh-keyscan`/`ssh-keygen` tidak bisa diparse jadi
    /// fingerprint. Sinyal kemungkinan binary sistem tidak ada / versi
    /// tidak kompatibel — bukan masalah jaringan.
    ParseFailed,
    /// Kegagalan IO menjalankan proses (binary tidak ditemukan, dsb).
    Io(String),
}

/// Ambil fingerprint host key `host:port` TANPA menyimpannya secara
/// permanen. Dipanggil dari `session::connect` sebelum autentikasi kunci
/// dicoba. Timeout dibagi rata antara dua sub-proses (`ssh-keyscan` lalu
/// `ssh-keygen`) supaya jumlahnya tidak melebihi `overall_timeout`.
pub(super) async fn probe(
    host: &str,
    port: u16,
    runtime_dir: &Path,
    overall_timeout: Duration,
) -> Result<FingerprintProbe, HostKeyError> {
    let half = overall_timeout / 2;

    let raw_line = run_ssh_keyscan(host, port, half).await?;
    let fingerprint = run_ssh_keygen_fingerprint(&raw_line, runtime_dir, half).await?;

    Ok(FingerprintProbe {
        fingerprint,
        known_hosts_entry: raw_line,
    })
}

/// Dipakai murni untuk pengambilan fingerprint tanpa membangun koneksi
/// penuh (mis. kalau suatu saat dibutuhkan endpoint terpisah). Sub-blok
/// 3d/3f tidak wajib memakai ini — `session::connect` sudah memanggil
/// `probe` secara internal.
pub async fn fetch_fingerprint_via_keyscan(
    host: &str,
    port: u16,
    runtime_dir: &Path,
    timeout: Duration,
) -> Result<FingerprintProbe, HostKeyError> {
    probe(host, port, runtime_dir, timeout).await
}

async fn run_ssh_keyscan(host: &str, port: u16, timeout: Duration) -> Result<String, HostKeyError> {
    let timeout_secs = timeout.as_secs().max(1);

    let mut command = Command::new("ssh-keyscan");
    command
        .arg("-T")
        .arg(timeout_secs.to_string())
        .arg("-p")
        .arg(port.to_string())
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| HostKeyError::Timeout)?
        .map_err(|err| HostKeyError::Io(err.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .collect();

    let chosen = pilih_baris_host_key(&lines);
    chosen.map(str::to_string).ok_or(HostKeyError::Unreachable)
}

/// Pilih baris host key secara DETERMINISTIK dari output `ssh-keyscan`.
///
/// `ssh-keyscan` mengembalikan SEMUA host key yang ditawarkan server
/// (biasanya RSA, ECDSA, dan ED25519) dengan URUTAN YANG TIDAK STABIL antar
/// eksekusi — memakai baris pertama membuat fingerprint yang dihitung bisa
/// berubah walau host tidak berubah sama sekali, dan server online akan
/// dilaporkan palsu `host_key_berubah`. Fungsi murni (dites langsung tanpa
/// proses): pilih dengan preferensi tetap `ed25519 > ecdsa > rsa`, sisanya
/// sebagai cadangan terakhir. Fungsi ini konsisten dipakai di verifikasi
/// (TOFU), konfirmasi ulang, dan polling sehingga fingerprint yang tersimpan
/// tidak pernah berubah tanpa alasan.
fn pilih_baris_host_key<'a>(lines: &'a [&'a str]) -> Option<&'a str> {
    lines.iter().copied().min_by_key(|line| {
        let key_type = line.split_whitespace().nth(1).unwrap_or("");
        preferensi_key_type(key_type)
    })
}

/// Bobot preferensi jenis host key — makin kecil makin diutamakan.
/// `ssh-ed25519` modern dan unik; `ecdsa-*` berikutnya; `rsa-*` cadangan
/// untuk server tua yang tidak punya host key lain.
fn preferensi_key_type(key_type: &str) -> u8 {
    match key_type {
        "ssh-ed25519" => 0,
        "ecdsa-sha2-nistp256" => 1,
        "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => 2,
        "ssh-rsa" | "rsa-sha2-256" | "rsa-sha2-512" => 3,
        _ => 4,
    }
}

async fn run_ssh_keygen_fingerprint(
    raw_line: &str,
    runtime_dir: &Path,
    timeout: Duration,
) -> Result<String, HostKeyError> {
    // `ssh-keygen -lf` butuh path file (tidak menerima stdin di semua
    // versi secara konsisten), jadi baris mentah ditulis ke file
    // sementara mode 0600 lalu dihapus otomatis lewat `TempFile::Drop`,
    // sama seperti file kunci privat di `session.rs`.
    let temp = TempFile::write(runtime_dir, "hostkey", raw_line.as_bytes())
        .map_err(|err| HostKeyError::Io(err.to_string()))?;

    let mut command = Command::new("ssh-keygen");
    command
        .arg("-lf")
        .arg(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| HostKeyError::Timeout)?
        .map_err(|err| HostKeyError::Io(err.to_string()))?;

    drop(temp);

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_fingerprint(&stdout).ok_or(HostKeyError::ParseFailed)
}

/// Ekstrak token `SHA256:...` dari output `ssh-keygen -lf`, contoh:
/// `256 SHA256:abc123xyz789... host (ED25519)`. Fungsi murni — dites
/// tanpa menjalankan proses apa pun.
fn parse_fingerprint(ssh_keygen_output: &str) -> Option<String> {
    ssh_keygen_output
        .split_whitespace()
        .find(|token| token.starts_with("SHA256:"))
        .map(str::to_string)
}

/// Path known_hosts milik aplikasi di dalam `runtime_dir`.
fn known_hosts_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(KNOWN_HOSTS_FILE_NAME)
}

/// Tulis entri known_hosts SETELAH pengguna eksplisit mengonfirmasi
/// fingerprint yang ditawarkan (`POST /servers/{id}/hostkey/konfirmasi`,
/// disambungkan sub-blok 3f). Menambah baris baru — tidak pernah menimpa
/// entri lama untuk host lain (invariant 1: tidak destruktif).
///
/// File dibuat mode `0600` kalau belum ada; direktori induk `0700`
/// (`docs/plan.md`, "izin file 0600").
pub fn confirm_and_store(runtime_dir: &Path, probe: &FingerprintProbe) -> Result<(), HostKeyError> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::create_dir_all(runtime_dir).map_err(|err| HostKeyError::Io(err.to_string()))?;
    set_mode(runtime_dir, 0o700).map_err(|err| HostKeyError::Io(err.to_string()))?;

    let path = known_hosts_path(runtime_dir);
    let file_existed = path.exists();

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .map_err(|err| HostKeyError::Io(err.to_string()))?;

    if !file_existed {
        // `OpenOptions::mode` hanya berlaku saat file BARU dibuat (umask
        // masih bisa memengaruhi); pastikan mode final tetap 0600 walau
        // umask proses longgar.
        set_mode(&path, 0o600).map_err(|err| HostKeyError::Io(err.to_string()))?;
    }

    use io::Write;
    writeln!(file, "{}", probe.known_hosts_entry)
        .map_err(|err| HostKeyError::Io(err.to_string()))?;
    file.sync_all()
        .map_err(|err| HostKeyError::Io(err.to_string()))?;

    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fingerprint_mengambil_token_sha256() {
        let output = "256 SHA256:abc123XYZ789+/= root@vps (ED25519)\n";
        assert_eq!(
            parse_fingerprint(output),
            Some("SHA256:abc123XYZ789+/=".to_string())
        );
    }

    #[test]
    fn parse_fingerprint_none_kalau_tidak_ada_token_sha256() {
        let output = "ssh-keygen: command not found\n";
        assert_eq!(parse_fingerprint(output), None);
    }

    #[test]
    fn parse_fingerprint_none_untuk_string_kosong() {
        assert_eq!(parse_fingerprint(""), None);
    }

    #[test]
    fn pilih_baris_mengutamakan_ed25519_walau_bukan_baris_pertama() {
        let lines = [
            "192.168.8.104 ssh-rsa AAAA-RSA",
            "192.168.8.104 ecdsa-sha2-nistp256 AAAA-ECDSA",
            "192.168.8.104 ssh-ed25519 AAAA-ED25519",
        ];
        assert_eq!(pilih_baris_host_key(&lines), Some(lines[2]));
    }

    #[test]
    fn pilih_baris_deterministik_tidak_bergantung_urutan_input() {
        // Urutan output `ssh-keyscan` bisa acak antar eksekusi — hasil
        // pilihan WAJIB sama untuk susunan yang berbeda (bug host_key_berubah
        // palsu berasal dari ketergantungan urutan ini).
        let urutan_a = [
            "h ssh-rsa AAAA-RSA",
            "h ecdsa-sha2-nistp256 AAAA-ECDSA",
            "h ssh-ed25519 AAAA-ED25519",
        ];
        let urutan_b = [
            "h ecdsa-sha2-nistp256 AAAA-ECDSA",
            "h ssh-ed25519 AAAA-ED25519",
            "h ssh-rsa AAAA-RSA",
        ];
        assert_eq!(
            pilih_baris_host_key(&urutan_a),
            Some("h ssh-ed25519 AAAA-ED25519")
        );
        assert_eq!(
            pilih_baris_host_key(&urutan_b),
            Some("h ssh-ed25519 AAAA-ED25519")
        );
    }

    #[test]
    fn pilih_baris_jatuh_ke_ecdsa_saat_ed25519_tidak_ada() {
        let lines = ["h ssh-rsa AAAA-RSA", "h ecdsa-sha2-nistp256 AAAA-ECDSA"];
        assert_eq!(
            pilih_baris_host_key(&lines),
            Some("h ecdsa-sha2-nistp256 AAAA-ECDSA")
        );
    }

    #[test]
    fn pilih_baris_jatuh_ke_rsa_saat_hanya_rsa_yang_tersedia() {
        let lines = ["h ssh-rsa AAAA-RSA"];
        assert_eq!(pilih_baris_host_key(&lines), Some("h ssh-rsa AAAA-RSA"));
    }

    #[test]
    fn pilih_baris_none_untuk_input_kosong() {
        assert_eq!(pilih_baris_host_key(&[]), None);
    }

    #[test]
    fn confirm_and_store_menulis_file_mode_0600_dan_menambah_baris() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-hostkey-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("waktu sistem harus valid")
                .as_nanos()
        ));

        let probe1 = FingerprintProbe {
            fingerprint: "SHA256:aaa".to_string(),
            known_hosts_entry: "[host1]:22 ssh-ed25519 AAAA...".to_string(),
        };
        confirm_and_store(&dir, &probe1).expect("simpan entri pertama harus sukses");

        let path = known_hosts_path(&dir);
        let mode = std::fs::metadata(&path)
            .expect("metadata known_hosts harus terbaca")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "known_hosts aplikasi harus bermode 0600");

        let probe2 = FingerprintProbe {
            fingerprint: "SHA256:bbb".to_string(),
            known_hosts_entry: "[host2]:22 ssh-ed25519 BBBB...".to_string(),
        };
        confirm_and_store(&dir, &probe2).expect("simpan entri kedua harus sukses");

        let isi = std::fs::read_to_string(&path).expect("baca known_hosts harus sukses");
        assert!(isi.contains("host1"), "entri lama tidak boleh hilang");
        assert!(isi.contains("host2"), "entri baru harus ditambahkan");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_and_store_direktori_induk_bermode_0700() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-hostkey-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("waktu sistem harus valid")
                .as_nanos()
        ));

        let probe = FingerprintProbe {
            fingerprint: "SHA256:ccc".to_string(),
            known_hosts_entry: "[host3]:22 ssh-ed25519 CCCC...".to_string(),
        };
        confirm_and_store(&dir, &probe).expect("simpan entri harus sukses");

        let mode = std::fs::metadata(&dir)
            .expect("metadata direktori runtime harus terbaca")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "direktori runtime harus bermode 0700");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
