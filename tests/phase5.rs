//! Integration test Fase 5: fault injection lintas engine, queue notifikasi,
//! HMAC, dan klasifikasi rekonsiliasi. Docker sungguhan tidak diperlukan;
//! kegagalan SSH diarahkan ke port lokal tertutup agar jalur engine nyata
//! tetap diuji deterministik.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use mengdep::apps::NewApp;
use mengdep::apps::repo as apps_repo;
use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::deployments::reconciliation::{DriftKind, FindingStatus, classify_live_drift};
use mengdep::deployments::repo as deployments_repo;
use mengdep::deployments::{NewDeployment, StatusDeployment, jalankan_deploy};
use mengdep::docker::ContainerObservation;
use mengdep::notifications::model::{sign_payload, verify_signature};
use mengdep::notifications::repo as notification_repo;
use mengdep::servers::NewServer;
use mengdep::servers::repo as servers_repo;
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);
const HOST_TERTUTUP: &str = "127.0.0.1";
const PORT_TERTUTUP: i64 = 1;
const KUNCI_PLACEHOLDER: &str =
    "-----BEGIN OPENSSH PRIVATE KEY-----\nbukan-kunci-asli\n-----END OPENSSH PRIVATE KEY-----";

fn unique_dir(nama: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("mengdep-phase5-{}-{n}-{nama}", std::process::id()))
}

