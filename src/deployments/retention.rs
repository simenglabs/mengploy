use std::collections::HashSet;

use super::model::{DeploymentRingkas, StatusDeployment};

/// Pilih digest yang aman dipertimbangkan untuk dihapus. Fungsi ini sengaja
/// murni; caller wajib memastikan server dapat dijangkau sebelum melakukan
/// operasi Docker destruktif.
pub fn kandidat_penghapusan(
    deployments: &[DeploymentRingkas],
    image_digests: &[String],
    container_digests: &HashSet<String>,
) -> Vec<String> {
    let mut dilindungi = container_digests.clone();
    let mut sukses = deployments
        .iter()
        .filter(|dep| matches!(dep.status, StatusDeployment::Live))
        .map(|dep| dep.image_digest.clone());
    for digest in sukses.by_ref().take(5) {
        dilindungi.insert(digest);
    }
    for dep in deployments {
        if matches!(
            dep.status,
            StatusDeployment::Live
                | StatusDeployment::Queued
                | StatusDeployment::Pulling
                | StatusDeployment::Starting
                | StatusDeployment::Checking
                | StatusDeployment::Unknown
        ) {
            dilindungi.insert(dep.image_digest.clone());
        }
    }
    image_digests
        .iter()
        .filter(|digest| !dilindungi.contains(*digest))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(status: StatusDeployment, digest: &str, created_at: i64) -> DeploymentRingkas {
        DeploymentRingkas {
            id: format!("dep-{created_at}"),
            app_id: "app".to_string(),
            commit_sha: "sha".to_string(),
            git_ref: None,
            image_digest: digest.to_string(),
            status,
            container_id: None,
            env_version_id: None,
            error_kind: None,
            error_detail: None,
            started_at: None,
            finished_at: None,
            created_at,
        }
    }

    #[test]
    fn retensi_melindungi_live_unknown_dan_container() {
        let deployments = vec![
            dep(StatusDeployment::Live, "d-live", 3),
            dep(StatusDeployment::Unknown, "d-unknown", 2),
            dep(StatusDeployment::Failed, "d-old", 1),
        ];
        let mut containers = HashSet::new();
        containers.insert("d-container".to_string());
        let candidates = kandidat_penghapusan(
            &deployments,
            &[
                "d-live".to_string(),
                "d-unknown".to_string(),
                "d-container".to_string(),
                "d-old".to_string(),
            ],
            &containers,
        );
        assert_eq!(candidates, vec!["d-old".to_string()]);
    }
}
