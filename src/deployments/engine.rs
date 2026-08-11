//! Mesin state deployment: pulling → starting → checking → live, cabang
//! failed. Urutan pergantian container TIDAK BOLEH dibalik (invariant 1,
//! `docs/plan.md` "Urutan pergantian container"): start container baru →
//! health check KITA → (gagal: tangkap log, hapus, container lama utuh) →
//! (lulus: stop container lama) → live.

use std::time::Duration;

use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use sqlx::SqlitePool;

use super::model::{DeploymentRingkas, StatusDeployment};
use super::repo as deployments_repo;
use crate::apps::model::AppRingkas;
use crate::apps::repo as apps_repo;
use crate::docker::{self, DockerCredentials};
use crate::events::DeploymentEvent;
use crate::logs::{self, LogWriter};
use crate::notifications::{self, model::WebhookEnvelope};
use crate::registries;
use crate::servers::repo as servers_repo;
use crate::ssh::{self, HostKeyMode, SshSession};
use crate::state::AppState;

/// Umur lock deploy per app (invariant §3 no.12 — WAJIB kedaluwarsa).
/// Cukup untuk skenario terburuk: pull 10 menit + start 30 detik + health
/// check beberapa puluh detik + drain 30 detik, dengan margin.
pub const LOCK_TTL_SECS: i64 = 900;

const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Toleransi TAMBAHAN di atas `apps.health_grace_secs` sebelum health check
/// dianggap gagal total — dua angka terpisah (bukan satu timeout global,
/// invariant 11): grace period per-app + ambang tetap platform.
const HEALTH_EXTRA_THRESHOLD_SECS: i64 = 30;
/// Nama jaringan docker platform — dikunci literal (`docs/plan.md` Q4).
const DOCKER_NETWORK: &str = "platform";
const LOG_TAIL_LINES: u32 = 50;
/// "Drain container lama: 30 detik" — `--time` WAJIB eksplisit
/// (`docs/plan.md`, default docker 10 detik terlalu pendek).
const DRAIN_GRACE_SECS: i32 = 30;
/// Tulis file env audit ke target — perintah pendek (`install`), pola sama
/// `SHORT_COMMAND_TIMEOUT` di `docker/registry_login.rs`.
const ENV_FILE_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// Layout on-disk server target dikunci `CLAUDE.md` §6 — satu file per
/// app, ditimpa tiap redeploy (`docs/plan.md` Fase 4 Q2).
const ENV_FILE_DIR: &str = "/var/lib/platform/env";

/// Kategori kegagalan deploy — `kind()` masuk `deployments.error_kind`,
/// `pesan()` pesan Bahasa Indonesia final. Tiga mode kegagalan health check
/// persis `docs/prd.md:266`, plus `PullGagal` dan `Lain` sebagai tampungan.
#[derive(Debug)]
pub(crate) enum DeployKegagalan {
    PullGagal(String),
    ContainerExited { exit_code: i64 },
    HealthNon2xx { status: u16, body: String },
    HealthNoResponse,
    Lain(String),
}

impl DeployKegagalan {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::PullGagal(_) => "pull_gagal",
            Self::ContainerExited { .. } => "container_exited",
            Self::HealthNon2xx { .. } => "health_non_2xx",
            Self::HealthNoResponse => "health_no_response",
            Self::Lain(_) => "lain",
        }
    }

    pub(crate) fn pesan(&self) -> String {
        match self {
            Self::PullGagal(detail) => format!(
                "Gagal menarik image dari registry: {detail}. Langkah perbaikan: Pastikan \
                 digest image benar dan kredensial registry (kalau privat) masih berlaku."
            ),
            Self::ContainerExited { exit_code } => format!(
                "Container keluar dengan kode {exit_code} sebelum lolos health check. \
                 Kemungkinan besar: env var salah atau dependency hilang. Periksa log \
                 aplikasi di server target."
            ),
            Self::HealthNon2xx { status, body } => format!(
                "Container berjalan tapi health check membalas status {status} (bukan 2xx). \
                 Kemungkinan besar: koneksi database atau migrasi gagal. Body respons:\n{body}"
            ),
            Self::HealthNoResponse => "Container berjalan tapi tidak merespons health check \
                 sama sekali. Kemungkinan besar: aplikasi bind ke 127.0.0.1, seharusnya ke \
                 0.0.0.0, atau port salah."
                .to_string(),
            Self::Lain(detail) => format!(
                "Deploy gagal karena kesalahan tak terduga: {detail}. Langkah perbaikan: Coba \
                 lagi; kalau berulang, periksa log aplikasi di server control plane."
            ),
        }
    }
}

