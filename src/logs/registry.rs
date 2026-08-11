//! Registry broadcast channel in-memory untuk log — KHUSUS log (deploy dan
//! runtime), bukan `crate::events::EventRegistry` yang dipakai untuk
//! checklist verifikasi (Fase 1) dan timeline deployment (Fase 2).
//!
//! Kenapa terpisah dari `EventRegistry<T>` (lihat `docs/plan.md` bagian
//! "`logs::registry` — lifetime broadcast channel"): `EventRegistry::subscribe`
//! **membuat** channel kalau belum ada, dan hanya `remove()` eksplisit yang
//! membuangnya. Itu aman untuk job verifikasi/deploy karena umurnya pendek dan
//! `remove()` selalu dipanggil tepat di akhir job. Untuk log ada dua jalur yang
//! tidak tercakup pola itu:
//!
//! 1. Klien membuka SSE untuk deployment yang **tidak** sedang berjalan —
//!    `subscribe()`-gaya-`EventRegistry` akan membuat channel kosong yang
//!    tidak pernah dibuang (tidak ada writer yang akan memanggil `remove()`).
//! 2. Writer selesai sementara subscriber masih menempel — `remove()` membuang
//!    entri map, tapi `Sender` tetap hidup selama ada receiver, dan tidak ada
//!    yang memberi tahu subscriber untuk berhenti menunggu.
//!
//! `LogRegistry` menutup jalur (1) secara struktural: **hanya** `mulai()`
//! (dipanggil writer) yang membuat sesi; `ikut()` (dipanggil pembaca/SSE)
//! HANYA menyambung ke sesi yang sudah ada, tidak pernah membuat. Jalur (2)
//! ditutup lewat `Weak` di map + `Drop` yang membuang entrinya sendiri: sesi
//! benar-benar hidup selama ada minimal satu `Arc<LogSession>` (writer ATAU
//! subscriber) yang memegangnya, dan lenyap dari map begitu `Arc` terakhir
//! drop — tidak ada channel yatim yang menumpuk (`docs/prd.md:291`, `:384`).
//!
//! `std::sync::Mutex` (bukan `dashmap`, tidak ada di `Cargo.toml`) — pola
//! sama `crate::events::EventRegistry`, jumlah sesi log aktif dibatasi keras
//! ([`MAX_SESSIONS`]) sehingga kontensi lock tidak relevan.
//!
//! TIDAK menyentuh SQLite sama sekali — murni in-memory (invariant 9).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::broadcast;

/// Kapasitas buffer broadcast per sesi log (`docs/plan.md`, "Angka yang
/// dikunci"). Subscriber yang tertinggal lebih dari ini menerima
/// `LogEvent::Tertinggal` (lewat `broadcast::error::RecvError::Lagged`),
/// bukan diam-diam kehilangan baris.
const CHANNEL_CAPACITY: usize = 256;

/// Batas sesi log aktif serentak (`docs/plan.md`, "Angka yang dikunci").
/// Dilewati → `mulai()` menolak; deploy tetap jalan, hanya tanpa streaming
/// langsung (log tetap tertulis ke file oleh writer).
const MAX_SESSIONS: usize = 64;

use super::LogEvent;

/// Registry sesi log. Satu instance melayani SATU alam (deploy ATAU
/// runtime) — `AppState` memegang `Arc<LogRegistry>` tunggal; pemanggil yang
/// menentukan namespace key (`deployment_id` untuk log deploy, `app_id`
/// untuk log runtime tidak akan pernah bertabrakan karena id keduanya
/// dari tabel berbeda dengan generator id yang sama, jadi secara praktik
/// hampir mustahil kolisi — kalau ini jadi masalah nyata, pisahkan jadi dua
/// instance seperti `events`/`deployment_events`).
#[derive(Default)]
pub struct LogRegistry {
    sessions: Mutex<HashMap<String, Weak<LogSession>>>,
}

