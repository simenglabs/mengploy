//! Domain aplikasi: `model` (view-model, aman dirender — `apps` tidak
//! punya kolom secret sama sekali), `repo` (persistensi + lock deploy per
//! app, invariant §3 no.12).

pub mod model;
pub mod repo;

pub use model::{AppRingkas, DeployTokenRingkas, DomainRingkas, EnvVersionRingkas};
pub use repo::NewApp;
