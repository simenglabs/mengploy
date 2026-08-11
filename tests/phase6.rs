//! Integration test Fase 6: metrik, rollup, retensi, alert, dan fault injection.
//! Jalur SSH/Docker sungguhan tetap dipisahkan dari test database deterministik;
//! parser dan repository diuji sehingga kegagalan parsial tidak menjadi data
//! palsu atau menghapus histori server lain.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::metrics::{
    AlertKind, AlertWrite, ContainerMetricWrite, ContainerStatsInput, HostMetricWrite, HostSample,
    HostSampleInput, container_sample, disk_alert, parse_host_sample, resource_spike_alert,
    restart_alert,
};
use mengdep::metrics_repo;
use mengdep::servers::NewServer;
use mengdep::servers::repo as servers_repo;
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\ndummy\n-----END OPENSSH PRIVATE KEY-----";

fn unique_dir(name: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("mengdep-phase6-{}-{id}-{name}", std::process::id()))
}

async fn setup(name: &str) -> (AppState, PathBuf) {
    let dir = unique_dir(name);
    let db_path = dir.join("test.db");
    std::fs::create_dir_all(&dir).expect("direktori test harus dibuat");
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi fase 6 harus sukses");

    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;
    let identity = age::x25519::Identity::generate();
    let key_path = dir.join("key.txt");
    std::fs::write(&key_path, identity.to_string().expose_secret())
        .expect("kunci test harus ditulis");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("kunci test harus privat");
    let crypto = CryptoKey::load_from_file(&key_path).expect("kunci enkripsi harus terbaca");

    let state = AppState {
        db_write: pools.write,
        db_read: pools.read,
        config: std::sync::Arc::new(Config {
            listen_addr: "127.0.0.1:0".to_string(),
            db_path,
            initial_password: None,
            encryption_key_path: Some(key_path),
            runtime_dir: dir.join("runtime"),
            log_dir: dir.join("logs"),
            log_retention_days: 30,
        }),
        crypto: std::sync::Arc::new(crypto),
        events: std::sync::Arc::new(mengdep::events::EventRegistry::new()),
        deployment_events: std::sync::Arc::new(mengdep::events::EventRegistry::new()),
        logs: std::sync::Arc::new(mengdep::logs::LogRegistry::new()),
        fleet_events: std::sync::Arc::new(mengdep::events::EventRegistry::new()),
    };
    (state, dir)
}

async fn seed_server(state: &AppState, name: &str) -> String {
    let encrypted = state
        .crypto
        .encrypt(KEY)
        .expect("kunci SSH dummy terenkripsi");
    let id = servers_repo::insert_pending(
        &state.db_write,
        NewServer {
            name,
            host: "127.0.0.1",
            port: 1,
            ssh_user: "root",
            ssh_key_encrypted: &encrypted,
        },
    )
    .await
    .expect("server test harus tersimpan");
    servers_repo::set_host_key_fingerprint(&state.db_write, &id, "SHA256:test")
        .await
        .expect("fingerprint test harus tersimpan");
    sqlx::query!("UPDATE servers SET status = 'online' WHERE id = ?", id)
        .execute(&state.db_write)
        .await
        .expect("server test harus online");
    id
}

fn host_sample(disk_used: i64) -> HostSample {
    HostSample {
        cpu_percent: 25.0,
        mem_used: 400,
        mem_total: 1_000,
        load1: 0.5,
        disk_used,
        disk_total: 1_000,
    }
}

fn container_write<'a>(
    server_id: &'a str,
    id: &'a str,
    sample: &'a mengdep::metrics::ContainerSample,
) -> ContainerMetricWrite<'a> {
    ContainerMetricWrite {
        server_id,
        container_id: id,
        app_id: None,
        sample,
    }
}

