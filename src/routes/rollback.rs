use axum::Form;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::apps::repo as apps_repo;
use crate::auth::session::Session;
use crate::deployments::{
    DeployJobPayload, LOCK_TTL_SECS, NewDeployment, repo as deployments_repo,
};
use crate::error::AppError;
use crate::state::AppState;

use super::servers::not_found;

const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

#[derive(Deserialize)]
pub struct RollbackForm {
    csrf_token: String,
}

pub async fn submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(deployment_id): Path<String>,
    Form(form): Form<RollbackForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }

    let target = deployments_repo::find_by_id(&state.db_read, &deployment_id)
        .await?
        .ok_or_else(not_found)?;
    if target.status == crate::deployments::StatusDeployment::Queued
        || target.status == crate::deployments::StatusDeployment::Pulling
        || target.status == crate::deployments::StatusDeployment::Starting
        || target.status == crate::deployments::StatusDeployment::Checking
    {
        return Err(AppError::Conflict(
            "deployment target masih berjalan dan belum dapat di-rollback".to_string(),
        ));
    }

    let app = apps_repo::find_by_id(&state.db_read, &target.app_id)
        .await?
        .ok_or_else(not_found)?;
    if let Some(env_version_id) = target.env_version_id.as_deref()
        && !apps_repo::env_version_belongs_to_app(&state.db_read, env_version_id, &app.id).await?
    {
        return Err(AppError::Conflict(
            "versi environment deployment target tidak tersedia".to_string(),
        ));
    }

    let rollback_id = deployments_repo::generate_id();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    if !apps_repo::acquire_lock(&state.db_write, &app.id, &rollback_id, now + LOCK_TTL_SECS).await?
    {
        return Err(AppError::Conflict(
            "app sedang dalam proses deploy lain".to_string(),
        ));
    }

    let job_id = deployments_repo::generate_id();
    let payload = serde_json::to_string(&DeployJobPayload {
        deployment_id: rollback_id.clone(),
    })
    .map_err(|err| AppError::Internal(err.into()))?;
    let result = deployments_repo::insert_queued_dengan_job(
        &state.db_write,
        &rollback_id,
        NewDeployment {
            app_id: &app.id,
            commit_sha: &target.commit_sha,
            git_ref: target.git_ref.as_deref(),
            image_digest: &target.image_digest,
            trigger_source: "rollback",
            env_version_id: target.env_version_id.as_deref(),
        },
        &job_id,
        &payload,
    )
    .await;
    if let Err(err) = result {
        let _ = apps_repo::release_lock(&state.db_write, &app.id, &rollback_id).await;
        return Err(AppError::from(err));
    }

    Ok(Redirect::to(&format!("/deployments/{rollback_id}")).into_response())
}
