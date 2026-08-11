//! `POST /api/v1/deploy` — SATU-SATUNYA endpoint bearer token (bukan sesi
//! cookie CSRF seperti route lain), `docs/plan.md` kontrak Fase 2. CI
//! memanggil ini langsung, tidak pernah lewat browser.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use serde::{Deserialize, Serialize};

use crate::apps::repo as apps_repo;
use crate::auth::deploy_token;
use crate::deployments::{self, DeployJobPayload, LOCK_TTL_SECS, NewDeployment};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct DeployRequest {
    app: String,
    image: String,
    commit: String,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

#[derive(Serialize)]
pub struct DeployResponse {
    deployment_id: String,
}

/// Urutan WAJIB (`docs/plan.md`): app dulu (404 kalau tidak dikenal, bahkan
/// dengan token app lain yang valid — jangan bocorkan app mana yang ada),
/// BARU token app itu diverifikasi (401), BARU validasi digest (400), BARU
/// lock diambil (409 kalau app sedang deploy lain).
pub async fn deploy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeployRequest>,
) -> Result<(StatusCode, Json<DeployResponse>), AppError> {
    let app = apps_repo::find_by_name(&state.db_read, &payload.app)
        .await?
        .ok_or(AppError::NotFound)?;

    let token = bearer_token(&headers).ok_or(AppError::Unauthorized)?;
    let kandidat = apps_repo::list_deploy_token_hashes(&state.db_read, &app.id).await?;
    let token_id = kandidat
        .iter()
        .find(|(_, hash)| deploy_token::verify(token, hash).unwrap_or(false))
        .map(|(id, _)| id.clone())
        .ok_or(AppError::Unauthorized)?;

    if !digest_valid(&payload.image) {
        return Err(AppError::BadRequest(
            "image harus referensi lengkap @sha256:<64 hex>, bukan tag".to_string(),
        ));
    }

    let deployment_id = deployments::repo::generate_id();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let terkunci = apps_repo::acquire_lock(
        &state.db_write,
        &app.id,
        &deployment_id,
        now + LOCK_TTL_SECS,
    )
    .await?;
    if !terkunci {
        return Err(AppError::Conflict(
            "app sedang dalam proses deploy lain".to_string(),
        ));
    }

    let job_id = deployments::repo::generate_id();
    let job_payload_json = serde_json::to_string(&DeployJobPayload {
        deployment_id: deployment_id.clone(),
    })
    .map_err(|err| AppError::Internal(err.into()))?;

    // Deploy CI TIDAK menyentuh env, tapi deployment yang dihasilkan tetap
    // harus tahu env AKTIF app ini saat ini (`docs/plan.md` Fase 4: "deploy
    // yang dipicu digest baru memakai env yang sedang berjalan") — supaya
    // `deployments::engine` selalu punya `env_version_id` untuk ditulis ke
    // target, apa pun pemicu deploy-nya. `None` kalau app belum pernah
    // punya env sama sekali.
    let env_version_id = apps_repo::find_latest_env_version(&state.db_read, &app.id)
        .await?
        .map(|v| v.id);

    if let Err(err) = deployments::repo::insert_queued_dengan_job(
        &state.db_write,
        &deployment_id,
        NewDeployment {
            app_id: &app.id,
            commit_sha: &payload.commit,
            git_ref: payload.git_ref.as_deref(),
            image_digest: &payload.image,
            trigger_source: "api",
            env_version_id: env_version_id.as_deref(),
        },
        &job_id,
        &job_payload_json,
    )
    .await
    {
        let _ = apps_repo::release_lock(&state.db_write, &app.id, &deployment_id).await;
        return Err(AppError::from(err));
    }

    let _ = apps_repo::touch_deploy_token_last_used(&state.db_write, &token_id).await;

    Ok((StatusCode::ACCEPTED, Json(DeployResponse { deployment_id })))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// `image` harus diakhiri `@sha256:<64 hex huruf kecil>` — invariant §5
/// no.6 "image selalu digest, tidak pernah tag", ditolak SEBELUM apa pun
/// disentuh (`docs/plan.md` tabel risiko).
fn digest_valid(image: &str) -> bool {
    let Some((_, digest_part)) = image.rsplit_once('@') else {
        return false;
    };
    let Some(hex) = digest_part.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_valid_menerima_referensi_lengkap_dengan_sha256() {
        assert!(digest_valid(&format!(
            "ghcr.io/org/app@sha256:{}",
            "a".repeat(64)
        )));
    }

    #[test]
    fn digest_valid_menolak_tag_polos() {
        assert!(!digest_valid("ghcr.io/org/app:latest"));
        assert!(!digest_valid("ghcr.io/org/app"));
    }

    #[test]
    fn digest_valid_menolak_huruf_besar_atau_panjang_salah() {
        assert!(!digest_valid(&format!("app@sha256:{}", "A".repeat(64))));
        assert!(!digest_valid("app@sha256:tidakcukuppanjang"));
    }

    #[test]
    fn bearer_token_mengambil_nilai_setelah_prefiks() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer abc123".parse().unwrap());
        assert_eq!(bearer_token(&headers), Some("abc123"));
    }

    #[test]
    fn bearer_token_none_tanpa_header_atau_prefiks_salah() {
        assert_eq!(bearer_token(&HeaderMap::new()), None);
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Basic abc123".parse().unwrap());
        assert_eq!(bearer_token(&headers), None);
    }
}
