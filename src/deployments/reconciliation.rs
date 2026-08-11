use std::collections::HashMap;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::docker::ContainerObservation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftKind {
    LiveContainerMissing,
    LiveContainerNotRunning,
    LiveDigestMismatch,
    LiveContainerIdMismatch,
    MultipleLiveContainers,
    OrphanPlatformContainer,
}

impl DriftKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LiveContainerMissing => "live_container_missing",
            Self::LiveContainerNotRunning => "live_container_not_running",
            Self::LiveDigestMismatch => "live_digest_mismatch",
            Self::LiveContainerIdMismatch => "live_container_id_mismatch",
            Self::MultipleLiveContainers => "multiple_live_containers",
            Self::OrphanPlatformContainer => "orphan_platform_container",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftObservation {
    pub kind: DriftKind,
    pub severity: &'static str,
    pub observed_container_id: Option<String>,
    pub observed_digest: Option<String>,
}

pub fn classify_live_drift(
    expected_deployment_id: &str,
    expected_digest: &str,
    expected_container_id: Option<&str>,
    containers: &[ContainerObservation],
) -> Vec<DriftObservation> {
    let matching: Vec<&ContainerObservation> = containers
        .iter()
        .filter(|container| {
            container
                .labels
                .get("platform.deployment")
                .map(String::as_str)
                == Some(expected_deployment_id)
        })
        .collect();
    if matching.is_empty() {
        return vec![DriftObservation {
            kind: DriftKind::LiveContainerMissing,
            severity: "critical",
            observed_container_id: None,
            observed_digest: None,
        }];
    }
    let mut findings = Vec::new();
    if matching.len() > 1 {
        findings.push(DriftObservation {
            kind: DriftKind::MultipleLiveContainers,
            severity: "critical",
            observed_container_id: matching.first().map(|container| container.id.clone()),
            observed_digest: None,
        });
    }
    let container = matching[0];
    if !container.running {
        findings.push(DriftObservation {
            kind: DriftKind::LiveContainerNotRunning,
            severity: "critical",
            observed_container_id: Some(container.id.clone()),
            observed_digest: None,
        });
    }
    let observed_digest = container.labels.get("platform.digest").cloned();
    if observed_digest.as_deref() != Some(expected_digest) {
        findings.push(DriftObservation {
            kind: DriftKind::LiveDigestMismatch,
            severity: "critical",
            observed_container_id: Some(container.id.clone()),
            observed_digest,
        });
    }
    if let Some(expected_id) = expected_container_id
        && expected_id != container.id
    {
        findings.push(DriftObservation {
            kind: DriftKind::LiveContainerIdMismatch,
            severity: "warning",
            observed_container_id: Some(container.id.clone()),
            observed_digest: container.labels.get("platform.digest").cloned(),
        });
    }
    findings
}

pub fn classify_orphan_containers(
    expected_deployments: &HashMap<String, String>,
    containers: &[ContainerObservation],
) -> Vec<DriftObservation> {
    containers
        .iter()
        .filter(|container| {
            container
                .labels
                .get("platform.deployment")
                .is_some_and(|deployment| !expected_deployments.contains_key(deployment))
        })
        .map(|container| DriftObservation {
            kind: DriftKind::OrphanPlatformContainer,
            severity: "warning",
            observed_container_id: Some(container.id.clone()),
            observed_digest: container.labels.get("platform.digest").cloned(),
        })
        .collect()
}

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingStatus {
    Open,
    Acknowledged,
    Resolved,
}

impl FindingStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::Resolved => "resolved",
        }
    }
}

pub struct FindingInput<'a> {
    pub id: &'a str,
    pub app_id: &'a str,
    pub server_id: &'a str,
    pub deployment_id: Option<&'a str>,
    pub kind: &'a str,
    pub severity: &'a str,
    pub fingerprint: &'a str,
    pub expected_json: Option<&'a str>,
    pub observed_json: Option<&'a str>,
}

