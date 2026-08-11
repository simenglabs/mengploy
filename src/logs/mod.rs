//! Log deploy dan log runtime — lihat `docs/plan.md` Fase 3.
//!
//! Kerangka minimal sub-blok 3a: hanya deklarasi modul dan tipe dasar yang
//! sudah jelas dari rencana. Logika (registry channel, penulisan file,
//! pembacaan/pencarian, metadata db, retensi) adalah sub-blok 3b-3g dan
//! SENGAJA belum diimplementasikan di sini.

pub mod reader;
pub mod registry;
pub mod repo;
pub mod retention;
pub mod writer;

use std::sync::Arc;

pub use registry::LogRegistry;
pub use repo::LogMeta;
pub use writer::LogWriter;

/// Satu event pada sesi log broadcast — dipakai `LogRegistry` (sub-blok 3b).
/// Didefinisikan di sini sekarang karena bentuknya sudah dikunci
/// `docs/plan.md` bagian "logs::registry", tanpa mendahului implementasi
/// logikanya.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// Satu baris log, siap disiarkan ke subscriber.
    Baris(Arc<str>),
    /// Subscriber tertinggal `n` baris (`broadcast::error::RecvError::Lagged`).
    Tertinggal(u64),
    /// Sesi log berakhir — pemicu handler SSE menutup stream.
    Selesai,
}
