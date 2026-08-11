//! Entrypoint: init tracing, config, pool, migrate, router, serve, shutdown.
//!
//! Modul domain dideklarasikan di `src/lib.rs` supaya `tests/` (integration
//! test, crate terpisah) bisa `use mengdep::...`. File ini hanya urutan
//! startup — tidak ada logika domain di sini.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::events::EventRegistry;
use mengdep::state::AppState;
use mengdep::{apps, auth, db, deployments, routes};

#[tokio::main]
async fn main() -> Result<()> {
    // Opsional untuk dev — ketiadaan `.env` tidak menggagalkan startup
    // (Q4, docs/plan.md). Produksi memakai env langsung dari systemd.
    dotenvy::dotenv().ok();

    init_tracing();
    // hyper-rustls memakai provider ring; instalasi eksplisit mencegah
    // panic runtime rustls saat worker delivery pertama kali membuat client.
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("provider TLS ring gagal dipasang"))?;

    warn_if_dotenv_permissions_longgar();

    // Set umask 0077 SEBELUM db/socket/known_hosts apa pun dibuat. Ini
    // membuat SEMUA file baru (db, -wal, -shm, -journal, socket forward
    // Fase 1, known_hosts aplikasi) lahir langsung 0600/0700 tanpa jendela
    // world-readable antara file dibuat dan chmod manual dijalankan (temuan
    // security Fase 0 HARUS-1). Ini tidak menggantikan chmod eksplisit di
    // db.rs — itu tetap perlu untuk file yang sudah ada dari sebelum
    // perbaikan ini.
    set_umask_private();

    let config = Config::from_env().context("muat konfigurasi")?;
    config
        .verify_encryption_key_permissions()
        .context("verifikasi izin file kunci enkripsi")?;
    config
        .verify_runtime_dir_available()
        .context("verifikasi direktori runtime tmpfs")?;
    config
        .verify_log_dir_available()
        .context("verifikasi direktori log deploy")?;

    // Socket forward Docker yang tersisa dari proses sebelumnya (crash,
    // bukan shutdown bersih) tidak pernah dianggap tepercaya — dibuang saat
    // startup (`docs/plan.md`, docker/forward.rs).
    mengdep::docker::cleanup_orphans(&config.runtime_dir);

    // `verify_encryption_key_permissions` di atas sudah memastikan path ini
    // `Some` dan bermode 0600 — gagal fatal kalau tidak, jadi baris ini tidak
    // pernah tercapai dengan path `None`.
    let key_path = config
        .encryption_key_path
        .as_deref()
        .context("path kunci enkripsi tidak tersedia setelah verifikasi izin")?;
    let crypto = CryptoKey::load_from_file(key_path).context("muat kunci enkripsi age")?;

    let pools = db::connect_and_migrate(&config.db_path)
        .await
        .context("siapkan database")?;

    seed_initial_password(
        &pools.write,
        &pools.read,
        config.initial_password.as_deref(),
    )
    .await
    .context("seed password awal")?;

    preflight_check_ssh_binaries();

    let state = AppState {
        db_write: pools.write,
        db_read: pools.read,
        config: Arc::new(config),
        crypto: Arc::new(crypto),
        events: Arc::new(EventRegistry::new()),
        deployment_events: Arc::new(EventRegistry::new()),
        logs: Arc::new(mengdep::logs::LogRegistry::new()),
        fleet_events: Arc::new(EventRegistry::new()),
    };

    rekonsiliasi_deployment_boot(&state.db_write, &state.db_read)
        .await
        .context("rekonsiliasi deployment saat boot")?;

    let listen_addr = state.config.listen_addr.clone();
    let worker_handle = mengdep::worker::spawn(state.clone());
    let deploy_worker_handle = mengdep::worker::deploy_worker::spawn(state.clone());
    let reconciliation_worker_handle = mengdep::worker::reconciliation::spawn(state.clone());
    let notification_worker_handle = mengdep::worker::notification_delivery::spawn(state.clone());
    let metrics_worker_handle = mengdep::worker::metrics::spawn(state.clone());
    let router = routes::build_router(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .with_context(|| format!("bind alamat {listen_addr}"))?;

    tracing::info!(addr = %listen_addr, "server mendengarkan");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("jalankan server")?;

    worker_handle.shutdown().await;
    deploy_worker_handle.shutdown().await;
    reconciliation_worker_handle.shutdown().await;
    notification_worker_handle.shutdown().await;
    metrics_worker_handle.shutdown().await;

    tracing::info!("server berhenti dengan bersih");

    Ok(())
}

