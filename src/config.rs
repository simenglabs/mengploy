//! Konfigurasi aplikasi, dimuat dari environment variable.
//!
//! Fase 0 tidak punya file config terpisah — semua lewat env, opsional
//! di-load dari `.env` saat dev lewat `dotenvy` (dipanggil di `main`).

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Alamat listen default kalau `MENGDEP_LISTEN_ADDR` tidak diset.
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:3000";

/// Path database default kalau `MENGDEP_DB_PATH` tidak diset.
const DEFAULT_DB_PATH: &str = "./data/mengdep.db";

/// Konfigurasi yang dimuat sekali saat startup.
///
/// Sengaja tidak derive `Debug` — kalaupun field di sini tidak memuat secret
/// langsung hari ini, menghindari derive otomatis mencegah kebocoran diam-diam
/// kalau field sensitif ditambahkan nanti (mis. path kunci) tanpa ada yang
/// sadar `Debug` ikut mencetaknya.
pub struct Config {
    /// Alamat + port tempat server mendengarkan.
    pub listen_addr: String,
    /// Path file database SQLite.
    pub db_path: PathBuf,
    /// Password awal untuk seed `settings.password_hash` saat startup pertama.
    /// `None` kalau env tidak diset (berarti settings sudah harus ter-seed).
    pub initial_password: Option<String>,
    /// Path file kunci enkripsi `age`. WAJIB sejak Fase 1 — dipakai untuk
    /// enkripsi/dekripsi private key SSH dan token registry (invariant PRD
    /// §3 nomor 8: kunci tidak pernah di dalam db). Tetap `Option` di tipe
    /// karena environment variable bisa tidak diset; kewajibannya
    /// ditegakkan di `verify_encryption_key_permissions()` yang sekarang
    /// gagal fatal (bukan `Ok(())`) kalau path ini `None`.
    pub encryption_key_path: Option<PathBuf>,
    /// Direktori privat aplikasi untuk file sensitif berumur pendek: private
    /// key SSH yang didekripsi (hidup hanya selama proses connect,
    /// `src/ssh/session.rs::TempFile`) dan socket forward Docker
    /// (`src/docker/forward.rs`). **WAJIB tmpfs** (`CLAUDE.md` §5 invariant
    /// 13, §6): default `/run/platform/ssh`. Override lewat
    /// `MENGDEP_RUNTIME_DIR` adalah keputusan EKSPLISIT operator (mis. dev
    /// di macOS yang tidak punya `/run`) — aplikasi sendiri TIDAK PERNAH
    /// diam-diam jatuh ke `/tmp` kalau `/run` tidak tersedia; lihat
    /// `verify_runtime_dir_available()`.
    pub runtime_dir: PathBuf,
    /// Direktori log deploy (`docs/plan.md` Fase 3, tabel "Angka yang
    /// dikunci"). Default `/var/lib/platform/logs`, override lewat
    /// `MENGDEP_LOG_DIR` — perilaku identik `runtime_dir`: gagal startup
    /// kalau tidak bisa dibuat/di-chmod, TIDAK PERNAH diam-diam jatuh ke
    /// `./data/logs` (Q3, `docs/plan.md`, belum dijawab manusia — asumsi
    /// sementara planner dipakai di sini).
    pub log_dir: PathBuf,
    /// Umur maksimum (hari) file log deploy sebelum disapu retensi
    /// (`docs/plan.md` Fase 3). Default 30, override `MENGDEP_LOG_RETENTION_DAYS`,
    /// rentang sah 1-3650 — di luar itu gagal startup, bukan di-clamp.
    pub log_retention_days: u32,
}

/// Default `runtime_dir` kalau `MENGDEP_RUNTIME_DIR` tidak diset —
/// `CLAUDE.md` §6 "Layout on-disk (control plane)".
const DEFAULT_RUNTIME_DIR: &str = "/run/platform/ssh";

/// Default `log_dir` kalau `MENGDEP_LOG_DIR` tidak diset —
/// `docs/plan.md` Fase 3, tabel "Angka yang dikunci".
const DEFAULT_LOG_DIR: &str = "/var/lib/platform/logs";

/// Default `log_retention_days` kalau `MENGDEP_LOG_RETENTION_DAYS` tidak diset.
const DEFAULT_LOG_RETENTION_DAYS: u32 = 30;

/// Rentang sah `MENGDEP_LOG_RETENTION_DAYS` — `docs/plan.md` Fase 3.
const LOG_RETENTION_DAYS_MIN: u32 = 1;
const LOG_RETENTION_DAYS_MAX: u32 = 3650;

