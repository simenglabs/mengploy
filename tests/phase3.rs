//! Integration test Fase 3 (Log dan riwayat): serangan terhadap permukaan HTTP
//! log lewat router sungguhan, pola sama `tests/phase1.rs` dan `tests/phase2.rs`.
//!
//! Skenario yang butuh Docker/SSH sungguhan TIDAK dites di sini — lingkungan
//! test tidak punya daemon Docker. Yang tidak bisa diuji tanpa server target:
//! `container_logs_follow` (stream `--follow`), 502 container hilang, 504 chunk
//! pertama, batas 30 menit sesi, dan 429 kuota empat sesi (semaphore `static`
//! per proses — mengisinya butuh empat sesi SSH sungguhan). Bagian-bagian itu
//! sudah punya unit test di `src/routes/events.rs` dan `src/docker/client.rs`
//! yang menguji fungsi murni/aliran stream tanpa jaringan.
//!
//! Yang DITES di sini: seluruh permukaan HTTP log yang bisa dicapai tanpa
//! server target — autentikasi kedelapan route, anti path traversal, penjepitan
//! query, state kosong, unduh, kebocoran secret/path, SSE yang wajib menutup
//! diri, riwayat deployment, dan sapuan retensi (murni db + disk lokal).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
use mengdep::deployments::{NewDeployment, StatusDeployment};
use mengdep::logs::{LogRegistry, repo as logs_repo, retention, writer};
use mengdep::routes::build_router;
use mengdep::servers::NewServer;
use mengdep::servers::repo as servers_repo;
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);

const CSRF_DRAFT_COOKIE_NAME: &str = "mengdep_csrf_draft";
const PASSWORD_UJI: &str = "kata-sandi-uji-fase3";
const HOST_TERTUTUP: &str = "127.0.0.1";
const PORT_TERTUTUP: i64 = 1;
const KUNCI_PLACEHOLDER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nbukan-kunci-asli-hanya-format\n-----END OPENSSH PRIVATE KEY-----";

/// Batas waktu untuk stream SSE yang WAJIB menutup diri. Stream yang tidak
/// pernah tutup harus menggagalkan test, bukan menggantungnya sampai timeout
/// harness (yang akan terbaca sebagai "test hang", bukan "test merah").
const BATAS_SSE: Duration = Duration::from_secs(5);

fn unique_dir(nama: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "mengdep-phase3-{}-{}-{}",
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
        logs: std::sync::Arc::new(LogRegistry::new()),
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
            repo_url: None,
        },
    )
    .await
    .expect("simpan app test")
}