#[test]
fn fault_parser_host_malformed_menghasilkan_error_bukan_nilai_nol() {
    let result = parse_host_sample(&HostSampleInput {
        proc_stat: "cpu rusak",
        proc_meminfo: "MemTotal: 100 kB\nMemAvailable: 20 kB",
        proc_loadavg: "0.1",
        df_output: "10 100",
        cpu_cores: 1,
        previous_cpu: None,
    });
    assert!(result.is_err());
}

#[test]
fn fault_container_stats_tanpa_delta_tidak_mengaku_cpu_valid() {
    let sample = container_sample(&ContainerStatsInput {
        cpu_delta: 100,
        system_delta: 0,
        online_cpus: 4,
        memory_usage: 100,
        inactive_file: 20,
        memory_max: 100,
        memory_limit: 200,
        net_rx: 0,
        net_tx: 0,
        restart_count: 0,
    });
    assert_eq!(sample.cpu_percent, 0.0);
    assert_eq!(sample.mem_bytes, 80);
}

#[test]
fn fault_alert_memiliki_tiga_jenis_dan_window_deploy() {
    assert_eq!(disk_alert(&host_sample(800)), Some(AlertKind::DiskHigh));
    assert_eq!(restart_alert(Some(1), 4), Some(AlertKind::RestartLoop));
    assert_eq!(
        resource_spike_alert(Some(100), 100 + 3_601, Some((10.0, 10.0)), (20.0, 20.0)),
        None
    );
}