/// Format satu baris log dengan stempel waktu `HH:MM:SS | pesan` — pola
/// persis contoh gutter `docs/design/log-viewer.md` (`12:04:55 |`). Baris
/// yang TIDAK diawali stempel ini (keluaran mentah aplikasi pengguna, kalau
/// nanti dipertimbangkan) tetap dirender apa adanya oleh viewer.
fn baris_berstempel(pesan: &str) -> String {
    let sekarang = time::OffsetDateTime::now_utc();
    format!(
        "{:02}:{:02}:{:02} | {pesan}",
        sekarang.hour(),
        sekarang.minute(),
        sekarang.second()
    )
}

/// Tulis satu baris log berstempel waktu kalau sesi log terbuka. Tidak
/// melakukan apa pun kalau `writer` `None` — kegagalan membuka sesi log
/// (`logs::writer::mulai`) TIDAK PERNAH membatalkan deploy (invariant §3
/// no.1); ini titik tunggal yang menegakkannya di seluruh `engine.rs`.
async fn catat(writer: &mut Option<LogWriter>, pool_tulis: &SqlitePool, pesan: &str) {
    if let Some(w) = writer {
        w.tulis(pool_tulis, &baris_berstempel(pesan)).await;
    }
}

/// Entry point dipanggil `worker::deploy_worker` untuk SATU job. Tidak
/// pernah panik keluar — semua kegagalan ditangkap sebagai
/// `DeployKegagalan` dan dipersist, lock SELALU dilepas di akhir apa pun
/// hasilnya.
pub async fn jalankan_deploy(state: AppState, deployment_id: String) {
    // Sesi log dibuka di awal, DITUTUP di titik yang sama dengan
    // `deployment_events.remove` di bawah — supaya tidak ada jalur keluar
    // (sukses maupun gagal) yang melewatkan penutupan (`docs/plan.md`
    // "Alur log deploy" langkah 1 & 4).
    let mut writer = match logs::writer::mulai(
        &state.db_write,
        &state.logs,
        &state.config.log_dir,
        &deployment_id,
    )
    .await
    {
        Ok(w) => Some(w),
        Err(err) => {
            tracing::warn!(
                error = %err,
                deployment_id,
                "gagal membuka sesi log deploy; deploy lanjut tanpa file log"
            );
            None
        }
    };

    let (heartbeat_stop_tx, mut heartbeat_stop_rx) = tokio::sync::watch::channel(false);
    let (lease_lost_tx, mut lease_lost_rx) = tokio::sync::watch::channel(false);
    let heartbeat_state = state.clone();
    let heartbeat_id = deployment_id.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match deployments_repo::heartbeat(&heartbeat_state.db_write, &heartbeat_id).await {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::warn!(deployment_id = %heartbeat_id, "heartbeat kehilangan lock deployment");
                            let _ = lease_lost_tx.send(true);
                            break;
                        }
                        Err(err) => tracing::warn!(error = %err, deployment_id = %heartbeat_id, "gagal memperbarui heartbeat deployment"),
                    }
                }
                _ = heartbeat_stop_rx.changed() => {
                    if *heartbeat_stop_rx.borrow() {
                        break;
                    }
                }
            }
        }
    });

    // Jangan membatalkan future deployment secara paksa saat lease hilang:
    // future yang diputus dapat melewati cleanup forward/session/container.
    // Worker memeriksa sinyal di batas aman dan mengembalikan error kooperatif;
    // jalur `jalankan_deploy_inner` tetap menutup resource remote.
    let hasil =
        jalankan_deploy_inner(&state, &deployment_id, &mut writer, &mut lease_lost_rx).await;
    let hasil = if *lease_lost_rx.borrow() {
        let sudah_live = deployments_repo::find_by_id(&state.db_read, &deployment_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|dep| dep.status == StatusDeployment::Live);
        if sudah_live {
            hasil
        } else {
            Err(DeployKegagalan::Lain(
                "lease deployment hilang; operasi dihentikan untuk mencegah perubahan tumpang tindih"
                    .to_string(),
            ))
        }
    } else {
        hasil
    };
    let _ = heartbeat_stop_tx.send(true);
    if let Err(err) = heartbeat_task.await {
        tracing::warn!(error = %err, deployment_id, "heartbeat task tidak berhenti dengan bersih");
    }

    match &hasil {
        Ok(()) => {
            catat(&mut writer, &state.db_write, "deploy selesai, status live").await;
            enqueue_deployment_notification(
                &state,
                &deployment_id,
                notifications::EVENT_DEPLOYMENT_RECOVERED,
                "live",
                None,
            )
            .await;
            state.deployment_events.publish(
                &deployment_id,
                DeploymentEvent {
                    status: StatusDeployment::Live,
                    pesan: None,
                },
            );
        }
        Err(kegagalan) => {
            catat(
                &mut writer,
                &state.db_write,
                &format!("deploy gagal: {}", kegagalan.pesan()),
            )
            .await;
            if let Err(err) = deployments_repo::mark_failed(
                &state.db_write,
                &deployment_id,
                kegagalan.kind(),
                &kegagalan.pesan(),
            )
            .await
            {
                tracing::warn!(error = %err, deployment_id, "gagal tandai deployment gagal");
            }
            enqueue_deployment_notification(
                &state,
                &deployment_id,
                notifications::EVENT_DEPLOYMENT_FAILED,
                "failed",
                Some(kegagalan.kind()),
            )
            .await;
            state.deployment_events.publish(
                &deployment_id,
                DeploymentEvent {
                    status: StatusDeployment::Failed,
                    pesan: Some(kegagalan.pesan()),
                },
            );
        }
    }

    if let Ok(Some(dep)) = deployments_repo::find_by_id(&state.db_read, &deployment_id).await
        && let Err(err) =
            apps_repo::release_lock(&state.db_write, &dep.app_id, &deployment_id).await
    {
        tracing::warn!(error = %err, deployment_id, "gagal lepas lock app setelah deploy");
    }

    if let Some(w) = writer.take() {
        w.tutup(&state.db_write).await;
    }
    state.deployment_events.remove(&deployment_id);
}

