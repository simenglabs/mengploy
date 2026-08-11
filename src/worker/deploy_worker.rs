//! Worker job deploy — LOOP TERPISAH dari `status_poll` (`docs/plan.md`:
//! "polling = tick 30 detik banyak server ringan; deploy = event-driven,
//! satu app pada satu waktu"). Tick pendek dipakai sebagai pengganti
//! notifikasi event murni — job deploy jarang, tick murah, dan tetap
//! berfungsi sebagai jaring pengaman kalau sinyal enqueue pernah terlewat.
//!
//! Satu job diklaim = satu `tokio::spawn` `deployments::engine::jalankan_deploy`
//! yang TIDAK ditunggu (`jobs.status` selesai begitu deploy di-spawn — lock
//! per app, bukan status job, yang mencegah dua deploy app yang sama
//! tumpang tindih).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, watch};

use crate::deployments::{self, DeployJobPayload};
use crate::jobs;
use crate::state::AppState;

use super::WorkerHandle;

/// Ponytail: 2 detik, bukan notifikasi `tokio::sync::Notify` — job deploy
/// tidak butuh respons sub-detik (pull image sendiri makan puluhan detik).
/// Upgrade ke event-driven murni kalau tick ini pernah jadi bottleneck nyata.
const TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Batas deploy PARALEL di seluruh armada (security review Fase 2 Q3) —
/// tiap deploy men-spawn koneksi SSH + docker pull, tanpa batas ini
/// `POST /api/v1/deploy` yang dipanggil banyak pipeline CI sekaligus bisa
/// menghabiskan resource control plane sendiri (fd, memori stream pull).
/// Job kelebihan MENUNGGU giliran (`Semaphore::acquire_owned`), bukan
/// ditolak — lock per app sudah mencegah app yang SAMA tumpang tindih,
/// batas ini soal APP BERBEDA yang kebetulan deploy bersamaan.
const MAX_DEPLOY_KONKUREN: usize = 4;

async fn jalankan_satu_siklus(state: &AppState, semaphore: &Arc<Semaphore>) {
    loop {
        let job = match jobs::repo::claim_next(&state.db_write, jobs::KIND_DEPLOY).await {
            Ok(job) => job,
            Err(err) => {
                tracing::warn!(error = %err, "gagal klaim job deploy");
                return;
            }
        };

        let Some(job) = job else {
            return;
        };

        let payload: DeployJobPayload = match serde_json::from_str(&job.payload_json) {
            Ok(payload) => payload,
            Err(err) => {
                tracing::error!(job_id = %job.id, error = %err, "payload job deploy rusak");
                let _ = jobs::repo::mark_failed(&state.db_write, &job.id, &err.to_string()).await;
                continue;
            }
        };

        // `acquire_owned` tidak pernah gagal — semaphore ini tidak pernah
        // di-`close()`. Permit dipegang TASK yang di-spawn, dilepas otomatis
        // begitu `jalankan_deploy` selesai (sukses atau gagal).
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore deploy tidak pernah ditutup");
        let state_clone = state.clone();
        tokio::spawn(async move {
            deployments::jalankan_deploy(state_clone, payload.deployment_id).await;
            drop(permit);
        });

        if let Err(err) = jobs::repo::mark_done(&state.db_write, &job.id).await {
            tracing::warn!(job_id = %job.id, error = %err, "gagal tandai job deploy selesai diklaim");
        }
    }
}

pub fn spawn(state: AppState) -> WorkerHandle {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let semaphore = Arc::new(Semaphore::new(MAX_DEPLOY_KONKUREN));

    let join_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    jalankan_satu_siklus(&state, &semaphore).await;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    });

    WorkerHandle {
        shutdown_tx,
        join_handle,
    }
}