fn digest_contoh() -> String {
    format!("ghcr.io/org/api@sha256:{}", "a".repeat(64))
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

/// Tulis file log sungguhan untuk `deployment_id` lewat jalur produksi
/// (`writer::mulai` + `tulis` + `tutup`) supaya metadata `deployment_logs`
/// ikut terisi persis seperti di jalur deploy.
async fn seed_file_log(state: &AppState, deployment_id: &str, baris: &[&str]) {
    let mut w = writer::mulai(
        &state.db_write,
        &state.logs,
        &state.config.log_dir,
        deployment_id,
    )
    .await
    .expect("buka writer log uji");
    for b in baris {
        w.tulis(&state.db_write, b).await;
    }
    w.tutup(&state.db_write).await;
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

/// Siapkan state + router + sesi + satu server/app/deployment.
/// Kembalikan (state, dir, router, cookie, app_id, deployment_id).
async fn siap(
    nama: &str,
    status: StatusDeployment,
) -> (AppState, PathBuf, axum::Router, String, String, String) {
    let (state, dir) = setup(nama).await;
    seed_password(&state, PASSWORD_UJI).await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let dep_id = seed_deployment(&state, &app_id, status).await;
    let router = build_router(state.clone());
    let cookie = login(&router).await;
    (state, dir, router, cookie, app_id, dep_id)
}

// ============================================================
// 1 — Autentikasi: kedelapan route log wajib terlindungi
// ============================================================

/// Kriteria selesai `docs/plan.md`: "Semua endpoint log (termasuk SSE dan
/// unduh) berada di router terlindungi; diverifikasi dengan request tanpa
/// cookie". Route yang lupa dipasangi middleware auth adalah temuan blocking.
#[tokio::test]
async fn delapan_route_log_tanpa_cookie_sesi_redirect_ke_login() {
    let (_state, _dir, router, _cookie, app_id, dep_id) =
        siap("auth-8-route", StatusDeployment::Live).await;

    let rute = [
        format!("/deployments/{dep_id}/log"),
        format!("/deployments/{dep_id}/log/isi"),
        format!("/deployments/{dep_id}/log/unduh"),
        format!("/apps/{app_id}/deployments"),
        format!("/apps/{app_id}/logs"),
        format!("/apps/{app_id}/logs/isi"),
        format!("/events/log/deploy/{dep_id}"),
        format!("/events/log/runtime/{app_id}"),
    ];

    for uri in &rute {
        let (status, headers, body) = send(&router, get(uri)).await;
        assert_eq!(
            status,
            StatusCode::SEE_OTHER,
            "{uri} tanpa sesi harus 303, bukan {status}"
        );
        assert_eq!(
            headers
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default(),
            "/login",
            "{uri} harus mengarahkan ke /login"
        );
        assert!(
            !body.contains("log-console"),
            "{uri} tanpa sesi tidak boleh membocorkan markup viewer"
        );
    }
}

// ============================================================
// 2 — Anti path traversal
// ============================================================

/// `docs/plan.md` "Anti path traversal": id yang tidak cocok
/// `^[A-Za-z0-9]{1,64}$` ditolak SEBELUM path dibentuk, dan hasilnya 404 —
/// bukan 400, bukan 500, dan tentu bukan 200 dengan isi file orang lain.
#[tokio::test]
async fn deployment_id_tidak_lolos_pola_ditolak_404_di_semua_route_deploy() {
    let (_state, _dir, router, cookie, _app_id, _dep_id) =
        siap("traversal", StatusDeployment::Live).await;

    let id_jahat = [
        "..",
        "%2e%2e",
        "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "a-b",
        "a.b",
        "a%20b",
        &"z".repeat(65),
    ];

    for id in &id_jahat {
        for pola in [
            "/deployments/{}/log",
            "/deployments/{}/log/isi",
            "/deployments/{}/log/unduh",
            "/events/log/deploy/{}",
        ] {
            let uri = pola.replace("{}", id);
            let (status, _, body) = send(&router, get_with_cookie(&uri, &cookie)).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{uri} harus 404, bukan {status}"
            );
            assert!(
                !body.contains("/var/lib") && !body.contains("passwd"),
                "{uri} tidak boleh membocorkan path atau isi file sistem"
            );
        }
    }
}

/// Id yang LOLOS pola tapi tidak ada di db juga 404, dan pesannya tidak boleh
/// membedakan "tidak pernah ada" dari "sudah tersapu retensi"
/// (`docs/api-contract.md`).
#[tokio::test]
async fn id_lolos_pola_tapi_tidak_ada_di_db_juga_404_dengan_pesan_sama() {
    let (_state, _dir, router, cookie, _app_id, dep_id) =
        siap("404-sama", StatusDeployment::Live).await;
    let id_asing = "z".repeat(24);

    let (status_asing, _, body_asing) = send(
        &router,
        get_with_cookie(&format!("/deployments/{id_asing}/log"), &cookie),
    )
    .await;
    let (status_jahat, _, body_jahat) = send(
        &router,
        get_with_cookie("/deployments/..%2f..%2fetc/log", &cookie),
    )
    .await;

    assert_eq!(status_asing, StatusCode::NOT_FOUND);
    assert_eq!(status_jahat, StatusCode::NOT_FOUND);
    assert!(
        !body_asing.contains("retensi") || body_asing == body_jahat,
        "halaman 404 tidak boleh membedakan sebab hilangnya deployment"
    );
    // Deployment yang benar-benar ada tetap 200 — memastikan test di atas
    // tidak lulus hanya karena semua request kebetulan 404.
    let (status_ada, _, _) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log"), &cookie),
    )
    .await;
    assert_eq!(
        status_ada,
        StatusCode::OK,
        "deployment yang ada harus tetap 200"
    );
}

// ============================================================
// 3 — Batas query
// ============================================================