/// Satu sesi broadcast log, dipegang lewat `Arc` oleh writer DAN oleh setiap
/// subscriber yang sedang menonton. Sesi hidup selama minimal satu `Arc`
/// beredar; begitu `Arc` terakhir drop, `Drop` membuang entrinya dari map
/// registry.
pub struct LogSession {
    key: String,
    tx: broadcast::Sender<LogEvent>,
    registry: Weak<LogRegistry>,
}

impl LogRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Mulai sesi log baru untuk `key`. **Hanya dipanggil writer** (mesin
    /// deploy / pembuka stream log runtime) — pembaca memakai [`ikut`].
    ///
    /// Menerima `self: &Arc<Self>` (bukan `&self`) karena `LogSession` perlu
    /// memegang `Weak<LogRegistry>` balik ke registry pemiliknya supaya
    /// `Drop` tahu map mana yang harus dibersihkan.
    ///
    /// Sesi lama (kalau ada, sisa job sebelumnya untuk key yang sama —
    /// seharusnya tidak pernah terjadi karena `deployment_id` sekali pakai,
    /// tapi tidak diasumsikan) **ditimpa** di map, bukan digabung. Sesi lama
    /// tetap hidup selama ada `Arc` yang memegangnya (mis. subscriber lama
    /// yang belum putus) — lihat komentar `Drop` soal identitas.
    ///
    /// Menolak (`None`) kalau sudah `MAX_SESSIONS` sesi aktif. Ini bug
    /// operasional bukan bug kode — dicatat lewat `tracing::warn!` supaya
    /// terlihat di log proses, tanpa menjatuhkan writer (log tetap ditulis
    /// ke file, hanya tanpa siaran langsung).
    pub fn mulai(self: &Arc<Self>, key: &str) -> Option<Arc<LogSession>> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|err| err.into_inner());

        if sessions.len() >= MAX_SESSIONS {
            tracing::warn!(
                key,
                batas = MAX_SESSIONS,
                "LogRegistry penuh, menolak sesi baru — log tetap ditulis ke file"
            );
            return None;
        }

        let session = Arc::new(LogSession {
            key: key.to_string(),
            tx: broadcast::channel(CHANNEL_CAPACITY).0,
            registry: Arc::downgrade(self),
        });
        sessions.insert(key.to_string(), Arc::downgrade(&session));
        Some(session)
    }

    /// Sambung ke sesi yang **sudah** aktif. Tidak pernah membuat sesi baru
    /// — dipanggil handler SSE (pembaca). Key tidak dikenal ATAU sesi sudah
    /// berakhir (writer sudah lepas `Arc`-nya) → `None`; pemanggil merender
    /// state "tidak ada sesi aktif" dari snapshot db/file, bukan menunggu
    /// channel yang tidak akan pernah terisi.
    pub fn ikut(&self, key: &str) -> Option<Arc<LogSession>> {
        let sessions = self.sessions.lock().unwrap_or_else(|err| err.into_inner());
        sessions.get(key).and_then(Weak::upgrade)
    }

    /// Sapuan jaring pengaman: buang entri map yang `Weak`-nya sudah tidak
    /// bisa di-upgrade (sisa sesi yang mestinya sudah dibuang `Drop`).
    /// Dipanggil worker tiap tick (sub-blok 3g).
    ///
    /// Dalam operasi normal fungsi ini **tidak menemukan apa pun** — `Drop`
    /// selalu membuang entrinya sendiri secara sinkron begitu `Arc` terakhir
    /// lenyap. Kalau sapuan ini menemukan entri yatim, itu pertanda `Drop`
    /// gagal berjalan (mis. panic saat memegang lock) — dicatat sebagai bug,
    /// bukan disapu diam-diam.
    pub fn sapu_yatim(&self) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|err| err.into_inner());
        let yatim: Vec<String> = sessions
            .iter()
            .filter(|(_, weak)| weak.upgrade().is_none())
            .map(|(key, _)| key.clone())
            .collect();
        for key in yatim {
            tracing::warn!(
                key,
                "sapuan LogRegistry menemukan entri yatim — Drop seharusnya \
                 sudah membersihkannya, ini bug"
            );
            sessions.remove(&key);
        }
    }

    /// Jumlah sesi aktif — dipakai test dan worker (metrik sederhana, bukan
    /// dipapar lewat HTTP).
    #[cfg(test)]
    pub(crate) fn jumlah_sesi(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }
}