pub struct FindingRingkas {
    pub id: String,
    pub app_id: String,
    pub server_id: String,
    pub deployment_id: Option<String>,
    pub kind: String,
    pub severity: String,
    pub status: FindingStatus,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

fn status_from_db(value: &str) -> FindingStatus {
    match value {
        "acknowledged" => FindingStatus::Acknowledged,
        "resolved" => FindingStatus::Resolved,
        _ => FindingStatus::Open,
    }
}

pub async fn list_active(pool: &SqlitePool, app_id: &str) -> Result<Vec<FindingRingkas>> {
    let rows = sqlx::query!(
        r#"SELECT id as "id!", app_id, server_id, deployment_id, kind, severity,
                  status, first_seen_at, last_seen_at
           FROM reconciliation_findings
           WHERE app_id = ? AND status IN ('open', 'acknowledged')
           ORDER BY last_seen_at DESC"#,
        app_id,
    )
    .fetch_all(pool)
    .await
    .context("baca finding rekonsiliasi aktif")?;

    Ok(rows
        .into_iter()
        .map(|row| FindingRingkas {
            id: row.id,
            app_id: row.app_id,
            server_id: row.server_id,
            deployment_id: row.deployment_id,
            kind: row.kind,
            severity: row.severity,
            status: status_from_db(&row.status),
            first_seen_at: row.first_seen_at,
            last_seen_at: row.last_seen_at,
        })
        .collect())
}

pub async fn acknowledge(pool: &SqlitePool, finding_id: &str, app_id: &str) -> Result<bool> {
    let now = now_epoch();
    let result = sqlx::query!(
        "UPDATE reconciliation_findings
         SET status = 'acknowledged', acknowledged_at = ?
         WHERE id = ? AND app_id = ? AND status = 'open'",
        now,
        finding_id,
        app_id,
    )
    .execute(pool)
    .await
    .context("akui finding rekonsiliasi")?;
    Ok(result.rows_affected() == 1)
}

pub async fn upsert_open(pool: &SqlitePool, finding: FindingInput<'_>) -> Result<bool> {
    let existing = sqlx::query!(
        "SELECT status FROM reconciliation_findings WHERE server_id = ? AND fingerprint = ?",
        finding.server_id,
        finding.fingerprint,
    )
    .fetch_optional(pool)
    .await
    .context("cek status finding rekonsiliasi")?;
    let notify = existing
        .as_ref()
        .map(|row| row.status == "resolved")
        .unwrap_or(true);
    let now = now_epoch();
    sqlx::query!(
        "INSERT INTO reconciliation_findings
            (id, app_id, server_id, deployment_id, kind, severity, status,
             fingerprint, expected_json, observed_json, first_seen_at, last_seen_at)
         VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?, ?, ?, ?)
         ON CONFLICT (server_id, fingerprint) DO UPDATE SET
             app_id = excluded.app_id,
             deployment_id = excluded.deployment_id,
             kind = excluded.kind,
             severity = excluded.severity,
             status = CASE
                 WHEN reconciliation_findings.status = 'acknowledged' THEN 'acknowledged'
                 ELSE 'open'
             END,
             expected_json = excluded.expected_json,
             observed_json = excluded.observed_json,
             last_seen_at = excluded.last_seen_at,
             resolved_at = NULL",
        finding.id,
        finding.app_id,
        finding.server_id,
        finding.deployment_id,
        finding.kind,
        finding.severity,
        finding.fingerprint,
        finding.expected_json,
        finding.observed_json,
        now,
        now,
    )
    .execute(pool)
    .await
    .context("buka atau perbarui finding rekonsiliasi")?;
    Ok(notify)
}

pub async fn resolve_missing(pool: &SqlitePool, server_id: &str, seen_at: i64) -> Result<u64> {
    let result = sqlx::query!(
        "UPDATE reconciliation_findings
         SET status = 'resolved', resolved_at = ?
         WHERE server_id = ? AND status IN ('open', 'acknowledged')
           AND last_seen_at < ?",
        seen_at,
        server_id,
        seen_at,
    )
    .execute(pool)
    .await
    .context("tandai finding rekonsiliasi pulih")?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_finding_memiliki_nilai_database_stabil() {
        assert_eq!(FindingStatus::Open.as_db_str(), "open");
        assert_eq!(FindingStatus::Acknowledged.as_db_str(), "acknowledged");
        assert_eq!(FindingStatus::Resolved.as_db_str(), "resolved");
    }

    fn container(id: &str, deployment: &str, digest: &str, running: bool) -> ContainerObservation {
        ContainerObservation {
            id: id.to_string(),
            image: None,
            labels: HashMap::from([
                ("platform.deployment".to_string(), deployment.to_string()),
                ("platform.digest".to_string(), digest.to_string()),
            ]),
            running,
            status: None,
        }
    }

    #[test]
    fn klasifikasi_drift_mendeteksi_hilang_digest_mismatch_dan_multiple() {
        let healthy = container("c1", "dep1", "digest1", true);
        assert!(classify_live_drift("dep1", "digest1", Some("c1"), &[healthy]).is_empty());

        let mismatch = container("c2", "dep1", "digest2", true);
        let findings = classify_live_drift("dep1", "digest1", Some("c1"), &[mismatch]);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == DriftKind::LiveDigestMismatch)
        );
        assert!(
            findings
                .iter()
                .any(|f| f.kind == DriftKind::LiveContainerIdMismatch)
        );

        let multiple = vec![
            container("c1", "dep1", "digest1", true),
            container("c2", "dep1", "digest1", true),
        ];
        assert!(
            classify_live_drift("dep1", "digest1", None, &multiple)
                .iter()
                .any(|f| f.kind == DriftKind::MultipleLiveContainers)
        );

        let missing = classify_live_drift("dep1", "digest1", None, &[]);
        assert_eq!(missing[0].kind, DriftKind::LiveContainerMissing);
    }

    #[test]
    fn klasifikasi_orphan_tidak_menawarkan_perbaikan() {
        let containers = vec![container("c1", "manual", "digest", true)];
        let expected = HashMap::new();
        let findings = classify_orphan_containers(&expected, &containers);
        assert_eq!(findings[0].kind, DriftKind::OrphanPlatformContainer);
    }
}