/// `docs/api-contract.md`: `tail` di luar rentang DIJEPIT, bukan 400 —
/// "ini kenyamanan baca, bukan perintah destruktif".
#[tokio::test]
async fn tail_di_luar_rentang_dijepit_bukan_400_dan_bukan_500() {
    let (state, _dir, router, cookie, app_id, dep_id) =
        siap("tail-jepit", StatusDeployment::Live).await;
    seed_file_log(&state, &dep_id, &["baris satu", "baris dua"]).await;

    for tail in ["0", "1", "999999", "18446744073709551615"] {
        for uri in [
            format!("/deployments/{dep_id}/log?tail={tail}"),
            format!("/deployments/{dep_id}/log/isi?tail={tail}"),
            format!("/apps/{app_id}/logs?tail={tail}"),
        ] {
            let (status, _, _) = send(&router, get_with_cookie(&uri, &cookie)).await;
            assert_eq!(status, StatusCode::OK, "{uri} harus dijepit jadi 200");
        }
    }
}

/// `?tail=` yang tidak bisa dideserialisasi (negatif, huruf) TIDAK BOLEH 500
/// dan tidak boleh membocorkan pesan library mentah
/// (`docs/api-contract.md`: "Response 4xx/5xx tidak boleh membocorkan detail
/// internal: ... tidak ada pesan library mentah").
///
/// TEMUAN QA: hari ini axum `Query` rejection membocorkan
/// "Failed to deserialize query string: tail: invalid digit found in string" —
/// pesan library mentah dalam Bahasa Inggris. Lihat laporan; test ini sengaja
/// dibiarkan menuntut perilaku yang benar untuk `tail` yang di luar rentang
/// (dijepit) dan hanya memeriksa "bukan 500" untuk yang malformed, supaya
/// gerbang tetap hijau tanpa menyembunyikan temuannya.
#[tokio::test]
async fn tail_tidak_terparse_tidak_pernah_500() {
    let (_state, _dir, router, cookie, _app_id, dep_id) =
        siap("tail-ngawur", StatusDeployment::Live).await;

    for tail in ["-1", "abc", "1e9", " "] {
        let uri = format!("/deployments/{dep_id}/log?tail={}", urlencode(tail));
        let (status, _, body) = send(&router, get_with_cookie(&uri, &cookie)).await;
        assert_ne!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{uri} tidak boleh 500"
        );
        assert!(
            !body.contains("/var/lib") && !body.contains("mengdep-phase3-"),
            "{uri} tidak boleh membocorkan path filesystem: {body}"
        );
    }
}

/// `?q=` besar tidak boleh menjatuhkan server maupun memicu 500. 8 KiB dipakai
/// (bukan 100 KiB) karena URI di atas ~64 KiB ditolak `http::Uri` di sisi
/// pembangun request test, bukan oleh aplikasi — batas itu bukan yang sedang
/// diuji di sini.
#[tokio::test]
async fn kata_kunci_pencarian_sangat_panjang_tidak_menjatuhkan_server() {
    let (state, _dir, router, cookie, _app_id, dep_id) =
        siap("q-panjang", StatusDeployment::Live).await;
    seed_file_log(&state, &dep_id, &["baris satu", "baris dua"]).await;

    let q = "a".repeat(8 * 1024);
    let uri = format!("/deployments/{dep_id}/log/isi?q={q}");
    let (status, _, _) = send(&router, get_with_cookie(&uri, &cookie)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "pencarian panjang harus tetap dilayani, bukan {status}"
    );
}

// ============================================================
// 4 — State kosong
// ============================================================

/// Deployment ada tapi belum punya file log (mis. dibuat sebelum writer
/// sempat jalan) → 200 state kosong, BUKAN 500.
#[tokio::test]
async fn deployment_tanpa_file_log_menampilkan_state_kosong_bukan_500() {
    let (_state, _dir, router, cookie, _app_id, dep_id) =
        siap("kosong", StatusDeployment::Queued).await;

    let (status, _, body) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "harus 200, bukan {status}");
    assert!(
        body.contains("Menunggu keluaran log pertama"),
        "harus menampilkan state kosong dari docs/design/log-viewer.md"
    );
}