/// Set umask proses ke `0077` supaya semua file/direktori baru lahir privat
/// (`0600`/`0700`) tanpa jendela world-readable (temuan security Fase 0
/// HARUS-1). Dipanggil sekali di awal `main`, sebelum thread lain atau
/// tugas async yang membuat file di-spawn.
fn set_umask_private() {
    unsafe extern "C" {
        fn umask(mask: u32) -> u32;
    }

    // SAFETY: `umask` adalah syscall POSIX dasar tanpa efek samping selain
    // mengubah umask proses saat ini. Dipanggil di titik paling awal
    // `main()`, sebelum runtime tokio menjalankan task lain apa pun —
    // proses masih single-threaded di sini, jadi tidak ada race dengan
    // thread lain yang sedang membuat file.
    unsafe {
        umask(0o077);
    }
}

/// `true` kalau permission file `.env` lebih longgar dari `0600` (group atau
/// other punya bit apa pun). Fungsi murni supaya bisa diuji tanpa menyentuh
/// filesystem atau tracing (temuan security Fase 0 HARUS-3).
fn dotenv_permissions_longgar(permissions: &std::fs::Permissions) -> bool {
    permissions.mode() & 0o077 != 0
}

/// Kalau `.env` ada dan permission-nya lebih longgar dari `0600`, catat
/// peringatan yang menyebut path dan instruksi perbaikan — TIDAK PERNAH isi
/// file (`.env` bisa memuat `MENGDEP_INITIAL_PASSWORD`, nanti `MENGDEP_KEY_PATH`
/// di Fase 1). Ini hanya peringatan, bukan kegagalan startup — `.env` cuma
/// dipakai jalur dev (`Q4`, `docs/plan.md`).
fn warn_if_dotenv_permissions_longgar() {
    let path = Path::new(".env");
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };

    if dotenv_permissions_longgar(&metadata.permissions()) {
        tracing::warn!(
            path = %path.display(),
            "file .env bisa dibaca/ditulis pihak lain (mode lebih longgar dari 0600) — \
             jalankan `chmod 600 .env` untuk memperbaikinya"
        );
    }
}

/// Cek `ssh`, `ssh-keyscan`, `ssh-keygen` ada di `PATH` — crate `openssh`
/// memanggil binary sistem, bukan implementasi SSH murni Rust
/// (`docs/plan.md`, catatan dependensi). TIDAK fatal kalau hilang: aplikasi
/// tetap berguna untuk login dan halaman fleet kosong (invariant 1 —
/// jangan bertindak keras karena sesuatu tidak terjangkau), tapi pesan
/// verifikasi server nanti akan membingungkan tanpa peringatan ini di log.
fn preflight_check_ssh_binaries() {
    for binary in ["ssh", "ssh-keyscan", "ssh-keygen"] {
        let ditemukan = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(binary).is_file()))
            .unwrap_or(false);

        if !ditemukan {
            tracing::warn!(
                binary,
                "binary tidak ditemukan di PATH — verifikasi dan polling server akan selalu \
                 gagal sampai ini terpasang"
            );
        }
    }
}

/// Inisialisasi `tracing` dengan keluaran JSON, level diatur lewat
/// `RUST_LOG` (default `info`).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
}

/// Kalau `settings.password_hash` belum ada dan `MENGDEP_INITIAL_PASSWORD`
/// di-set, hash password itu dan simpan. Tidak pernah mencatat isi password
/// ke tracing (invariant 7).
async fn seed_initial_password(
    write: &sqlx::SqlitePool,
    read: &sqlx::SqlitePool,
    initial_password: Option<&str>,
) -> Result<()> {
    let existing = sqlx::query!("SELECT value FROM settings WHERE key = 'password_hash'")
        .fetch_optional(read)
        .await
        .context("cek password_hash tersimpan")?;

    if existing.is_some() {
        return Ok(());
    }

    let Some(password) = initial_password else {
        tracing::warn!(
            "settings.password_hash belum ada dan MENGDEP_INITIAL_PASSWORD tidak diset — \
             login tidak akan bisa dilakukan sampai salah satunya tersedia"
        );
        return Ok(());
    };

    let hash = auth::password::hash_password(password).context("hash password awal")?;

    sqlx::query!(
        "INSERT INTO settings (key, value) VALUES ('password_hash', ?)",
        hash
    )
    .execute(write)
    .await
    .context("simpan password_hash awal")?;

    tracing::info!(
        "password awal berhasil di-seed dari MENGDEP_INITIAL_PASSWORD — hapus env ini sekarang"
    );

    Ok(())
}

