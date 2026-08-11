//! Middleware yang menolak request tanpa sesi valid dan mengalihkan ke
//! `/login` (api-contract.md: semua halaman kecuali `/healthz`, `GET /login`,
//! `POST /login` wajib masuk router terlindungi).

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;

use crate::auth::session;
use crate::state::AppState;

/// Nama cookie yang menyimpan token sesi. Dipakai bersama oleh middleware dan
/// handler login/logout — satu sumber kebenaran supaya tidak ada typo nama
/// cookie yang membuat sesi "hilang" secara diam-diam.
pub const SESSION_COOKIE_NAME: &str = "mengdep_session";

/// Middleware proteksi route. Kalau tidak ada cookie sesi, atau sesi tidak
/// ditemukan/kedaluwarsa di db, request ditolak dengan redirect `303` ke
/// `/login` — bukan 401, supaya pengalaman browser langsung mengarahkan
/// pengguna (dashboard.rs di api-contract.md juga memakai pola ini).
pub async fn require_session(
    State(state): State<AppState>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Response {
    let Some(cookie) = jar.get(SESSION_COOKIE_NAME) else {
        return Redirect::to("/login").into_response();
    };

    let found = session::find_valid_session(&state.db_read, cookie.value()).await;

    match found {
        Ok(Some(session)) => {
            let mut request = request;
            request.extensions_mut().insert(session);
            next.run(request).await
        }
        Ok(None) => Redirect::to("/login").into_response(),
        Err(err) => {
            tracing::warn!(error = ?err, "gagal memvalidasi sesi, perlakukan sebagai tidak login");
            Redirect::to("/login").into_response()
        }
    }
}
