//! Integration test end-to-end Fase 0 (fondasi): Axum + SQLite dua pool,
//! login pengguna tunggal Argon2, sesi cookie, graceful shutdown.
//!
//! Pendekatan: router diuji lewat `axum::ServiceExt::oneshot` (tanpa server
//! TCP nyata) untuk skenario HTTP murni; skenario SIGTERM memakai binary
//! `mengdep` sebagai proses anak dengan port ephemeral + file db sementara.
//!
//! Setiap test memakai direktori sementara unik di `temp_dir()` supaya tidak
//! berebut file db dan bisa jalan paralel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::util::ServiceExt;

use mengdep::auth::middleware::SESSION_COOKIE_NAME;
use mengdep::auth::password::hash_password;
use mengdep::auth::session;
use mengdep::config::Config;
use mengdep::crypto::CryptoKey;
use mengdep::db;
use mengdep::routes::build_router;
use mengdep::state::AppState;

/// Penghitung global supaya direktori temp unik per test, tidak berebut
/// meskipun test jalan paralel.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Nama cookie draft CSRF yang dipakai `GET /login` (privat di src; literal
/// di sini sengaja — kalau backend mengganti nama, test harus merah).
const CSRF_DRAFT_COOKIE_NAME: &str = "mengdep_csrf_draft";

/// Buat direktori sementara unik per test.
fn unique_dir(nama: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("mengdep-qa-{}-{}-{}", std::process::id(), n, nama))
}

/// Tulis file kunci `age` sementara bermode `0600` ke `dir`, kembalikan
/// path-nya. Pola sama dengan `buat_file_kunci_sementara()` di `src/crypto.rs`
/// — kunci enkripsi kini WAJIB untuk `AppState`/`main()` (Fase 1), jadi test
/// harus menyediakan kunci valid supaya `CryptoKey::load_from_file` dan
/// startup binary tidak gagal fatal. Kunci ditaruh di dalam direktori temp
/// test yang sama, jadi ikut terhapus oleh `remove_dir_all` pembersih test.
fn tulis_kunci_age_ke(dir: &std::path::Path) -> PathBuf {
    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;

    let identity = age::x25519::Identity::generate();
    let path = dir.join("key.txt");
    // Direktori temp belum tentu ada (mis. dipanggil sebelum db di-migrasi);
    // bikin dulu supaya `std::fs::write` tidak gagal "No such file".
    std::fs::create_dir_all(dir).expect("bikin direktori temp test harus sukses");
    std::fs::write(&path, identity.to_string().expose_secret())
        .expect("tulis file kunci sementara harus sukses");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("set mode file kunci sementara harus sukses");
    path
}

