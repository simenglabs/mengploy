//! Endpoint Fase 7: operasi armada dan pintu darurat.
//! Semua endpoint berada di router session-protected; POST memakai CSRF.

use axum::Form;
use axum::extract::{Extension, Path as AxumPath, State};
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::convert::Infallible;
use std::path::Path as FsPath;

use tokio::io::AsyncReadExt;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::session::Session;
use crate::error::AppError;
use crate::fleet::{self, FleetEvent, FleetOperationKind};
use crate::fleet_repo;
use crate::state::AppState;
use crate::web;

use super::servers::fleet_strip;

const CSRF_ERROR: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

#[derive(Debug, Deserialize)]
pub struct FleetForm {
    csrf_token: String,
    #[serde(default)]
    server_id: Vec<String>,
    command: Option<String>,
    confirm: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExecForm {
    csrf_token: String,
    container_id: String,
    command: String,
    confirm: Option<String>,
}

pub async fn halaman(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let servers = crate::servers::repo::list_ringkas(&state.db_read).await?;
    let disks = fleet_repo::list_disk(&state.db_read).await?;
    let operations = fleet_repo::list_operations(&state.db_read).await?;
    let strip = fleet_strip(&state).await?;
    Ok(web::render_fleet_actions(
        &servers,
        &disks,
        &operations,
        &session.csrf_token,
        None,
        &[],
        Some(strip),
    )
    .into_response())
}

pub async fn command_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<FleetForm>,
) -> Result<Response, AppError> {
    validasi_csrf(&form.csrf_token, &session)?;
    let command = fleet::validate_command(form.command.as_deref().unwrap_or(""))
        .map_err(|err| AppError::BadRequest(err.to_string()))?;
    if form.confirm.as_deref() != Some("jalankan") {
        return Err(AppError::BadRequest(
            "Konfirmasi eksplisit diperlukan sebelum perintah dijalankan.".to_string(),
        ));
    }
    let targets = fleet::validate_targets(&form.server_id)
        .map_err(|err| AppError::BadRequest(err.to_string()))?;
    let operation_id = fleet_repo::generate_id();
    let targets_json = serde_json::to_string(&targets).map_err(internal)?;
    let payload_plain =
        serde_json::to_string(&serde_json::json!({ "command": command })).map_err(internal)?;
    let payload = state
        .crypto
        .encrypt(&payload_plain)
        .map_err(AppError::Internal)?;
    fleet_repo::insert_operation(
        &state.db_write,
        &operation_id,
        FleetOperationKind::Command,
        &targets_json,
        &payload,
    )
    .await?;
    state.fleet_events.subscribe(&operation_id);
    crate::worker::fleet::spawn(state.clone(), operation_id.clone());
    Ok(redirect_operation(&operation_id))
}

pub async fn prune_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<FleetForm>,
) -> Result<Response, AppError> {
    validasi_csrf(&form.csrf_token, &session)?;
    if form.confirm.as_deref() != Some("prune") {
        return Err(AppError::BadRequest(
            "Konfirmasi eksplisit diperlukan sebelum image dibersihkan.".to_string(),
        ));
    }
    let targets = fleet::validate_targets(&form.server_id)
        .map_err(|err| AppError::BadRequest(err.to_string()))?;
    let operation_id = fleet_repo::generate_id();
    let targets_json = serde_json::to_string(&targets).map_err(internal)?;
    let payload = state.crypto.encrypt("{}").map_err(AppError::Internal)?;
    fleet_repo::insert_operation(
        &state.db_write,
        &operation_id,
        FleetOperationKind::Prune,
        &targets_json,
        &payload,
    )
    .await?;
    state.fleet_events.subscribe(&operation_id);
    crate::worker::fleet::spawn(state.clone(), operation_id.clone());
    Ok(redirect_operation(&operation_id))
}