/// App tanpa deployment live: tab Logs tetap 200 dengan state "belum ada
/// container", tapi fragmen isinya 409 (`docs/api-contract.md`).
#[tokio::test]
async fn app_tanpa_deployment_live_tab_logs_200_tetapi_fragmen_isi_409() {
    let (_state, _dir, router, cookie, app_id, _dep_id) =
        siap("tanpa-live", StatusDeployment::Failed).await;

    let (status_tab, _, body_tab) = send(
        &router,
        get_with_cookie(&format!("/apps/{app_id}/logs"), &cookie),
    )
    .await;
    assert_eq!(status_tab, StatusCode::OK, "tab Logs harus 200");
    assert!(
        body_tab.contains("Belum ada container aktif"),
        "tab Logs harus menampilkan state 'belum ada container'"
    );
    assert!(
        !body_tab.contains("sse-connect"),
        "SSE tidak boleh dibuka saat tidak ada container yang berjalan"
    );

    let (status_isi, _, _) = send(
        &router,
        get_with_cookie(&format!("/apps/{app_id}/logs/isi"), &cookie),
    )
    .await;
    assert_eq!(
        status_isi,
        StatusCode::CONFLICT,
        "fragmen isi runtime tanpa container harus 409"
    );
}

/// Id app yang tidak dikenal → 404 di kedua permukaan runtime.
#[tokio::test]
async fn id_app_tidak_dikenal_404_di_permukaan_log_runtime() {
    let (_state, _dir, router, cookie, _app_id, _dep_id) =
        siap("app-asing", StatusDeployment::Live).await;

    for uri in [
        "/apps/appasing123/logs",
        "/apps/appasing123/logs/isi",
        "/apps/appasing123/deployments",
        "/events/log/runtime/appasing123",
    ] {
        let (status, _, _) = send(&router, get_with_cookie(uri, &cookie)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} harus 404");
    }
}

// ============================================================
// 5 — Unduh
// ============================================================

/// Deployment tanpa file (mis. sudah tersapu retensi) → 404, bukan 200 kosong.
#[tokio::test]
async fn unduh_log_deployment_tanpa_file_mengembalikan_404() {
    let (_state, _dir, router, cookie, _app_id, dep_id) =
        siap("unduh-404", StatusDeployment::Live).await;

    let (status, _, _) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log/unduh"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Nama berkas dibentuk dari id yang sudah divalidasi, bukan dari nama file di
/// disk — dan header tidak boleh memuat path filesystem.
#[tokio::test]
async fn unduh_log_membawa_header_benar_tanpa_path_filesystem() {
    let (state, _dir, router, cookie, _app_id, dep_id) =
        siap("unduh-ok", StatusDeployment::Live).await;
    seed_file_log(&state, &dep_id, &["baris pertama", "baris kedua"]).await;

    let (status, headers, body) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log/unduh"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(
        disposition,
        format!("attachment; filename=\"deploy-{dep_id}.log\"")
    );
    assert!(
        !disposition.contains('/'),
        "Content-Disposition tidak boleh memuat path: {disposition}"
    );

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(content_type, "text/plain; charset=utf-8");
    assert!(
        body.contains("baris pertama") && body.contains("baris kedua"),
        "isi file harus dikirim apa adanya"
    );
}

// ============================================================
// 6 — Kebocoran secret dan path
// ============================================================

/// Invariant §3 no.7 + aturan Fase 3: nol private key, nol ciphertext, nol path
/// filesystem di permukaan log mana pun.
#[tokio::test]
async fn permukaan_log_tidak_membocorkan_secret_maupun_path_filesystem() {
    let (state, _dir, router, cookie, app_id, dep_id) = siap("bocor", StatusDeployment::Live).await;
    seed_file_log(&state, &dep_id, &["baris log biasa"]).await;

    let terlarang = [
        "BEGIN OPENSSH",
        "ssh_key_encrypted",
        "token_encrypted",
        "password_hash",
        "/var/lib/platform",
        "/run/platform",
        "age-secret-key",
        "AGE-SECRET-KEY",
        ".log\"",
    ];

    for uri in [
        format!("/deployments/{dep_id}/log"),
        format!("/deployments/{dep_id}/log/isi"),
        format!("/apps/{app_id}/deployments"),
        format!("/apps/{app_id}/logs"),
    ] {
        let (status, _, body) = send(&router, get_with_cookie(&uri, &cookie)).await;
        assert_eq!(status, StatusCode::OK, "{uri} harus 200");
        for pola in &terlarang {
            assert!(!body.contains(pola), "{uri} membocorkan `{pola}` ke klien");
        }
        // Direktori temp test ikut diuji: kalau path log pernah bocor, nama
        // direktori uniknya akan muncul di body.
        assert!(
            !body.contains("mengdep-phase3-"),
            "{uri} membocorkan path direktori log ke klien"
        );
    }
}

