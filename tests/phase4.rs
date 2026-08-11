//! Integration test Fase 4 (Pengelolaan environment): serangan terhadap
//! permukaan HTTP `/apps/{id}/env` lewat router sungguhan, pola sama
//! `tests/phase1-3.rs`.
//!
//! Skenario yang butuh Docker/SSH sungguhan (env benar-benar sampai ke
//! container, file audit tertulis di server target) TIDAK dites di sini —
//! lingkungan test tidak punya daemon Docker, keterbatasan yang sama sejak
//! Fase 2. Yang DITES: seluruh kontrak `POST /apps/{id}/env` (auth, CSRF,
//! validasi key/value, snapshot+deployment baru dengan digest sama, lock
//! aktif tidak menghalangi env tersimpan) murni lewat router + db, tanpa
//! menyalakan worker deploy.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::util::ServiceExt;

use mengdep::apps::NewApp;
use mengdep::apps::repo as apps_repo;
use mengdep::auth::middleware::SESSION_COOKIE_NAME;
use mengdep::auth::password::hash_password;
use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::deployments::repo as deployments_repo;
use mengdep::deployments::{LOCK_TTL_SECS, NewDeployment, StatusDeployment};
use mengdep::routes::build_router;
use mengdep::servers::NewServer;
use mengdep::servers::repo as servers_repo;
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);

const CSRF_DRAFT_COOKIE_NAME: &str = "mengdep_csrf_draft";
const PASSWORD_UJI: &str = "kata-sandi-uji-fase4";
const HOST_TERTUTUP: &str = "127.0.0.1";
const PORT_TERTUTUP: i64 = 1;
const KUNCI_PLACEHOLDER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nbukan-kunci-asli-hanya-format\n-----END OPENSSH PRIVATE KEY-----";

fn digest_contoh() -> String {
    format!("ghcr.io/org/api@sha256:{}", "a".repeat(64))
}