impl Config {
    /// Muat konfigurasi dari environment variable.
    pub fn from_env() -> Result<Self> {
        let listen_addr = std::env::var("MENGDEP_LISTEN_ADDR")
            .unwrap_or_else(|_| DEFAULT_LISTEN_ADDR.to_string());

        let db_path = std::env::var("MENGDEP_DB_PATH")
            .unwrap_or_else(|_| DEFAULT_DB_PATH.to_string())
            .into();

        // Opsi (a) dari docs/plan.md "Utang Fase 0" temuan #2: string
        // kosong/whitespace-only diperlakukan sama seperti env var tidak
        // diset (None), bukan menggagalkan startup. Alasan: env var kosong
        // biasanya berasal dari template `.env`/systemd unit yang belum
        // diisi operator, bukan niat eksplisit — menggagalkan startup untuk
        // kasus ini bertentangan dengan invariant 1 ("jangan bertindak keras
        // karena sesuatu belum lengkap") dan aplikasi tetap berguna tanpa
        // user pertama (login sekadar tertunda sampai env diisi + restart).
        // Yang tidak boleh terjadi: password kosong lolos ke hash_password —
        // trim di sini memastikan whitespace-only juga tertangkap sebagai
        // "kosong", bukan hanya string panjang nol.
        let initial_password = std::env::var("MENGDEP_INITIAL_PASSWORD")
            .ok()
            .filter(|value| !value.trim().is_empty());

        let encryption_key_path = std::env::var("MENGDEP_KEY_PATH").ok().map(PathBuf::from);

        let runtime_dir = std::env::var("MENGDEP_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_RUNTIME_DIR));

        let log_dir = std::env::var("MENGDEP_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_LOG_DIR));

        let log_retention_days =
            parse_log_retention_days(std::env::var("MENGDEP_LOG_RETENTION_DAYS").ok())
                .context("MENGDEP_LOG_RETENTION_DAYS tidak valid")?;

        Ok(Self {
            listen_addr,
            db_path,
            initial_password,
            encryption_key_path,
            runtime_dir,
            log_dir,
            log_retention_days,
        })
    }

    /// Pastikan `runtime_dir` bisa dibuat dan ditulis. TIDAK PERNAH mencoba
    /// path lain kalau ini gagal — kegagalan di sini WAJIB menggagalkan
    /// startup dengan pesan jelas (invariant 13: "kalau `/run` tidak
    /// tersedia, gagal dan katakan — jangan diam-diam jatuh ke `/tmp`").
    /// Operator yang perlu path lain (mis. dev tanpa `/run`) mengatasi ini
    /// lewat `MENGDEP_RUNTIME_DIR`, sebuah pilihan eksplisit, bukan fallback
    /// otomatis aplikasi.
    pub fn verify_runtime_dir_available(&self) -> Result<()> {
        std::fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "direktori runtime {} tidak bisa dibuat/ditulis — kunci SSH dan socket forward \
                 Docker butuh tmpfs privat di sini (invariant 13). Kalau `/run` tidak tersedia \
                 di mesin ini, set MENGDEP_RUNTIME_DIR eksplisit ke path tmpfs lain; aplikasi \
                 tidak akan pernah otomatis memilih /tmp.",
                self.runtime_dir.display()
            )
        })?;