/// Siapkan AppState dengan db baru di direktori temp unik.
async fn setup(nama: &str) -> (AppState, PathBuf) {
    let dir = unique_dir(nama);
    let db_path = dir.join("test.db");
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi db baru harus sukses");

    // Kunci enkripsi age kini wajib (Fase 1) — tulis file sementara 0600 ke
    // direktori temp test (ikut terhapus oleh pembersih di bawah) dan muat
    // lewat jalur produksi yang sama dengan `main.rs`.
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

/// Seed `settings.password_hash` lewat hash Argon2 — menggantikan peran
/// `seed_initial_password` di `main.rs` yang privat.
async fn seed_password(state: &AppState, password: &str) {
    let hash = hash_password(password).expect("hash password untuk seed");
    sqlx::query("INSERT INTO settings (key, value) VALUES ('password_hash', ?)")
        .bind(hash)
        .execute(&state.db_write)
        .await
        .expect("simpan password_hash ke settings");
}

/// Kirim request ke router dan kembalikan (status, headers, body teks).
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

/// Bangun request GET tanpa cookie.
fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

/// Bangun request GET dengan header Cookie.
fn get_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

/// Bangun request POST form-urlencoded. `cookie` kosong berarti tanpa header
/// Cookie sama sekali.
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

/// Encode form-urlencoded sederhana (spasi jadi `+`, sisanya `%XX`).
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

/// Ambil nilai cookie dari header `Set-Cookie` (bisa lebih dari satu).
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

/// Ambil nilai hidden input `csrf_token` dari HTML.
fn parse_hidden_csrf(html: &str) -> Option<String> {
    let marker = r#"name="csrf_token" value=""#;
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Ambil isi elemen `role="alert"` (pesan error pada halaman login).
fn ambil_pesan_alert(body: &str) -> String {
    let marker = r#"role="alert">"#;
    let start = body
        .find(marker)
        .unwrap_or_else(|| panic!("tidak ada elemen alert di body: {body}"))
        + marker.len();
    let rest = &body[start..];
    let end = rest
        .find("</div>")
        .unwrap_or_else(|| panic!("elemen alert tidak ditutup"));
    rest[..end].to_string()
}

/// Ambil halaman login: kembalikan (cookie draft siap-pasang di header
/// `Cookie:` dalam format `nama=nilai`, token csrf dari form).
async fn ambil_login(app: &axum::Router) -> (String, String) {
    let (status, headers, body) = send(app, get("/login")).await;
    assert_eq!(status, StatusCode::OK, "GET /login harus 200");
    let draft = ambil_cookie_dari_set_cookie(&headers, CSRF_DRAFT_COOKIE_NAME)
        .expect("GET /login harus men-set cookie draft csrf");
    // Cookie harus dikirim balik dalam format `nama=nilai`; header `Cookie:`
    // dengan nilai telanjang membuat `jar.get(CSRF_DRAFT_COOKIE_NAME)` di
    // src/routes/login.rs gagal menemukan draft (return None) sehingga validasi
    // CSRF selalu 400 sebelum verifikasi password.
    let draft_header = format!("{CSRF_DRAFT_COOKIE_NAME}={draft}");
    let token = parse_hidden_csrf(&body).expect("form login harus menanam csrf_token");
    (draft_header, token)
}

/// Login dengan password benar: kembalikan (cookie sesi, csrf dari dashboard).
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
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, "/", "redirect login harus ke /");
    let session_cookie = ambil_cookie_dari_set_cookie(&headers, SESSION_COOKIE_NAME)
        .expect("response login harus men-set cookie sesi");

    let (status2, _, body2) = send(
        app,
        get_with_cookie("/", &format!("{SESSION_COOKIE_NAME}={session_cookie}")),
    )
    .await;
    assert_eq!(status2, StatusCode::OK, "dashboard harus 200 setelah login");
    let csrf =
        parse_hidden_csrf(&body2).expect("dashboard harus menanam csrf_token di form logout");
    (session_cookie, csrf)
}

// ============================================================
// Enam skenario wajib
// ============================================================

