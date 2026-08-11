//! Domain server: `model` (tipe view-model, aman dirender), `repo`
//! (persistensi `sqlx::query!`), `verify` (mesin verifikasi 3 langkah:
//! koneksi → Docker → registry).

pub mod model;
pub mod repo;
pub mod verify;

pub use model::{LangkahStatus, LangkahVerifikasi, ServerRingkas, StatusServer};
pub use repo::{NewServer, ServerRow};
pub use verify::{
    KonfirmasiHostkeyError, NORMAL_POLL_INTERVAL_SECS, RegistryStepError, RegistryStepInput,
    konfirmasi_hostkey_dan_lanjutkan, mulai_verifikasi, tautkan_registry,
};