/// Deployment berstatus aktif (queued/pulling/starting/checking) yang masih
/// tercatat saat proses ini baru mulai HANYA bisa berasal dari proses
/// sebelumnya yang mati di tengah jalan — deployment cuma hidup di dalam
/// task `tokio` proses yang menjalankannya, tidak ada tracking lintas proses.
/// Jadi `staleness_secs = 0`: SEMUA yang aktif saat boot pasti basi, bukan
/// tebakan (`docs/prd.md`: "unknown artinya kita tidak tahu").
async fn rekonsiliasi_deployment_boot(
    write: &sqlx::SqlitePool,
    read: &sqlx::SqlitePool,
) -> Result<()> {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let basi = deployments::repo::list_stale_active(read, now, 0)
        .await
        .context("cari deployment aktif basi saat boot")?;

    for dep in basi {
        tracing::warn!(
            deployment_id = %dep.id,
            app_id = %dep.app_id,
            "deployment aktif ditemukan saat boot, ditandai unknown"
        );
        deployments::repo::mark_unknown(write, &dep.id)
            .await
            .context("tandai deployment unknown saat boot")?;
        // id deployment dipakai sekaligus sebagai lock_token (lihat
        // `deployments::repo::generate_id`) — lepas supaya app ini bisa
        // langsung dideploy ulang tanpa menunggu `lock_expires_at`.
        apps::repo::release_lock(write, &dep.app_id, &dep.id)
            .await
            .context("lepas lock app saat rekonsiliasi boot")?;
    }

    Ok(())
}

/// Tunggu SIGTERM atau Ctrl+C untuk graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        // `expect` di sini boleh karena ini bukan jalur request — hanya
        // startup shutdown handler yang gagal berarti sistem sinyal rusak
        // total, tidak ada cara pulih. Namun sesuai konvensi "tidak ada
        // unwrap/expect di luar test", kita catat error lalu tunggu selamanya
        // supaya sinyal lain (mis. SIGTERM) tetap bisa memicu shutdown.
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = ?err, "gagal memasang handler ctrl_c");
            std::future::pending::<()>().await;
        }
    };

    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = ?err, "gagal memasang handler SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }

    tracing::info!("sinyal shutdown diterima, mematikan server dengan bersih");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_permissions_longgar_mendeteksi_group_dan_other_readable() {
        assert!(dotenv_permissions_longgar(
            &std::fs::Permissions::from_mode(0o644)
        ));
        assert!(dotenv_permissions_longgar(
            &std::fs::Permissions::from_mode(0o640)
        ));
        assert!(dotenv_permissions_longgar(
            &std::fs::Permissions::from_mode(0o604)
        ));
        assert!(dotenv_permissions_longgar(
            &std::fs::Permissions::from_mode(0o660)
        ));
    }

    #[test]
    fn dotenv_permissions_longgar_menerima_0600() {
        assert!(!dotenv_permissions_longgar(
            &std::fs::Permissions::from_mode(0o600)
        ));
        assert!(!dotenv_permissions_longgar(
            &std::fs::Permissions::from_mode(0o400)
        ));
    }

    // umask proses bersifat global per-proses (bukan per-thread), sehingga
    // tidak bisa diuji langsung di sini tanpa mengganggu test lain yang
    // jalan paralel dalam binary test yang sama (semua test tokio berbagi
    // satu proses). Efeknya diverifikasi secara tidak langsung lewat
    // `src/db.rs` — file db yang lahir setelah main() jalan sudah bermode
    // 0600 walau chmod eksplisit sengaja dilewati (lihat komentar db.rs).
    // Verifikasi end-to-end penuh (umask + file baru + tanpa chmod manual)
    // ada di integration test `tests/phase0.rs` yang menjalankan binary
    // sungguhan sebagai proses terpisah.
    #[test]
    fn set_umask_private_tidak_panic() {
        // Test ini hanya memastikan pemanggilan syscall tidak panic/UB pada
        // platform target. Tidak mengembalikan umask ke nilai semula karena
        // test lain di proses ini tidak bergantung pada umask spesifik.
        set_umask_private();
    }
}
