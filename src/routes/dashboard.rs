//! `GET /` — terlindungi, render shell dashboard + ringkasan armada
//! (`docs/api-contract.md` "GET / (perubahan, bukan endpoint baru)").

use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};

use crate::auth::session::Session;
use crate::error::AppError;
use crate::servers::repo;
use crate::state::AppState;
use crate::web;

/// `Session` diisi ke request extensions oleh middleware `require_session`.
pub async fn dashboard(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let servers = repo::list_ringkas(&state.db_read)
        .await
        .map_err(AppError::from)?;
    let strip = web::render_fleet_strip(&servers);

    Ok(web::render_dashboard(Some(strip), &session.csrf_token, servers.len()).into_response())
}