pub async fn exec_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    AxumPath(server_id): AxumPath<String>,
    Form(form): Form<ExecForm>,
) -> Result<Response, AppError> {
    validasi_csrf(&form.csrf_token, &session)?;
    if form.confirm.as_deref() != Some("exec") {
        return Err(AppError::BadRequest(
            "Konfirmasi eksplisit diperlukan sebelum exec container.".to_string(),
        ));
    }
    let container_id = form.container_id.trim();
    let command = fleet::validate_exec_command(&form.command)
        .map_err(|err| AppError::BadRequest(err.to_string()))?;
    let output =
        crate::worker::fleet::exec_container_once(&state, &server_id, container_id, &command)
            .await
            .map_err(|err| {
                tracing::warn!(server_id, error = %err, "exec container gagal");
                AppError::Timeout("Exec container gagal atau server tidak merespons.".to_string())
            })?;
    let strip = fleet_strip(&state).await?;
    let servers = crate::servers::repo::list_ringkas(&state.db_read).await?;
    let disks = fleet_repo::list_disk(&state.db_read).await?;
    let operations = fleet_repo::list_operations(&state.db_read).await?;
    Ok(web::render_fleet_actions(
        &servers,
        &disks,
        &operations,
        &session.csrf_token,
        Some((server_id, container_id.to_string(), output)),
        &[],
        Some(strip),
    )
    .into_response())
}

pub async fn operation_detail(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    let operation = fleet_repo::find_operation(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;
    let results = fleet_repo::list_results(&state.db_read, &id).await?;
    let servers = crate::servers::repo::list_ringkas(&state.db_read).await?;
    let disks = fleet_repo::list_disk(&state.db_read).await?;
    let operations = vec![operation];
    let strip = fleet_strip(&state).await?;
    Ok(web::render_fleet_actions(
        &servers,
        &disks,
        &operations,
        &session.csrf_token,
        None,
        &results,
        Some(strip),
    )
    .into_response())
}

pub async fn operation_output(
    State(state): State<AppState>,
    Extension(_session): Extension<Session>,
    AxumPath((operation_id, server_id)): AxumPath<(String, String)>,
) -> Result<Response, AppError> {
    let operation = fleet_repo::find_operation(&state.db_read, &operation_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let result = fleet_repo::list_results(&state.db_read, &operation_id)
        .await?
        .into_iter()
        .find(|result| result.server_id == server_id)
        .ok_or(AppError::NotFound)?;
    let Some(path) = result.output_path else {
        return Err(AppError::NotFound);
    };
    let base = state.config.log_dir.join("operations").join(&operation.id);
    if !crate::fleet::output_path_is_safe(&path, &base) {
        return Err(AppError::NotFound);
    }
    let body = baca_output_aman(FsPath::new(&path), &base)
        .await
        .map_err(|err| {
            tracing::warn!(operation_id, server_id, error = %err, "gagal membaca output operasi");
            AppError::NotFound
        })?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/plain; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"output-{server_id}.txt\""),
            ),
        ],
        body,
    )
        .into_response())
}

pub async fn operation_stream(
    State(state): State<AppState>,
    Extension(_session): Extension<Session>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    let operation = fleet_repo::find_operation(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;
    if matches!(
        operation.status.as_str(),
        "succeeded" | "partial" | "failed"
    ) {
        let event = Event::default().data(operation.status);
        return Ok(Sse::new(tokio_stream::once(Ok::<Event, Infallible>(event)))
            .keep_alive(KeepAlive::default())
            .into_response());
    }
    let rx = state.fleet_events.subscribe(&id);
    let stream = BroadcastStream::new(rx).filter_map(move |item| {
        let event: FleetEvent = item.ok()?;
        Some(Ok::<Event, Infallible>(
            Event::default().data(
                serde_json::json!({
                    "status": event.status,
                    "server_id": event.server_id,
                    "message": event.message,
                })
                .to_string(),
            ),
        ))
    });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// Baca output hanya setelah parent dan target di-canonicalize, lalu buka
/// komponen file terakhir dengan `O_NOFOLLOW`. Canonicalize menolak symlink
/// yang sudah ada di jalur; `O_NOFOLLOW` menutup symlink yang dibuat dalam
/// race sebelum `open`. Ini tetap bukan sandbox kernel penuh untuk seluruh
/// pohon (openat berantai diperlukan untuk itu), tetapi cukup untuk file
/// privat yang dibuat worker di bawah direktori operasi.
#[cfg(target_os = "macos")]
const O_NOFOLLOW_FLAG: i32 = 0x0100;
#[cfg(any(target_os = "linux", target_os = "android"))]
const O_NOFOLLOW_FLAG: i32 = 0x20000;

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "linux", target_os = "android"))
))]
compile_error!("platform Unix ini belum memiliki konstanta O_NOFOLLOW yang diverifikasi");

