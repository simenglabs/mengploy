//! Tipe domain dan view-model `deployments` — dibaca `src/web/**`.
//!
//! State machine (`docs/plan.md` Fase 2, CLAUDE.md §9 lama):
//! ```text
//! queued → pulling → starting → checking → live
//!               ↘ failed
//!               ↘ cancelled
//!               ↘ unknown
//! ```
//! `unknown` BUKAN `failed` — dipakai murni saat control plane restart di
//! tengah deployment dan heartbeat basi (rekonsiliasi penuh baru Fase 5;
//! Fase 2 cukup MENANDAI, tidak menebak).

/// State machine status deployment (`deployments.status` CHECK constraint,
/// `migrations/0003_deploy.sql`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDeployment {
    Queued,
    Pulling,
    Starting,
    Checking,
    Live,
    Failed,
    Cancelled,
    Unknown,
}

impl StatusDeployment {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Pulling => "pulling",
            Self::Starting => "starting",
            Self::Checking => "checking",
            Self::Live => "live",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    /// Nilai tak dikenal dipetakan ke `Unknown` — bukan ditebak jadi status
    /// lain (`docs/prd.md`: "unknown artinya kita tidak tahu, jangan
    /// pura-pura tahu").
    pub fn from_db_str(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "pulling" => Self::Pulling,
            "starting" => Self::Starting,
            "checking" => Self::Checking,
            "live" => Self::Live,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    /// Status akhir (tidak akan bertransisi lagi tanpa job baru).
    pub fn selesai(self) -> bool {
        matches!(
            self,
            Self::Live | Self::Failed | Self::Cancelled | Self::Unknown
        )
    }
}

pub struct DeploymentRingkas {
    pub id: String,
    pub app_id: String,
    pub commit_sha: String,
    pub git_ref: Option<String>,
    pub image_digest: String,
    pub status: StatusDeployment,
    pub container_id: Option<String>,
    /// `Some` sejak Fase 4 — versi env yang dipakai deployment ini (baik
    /// dipicu perubahan env langsung MAUPUN deploy CI biasa, keduanya
    /// selalu mengisi ini dengan versi env AKTIF app saat itu,
    /// `docs/plan.md` "Desain teknis"). `None` HANYA untuk app yang belum
    /// pernah punya env sama sekali.
    pub env_version_id: Option<String>,
    pub error_kind: Option<String>,
    pub error_detail: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_deployment_roundtrip_lewat_string_db() {
        for status in [
            StatusDeployment::Queued,
            StatusDeployment::Pulling,
            StatusDeployment::Starting,
            StatusDeployment::Checking,
            StatusDeployment::Live,
            StatusDeployment::Failed,
            StatusDeployment::Cancelled,
            StatusDeployment::Unknown,
        ] {
            assert_eq!(StatusDeployment::from_db_str(status.as_db_str()), status);
        }
    }

    #[test]
    fn status_deployment_nilai_tak_dikenal_jatuh_ke_unknown() {
        assert_eq!(
            StatusDeployment::from_db_str("apa-saja"),
            StatusDeployment::Unknown
        );
    }

    #[test]
    fn selesai_benar_untuk_status_akhir_saja() {
        assert!(!StatusDeployment::Queued.selesai());
        assert!(!StatusDeployment::Pulling.selesai());
        assert!(!StatusDeployment::Starting.selesai());
        assert!(!StatusDeployment::Checking.selesai());
        assert!(StatusDeployment::Live.selesai());
        assert!(StatusDeployment::Failed.selesai());
        assert!(StatusDeployment::Cancelled.selesai());
        assert!(StatusDeployment::Unknown.selesai());
    }
}