async fn enqueue_deployment_notification(
    state: &AppState,
    deployment_id: &str,
    event_type: &str,
    status: &str,
    error_kind: Option<&str>,
) {
    let Ok(Some(deployment)) = deployments_repo::find_by_id(&state.db_read, deployment_id).await
    else {
        tracing::warn!(
            deployment_id,
            "deployment tidak ditemukan saat membuat notifikasi"
        );
        return;
    };
    if event_type == notifications::EVENT_DEPLOYMENT_RECOVERED {
        let Ok(Some(_previous)) =
            deployments_repo::find_current_live(&state.db_read, &deployment.app_id, deployment_id)
                .await
        else {
            // Deploy sukses pertama bukan recovery; event recovery hanya
            // berarti ada deployment live sebelumnya yang pulih/tergantikan.
            return;
        };
    }
    let occurred_at = time::OffsetDateTime::now_utc().unix_timestamp();
    let envelope = WebhookEnvelope {
        event_id: deployment_id,
        event_type,
        occurred_at,
        data: serde_json::json!({
            "deployment_id": deployment_id,
            "app_id": deployment.app_id,
            "status": status,
            "error_kind": error_kind,
        }),
    };
    let payload = match serde_json::to_string(&envelope) {
        Ok(payload) => payload,
        Err(err) => {
            tracing::warn!(error = %err, deployment_id, "gagal serialisasi payload notifikasi deployment");
            return;
        }
    };
    if let Err(err) = notifications::repo::enqueue(
        &state.db_write,
        &deployments_repo::generate_id(),
        deployment_id,
        event_type,
        Some(&deployment.app_id),
        &payload,
    )
    .await
    {
        tracing::warn!(error = %err, deployment_id, event_type, "gagal memasukkan notifikasi deployment");
    }
}

async fn jalankan_deploy_inner(
    state: &AppState,
    deployment_id: &str,
    writer: &mut Option<LogWriter>,
    lease_lost: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), DeployKegagalan> {
    let dep = muat(
        deployments_repo::find_by_id(&state.db_read, deployment_id).await,
        "deployment tidak ditemukan",
    )?;
    let app = muat(
        apps_repo::find_by_id(&state.db_read, &dep.app_id).await,
        "app tidak ditemukan",
    )?;
    let server = muat(
        servers_repo::find_by_id(&state.db_read, &app.server_id).await,
        "server tidak ditemukan",
    )?;

    let Some(fingerprint) = server.host_key_fingerprint.clone() else {
        return Err(DeployKegagalan::Lain(
            "server belum terverifikasi (tanpa fingerprint host key tersimpan)".to_string(),
        ));
    };
    let plaintext_key = state
        .crypto
        .decrypt(&server.ssh_key_encrypted)
        .map_err(|err| DeployKegagalan::Lain(err.to_string()))?;

    publish_dan_set(state, deployment_id, StatusDeployment::Pulling, writer).await;

    catat(
        writer,
        &state.db_write,
        "menyambung ke server target lewat SSH",
    )
    .await;
    cek_lease_deploy(lease_lost)?;
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
            return Err(DeployKegagalan::Lain(
                "status host key tidak konsisten saat deploy".to_string(),
            ));
        }
        Err(err) => return Err(DeployKegagalan::Lain(format!("koneksi SSH gagal: {err:?}"))),
    };

    catat(writer, &state.db_write, "membuka forward socket docker").await;
    let forward = match docker::establish(&session, &state.config.runtime_dir, &server.id).await {
        Ok(forward) => forward,
        Err(err) => {
            let _ = session.close().await;
            return Err(DeployKegagalan::Lain(format!(
                "gagal membuka forward socket docker: {err:?}"
            )));
        }
    };

    let hasil = jalankan_docker(
        state,
        deployment_id,
        &dep,
        &app,
        &server.id,
        &session,
        &forward,
        writer,
        lease_lost,
    )
    .await;

    docker::close(&session, forward).await;
    let _ = session.close().await;

    hasil
}

