//! `GET/POST /servers`, `GET /servers/baru`, `GET /servers/{id}/verifikasi`,
//! `POST /servers/{id}/verifikasi/ulang`, `POST /servers/{id}/hostkey/konfirmasi`,
//! `GET /servers/{id}` — `docs/api-contract.md` §"Fase 1".
//!
//! Handler tidak berisi HTML atau logika verifikasi/kriptografi — itu
//! `src/web/**` dan `src/servers/verify.rs`. Handler hanya orkestrasi +
//! validasi input + mapping ke response.

use axum::Form;
use axum::extract::{Extension, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::metrics_repo;

use crate::auth::session::Session;
use crate::error::AppError;
use crate::events::VerificationEvent;
use crate::servers::model::{LangkahStatus, LangkahVerifikasi, StatusServer};
use crate::servers::verify::{self, NAMA_DOCKER, NAMA_KONEKSI, NAMA_REGISTRY};
use crate::servers::{NewServer, repo};
use crate::state::AppState;
use crate::web;

const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

/// `GET /servers` — fleet overview.
pub async fn fleet(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let servers = repo::list_ringkas(&state.db_read)
        .await
        .map_err(AppError::from)?;
    let strip = web::render_fleet_strip(&servers);

    Ok(web::render_fleet(&servers, &session.csrf_token, Some(strip)).into_response())
}

/// `GET /servers/baru` — wizard langkah 1.
pub async fn server_baru_form(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let strip = fleet_strip(&state).await?;
    Ok(web::render_server_baru(&session.csrf_token, None, Some(strip)).into_response())
}

#[derive(Deserialize)]
pub struct ServerBaruForm {
    csrf_token: String,
    name: String,
    host: String,
    port: Option<i64>,
    ssh_user: String,
    ssh_key: String,
}

/// `POST /servers` — validasi, enkripsi kunci, simpan `pending`, spawn
/// verifikasi, redirect ke checklist.
pub async fn server_baru_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<ServerBaruForm>,
) -> Result<Response, AppError> {
    let strip = fleet_strip(&state).await?;

    if form.csrf_token != session.csrf_token {
        let body = web::render_server_baru(
            &session.csrf_token,
            Some(PESAN_CSRF_TIDAK_VALID),
            Some(strip),
        );
        return Ok((axum::http::StatusCode::BAD_REQUEST, body).into_response());
    }

    if let Err(pesan) = validasi_server_baru(&form) {
        let body = web::render_server_baru(&session.csrf_token, Some(pesan), Some(strip));
        return Ok((axum::http::StatusCode::BAD_REQUEST, body).into_response());
    }

    // `.trim()` saja akan membuang newline penutup `-----END OPENSSH PRIVATE
    // KEY-----` — OpenSSH menolak file kunci tanpa newline akhir (`invalid
    // format`), sehingga key yang valid pun ditolak server. Normalisasi
    // memakai `trim` untuk whitespace tepi lalu memastikan newline akhir ada.
    let ssh_key = normalisasi_ssh_key(&form.ssh_key);
    let ssh_key_encrypted = state.crypto.encrypt(&ssh_key).map_err(AppError::from)?;

    let id = repo::insert_pending(
        &state.db_write,
        NewServer {
            name: form.name.trim(),
            host: form.host.trim(),
            port: form.port.unwrap_or(22),
            ssh_user: form.ssh_user.trim(),
            ssh_key_encrypted: &ssh_key_encrypted,
        },
    )
    .await
    .map_err(AppError::from)?;

    // Channel dibuat SEBELUM job di-spawn supaya SSE yang menyambung
    // sesaat setelah redirect tidak kehilangan channel-nya (`events.rs`).
    let _ = state.events.subscribe(&id);
    tokio::spawn(verify::mulai_verifikasi(state, id.clone()));

    Ok(Redirect::to(&format!("/servers/{id}/verifikasi")).into_response())
}

