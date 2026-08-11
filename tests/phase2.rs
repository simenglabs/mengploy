//! Integration test Fase 2 (Loop deploy): skenario injeksi kegagalan lewat
//! router sungguhan, pola sama `tests/phase1.rs`.
//!
//! Skenario yang butuh Docker sungguhan (container exited, health check
//! gagal/lulus, log tertangkap sebelum dihapus, port bentrok) TIDAK
//! dites integrasi penuh di sini — lingkungan test ini tidak punya daemon
//! Docker. Klasifikasi kegagalan itu (`DeployKegagalan::kind`/`pesan`) sudah
//! diuji unit di `src/deployments/engine.rs`; urutan literal
//! start-baru→health-check→stop-lama dan tangkap-log-sebelum-hapus diverifikasi
//! lewat code review (`docs/progress.md`), bukan integration test — sama
//! keterbatasan seperti Fase 1 (SSH sungguhan bisa dites lewat trik port
//! tertutup, Docker tidak).
//!
//! Yang DITES di sini: seluruh kontrak `POST /api/v1/deploy` (auth, validasi
//! digest, 404 tanpa bocor, lock 409) — murni lewat router, TANPA
//! menyalakan worker deploy — dan jalur engine yang bisa dipicu tanpa
//! Docker: kegagalan koneksi SSH (server dituju ke port tertutup, sama
//! trik `tests/phase1.rs`) menghasilkan `failed` + lock terlepas
//! (invariant §3 no.12), bukan macet diam-diam.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::util::ServiceExt;

use mengdep::apps::NewApp;
use mengdep::apps::repo as apps_repo;
use mengdep::auth::deploy_token;
use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::deployments::repo as deployments_repo;
use mengdep::deployments::{StatusDeployment, jalankan_deploy};
use mengdep::routes::build_router;
use mengdep::servers::NewServer;
use mengdep::servers::repo as servers_repo;
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);

const HOST_TERTUTUP: &str = "127.0.0.1";
const PORT_TERTUTUP: i64 = 1;
const KUNCI_PLACEHOLDER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nbukan-kunci-asli-hanya-format\n-----END OPENSSH PRIVATE KEY-----";
fn digest_contoh() -> String {
    format!("ghcr.io/org/api@sha256:{}", "a".repeat(64))
}

fn unique_dir(nama: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "mengdep-phase2-{}-{}-{}",
        std::process::id(),
        n,
        nama
    ))
}