impl LogSession {
    /// Sambung subscriber baru ke sesi ini.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEvent> {
        self.tx.subscribe()
    }

    /// Siarkan satu event. Tidak ada subscriber aktif bukan error —
    /// halaman viewer mungkin belum/sudah ditutup; baris tetap tertulis ke
    /// file oleh writer terlepas dari ada-tidaknya pendengar siaran.
    pub fn kirim(&self, event: LogEvent) {
        let _ = self.tx.send(event);
    }
}

impl Drop for LogSession {
    /// Buang entri map registry milik sesi ini SAAT `Arc` terakhir (writer
    /// atau subscriber manapun yang memegangnya) lenyap.
    ///
    /// Penjagaan identitas (bukan cuma key) untuk race `mulai()` vs `Drop`:
    /// kalau `mulai()` dipanggil lagi untuk key yang sama SEBELUM sesi lama
    /// selesai di-drop, map sudah berisi `Weak` ke sesi BARU. `Drop` sesi
    /// LAMA yang berjalan setelah itu tidak boleh menghapus entri yang
    /// sekarang menunjuk sesi baru. `Weak::as_ptr` dibandingkan sebagai
    /// alamat mentah (bukan `upgrade` — pada titik ini strong count sesi ini
    /// sendiri sudah nol, `upgrade` pasti gagal untuk KEDUA weak, jadi
    /// upgrade tidak bisa dipakai membedakan "punyaku" vs "punya sesi baru").
    ///
    /// Tidak ada deadlock: lock `sessions` hanya diambil DI SINI, dilepas
    /// sebelum `drop` selesai, dan tidak ada pemanggilan balik ke kode lain
    /// yang mengambil lock yang sama sambil lock ini masih dipegang.
    fn drop(&mut self) {
        let Some(registry) = self.registry.upgrade() else {
            // Registry (AppState) sendiri sudah lenyap (proses mati) —
            // tidak ada map untuk dibersihkan.
            return;
        };
        let mut sessions = registry
            .sessions
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(weak) = sessions.get(&self.key)
            && std::ptr::eq(weak.as_ptr(), self as *const LogSession)
        {
            sessions.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_kosong_setelah_writer_dan_semua_subscriber_drop() {
        let registry = Arc::new(LogRegistry::new());
        let session = registry.mulai("deploy-a").expect("mulai harus berhasil");
        let rx1 = session.subscribe();
        let rx2 = session.subscribe();

        assert_eq!(registry.jumlah_sesi(), 1);

        drop(rx1);
        drop(rx2);
        drop(session);

        assert_eq!(
            registry.jumlah_sesi(),
            0,
            "map registry wajib kosong setelah writer dan semua subscriber drop"
        );
    }

    #[test]
    fn ikut_untuk_key_tidak_dikenal_mengembalikan_none_tanpa_menyisakan_entri() {
        let registry = Arc::new(LogRegistry::new());
        assert!(registry.ikut("tidak-ada").is_none());
        assert_eq!(registry.jumlah_sesi(), 0);
    }

    #[test]
    fn mulai_menolak_setelah_batas_max_sessions_tercapai() {
        let registry = Arc::new(LogRegistry::new());
        let mut sesi_hidup = Vec::new();
        for i in 0..MAX_SESSIONS {
            let sesi = registry
                .mulai(&format!("deploy-{i}"))
                .expect("belum mencapai batas");
            sesi_hidup.push(sesi);
        }

        assert!(
            registry.mulai("deploy-lewat-batas").is_none(),
            "sesi ke-65 wajib ditolak"
        );

        // Sesi yang sudah diterima sebelumnya tetap hidup dan tidak
        // terpengaruh penolakan sesi baru.
        assert_eq!(registry.jumlah_sesi(), MAX_SESSIONS);
        drop(sesi_hidup);
    }

    #[test]
    fn subscriber_drop_lebih_dulu_tidak_menghapus_sesi_selama_writer_hidup() {
        let registry = Arc::new(LogRegistry::new());
        let session = registry.mulai("deploy-b").expect("mulai harus berhasil");
        let rx = session.subscribe();

        drop(rx);
        assert_eq!(
            registry.jumlah_sesi(),
            1,
            "sesi wajib tetap ada selama writer (Arc pertama) masih hidup"
        );

        // `ikut` masih bisa menyambung setelah subscriber lama putus.
        let masih_ada = registry.ikut("deploy-b");
        assert!(masih_ada.is_some());
        drop(masih_ada);
        drop(session);
        assert_eq!(registry.jumlah_sesi(), 0);
    }

    #[test]
    fn dua_subscriber_pada_key_yang_sama_menerima_event_yang_sama() {
        let registry = Arc::new(LogRegistry::new());
        let session = registry.mulai("deploy-c").expect("mulai harus berhasil");
        let mut rx1 = session.subscribe();
        let mut rx2 = session.subscribe();

        session.kirim(LogEvent::Baris(Arc::from("baris pertama")));

        let e1 = rx1.try_recv().expect("subscriber 1 harus menerima event");
        let e2 = rx2.try_recv().expect("subscriber 2 harus menerima event");
        match (e1, e2) {
            (LogEvent::Baris(a), LogEvent::Baris(b)) => {
                assert_eq!(a.as_ref(), "baris pertama");
                assert_eq!(b.as_ref(), "baris pertama");
            }
            _ => panic!("event yang diterima harus LogEvent::Baris"),
        }
    }

    #[test]
    fn event_selesai_diterima_subscriber() {
        let registry = Arc::new(LogRegistry::new());
        let session = registry.mulai("deploy-d").expect("mulai harus berhasil");
        let mut rx = session.subscribe();

        session.kirim(LogEvent::Selesai);
        let event = rx.try_recv().expect("subscriber harus menerima Selesai");
        assert!(matches!(event, LogEvent::Selesai));
    }

    #[test]
    fn sapu_yatim_tidak_menemukan_apa_pun_pada_operasi_normal() {
        let registry = Arc::new(LogRegistry::new());
        let session = registry.mulai("deploy-e").expect("mulai harus berhasil");
        drop(session);

        // Drop sudah membersihkan entrinya sendiri secara sinkron — sapuan
        // tidak boleh menemukan apa pun untuk dibuang lagi.
        registry.sapu_yatim();
        assert_eq!(registry.jumlah_sesi(), 0);
    }

    #[test]
    fn mulai_ulang_untuk_key_yang_sama_menimpa_map_dan_sesi_lama_tetap_hidup_lewat_arc_terpisah() {
        let registry = Arc::new(LogRegistry::new());
        let sesi_lama = registry.mulai("deploy-f").expect("mulai harus berhasil");
        let sesi_baru = registry.mulai("deploy-f").expect("mulai harus berhasil");

        // Map sekarang menunjuk sesi baru.
        let dari_ikut = registry.ikut("deploy-f").expect("sesi baru harus ada");
        assert!(Arc::ptr_eq(&dari_ikut, &sesi_baru));
        drop(dari_ikut);

        // Drop sesi lama TIDAK boleh menghapus entri milik sesi baru
        // (penjagaan identitas `Weak::as_ptr`, bukan cuma key).
        drop(sesi_lama);
        assert_eq!(registry.jumlah_sesi(), 1);
        let masih_baru = registry
            .ikut("deploy-f")
            .expect("entri sesi baru wajib tetap ada");
        assert!(Arc::ptr_eq(&masih_baru, &sesi_baru));
    }
}