fn validasi_server_baru(form: &ServerBaruForm) -> Result<(), &'static str> {
    if form.name.trim().is_empty() {
        return Err("Nama server wajib diisi.");
    }
    validasi_host(&form.host)?;
    if let Some(port) = form.port
        && !(1..=65535).contains(&port)
    {
        return Err(
            "Port harus berupa angka bulat dalam rentang 1 - 65535. Langkah perbaikan: Ganti \
             dengan port SSH server target yang benar.",
        );
    }
    if form.ssh_user.trim().is_empty() {
        return Err("Pengguna SSH wajib diisi.");
    }
    if !form.ssh_key.trim().starts_with("-----BEGIN") {
        return Err(
            "Format kunci privat tidak valid. Langkah perbaikan: Pastikan Anda menyalin \
             seluruh teks kunci termasuk baris pembuka dan penutup OpenSSH.",
        );
    }
    Ok(())
}

/// Buang whitespace tepi dari kunci privat SSH tanpa menghilangkan newline
/// penutup wajib (`\n` setelah `-----END OPENSSH PRIVATE KEY-----`).
fn normalisasi_ssh_key(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.ends_with('\n') {
        trimmed.to_string()
    } else {
        format!("{trimmed}\n")
    }
}

fn validasi_host(host: &str) -> Result<(), &'static str> {
    let host = host.trim();
    if host.is_empty() {
        return Err("Alamat host wajib diisi.");
    }
    if host.contains("://") || host.contains(':') {
        return Err(
            "Host tidak boleh mengandung skema URL (http://) atau gabungan port (:22). \
             Masukkan alamat IP atau nama domain saja. Langkah perbaikan: Hapus 'http://' \
             atau ':port' dari input Host.",
        );
    }
    Ok(())
}

/// `GET /servers/{id}/verifikasi` — checklist. TIDAK memulai job baru
/// (`docs/api-contract.md`).
pub async fn verifikasi_halaman(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let server = repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    let strip = fleet_strip(&state).await?;
    let event = checklist_awal(&server);

    Ok(web::render_verifikasi(&server, &event, &session.csrf_token, Some(strip)).into_response())
}

/// Snapshot checklist dari status db — dipakai render pertama sebelum SSE
/// membawa event lebih mutakhir (`docs/api-contract.md`: "Status terakhir
/// yang sudah diketahui dirender langsung supaya halaman tetap benar kalau
/// SSE gagal tersambung").
pub(super) fn checklist_awal(server: &crate::servers::model::ServerRingkas) -> VerificationEvent {
    let (status_koneksi, status_docker, pesan_koneksi) = match server.status {
        StatusServer::Pending => (LangkahStatus::Menunggu, LangkahStatus::Menunggu, None),
        StatusServer::Verifying => (LangkahStatus::Berjalan, LangkahStatus::Menunggu, None),
        StatusServer::Online => (LangkahStatus::Sukses, LangkahStatus::Sukses, None),
        StatusServer::Unreachable => (
            LangkahStatus::Gagal,
            LangkahStatus::Menunggu,
            server.last_error_message.clone(),
        ),
    };

    VerificationEvent {
        langkah: vec![
            LangkahVerifikasi {
                nama: NAMA_KONEKSI.to_string(),
                status: status_koneksi,
                pesan: pesan_koneksi,
            },
            LangkahVerifikasi {
                nama: NAMA_DOCKER.to_string(),
                status: status_docker,
                pesan: None,
            },
            LangkahVerifikasi {
                nama: NAMA_REGISTRY.to_string(),
                status: LangkahStatus::Menunggu,
                pesan: None,
            },
        ],
        tofu_pending_fingerprint: None,
    }
}

#[derive(Deserialize)]
pub struct VerifikasiUlangForm {
    csrf_token: String,
}

/// `POST /servers/{id}/verifikasi/ulang`.
pub async fn verifikasi_ulang(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<VerifikasiUlangForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }

    let row = repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    if row.status == StatusServer::Verifying {
        return Err(AppError::Conflict(
            "Verifikasi sedang berjalan. Silakan tunggu hingga proses selesai.".to_string(),
        ));
    }

    let _ = state.events.subscribe(&id);
    tokio::spawn(verify::mulai_verifikasi(state, id.clone()));

    Ok(Redirect::to(&format!("/servers/{id}/verifikasi")).into_response())
}

#[derive(Deserialize)]
pub struct KonfirmasiHostkeyForm {
    csrf_token: String,
    fingerprint: String,
}

