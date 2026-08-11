//! Registry broadcast channel in-memory generik — dipakai untuk progres
//! verifikasi server (`VerificationEvent`, Fase 1) DAN timeline deployment
//! (`DeploymentEvent`, Fase 2), lewat DUA instance terpisah
//! (`AppState.events` vs `AppState.deployment_events`) — namespace job_id
//! beda alam (id server vs id deployment), jadi TIDAK dicampur dalam satu
//! `HashMap` walau tipenya sekarang generik.
//!
//! TIDAK PERNAH persisten — progres ini bukan log runtime (invariant 9);
//! status akhir yang benar-benar dipercaya tetap kolom db (`servers.status`
//! / `deployments.status`, invariant 2). Channel ini murni dorongan UI.
//!
//! Dipakai `std::sync::Mutex` (bukan `dashmap`, yang tidak ada di
//! `Cargo.toml`) — jumlah job aktif sekaligus di instance ini kecil,
//! kontensi lock tidak relevan.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::deployments::model::StatusDeployment;
use crate::servers::model::LangkahVerifikasi;

/// Kapasitas buffer per channel — cukup untuk seluruh checklist/timeline
/// dikirim ulang beberapa kali tanpa ada subscriber tertinggal.
const CHANNEL_CAPACITY: usize = 32;

/// Satu snapshot checklist verifikasi server (Fase 1) untuk dipancarkan ke
/// SSE `GET /events/verifikasi/{server_id}`.
#[derive(Clone)]
pub struct VerificationEvent {
    pub langkah: Vec<LangkahVerifikasi>,
    /// `Some(fingerprint)` kalau checklist sedang menunggu konfirmasi TOFU
    /// pengguna (`docs/design/tambah-server.md` §4.2 poin 6) — fingerprint
    /// BUKAN secret, aman ditampilkan.
    pub tofu_pending_fingerprint: Option<String>,
}

/// Satu snapshot timeline deployment (Fase 2) untuk dipancarkan ke SSE
/// `GET /events/deploy/{deployment_id}`.
#[derive(Clone)]
pub struct DeploymentEvent {
    pub status: StatusDeployment,
    /// Pesan Bahasa Indonesia yang sudah dipetakan (kategori kegagalan
    /// health check, dsb) — TIDAK PERNAH log mentah (invariant 9).
    pub pesan: Option<String>,
}

/// Registry broadcast channel generik, satu job = satu key string.
#[derive(Default)]
pub struct EventRegistry<T> {
    channels: Mutex<HashMap<String, broadcast::Sender<T>>>,
}

impl<T: Clone> EventRegistry<T> {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
        }
    }

    /// Sambungkan (atau buat baru) channel untuk `job_id`. Dipanggil route
    /// SSE **dan** dipanggil pemula job SEBELUM task di-spawn, supaya
    /// subscriber yang menyambung tepat setelah submit tidak kehilangan
    /// channel-nya (event pertama tetap bisa terlewat kalau SSE belum
    /// sempat subscribe — bukan kebocoran data, halaman selalu merender
    /// snapshot awal dari db sebagai fallback).
    pub fn subscribe(&self, job_id: &str) -> broadcast::Receiver<T> {
        let mut channels = self.channels.lock().unwrap_or_else(|err| err.into_inner());
        channels
            .entry(job_id.to_string())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Pancarkan snapshot baru. Tidak ada subscriber aktif bukan error —
    /// pengguna mungkin belum/sudah menutup halaman.
    pub fn publish(&self, job_id: &str, event: T) {
        let channels = self.channels.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(sender) = channels.get(job_id) {
            let _ = sender.send(event);
        }
    }

    /// Buang channel setelah job selesai (sukses atau gagal) — mencegah
    /// `channels` bertumbuh tanpa batas (kelas kebocoran yang PRD tandai
    /// krusial untuk Fase 3 log streaming, dipraktikkan lebih awal di sini).
    pub fn remove(&self, job_id: &str) {
        let mut channels = self.channels.lock().unwrap_or_else(|err| err.into_inner());
        channels.remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::model::LangkahStatus;

    fn event_contoh() -> VerificationEvent {
        VerificationEvent {
            langkah: vec![LangkahVerifikasi {
                nama: "Membangun Koneksi SSH".to_string(),
                status: LangkahStatus::Berjalan,
                pesan: None,
            }],
            tofu_pending_fingerprint: None,
        }
    }

    #[test]
    fn subscribe_dua_kali_mengembalikan_channel_yang_sama() {
        let registry: EventRegistry<VerificationEvent> = EventRegistry::new();
        let mut rx1 = registry.subscribe("server-a");
        registry.publish("server-a", event_contoh());

        let event = rx1.try_recv().expect("subscriber harus menerima event");
        assert_eq!(event.langkah.len(), 1);
    }

    #[test]
    fn publish_tanpa_subscriber_tidak_panik() {
        let registry: EventRegistry<VerificationEvent> = EventRegistry::new();
        registry.publish("server-tanpa-subscriber", event_contoh());
    }

    #[test]
    fn remove_membuang_channel_subscriber_baru_setelahnya_tidak_dapat_event_lama() {
        let registry: EventRegistry<VerificationEvent> = EventRegistry::new();
        let _rx = registry.subscribe("server-b");
        registry.publish("server-b", event_contoh());
        registry.remove("server-b");

        let mut rx_baru = registry.subscribe("server-b");
        assert!(
            rx_baru.try_recv().is_err(),
            "channel baru setelah remove tidak boleh mewarisi event lama"
        );
    }

    #[test]
    fn registry_generik_bekerja_untuk_deployment_event_juga() {
        let registry: EventRegistry<DeploymentEvent> = EventRegistry::new();
        let mut rx = registry.subscribe("deploy-a");
        registry.publish(
            "deploy-a",
            DeploymentEvent {
                status: StatusDeployment::Pulling,
                pesan: None,
            },
        );
        let event = rx
            .try_recv()
            .expect("subscriber harus menerima event deployment");
        assert_eq!(event.status, StatusDeployment::Pulling);
    }
}
