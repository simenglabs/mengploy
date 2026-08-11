//! `GET /events/verifikasi/{id}` — satu-satunya endpoint SSE Fase 1, HANYA
//! untuk progres verifikasi (bukan viewer log — itu Fase 3),
//! `docs/api-contract.md`.
//!
//! `job_id` = id server (`crate::events::EventRegistry` dokumentasi: Fase 1
//! hanya satu jalur verifikasi per server, jadi id server sudah cukup
//! sebagai kunci job).
//!
//! Fase 3 menambah dua SSE log: `GET /events/log/deploy/{id}` (siaran baris
//! yang sedang ditulis writer) dan `GET /events/log/runtime/{id}`
//! (`docker logs --follow` lewat socket yang di-forward SSH). Keduanya WAJIB
//! terautentikasi — tidak ada token buram yang menggantikan sesi
//! (`docs/prd.md:289`).

use std::convert::Infallible;
use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::{Extension, Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use tokio::sync::{Semaphore, mpsc};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};

use crate::apps::repo as apps_repo;
use crate::auth::session::Session;
use crate::deployments::repo as deployments_repo;
use crate::docker::{self, LogFollowError};
use crate::error::AppError;
use crate::events::VerificationEvent;
use crate::logs::LogEvent;
use crate::logs::reader;
use crate::servers::model::LangkahStatus;
use crate::servers::repo;
use crate::ssh::{self, HostKeyMode};
use crate::state::AppState;
use crate::web;

use super::logs::{RuntimeLogQuery, jepit_tail_runtime};
use super::servers::{checklist_awal, not_found};
use crate::web as render_log;

/// Batas sesi log runtime serentak (`docs/plan.md` tabel angka: 4).
const MAX_SESI_LOG_RUNTIME: usize = 4;

/// Durasi maksimum SATU sesi SSE log runtime (`docs/plan.md`: 30 menit).
/// Ini BUKAN timeout global atas operasi jarak jauh (invariant §3 no.11) —
/// sunyi tanpa baris baru tetap sah selama batas ini belum lewat; yang
/// dibatasi adalah umur sesi, dan habisnya batas menghasilkan penutupan
/// RAPI dengan event `selesai`, bukan error.
const DURASI_MAKS_SESI_RUNTIME: Duration = Duration::from_secs(30 * 60);

/// Semaphore pembatas sesi log runtime.
///
// ponytail: `static` alih-alih field `AppState` karena `src/state.rs` di luar
// glob sub-blok ini. Batasnya: satu proses satu pembatas — benar untuk arsitektur
// single-binary sekarang, dan test tidak bisa mengisolasi kuotanya. Upgrade jadi
// field `AppState` saat ada dua instance router dalam satu proses (mis. test
// integrasi yang menuntut kuota terpisah per instance).
fn semaphore_runtime() -> &'static Semaphore {
    static SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();
    SEMAPHORE.get_or_init(|| Semaphore::new(MAX_SESI_LOG_RUNTIME))
}

/// Nama event SSE penutup. Klien berhenti menunggu setelah menerimanya.
const EVENT_SELESAI: &str = "selesai";

/// Nama event SSE penanda baris terlewat karena subscriber lag.
const EVENT_TERTINGGAL: &str = "tertinggal";

fn event_selesai(pesan: &str) -> Event {
    Event::default()
        .event(EVENT_SELESAI)
        .data(render_log::render_log_pesan(pesan).into_string())
}

/// Isi `data` fragmen baris log, dipisah dari pembungkus `Event` supaya bisa
/// diuji langsung (`axum::response::sse::Event` tidak mengekspos `data`-nya).
fn data_baris(teks: &str) -> String {
    let baris: Vec<reader::LogLine> = teks
        .lines()
        .enumerate()
        .map(|(i, t)| reader::LogLine {
            nomor: (i + 1) as u64,
            teks: t.to_string(),
        })
        .collect();
    render_log::render_log_fragmen(&baris, false, false, false).into_string()
}