        set_mode(&self.runtime_dir, 0o700).with_context(|| {
            format!(
                "gagal mengeset mode 0700 pada direktori runtime {}",
                self.runtime_dir.display()
            )
        })
    }

    /// Verifikasi kunci enkripsi `age` tersedia dan bermode `0600`. WAJIB
    /// sejak Fase 1 — kunci ini dipakai untuk mengenkripsi private key SSH
    /// dan token registry sebelum masuk db (invariant PRD §3 nomor 8).
    /// Path tidak diset adalah kegagalan fatal, bukan `Ok(())`.
    pub fn verify_encryption_key_permissions(&self) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let Some(path) = &self.encryption_key_path else {
            anyhow::bail!(
                "MENGDEP_KEY_PATH belum diset — kunci enkripsi age wajib sejak Fase 1. \
                 Set env var ini ke path file identity age bermode 0600."
            );
        };

        let metadata = std::fs::metadata(path)
            .with_context(|| format!("baca metadata file kunci enkripsi {}", path.display()))?;

        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            anyhow::bail!(
                "file kunci enkripsi harus bermode 0600, saat ini {:o}",
                mode
            );
        }

        Ok(())
    }

    /// Pastikan `log_dir` dan sub-direktori `log_dir/deploy/` bisa
    /// dibuat/ditulis, keduanya bermode `0700`. Meniru
    /// `verify_runtime_dir_available()` persis: gagal di sini WAJIB
    /// menggagalkan startup, TANPA fallback diam-diam ke path lain.
    pub fn verify_log_dir_available(&self) -> Result<()> {
        std::fs::create_dir_all(&self.log_dir).with_context(|| {
            format!(
                "direktori log {} tidak bisa dibuat/ditulis — log deploy butuh direktori ini. \
                 Kalau path default tidak tersedia di mesin ini, set MENGDEP_LOG_DIR eksplisit \
                 ke path lain; aplikasi tidak akan pernah otomatis memilih path fallback lain.",
                self.log_dir.display()
            )
        })?;
        set_mode(&self.log_dir, 0o700).with_context(|| {
            format!(
                "gagal mengeset mode 0700 pada direktori log {}",
                self.log_dir.display()
            )
        })?;

        let deploy_dir = self.log_dir.join("deploy");
        std::fs::create_dir_all(&deploy_dir).with_context(|| {
            format!(
                "direktori log deploy {} tidak bisa dibuat/ditulis — set MENGDEP_LOG_DIR ke \
                 path lain kalau ini tidak tersedia.",
                deploy_dir.display()
            )
        })?;
        set_mode(&deploy_dir, 0o700).with_context(|| {
            format!(
                "gagal mengeset mode 0700 pada direktori log deploy {}",
                deploy_dir.display()
            )
        })
    }
}

/// Parse dan validasi `MENGDEP_LOG_RETENTION_DAYS`. `None` (env tidak diset)
/// → default. Nilai yang tidak bisa di-parse atau di luar rentang 1-3650
/// adalah error fatal — TIDAK di-clamp diam-diam (`docs/plan.md` Fase 3).
fn parse_log_retention_days(raw: Option<String>) -> Result<u32> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_LOG_RETENTION_DAYS);
    };

    let value: u32 = raw
        .trim()
        .parse()
        .with_context(|| format!("nilai '{raw}' bukan angka bulat positif"))?;

    if !(LOG_RETENTION_DAYS_MIN..=LOG_RETENTION_DAYS_MAX).contains(&value) {
        anyhow::bail!(
            "nilai {value} di luar rentang sah {LOG_RETENTION_DAYS_MIN}-{LOG_RETENTION_DAYS_MAX} hari"
        );
    }

    Ok(value)
}

