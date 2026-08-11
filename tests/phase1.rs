//! Integration test Fase 1 (Registry server dan konektivitas): skenario
//! injeksi kegagalan lewat router sungguhan (`docs/plan.md` "qa") — host
//! tidak terjangkau, kredensial ditolak sebelum menyentuh jaringan, id
//! tidak dikenal, job verifikasi ganda, dan worker polling tidak pernah
//! menyentuh server yang belum lolos verifikasi awal.
//!
//! Sama pendekatan dengan `tests/phase0.rs`: router lewat
//! `axum::ServiceExt::oneshot`, tanpa server TCP nyata untuk request HTTP
//! murni. Skenario yang butuh SSH sungguhan (host tidak terjangkau, dsb)
//! sengaja mengarah ke `127.0.0.1` pada port tertutup — RST langsung dari
//! kernel, bukan menunggu timeout penuh, supaya test tetap cepat tanpa
//! kehilangan makna "koneksi ditolak/gagal" yang sesungguhnya.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::util::ServiceExt;

use mengdep::auth::middleware::SESSION_COOKIE_NAME;
use mengdep::auth::password::hash_password;
use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::routes::build_router;
use mengdep::servers::model::StatusServer;
use mengdep::servers::repo as servers_repo;
use mengdep::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);

const CSRF_DRAFT_COOKIE_NAME: &str = "mengdep_csrf_draft";

/// Host+port yang dijamin menolak koneksi TCP segera (localhost, port
/// istimewa yang tidak pernah punya listener tanpa hak root) — dipakai
/// setiap skenario yang butuh kegagalan koneksi SSH nyata tapi cepat.
const HOST_TERTUTUP: &str = "127.0.0.1";
const PORT_TERTUTUP: i64 = 1;

const KUNCI_PLACEHOLDER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nbukan-kunci-asli-hanya-format\n-----END OPENSSH PRIVATE KEY-----";

fn unique_dir(nama: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "mengdep-phase1-{}-{}-{}",
        std::process::id(),
        n,
        nama
    ))
}

fn tulis_kunci_age_ke(dir: &std::path::Path) -> PathBuf {
    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;

    let identity = age::x25519::Identity::generate();
    let path = dir.join("key.txt");
    std::fs::create_dir_all(dir).expect("bikin direktori temp test harus sukses");
    std::fs::write(&path, identity.to_string().expose_secret())
        .expect("tulis file kunci sementara harus sukses");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("set mode file kunci sementara harus sukses");
    path
}

