//! `GET /deployments/{id}` — detail + timeline SSE, `docs/plan.md` Fase 2.

use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};

use crate::apps::repo as apps_repo;
use crate::auth::session::Session;
use crate::deployments::repo as deployments_repo;
use crate::error::AppError;
use crate::state::AppState;
use crate::web;

use super::servers::fleet_strip;

pub async fn detail(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let dep = deployments_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;
    let app = apps_repo::find_by_id(&state.db_read, &dep.app_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let strip = fleet_strip(&state).await?;

    Ok(
        web::render_deployment_detail(&dep, &app.name, &session.csrf_token, Some(strip))
            .into_response(),
    )
}
