use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::deployments::reconciliation;
use crate::docker;
use crate::notifications::{self, model::WebhookEnvelope};
use crate::servers::repo::ServerRow;
use crate::ssh::{self, HostKeyMode};
use crate::state::AppState;

use super::WorkerHandle;

const TICK_INTERVAL: Duration = Duration::from_secs(60);
const MAX_CONCURRENT_SCANS: usize = 4;

/// Siklus rekonsiliasi benar-benar mengobservasi Docker target melalui SSH
/// dan forward socket. Jalur ini read-only: tidak ada stop/remove/adopt.
async fn jalankan_satu_siklus(state: &AppState) {
    // Rekonsiliasi adalah scanner penuh tersendiri, bukan polling ringan:
    // setiap server yang sudah lolos verifikasi diobservasi pada setiap tick.
    // `i64::MAX` mempertahankan query repository yang sama sambil memilih
    // semua baris ber-fingerprint; server pending tetap tersaring.
    let servers = match crate::servers::repo::list_due_for_poll(&state.db_read, i64::MAX).await {
        Ok(servers) => servers,
        Err(err) => {
            tracing::warn!(error = %err, "gagal membaca server untuk rekonsiliasi");
            return;
        }
    };

    let mut tasks: JoinSet<(ServerRow, anyhow::Result<Vec<docker::ContainerObservation>>)> =
        JoinSet::new();
    let mut remaining = servers.into_iter();
    for row in remaining.by_ref().take(MAX_CONCURRENT_SCANS) {
        spawn_scan(&mut tasks, state.clone(), row);
    }

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((row, Ok(containers))) => {
                if let Err(err) = simpan_observasi(state, &row, &containers).await {
                    tracing::warn!(error = %err, server_id = %row.id, "gagal menyimpan hasil rekonsiliasi");
                }
            }
            Ok((row, Err(err))) => {
                tracing::warn!(error = %err, server_id = %row.id, "scanner rekonsiliasi gagal; tidak ada tindakan otomatis");
            }
            Err(err) => tracing::warn!(error = %err, "task scanner rekonsiliasi dibatalkan"),
        }
        if let Some(row) = remaining.next() {
            spawn_scan(&mut tasks, state.clone(), row);
        }
    }
}

fn spawn_scan(
    tasks: &mut JoinSet<(ServerRow, anyhow::Result<Vec<docker::ContainerObservation>>)>,
    state: AppState,
    row: ServerRow,
) {
    tasks.spawn(async move {
        let result = observasi_docker(&state, &row).await;
        (row, result)
    });
}

async fn observasi_docker(
    state: &AppState,
    row: &ServerRow,
) -> anyhow::Result<Vec<docker::ContainerObservation>> {
    let fingerprint = row
        .host_key_fingerprint
        .clone()
        .ok_or_else(|| anyhow::anyhow!("fingerprint host key belum tersedia"))?;
    let key = state
        .crypto
        .decrypt(&row.ssh_key_encrypted)
        .map_err(|_| anyhow::anyhow!("kunci SSH tersimpan tidak bisa didekripsi"))?;
    let session = match ssh::connect(
        &row.host,
        row.port as u16,
        &row.ssh_user,
        &key,
        &state.config.runtime_dir,
        HostKeyMode::Strict {
            expected_fingerprint: fingerprint,
        },
    )
    .await
    .map_err(|_| anyhow::anyhow!("koneksi SSH scanner gagal"))?
    {
        ssh::ConnectOutcome::Established(session) => session,
        ssh::ConnectOutcome::TofuPending { session, .. } => {
            let _ = session.close().await;
            return Err(anyhow::anyhow!("host key scanner tidak konsisten"));
        }
    };

    let forward = match docker::establish(&session, &state.config.runtime_dir, &row.id).await {
        Ok(forward) => forward,
        Err(_) => {
            let _ = session.close().await;
            return Err(anyhow::anyhow!("forward Docker scanner gagal"));
        }
    };
    let result = async {
        let client = docker::connect(forward.socket_path())
            .map_err(|_| anyhow::anyhow!("koneksi Docker scanner gagal"))?;
        docker::ping(&client)
            .await
            .map_err(|_| anyhow::anyhow!("ping Docker scanner gagal"))?;
        docker::list_containers_with_label(&client, "platform.deployment")
            .await
            .map_err(|_| anyhow::anyhow!("observasi container Docker gagal"))
    }
    .await;
    docker::close(&session, forward).await;
    let _ = session.close().await;
    result
}