fn cek_lease_deploy(
    lease_lost: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), DeployKegagalan> {
    if lease_lost.has_changed().unwrap_or(false) {
        let _ = lease_lost.borrow_and_update();
    }
    if *lease_lost.borrow() {
        return Err(DeployKegagalan::Lain(
            "lease deployment hilang; operasi dihentikan untuk mencegah perubahan tumpang tindih"
                .to_string(),
        ));
    }
    Ok(())
}

fn muat<T>(hasil: anyhow::Result<Option<T>>, pesan_kosong: &str) -> Result<T, DeployKegagalan> {
    match hasil {
        Ok(Some(nilai)) => Ok(nilai),
        Ok(None) => Err(DeployKegagalan::Lain(pesan_kosong.to_string())),
        Err(err) => Err(DeployKegagalan::Lain(err.to_string())),
    }
}

async fn publish_dan_set(
    state: &AppState,
    deployment_id: &str,
    status: StatusDeployment,
    writer: &mut Option<LogWriter>,
) {
    catat(
        writer,
        &state.db_write,
        &format!("tahap: {}", status.as_db_str()),
    )
    .await;
    if let Err(err) = deployments_repo::set_status(&state.db_write, deployment_id, status).await {
        tracing::warn!(error = %err, deployment_id, "gagal set status deployment");
    }
    state.deployment_events.publish(
        deployment_id,
        DeploymentEvent {
            status,
            pesan: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
async fn jalankan_docker(
    state: &AppState,
    deployment_id: &str,
    dep: &DeploymentRingkas,
    app: &AppRingkas,
    server_id: &str,
    session: &SshSession,
    forward: &docker::DockerForward,
    writer: &mut Option<LogWriter>,
    lease_lost: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), DeployKegagalan> {
    let client = docker::connect(forward.socket_path()).map_err(|_| {
        DeployKegagalan::Lain("gagal menyambung ke docker lewat socket forward".to_string())
    })?;

    pastikan_traefik(&client).await?;

    // Env TIDAK PERNAH dicatat (invariant §3 no.9 — nol isi env di baris
    // log), hanya JUMLAH variabel yang dicatat lewat `catat()` di bawah.
    let env = resolve_env(state, dep.env_version_id.as_deref()).await?;

    let credentials = resolve_credentials(state, server_id, &dep.image_digest).await;
    // Kredensial registry TIDAK PERNAH masuk baris log — hanya digest image
    // (bukan secret) yang dicatat, bukan `credentials` itu sendiri.
    catat(
        writer,
        &state.db_write,
        &format!("menarik image {}", dep.image_digest),
    )
    .await;

    docker::pull_image(&client, &dep.image_digest, credentials)
        .await
        .map_err(|err| DeployKegagalan::PullGagal(format!("{err:?}")))?;
    cek_lease_deploy(lease_lost)?;
    catat(writer, &state.db_write, "image berhasil ditarik").await;

    publish_dan_set(state, deployment_id, StatusDeployment::Starting, writer).await;

    let container_name = format!("{}-{}", app.name, deployment_id);
    let domains = apps_repo::list_domains(&state.db_read, &app.id)
        .await
        .unwrap_or_default();

    let mut labels = vec![
        ("platform.app".to_string(), app.name.clone()),
        ("platform.deployment".to_string(), deployment_id.to_string()),
        ("platform.digest".to_string(), dep.image_digest.clone()),
        ("traefik.enable".to_string(), "true".to_string()),
        (
            format!(
                "traefik.http.services.{}.loadbalancer.server.port",
                app.name
            ),
            app.port.to_string(),
        ),
        (
            format!(
                "traefik.http.services.{}.loadbalancer.healthcheck.path",
                app.name
            ),
            app.health_path.clone(),
        ),
        (
            format!(
                "traefik.http.services.{}.loadbalancer.healthcheck.interval",
                app.name
            ),
            "2s".to_string(),
        ),
    ];
    if let Some(domain) = domains.first() {
        labels.push((
            format!("traefik.http.routers.{}.rule", app.name),
            format!("Host(`{}`)", domain.host),
        ));
    }

    let container_id = docker::create_container(
        &client,
        docker::NewContainer {
            name: &container_name,
            image_ref: &dep.image_digest,
            network: DOCKER_NETWORK,
            labels: &labels,
            env: &env,
        },
    )
    .await
    .map_err(|err| DeployKegagalan::Lain(format!("gagal membuat container: {err:?}")))?;
    catat(
        writer,
        &state.db_write,
        &format!(
            "container {container_name} dibuat ({} variabel environment)",
            env.len()
        ),
    )
    .await;

    if let Err(err) =
        deployments_repo::set_container_id(&state.db_write, deployment_id, &container_id).await
    {
        tracing::warn!(error = %err, deployment_id, "gagal simpan container_id");
    }

    if let Err(err) = cek_lease_deploy(lease_lost) {
        let _ = docker::remove_container(&client, &container_id).await;
        return Err(err);
    }
    if let Err(err) = docker::start_container(&client, &container_id).await {
        let _ = docker::remove_container(&client, &container_id).await;
        return Err(DeployKegagalan::Lain(format!(
            "gagal start container: {err:?}"
        )));
    }
    if let Err(err) = cek_lease_deploy(lease_lost) {
        let _ = docker::remove_container(&client, &container_id).await;
        return Err(err);
    }
    catat(writer, &state.db_write, "container dimulai").await;

    publish_dan_set(state, deployment_id, StatusDeployment::Checking, writer).await;

    match jalankan_health_check(&client, &container_id, app, lease_lost).await {
        Ok(()) => {
            if let Err(err) = cek_lease_deploy(lease_lost) {
                let _ = docker::remove_container(&client, &container_id).await;
                return Err(err);
            }
            let owned =
                match deployments_repo::mark_live_if_owned(&state.db_write, deployment_id).await {
                    Ok(owned) => owned,
                    Err(err) => {
                        let _ = docker::remove_container(&client, &container_id).await;
                        return Err(DeployKegagalan::Lain(format!(
                            "gagal handoff deployment live: {err}"
                        )));
                    }
                };
            if !owned {
                let _ = docker::remove_container(&client, &container_id).await;
                return Err(DeployKegagalan::Lain(
                    "lease deployment hilang sebelum handoff live".to_string(),
                ));
            }
            catat(writer, &state.db_write, "health check lulus").await;
            // Handoff live sudah dikomit secara atomik. Setelah titik ini,
            // cleanup housekeeping best-effort tidak boleh membatalkan
            // container baru hanya karena heartbeat lease berhenti.
            // File audit env di target ditulis SETELAH terbukti sehat, BUKAN
            // sebelum create_container — supaya kalau deploy gagal, file di
            // target tetap merefleksikan env yang BENAR-BENAR jalan
            // (deployment lama), bukan percobaan yang gagal (invariant §3
            // no.1 "kegagalan tidak boleh memperburuk keadaan", diterapkan
            // ke file audit juga). Ini sekaligus mekanisme "hapus env lama"
            // PRD Fase 4 — satu file per app yang SELALU ditimpa nilai
            // terbaru yang benar-benar live, jadi tidak pernah ada file
            // basi dari deployment yang gagal.
            if let Err(detail) = tulis_env_file_target(session, &app.name, &env).await {
                tracing::warn!(
                    deployment_id,
                    detail,
                    "gagal menulis file audit env ke target (env tetap sampai ke container lewat API docker, ini cuma file bantu operator)"
                );
            }
            drain_container_lama(state, &client, &app.id, deployment_id, writer).await;
            if let Err(detail) = hapus_env_file_target(session, &app.name).await {
                tracing::warn!(
                    deployment_id,
                    detail,
                    "gagal menghapus file audit env dari target setelah pergantian container"
                );
            }
            Ok(())
        }
        Err(kegagalan) => {
            // Invariant §3 no.7: tangkap log SEBELUM hapus, tanpa kecuali —
            // untuk SEMUA mode kegagalan, bukan hanya container exited. Fase 3
            // HANYA menambah tujuan tulis kedua (file log) — urutan tangkap
            // (baris ini) SEBELUM `remove_container` (di bawah) TIDAK
            // digeser sama sekali.
            let _ = docker::container_logs(&client, &container_id, LOG_TAIL_LINES).await;
            tracing::info!(
                deployment_id,
                "log container ditangkap sebelum container dihapus; isi tidak disimpan karena dapat memuat secret"
            );
            catat(
                writer,
                &state.db_write,
                "health check gagal; log container ditangkap tetapi tidak disimpan karena dapat memuat secret",
            )
            .await;

            let kegagalan_final = match kegagalan {
                DeployKegagalan::ContainerExited { exit_code } => {
                    DeployKegagalan::ContainerExited { exit_code }
                }
                lain => lain,
            };

            let _ = docker::remove_container(&client, &container_id).await;
            Err(kegagalan_final)
        }
    }
}

/// Bootstrap Traefik lazy (`docs/plan.md` Q2) — cek label
/// `docker::TRAEFIK_LABEL` TIAP deploy (murah, satu `list_containers`),
/// pasang kalau belum ada. Kegagalan bootstrap menggagalkan deploy dengan
/// pesan eksplisit, bukan diam-diam dilewati — app baru tidak ada gunanya
/// kalau tidak ada yang meneruskan traffic ke dia.
async fn pastikan_traefik(client: &bollard::Docker) -> Result<(), DeployKegagalan> {
    let sudah_ada = docker::container_exists_with_label(client, docker::TRAEFIK_LABEL)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("cek container traefik gagal: {err:?}")))?;
    if sudah_ada {
        return Ok(());
    }

    docker::ensure_network(client, DOCKER_NETWORK)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("gagal siapkan network platform: {err:?}")))?;

    docker::pull_image(client, docker::TRAEFIK_IMAGE_TAG, None)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("gagal tarik image traefik: {err:?}")))?;

    let digest = docker::resolve_image_digest(client, docker::TRAEFIK_IMAGE_TAG)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("gagal resolusi digest traefik: {err:?}")))?;

    let container_id = docker::create_traefik_container(client, &digest, DOCKER_NETWORK)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("gagal buat container traefik: {err:?}")))?;

    docker::start_container(client, &container_id)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("gagal start container traefik: {err:?}")))
}

