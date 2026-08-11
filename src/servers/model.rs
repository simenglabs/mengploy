//! Tipe domain dan view-model server — dibaca `src/web/**` (frontend) untuk
//! merender fleet overview, fleet strip, dan detail server.
//!
//! **`ServerRingkas` TIDAK PERNAH punya field kunci SSH, token registry,
//! atau turunannya** (invariant 7, `docs/plan.md` "Kontrak render"). Baris
//! mentah dengan secret (`ssh_key_encrypted`) tinggal di
//! `servers::repo::ServerRow`, yang tidak diekspor ke `src/web/`.

/// State machine status server (`servers.status` CHECK constraint,
/// `migrations/0002_servers.sql`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusServer {
    Pending,
    Verifying,
    Online,
    Unreachable,
}

impl StatusServer {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Verifying => "verifying",
            Self::Online => "online",
            Self::Unreachable => "unreachable",
        }
    }

    /// Nilai tak dikenal (seharusnya tidak mungkin — kolom punya `CHECK`)
    /// dipetakan ke `Pending` sebagai default paling aman, bukan panik.
    pub fn from_db_str(value: &str) -> Self {
        match value {
            "verifying" => Self::Verifying,
            "online" => Self::Online,
            "unreachable" => Self::Unreachable,
            _ => Self::Pending,
        }
    }
}

/// Ringkasan server untuk fleet overview/strip/detail.
pub struct ServerRingkas {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub ssh_user: String,
    pub status: StatusServer,
    pub last_seen_at: Option<i64>,
    pub docker_version: Option<String>,
    pub os_info: Option<String>,
    /// Fingerprint host key server target — BUKAN secret, aman ditampilkan
    /// (`docs/prd.md:245`, `docs/design/server-detail.md` §4.1).
    pub host_key_fingerprint: Option<String>,
    pub consecutive_failures: i64,
    pub last_error_kind: Option<String>,
    pub last_error_message: Option<String>,
}

/// Satu langkah dalam checklist verifikasi wizard tambah-server
/// (`docs/design/tambah-server.md`): koneksi → Docker → registry.
#[derive(Clone)]
pub struct LangkahVerifikasi {
    pub nama: String,
    pub status: LangkahStatus,
    /// Pesan Bahasa Indonesia yang sudah dipetakan dari kategori error —
    /// TIDAK PERNAH stderr/pesan mentah `openssh`/`bollard` (invariant 9,
    /// sama seperti `servers.last_error_message`).
    pub pesan: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LangkahStatus {
    Menunggu,
    Berjalan,
    Sukses,
    Gagal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_server_roundtrip_lewat_string_db() {
        for status in [
            StatusServer::Pending,
            StatusServer::Verifying,
            StatusServer::Online,
            StatusServer::Unreachable,
        ] {
            assert_eq!(StatusServer::from_db_str(status.as_db_str()), status);
        }
    }

    #[test]
    fn status_server_nilai_tak_dikenal_jatuh_ke_pending() {
        assert_eq!(StatusServer::from_db_str("apa-saja"), StatusServer::Pending);
    }
}