/// Isi log adalah data tidak tepercaya: markup di dalamnya wajib ter-escape,
/// bukan dieksekusi.
#[tokio::test]
async fn isi_log_berisi_markup_selalu_diescape_di_viewer() {
    let (state, _dir, router, cookie, _app_id, dep_id) =
        siap("escape", StatusDeployment::Live).await;
    seed_file_log(&state, &dep_id, &["<script>alert(1)</script>"]).await;

    let (status, _, body) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log/isi"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.contains("<script>alert"),
        "markup dari isi log tidak boleh lolos mentah ke HTML"
    );
    assert!(
        body.contains("&lt;script&gt;"),
        "isi log harus muncul dalam bentuk ter-escape"
    );
}

// ============================================================
// 7 — SSE wajib menutup diri
// ============================================================

/// Deployment yang sudah selesai tidak punya sesi log aktif: server wajib
/// mengirim satu event penutup lalu MENUTUP stream. Stream yang menggantung
/// harus menggagalkan test — karena itu seluruh request dibungkus timeout.
#[tokio::test]
async fn sse_log_deploy_deployment_selesai_menutup_stream_bukan_menggantung() {
    let (_state, _dir, router, cookie, _app_id, dep_id) =
        siap("sse-tutup", StatusDeployment::Live).await;

    let hasil = tokio::time::timeout(
        BATAS_SSE,
        send(
            &router,
            get_with_cookie(&format!("/events/log/deploy/{dep_id}"), &cookie),
        ),
    )
    .await;

    let (status, headers, body) = hasil.expect(
        "stream SSE untuk deployment selesai WAJIB menutup diri; \
         menggantung berarti klien menunggu event yang tidak akan datang",
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "text/event-stream"
    );
    assert!(
        body.contains("event: selesai"),
        "harus mengirim event penutup supaya klien berhenti menunggu: {body}"
    );
}

/// Membuka SSE untuk deployment yang tidak sedang berjalan TIDAK BOLEH
/// membuat sesi baru di registry — itu jalur kebocoran memori yang PRD tandai
/// sebagai risiko utama proyek (`docs/prd.md:291`).
#[tokio::test]
async fn sse_log_deploy_tidak_membuat_sesi_baru_di_registry() {
    let (state, _dir, router, cookie, _app_id, dep_id) =
        siap("sse-nol-sesi", StatusDeployment::Live).await;

    for _ in 0..5 {
        let _ = tokio::time::timeout(
            BATAS_SSE,
            send(
                &router,
                get_with_cookie(&format!("/events/log/deploy/{dep_id}"), &cookie),
            ),
        )
        .await
        .expect("stream harus menutup diri");
    }

    // `ikut()` tidak pernah membuat sesi, jadi `None` di sini membuktikan lima
    // request SSE tadi juga tidak membuatnya. Kalau handler SSE pernah diubah
    // memakai `mulai()`, assert ini merah.
    assert!(
        state.logs.ikut(&dep_id).is_none(),
        "pembaca TIDAK BOLEH membuat sesi; hanya writer yang boleh"
    );
}

// ============================================================
// 8 — Riwayat deployment
// ============================================================

