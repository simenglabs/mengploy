//! Integration test Fase 7: operasi armada dan pintu darurat.
//! Tidak menyentuh server nyata; fault injection memakai database, lock, dan
//! parser deterministik. Jalur SSH/Docker nyata tetap dikategorikan aman oleh
//! worker sebelum tindakan destruktif.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use mengdep::apps::{NewApp, repo as apps_repo};
use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::fleet::{self, FleetOperationKind, FleetResultStatus};
use mengdep::fleet_repo;
use mengdep::servers::{NewServer, repo as servers_repo};
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);
const SSH_KEY: &str =
    "-----BEGIN OPENSSH PRIVATE KEY-----\ndummy\n-----END OPENSSH PRIVATE KEY-----";

fn unique_dir(name: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("mengdep-phase7-{}-{id}-{name}", std::process::id()))
}

async fn setup(name: &str) -> (AppState, PathBuf) {
    let dir = unique_dir(name);
    std::fs::create_dir_all(&dir).expect("direktori test harus dibuat");
    let db_path = dir.join("test.db");
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi fase 7 harus sukses");
    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;
    let identity = age::x25519::Identity::generate();
    let key_path = dir.join("key.txt");
    std::fs::write(&key_path, identity.to_string().expose_secret()).expect("kunci test ditulis");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("kunci test privat");
    let crypto = CryptoKey::load_from_file(&key_path).expect("kunci enkripsi terbaca");
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

async fn server(state: &AppState, name: &str, status: &str) -> String {
    let key = state
        .crypto
        .encrypt(SSH_KEY)
        .expect("kunci SSH terenkripsi");
    let id = servers_repo::insert_pending(
        &state.db_write,
        NewServer {
            name,
            host: "127.0.0.1",
            port: 1,
            ssh_user: "root",
            ssh_key_encrypted: &key,
        },
    )
    .await
    .expect("server tersimpan");
    sqlx::query!(
        "UPDATE servers SET status = ?, host_key_fingerprint = 'SHA256:test' WHERE id = ?",
        status,
        id,
    )
    .execute(&state.db_write)
    .await
    .expect("status server diubah untuk fault injection");
    id
}

#[test]
fn validasi_command_menolak_kosong_nul_dan_ukuran_berlebih() {
    assert!(fleet::validate_command("").is_err());
    assert!(fleet::validate_command("echo\0x").is_err());
    assert!(fleet::validate_command(&"x".repeat(fleet::COMMAND_MAX_BYTES + 1)).is_err());
    assert_eq!(
        fleet::validate_command(" uptime ").expect("valid"),
        "uptime"
    );
}

#[test]
fn keluaran_besar_dipotong_dan_path_traversal_ditolak() {
    let (output, truncated) = fleet::bounded_output(&vec![b'x'; fleet::OUTPUT_MAX_BYTES + 1]);
    assert!(truncated);
    assert_eq!(output.len(), fleet::OUTPUT_MAX_BYTES);
    let base = std::path::Path::new("/var/lib/mengdep/operations");
    assert!(fleet::output_path_is_safe(
        "/var/lib/mengdep/operations/op/a.out",
        base
    ));
    assert!(!fleet::output_path_is_safe("/tmp/a.out", base));
    assert!(!fleet::output_path_is_safe(
        "/var/lib/mengdep/operations/../a.out",
        base
    ));
}

#[test]
fn parser_disk_menolak_keluaran_rusak_dan_tidak_menebak() {
    assert_eq!(
        fleet::parse_disk_output("100 1000\n").expect("valid"),
        (100, 1000)
    );
    assert!(fleet::parse_disk_output("disk gagal").is_err());
    assert!(fleet::parse_disk_output("100 10").is_err());
}

#[tokio::test]
async fn payload_command_terenkripsi_dan_tidak_muncul_di_sqlite() {
    let (state, dir) = setup("payload").await;
    let operation_id = "operation-payload";
    let plaintext = r#"{"command":"echo token-rahasia"}"#;
    let encrypted = state
        .crypto
        .encrypt(plaintext)
        .expect("payload terenkripsi");
    fleet_repo::insert_operation(
        &state.db_write,
        operation_id,
        FleetOperationKind::Command,
        r#"["server-1"]"#,
        &encrypted,
    )
    .await
    .expect("operasi tersimpan");
    let row = sqlx::query!(
        "SELECT payload_json FROM fleet_operations WHERE id = ?",
        operation_id
    )
    .fetch_one(&state.db_read)
    .await
    .expect("payload terbaca");
    assert!(!row.payload_json.contains("token-rahasia"));
    assert_eq!(
        state
            .crypto
            .decrypt(&row.payload_json)
            .expect("payload bisa didekripsi"),
        plaintext
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn lock_server_atomically_mencegah_prune_saat_deploy_aktif() {
    let (state, dir) = setup("lock").await;
    let server_id = server(&state, "server-lock", "online").await;
    let app_id = apps_repo::insert(
        &state.db_write,
        NewApp {
            server_id: &server_id,
            name: "app-lock",
            health_path: "/health",
            health_grace_secs: 1,
            port: 8080,
            restart_policy: "unless-stopped",
            repo_url: None,
        },
    )
    .await
    .expect("app tersimpan");
    let expiry = time::OffsetDateTime::now_utc().unix_timestamp() + 9_999_999;
    assert!(
        apps_repo::acquire_lock(&state.db_write, &app_id, "deploy-lock", expiry)
            .await
            .expect("lock deploy diambil")
    );
    assert!(
        !apps_repo::acquire_server_locks(&state.db_write, &server_id, "prune-lock", expiry)
            .await
            .expect("lock prune harus menjawab aman")
    );
    let row = sqlx::query!("SELECT lock_token FROM apps WHERE id = ?", app_id)
        .fetch_one(&state.db_read)
        .await
        .expect("lock terbaca");
    assert_eq!(row.lock_token.as_deref(), Some("deploy-lock"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn app_baru_ditolak_saat_lock_server_prune_aktif() {
    let (state, dir) = setup("lock-app-baru").await;
    let server_id = server(&state, "server-lock-app-baru", "online").await;
    let expiry = time::OffsetDateTime::now_utc().unix_timestamp() + 300;
    assert!(
        apps_repo::acquire_server_locks(&state.db_write, &server_id, "prune-lock", expiry)
            .await
            .expect("lock prune harus diambil")
    );

    let result = apps_repo::insert(
        &state.db_write,
        NewApp {
            server_id: &server_id,
            name: "app-diblokir-lock-prune",
            health_path: "/health",
            health_grace_secs: 1,
            port: 8081,
            restart_policy: "unless-stopped",
            repo_url: None,
        },
    )
    .await;
    assert!(
        result
            .as_ref()
            .err()
            .and_then(|err| err.downcast_ref::<apps_repo::ServerLocked>())
            .is_some(),
        "app baru harus ditolak dengan error server terkunci"
    );

    apps_repo::release_server_locks(&state.db_write, &server_id, "prune-lock")
        .await
        .expect("lock prune dilepas");
    let app_id = apps_repo::insert(
        &state.db_write,
        NewApp {
            server_id: &server_id,
            name: "app-setelah-lock-prune",
            health_path: "/health",
            health_grace_secs: 1,
            port: 8082,
            restart_policy: "unless-stopped",
            repo_url: None,
        },
    )
    .await
    .expect("app boleh dibuat setelah lock dilepas");
    assert!(!app_id.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn hasil_per_server_menyimpan_partial_success_dan_menolak_path_tidak_aman() {
    let (state, dir) = setup("hasil").await;
    let operation_id = "operation-hasil";
    let server_a = server(&state, "server-a", "online").await;
    let server_b = server(&state, "server-b", "online").await;
    let targets = serde_json::to_string(&vec![&server_a, &server_b]).expect("target json valid");
    fleet_repo::insert_operation(
        &state.db_write,
        operation_id,
        FleetOperationKind::Command,
        &targets,
        &state.crypto.encrypt("{}").expect("payload terenkripsi"),
    )
    .await
    .expect("operasi tersimpan");
    fleet_repo::insert_result(
        &state.db_write,
        &state.config.log_dir.join("operations"),
        operation_id,
        &server_a,
        Some(0),
        None,
        FleetResultStatus::Succeeded,
    )
    .await
    .expect("server sukses tersimpan");
    fleet_repo::insert_result(
        &state.db_write,
        &state.config.log_dir.join("operations"),
        operation_id,
        &server_b,
        None,
        None,
        FleetResultStatus::Failed,
    )
    .await
    .expect("server gagal tersimpan");
    assert!(
        fleet_repo::insert_result(
            &state.db_write,
            &state.config.log_dir.join("operations"),
            operation_id,
            "server-c",
            None,
            Some("/tmp/di-luar-direktori-operasi.out"),
            FleetResultStatus::Failed,
        )
        .await
        .is_err()
    );
    assert_eq!(
        fleet_repo::list_results(&state.db_read, operation_id)
            .await
            .expect("hasil terbaca")
            .len(),
        2
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn server_unreachable_tidak_mengubah_status_destructive() {
    let (state, dir) = setup("unreachable").await;
    let server_id = server(&state, "server-mati", "unreachable").await;
    assert!(
        mengdep::worker::fleet::exec_container_once(&state, &server_id, "container", "id")
            .await
            .is_err()
    );
    let row = sqlx::query!("SELECT status FROM servers WHERE id = ?", server_id)
        .fetch_one(&state.db_read)
        .await
        .expect("status terbaca");
    assert_eq!(row.status, "unreachable");
    let _ = std::fs::remove_dir_all(dir);
}