async fn setup(nama: &str) -> (AppState, PathBuf) {
    let dir = unique_dir(nama);
    let db_path = dir.join("test.db");
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi db baru harus sukses");

    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;
    let identity = age::x25519::Identity::generate();
    let key_path = dir.join("key.txt");
    std::fs::create_dir_all(&dir).expect("bikin direktori temp test harus sukses");
    std::fs::write(&key_path, identity.to_string().expose_secret())
        .expect("tulis file kunci sementara harus sukses");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("set mode file kunci sementara harus sukses");
    let crypto =
        CryptoKey::load_from_file(&key_path).expect("muat kunci enkripsi sementara harus sukses");

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

/// Server dengan fingerprint SUDAH terisi (melewati verifikasi Fase 1 secara
/// artifisial) supaya `deployments::engine` mau mencoba konek — arahkan ke
/// port tertutup supaya kegagalannya cepat & deterministik (sama trik
/// `tests/phase1.rs`).
async fn seed_server_tak_terjangkau(state: &AppState) -> String {
    let ssh_key_encrypted = state
        .crypto
        .encrypt(KUNCI_PLACEHOLDER)
        .expect("enkripsi kunci placeholder");
    let id = servers_repo::insert_pending(
        &state.db_write,
        NewServer {
            name: "vps-test",
            host: HOST_TERTUTUP,
            port: PORT_TERTUTUP,
            ssh_user: "root",
            ssh_key_encrypted: &ssh_key_encrypted,
        },
    )
    .await
    .expect("simpan server test");
    servers_repo::set_host_key_fingerprint(&state.db_write, &id, "SHA256:palsu")
        .await
        .expect("set fingerprint palsu");
    id
}

async fn seed_app(state: &AppState, server_id: &str, name: &str) -> String {
    apps_repo::insert(
        &state.db_write,
        NewApp {
            server_id,
            name,
            health_path: "/health",
            health_grace_secs: 5,
            port: 8080,
            restart_policy: "unless-stopped",
        },
    )
    .await
    .expect("simpan app test")
}

/// Kembalikan (id token, token plaintext).
async fn seed_deploy_token(state: &AppState, app_id: &str) -> String {
    let plaintext = deploy_token::generate();
    let hash = deploy_token::hash(&plaintext).expect("hash token test");
    apps_repo::insert_deploy_token(&state.db_write, app_id, "test-token", &hash)
        .await
        .expect("simpan token test");
    plaintext
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("request harus diproses tanpa panic");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body response harus terbaca");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn deploy_request(app_name: &str, image: &str, token: Option<&str>) -> Request<Body> {
    let body =
        format!(r#"{{"app":"{app_name}","image":"{image}","commit":"deadbeef","ref":"main"}}"#);
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/v1/deploy")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body)).unwrap()
}

/// Skenario 1 — token salah/tidak ada ditolak 401, tanpa membuat deployment.
#[tokio::test]
async fn token_salah_ditolak_401_tanpa_membuat_deployment() {
    let (state, _dir) = setup("token-salah").await;
    let server_id = seed_server_tak_terjangkau(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    seed_deploy_token(&state, &app_id).await;
    let router = build_router(state.clone());

    let (status, _) = send(
        &router,
        deploy_request("api", &digest_contoh(), Some("token-ngawur")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let riwayat = deployments_repo::list_by_app(&state.db_read, &app_id)
        .await
        .expect("baca riwayat");
    assert!(
        riwayat.is_empty(),
        "token salah tidak boleh membuat deployment"
    );
}

/// Skenario 2 — app tidak dikenal selalu 404, WALAU token app lain valid
/// (`docs/plan.md`: jangan bocorkan app mana yang ada).
#[tokio::test]
async fn app_tidak_dikenal_selalu_404_walau_token_app_lain_valid() {
    let (state, _dir) = setup("app-tidak-dikenal").await;
    let server_id = seed_server_tak_terjangkau(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let token = seed_deploy_token(&state, &app_id).await;
    let router = build_router(state.clone());

    let (status, _) = send(
        &router,
        deploy_request("app-tidak-ada", &digest_contoh(), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Skenario 3 — image tanpa digest lengkap (tag polos) ditolak 400
/// (invariant §5 no.6).
#[tokio::test]
async fn image_tanpa_digest_ditolak_400() {
    let (state, _dir) = setup("image-tanpa-digest").await;
    let server_id = seed_server_tak_terjangkau(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let token = seed_deploy_token(&state, &app_id).await;
    let router = build_router(state.clone());

    let (status, _) = send(
        &router,
        deploy_request("api", "ghcr.io/org/api:latest", Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let riwayat = deployments_repo::list_by_app(&state.db_read, &app_id)
        .await
        .expect("baca riwayat");
    assert!(
        riwayat.is_empty(),
        "digest tidak valid tidak boleh membuat deployment"
    );
}

/// Skenario 4 — deploy sukses membuat deployment `queued` DAN job dalam
/// satu transaksi (kontrak `docs/plan.md`), 202 + `deployment_id` balik.
#[tokio::test]
async fn deploy_valid_membuat_deployment_queued_dan_202() {
    let (state, _dir) = setup("deploy-valid").await;
    let server_id = seed_server_tak_terjangkau(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let token = seed_deploy_token(&state, &app_id).await;
    let router = build_router(state.clone());

    let (status, body) = send(
        &router,
        deploy_request("api", &digest_contoh(), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    assert!(body.contains("deployment_id"));

    let riwayat = deployments_repo::list_by_app(&state.db_read, &app_id)
        .await
        .expect("baca riwayat");
    assert_eq!(riwayat.len(), 1);
    assert_eq!(riwayat[0].status, StatusDeployment::Queued);
}

/// Skenario 5 — deploy kedua untuk app yang SAMA saat lock masih aktif
/// (deploy pertama belum diproses worker, lock masih dipegang) ditolak 409,
/// tidak dua job aktif untuk app yang sama.
#[tokio::test]
async fn deploy_kedua_saat_lock_aktif_ditolak_409() {
    let (state, _dir) = setup("lock-aktif").await;
    let server_id = seed_server_tak_terjangkau(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let token = seed_deploy_token(&state, &app_id).await;
    let router = build_router(state.clone());

    let (status1, _) = send(
        &router,
        deploy_request("api", &digest_contoh(), Some(&token)),
    )
    .await;
    assert_eq!(status1, StatusCode::ACCEPTED);

    let (status2, _) = send(
        &router,
        deploy_request("api", &digest_contoh(), Some(&token)),
    )
    .await;
    assert_eq!(status2, StatusCode::CONFLICT);

    let riwayat = deployments_repo::list_by_app(&state.db_read, &app_id)
        .await
        .expect("baca riwayat");
    assert_eq!(
        riwayat.len(),
        1,
        "deploy kedua yang ditolak tidak boleh membuat baris baru"
    );
}

/// Skenario 6 — server tidak terjangkau (SSH gagal konek): deployment
/// berakhir `failed` (bukan macet di `pulling` selamanya), dan lock APP
/// TERLEPAS setelahnya (invariant §3 no.12) — dibuktikan lewat percobaan
/// deploy KEDUA yang langsung sukses 202, bukan 409.
#[tokio::test]
async fn deploy_ke_server_tak_terjangkau_gagal_dan_lock_terlepas() {
    let (state, _dir) = setup("server-tak-terjangkau").await;
    let server_id = seed_server_tak_terjangkau(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let token = seed_deploy_token(&state, &app_id).await;
    let router = build_router(state.clone());

    let (status, body) = send(
        &router,
        deploy_request("api", &digest_contoh(), Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let deployment_id = ekstrak_deployment_id(&body);

    // Worker deploy TIDAK di-spawn di test ini (pola sama phase1: panggil
    // mesinnya langsung, bukan lewat worker tick, supaya test deterministik
    // dan cepat) — jalankan engine langsung seperti `worker::deploy_worker`
    // akan lakukan.
    jalankan_deploy(state.clone(), deployment_id.clone()).await;

    let dep = deployments_repo::find_by_id(&state.db_read, &deployment_id)
        .await
        .expect("baca deployment")
        .expect("deployment harus ada");
    assert_eq!(dep.status, StatusDeployment::Failed);
    assert!(dep.error_kind.is_some(), "kegagalan harus punya error_kind");

    // Lock sudah terlepas — deploy kedua harus 202, bukan 409.
    let (status2, _) = send(
        &router,
        deploy_request("api", &digest_contoh(), Some(&token)),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::ACCEPTED,
        "lock harus terlepas setelah deploy gagal supaya app bisa dideploy ulang"
    );
}

fn ekstrak_deployment_id(body: &str) -> String {
    let marker = r#""deployment_id":""#;
    let start = body.find(marker).expect("body harus memuat deployment_id") + marker.len();
    let rest = &body[start..];
    let end = rest.find('"').expect("deployment_id harus diakhiri kutip");
    rest[..end].to_string()
}

/// Skenario 7 — timeout waktu tunggu tidak dipakai: cukup pastikan
/// `jalankan_deploy` tidak pernah panik walau baris deployment sudah
/// dihapus/tidak ada (`worker::deploy_worker` selalu memanggil ini secara
/// fire-and-forget) — kegagalan lookup tidak boleh menjatuhkan proses.
#[tokio::test]
async fn jalankan_deploy_dengan_id_tidak_dikenal_tidak_panik() {
    let (state, _dir) = setup("id-tidak-dikenal").await;
    jalankan_deploy(state.clone(), "deployment-tidak-ada".to_string()).await;
    // Sampai baris ini tanpa panik = lolos.
}