fn unique_dir(nama: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "mengdep-phase4-{}-{}-{}",
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

async fn seed_password(state: &AppState, password: &str) {
    let hash = hash_password(password).expect("hash password untuk seed");
    sqlx::query("INSERT INTO settings (key, value) VALUES ('password_hash', ?)")
        .bind(hash)
        .execute(&state.db_write)
        .await
        .expect("simpan password_hash ke settings");
}

async fn seed_server(state: &AppState) -> String {
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

/// Buat satu deployment berstatus `status`. Kembalikan idnya.
async fn seed_deployment(state: &AppState, app_id: &str, status: StatusDeployment) -> String {
    let digest = digest_contoh();
    let id = deployments_repo::generate_id();
    let job_id = deployments_repo::generate_id();
    deployments_repo::insert_queued_dengan_job(
        &state.db_write,
        &id,
        NewDeployment {
            app_id,
            commit_sha: "deadbeefdeadbeef",
            git_ref: Some("main"),
            image_digest: &digest,
            trigger_source: "api",
            env_version_id: None,
        },
        &job_id,
        "{}",
    )
    .await
    .expect("simpan deployment test");
    deployments_repo::set_status(&state.db_write, &id, status)
        .await
        .expect("set status deployment test");
    id
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("request harus diproses tanpa panic");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body response harus terbaca");
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn get_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn post_form(uri: &str, cookie: &str, fields: &[(&str, &str)]) -> Request<Body> {
    let mut body = String::new();
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            body.push('&');
        }
        body.push_str(&urlencode(k));
        body.push('=');
        body.push_str(&urlencode(v));
    }
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if !cookie.is_empty() {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::from(body)).unwrap()
}

fn urlencode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn ambil_cookie_dari_set_cookie(headers: &HeaderMap, nama: &str) -> Option<String> {
    for value in headers.get_all(header::SET_COOKIE) {
        let s = value.to_str().ok()?;
        if let Some(rest) = s.strip_prefix(nama) {
            let value_part = rest.strip_prefix('=')?.split(';').next()?.trim();
            return Some(value_part.to_string());
        }
    }
    None
}

fn parse_hidden_csrf(html: &str) -> Option<String> {
    let marker = r#"name="csrf_token" value=""#;
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Login dan kembalikan header `Cookie:` siap pakai.
async fn login(app: &axum::Router) -> String {
    let (status, headers, body) = send(app, get("/login")).await;
    assert_eq!(status, StatusCode::OK, "GET /login harus 200");
    let draft = ambil_cookie_dari_set_cookie(&headers, CSRF_DRAFT_COOKIE_NAME)
        .expect("GET /login harus men-set cookie draft csrf");
    let token = parse_hidden_csrf(&body).expect("form login harus menanam csrf_token");

    let (status, headers, _) = send(
        app,
        post_form(
            "/login",
            &format!("{CSRF_DRAFT_COOKIE_NAME}={draft}"),
            &[("password", PASSWORD_UJI), ("csrf_token", &token)],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "login benar harus 303");
    let sesi = ambil_cookie_dari_set_cookie(&headers, SESSION_COOKIE_NAME)
        .expect("login harus men-set cookie sesi");
    format!("{SESSION_COOKIE_NAME}={sesi}")
}

/// Siapkan state + router + sesi + satu server/app. Kembalikan
/// (state, dir, router, cookie, app_id).
async fn siap(nama: &str) -> (AppState, PathBuf, axum::Router, String, String) {
    let (state, dir) = setup(nama).await;
    seed_password(&state, PASSWORD_UJI).await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let router = build_router(state.clone());
    let cookie = login(&router).await;
    (state, dir, router, cookie, app_id)
}

async fn ambil_csrf(router: &axum::Router, cookie: &str, uri: &str) -> String {
    let (status, _headers, body) = send(router, get_with_cookie(uri, cookie)).await;
    assert_eq!(status, StatusCode::OK, "GET {uri} harus 200");
    parse_hidden_csrf(&body).expect("halaman env harus menanam csrf_token")
}

// ============================================================
// 1 — Autentikasi
// ============================================================

#[tokio::test]
async fn env_tab_dan_submit_tanpa_cookie_sesi_redirect_ke_login() {
    let (_state, _dir, router, _cookie, app_id) = siap("auth").await;

    let (status, headers, _) = send(&router, get(&format!("/apps/{app_id}/env"))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some("/login")
    );

    let (status, _, _) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            "",
            &[("csrf_token", "apa saja"), ("new_key_0", "X")],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

// ============================================================
// 2 — CSRF salah ditolak tanpa efek samping
// ============================================================

#[tokio::test]
async fn csrf_salah_pada_env_submit_ditolak_tanpa_menyimpan_apa_pun() {
    let (state, _dir, router, cookie, app_id) = siap("csrf-salah").await;

    let (status, _, _) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", "token-salah-yang-tidak-cocok"),
                ("new_key_0", "DATABASE_URL"),
                ("new_value_0", "postgres://x"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let tersimpan = apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
        .await
        .expect("baca env vars");
    assert!(
        tersimpan.is_empty(),
        "CSRF salah tidak boleh menyisakan env var apa pun"
    );
}

// ============================================================
// 3 — Key duplikat dalam satu submit ditolak
// ============================================================

#[tokio::test]
async fn key_baru_duplikat_dalam_satu_submit_ditolak_tanpa_menyimpan() {
    let (state, _dir, router, cookie, app_id) = siap("key-duplikat").await;
    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;

    let (status, _, body) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("new_key_0", "SAMA"),
                ("new_value_0", "satu"),
                ("new_key_1", "SAMA"),
                ("new_value_1", "dua"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("SAMA"), "pesan error harus menyebut key-nya");

    let tersimpan = apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
        .await
        .expect("baca env vars");
    assert!(
        tersimpan.is_empty(),
        "key duplikat tidak boleh menyisakan baris apa pun (bukan yang pertama saja)"
    );
}

// ============================================================
// 4 — Value dengan newline ditolak
// ============================================================

#[tokio::test]
async fn value_dengan_newline_ditolak() {
    let (state, _dir, router, cookie, app_id) = siap("newline").await;
    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;

    let (status, _, _) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("new_key_0", "MULTILINE"),
                ("new_value_0", "baris1\nbaris2"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let tersimpan = apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
        .await
        .expect("baca env vars");
    assert!(tersimpan.is_empty());
}

// ============================================================
// 5 — Simpan sukses: snapshot baru + deployment baru digest SAMA,
//     secret tidak pernah bocor ke response
// ============================================================

#[tokio::test]
async fn simpan_env_membuat_snapshot_dan_deployment_baru_dengan_digest_sama() {
    let (state, _dir, router, cookie, app_id) = siap("simpan-sukses").await;
    let dep_lama = seed_deployment(&state, &app_id, StatusDeployment::Live).await;
    let digest_lama = deployments_repo::find_by_id(&state.db_read, &dep_lama)
        .await
        .unwrap()
        .unwrap()
        .image_digest;

    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;
    let nilai_secret = "s3cr3t-p@ssw0rd!#%^&*()_+=-nilai panjang sedikit lebih dari biasa";
    let (status, _, body) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("new_key_0", "DB_PASSWORD"),
                ("new_value_0", nilai_secret),
                ("new_secret_0", "1"),
                ("new_key_1", "NODE_ENV"),
                ("new_value_1", "production"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains(nilai_secret),
        "nilai secret TIDAK BOLEH muncul di response setelah disimpan (invariant §3 no.7)"
    );
    assert!(
        body.contains("production"),
        "nilai non-secret boleh ditampilkan"
    );

    // Snapshot tersimpan benar dan bisa didekripsi balik.
    let semua = apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
        .await
        .expect("baca env vars");
    assert_eq!(semua.len(), 2);
    let (_, val_enc, is_secret) = semua.iter().find(|(k, _, _)| k == "DB_PASSWORD").unwrap();
    assert!(is_secret);
    assert_eq!(state.crypto.decrypt(val_enc).unwrap(), nilai_secret);

    // Deployment baru dibuat, digest identik deployment lama, trigger_source
    // 'env', env_version_id terisi.
    let deploys = deployments_repo::list_by_app(&state.db_read, &app_id)
        .await
        .unwrap();
    assert_eq!(deploys.len(), 2, "harus ada deployment baru selain lama");
    let baru = deploys
        .iter()
        .find(|d| d.id != dep_lama)
        .expect("deployment baru harus ada");
    assert_eq!(baru.image_digest, digest_lama);
    assert!(baru.env_version_id.is_some());
    assert_eq!(baru.status, StatusDeployment::Queued);
}

// ============================================================
// 6 — Nilai panjang dan karakter khusus tetap round-trip utuh.
// ============================================================

#[tokio::test]
async fn nilai_env_panjang_dan_karakter_khusus_roundtrip_utuh() {
    let (state, _dir, router, cookie, app_id) = siap("nilai-panjang").await;
    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;
    let nilai = format!("{}= % # \\ unicode-å", "x".repeat(8_001));
    let (status, _, body) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("new_key_0", "LONG_VALUE"),
                ("new_value_0", &nilai),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let semua = apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
        .await
        .expect("baca env vars");
    let (_, encrypted, _) = semua
        .iter()
        .find(|(key, _, _)| key == "LONG_VALUE")
        .expect("env panjang harus tersimpan");
    assert_eq!(state.crypto.decrypt(encrypted).unwrap(), nilai);
}

// ============================================================
// 7 — Simpan env saat app terkunci deploy lain: env TETAP tersimpan,
//     redeploy ditolak 409 (invariant §3 no.12: lock tetap dihormati)
// ============================================================

#[tokio::test]
async fn simpan_env_saat_lock_aktif_env_tetap_tersimpan_tapi_redeploy_ditolak_409() {
    let (state, _dir, router, cookie, app_id) = siap("lock-aktif").await;
    seed_deployment(&state, &app_id, StatusDeployment::Live).await;

    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let terkunci = apps_repo::acquire_lock(
        &state.db_write,
        &app_id,
        "lock-deploy-lain-yang-sedang-berjalan",
        now + LOCK_TTL_SECS,
    )
    .await
    .expect("ambil lock simulasi deploy lain");
    assert!(terkunci, "lock awal harus berhasil diambil");

    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;
    let (status, _, body) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("new_key_0", "FEATURE_FLAG"),
                ("new_value_0", "on"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert!(body.contains("deploy lain"));

    let tersimpan = apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
        .await
        .expect("baca env vars");
    assert_eq!(
        tersimpan.len(),
        1,
        "env HARUS tetap tersimpan walau redeploy ditolak"
    );

    let deploys = deployments_repo::list_by_app(&state.db_read, &app_id)
        .await
        .unwrap();
    assert_eq!(
        deploys.len(),
        1,
        "TIDAK ADA deployment baru dibuat saat lock aktif"
    );
}

// ============================================================
// 8 — Hapus dan set kosong bersamaan ditolak tanpa efek samping.
// ============================================================

#[tokio::test]
async fn hapus_dan_kosongkan_bersamaan_ditolak() {
    let (state, _dir, router, cookie, app_id) = siap("empty-conflict").await;
    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;
    let (status, _, body) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("new_key_0", "VALUE"),
                ("new_value_0", "lama"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;
    let (status, _, body) = send(
        &router,
        post_form(
            &format!("/apps/{app_id}/env"),
            &cookie,
            &[
                ("csrf_token", &csrf),
                ("delete__VALUE", "1"),
                ("empty__VALUE", "1"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body.contains("hapus atau set value menjadi kosong"));
    assert_eq!(
        apps_repo::list_env_vars_encrypted(&state.db_read, &app_id)
            .await
            .expect("baca env vars")
            .len(),
        1
    );
}

// ============================================================
// 9 — Id app tidak dikenal selalu 404, bukan 500
// ============================================================

#[tokio::test]
async fn id_app_tidak_dikenal_pada_env_selalu_404() {
    let (_state, _dir, router, cookie, app_id) = siap("404").await;
    // CSRF token VALID (milik sesi ini) dipakai supaya 404 yang diuji murni
    // soal id app, bukan tercampur dengan penolakan CSRF (pola sama
    // `tests/phase1.rs`: 404 harus berlaku "bahkan dengan token lain yang
    // valid").
    let csrf = ambil_csrf(&router, &cookie, &format!("/apps/{app_id}/env")).await;

    let (status, _, _) = send(
        &router,
        get_with_cookie("/apps/tidak-ada-idnya/env", &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = send(
        &router,
        post_form(
            "/apps/tidak-ada-idnya/env",
            &cookie,
            &[("csrf_token", &csrf), ("new_key_0", "X")],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