/// Wajib 1 — login dengan password salah ditolak 401, pesan generik,
/// tidak membocorkan hash Argon2 / detail library / path filesystem.
#[tokio::test]
async fn login_password_salah_ditolak_dengan_pesan_generik() {
    let (state, dir) = setup("wajib-login-salah").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi-benar-123").await;

    let (draft, token) = ambil_login(&app).await;
    let (status, _headers, body) = send(
        &app,
        post_form(
            "/login",
            &draft,
            &[("password", "kata-sandi-salah"), ("csrf_token", &token)],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED, "password salah harus 401");
    assert!(
        body.contains("Kata sandi salah. Silakan coba lagi."),
        "harus menampilkan pesan generik Bahasa Indonesia"
    );
    assert!(
        !body.contains("kata-sandi-benar-123"),
        "password asli tidak boleh bocor"
    );
    assert!(
        !body.contains("$argon2") && !body.contains("argon2"),
        "hash argon2 tidak boleh bocor ke response"
    );
    assert!(
        !body.contains(dir.to_str().unwrap()),
        "path filesystem tidak boleh bocor"
    );
    assert!(
        !body.contains("/private/") && !body.contains("/Users/"),
        "path absolut tidak boleh bocor"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Wajib 1b — "user tidak ada" (belum di-seed) menghasilkan pesan yang sama
/// persis dengan "password salah", supaya tidak bisa dibedakan (anti enumerasi).
#[tokio::test]
async fn login_user_tidak_ada_pesan_sama_dengan_password_salah() {
    // Tanpa seed sama sekali — seolah user tidak ada.
    let (state_tanpa_user, dir1) = setup("wajib-user-tidak-ada").await;
    let app_tanpa_user = build_router(state_tanpa_user.clone());

    let (draft1, token1) = ambil_login(&app_tanpa_user).await;
    let (status1, _, body1) = send(
        &app_tanpa_user,
        post_form(
            "/login",
            &draft1,
            &[("password", "apa-saja-123"), ("csrf_token", &token1)],
        ),
    )
    .await;
    assert_eq!(status1, StatusCode::UNAUTHORIZED);

    // User ada tapi password salah.
    let (state_ada_user, dir2) = setup("wajib-user-ada").await;
    let app_ada_user = build_router(state_ada_user.clone());
    seed_password(&state_ada_user, "benar-456").await;

    let (draft2, token2) = ambil_login(&app_ada_user).await;
    let (status2, _, body2) = send(
        &app_ada_user,
        post_form(
            "/login",
            &draft2,
            &[("password", "salah-789"), ("csrf_token", &token2)],
        ),
    )
    .await;
    assert_eq!(status2, StatusCode::UNAUTHORIZED);

    let pesan1 = ambil_pesan_alert(&body1);
    let pesan2 = ambil_pesan_alert(&body2);
    assert_eq!(
        pesan1, pesan2,
        "pesan 'user tidak ada' dan 'password salah' harus identik"
    );

    let _ = std::fs::remove_dir_all(&dir1);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// Wajib 2 — sesi dengan `expires_at` di masa lalu ditolak seolah tidak ada:
/// `GET /` → 303 ke `/login`, bukan 200 dan bukan 500.
#[tokio::test]
async fn sesi_kedaluwarsa_ditolak_seolah_tidak_ada() {
    let (state, dir) = setup("wajib-sesi-kedaluwarsa").await;
    let app = build_router(state.clone());

    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    sqlx::query(
        "INSERT INTO sessions (id, created_at, expires_at, csrf_token) VALUES (?, ?, ?, ?)",
    )
    .bind("sesi-lama-kedaluwarsa")
    .bind(now - 200)
    .bind(now - 100)
    .bind("csrf-lama-123")
    .execute(&state.db_write)
    .await
    .expect("sisipkan sesi kedaluwarsa");

    let (status, headers, body) = send(
        &app,
        get_with_cookie("/", &format!("{SESSION_COOKIE_NAME}=sesi-lama-kedaluwarsa")),
    )
    .await;

    assert_eq!(status, StatusCode::SEE_OTHER, "sesi kedaluwarsa harus 303");
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, "/login", "redirect harus ke /login");
    assert!(
        body.is_empty(),
        "redirect tidak boleh membawa body halaman dashboard"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Wajib 3 — db baru dibuat dari nol, migrasi idempoten (dua kali), izin 0600.
#[tokio::test]
async fn db_baru_terbuat_migrasi_idempoten_dan_bermode_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = unique_dir("wajib-db-baru");
    let db_path = dir.join("test.db");
    assert!(!db_path.exists(), "db belum boleh ada sebelum migrasi");

    let pools1 = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi pertama harus sukses");

    // Tabel wajib ada setelah migrasi pertama.
    let ada_sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
    )
    .fetch_one(&pools1.read)
    .await
    .unwrap();
    let ada_settings: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='settings'",
    )
    .fetch_one(&pools1.read)
    .await
    .unwrap();
    assert_eq!(ada_sessions, 1, "tabel sessions harus ada");
    assert_eq!(ada_settings, 1, "tabel settings harus ada");
    pools1.write.close().await;
    pools1.read.close().await;

    // Migrasi kedua pada db yang sama tidak boleh error (idempoten).
    let pools2 = db::connect_and_migrate(&db_path)
        .await
        .expect("migrasi kedua (idempoten) harus sukses");
    pools2.write.close().await;
    pools2.read.close().await;

    // Izin file db harus 0600.
    let mode = std::fs::metadata(&db_path)
        .expect("metadata db harus terbaca")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "file db harus bermode 0600, dapat {mode:o}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Wajib 4 — SIGTERM ke proses anak: graceful shutdown, db tetap utuh dan
/// bisa dibuka ulang, log tidak membocorkan password/hash.
#[cfg(unix)]
#[tokio::test]
async fn sigterm_mematikan_server_dengan_bersih_dan_db_tetap_utuh() {
    use std::io::Read as _;

    let dir = unique_dir("wajib-sigterm");
    let db_path = dir.join("test.db");
    let key_path = tulis_kunci_age_ke(&dir);
    let port = cari_port_kosong();
    let addr = format!("127.0.0.1:{port}");

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_mengdep"))
        .env("MENGDEP_DB_PATH", &db_path)
        .env("MENGDEP_LISTEN_ADDR", &addr)
        .env("MENGDEP_KEY_PATH", &key_path)
        .env("MENGDEP_RUNTIME_DIR", dir.join("runtime"))
        .env("MENGDEP_LOG_DIR", dir.join("logs"))
        .env("MENGDEP_INITIAL_PASSWORD", "kata-sandi-awal-sigterm")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("binary mengdep harus bisa di-spawn");
    let pid = child.id();

    // Tunggu server sehat (healthz merespons "ok").
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if cek_healthz(&addr).await {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "server tidak pernah sehat setelah 15 detik"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Kirim SIGTERM.
    let kill_status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("kirim SIGTERM ke proses anak");
    assert!(kill_status.success(), "perintah kill harus sukses");

    // Tunggu proses anak keluar (graceful shutdown punya waktu 15 detik).
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "proses anak tidak mau mati setelah SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_string(&mut stdout).unwrap();
    }
    if let Some(mut err) = child.stderr.take() {
        err.read_to_string(&mut stderr).unwrap();
    }

    // Invariant 7: secret tidak boleh bocor ke log mana pun.
    assert!(
        !stdout.contains("kata-sandi-awal-sigterm"),
        "password awal bocor ke stdout log"
    );
    assert!(
        !stderr.contains("kata-sandi-awal-sigterm"),
        "password awal bocor ke stderr log"
    );
    assert!(
        !stdout.contains("$argon2"),
        "hash argon2 bocor ke stdout log"
    );
    assert!(
        !stderr.contains("$argon2"),
        "hash argon2 bocor ke stderr log"
    );

    // Log shutdown bersih (bukti graceful shutdown jalan, bukan kill paksa).
    assert!(
        stdout.contains("server berhenti dengan bersih"),
        "log graceful shutdown tidak muncul; stdout={stdout:?}"
    );

    // Db harus utuh: bisa dibuka ulang, password_hash ter-seed, bisa dibaca.
    let pools = db::connect_and_migrate(&db_path)
        .await
        .expect("db harus bisa dibuka ulang setelah SIGTERM");
    let hash: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'password_hash'")
            .fetch_optional(&pools.read)
            .await
            .unwrap();
    let hash = hash.expect("password_hash harus ter-seed saat startup");
    assert!(
        hash.starts_with("$argon2"),
        "hash tersimpan harus format PHC argon2"
    );
    pools.write.close().await;
    pools.read.close().await;

    let _ = std::fs::remove_dir_all(&dir);
}

/// Cari port TCP kosong dengan bind sementara lalu lepas.
fn cari_port_kosong() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port kosong");
    let port = listener.local_addr().expect("alamat lokal").port();
    drop(listener);
    port
}

/// Cek `GET /healthz` via TcpStream mentah; true kalau respons "200 ... ok".
async fn cek_healthz(addr: &str) -> bool {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let Ok(mut stream) = tokio::net::TcpStream::connect(addr).await else {
        return false;
    };
    let request = b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(request).await.is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if stream.read_to_end(&mut buf).await.is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&buf);
    text.contains("200") && text.trim_end().ends_with("ok")
}

/// Wajib 5 — logout dengan CSRF valid: sesi terhapus dari tabel, cookie
/// dibersihkan, redirect 303 ke /login, dan cookie lama tidak berlaku lagi.
#[tokio::test]
async fn logout_dengan_csrf_valid_menghapus_sesi_dan_cookie() {
    let (state, dir) = setup("wajib-logout-valid").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi-logout").await;

    let (session_cookie, csrf) = login(&app, "kata-sandi-logout").await;

    let count_sebelum: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db_read)
        .await
        .unwrap();
    assert_eq!(count_sebelum, 1, "harus ada satu sesi sebelum logout");

    let (status, headers, _body) = send(
        &app,
        post_form(
            "/logout",
            &format!("{SESSION_COOKIE_NAME}={session_cookie}"),
            &[("csrf_token", &csrf)],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::SEE_OTHER, "logout valid harus 303");
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location, "/login", "redirect logout harus ke /login");

    // Cookie sesi dihapus (Max-Age=0 / kedaluwarsa).
    let cookie_clear = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .find(|v| v.to_str().unwrap().starts_with(SESSION_COOKIE_NAME))
        .expect("respons logout harus men-set cookie sesi untuk dihapus")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cookie_clear.contains("Max-Age=0") || cookie_clear.to_lowercase().contains("expires"),
        "cookie sesi harus dihapus, dapat: {cookie_clear}"
    );

    // Baris sesi harus hilang dari tabel.
    let count_sesudah: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db_read)
        .await
        .unwrap();
    assert_eq!(count_sesudah, 0, "sesi harus terhapus dari tabel sessions");

    // Cookie lama tidak berlaku lagi → redirect ke /login.
    let (status2, headers2, _) = send(
        &app,
        get_with_cookie("/", &format!("{SESSION_COOKIE_NAME}={session_cookie}")),
    )
    .await;
    assert_eq!(status2, StatusCode::SEE_OTHER);
    let location2 = headers2
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(location2, "/login", "cookie lama harus tidak valid");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Wajib 6 — logout dengan CSRF salah: 400, dan sesi TIDAK dihapus.
#[tokio::test]
async fn logout_csrf_salah_ditolak_dan_sesi_tetap_ada() {
    let (state, dir) = setup("wajib-logout-csrf-salah").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi-logout").await;

    let (session_cookie, _csrf) = login(&app, "kata-sandi-logout").await;

    let (status, _headers, body) = send(
        &app,
        post_form(
            "/logout",
            &format!("{SESSION_COOKIE_NAME}={session_cookie}"),
            &[("csrf_token", "token-csrf-salah-1234567890abcdef")],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "CSRF salah harus 400");
    assert!(
        body.contains("Sesi tidak valid atau kedaluwarsa"),
        "harus ada pesan generik CSRF tidak valid"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db_read)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "sesi harus TETAP ada setelah logout ditolak karena CSRF salah"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Wajib 6b — logout dengan field csrf_token hilang total: 400, sesi tetap ada.
#[tokio::test]
async fn logout_csrf_hilang_ditolak_dan_sesi_tetap_ada() {
    let (state, dir) = setup("wajib-logout-csrf-hilang").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi-logout").await;

    let (session_cookie, _csrf) = login(&app, "kata-sandi-logout").await;

    // Form tanpa field csrf_token sama sekali.
    let (status, _headers, _body) = send(
        &app,
        post_form(
            "/logout",
            &format!("{SESSION_COOKIE_NAME}={session_cookie}"),
            &[],
        ),
    )
    .await;

    // LogoutForm.csrf_token wajib (src/routes/login.rs:36-38); field hilang
    // → axum rejection 4xx (default 422), bukan 400 dan bukan 500. Yang
    // penting: sesi TIDAK terhapus.
    assert!(
        status.is_client_error(),
        "csrf hilang harus 4xx, dapat {status}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR, "tidak boleh 500");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db_read)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "sesi harus TETAP ada setelah logout ditolak karena CSRF hilang"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// Skenario tambahan — serang batas
// ============================================================

/// Login tanpa field csrf_token sama sekali: harus 4xx, bukan panic/500.
/// Konfirmasi temuan: body rejection axum masih berbahasa Inggris mentah.
#[tokio::test]
async fn login_tanpa_field_csrf_ditolak_bukan_panic() {
    let (state, dir) = setup("tambah-login-no-csrf").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (draft, _token) = ambil_login(&app).await;
    let (status, _headers, body) = send(
        &app,
        post_form("/login", &draft, &[("password", "kata-sandi")]),
    )
    .await;

    assert!(
        status.is_client_error(),
        "form tanpa csrf_token harus 4xx, dapat {status}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR, "tidak boleh 500");
    // Temuan: axum rejection memakai teks Inggris mentah, bukan Bahasa Indonesia.
    assert!(
        body.contains("Failed to deserialize form"),
        "temuan: body rejection axum berbahasa Inggris — harusnya Bahasa Indonesia"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Login dengan token csrf yang tidak cocok dengan cookie draft: 400.
#[tokio::test]
async fn login_csrf_tidak_cocok_cookie_draft_ditolak() {
    let (state, dir) = setup("tambah-login-csrf-mismatch").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (draft, _token) = ambil_login(&app).await;
    let (status, _headers, body) = send(
        &app,
        post_form(
            "/login",
            &draft,
            &[
                ("password", "kata-sandi"),
                ("csrf_token", "token-yang-berbeda-1234567890"),
            ],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("Sesi tidak valid atau kedaluwarsa"),
        "harus pesan generik CSRF"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Login membawa token csrf tapi cookie draft hilang sama sekali: 400.
#[tokio::test]
async fn login_cookie_draft_hilang_tapi_token_dibawa_ditolak() {
    let (state, dir) = setup("tambah-login-no-draft-cookie").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    // Cookie kosong → header Cookie tidak dikirim.
    let (status, _headers, body) = send(
        &app,
        post_form(
            "/login",
            "",
            &[
                ("password", "kata-sandi"),
                ("csrf_token", "token-asal-dari-form-1234567890"),
            ],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("Sesi tidak valid atau kedaluwarsa"),
        "harus pesan generik CSRF"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Cookie sesi sampah / kosong / sangat panjang: semua harus redirect /login,
/// tidak pernah 500 atau panic.
#[tokio::test]
async fn cookie_sesi_sampah_ditolak_dengan_redirect_login() {
    let (state, dir) = setup("tambah-cookie-sampah").await;
    let app = build_router(state.clone());

    // String sampah alfanumerik.
    let (status, headers, _) = send(
        &app,
        get_with_cookie(
            "/",
            &format!("{SESSION_COOKIE_NAME}=abcdefghijklmnopqrstuvwxyz123456"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
        "/login"
    );

    // Nilai kosong.
    let (status2, _, _) = send(
        &app,
        get_with_cookie("/", &format!("{SESSION_COOKIE_NAME}=")),
    )
    .await;
    assert_eq!(status2, StatusCode::SEE_OTHER);

    // Sangat panjang (10 ribu karakter).
    let panjang = "x".repeat(10_000);
    let (status3, _, _) = send(
        &app,
        get_with_cookie("/", &format!("{SESSION_COOKIE_NAME}={panjang}")),
    )
    .await;
    assert_eq!(status3, StatusCode::SEE_OTHER);

    // Cookie bernama beda — dianggap tidak login.
    let (status4, _, _) = send(&app, get_with_cookie("/", "cookie_lain=abc123")).await;
    assert_eq!(status4, StatusCode::SEE_OTHER);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Dua login berturut-turut: sesi pertama harus mati (rotasi penuh), tabel
/// sessions hanya berisi satu baris.
#[tokio::test]
async fn login_kedua_merotasi_dan_mematikan_sesi_pertama() {
    let (state, dir) = setup("tambah-rotasi").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (sesi1, _) = login(&app, "kata-sandi").await;
    let (sesi2, _) = login(&app, "kata-sandi").await;
    assert_ne!(sesi1, sesi2, "dua login harus menghasilkan token berbeda");

    let ditemukan = session::find_valid_session(&state.db_read, &sesi1)
        .await
        .expect("baca sesi pertama");
    assert!(
        ditemukan.is_none(),
        "sesi pertama harus mati setelah login kedua"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db_read)
        .await
        .unwrap();
    assert_eq!(count, 1, "tabel sessions hanya boleh berisi sesi terbaru");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `GET /healthz` publik: 200, body "ok", tidak ada cookie, tidak ada detail
/// library / path / config.
#[tokio::test]
async fn healthz_publik_ok_tanpa_bocor_detail() {
    let (state, dir) = setup("tambah-healthz").await;
    let app = build_router(state);

    let (status, headers, body) = send(&app, get("/healthz")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "ok", "body harus 'ok', bukan HTML shell");
    assert!(
        !headers.contains_key(header::SET_COOKIE),
        "healthz tidak boleh men-set cookie"
    );
    assert!(
        !body.contains("axum") && !body.contains("/") && !body.contains("config"),
        "healthz tidak boleh membocorkan detail"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Path tidak dikenal: 404 lewat error_page, teks Bahasa Indonesia dari desain,
/// tanpa detail internal.
#[tokio::test]
async fn path_tidak_dikenal_404_dengan_halaman_error_bahasa_indonesia() {
    let (state, dir) = setup("tambah-404").await;
    let app = build_router(state);

    let (status, _headers, body) = send(&app, get("/tidak-ada-123")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body.contains("Halaman tidak ditemukan"),
        "harus teks 404 Bahasa Indonesia dari docs/design"
    );
    assert!(
        body.contains("tidak dikenal atau telah dipindahkan"),
        "harus kalimat lengkap desain 404"
    );
    assert!(
        !body.contains(".rs:") && !body.contains("panic") && !body.contains("Failed to"),
        "404 tidak boleh membocorkan detail internal"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Dua POST /logout bersamaan dengan token valid sama: tidak boleh 500,
/// dan sesi akhirnya benar-benar terhapus.
#[tokio::test]
async fn dua_logout_bersamaan_token_sama_tidak_mengakibatkan_500() {
    let (state, dir) = setup("tambah-logout-race").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (session_cookie, csrf) = login(&app, "kata-sandi").await;
    let cookie = format!("{SESSION_COOKIE_NAME}={session_cookie}");

    let req1 = post_form("/logout", &cookie, &[("csrf_token", &csrf)]);
    let req2 = post_form("/logout", &cookie, &[("csrf_token", &csrf)]);
    let f1 = send(&app, req1);
    let f2 = send(&app, req2);
    let ((s1, _, _), (s2, _, _)) = tokio::join!(f1, f2);

    assert_ne!(s1, StatusCode::INTERNAL_SERVER_ERROR, "logout pertama 500");
    assert_ne!(s2, StatusCode::INTERNAL_SERVER_ERROR, "logout kedua 500");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db_read)
        .await
        .unwrap();
    assert_eq!(count, 0, "sesi harus terhapus setelah logout");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant 7: id sesi tidak boleh muncul di HTML mana pun — dashboard
/// maupun halaman login. Yang boleh ditanam di form hanya csrf_token.
#[tokio::test]
async fn id_sesi_tidak_pernah_muncul_di_html() {
    let (state, dir) = setup("tambah-id-sesi-bocor").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (session_cookie, _csrf) = login(&app, "kata-sandi").await;

    let (_status, _headers, body_dashboard) = send(
        &app,
        get_with_cookie("/", &format!("{SESSION_COOKIE_NAME}={session_cookie}")),
    )
    .await;
    assert!(
        !body_dashboard.contains(&session_cookie),
        "id sesi bocor ke body dashboard"
    );
    assert!(
        !body_dashboard.contains(SESSION_COOKIE_NAME),
        "nama cookie sesi bocor ke body dashboard"
    );

    let (_status2, _headers2, body_login) = send(&app, get("/login")).await;
    assert!(
        !body_login.contains(SESSION_COOKIE_NAME),
        "nama cookie sesi bocor ke body halaman login"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `Set-Cookie` sesi harus membawa HttpOnly, Secure, SameSite=Lax, Path=/.
#[tokio::test]
async fn set_cookie_sesi_membawa_flag_keamanan_wajib() {
    let (state, dir) = setup("tambah-set-cookie").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (draft, token) = ambil_login(&app).await;
    let (status, headers, _body) = send(
        &app,
        post_form(
            "/login",
            &draft,
            &[("password", "kata-sandi"), ("csrf_token", &token)],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let set_cookie = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .find(|v| v.to_str().unwrap().starts_with(SESSION_COOKIE_NAME))
        .expect("cookie sesi harus di-set")
        .to_str()
        .unwrap()
        .to_string();

    assert!(
        set_cookie.contains("HttpOnly"),
        "harus HttpOnly: {set_cookie}"
    );
    assert!(set_cookie.contains("Secure"), "harus Secure: {set_cookie}");
    assert!(
        set_cookie.contains("SameSite=Lax"),
        "harus SameSite=Lax: {set_cookie}"
    );
    assert!(set_cookie.contains("Path=/"), "harus Path=/: {set_cookie}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Password kosong: ditolak 401 dengan pesan generik, bukan panic.
#[tokio::test]
async fn login_password_kosong_ditolak_generik() {
    let (state, dir) = setup("tambah-password-kosong").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (draft, token) = ambil_login(&app).await;
    let (status, _headers, body) = send(
        &app,
        post_form(
            "/login",
            &draft,
            &[("password", ""), ("csrf_token", &token)],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.contains("Kata sandi salah. Silakan coba lagi."),
        "password kosong harus pesan generik"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Password sangat panjang (200 ribu karakter): ditolak 401 generik, bukan
/// 500 / panic / body limit error.
#[tokio::test]
async fn login_password_sangat_panjang_ditolak_generik() {
    let (state, dir) = setup("tambah-password-panjang").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (draft, token) = ambil_login(&app).await;
    let panjang = "a".repeat(200_000);
    let (status, _headers, body) = send(
        &app,
        post_form(
            "/login",
            &draft,
            &[("password", &panjang), ("csrf_token", &token)],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.contains("Kata sandi salah. Silakan coba lagi."),
        "password panjang harus pesan generik"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Password berisi byte non-UTF8: harus 4xx (rejection), bukan 500/panic.
#[tokio::test]
async fn login_password_byte_non_utf8_ditolak_tanpa_panic() {
    let (state, dir) = setup("tambah-password-non-utf8").await;
    let app = build_router(state.clone());
    seed_password(&state, "kata-sandi").await;

    let (draft, token) = ambil_login(&app).await;

    // Body form berisi byte non-UTF8 pada nilai password.
    let mut body_bytes = b"password=".to_vec();
    body_bytes.extend_from_slice(&[0xFF, 0xFE, 0x80]);
    body_bytes.extend_from_slice(format!("&csrf_token={}", urlencode(&token)).as_bytes());

    let request = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, &draft)
        .body(Body::from(body_bytes))
        .unwrap();

    let (status, _headers, _body) = send(&app, request).await;

    assert!(
        status.is_client_error(),
        "byte non-UTF8 harus ditolak 4xx, dapat {status}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR, "tidak boleh 500");

    let _ = std::fs::remove_dir_all(&dir);
}
