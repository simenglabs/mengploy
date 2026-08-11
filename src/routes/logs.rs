//! Endpoint log NON-SSE Fase 3 (`docs/api-contract.md` Fase 3):
//! `GET /deployments/{id}/log`, `/log/isi`, `/log/unduh`, dan
//! `GET /apps/{id}/logs/isi`. Dua endpoint SSE log (`/events/log/deploy/{id}`
//! dan `/events/log/runtime/{id}`) TIDAK di sini — itu `routes::events`.
//!
//! **Anti path traversal** (`docs/plan.md`): path file log HANYA dibentuk
//! lewat `logs::reader::nama_file_aman` + `logs::writer::path_log`. Tidak ada
//! `PathBuf::from(<input>)` di file ini, dan tidak boleh pernah ada.
//!
//! **Path file tidak pernah sampai ke klien** — tidak di HTML, tidak di pesan
//! error, tidak di header. Klien hanya pernah melihat `deployment_id`.
use axum::extract::{Extension, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::apps::repo as apps_repo;
use crate::auth::session::Session;
use crate::deployments::repo as deployments_repo;
use crate::docker;
use crate::error::AppError;
use crate::logs::reader::{self, LogLine, LogReadError};
use crate::logs::{repo as logs_repo, writer as logs_writer};
use crate::ssh::{self, HostKeyMode};
use crate::state::AppState;
use crate::web as render;

use super::servers::fleet_strip;

/// Teks kategori final dari `docs/design/log-viewer.md` §8. Konstanta, bukan
/// literal di tempat pakai, supaya satu pesan tidak pernah punya dua versi
/// yang berbeda antara fragmen HTMX dan status HTTP.
pub(super) const PESAN_BELUM_ADA_CONTAINER: &str = "[i] Belum ada container aktif untuk aplikasi ini. Langkah perbaikan: Silakan lakukan deployment pertama Anda untuk melihat log runtime container di sini.";
pub(super) const PESAN_CONTAINER_HILANG: &str = "[x] Container tidak ditemukan di server target. Log runtime tidak dapat ditampilkan lagi. Langkah perbaikan: Silakan periksa tab Riwayat Deployment untuk melihat log deploy terakhir, atau pastikan container dalam keadaan berjalan.";
pub(super) const PESAN_TIMEOUT_KONEKSI: &str = "[x] Batas waktu koneksi ke server target terlampaui saat mencoba menarik log. Langkah perbaikan: Pastikan server target dalam keadaan aktif, jaringan stabil, dan Docker Engine berjalan dengan normal.";
pub(super) const PESAN_TERLALU_BANYAK_SESI: &str = "[x] Terlalu banyak sesi log runtime aktif terbuka. Aplikasi membatasi maksimal 4 sesi streaming runtime secara bersamaan untuk menghemat memori. Langkah perbaikan: Tutup salah satu tab browser yang sedang memutar log runtime, lalu coba lagi.";
pub(super) const PESAN_PENCARIAN_TIMEOUT: &str = "[x] Pencarian terlalu lama. Kata kunci yang Anda masukkan menghasilkan pencarian yang lambat. Langkah perbaikan: Silakan masukkan kata kunci yang lebih spesifik.";
pub(super) const PESAN_SESI_30_MENIT: &str = "--- [i] SESI LOG SELESAI: Aliran log otomatis dihentikan setelah 30 menit demi menghemat bandwidth. Langkah perbaikan: Silakan muat ulang halaman ini untuk memulai sesi streaming baru. ---";
pub(super) const PESAN_LAG: &str = "--- [x] ALIRAN LOG TERTINGGAL: Beberapa baris log terlewat karena aktivitas transfer terlalu padat. Langkah perbaikan: Silakan muat ulang halaman (refresh) untuk mengambil histori log yang utuh dari file disk. ---";

/// Tail awal log runtime dan batas atasnya (`docs/plan.md` tabel angka).
/// Log deploy memakai `reader::TAIL_DEFAULT`/`TAIL_MAX` (500/5000); runtime
/// lebih kecil karena ditarik lewat SSH tiap kali, bukan dibaca dari disk.
const RUNTIME_TAIL_DEFAULT: u32 = 200;
const RUNTIME_TAIL_MAX: u32 = 2000;

/// Query bersama tiga endpoint log deploy. `tail` di luar rentang DIJEPIT,
/// bukan 400 — ini kenyamanan baca, bukan perintah destruktif
/// (`docs/api-contract.md`).
#[derive(Debug, Deserialize, Default)]
pub struct LogQuery {
    pub tail: Option<usize>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RuntimeLogQuery {
    pub tail: Option<u32>,
    pub q: Option<String>,
}

/// Jepit `tail` log runtime: `None`/`0` → default 200, di atas 2000 → 2000.
pub(super) fn jepit_tail_runtime(n: Option<u32>) -> u32 {
    match n {
        None | Some(0) => RUNTIME_TAIL_DEFAULT,
        Some(n) if n > RUNTIME_TAIL_MAX => RUNTIME_TAIL_MAX,
        Some(n) => n,
    }
}

/// Petakan kegagalan baca file log ke status HTTP. `match` atas enum, BUKAN
/// parsing string pesan — itu sebabnya `LogReadError` ada.
///
/// `IdTidakValid` → 404 supaya tidak bisa dibedakan dari "id tidak dikenal"
/// (`docs/api-contract.md`: "Tidak ada perbedaan pesan antara keduanya").
fn petakan_error_baca(err: LogReadError) -> AppError {
    match err {
        LogReadError::IdTidakValid => AppError::NotFound,
        LogReadError::Timeout => AppError::Timeout(PESAN_PENCARIAN_TIMEOUT.to_string()),
        LogReadError::Io => {
            AppError::Internal(anyhow::anyhow!("gagal membaca file log deployment"))
        }
    }
}

/// Ambil baris log deploy dari FILE (tidak pernah dari SQLite — invariant §3
/// no.9). Mengembalikan `(baris, dipotong_karena_pencarian)`.
///
/// `q` kosong/absen → tail biasa. `q` terisi → pencarian dalam file.
async fn baca_baris(
    state: &AppState,
    deployment_id: &str,
    query: &LogQuery,
) -> Result<(Vec<LogLine>, bool), AppError> {
    // Gerbang tunggal: id divalidasi SEBELUM path dibentuk.
    reader::nama_file_aman(deployment_id).map_err(petakan_error_baca)?;
    let path = logs_writer::path_log(&state.config.log_dir, deployment_id);

    match query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => {
            let hasil = reader::cari(&path, q).await.map_err(petakan_error_baca)?;
            Ok((hasil.baris, hasil.dipotong))
        }
        None => {
            let hasil = reader::tail(&path, query.tail.unwrap_or(0))
                .await
                .map_err(petakan_error_baca)?;
            Ok((hasil.baris, false))
        }
    }
}

/// `GET /deployments/{id}/log` — halaman viewer log deploy.
///
/// Isi awal dirender dari FILE, bukan menunggu SSE: reload di tengah deploy
/// tetap menampilkan log yang benar. SSE hanya dipasang kalau deployment
/// BELUM selesai — yang sudah selesai dirender statis supaya klien tidak
/// menggantung menunggu event yang tidak akan datang.
pub async fn deploy_log_halaman(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Response, AppError> {
    let dep = deployments_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;
    let meta = logs_repo::find(&state.db_read, &id).await?;
    let (baris, dipotong) = baca_baris(&state, &id, &query).await?;
    let strip = fleet_strip(&state).await?;

    Ok(render::render_deploy_log(
        &dep,
        meta.as_ref().is_some_and(|m| m.truncated),
        &baris,
        dipotong,
        query.q.as_deref(),
        !dep.status.selesai(),
        &session.csrf_token,
        Some(strip),
    )
    .into_response())
}

/// `GET /deployments/{id}/log/isi` — fragmen HTML untuk HTMX (ganti `tail`,
/// jalankan pencarian). Tanpa app shell.
pub async fn deploy_log_isi(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Response, AppError> {
    let dep = deployments_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;
    let meta = logs_repo::find(&state.db_read, &id).await?;
    let (baris, dipotong) = baca_baris(&state, &id, &query).await?;

    Ok(render::render_log_fragmen(
        &baris,
        meta.as_ref().is_some_and(|m| m.truncated),
        dipotong,
        dep.status.selesai(),
    )
    .into_response())
}

/// `GET /deployments/{id}/log/unduh` — isi file APA ADANYA sebagai
/// `text/plain`.
///
/// Nama berkas dibentuk dari id yang SUDAH divalidasi (`deploy-{id}.log`),
/// bukan dari nama file di disk dan bukan dari input klien — id yang lolos
/// `^[A-Za-z0-9]{1,64}$` tidak mungkin menyuntik header.
///
/// File tidak ada (deployment lebih tua dari retensi 30 hari dan sudah
/// tersapu) → 404 biasa, tanpa membedakan "belum pernah ada" vs "sudah
/// dihapus" di level status.
pub async fn deploy_log_unduh(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    reader::nama_file_aman(&id).map_err(petakan_error_baca)?;
    deployments_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;

    let path = logs_writer::path_log(&state.config.log_dir, &id);
    let isi = match tokio::fs::read(&path).await {
        Ok(isi) => isi,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(AppError::NotFound),
        Err(err) => {
            // Path dan pesan io mentah HANYA ke tracing, tidak pernah ke klien.
            tracing::warn!(deployment_id = %id, error = %err, "gagal baca file log untuk diunduh");
            return Err(AppError::Internal(anyhow::anyhow!(
                "gagal membaca file log deployment"
            )));
        }
    };

    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/plain; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"deploy-{id}.log\""),
            ),
        ],
        isi,
    )
        .into_response())
}

