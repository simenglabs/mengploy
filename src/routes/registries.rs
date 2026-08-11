//! `GET/POST /servers/{id}/registry` — wizard langkah 3 (opsional),
//! `docs/api-contract.md`.

use axum::Form;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth::session::Session;
use crate::error::AppError;
use crate::registries::repo::{self as registries_repo, RegistryRingkas};
use crate::servers::model::ServerRingkas;
use crate::servers::repo;
use crate::servers::verify::{self, RegistryStepInput};
use crate::state::AppState;
use crate::web;

use super::servers::{fleet_strip, not_found};

const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

/// `GET /servers/{id}/registry`.
pub async fn registry_form(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let server = repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;
    let registries = registries_repo::list_all(&state.db_read)
        .await
        .map_err(AppError::from)?;
    let strip = fleet_strip(&state).await?;

    Ok(
        web::render_registry_form(&server, &registries, &session.csrf_token, None, Some(strip))
            .into_response(),
    )
}

#[derive(Deserialize)]
pub struct RegistryForm {
    csrf_token: String,
    #[serde(default)]
    registry_id: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    token: String,
}

/// `POST /servers/{id}/registry`.
pub async fn registry_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<RegistryForm>,
) -> Result<Response, AppError> {
    let server = repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }

    let registry_id = form.registry_id.trim().to_string();
    let host = form.host.trim().to_string();
    let username = form.username.trim().to_string();

    if registry_id.is_empty() && (host.is_empty() || username.is_empty() || form.token.is_empty()) {
        return render_ulang(
            &state,
            &server,
            &session.csrf_token,
            StatusCode::BAD_REQUEST,
            "Host, username, dan token wajib diisi untuk mendaftarkan registry baru.",
        )
        .await;
    }

    let input = if registry_id.is_empty() {
        RegistryStepInput::Baru {
            registry_host: &host,
            username: &username,
            password: &form.token,
        }
    } else {
        RegistryStepInput::PakaiUlang {
            registry_id: &registry_id,
        }
    };

    match verify::tautkan_registry(&state, &id, input).await {
        Ok(()) => Ok(Redirect::to(&format!("/servers/{id}")).into_response()),
        Err(verify::RegistryStepError::ServerTidakDitemukan)
        | Err(verify::RegistryStepError::RegistryTidakDitemukan) => Err(not_found()),
        Err(verify::RegistryStepError::KoneksiGagal) => {
            render_ulang(
                &state,
                &server,
                &session.csrf_token,
                StatusCode::BAD_REQUEST,
                "Server belum terverifikasi atau tidak bisa dihubungi. Selesaikan verifikasi \
                 koneksi lebih dulu sebelum menautkan registry.",
            )
            .await
        }
        Err(verify::RegistryStepError::Ditolak { .. }) => {
            render_ulang(
                &state,
                &server,
                &session.csrf_token,
                StatusCode::UNPROCESSABLE_ENTITY,
                "Kredensial ditolak oleh host registry. Username atau token yang Anda masukkan \
                 salah. Langkah perbaikan: Periksa kembali token akses Anda di registry dan \
                 pastikan izin read/write sudah benar.",
            )
            .await
        }
        Err(verify::RegistryStepError::Timeout) => {
            render_ulang(
                &state,
                &server,
                &session.csrf_token,
                StatusCode::GATEWAY_TIMEOUT,
                "Batas waktu koneksi ke registry terlampaui. Langkah perbaikan: Periksa apakah \
                 server target dapat mengakses internet luar atau apakah registry sedang \
                 mengalami gangguan.",
            )
            .await
        }
        Err(verify::RegistryStepError::Lain(err)) => Err(AppError::from(anyhow::anyhow!(err))),
    }
}

async fn render_ulang(
    state: &AppState,
    server: &ServerRingkas,
    csrf_token: &str,
    status: StatusCode,
    pesan: &str,
) -> Result<Response, AppError> {
    let registries: Vec<RegistryRingkas> = registries_repo::list_all(&state.db_read)
        .await
        .map_err(AppError::from)?;
    let strip = fleet_strip(state).await?;
    let body = web::render_registry_form(server, &registries, csrf_token, Some(pesan), Some(strip));
    Ok((status, body).into_response())
}