async fn setup(nama: &str) -> (AppState, PathBuf) {
    let dir = unique_dir(nama);
    let db_path = dir.join("test.db");
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi db baru harus sukses");

    let key_path = tulis_kunci_age_ke(&dir);
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

async fn ambil_login(app: &axum::Router) -> (String, String) {
    let (status, headers, body) = send(app, get("/login")).await;
    assert_eq!(status, StatusCode::OK, "GET /login harus 200");
    let draft = ambil_cookie_dari_set_cookie(&headers, CSRF_DRAFT_COOKIE_NAME)
        .expect("GET /login harus men-set cookie draft csrf");
    let draft_header = format!("{CSRF_DRAFT_COOKIE_NAME}={draft}");
    let token = parse_hidden_csrf(&body).expect("form login harus menanam csrf_token");
    (draft_header, token)
}

/// Login, kembalikan header `Cookie:` sesi siap pakai dan token CSRF sesi
/// (dipanen dari dashboard, dipakai semua form Fase 1 di test ini).
async fn login(app: &axum::Router, password: &str) -> (String, String) {
    let (draft, token) = ambil_login(app).await;
    let (status, headers, _body) = send(
        app,
        post_form(
            "/login",
            &draft,
            &[("password", password), ("csrf_token", &token)],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "login benar harus 303");
    let session_cookie = ambil_cookie_dari_set_cookie(&headers, SESSION_COOKIE_NAME)
        .expect("response login harus men-set cookie sesi");
    let session_header = format!("{SESSION_COOKIE_NAME}={session_cookie}");

    let (status2, _, body2) = send(app, get_with_cookie("/", &session_header)).await;
    assert_eq!(status2, StatusCode::OK, "dashboard harus 200 setelah login");
    let csrf =
        parse_hidden_csrf(&body2).expect("dashboard harus menanam csrf_token di form logout");

    (session_header, csrf)
}

/// Tunggu sampai job verifikasi mencatat hasil akhir: `last_error_kind`
/// terisi (gagal) atau status jadi `online` (sukses), atau batas waktu
/// tercapai. **Bukan** menunggu status keluar dari `Verifying` — tepat
/// setelah `POST /servers`, status masih `pending` (task yang di-spawn
/// belum sempat jalan sama sekali), jadi cek "bukan lagi Verifying" bisa
/// benar SEBELUM job pernah mulai (race, pernah bikin test ini flaky-false-pass).
async fn tunggu_verifikasi_selesai(state: &AppState, id: &str, batas: Duration) -> String {
    let mulai = std::time::Instant::now();
    loop {
        let row = servers_repo::find_ringkas_by_id(&state.db_read, id)
            .await
            .expect("baca status server")
            .expect("server harus ada");
        if row.last_error_kind.is_some() || row.status == StatusServer::Online {
            return row.last_error_kind.unwrap_or_default();
        }
        if mulai.elapsed() > batas {
            panic!(
                "verifikasi tidak selesai dalam {batas:?} — status terakhir: {:?}",
                row.status
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Skenario 1 — host tidak terjangkau: job verifikasi gagal dengan kategori
/// `host_unreachable`, server TETAP ada (invariant 1), status kembali ke
/// `pending` (bukan dihapus, bukan `unreachable` di percobaan pertama).
#[tokio::test]
async fn host_tidak_terjangkau_verifikasi_gagal_server_tetap_ada() {
    let (state, dir) = setup("host-tertutup").await;
    seed_password(&state, "kata-sandi-qa").await;
    let app = build_router(state.clone());
    let (session, csrf) = login(&app, "kata-sandi-qa").await;

    let (status, headers, _) = send(
        &app,
        post_form(
            "/servers",
            &session,
            &[
                ("csrf_token", &csrf),
                ("name", "vps-tidak-terjangkau"),
                ("host", HOST_TERTUTUP),
                ("port", &PORT_TERTUTUP.to_string()),
                ("ssh_user", "root"),
                ("ssh_key", KUNCI_PLACEHOLDER),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "POST /servers harus 303");
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let id = location
        .strip_prefix("/servers/")
        .and_then(|s| s.strip_suffix("/verifikasi"))
        .expect("redirect harus ke /servers/{id}/verifikasi")
        .to_string();

    let error_kind = tunggu_verifikasi_selesai(&state, &id, Duration::from_secs(15)).await;
    assert!(
        !error_kind.is_empty(),
        "verifikasi ke host tertutup harus tercatat gagal dengan kategori, bukan kosong"
    );

    let server = servers_repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .expect("baca server")
        .expect("server TIDAK BOLEH hilang setelah verifikasi gagal (invariant 1)");
    assert_eq!(
        server.status,
        StatusServer::Pending,
        "percobaan pertama gagal harus kembali ke pending, bukan unreachable/dihapus"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Skenario 2 — kredensial (format kunci) ditolak SEBELUM menyentuh
/// jaringan sama sekali: validasi gagal di handler, tidak ada baris
/// `servers` yang dibuat.
#[tokio::test]
async fn format_kunci_salah_ditolak_tanpa_menyentuh_db() {
    let (state, dir) = setup("kunci-salah").await;
    seed_password(&state, "kata-sandi-qa").await;
    let app = build_router(state.clone());
    let (session, csrf) = login(&app, "kata-sandi-qa").await;

    let (status, _, body) = send(
        &app,
        post_form(
            "/servers",
            &session,
            &[
                ("csrf_token", &csrf),
                ("name", "vps-kunci-salah"),
                ("host", "example.internal"),
                ("port", "22"),
                ("ssh_user", "root"),
                ("ssh_key", "bukan-format-openssh-sama-sekali"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Format kunci privat tidak valid"));

    let jumlah = servers_repo::list_ringkas(&state.db_read)
        .await
        .expect("baca daftar server");
    assert!(
        jumlah.is_empty(),
        "validasi gagal tidak boleh membuat baris server sama sekali"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Skenario 3 — CSRF hilang pada `POST /servers`: ditolak, tanpa efek
/// samping (tidak ada baris server dibuat, tidak ada job di-spawn).
#[tokio::test]
async fn csrf_salah_pada_post_servers_ditolak_tanpa_efek() {
    let (state, dir) = setup("csrf-salah").await;
    seed_password(&state, "kata-sandi-qa").await;
    let app = build_router(state.clone());
    let (session, _csrf) = login(&app, "kata-sandi-qa").await;

    let (status, _, _) = send(
        &app,
        post_form(
            "/servers",
            &session,
            &[
                ("csrf_token", "token-ngawur"),
                ("name", "vps-csrf-salah"),
                ("host", "example.internal"),
                ("port", "22"),
                ("ssh_user", "root"),
                ("ssh_key", KUNCI_PLACEHOLDER),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let jumlah = servers_repo::list_ringkas(&state.db_read)
        .await
        .expect("baca daftar server");
    assert!(jumlah.is_empty(), "csrf salah tidak boleh membuat server");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Skenario 4 — id tidak dikenal selalu 404 di seluruh permukaan Fase 1,
/// tidak pernah 500 (`docs/api-contract.md`: "Id yang tidak dikenal → 404,
/// bukan 403 dan bukan 500").
#[tokio::test]
async fn id_tidak_dikenal_selalu_404_bukan_500() {
    let (state, dir) = setup("id-tidak-dikenal").await;
    seed_password(&state, "kata-sandi-qa").await;
    let app = build_router(state.clone());
    let (session, csrf) = login(&app, "kata-sandi-qa").await;

    let fake_id = "id-yang-tidak-pernah-ada";

    let (s1, _, _) = send(
        &app,
        get_with_cookie(&format!("/servers/{fake_id}"), &session),
    )
    .await;
    assert_eq!(s1, StatusCode::NOT_FOUND, "GET /servers/{{id}}");

    let (s2, _, _) = send(
        &app,
        get_with_cookie(&format!("/servers/{fake_id}/verifikasi"), &session),
    )
    .await;
    assert_eq!(s2, StatusCode::NOT_FOUND, "GET /servers/{{id}}/verifikasi");

    let (s3, _, _) = send(
        &app,
        post_form(
            &format!("/servers/{fake_id}/verifikasi/ulang"),
            &session,
            &[("csrf_token", &csrf)],
        ),
    )
    .await;
    assert_eq!(s3, StatusCode::NOT_FOUND, "POST verifikasi/ulang");

    let (s4, _, _) = send(
        &app,
        post_form(
            &format!("/servers/{fake_id}/hostkey/konfirmasi"),
            &session,
            &[("csrf_token", &csrf), ("fingerprint", "SHA256:apa-saja")],
        ),
    )
    .await;
    assert_eq!(s4, StatusCode::NOT_FOUND, "POST hostkey/konfirmasi");

    let (s5, _, _) = send(
        &app,
        get_with_cookie(&format!("/servers/{fake_id}/registry"), &session),
    )
    .await;
    assert_eq!(s5, StatusCode::NOT_FOUND, "GET /servers/{{id}}/registry");

    let (s6, _, _) = send(
        &app,
        post_form(
            &format!("/servers/{fake_id}/registry"),
            &session,
            &[
                ("csrf_token", &csrf),
                ("host", "ghcr.io"),
                ("username", "u"),
                ("token", "t"),
            ],
        ),
    )
    .await;
    assert_eq!(s6, StatusCode::NOT_FOUND, "POST /servers/{{id}}/registry");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Skenario 5 — job verifikasi ganda: server yang statusnya SUDAH
/// `verifying` menolak `POST .../verifikasi/ulang` dengan 409, TIDAK
/// memulai job kedua (`docs/api-contract.md`).
#[tokio::test]
async fn verifikasi_ulang_saat_sudah_berjalan_ditolak_409() {
    let (state, dir) = setup("verifikasi-ganda").await;
    seed_password(&state, "kata-sandi-qa").await;
    let app = build_router(state.clone());
    let (session, csrf) = login(&app, "kata-sandi-qa").await;

    let ssh_key_encrypted = state
        .crypto
        .encrypt(KUNCI_PLACEHOLDER)
        .expect("enkripsi kunci placeholder");
    let id = servers_repo::insert_pending(
        &state.db_write,
        servers_repo::NewServer {
            name: "vps-verifying",
            host: HOST_TERTUTUP,
            port: PORT_TERTUTUP,
            ssh_user: "root",
            ssh_key_encrypted: &ssh_key_encrypted,
        },
    )
    .await
    .expect("insert server langsung lewat repo");
    servers_repo::set_status_verifying(&state.db_write, &id)
        .await
        .expect("paksa status verifying");

    let (status, _, body) = send(
        &app,
        post_form(
            &format!("/servers/{id}/verifikasi/ulang"),
            &session,
            &[("csrf_token", &csrf)],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "job ganda harus 409");
    assert!(body.contains("Verifikasi sedang berjalan"));

    let server = servers_repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .expect("baca server")
        .expect("server harus tetap ada");
    assert_eq!(
        server.status,
        StatusServer::Verifying,
        "status tidak boleh berubah akibat percobaan job kedua yang ditolak"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Skenario 6 (regresi) — worker polling TIDAK PERNAH menyentuh server
/// yang belum lolos verifikasi awal (belum ada `host_key_fingerprint`),
/// walau `next_poll_at`-nya sudah jatuh tempo. Ini bug nyata yang pernah
/// terjadi (`docs/progress.md`, smoke test manual): server baru bisa
/// disulap statusnya jadi `online` oleh worker sebelum verifikasi awal
/// selesai.
#[tokio::test]
async fn worker_tidak_menyentuh_server_yang_belum_terverifikasi() {
    let (state, dir) = setup("worker-belum-verifikasi").await;

    let ssh_key_encrypted = state
        .crypto
        .encrypt(KUNCI_PLACEHOLDER)
        .expect("enkripsi kunci placeholder");
    let id = servers_repo::insert_pending(
        &state.db_write,
        servers_repo::NewServer {
            name: "vps-baru",
            host: HOST_TERTUTUP,
            port: PORT_TERTUTUP,
            ssh_user: "root",
            ssh_key_encrypted: &ssh_key_encrypted,
        },
    )
    .await
    .expect("insert server baru (next_poll_at=0, tanpa fingerprint)");

    mengdep::worker::status_poll::jalankan_satu_siklus(&state).await;

    let server = servers_repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .expect("baca server")
        .expect("server harus tetap ada");
    assert_eq!(
        server.status,
        StatusServer::Pending,
        "worker tidak boleh mengubah status server yang belum pernah lolos verifikasi awal"
    );
    assert_eq!(
        server.consecutive_failures, 0,
        "worker tidak boleh menghitung kegagalan untuk server yang bahkan tidak dipollingnya"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Skenario 7 — worker: kegagalan berturut-turut membuat backoff makin
/// panjang dan status baru menjadi `unreachable` PERSIS di kegagalan
/// ketiga, tidak lebih cepat (`docs/plan.md` "verifikasi backoff benar-benar
/// melambat"). Diuji lewat siklus worker sungguhan (bukan hanya fungsi
/// murni) supaya integrasi db-nya ikut teruji.
#[tokio::test]
async fn worker_backoff_bertambah_dan_unreachable_persis_kegagalan_ketiga() {
    let (state, dir) = setup("worker-backoff").await;

    let ssh_key_encrypted = state
        .crypto
        .encrypt(KUNCI_PLACEHOLDER)
        .expect("enkripsi kunci placeholder");
    let id = servers_repo::insert_pending(
        &state.db_write,
        servers_repo::NewServer {
            name: "vps-online-lalu-mati",
            host: HOST_TERTUTUP,
            port: PORT_TERTUTUP,
            ssh_user: "root",
            ssh_key_encrypted: &ssh_key_encrypted,
        },
    )
    .await
    .expect("insert server");
    servers_repo::set_host_key_fingerprint(&state.db_write, &id, "SHA256:fingerprint-uji")
        .await
        .expect("set fingerprint supaya lolos filter list_due_for_poll");
    sqlx::query("UPDATE servers SET status = 'online' WHERE id = ?")
        .bind(&id)
        .execute(&state.db_write)
        .await
        .expect("paksa status online sebelum siklus gagal pertama");

    let mut next_poll_sebelumnya = 0i64;
    for kegagalan_ke in 1..=3 {
        sqlx::query("UPDATE servers SET next_poll_at = 0 WHERE id = ?")
            .bind(&id)
            .execute(&state.db_write)
            .await
            .expect("paksa jatuh tempo untuk siklus berikutnya");

        mengdep::worker::status_poll::jalankan_satu_siklus(&state).await;

        let server = servers_repo::find_ringkas_by_id(&state.db_read, &id)
            .await
            .expect("baca server")
            .expect("server harus tetap ada");
        assert_eq!(server.consecutive_failures, kegagalan_ke);

        if kegagalan_ke < 3 {
            assert_eq!(
                server.status,
                StatusServer::Online,
                "status dipertahankan Online sebelum ambang 3 kegagalan"
            );
        } else {
            assert_eq!(
                server.status,
                StatusServer::Unreachable,
                "status harus Unreachable PERSIS di kegagalan ketiga"
            );
        }

        let row = sqlx::query!("SELECT next_poll_at FROM servers WHERE id = ?", id)
            .fetch_one(&state.db_read)
            .await
            .expect("baca next_poll_at");
        assert!(
            row.next_poll_at > next_poll_sebelumnya,
            "next_poll_at harus terus bertambah (backoff melambat), bukan diam/mengecil"
        );
        next_poll_sebelumnya = row.next_poll_at;
    }

    let _ = std::fs::remove_dir_all(&dir);
}