/// Hentikan container LAMA (deployment `live` sebelumnya untuk app yang
/// sama) setelah container baru terbukti sehat. Kegagalan drain TIDAK
/// menggagalkan deploy — container baru sudah live dan melayani, container
/// lama yang gagal berhenti bersih adalah masalah housekeeping, bukan
/// alasan menandai deployment ini gagal (invariant 1 sudah terjaga di titik
/// ini).
async fn drain_container_lama(
    state: &AppState,
    client: &bollard::Docker,
    app_id: &str,
    deployment_id: &str,
    writer: &mut Option<LogWriter>,
) {
    let Ok(Some(lama)) =
        deployments_repo::find_current_live(&state.db_read, app_id, deployment_id).await
    else {
        return;
    };
    let Some(container_lama) = &lama.container_id else {
        return;
    };

    catat(writer, &state.db_write, "stop container lama (drain)").await;

    if let Err(err) = docker::stop_container(client, container_lama, DRAIN_GRACE_SECS).await {
        tracing::warn!(
            error = ?err,
            deployment_id,
            container_lama,
            "gagal stop container lama (dilanjutkan — container baru sudah live)"
        );
    }
}

/// Dekripsi snapshot env yang dirujuk `env_version_id`, kembalikan
/// `(KEY, value)` siap dipakai `ContainerCreateBody.env`. `None` (app belum
/// pernah punya env) → vec kosong, BUKAN error. Kegagalan dekripsi/parse
/// (kunci salah, snapshot korup) MENGGAGALKAN deploy dengan pesan jelas
/// (`docs/prd.md` Fase 4 baris Debugger: "kegagalan dekripsi memberi pesan
/// jelas, bukan panic") — silently melanjutkan tanpa env bisa membuat app
/// start dengan config kosong yang membingungkan untuk didiagnosis.
async fn resolve_env(
    state: &AppState,
    env_version_id: Option<&str>,
) -> Result<Vec<(String, String)>, DeployKegagalan> {
    let Some(id) = env_version_id else {
        return Ok(Vec::new());
    };

    let snapshot_encrypted = apps_repo::find_env_version_snapshot(&state.db_read, id)
        .await
        .map_err(|err| DeployKegagalan::Lain(format!("baca snapshot env: {err}")))?
        .ok_or_else(|| {
            DeployKegagalan::Lain("snapshot env yang dirujuk tidak ditemukan".to_string())
        })?;

    let plaintext = state.crypto.decrypt(&snapshot_encrypted).map_err(|err| {
        DeployKegagalan::Lain(format!(
            "gagal dekripsi environment tersimpan: {err} — kemungkinan kunci enkripsi salah atau berubah"
        ))
    })?;

    let map: std::collections::BTreeMap<String, String> = serde_json::from_str(&plaintext)
        .map_err(|err| DeployKegagalan::Lain(format!("snapshot env korup: {err}")))?;

    Ok(map.into_iter().collect())
}