async fn simpan_observasi(
    state: &AppState,
    row: &ServerRow,
    containers: &[docker::ContainerObservation],
) -> anyhow::Result<()> {
    let apps = crate::apps::repo::list_by_server(&state.db_read, &row.id).await?;
    let mut expected = HashMap::new();
    for app in apps {
        if let Some(deployment) =
            crate::deployments::repo::find_current_live(&state.db_read, &app.id, "").await?
        {
            expected.insert(deployment.id.clone(), app.id.clone());
            let findings = reconciliation::classify_live_drift(
                &deployment.id,
                &deployment.image_digest,
                deployment.container_id.as_deref(),
                containers,
            );
            for finding in findings {
                let fingerprint = format!("{}:{}", deployment.id, finding.kind.as_str());
                let observed = serde_json::json!({
                    "container_id": finding.observed_container_id,
                    "digest": finding.observed_digest,
                });
                let notify = reconciliation::upsert_open(
                    &state.db_write,
                    reconciliation::FindingInput {
                        id: &crate::deployments::repo::generate_id(),
                        app_id: &app.id,
                        server_id: &row.id,
                        deployment_id: Some(&deployment.id),
                        kind: finding.kind.as_str(),
                        severity: finding.severity,
                        fingerprint: &fingerprint,
                        expected_json: Some("{\"state\":\"live\"}"),
                        observed_json: Some(&observed.to_string()),
                    },
                )
                .await?;
                if notify {
                    enqueue_drift_notification(
                        state,
                        &format!(
                            "{}:{}",
                            fingerprint,
                            time::OffsetDateTime::now_utc().unix_timestamp()
                        ),
                        Some(&app.id),
                        &row.id,
                        finding.kind.as_str(),
                        &observed,
                    )
                    .await;
                }
            }
        }
    }
    for finding in reconciliation::classify_orphan_containers(&expected, containers) {
        let fingerprint = format!(
            "orphan:{}",
            finding
                .observed_container_id
                .as_deref()
                .unwrap_or("unknown")
        );
        let observed = serde_json::json!({"container_id": finding.observed_container_id});
        // Orphan container tidak punya app FK yang dapat diisi. Ia hanya
        // dicatat lewat log aman sampai ada pemetaan app yang valid; tidak
        // pernah diadopsi otomatis. Temuan orphan tetap memakai event queue
        // agar operator mendapat notifikasi, sementara tabel finding yang
        // mewajibkan app_id tidak dipalsukan.
        tracing::warn!(server_id = %row.id, fingerprint, observed = %observed, "container platform orphan terdeteksi; tidak diadopsi");
        enqueue_drift_notification(
            state,
            &fingerprint,
            None,
            &row.id,
            finding.kind.as_str(),
            &observed,
        )
        .await;
    }
    let resolved = reconciliation::resolve_missing(
        &state.db_write,
        &row.id,
        time::OffsetDateTime::now_utc().unix_timestamp(),
    )
    .await?;
    if resolved > 0 {
        enqueue_drift_notification(
            state,
            &format!(
                "{}:resolved:{}",
                row.id,
                time::OffsetDateTime::now_utc().unix_timestamp()
            ),
            None,
            &row.id,
            "resolved",
            &serde_json::json!({"resolved_count": resolved}),
        )
        .await;
    }
    Ok(())
}

async fn enqueue_drift_notification(
    state: &AppState,
    event_id: &str,
    app_id: Option<&str>,
    server_id: &str,
    kind: &str,
    observed: &serde_json::Value,
) {
    let event_type = if kind == "resolved" {
        notifications::EVENT_DRIFT_RESOLVED
    } else {
        notifications::EVENT_DRIFT_DETECTED
    };
    let envelope = WebhookEnvelope {
        event_id,
        event_type,
        occurred_at: time::OffsetDateTime::now_utc().unix_timestamp(),
        data: serde_json::json!({
            "server_id": server_id,
            "app_id": app_id,
            "kind": kind,
            "observed": observed,
        }),
    };
    let Ok(payload) = serde_json::to_string(&envelope) else {
        tracing::warn!(event_id, "gagal serialisasi notifikasi drift");
        return;
    };
    if let Err(err) = notifications::repo::enqueue(
        &state.db_write,
        &crate::deployments::repo::generate_id(),
        event_id,
        event_type,
        app_id,
        &payload,
    )
    .await
    {
        tracing::warn!(error = %err, event_id, "gagal memasukkan notifikasi drift");
    }
}

pub fn spawn(state: AppState) -> WorkerHandle {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => jalankan_satu_siklus(&state).await,
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() { break; }
                }
            }
        }
    });
    WorkerHandle {
        shutdown_tx,
        join_handle,
    }
}
