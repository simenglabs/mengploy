//! Domain deployment: `model` (state machine + view-model), `repo`
//! (persistensi), `engine` (mesin state penuh: pull → start → check → swap).

pub mod engine;
pub mod model;
pub mod reconciliation;
pub mod repo;
pub mod retention;

pub use engine::{LOCK_TTL_SECS, jalankan_deploy};
pub use model::{DeploymentRingkas, StatusDeployment};
pub use repo::NewDeployment;

/// Payload `jobs.payload_json` untuk `jobs::KIND_DEPLOY` — dibuat
/// `routes::deploy_api` (Fase 2g), dibaca `worker::deploy_worker`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct DeployJobPayload {
    pub deployment_id: String,
}
