//! Worker in-process satu tugas: polling status armada tiap 30 detik.
//! Di-spawn `main.rs` saat startup, dihentikan lewat token pembatalan saat
//! shutdown (`docs/plan.md` "Polling status dan backoff").

pub mod deploy_worker;
pub mod fleet;
pub mod log_retention;
pub mod metrics;
pub mod notification_delivery;
pub mod reconciliation;
pub mod status_poll;

use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::state::AppState;

/// Interval tick worker — usulan `docs/plan.md` (bukan interval server
/// sehat, itu `servers::verify::NORMAL_POLL_INTERVAL_SECS`; ini seberapa
/// sering worker BERTANYA "server mana yang jatuh tempo").
const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// Pegangan worker yang sedang berjalan — dipegang `main.rs` untuk
/// menghentikannya dengan bersih saat shutdown.
pub struct WorkerHandle {
    shutdown_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

impl WorkerHandle {
    /// Kirim sinyal berhenti dan tunggu siklus yang sedang berjalan
    /// (kalau ada) selesai dengan bersih.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        if let Err(err) = self.join_handle.await {
            tracing::warn!(error = %err, "worker polling tidak berhenti dengan bersih");
        }
    }
}

/// Mulai worker polling status. `state` di-`clone()` (murah — semua isinya
/// `Arc`/pool) supaya `main.rs` tetap memegang salinannya sendiri untuk
/// router.
pub fn spawn(state: AppState) -> WorkerHandle {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let boot = Instant::now();

    let join_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    status_poll::jalankan_satu_siklus(&state).await;
                    log_retention::jalankan_jika_jatuh_tempo(&state, boot).await;
                    state.logs.sapu_yatim();
                    fleet::sapu_output_lama(&state).await;
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
