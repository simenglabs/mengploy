//! Library crate `mengdep`: deklarasi modul yang dipakai bersama oleh
//! `src/main.rs` (binary) dan `tests/` (integration test).
//!
//! `src/main.rs` tetap tipis — hanya urutan startup. Semua logika domain,
//! wiring Axum, dan query sqlx tinggal di modul-modul di bawah ini, persis
//! seperti sebelum `src/lib.rs` ada (lihat docs/plan.md "Struktur direktori
//! dan batas modul").
//!
//! `web` sengaja tidak `pub` — itu detail render internal (milik agent
//! frontend), bukan permukaan yang perlu diakses `tests/`.

pub mod apps;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod deployments;
pub mod docker;
pub mod error;
pub mod events;
pub mod fleet;
pub mod fleet_repo;
pub mod jobs;
pub mod logs;
pub mod metrics;
pub mod metrics_repo;
pub mod notifications;
pub mod registries;
pub mod routes;
pub mod servers;
pub mod ssh;
pub mod state;
mod web;
pub mod worker;