#[tokio::test]
async fn fault_satu_transaksi_menyimpan_host_container_dan_alert_bersama() {
    let (state, dir) = setup("transaksi").await;
    let server_id = seed_server(&state, "server-transaksi").await;
    let sample = container_sample(&ContainerStatsInput {
        cpu_delta: 1,
        system_delta: 10,
        online_cpus: 1,
        memory_usage: 100,
        inactive_file: 10,
        memory_max: 100,
        memory_limit: 200,
        net_rx: 1,
        net_tx: 2,
        restart_count: 4,
    });
    let host = host_sample(900);
    let alert = AlertWrite {
        server_id: &server_id,
        app_id: None,
        container_id: Some("container-1"),
        deployment_id: None,
        kind: AlertKind::DiskHigh,
        severity: "critical",
        target: "root",
        message: "Disk host terpakai 80% atau lebih.",
    };
    metrics_repo::insert_cycle(
        &state.db_write,
        1_000,
        &[&server_id],
        &[HostMetricWrite {
            server_id: &server_id,
            sample: &host,
        }],
        &[container_write(&server_id, "container-1", &sample)],
        &[alert],
    )
    .await
    .expect("satu siklus harus commit");

    let counts = sqlx::query!(
        "SELECT
            (SELECT COUNT(*) FROM metrics_host WHERE server_id = ?) as host_count,
            (SELECT COUNT(*) FROM metrics_container WHERE server_id = ?) as container_count,
            (SELECT COUNT(*) FROM metric_alerts WHERE server_id = ?) as alert_count",
        server_id,
        server_id,
        server_id,
    )
    .fetch_one(&state.db_read)
    .await
    .expect("hasil transaksi harus terbaca");
    assert_eq!(counts.host_count, 1);
    assert_eq!(counts.container_count, 1);
    assert_eq!(counts.alert_count, 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn fault_alert_pulih_dan_server_lain_tidak_terhapus() {
    let (state, dir) = setup("alert-pulih").await;
    let server_a = seed_server(&state, "server-a").await;
    let server_b = seed_server(&state, "server-b").await;
    let alert = AlertWrite {
        server_id: &server_a,
        app_id: None,
        container_id: None,
        deployment_id: None,
        kind: AlertKind::DiskHigh,
        severity: "critical",
        target: "root",
        message: "Disk host terpakai 80% atau lebih.",
    };
    let sample = host_sample(900);
    metrics_repo::insert_cycle(
        &state.db_write,
        2_000,
        &[&server_a, &server_b],
        &[HostMetricWrite {
            server_id: &server_a,
            sample: &sample,
        }],
        &[],
        &[alert],
    )
    .await
    .expect("alert awal harus tersimpan");
    metrics_repo::insert_cycle(
        &state.db_write,
        2_001,
        &[&server_a],
        &[HostMetricWrite {
            server_id: &server_a,
            sample: &host_sample(100),
        }],
        &[],
        &[],
    )
    .await
    .expect("siklus pemulihan harus tersimpan");

    let row = sqlx::query!(
        "SELECT status, resolved_at FROM metric_alerts WHERE server_id = ? AND kind = 'disk_high'",
        server_a,
    )
    .fetch_one(&state.db_read)
    .await
    .expect("status alert harus terbaca");
    assert_eq!(row.status, "resolved");
    assert!(row.resolved_at.is_some());
    let host_b = sqlx::query!(
        "SELECT COUNT(*) as count FROM metrics_host WHERE server_id = ?",
        server_b,
    )
    .fetch_one(&state.db_read)
    .await
    .expect("histori server lain harus terbaca");
    assert_eq!(host_b.count, 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn fault_transaksi_gagal_mengembalikan_tulisan_sebelumnya() {
    let (state, dir) = setup("rollback").await;
    let server_id = seed_server(&state, "server-rollback").await;
    let host = host_sample(100);
    let invalid_alert = AlertWrite {
        server_id: &server_id,
        app_id: None,
        container_id: None,
        deployment_id: None,
        kind: AlertKind::DiskHigh,
        severity: "severity-tidak-valid",
        target: "root",
        message: "harus ditolak constraint",
    };
    let result = metrics_repo::insert_cycle(
        &state.db_write,
        3_000,
        &[&server_id],
        &[HostMetricWrite {
            server_id: &server_id,
            sample: &host,
        }],
        &[],
        &[invalid_alert],
    )
    .await;
    assert!(result.is_err());
    let count = sqlx::query!(
        "SELECT COUNT(*) as count FROM metrics_host WHERE server_id = ?",
        server_id,
    )
    .fetch_one(&state.db_read)
    .await
    .expect("hasil rollback harus terbaca");
    assert_eq!(count.count, 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn fault_rollup_dan_retensi_mempertahankan_max_dan_menghapus_raw_lama() {
    let (state, dir) = setup("retensi").await;
    let server_id = seed_server(&state, "server-retensi").await;
    let first = mengdep::metrics::HostSample {
        cpu_percent: 10.0,
        ..host_sample(100)
    };
    let second = mengdep::metrics::HostSample {
        cpu_percent: 90.0,
        ..host_sample(900)
    };
    metrics_repo::insert_cycle(
        &state.db_write,
        600,
        &[&server_id],
        &[HostMetricWrite {
            server_id: &server_id,
            sample: &first,
        }],
        &[],
        &[],
    )
    .await
    .expect("sampel pertama harus tersimpan");
    metrics_repo::insert_cycle(
        &state.db_write,
        620,
        &[&server_id],
        &[HostMetricWrite {
            server_id: &server_id,
            sample: &second,
        }],
        &[],
        &[],
    )
    .await
    .expect("sampel kedua harus tersimpan");
    metrics_repo::rollup_and_retain(&state.db_write, 600 + 7 * 24 * 60 * 60 - 1)
        .await
        .expect("rollup dan retensi harus sukses");

    let row = sqlx::query!(
        "SELECT cpu_avg, cpu_max FROM metrics_host
         WHERE server_id = ? AND res = 'min' LIMIT 1",
        server_id,
    )
    .fetch_one(&state.db_read)
    .await
    .expect("rollup menit harus ada");
    assert_eq!(row.cpu_avg, Some(50.0));
    assert_eq!(row.cpu_max, Some(90.0));
    let raw_count = sqlx::query!(
        "SELECT COUNT(*) as count FROM metrics_host WHERE server_id = ? AND res = 'raw'",
        server_id,
    )
    .fetch_one(&state.db_read)
    .await
    .expect("retensi raw harus terbaca");
    assert_eq!(raw_count.count, 0);
    let _ = std::fs::remove_dir_all(dir);
}