/// `POST /servers/{id}/hostkey/konfirmasi`.
pub async fn konfirmasi_hostkey(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<KonfirmasiHostkeyForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }

    match verify::konfirmasi_hostkey_dan_lanjutkan(state, id.clone(), &form.fingerprint).await {
        Ok(()) => Ok(Redirect::to(&format!("/servers/{id}/verifikasi")).into_response()),
        Err(verify::KonfirmasiHostkeyError::ServerTidakDitemukan) => Err(not_found()),
        Err(verify::KonfirmasiHostkeyError::FingerprintTidakCocok) => Err(AppError::BadRequest(
            "Sidik jari tidak lagi cocok dengan yang ditawarkan server — verifikasi ulang \
             diperlukan."
                .to_string(),
        )),
        Err(verify::KonfirmasiHostkeyError::FingerprintSudahTersimpanBerbeda) => {
            Err(AppError::Conflict(
                "Server ini sudah punya sidik jari host key tersimpan yang berbeda. Tambahkan \
                 ulang sebagai server baru kalau Anda sengaja mengganti host."
                    .to_string(),
            ))
        }
        Err(verify::KonfirmasiHostkeyError::Lain(err)) => Err(AppError::from(anyhow::anyhow!(err))),
    }
}

/// `POST /servers/{id}/hapus` — hapus server beserta seluruh data terkait
/// (app, deployment, log, metrik, tautan registry) dalam satu transaksi,
/// lalu kembali ke fleet. Tindakan destruktif dijalankan lewat tombol dengan
/// konfirmasi di halaman detail — tidak ada endpoint GET yang menghapus apa
/// pun.
#[derive(Deserialize)]
pub struct HapusServerForm {
    csrf_token: String,
}

pub async fn hapus_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<HapusServerForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }

    // Pastikan server benar-benar ada — id tak dikenal → 404, bukan sukses
    // diam-diam.
    repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    repo::hapus(&state.db_write, &id)
        .await
        .map_err(AppError::from)?;

    state.events.remove(&id);

    Ok(Redirect::to("/servers").into_response())
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    /// Rentang dalam jam; dibatasi agar handler tidak membaca histori tanpa batas.
    range: Option<u32>,
}

pub async fn detail(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Query(query): Query<MetricsQuery>,
) -> Result<Response, AppError> {
    let server = repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    let registries_tertaut = crate::registries::repo::list_linked(&state.db_read, &id)
        .await
        .map_err(AppError::from)?;
    let strip = fleet_strip(&state).await?;
    let range_hours = query.range.unwrap_or(6).clamp(1, 168);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let metrics =
        metrics_repo::dashboard(&state.db_read, &id, now - i64::from(range_hours) * 60 * 60)
            .await
            .map_err(AppError::from)?;

    Ok(web::render_server_detail(
        &server,
        &registries_tertaut,
        &metrics,
        range_hours,
        &session.csrf_token,
        Some(strip),
    )
    .into_response())
}

pub(super) async fn fleet_strip(state: &AppState) -> Result<maud::Markup, AppError> {
    let servers = repo::list_ringkas(&state.db_read)
        .await
        .map_err(AppError::from)?;
    Ok(web::render_fleet_strip(&servers))
}

pub(super) fn not_found() -> AppError {
    AppError::NotFound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisasi_mempertahankan_newline_akhir_saat_ada() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nisi\n-----END OPENSSH PRIVATE KEY-----\n";
        assert_eq!(normalisasi_ssh_key(key), key);
    }

    #[test]
    fn normalisasi_menambahkan_newline_akhir_saat_tidak_ada() {
        let tanpa_newline =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nisi\n-----END OPENSSH PRIVATE KEY-----";
        let hasil = normalisasi_ssh_key(tanpa_newline);
        assert!(hasil.ends_with('\n'), "key wajib diakhiri newline");
        assert_eq!(hasil.trim_end(), tanpa_newline);
    }

    #[test]
    fn normalisasi_membuang_whitespace_tepi() {
        let key =
            "  \n-----BEGIN OPENSSH PRIVATE KEY-----\nisi\n-----END OPENSSH PRIVATE KEY-----\n  ";
        let hasil = normalisasi_ssh_key(key);
        assert!(hasil.starts_with("-----BEGIN"));
        assert!(hasil.ends_with('\n'));
        assert!(!hasil.starts_with(' ') && !hasil.starts_with('\n'));
    }
}