/// `GET /apps/{id}/logs/isi` — fragmen HTML satu tarikan `docker logs --tail`
/// (TANPA follow). Dipakai untuk pencarian dan memuat ulang histori runtime.
///
/// Log runtime TIDAK PERNAH ditulis ke disk control plane — sumbernya sudah
/// persisten di server target (`docs/plan.md`).
///
/// Forward socket WAJIB ditutup sebelum handler mengembalikan respons, di
/// SEMUA jalur termasuk error — forward yang bocor adalah kebocoran fd.
pub async fn app_log_isi(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<RuntimeLogQuery>,
) -> Result<Response, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or(AppError::NotFound)?;

    // 409: tidak ada deployment live / container_id NULL. Bukan 404 (app-nya
    // ada) dan bukan 500 (ini keadaan normal untuk app yang belum pernah
    // dideploy) — `docs/api-contract.md`.
    let Some(live) = deployments_repo::find_current_live(&state.db_read, &app.id, "").await? else {
        return Ok(fragmen_tanpa_container());
    };
    let Some(container_id) = live.container_id.clone() else {
        return Ok(fragmen_tanpa_container());
    };

    let server = crate::servers::repo::find_by_id(&state.db_read, &app.server_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let Some(fingerprint) = server.host_key_fingerprint.clone() else {
        return Err(AppError::Conflict(
            "Server belum terverifikasi. Selesaikan verifikasi server dulu.".to_string(),
        ));
    };
    let plaintext_key = state
        .crypto
        .decrypt(&server.ssh_key_encrypted)
        .map_err(AppError::Internal)?;

    let session = match ssh::connect(
        &server.host,
        server.port as u16,
        &server.ssh_user,
        &plaintext_key,
        &state.config.runtime_dir,
        HostKeyMode::Strict {
            expected_fingerprint: fingerprint,
        },
    )
    .await
    {
        Ok(ssh::ConnectOutcome::Established(session)) => session,
        Ok(ssh::ConnectOutcome::TofuPending { session, .. }) => {
            let _ = session.close().await;
            return Err(AppError::Conflict(
                "Host key server berubah. Verifikasi ulang server sebelum membaca log.".to_string(),
            ));
        }
        // stderr ssh mentah TIDAK pernah sampai ke klien — hanya kategori.
        Err(err) => {
            tracing::warn!(app_id = %app.id, error = ?err, "gagal ssh untuk log runtime");
            return Ok(fragmen_server_tidak_merespons());
        }
    };

    let forward = match docker::establish(&session, &state.config.runtime_dir, &server.id).await {
        Ok(forward) => forward,
        Err(err) => {
            let _ = session.close().await;
            tracing::warn!(app_id = %app.id, error = ?err, "gagal forward socket untuk log runtime");
            return Ok(fragmen_server_tidak_merespons());
        }
    };

    let hasil = tarik_log_runtime(
        forward.socket_path(),
        &container_id,
        jepit_tail_runtime(query.tail),
    )
    .await;

    // Satu jalur penutupan untuk sukses MAUPUN gagal.
    docker::close(&session, forward).await;
    let _ = session.close().await;

    let teks = match hasil {
        Ok(teks) => teks,
        Err(status) => return Ok(status),
    };

    let baris = potong_hasil_pencarian(&teks, query.q.as_deref());
    Ok(render::render_log_fragmen(&baris.0, false, baris.1, false).into_response())
}

