//! Rakit `Router`: router luar (publik) dan router terlindungi (butuh sesi).
//!
//! api-contract.md: SEMUA halaman kecuali `/healthz`, `GET /login`,
//! `POST /login`, dan `GET /assets/*` wajib masuk router terlindungi.

pub mod apps;
pub mod assets;
pub mod dashboard;
pub mod deploy_api;
pub mod deployments;
pub mod events;
pub mod fleet;
pub mod health;
pub mod login;
pub mod logs;
pub mod reconciliation;
pub mod registries;
pub mod rollback;
pub mod servers;
pub mod settings;

use axum::Router;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::IntoResponse;
use axum::routing::{get, post};

use crate::auth::middleware::require_session;
use crate::state::AppState;
use crate::web;

/// Bangun router lengkap aplikasi.
pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/", get(dashboard::dashboard))
        .route("/logout", post(login::logout_submit))
        .route(
            "/servers",
            get(servers::fleet).post(servers::server_baru_submit),
        )
        .route("/servers/baru", get(servers::server_baru_form))
        .route("/servers/{id}", get(servers::detail))
        .route("/servers/{id}/hapus", post(servers::hapus_submit))
        .route("/fleet", get(fleet::halaman))
        .route("/fleet/command", post(fleet::command_submit))
        .route("/fleet/prune", post(fleet::prune_submit))
        .route("/fleet/exec/{server_id}", post(fleet::exec_submit))
        .route("/fleet/operations/{id}", get(fleet::operation_detail))
        .route(
            "/fleet/operations/{id}/output/{server_id}",
            get(fleet::operation_output),
        )
        .route("/events/fleet/{id}", get(fleet::operation_stream))
        .route("/servers/{id}/verifikasi", get(servers::verifikasi_halaman))
        .route(
            "/servers/{id}/verifikasi/ulang",
            post(servers::verifikasi_ulang),
        )
        .route(
            "/servers/{id}/hostkey/konfirmasi",
            post(servers::konfirmasi_hostkey),
        )
        .route(
            "/servers/{id}/registry",
            get(registries::registry_form).post(registries::registry_submit),
        )
        .route("/events/verifikasi/{id}", get(events::verifikasi_stream))
        .route("/apps", get(apps::daftar).post(apps::app_baru_submit))
        .route("/apps/baru", get(apps::app_baru_form))
        .route("/apps/{id}", get(apps::detail))
        .route("/apps/{id}/domain", post(apps::domain_submit))
        .route("/apps/{id}/token", post(apps::token_submit))
        .route("/apps/{id}/deployments", get(apps::tab_deployments))
        .route("/apps/{id}/workflow/{jenis}", get(apps::workflow_unduh))
        .route("/apps/{id}/logs", get(apps::tab_logs))
        .route("/apps/{id}/logs/isi", get(logs::app_log_isi))
        .route(
            "/apps/{id}/env",
            get(apps::tab_environment).post(apps::env_submit),
        )
        .route("/apps/{id}/reconciliation", get(reconciliation::daftar))
        .route(
            "/apps/{app_id}/reconciliation/{finding_id}/acknowledge",
            post(reconciliation::acknowledge),
        )
        .route(
            "/settings/notifications",
            get(settings::page).post(settings::save),
        )
        .route("/deployments/{id}", get(deployments::detail))
        .route("/deployments/{id}/rollback", post(rollback::submit))
        .route("/deployments/{id}/log", get(logs::deploy_log_halaman))
        .route("/deployments/{id}/log/isi", get(logs::deploy_log_isi))
        .route("/deployments/{id}/log/unduh", get(logs::deploy_log_unduh))
        .route("/events/deploy/{id}", get(events::deploy_stream))
        .route("/events/log/deploy/{id}", get(events::log_deploy_stream))
        .route("/events/log/runtime/{id}", get(events::log_runtime_stream))
        .route_layer(from_fn_with_state(state.clone(), require_session));

    let public = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/login", get(login::login_form).post(login::login_submit))
        .route("/assets/htmx.min.js", get(assets::htmx_js))
        .route("/assets/htmx-sse.min.js", get(assets::htmx_sse_js));

    // Router bearer token TERPISAH dari sesi cookie — `docs/plan.md`
    // kontrak `POST /api/v1/deploy`. Tidak ada `require_session`, tidak ada
    // CSRF (CI bukan browser); autentikasi dilakukan DI DALAM handler
    // sendiri karena butuh body request (`app`) untuk tahu token app mana
    // yang harus dicocokkan.
    let api = Router::new().route("/api/v1/deploy", post(deploy_api::deploy));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(api)
        .fallback(fallback_404)
        .with_state(state)
}

async fn fallback_404() -> axum::response::Response {
    (StatusCode::NOT_FOUND, web::render_404(None)).into_response()
}
