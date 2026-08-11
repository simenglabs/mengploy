use axum::Form;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::auth::session::Session;
use crate::deployments::reconciliation;
use crate::error::AppError;
use crate::state::AppState;
use crate::web;

use super::servers::{fleet_strip, not_found};

const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

#[derive(Deserialize)]
pub struct AcknowledgeForm {
    csrf_token: String,
}

pub async fn daftar(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(app_id): Path<String>,
) -> Result<Response, AppError> {
    let app = crate::apps::repo::find_by_id(&state.db_read, &app_id)
        .await?
        .ok_or_else(not_found)?;
    let findings = reconciliation::list_active(&state.db_read, &app_id).await?;
    let strip = fleet_strip(&state).await?;
    Ok(
        web::render_reconciliation(&app, &findings, None, &session.csrf_token, Some(strip))
            .into_response(),
    )
}

pub async fn acknowledge(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path((app_id, finding_id)): Path<(String, String)>,
    Form(form): Form<AcknowledgeForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }
    crate::apps::repo::find_by_id(&state.db_read, &app_id)
        .await?
        .ok_or_else(not_found)?;
    if !reconciliation::acknowledge(&state.db_write, &finding_id, &app_id).await? {
        return Err(AppError::NotFound);
    }
    let findings = reconciliation::list_active(&state.db_read, &app_id).await?;
    let app = crate::apps::repo::find_by_id(&state.db_read, &app_id)
        .await?
        .ok_or_else(not_found)?;
    let strip = fleet_strip(&state).await?;
    Ok(web::render_reconciliation(
        &app,
        &findings,
        Some("Finding diakui. Sistem tidak mengubah container target."),
        &session.csrf_token,
        Some(strip),
    )
    .into_response())
}
