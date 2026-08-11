//! `GET/POST /login`, `POST /logout`.
//!
//! Handler tidak berisi HTML atau logika domain — verifikasi password dan
//! pembuatan/penghapusan sesi ada di `src/auth/`. Handler hanya orkestrasi +
//! mapping ke response, sesuai `docs/plan.md` (batas modul).

use axum::Form;
use axum::extract::State;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::Deserialize;
use time::Duration as TimeDuration;

use crate::auth::middleware::SESSION_COOKIE_NAME;
use crate::auth::{password, session};
use crate::error::AppError;
use crate::state::AppState;
use crate::web;

/// Pesan generik untuk kredensial salah — tidak boleh membedakan "user tidak
/// ada" dari "password salah" (api-contract.md).
const PESAN_KREDENSIAL_SALAH: &str = "Kata sandi salah. Silakan coba lagi.";

/// Pesan generik untuk CSRF token hilang/tidak cocok.
const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

#[derive(Deserialize)]
pub struct LoginForm {
    password: String,
    csrf_token: String,
}

#[derive(Deserialize)]
pub struct LogoutForm {
    csrf_token: String,
}

/// `GET /login` — render form dengan token CSRF baru yang ditanam di sesi
/// draft (dibuat sekarang, dipakai untuk validasi saat `POST /login`).
///
/// ponytail: token CSRF di Fase 0 dibuat bersamaan dengan sesi asli saat
/// login sukses (lihat `login_submit`), bukan sesi terpisah "draft" sebelum
/// login — pendekatan paling sederhana untuk pengguna tunggal tanpa
/// menambah tabel/state baru. Form GET menanam token CSRF acak sekali-pakai
/// yang divalidasi ulang di POST via perbandingan string sederhana yang
/// disimpan di cookie sementara, supaya tidak butuh baris db untuk request
/// yang belum tentu berlanjut jadi login.
pub async fn login_form(jar: CookieJar) -> (CookieJar, Response) {
    let csrf_token = generate_csrf_draft_token();
    let jar = jar.add(csrf_cookie(csrf_token.clone()));
    let body = web::render_login(None, &csrf_token).into_response();
    (jar, body)
}

/// `POST /login` — validasi CSRF, verifikasi password, buat sesi baru kalau
/// cocok.
pub async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let expected_csrf = jar
        .get(CSRF_DRAFT_COOKIE_NAME)
        .map(|c| c.value().to_string());

    if expected_csrf.as_deref() != Some(form.csrf_token.as_str()) {
        let body = web::render_login(Some(PESAN_CSRF_TIDAK_VALID), &form.csrf_token);
        return Ok((axum::http::StatusCode::BAD_REQUEST, body).into_response());
    }

    let stored_hash = sqlx::query!("SELECT value FROM settings WHERE key = 'password_hash'")
        .fetch_optional(&state.db_read)
        .await
        .map_err(|err| anyhow::anyhow!(err))
        .map_err(AppError::from)?
        .map(|row| row.value);

    let password_ok = match &stored_hash {
        Some(hash) => password::verify_password(&form.password, hash).unwrap_or(false),
        None => false,
    };

    if !password_ok {
        let new_csrf = generate_csrf_draft_token();
        let jar = jar.add(csrf_cookie(new_csrf.clone()));
        let body = web::render_login(Some(PESAN_KREDENSIAL_SALAH), &new_csrf);
        return Ok((axum::http::StatusCode::UNAUTHORIZED, jar, body).into_response());
    }

    let new_session = session::create_session(&state.db_write)
        .await
        .map_err(AppError::from)?;

    let jar = jar
        .remove(Cookie::from(CSRF_DRAFT_COOKIE_NAME))
        .add(session_cookie(new_session.id));

    Ok((jar, Redirect::to("/")).into_response())
}

/// `POST /logout` — validasi CSRF terhadap sesi aktif, hapus sesi, clear
/// cookie, redirect ke `/login`.
pub async fn logout_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LogoutForm>,
) -> Result<Response, AppError> {
    let Some(cookie) = jar.get(SESSION_COOKIE_NAME) else {
        // Tidak ada sesi valid — idempoten aman, redirect saja.
        return Ok(Redirect::to("/login").into_response());
    };

    let session_id = cookie.value().to_string();

    let current = session::find_valid_session(&state.db_read, &session_id)
        .await
        .map_err(AppError::from)?;

    let Some(current) = current else {
        return Ok(Redirect::to("/login").into_response());
    };

    if current.csrf_token != form.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }

    session::delete_session(&state.db_write, &session_id)
        .await
        .map_err(AppError::from)?;

    let jar = jar.remove(Cookie::from(SESSION_COOKIE_NAME));

    Ok((jar, Redirect::to("/login")).into_response())
}

/// Nama cookie sementara yang menyimpan token CSRF sebelum sesi asli ada.
const CSRF_DRAFT_COOKIE_NAME: &str = "mengdep_csrf_draft";

fn generate_csrf_draft_token() -> String {
    use rand::RngExt;
    use rand::distr::Alphanumeric;

    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn csrf_cookie(value: String) -> Cookie<'static> {
    Cookie::build((CSRF_DRAFT_COOKIE_NAME, value))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(TimeDuration::minutes(15))
        .build()
}

fn session_cookie(session_id: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE_NAME, session_id))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(TimeDuration::days(30))
        .build()
}
