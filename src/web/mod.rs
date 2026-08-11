//! Lapisan tampilan (`src/web/**`): template Maud, `const CSS`, dan fungsi
//! render yang dipanggil handler di `src/routes/**`.
//!
//! Kontrak pemanggilan (signature) di bawah ini WAJIB dipertahankan persis
//! supaya `src/routes/**` (milik agent backend) tetap bisa dikompilasi:
//!
//! - `render_login(error: Option<&str>, csrf_token: &str) -> Markup`
//! - `render_dashboard(strip: Option<Markup>, csrf_token: &str, jumlah_server: usize) -> Markup`
//! - `render_404(strip: Option<Markup>) -> Markup`
//! - `render_500() -> Markup`
//! - `render_fleet(servers: &[ServerRingkas], csrf_token: &str, strip: Option<Markup>) -> Markup`
//! - `render_fleet_strip(servers: &[ServerRingkas]) -> Markup`
//! - `render_server_baru(csrf_token: &str, error: Option<&str>, strip: Option<Markup>) -> Markup`
//! - `render_verifikasi(server: &ServerRingkas, event: &VerificationEvent, csrf_token: &str, strip: Option<Markup>) -> Markup`
//! - `render_verifikasi_fragmen(server_id: &str, event: &VerificationEvent, csrf_token: &str) -> Markup`
//!   (menyimpang dari daftar minimum uiux — form konfirmasi TOFU hidup DI
//!   DALAM fragmen yang di-swap SSE, jadi butuh `csrf_token` juga)
//! - `render_registry_form(server: &ServerRingkas, registries: &[RegistryRingkas], csrf_token: &str, error: Option<&str>, strip: Option<Markup>) -> Markup`
//! - `render_server_detail(server: &ServerRingkas, registries_tertaut: &[RegistryRingkas], metrics: &MetricDashboard, range_hours: u32, csrf_token: &str, strip: Option<Markup>) -> Markup`
//!
//! Fase 3 (`src/web/logs.rs`, menggantikan `routes::logs::render_sementara`):
//!
//! - `render_deploy_log(dep: &DeploymentRingkas, truncated: bool, baris: &[LogLine], pencarian_dipotong: bool, q: Option<&str>, streaming: bool, csrf_token: &str, strip: Option<Markup>) -> Markup`
//! - `render_log_fragmen(baris: &[LogLine], truncated: bool, pencarian_dipotong: bool, selesai: bool) -> Markup`
//! - `render_log_pesan(pesan: &str) -> Markup`
//! - `render_app_tab_deployments(app: &AppRingkas, deploys: &[DeploymentRingkas], dipotong: bool, csrf_token: &str, strip: Option<Markup>) -> Markup`
//! - `render_app_tab_logs(app: &AppRingkas, ada_container: bool, csrf_token: &str, strip: Option<Markup>) -> Markup`
//!
//! `app_shell(csrf_token: Option<&str>, strip: Option<Markup>, content: Markup) -> Markup`
//! (`src/web/layout.rs`) BERUBAH dari Fase 0: menerima fleet strip sebagai
//! parameter kedua. Semua pemanggil lama (`dashboard.rs`, `error_page.rs`)
//! ikut diperbarui — lihat `docs/plan.md` "Kontrak render backend ↔
//! frontend".

mod apps;
mod dashboard;
pub mod deployments;
mod env;
mod error_page;
mod fleet;
mod fleet_actions;
mod fleet_strip;
mod layout;
mod login;
mod logs;
pub(crate) mod reconciliation;
mod server_add;
mod server_detail;
pub(crate) mod settings;
mod styles;

pub use apps::{render_app_baru, render_app_detail, render_apps};
pub use dashboard::render_dashboard;
pub use deployments::{render_deployment_detail, render_deployment_fragmen};
pub use env::{EnvDiff, EnvDiffKind, EnvVarTampil, render_app_tab_environment};
pub use error_page::{render_404, render_500};
pub use fleet::render_fleet;
pub use fleet_actions::render_fleet_actions;
pub use fleet_strip::render_fleet_strip;
pub use login::render_login;
pub use logs::{
    render_app_tab_deployments, render_app_tab_logs, render_deploy_log, render_log_fragmen,
    render_log_pesan,
};
pub use reconciliation::render_reconciliation;
pub use server_add::{
    render_registry_form, render_server_baru, render_verifikasi, render_verifikasi_fragmen,
};
pub use server_detail::render_server_detail;
pub use settings::render_notification_settings;
pub use styles::CSS;