/// Tarik log dari Docker lewat socket yang sudah di-forward. Kegagalan
/// dipetakan ke fragmen kategori — nol stderr mentah, nol exit code
/// telanjang, nol path socket forward.
async fn tarik_log_runtime(
    socket_path: &std::path::Path,
    container_id: &str,
    tail: u32,
) -> Result<String, Response> {
    let client = docker::connect(socket_path).map_err(|err| {
        tracing::warn!(error = ?err, "gagal menyambung docker lewat socket forward");
        fragmen_server_tidak_merespons()
    })?;

    match docker::container_logs(&client, container_id, tail).await {
        Ok(teks) => Ok(teks),
        Err(err) => {
            tracing::warn!(error = ?err, "gagal menarik log runtime container");
            Err(fragmen_container_hilang())
        }
    }
}

/// Saring baris hasil tarikan runtime dengan `q`, dibatasi
/// `reader::SEARCH_MAX_RESULTS`. Mengembalikan `(baris, dipotong)`.
fn potong_hasil_pencarian(teks: &str, q: Option<&str>) -> (Vec<LogLine>, bool) {
    let q = q.map(str::trim).filter(|q| !q.is_empty());
    let mut baris = Vec::new();
    let mut dipotong = false;

    for (i, teks_baris) in teks.lines().enumerate() {
        if let Some(q) = q
            && !teks_baris.contains(q)
        {
            continue;
        }
        if baris.len() >= reader::SEARCH_MAX_RESULTS {
            dipotong = true;
            break;
        }
        baris.push(LogLine {
            nomor: (i + 1) as u64,
            teks: teks_baris.to_string(),
        });
    }

    (baris, dipotong)
}