/// Tulis file audit env ke server target — BUKAN jalur env sampai ke
/// proses container (itu `ContainerCreateBody.env`, `docs/plan.md` Q1
/// opsi A), murni bantuan operator (`docker exec`/inspeksi manual). Best
/// effort: kegagalan di sini TIDAK PERNAH menggagalkan deploy (pemanggil
/// hanya `tracing::warn!`). `install -D -m 0600` sekaligus `mkdir -p`
/// parent dan set mode dalam satu perintah.
async fn tulis_env_file_target(
    session: &SshSession,
    app_name: &str,
    env: &[(String, String)],
) -> Result<(), String> {
    let isi: String = env.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    let path = format!("{ENV_FILE_DIR}/{app_name}.env");

    let result = ssh::exec_with_stdin(
        session,
        "install",
        &["-D", "-m", "0600", "/dev/stdin", &path],
        isi.as_bytes(),
        ENV_FILE_WRITE_TIMEOUT,
    )
    .await
    .map_err(|err| format!("{err:?}"))?;

    if result.success() {
        Ok(())
    } else {
        Err(result.stderr)
    }
}

async fn hapus_env_file_target(session: &SshSession, app_name: &str) -> Result<(), String> {
    let path = format!("{ENV_FILE_DIR}/{app_name}.env");
    let result = ssh::exec(session, "rm", &["-f", &path], ENV_FILE_WRITE_TIMEOUT)
        .await
        .map_err(|err| format!("{err:?}"))?;
    if result.success() {
        Ok(())
    } else {
        Err(result.stderr)
    }
}