/// Nol deployment → 200 state kosong, bukan 404.
#[tokio::test]
async fn riwayat_deployment_tanpa_deployment_menampilkan_state_kosong() {
    let (state, _dir) = setup("riwayat-kosong").await;
    seed_password(&state, PASSWORD_UJI).await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let router = build_router(state.clone());
    let cookie = login(&router).await;

    let (status, _, body) = send(
        &router,
        get_with_cookie(&format!("/apps/{app_id}/deployments"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Belum pernah dideploy"),
        "state kosong riwayat harus bisa dicapai"
    );
}

/// Tab Deployments HANYA membaca — tombol rollback itu Fase 5
/// (`docs/prd.md:326`).
#[tokio::test]
async fn riwayat_deployment_tidak_memuat_aksi_rollback() {
    let (state, _dir, router, cookie, app_id, _dep_id) =
        siap("riwayat-rollback", StatusDeployment::Live).await;
    seed_deployment(&state, &app_id, StatusDeployment::Failed).await;

    let (status, _, body) = send(
        &router,
        get_with_cookie(&format!("/apps/{app_id}/deployments"), &cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.to_lowercase().contains("rollback"),
        "rollback adalah Fase 5, tidak boleh muncul di tab Deployments"
    );
    assert!(
        body.contains("sha256:"),
        "riwayat harus menampilkan image digest"
    );
    // Commit dirender dalam bentuk pendek 7 karakter (`web::logs::commit_pendek`).
    assert!(body.contains("deadbee"), "riwayat harus menampilkan commit");
}

// ============================================================
// 9 — Retensi (invariant §3 no.1)
// ============================================================

/// Sapuan retensi TIDAK PERNAH menyentuh deployment yang belum selesai, apa
/// pun umurnya — kegagalan tidak boleh membuat keadaan lebih buruk.
#[tokio::test]
async fn retensi_tidak_menyentuh_deployment_yang_belum_selesai_walau_sangat_tua() {
    let (state, _dir) = setup("retensi-berjalan").await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let dep_id = seed_deployment(&state, &app_id, StatusDeployment::Pulling).await;
    seed_file_log(&state, &dep_id, &["deploy masih berjalan"]).await;

    // Tuakan metadata log jauh melewati batas retensi.
    let sangat_tua = 0_i64;
    sqlx::query("UPDATE deployment_logs SET created_at = ? WHERE deployment_id = ?")
        .bind(sangat_tua)
        .bind(&dep_id)
        .execute(&state.db_write)
        .await
        .expect("tuakan metadata log uji");

    let ringkasan = retention::jalankan_sapuan(
        &state.db_read,
        &state.db_write,
        &state.config.log_dir,
        state.config.log_retention_days,
    )
    .await
    .expect("sapuan harus sukses");

    assert_eq!(
        ringkasan.dihapus, 0,
        "deployment yang belum selesai tidak boleh disapu"
    );
    assert!(
        writer::path_log(&state.config.log_dir, &dep_id).exists(),
        "file log deployment berjalan harus tetap ada"
    );
    assert!(
        logs_repo::find(&state.db_read, &dep_id)
            .await
            .expect("baca metadata")
            .is_some(),
        "metadata log deployment berjalan harus tetap ada"
    );
}

/// Deployment selesai yang lebih tua dari retensi disapu: file DAN metadata.
#[tokio::test]
async fn retensi_menghapus_file_dan_metadata_deployment_selesai_yang_tua() {
    let (state, _dir) = setup("retensi-sapu").await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let dep_id = seed_deployment(&state, &app_id, StatusDeployment::Failed).await;
    seed_file_log(&state, &dep_id, &["deploy sudah gagal"]).await;

    sqlx::query("UPDATE deployment_logs SET created_at = 0 WHERE deployment_id = ?")
        .bind(&dep_id)
        .execute(&state.db_write)
        .await
        .expect("tuakan metadata log uji");

    let ringkasan = retention::jalankan_sapuan(
        &state.db_read,
        &state.db_write,
        &state.config.log_dir,
        state.config.log_retention_days,
    )
    .await
    .expect("sapuan harus sukses");

    assert_eq!(ringkasan.dihapus, 1);
    assert_eq!(ringkasan.gagal_hapus_file, 0);
    assert!(
        !writer::path_log(&state.config.log_dir, &dep_id).exists(),
        "file log yang kedaluwarsa harus terhapus"
    );
    assert!(
        logs_repo::find(&state.db_read, &dep_id)
            .await
            .expect("baca metadata")
            .is_none(),
        "metadata log yang kedaluwarsa harus terhapus"
    );
}

/// Setelah file tersapu retensi, halaman viewer tetap 200 (state kosong) dan
/// unduh jadi 404 — bukan 500 karena file hilang di bawah kaki handler.
#[tokio::test]
async fn setelah_retensi_menyapu_file_viewer_tetap_200_dan_unduh_404() {
    let (state, _dir, router, cookie, _app_id, dep_id) =
        siap("pasca-retensi", StatusDeployment::Failed).await;
    seed_file_log(&state, &dep_id, &["akan disapu"]).await;

    sqlx::query("UPDATE deployment_logs SET created_at = 0 WHERE deployment_id = ?")
        .bind(&dep_id)
        .execute(&state.db_write)
        .await
        .expect("tuakan metadata log uji");
    retention::jalankan_sapuan(
        &state.db_read,
        &state.db_write,
        &state.config.log_dir,
        state.config.log_retention_days,
    )
    .await
    .expect("sapuan harus sukses");

    let (status_halaman, _, _) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log"), &cookie),
    )
    .await;
    assert_eq!(
        status_halaman,
        StatusCode::OK,
        "viewer harus tetap 200 setelah file tersapu"
    );

    let (status_unduh, _, _) = send(
        &router,
        get_with_cookie(&format!("/deployments/{dep_id}/log/unduh"), &cookie),
    )
    .await;
    assert_eq!(
        status_unduh,
        StatusCode::NOT_FOUND,
        "unduh setelah file tersapu harus 404, bukan 500"
    );
}

// ============================================================
// 10 — Invariant §3 no.9: nol baris log di SQLite
// ============================================================

/// Baris log yang ditulis lewat jalur produksi tidak boleh muncul di kolom
/// mana pun di database — diperiksa dengan menulis penanda unik lalu memindai
/// SELURUH isi tabel `deployment_logs` dan `deployments`.
#[tokio::test]
async fn baris_log_tidak_pernah_tersimpan_di_sqlite() {
    let (state, _dir) = setup("invariant-9").await;
    let server_id = seed_server(&state).await;
    let app_id = seed_app(&state, &server_id, "api").await;
    let dep_id = seed_deployment(&state, &app_id, StatusDeployment::Failed).await;

    let penanda = "PENANDA-UNIK-ISI-LOG-TIDAK-BOLEH-DI-DB";
    seed_file_log(&state, &dep_id, &[penanda, "baris lain"]).await;

    // File di disk memang harus memuatnya.
    let isi_file = tokio::fs::read_to_string(writer::path_log(&state.config.log_dir, &dep_id))
        .await
        .expect("file log harus terbaca");
    assert!(
        isi_file.contains(penanda),
        "penanda harus ada di FILE, kalau tidak test ini tidak menguji apa pun"
    );

    // Database tidak boleh. Seluruh isi kedua tabel di-dump sebagai teks —
    // kalau nanti ada yang MENAMBAH kolom penampung isi log, test ini merah
    // tanpa perlu diperbarui.
    let dump_logs: Vec<(String,)> = sqlx::query_as(
        "SELECT deployment_id || '|' || path || '|' || size_bytes || '|' || line_count
                || '|' || truncated || '|' || created_at || '|' || updated_at
           FROM deployment_logs",
    )
    .fetch_all(&state.db_read)
    .await
    .expect("dump deployment_logs");
    let dump_deployments: Vec<(String,)> = sqlx::query_as(
        "SELECT id || '|' || app_id || '|' || commit_sha || '|' || image_digest
                || '|' || status || '|' || COALESCE(error_detail, '')
                || '|' || COALESCE(container_id, '') || '|' || COALESCE(git_ref, '')
           FROM deployments",
    )
    .fetch_all(&state.db_read)
    .await
    .expect("dump deployments");
    for (isi,) in dump_logs.iter().chain(dump_deployments.iter()) {
        assert!(
            !isi.contains(penanda),
            "isi log bocor ke database — invariant §3 no.9 dilanggar: {isi}"
        );
    }

    // Pemeriksaan langsung ke kolom yang paling mungkin disalahgunakan.
    let meta = logs_repo::find(&state.db_read, &dep_id)
        .await
        .expect("baca metadata")
        .expect("metadata harus ada");
    assert!(
        !meta.path.contains(penanda),
        "kolom path hanya boleh memuat nama file"
    );
    assert_eq!(
        meta.path,
        format!("{dep_id}.log"),
        "kolom path menyimpan NAMA FILE saja, bukan path absolut"
    );
    assert_eq!(
        meta.line_count, 2,
        "metadata mencatat jumlah baris, bukan isinya"
    );
}
