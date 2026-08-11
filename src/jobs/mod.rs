//! Antrean job in-database — tanpa crate eksternal (CLAUDE.md §4).

pub mod repo;

pub use repo::Job;

/// Satu-satunya `kind` job Fase 2.
pub const KIND_DEPLOY: &str = "deploy";