fn fragmen_tanpa_container() -> Response {
    (
        StatusCode::CONFLICT,
        render::render_log_pesan(PESAN_BELUM_ADA_CONTAINER),
    )
        .into_response()
}

fn fragmen_container_hilang() -> Response {
    (
        StatusCode::BAD_GATEWAY,
        render::render_log_pesan(PESAN_CONTAINER_HILANG),
    )
        .into_response()
}

fn fragmen_server_tidak_merespons() -> Response {
    (
        StatusCode::GATEWAY_TIMEOUT,
        render::render_log_pesan(PESAN_TIMEOUT_KONEKSI),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jepit_tail_runtime_default_saat_absen_atau_nol() {
        assert_eq!(jepit_tail_runtime(None), RUNTIME_TAIL_DEFAULT);
        assert_eq!(jepit_tail_runtime(Some(0)), RUNTIME_TAIL_DEFAULT);
    }

    #[test]
    fn jepit_tail_runtime_menjepit_di_atas_maksimum_bukan_menolak() {
        assert_eq!(jepit_tail_runtime(Some(999_999)), RUNTIME_TAIL_MAX);
        assert_eq!(jepit_tail_runtime(Some(2001)), RUNTIME_TAIL_MAX);
    }

    #[test]
    fn jepit_tail_runtime_meneruskan_nilai_dalam_rentang() {
        assert_eq!(jepit_tail_runtime(Some(1)), 1);
        assert_eq!(jepit_tail_runtime(Some(2000)), 2000);
    }

    #[test]
    fn id_tidak_valid_dipetakan_ke_404_bukan_400() {
        assert!(matches!(
            petakan_error_baca(LogReadError::IdTidakValid),
            AppError::NotFound
        ));
    }

    #[test]
    fn timeout_pencarian_dipetakan_ke_504() {
        assert!(matches!(
            petakan_error_baca(LogReadError::Timeout),
            AppError::Timeout(_)
        ));
    }

    #[test]
    fn io_dipetakan_ke_internal_tanpa_membocorkan_path() {
        let AppError::Internal(err) = petakan_error_baca(LogReadError::Io) else {
            panic!("LogReadError::Io harus jadi AppError::Internal");
        };
        let pesan = err.to_string();
        assert!(!pesan.contains('/'), "pesan bocor path: {pesan}");
    }

    #[test]
    fn pencarian_runtime_menyaring_dan_menomori_baris_asli() {
        let teks = "satu\ndua\ntiga dua\n";
        let (baris, dipotong) = potong_hasil_pencarian(teks, Some("dua"));

        assert!(!dipotong);
        assert_eq!(baris.len(), 2);
        assert_eq!(baris[0].nomor, 2);
        assert_eq!(baris[1].nomor, 3);
    }

    #[test]
    fn pencarian_runtime_tanpa_query_mengembalikan_semua() {
        let teks = "a\nb\nc\n";
        let (baris, dipotong) = potong_hasil_pencarian(teks, None);

        assert!(!dipotong);
        assert_eq!(baris.len(), 3);
    }

    #[test]
    fn pencarian_runtime_dipotong_di_batas_hasil() {
        let teks: String = (0..reader::SEARCH_MAX_RESULTS + 50)
            .map(|i| format!("cocok-{i}\n"))
            .collect();
        let (baris, dipotong) = potong_hasil_pencarian(&teks, Some("cocok"));

        assert!(dipotong, "melebihi batas harus ditandai dipotong");
        assert_eq!(baris.len(), reader::SEARCH_MAX_RESULTS);
    }
}