#[cfg(unix)]
async fn baca_output_aman(path: &FsPath, base: &FsPath) -> std::io::Result<Vec<u8>> {
    let canonical_base = tokio::fs::canonicalize(base).await?;
    let canonical_path = tokio::fs::canonicalize(path).await?;
    if !canonical_path.starts_with(&canonical_base) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "output berada di luar direktori operasi",
        ));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        // Nilai ABI berbeda antara macOS dan Linux; keduanya mencegah
        // symlink sebagai komponen terakhir tanpa dependensi libc tambahan.
        .custom_flags(O_NOFOLLOW_FLAG)
        .open(path)
        .await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "output bukan file biasa",
        ));
    }
    let mut body = Vec::new();
    file.read_to_end(&mut body).await?;
    Ok(body)
}

#[cfg(not(unix))]
async fn baca_output_aman(path: &FsPath, base: &FsPath) -> std::io::Result<Vec<u8>> {
    let canonical_base = tokio::fs::canonicalize(base).await?;
    let canonical_path = tokio::fs::canonicalize(path).await?;
    if !canonical_path.starts_with(&canonical_base) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "output berada di luar direktori operasi",
        ));
    }
    tokio::fs::read(path).await
}

fn redirect_operation(id: &str) -> Response {
    axum::response::Redirect::to(&format!("/fleet/operations/{id}")).into_response()
}

fn validasi_csrf(value: &str, session: &Session) -> Result<(), AppError> {
    if value != session.csrf_token {
        return Err(AppError::BadRequest(CSRF_ERROR.to_string()));
    }
    Ok(())
}

fn internal(err: serde_json::Error) -> AppError {
    AppError::Internal(err.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn output_symlink_ke_luar_root_tidak_pernah_dibaca() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mengdep-fleet-output-{}-{id}", std::process::id()));
        let base = root.join("operations").join("op");
        tokio::fs::create_dir_all(&base)
            .await
            .expect("direktori output test harus dibuat");
        let target = root.join("secret.txt");
        tokio::fs::write(&target, b"rahasia di luar root")
            .await
            .expect("target symlink test harus ditulis");
        let link = base.join("server.out");
        symlink(&target, &link).expect("symlink test harus dibuat");
        assert!(baca_output_aman(&link, &base).await.is_err());

        let inside_target = base.join("target.out");
        tokio::fs::write(&inside_target, b"target biasa")
            .await
            .expect("target dalam root harus ditulis");
        let inside_link = base.join("alias.out");
        symlink(&inside_target, &inside_link).expect("symlink dalam root harus dibuat");
        assert!(
            baca_output_aman(&inside_link, &base).await.is_err(),
            "O_NOFOLLOW harus menolak symlink final walau target berada di root"
        );
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn output_file_biasa_dibaca_dengan_benar() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mengdep-fleet-output-{}-{id}", std::process::id()));
        let base = root.join("operations").join("op");
        tokio::fs::create_dir_all(&base)
            .await
            .expect("direktori output test harus dibuat");
        let path = base.join("server.out");
        tokio::fs::write(&path, b"output aman")
            .await
            .expect("output test harus ditulis");

        assert_eq!(
            baca_output_aman(&path, &base).await.expect("output aman"),
            b"output aman"
        );
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
