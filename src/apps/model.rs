//! Tipe domain dan view-model `apps`/`domains` — dibaca `src/web/**` untuk
//! merender. Tidak ada secret di tabel `apps` sama sekali (token deploy
//! tinggal di `deploy_tokens`, terpisah), jadi tidak ada varian "row mentah
//! vs ringkasan aman" seperti `servers`/`registries` — satu tipe cukup.
//!
//! `DeployTokenRingkas` PENGECUALIAN sengaja: `deploy_tokens.token_hash`
//! TIDAK PERNAH masuk tipe ini (invariant §5 no.11 — secret tidak pernah
//! dikembalikan API setelah disimpan). Token plaintext hanya lewat sekali,
//! langsung dari `auth::deploy_token::generate()` saat dibuat.

pub struct AppRingkas {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub health_path: String,
    pub health_grace_secs: i64,
    pub port: i64,
    pub restart_policy: String,
    /// URL repository (GitHub/GitLab) — referensi metadata opsional,
    /// TANPA akses API dan TANPA build oleh mengploy (PRD §1.5 non-goal
    /// "Membangun image sendiri"). `None` = tidak ada referensi repo.
    pub repo_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct DomainRingkas {
    pub id: String,
    pub app_id: String,
    pub host: String,
    pub tls_enabled: bool,
}

pub struct DeployTokenRingkas {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

pub struct EnvVersionRingkas {
    pub id: String,
    pub version: i64,
    pub note: Option<String>,
    pub created_at: i64,
}