async fn jalankan_health_check(
    client: &bollard::Docker,
    container_id: &str,
    app: &AppRingkas,
    lease_lost: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), DeployKegagalan> {
    let batas =
        Duration::from_secs((app.health_grace_secs + HEALTH_EXTRA_THRESHOLD_SECS).max(0) as u64);
    let mulai = tokio::time::Instant::now();

    loop {
        cek_lease_deploy(lease_lost)?;
        let status = docker::inspect(client, container_id, DOCKER_NETWORK)
            .await
            .map_err(|err| DeployKegagalan::Lain(format!("{err:?}")))?;

        if !status.running {
            return Err(DeployKegagalan::ContainerExited {
                exit_code: status.exit_code.unwrap_or(-1),
            });
        }

        if let Some(ip) = &status.ip_address {
            match cek_http(ip, app.port, &app.health_path).await {
                Ok(()) => return Ok(()),
                Err(HealthCekError::NonSukses(kode, body)) if mulai.elapsed() >= batas => {
                    return Err(DeployKegagalan::HealthNon2xx { status: kode, body });
                }
                Err(_) => {} // belum lewat batas, coba lagi
            }
        }

        if mulai.elapsed() >= batas {
            return Err(DeployKegagalan::HealthNoResponse);
        }

        tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
    }
}

enum HealthCekError {
    NonSukses(u16, String),
    TidakMerespons,
}

/// Satu request health check ke IP container LANGSUNG (invariant 14 —
/// TIDAK PERNAH lewat domain publik/proxy). Klien `hyper` polos tanpa TLS
/// (`HttpConnector`, bukan `HttpsConnector`) — jaringan docker internal,
/// tidak butuh dan tidak boleh menambah kompleksitas TLS di sini.
async fn cek_http(ip: &str, port: i64, path: &str) -> Result<(), HealthCekError> {
    let uri: hyper::Uri = format!("http://{ip}:{port}{path}")
        .parse()
        .map_err(|_| HealthCekError::TidakMerespons)?;

    let client: Client<HttpConnector, Empty<Bytes>> =
        Client::builder(TokioExecutor::new()).build(HttpConnector::new());
    let req = Request::get(uri)
        .body(Empty::<Bytes>::new())
        .map_err(|_| HealthCekError::TidakMerespons)?;

    match tokio::time::timeout(HEALTH_REQUEST_TIMEOUT, client.request(req)).await {
        Ok(Ok(resp)) => {
            let status = resp.status();
            if status.is_success() {
                Ok(())
            } else {
                let kode = status.as_u16();
                let body_bytes = resp
                    .into_body()
                    .collect()
                    .await
                    .map(|collected| collected.to_bytes())
                    .unwrap_or_default();
                let body: String = String::from_utf8_lossy(&body_bytes)
                    .chars()
                    .take(500)
                    .collect();
                Err(HealthCekError::NonSukses(kode, body))
            }
        }
        Ok(Err(_)) | Err(_) => Err(HealthCekError::TidakMerespons),
    }
}