fn set_mode(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
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
    fn default_dipakai_kalau_env_kosong() {
        // ponytail: test ini tidak menyentuh env var proses nyata supaya tidak
        // mengganggu test lain yang jalan paralel; verifikasi konstanta saja.
        assert_eq!(DEFAULT_LISTEN_ADDR, "127.0.0.1:3000");
        assert_eq!(DEFAULT_DB_PATH, "./data/mengdep.db");
    }

    #[test]
    fn initial_password_kosong_atau_spasi_diperlakukan_sebagai_none() {
        // ponytail: manipulasi env var proses nyata di sini sengaja dihindari
        // di test lain (lihat komentar default_dipakai_kalau_env_kosong)
        // supaya tidak mengganggu test paralel; test ini mengulang logika
        // filter yang sama secara langsung terhadap Option<String> supaya
        // tetap deterministik tanpa menyentuh env var global.
        let filter = |raw: Option<&str>| -> Option<String> {
            raw.map(str::to_string)
                .filter(|value| !value.trim().is_empty())
        };

        assert_eq!(filter(Some("")), None, "string kosong harus jadi None");
        assert_eq!(filter(Some("   ")), None, "whitespace-only harus jadi None");
        assert_eq!(
            filter(Some("\t\n ")),
            None,
            "campuran whitespace harus jadi None"
        );
        assert_eq!(
            filter(Some("rahasia")),
            Some("rahasia".to_string()),
            "password non-kosong harus tetap lolos"
        );
        assert_eq!(filter(None), None, "env tidak diset harus tetap None");
    }

    #[test]
    fn verify_encryption_key_permissions_error_kalau_path_tidak_diset() {
        let config = Config {
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            db_path: DEFAULT_DB_PATH.into(),
            initial_password: None,
            encryption_key_path: None,
            runtime_dir: PathBuf::from(DEFAULT_RUNTIME_DIR),
            log_dir: PathBuf::from(DEFAULT_LOG_DIR),
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
        };

        assert!(
            config.verify_encryption_key_permissions().is_err(),
            "path kunci enkripsi kosong wajib fatal sejak Fase 1"
        );
    }

    #[test]
    fn verify_encryption_key_permissions_error_kalau_mode_bukan_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-config-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("waktu sistem harus valid")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("bikin direktori sementara harus sukses");
        let path = dir.join("key.txt");
        std::fs::write(&path, "isi-dummy").expect("tulis file kunci harus sukses");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set mode longgar harus sukses");

        let config = Config {
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            db_path: DEFAULT_DB_PATH.into(),
            initial_password: None,
            encryption_key_path: Some(path),
            runtime_dir: PathBuf::from(DEFAULT_RUNTIME_DIR),
            log_dir: PathBuf::from(DEFAULT_LOG_DIR),
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
        };

        assert!(
            config.verify_encryption_key_permissions().is_err(),
            "mode selain 0600 harus ditolak"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_encryption_key_permissions_ok_kalau_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-config-key-ok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("waktu sistem harus valid")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("bikin direktori sementara harus sukses");
        let path = dir.join("key.txt");
        std::fs::write(&path, "isi-dummy").expect("tulis file kunci harus sukses");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("set mode 0600 harus sukses");

        let config = Config {
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            db_path: DEFAULT_DB_PATH.into(),
            initial_password: None,
            encryption_key_path: Some(path),
            runtime_dir: PathBuf::from(DEFAULT_RUNTIME_DIR),
            log_dir: PathBuf::from(DEFAULT_LOG_DIR),
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
        };

        assert!(config.verify_encryption_key_permissions().is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_runtime_dir_available_membuat_direktori_dan_set_mode_0700() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-runtime-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("waktu sistem harus valid")
                .as_nanos()
        ));
        // Sengaja TIDAK dibuat lebih dulu — verifikasi bahwa fungsi ini yang
        // membuatnya (skenario `/run/platform/ssh` belum ada saat boot).

        let config = Config {
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            db_path: DEFAULT_DB_PATH.into(),
            initial_password: None,
            encryption_key_path: None,
            runtime_dir: dir.clone(),
            log_dir: PathBuf::from(DEFAULT_LOG_DIR),
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
        };

        assert!(config.verify_runtime_dir_available().is_ok());

        let mode = std::fs::metadata(&dir)
            .expect("direktori runtime harus ada setelah verifikasi")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "direktori runtime harus bermode 0700");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_log_dir_available_membuat_direktori_dan_sub_deploy_mode_0700() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "mengdep-test-log-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("waktu sistem harus valid")
                .as_nanos()
        ));

        let config = Config {
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            db_path: DEFAULT_DB_PATH.into(),
            initial_password: None,
            encryption_key_path: None,
            runtime_dir: PathBuf::from(DEFAULT_RUNTIME_DIR),
            log_dir: dir.clone(),
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
        };

        assert!(config.verify_log_dir_available().is_ok());

        let mode = std::fs::metadata(&dir)
            .expect("direktori log harus ada setelah verifikasi")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "direktori log harus bermode 0700");

        let deploy_dir = dir.join("deploy");
        let deploy_mode = std::fs::metadata(&deploy_dir)
            .expect("sub-direktori deploy harus ada setelah verifikasi")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            deploy_mode, 0o700,
            "sub-direktori deploy harus bermode 0700"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_log_retention_days_default_kalau_env_tidak_diset() {
        assert_eq!(
            parse_log_retention_days(None).expect("default harus Ok"),
            DEFAULT_LOG_RETENTION_DAYS
        );
    }

    #[test]
    fn parse_log_retention_days_terima_nilai_valid_di_dalam_rentang() {
        assert_eq!(
            parse_log_retention_days(Some("1".to_string())).expect("1 harus valid"),
            1
        );
        assert_eq!(
            parse_log_retention_days(Some("3650".to_string())).expect("3650 harus valid"),
            3650
        );
        assert_eq!(
            parse_log_retention_days(Some("60".to_string())).expect("60 harus valid"),
            60
        );
    }

    #[test]
    fn parse_log_retention_days_tolak_nol() {
        assert!(
            parse_log_retention_days(Some("0".to_string())).is_err(),
            "0 hari di luar rentang sah, harus ditolak"
        );
    }

    #[test]
    fn parse_log_retention_days_tolak_lebih_dari_3650() {
        assert!(
            parse_log_retention_days(Some("3651".to_string())).is_err(),
            "3651 hari di luar rentang sah, harus ditolak"
        );
    }

    #[test]
    fn parse_log_retention_days_tolak_bukan_angka() {
        assert!(
            parse_log_retention_days(Some("banyak".to_string())).is_err(),
            "nilai non-angka harus ditolak"
        );
        assert!(
            parse_log_retention_days(Some("-5".to_string())).is_err(),
            "nilai negatif harus ditolak (u32 tidak bisa menampung minus)"
        );
    }
}