/// Isi `data` penanda baris terlewat. `n` WAJIB muncul — pengguna harus tahu
/// BERAPA banyak yang hilang, bukan hanya bahwa ada yang hilang.
fn data_tertinggal(n: u64) -> String {
    // Teks final `docs/design/log-viewer.md` §8, dengan jumlah baris yang
    // hilang disisipkan — pengguna harus tahu BERAPA banyak yang terlewat,
    // bukan hanya bahwa ada yang terlewat.
    render_log::render_log_pesan(&format!(
        "{} ({n} baris)",
        super::logs::PESAN_LAG.trim_end_matches(" ---")
    ))
    .into_string()
}

/// Stream berisi TEPAT satu event `selesai`, lalu tutup. Dipakai saat tidak
/// ada sesi log aktif — **tanpa membuat channel baru**.
fn sse_satu_event_selesai(pesan: &str) -> Response {
    let satu = tokio_stream::once(Ok::<Event, Infallible>(event_selesai(pesan)));
    Sse::new(satu)
        .keep_alive(KeepAlive::default())
        .into_response()
}

pub async fn verifikasi_stream(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let server = repo::find_ringkas_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    let csrf_token = session.csrf_token;

    // Job sudah selesai sebelum klien sempat menyambung (mis. reload
    // halaman setelah verifikasi kelar) — channel-nya sudah dibuang
    // (`events::EventRegistry::remove`). Kirim SATU snapshot dari status db
    // lalu tutup, alih-alih membuka koneksi yang menggantung tanpa pernah
    // menerima event apa pun.
    if server.status != crate::servers::model::StatusServer::Verifying {
        let event = checklist_awal(&server);
        let fragmen = web::render_verifikasi_fragmen(&id, &event, &csrf_token).into_string();
        let satu = tokio_stream::once(Ok::<Event, Infallible>(Event::default().data(fragmen)));
        return Ok(Sse::new(satu)
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let rx = state.events.subscribe(&id);
    // `tokio_stream::StreamExt` tidak punya `scan` (itu `futures_util`, yang
    // sengaja tidak ditambah sebagai dependency — Q3 `docs/plan.md`).
    // Task terpisah meneruskan event satu-per-satu ke channel `mpsc` dan
    // BERHENTI segera setelah event yang menandai job selesai diteruskan —
    // ini yang membuat stream SSE ditutup TEPAT setelah event terakhir,
    // bukan sebelum atau after itu (`docs/api-contract.md`).
    let (tx, out_rx) = mpsc::channel::<VerificationEvent>(8);
    tokio::spawn(async move {
        let mut broadcast_stream = BroadcastStream::new(rx);
        while let Some(hasil) = broadcast_stream.next().await {
            let Ok(event) = hasil else {
                continue; // lag di broadcast channel — lewati, bukan fatal
            };
            let selesai = job_selesai(&event);
            if tx.send(event).await.is_err() {
                break; // klien sudah terputus
            }
            if selesai {
                break;
            }
        }
    });

    let stream = ReceiverStream::new(out_rx).map(move |event| {
        let fragmen = web::render_verifikasi_fragmen(&id, &event, &csrf_token).into_string();
        Ok::<Event, Infallible>(Event::default().data(fragmen))
    });

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `GET /events/deploy/{id}` — timeline deployment SSE, pola sama
/// `verifikasi_stream` (job selesai sebelum klien menyambung → satu
/// snapshot lalu tutup; belum selesai → forward tiap event, tutup TEPAT
/// setelah event yang menandai status akhir). BEDA: forwarder di sini
/// membaca ULANG baris `deployments` tiap event (bukan cuma meneruskan
/// payload broadcast) supaya fragmen selalu punya `error_kind`/
/// `error_detail` terbaru, bukan hanya `status`.
pub async fn deploy_stream(
    State(state): State<AppState>,
    Extension(_session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let dep = deployments_repo::find_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;
    let app = apps_repo::find_by_id(&state.db_read, &dep.app_id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    if dep.status.selesai() {
        let fragmen = web::render_deployment_fragmen(&dep, &app.name).into_string();
        let satu = tokio_stream::once(Ok::<Event, Infallible>(Event::default().data(fragmen)));
        return Ok(Sse::new(satu)
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let rx = state.deployment_events.subscribe(&id);
    let (tx, out_rx) = mpsc::channel::<String>(8);
    let db_read = state.db_read.clone();
    let deployment_id = id.clone();
    let app_name = app.name.clone();
    tokio::spawn(async move {
        let mut broadcast_stream = BroadcastStream::new(rx);
        while let Some(hasil) = broadcast_stream.next().await {
            let Ok(event) = hasil else {
                continue; // lag di broadcast channel — lewati, bukan fatal
            };
            let selesai = web::deployments::job_selesai(&event);
            let Ok(Some(dep_terbaru)) =
                deployments_repo::find_by_id(&db_read, &deployment_id).await
            else {
                continue; // baris hilang di antara event — lewati, bukan fatal
            };
            let fragmen = web::render_deployment_fragmen(&dep_terbaru, &app_name).into_string();
            if tx.send(fragmen).await.is_err() {
                break; // klien sudah terputus
            }
            if selesai {
                break;
            }
        }
    });

    let stream = ReceiverStream::new(out_rx)
        .map(move |fragmen| Ok::<Event, Infallible>(Event::default().data(fragmen)));

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `GET /events/log/deploy/{id}` — SSE log deploy langsung.
///
/// Fragmen tiap event di-**APPEND** HTMX, bukan swap seluruh isi: histori yang
/// sudah dirender tidak boleh hilang tiap event.
///
/// Tidak ada sesi log aktif (`LogRegistry::ikut` → `None`, mis. deployment
/// sudah selesai sebelum klien menyambung) → kirim SATU event `selesai` lalu
/// tutup. **JANGAN membuat channel baru** — hanya writer yang boleh membuat
/// sesi (`docs/plan.md` aturan 1). Membuat channel di sini adalah kebocoran
/// memori yang PRD tandai sebagai risiko utama proyek (`docs/prd.md:291`).
pub async fn log_deploy_stream(
    State(state): State<AppState>,
    Extension(_session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    // Id divalidasi SEBELUM dipakai sebagai kunci registry; tidak lolos pola
    // → 404, sama seperti id tidak dikenal.
    if reader::nama_file_aman(&id).is_err() {
        return Err(AppError::NotFound);
    }
    deployments_repo::find_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    let Some(sesi) = state.logs.ikut(&id) else {
        return Ok(sse_satu_event_selesai(
            "Log deploy sudah selesai ditulis. Muat ulang halaman untuk melihat histori lengkap.",
        ));
    };

    let rx = sesi.subscribe();
    let (tx, out_rx) = mpsc::channel::<Event>(16);
    tokio::spawn(async move {
        // `sesi` dipegang selama forwarder hidup supaya `Arc` tidak drop di
        // tengah jalan; `Drop for LogSession` yang membersihkan entri map
        // baru jalan setelah writer DAN semua subscriber lepas.
        let _sesi = sesi;
        let mut broadcast_stream = BroadcastStream::new(rx);
        while let Some(hasil) = broadcast_stream.next().await {
            let (event, selesai) = match hasil {
                Ok(LogEvent::Baris(baris)) => (fragmen_baris(&baris), false),
                Ok(LogEvent::Tertinggal(n)) => (fragmen_tertinggal(n), false),
                Ok(LogEvent::Selesai) => (
                    event_selesai("Log deploy selesai. Deployment sudah mencapai status akhir."),
                    true,
                ),
                // Lag di broadcast channel TIDAK PERNAH didiamkan untuk log
                // (`docs/plan.md` aturan 4) — beda dari `verifikasi_stream`
                // dan `deploy_stream` di atas yang `continue`, karena di sana
                // tiap event adalah SNAPSHOT penuh sehingga melewatkan satu
                // event tidak menghilangkan informasi. Log adalah aliran
                // append-only: baris yang hilang hilang selamanya, dan
                // pengguna akan mengira lognya utuh padahal berlubang.
                Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                    (fragmen_tertinggal(n), false)
                }
            };

            if tx.send(event).await.is_err() {
                break; // klien sudah terputus
            }
            if selesai {
                break;
            }
        }
    });

    let stream = ReceiverStream::new(out_rx).map(Ok::<Event, Infallible>);
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `GET /events/log/runtime/{id}` — SSE log runtime dari `docker logs
/// --follow` lewat socket yang di-forward SSH. `{id}` adalah id **app**.
///
/// Endpoint ini murni MEMBACA: tidak menulis db, tidak menulis file (log
/// runtime tidak dipersistensi di control plane), tidak mengubah state
/// container apa pun.
pub async fn log_runtime_stream(
    State(state): State<AppState>,
    Extension(_session): Extension<Session>,
    Path(id): Path<String>,
    Query(query): Query<RuntimeLogQuery>,
) -> Result<Response, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, &id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;

    // 429 sebelum menyentuh jaringan: kuota habis berarti kita menolak, bukan
    // menunggu — antre di sini akan menahan koneksi HTTP tanpa batas.
    let Ok(izin) = semaphore_runtime().try_acquire() else {
        return Err(AppError::TooManyRequests(
            super::logs::PESAN_TERLALU_BANYAK_SESI.to_string(),
        ));
    };

    // 409: tidak ada deployment live / container_id NULL → stream TIDAK
    // dibuka sama sekali.
    let Some(live) = deployments_repo::find_current_live(&state.db_read, &app.id, "")
        .await
        .map_err(AppError::from)?
    else {
        return Err(AppError::Conflict(
            "Belum ada container yang berjalan untuk app ini.".to_string(),
        ));
    };
    let Some(container_id) = live.container_id.clone() else {
        return Err(AppError::Conflict(
            "Belum ada container yang berjalan untuk app ini.".to_string(),
        ));
    };

    let server = crate::servers::repo::find_by_id(&state.db_read, &app.server_id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(not_found)?;
    let Some(fingerprint) = server.host_key_fingerprint.clone() else {
        return Err(AppError::Conflict(
            "Server belum terverifikasi. Selesaikan verifikasi server dulu.".to_string(),
        ));
    };
    let plaintext_key = state
        .crypto
        .decrypt(&server.ssh_key_encrypted)
        .map_err(AppError::Internal)?;

    let sesi_ssh = match ssh::connect(
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
        Ok(ssh::ConnectOutcome::Established(sesi_ssh)) => sesi_ssh,
        Ok(ssh::ConnectOutcome::TofuPending { session, .. }) => {
            let _ = session.close().await;
            return Err(AppError::Conflict(
                "Host key server berubah. Verifikasi ulang server sebelum membaca log.".to_string(),
            ));
        }
        // stderr ssh mentah TIDAK pernah sampai ke klien — hanya kategori.
        Err(err) => {
            tracing::warn!(app_id = %app.id, error = ?err, "gagal ssh untuk stream log runtime");
            return Err(AppError::Timeout(
                "Server tidak merespons saat membuka log. Periksa koneksi server lalu coba lagi."
                    .to_string(),
            ));
        }
    };

    let forward = match docker::establish(&sesi_ssh, &state.config.runtime_dir, &server.id).await {
        Ok(forward) => forward,
        Err(err) => {
            let _ = sesi_ssh.close().await;
            tracing::warn!(app_id = %app.id, error = ?err, "gagal forward socket untuk stream log runtime");
            return Err(AppError::Timeout(
                "Server tidak merespons saat membuka log. Periksa koneksi server lalu coba lagi."
                    .to_string(),
            ));
        }
    };

    let tail = jepit_tail_runtime(query.tail);
    let (tx, out_rx) = mpsc::channel::<Event>(16);
    // `bollard::Docker` dan stream turunannya tidak bisa hidup di luar task
    // yang memilikinya (stream meminjam client), jadi SELURUH sesi streaming
    // berjalan di dalam satu task. Kegagalan MEMBUKA stream tetap harus jadi
    // status HTTP, bukan event — jadi hasil pembukaan dikirim balik lewat
    // `oneshot` dan handler menunggunya sebelum mengembalikan respons.
    let (buka_tx, buka_rx) = tokio::sync::oneshot::channel::<Result<(), LogFollowError>>();
    // Izin Semaphore di-`forget` lalu dilepas manual di akhir task, supaya
    // kuota baru bebas saat sesi benar-benar berakhir — bukan saat handler
    // mengembalikan respons (yang terjadi jauh lebih dulu).
    izin.forget();
    let app_id = app.id.clone();
    tokio::spawn(async move {
        let pesan_penutup = match docker::connect(forward.socket_path()) {
            Err(err) => {
                tracing::warn!(app_id = %app_id, error = ?err, "gagal menyambung docker lewat socket forward");
                let _ = buka_tx.send(Err(LogFollowError::Unreachable));
                None
            }
            Ok(client) => match docker::container_logs_follow(&client, &container_id, tail).await {
                Err(err) => {
                    let _ = buka_tx.send(Err(err));
                    None
                }
                Ok((chunk_pertama, docker_stream)) => {
                    if buka_tx.send(Ok(())).is_err() {
                        // Handler sudah pergi (klien membatalkan request
                        // sebelum respons terbentuk) — jangan mulai streaming.
                        None
                    } else {
                        alirkan_runtime(&tx, chunk_pertama, docker_stream, DURASI_MAKS_SESI_RUNTIME)
                            .await
                    }
                }
            },
        };

        // SATU jalur penutupan untuk KETIGA sebab (klien putus / 30 menit /
        // stream Docker berakhir) DAN untuk jalur gagal-membuka. Menduplikasi
        // pembersihan di tiap cabang adalah cara paling mudah melewatkan
        // salah satunya — forward yang bocor adalah kebocoran fd di /run.
        tutup_sesi_runtime(sesi_ssh, forward).await;
        semaphore_runtime().add_permits(1);

        if let Some(pesan) = pesan_penutup {
            // Klien yang masih tersambung diberi tahu alasan penutupan;
            // gagal kirim berarti dia sudah pergi, dan itu bukan error.
            let _ = tx.send(event_selesai(&pesan)).await;
        }
        tracing::debug!(app_id = %app_id, "sesi log runtime ditutup dan izin dilepas");
    });

    match buka_rx.await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(petakan_error_follow(err)),
        // Task hilang tanpa mengirim hasil — anggap tidak terjangkau, dan
        // jangan biarkan klien menggantung menunggu stream yang tidak ada.
        Err(_) => {
            return Err(AppError::Timeout(
                "Server tidak merespons saat membuka log. Periksa koneksi server lalu coba lagi."
                    .to_string(),
            ));
        }
    }

    let stream = ReceiverStream::new(out_rx).map(Ok::<Event, Infallible>);
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// Alirkan baris log runtime ke klien sampai salah satu dari tiga sebab
/// penutupan terjadi. Mengembalikan pesan penutup yang perlu dikirim, atau
/// `None` kalau klien sudah terputus (tidak ada yang perlu diberi tahu).
///
/// **Sunyi tanpa baris baru BUKAN error** — tidak ada timeout di dalam loop
/// ini; keep-alive SSE yang menjaga koneksi (invariant §3 no.11). Yang
/// dibatasi hanya UMUR sesi, lewat `tokio::time::timeout` yang membungkus
/// seluruh aliran dan berakhir RAPI.
async fn alirkan_runtime<S>(
    tx: &mpsc::Sender<Event>,
    chunk_pertama: String,
    docker_stream: S,
    batas_sesi: Duration,
) -> Option<String>
where
    S: tokio_stream::Stream<Item = Result<String, LogFollowError>>,
{
    let aliran = async {
        if tx.send(fragmen_baris(&chunk_pertama)).await.is_err() {
            return None; // sebab 1: klien terputus
        }

        let mut docker_stream = std::pin::pin!(docker_stream);
        while let Some(hasil) = docker_stream.next().await {
            let chunk = match hasil {
                Ok(chunk) => chunk,
                Err(err) => {
                    tracing::warn!(error = ?err, "stream log runtime berakhir dengan kegagalan");
                    return Some(pesan_penutup_follow(err));
                }
            };
            if tx.send(fragmen_baris(&chunk)).await.is_err() {
                return None; // sebab 1: klien terputus
            }
        }

        // sebab 3: stream Docker berakhir (container berhenti/dihapus).
        Some("Stream log berakhir; container sudah tidak mengirim keluaran baru.".to_string())
    };

    match tokio::time::timeout(batas_sesi, aliran).await {
        Ok(pesan) => pesan,
        // sebab 2: batas 30 menit.
        Err(_) => Some(super::logs::PESAN_SESI_30_MENIT.to_string()),
    }
}

/// Jalur penutupan bersih: tutup forward socket lalu sesi SSH. Dipanggil dari
/// SATU tempat di jalur streaming, plus jalur error sebelum streaming dimulai.
async fn tutup_sesi_runtime(sesi_ssh: ssh::SshSession, forward: docker::DockerForward) {
    docker::close(&sesi_ssh, forward).await;
    let _ = sesi_ssh.close().await;
}

/// Fragmen satu-atau-beberapa baris log. Maud meng-escape otomatis — isi log
/// adalah keluaran aplikasi pengguna dan diperlakukan sebagai data tidak
/// tepercaya (nol `PreEscaped`).
fn fragmen_baris(teks: &str) -> Event {
    Event::default().data(data_baris(teks))
}

/// Penanda baris terlewat. `n` WAJIB muncul — pengguna harus tahu berapa
/// banyak yang hilang, bukan hanya bahwa ada yang hilang.
fn fragmen_tertinggal(n: u64) -> Event {
    Event::default()
        .event(EVENT_TERTINGGAL)
        .data(data_tertinggal(n))
}

/// Kegagalan MEMBUKA stream → status HTTP (handler belum mengirim apa pun).
fn petakan_error_follow(err: LogFollowError) -> AppError {
    match err {
        LogFollowError::ContainerHilang => AppError::BadGateway(
            "Container sudah tidak ada di server; log runtimenya tidak bisa ditampilkan lagi. \
             Lihat log deploy terakhir untuk tahu apa yang terjadi."
                .to_string(),
        ),
        LogFollowError::TimeoutChunkPertama | LogFollowError::Unreachable => AppError::Timeout(
            "Server tidak merespons saat membuka log. Periksa koneksi server lalu coba lagi."
                .to_string(),
        ),
    }
}

/// Kegagalan di TENGAH stream → pesan penutup (status HTTP sudah terkirim,
/// jadi satu-satunya cara memberi tahu klien adalah event `selesai`).
fn pesan_penutup_follow(err: LogFollowError) -> String {
    match err {
        LogFollowError::ContainerHilang => {
            "Container sudah tidak ada di server; log runtimenya tidak bisa ditampilkan lagi."
                .to_string()
        }
        LogFollowError::TimeoutChunkPertama | LogFollowError::Unreachable => {
            "Server berhenti merespons; sesi log ditutup. Muat ulang untuk mencoba lagi."
                .to_string()
        }
    }
}

/// Job dianggap selesai (koneksi & Docker sudah di titik akhir) kalau
/// koneksi gagal, ATAU langkah Docker sudah `Sukses`/`Gagal`. TOFU pending
/// BUKAN selesai — stream tetap terbuka menunggu
/// `konfirmasi_hostkey_dan_lanjutkan` mempublikasikan event lanjutan ke
/// channel yang sama.
fn job_selesai(event: &VerificationEvent) -> bool {
    if event.tofu_pending_fingerprint.is_some() {
        return false;
    }
    let koneksi_gagal = event
        .langkah
        .first()
        .is_some_and(|l| l.status == LangkahStatus::Gagal);
    let docker_selesai = event
        .langkah
        .get(1)
        .is_some_and(|l| matches!(l.status, LangkahStatus::Sukses | LangkahStatus::Gagal));

    koneksi_gagal || docker_selesai
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::model::LangkahVerifikasi;

    fn langkah(status: LangkahStatus) -> LangkahVerifikasi {
        LangkahVerifikasi {
            nama: "x".to_string(),
            status,
            pesan: None,
        }
    }

    #[test]
    fn penanda_tertinggal_memuat_jumlah_baris_yang_hilang() {
        let teks = data_tertinggal(42);

        assert!(
            teks.contains("42"),
            "jumlah baris terlewat WAJIB terlihat pengguna: {teks}"
        );
        assert!(teks.contains("terlewat"), "{teks}");
    }

    #[test]
    fn isi_log_dieskape_bukan_dieksekusi() {
        let teks = data_baris("<script>alert(1)</script>");

        assert!(
            !teks.contains("<script>"),
            "isi log adalah data tidak tepercaya dan wajib di-escape: {teks}"
        );
        assert!(teks.contains("&lt;script&gt;"), "{teks}");
    }

    #[test]
    fn container_hilang_dipetakan_ke_502_bukan_504() {
        assert!(matches!(
            petakan_error_follow(LogFollowError::ContainerHilang),
            AppError::BadGateway(_)
        ));
    }

    #[test]
    fn timeout_dan_unreachable_dipetakan_ke_504() {
        assert!(matches!(
            petakan_error_follow(LogFollowError::TimeoutChunkPertama),
            AppError::Timeout(_)
        ));
        assert!(matches!(
            petakan_error_follow(LogFollowError::Unreachable),
            AppError::Timeout(_)
        ));
    }

    #[test]
    fn pesan_penutup_tidak_memuat_detail_internal() {
        for err in [
            LogFollowError::ContainerHilang,
            LogFollowError::TimeoutChunkPertama,
            LogFollowError::Unreachable,
        ] {
            let pesan = pesan_penutup_follow(err);
            assert!(!pesan.is_empty());
            assert!(!pesan.contains('/'), "pesan bocor path: {pesan}");
            assert!(
                !pesan.contains("Error"),
                "pesan bocor tipe library: {pesan}"
            );
        }
    }

    /// `ikut()` untuk key tanpa sesi aktif mengembalikan `None` DAN tidak
    /// menyisakan entri — jaminan struktural yang mencegah SSE membuat channel
    /// untuk deployment yang sudah mati (`docs/prd.md:291`).
    #[test]
    fn ikut_tanpa_sesi_tidak_membuat_channel_baru() {
        let registry = std::sync::Arc::new(crate::logs::LogRegistry::new());

        assert!(registry.ikut("deployment-yang-sudah-mati").is_none());
        assert_eq!(registry.jumlah_sesi(), 0);
    }

    #[test]
    fn tail_runtime_dijepit_bukan_ditolak() {
        assert_eq!(jepit_tail_runtime(None), 200);
        assert_eq!(jepit_tail_runtime(Some(0)), 200);
        assert_eq!(jepit_tail_runtime(Some(999_999)), 2000);
        assert_eq!(jepit_tail_runtime(Some(150)), 150);
    }

    /// Batas sesi membungkus UMUR sesi, bukan menjadi timeout global atas
    /// operasi jarak jauh: aliran yang sunyi (tidak ada baris baru) tetap
    /// hidup sampai batas, lalu ditutup dengan pesan RAPI.
    #[tokio::test]
    async fn sunyi_bukan_error_dan_batas_sesi_menutup_dengan_rapi() {
        let (tx, mut rx) = mpsc::channel::<Event>(4);
        // Stream yang tidak pernah menghasilkan apa pun dan tidak pernah
        // berakhir — meniru container yang hidup tapi diam.
        let sunyi = tokio_stream::pending::<Result<String, LogFollowError>>();

        let pesan = alirkan_runtime(
            &tx,
            "baris pertama".to_string(),
            sunyi,
            Duration::from_millis(80),
        )
        .await
        .expect("batas sesi harus menghasilkan pesan penutup");

        assert!(rx.recv().await.is_some(), "chunk pertama harus terkirim");
        assert!(pesan.contains("30 menit"), "{pesan}");
    }

    #[tokio::test]
    async fn klien_terputus_tidak_menghasilkan_pesan_penutup() {
        let (tx, rx) = mpsc::channel::<Event>(1);
        drop(rx); // klien pergi sebelum baris pertama terkirim

        let pesan = alirkan_runtime(
            &tx,
            "baris".to_string(),
            tokio_stream::pending::<Result<String, LogFollowError>>(),
            DURASI_MAKS_SESI_RUNTIME,
        )
        .await;

        assert!(
            pesan.is_none(),
            "tidak ada yang perlu diberi tahu kalau klien sudah pergi"
        );
    }

    #[tokio::test]
    async fn stream_docker_berakhir_menutup_dengan_pesan_kategori() {
        let (tx, mut rx) = mpsc::channel::<Event>(8);
        let stream = tokio_stream::iter(vec![Ok::<String, LogFollowError>("satu".to_string())]);

        let pesan = alirkan_runtime(&tx, "nol".to_string(), stream, DURASI_MAKS_SESI_RUNTIME).await;

        assert!(pesan.is_some_and(|p| p.contains("berakhir")));
        assert!(rx.recv().await.is_some(), "chunk pertama harus terkirim");
        assert!(rx.recv().await.is_some(), "chunk kedua harus terkirim");
    }

    #[test]
    fn belum_selesai_saat_koneksi_masih_berjalan() {
        let event = VerificationEvent {
            langkah: vec![
                langkah(LangkahStatus::Berjalan),
                langkah(LangkahStatus::Menunggu),
            ],
            tofu_pending_fingerprint: None,
        };
        assert!(!job_selesai(&event));
    }

    #[test]
    fn selesai_saat_koneksi_gagal() {
        let event = VerificationEvent {
            langkah: vec![
                langkah(LangkahStatus::Gagal),
                langkah(LangkahStatus::Menunggu),
            ],
            tofu_pending_fingerprint: None,
        };
        assert!(job_selesai(&event));
    }

    #[test]
    fn selesai_saat_docker_sukses_walau_registry_masih_menunggu() {
        let event = VerificationEvent {
            langkah: vec![
                langkah(LangkahStatus::Sukses),
                langkah(LangkahStatus::Sukses),
                langkah(LangkahStatus::Menunggu),
            ],
            tofu_pending_fingerprint: None,
        };
        assert!(job_selesai(&event));
    }

    #[test]
    fn belum_selesai_saat_tofu_pending_walau_docker_masih_menunggu() {
        let event = VerificationEvent {
            langkah: vec![
                langkah(LangkahStatus::Berjalan),
                langkah(LangkahStatus::Menunggu),
            ],
            tofu_pending_fingerprint: Some("SHA256:abc".to_string()),
        };
        assert!(!job_selesai(&event));
    }
}