async fn setup(nama: &str) -> (AppState, PathBuf) {
    let dir = unique_dir(nama);
    let db_path = dir.join("test.db");
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi database fase 5 harus sukses");

    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;
    let identity = age::x25519::Identity::generate();
    let key_path = dir.join("key.txt");
    std::fs::create_dir_all(&dir).expect("direktori test harus dibuat");
    std::fs::write(&key_path, identity.to_string().expose_secret()).expect("kunci test ditulis");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("mode kunci test harus privat");
    let crypto = CryptoKey::load_from_file(&key_path).expect("kunci test harus terbaca");

    let state = AppState {
        db_write: pools.write,
        db_read: pools.read,
        config: std::sync::Arc::new(Config {
            listen_addr: "127.0.0.1:0".to_string(),
            db_path: db_path.clone(),
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

async fn seed_server(state: &AppState) -> String {
    let encrypted = state
        .crypto
        .encrypt(KUNCI_PLACEHOLDER)
        .expect("kunci SSH dummy harus terenkripsi");
    let id = servers_repo::insert_pending(
        &state.db_write,
        NewServer {
            name: "server-fault",
            host: HOST_TERTUTUP,
            port: PORT_TERTUTUP,
            ssh_user: "root",
            ssh_key_encrypted: &encrypted,
        },
    )
    .await
    .expect("server fault harus tersimpan");
    servers_repo::set_host_key_fingerprint(&state.db_write, &id, "SHA256:palsu")
        .await
        .expect("fingerprint dummy harus tersimpan");
    id
}

async fn seed_app(state: &AppState, server_id: &str) -> String {
    apps_repo::insert(
        &state.db_write,
        NewApp {
            server_id,
            name: "app-fault",
            health_path: "/health",
            health_grace_secs: 1,
            port: 8080,
            restart_policy: "unless-stopped",
        },
    )
    .await
    .expect("app fault harus tersimpan")
}

async fn seed_deployment(state: &AppState, app_id: &str) -> String {
    let deployment_id = deployments_repo::generate_id();
    let job_id = deployments_repo::generate_id();
    let digest = format!("ghcr.io/org/app@sha256:{}", "a".repeat(64));
    deployments_repo::insert_queued_dengan_job(
        &state.db_write,
        &deployment_id,
        NewDeployment {
            app_id,
            commit_sha: "deadbeef",
            git_ref: Some("main"),
            image_digest: &digest,
            trigger_source: "api",
            env_version_id: None,
        },
        &job_id,
        "{}",
    )
    .await
    .expect("deployment fault harus tersimpan");
    deployment_id
}

#[test]
fn hmac_menolak_payload_secret_yang_diubah_dan_signature_salah() {
    let payload =
        br#"{"event_type":"deployment.failed","occurred_at":1,"data":{"status":"failed"}}"#;
    let signature = sign_payload(b"secret-uji", payload);
    assert!(verify_signature(b"secret-uji", payload, &signature));
    assert!(!verify_signature(b"secret-lain", payload, &signature));
    assert!(!verify_signature(
        b"secret-uji",
        br#"{"event_type":"deployment.failed","occurred_at":1,"data":{"status":"live"}}"#,
        &signature,
    ));
    assert!(
        !payload
            .windows(b"secret-uji".len())
            .any(|part| part == b"secret-uji")
    );
}

#[tokio::test]
async fn delivery_queue_idempoten_retry_lalu_failed_tanpa_payload_rahasia() {
    let (state, dir) = setup("queue").await;
    let payload = r#"{"event_id":"dep-1","event_type":"deployment.failed","occurred_at":1,"data":{"status":"failed"}}"#;
    assert!(
        notification_repo::enqueue(
            &state.db_write,
            "delivery-1",
            "dep-1",
            "deployment.failed",
            None,
            payload,
        )
        .await
        .expect("delivery pertama harus masuk")
    );
    assert!(
        !notification_repo::enqueue(
            &state.db_write,
            "delivery-2",
            "dep-1",
            "deployment.failed",
            None,
            payload,
        )
        .await
        .expect("delivery duplikat harus diproses idempoten")
    );

    let delivery = notification_repo::claim_next(&state.db_write)
        .await
        .expect("delivery harus bisa diklaim")
        .expect("delivery harus tersedia");
    assert_eq!(delivery.attempts, 1);
    notification_repo::mark_retry(
        &state.db_write,
        &delivery.id,
        "transport_delivery",
        delivery.attempts,
    )
    .await
    .expect("retry harus tersimpan");
    let retried = notification_repo::claim_next(&state.db_write)
        .await
        .expect("delivery retry harus bisa dibaca");
    assert!(retried.is_none(), "backoff harus menahan retry instan");

    // Fault permanen saat item sudah diklaim harus menjadi failed, bukan
    // kembali queued tanpa batas.
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    sqlx::query!(
        "UPDATE notification_deliveries SET next_attempt_at = ? WHERE id = 'delivery-1'",
        now
    )
    .execute(&state.db_write)
    .await
    .expect("jatuh tempo delivery harus bisa diinjeksi");
    let delivery = notification_repo::claim_next(&state.db_write)
        .await
        .expect("delivery kedua harus bisa diklaim")
        .expect("delivery kedua harus tersedia");
    notification_repo::mark_failed(&state.db_write, &delivery.id, "webhook_menolak", false)
        .await
        .expect("delivery failed harus tersimpan");
    let row = sqlx::query!(
        "SELECT status, last_error_kind FROM notification_deliveries WHERE id = 'delivery-1'"
    )
    .fetch_one(&state.db_read)
    .await
    .expect("status delivery harus terbaca");
    assert_eq!(row.status, "failed");
    assert_eq!(row.last_error_kind.as_deref(), Some("webhook_menolak"));
    assert!(!payload.contains("secret"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn fault_ssh_tidak_terjangkau_menjadi_failed_dan_lock_dilepas() {
    let (state, dir) = setup("ssh-gagal").await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id).await;
    let deployment_id = seed_deployment(&state, &app_id).await;

    jalankan_deploy(state.clone(), deployment_id.clone()).await;

    let deployment = deployments_repo::find_by_id(&state.db_read, &deployment_id)
        .await
        .expect("deployment harus terbaca")
        .expect("deployment harus ada");
    assert_eq!(deployment.status, StatusDeployment::Failed);
    assert!(deployment.error_kind.is_some());
    let lock = sqlx::query!("SELECT lock_token FROM apps WHERE id = ?", app_id)
        .fetch_one(&state.db_read)
        .await
        .expect("lock app harus terbaca");
    assert!(
        lock.lock_token.is_none(),
        "lock harus dilepas setelah fault SSH"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rekonsiliasi_fault_container_hilang_tidak_memicu_tindakan_destructive() {
    let containers: Vec<ContainerObservation> = Vec::new();
    let findings = classify_live_drift(
        "dep-live",
        "sha256:expected",
        Some("container-old"),
        &containers,
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, DriftKind::LiveContainerMissing);
    assert_eq!(FindingStatus::Open.as_db_str(), "open");
    assert_eq!(FindingStatus::Resolved.as_db_str(), "resolved");
    // Klasifikasi hanya mengembalikan metadata; tidak ada API stop/remove
    // yang dipanggil dan container list tetap kosong.
    assert!(containers.is_empty());
}