/// Cocokkan host registry dari referensi image ke registry yang SUDAH
/// login di server ini (Fase 1 `server_registries`). `None` untuk image
/// publik atau registry yang tidak dikenal (pull dicoba tanpa kredensial —
/// akan gagal wajar kalau ternyata privat, bukan ditebak-tebak).
async fn resolve_credentials(
    state: &AppState,
    server_id: &str,
    image_ref: &str,
) -> Option<DockerCredentials> {
    let host = extract_registry_host(image_ref)?;
    let registry = registries::repo::find_for_server_by_host(&state.db_read, server_id, host)
        .await
        .ok()??;
    let password = state.crypto.decrypt(&registry.token_encrypted).ok()?;

    Some(DockerCredentials {
        username: Some(registry.username),
        password: Some(password),
        serveraddress: Some(registry.host),
        ..Default::default()
    })
}

/// Ekstrak host registry dari referensi image (`ghcr.io/org/app@sha256:...`
/// → `ghcr.io`). Heuristik sama seperti Docker CLI: segmen pertama sebelum
/// `/` dianggap host HANYA kalau memuat `.` atau `:` — kalau tidak, itu
/// nama organisasi Docker Hub, bukan host (`org/app` → Docker Hub, image
/// publik, tidak butuh kredensial).
fn extract_registry_host(image_ref: &str) -> Option<&str> {
    let (segmen_pertama, _) = image_ref.split_once('/')?;
    if segmen_pertama.contains('.') || segmen_pertama.contains(':') {
        Some(segmen_pertama)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `catat` dengan `writer` `None` (sesi log gagal dibuka) harus diam saja,
    /// bukan panik — inilah yang menjaga deploy tetap jalan tanpa file log
    /// (invariant §3 no.1).
    #[tokio::test]
    async fn catat_tanpa_sesi_log_tidak_panik() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("pool memori uji harus terbuka");
        let mut writer: Option<LogWriter> = None;

        catat(&mut writer, &pool, "tahap apa pun").await;

        assert!(writer.is_none());
    }

    #[tokio::test]
    async fn cek_lease_menolak_sinyal_lease_hilang() {
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        tx.send(true).expect("sinyal lease test harus terkirim");

        let hasil = cek_lease_deploy(&mut rx);

        assert!(hasil.is_err());
        assert!(
            hasil
                .expect_err("lease hilang harus menjadi kegagalan")
                .pesan()
                .contains("lease deployment hilang")
        );
    }

    #[test]
    fn baris_berstempel_memakai_format_gutter_jam_menit_detik() {
        let baris = baris_berstempel("container dimulai");

        let (stempel, sisa) = baris
            .split_once(" | ")
            .expect("baris log wajib berformat 'HH:MM:SS | pesan'");
        assert_eq!(sisa, "container dimulai");
        assert_eq!(stempel.len(), 8, "stempel: {stempel}");
        let bagian: Vec<&str> = stempel.split(':').collect();
        assert_eq!(bagian.len(), 3);
        for b in bagian {
            assert_eq!(b.len(), 2);
            assert!(b.parse::<u8>().is_ok(), "bagian stempel: {b}");
        }
    }

    #[test]
    fn extract_registry_host_mengenali_host_dengan_titik() {
        assert_eq!(
            extract_registry_host("ghcr.io/org/app@sha256:abc"),
            Some("ghcr.io")
        );
    }

    #[test]
    fn extract_registry_host_mengenali_host_dengan_port() {
        assert_eq!(
            extract_registry_host("registry.internal:5000/app@sha256:abc"),
            Some("registry.internal:5000")
        );
    }

    #[test]
    fn extract_registry_host_none_untuk_docker_hub_tanpa_host_eksplisit() {
        assert_eq!(extract_registry_host("org/app@sha256:abc"), None);
        assert_eq!(extract_registry_host("nginx@sha256:abc"), None);
    }

    #[test]
    fn setiap_kategori_deploy_kegagalan_punya_kind_dan_pesan_tidak_kosong() {
        let kegagalan = [
            DeployKegagalan::PullGagal("detail".to_string()),
            DeployKegagalan::ContainerExited { exit_code: 1 },
            DeployKegagalan::HealthNon2xx {
                status: 500,
                body: "err".to_string(),
            },
            DeployKegagalan::HealthNoResponse,
            DeployKegagalan::Lain("detail".to_string()),
        ];

        for k in &kegagalan {
            assert!(!k.kind().is_empty());
            assert!(!k.pesan().is_empty());
        }
        let pesan = DeployKegagalan::ContainerExited { exit_code: 1 }.pesan();
        assert!(!pesan.contains("log rahasia"));
    }
}
