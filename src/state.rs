//! `AppState` yang di-share ke semua handler lewat `axum::extract::State`.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::crypto::CryptoKey;
use crate::events::{DeploymentEvent, EventRegistry, VerificationEvent};
use crate::logs::LogRegistry;

/// State bersama aplikasi. `Clone` murah karena semua isinya `Arc`/pool
/// (pool sqlx internalnya sudah `Arc`).
///
/// Sengaja tidak derive `Debug` — `Config` bisa memuat `initial_password`
/// dan `crypto` memegang private key `age` (keduanya secret); mencegah
/// kebocoran diam-diam lewat log/debug print di masa depan (invariant 7).
#[derive(Clone)]
pub struct AppState {
    pub db_write: SqlitePool,
    pub db_read: SqlitePool,
    pub config: Arc<Config>,
    pub crypto: Arc<CryptoKey>,
    /// Progres verifikasi server in-memory (SSE) — lihat `src/events.rs`.
    pub events: Arc<EventRegistry<VerificationEvent>>,
    /// Progres timeline deployment in-memory (SSE) — namespace job_id
    /// TERPISAH dari `events` (id deployment vs id server).
    pub deployment_events: Arc<EventRegistry<DeploymentEvent>>,
    /// Sesi broadcast log (deploy dan runtime) — lihat `src/logs/registry.rs`
    /// untuk alasan ini bukan `EventRegistry`.
    pub logs: Arc<LogRegistry>,
    /// Progres operasi armada satu kali untuk SSE, dibersihkan setelah operasi selesai.
    pub fleet_events: Arc<EventRegistry<crate::fleet::FleetEvent>>,
}
