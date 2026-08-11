//! Eksekutor operasi armada Fase 7.
//! Setiap target diproses independen dengan konkurensi maksimum empat;
//! kegagalan satu server tidak membatalkan target lain.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio::task::JoinSet;
use tokio_stream::StreamExt as _;

use crate::apps::repo as apps_repo;
use crate::deployments::repo as deployments_repo;
use crate::deployments::retention;
use crate::docker;
use crate::fleet::{self, FleetEvent, FleetResultStatus, OUTPUT_MAX_BYTES};
use crate::fleet_repo;
use crate::servers::repo::{self, ServerRow};
use crate::ssh::{self, HostKeyMode};
use crate::state::AppState;

const MAX_CONCURRENT_TARGETS: usize = 4;
const REMOTE_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const PRUNE_LOCK_TTL_SECS: i64 = 180;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OperationPayload {
    pub command: Option<String>,
    pub container_id: Option<String>,
    pub exec_command: Option<String>,
}

pub fn spawn(state: AppState, operation_id: String) {
    tokio::spawn(async move {
        if let Err(err) = jalankan_operasi(&state, &operation_id).await {
            tracing::warn!(error = %err, operation_id, "operasi armada gagal di level pengendali");
            let _ = fleet_repo::set_status(&state.db_write, &operation_id, "failed").await;
            state.fleet_events.publish(
                &operation_id,
                FleetEvent {
                    operation_id: operation_id.clone(),
                    status: "failed".to_string(),
                    server_id: None,
                    message: Some(
                        "Operasi armada gagal sebelum semua target diproses.".to_string(),
                    ),
                },
            );
            state.fleet_events.remove(&operation_id);
        }
    });
}

async fn jalankan_operasi(state: &AppState, operation_id: &str) -> Result<()> {
    let operation = fleet_repo::find_operation(&state.db_read, operation_id)
        .await?
        .context("operasi armada tidak ditemukan")?;
    let payload_row = sqlx::query!(
        "SELECT payload_json FROM fleet_operations WHERE id = ?",
        operation_id
    )
    .fetch_one(&state.db_read)
    .await
    .context("baca payload operasi armada")?;
    let payload_plain = state
        .crypto
        .decrypt(&payload_row.payload_json)
        .context("payload operasi armada tidak bisa didekripsi")?;
    let payload: OperationPayload =
        serde_json::from_str(&payload_plain).context("payload operasi armada rusak")?;

    fleet_repo::set_status(&state.db_write, operation_id, "running").await?;
    publish(state, operation_id, "running", None, None);

    let target_ids: HashSet<String> = operation.targets.iter().cloned().collect();
    let mut tasks = JoinSet::new();
    let mut targets = operation.targets.into_iter();
    for server_id in targets.by_ref().take(MAX_CONCURRENT_TARGETS) {
        spawn_target(
            &mut tasks,
            state.clone(),
            operation_id.to_string(),
            operation.kind.clone(),
            server_id,
            payload.clone(),
        );
    }

    let mut hasil = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(outcome)) => {
                hasil.push(outcome);
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, operation_id, "target operasi armada gagal");
            }
            Err(err) => {
                tracing::warn!(error = %err, operation_id, "task target operasi armada dibatalkan")
            }
        }
        if let Some(server_id) = targets.next() {
            spawn_target(
                &mut tasks,
                state.clone(),
                operation_id.to_string(),
                operation.kind.clone(),
                server_id,
                payload.clone(),
            );
        }
    }

    let recorded: HashSet<String> = hasil
        .iter()
        .map(|outcome| outcome.server_id.clone())
        .collect();
    for server_id in target_ids.difference(&recorded) {
        if let Err(err) = fleet_repo::insert_result(
            &state.db_write,
            &state.config.log_dir.join("operations"),
            operation_id,
            server_id,
            None,
            None,
            FleetResultStatus::Failed,
        )
        .await
        {
            tracing::warn!(error = %err, operation_id, server_id, "gagal menyimpan hasil target yang task-nya hilang");
        }
        hasil.push(TargetOutcome {
            server_id: server_id.clone(),
            status: FleetResultStatus::Failed,
        });
    }

    let sukses = hasil
        .iter()
        .filter(|outcome| outcome.status == FleetResultStatus::Succeeded)
        .count();
    let gagal = hasil.len().saturating_sub(sukses);
    let status = if gagal == 0 {
        "succeeded"
    } else if sukses == 0 {
        "failed"
    } else {
        "partial"
    };
    fleet_repo::set_status(&state.db_write, operation_id, status).await?;
    publish(
        state,
        operation_id,
        status,
        None,
        Some(if status == "succeeded" {
            "Operasi selesai di semua server yang dipilih.".to_string()
        } else if status == "partial" {
            "Operasi selesai sebagian; buka hasil per server untuk melihat kegagalan.".to_string()
        } else {
            "Operasi gagal di semua server yang diproses.".to_string()
        }),
    );
    state.fleet_events.remove(operation_id);
    Ok(())
}

struct TargetOutcome {
    server_id: String,
    status: FleetResultStatus,
}

fn spawn_target(
    tasks: &mut JoinSet<Result<TargetOutcome>>,
    state: AppState,
    operation_id: String,
    kind: String,
    server_id: String,
    payload: OperationPayload,
) {
    tasks.spawn(async move {
        let outcome = match kind.as_str() {
            "command" => {
                jalankan_command(
                    &state,
                    &operation_id,
                    &server_id,
                    payload.command.as_deref(),
                )
                .await
            }
            "prune" => jalankan_prune(&state, &operation_id, &server_id).await,
            _ => Err(anyhow!("jenis operasi armada tidak didukung")),
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(err) => {
                tracing::warn!(error = %err, operation_id, server_id, "target armada gagal sebelum hasil tersimpan");
                if let Err(simpan_err) = fleet_repo::insert_result(
                    &state.db_write,
                    &state.config.log_dir.join("operations"),
                    &operation_id,
                    &server_id,
                    None,
                    None,
                    FleetResultStatus::Failed,
                )
                .await
                {
                    tracing::warn!(error = %simpan_err, operation_id, server_id, "gagal menyimpan hasil target armada");
                }
                TargetOutcome {
                    server_id: server_id.clone(),
                    status: FleetResultStatus::Failed,
                }
            }
        };
        publish(
            &state,
            &operation_id,
            outcome.status.as_db_str(),
            Some(server_id.clone()),
            None,
        );
        Ok(outcome)
    });
}

async fn jalankan_command(
    state: &AppState,
    operation_id: &str,
    server_id: &str,
    command: Option<&str>,
) -> Result<TargetOutcome> {
    let command = command.context("perintah armada tidak tersedia")?;
    let server = server_online(state, server_id).await?;
    let session = match buka_ssh(state, &server).await {
        Ok(session) => session,
        Err(_err) => {
            tulis_hasil_gagal(
                state,
                operation_id,
                server_id,
                "Server tidak dapat dijangkau.",
            )
            .await?;
            return Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Failed,
            });
        }
    };
    let result = ssh::exec_bounded(
        &session,
        "sh",
        &["-c", command],
        REMOTE_COMMAND_TIMEOUT,
        OUTPUT_MAX_BYTES,
    )
    .await;
    let _ = session.close().await;

    match result {
        Ok((output, truncated)) => {
            let mut text = String::new();
            text.push_str(&output.stdout);
            if !output.stderr.is_empty() {
                text.push_str("\n[stderr]\n");
                text.push_str(&output.stderr);
            }
            if truncated {
                text.push_str("\n\n[Keluaran dipotong karena melewati batas ukuran.]\n");
            }
            let path = tulis_hasil(state, operation_id, server_id, &text).await?;
            let status = if output.success() {
                FleetResultStatus::Succeeded
            } else {
                FleetResultStatus::Failed
            };
            fleet_repo::insert_result(
                &state.db_write,
                &state.config.log_dir.join("operations"),
                operation_id,
                server_id,
                Some(i64::from(output.code)),
                Some(&path.to_string_lossy()),
                status,
            )
            .await?;
            Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status,
            })
        }
        Err(_err) => {
            let message = match _err {
                ssh::SshExecError::Timeout => "Perintah melewati batas waktu.",
                ssh::SshExecError::Disconnected => {
                    "Koneksi server terputus saat perintah berjalan."
                }
                ssh::SshExecError::Other(_) => "Perintah gagal di lapisan transport.",
            };
            tulis_hasil_gagal(state, operation_id, server_id, message).await?;
            Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Failed,
            })
        }
    }
}

async fn jalankan_prune(
    state: &AppState,
    operation_id: &str,
    server_id: &str,
) -> Result<TargetOutcome> {
    let server = match server_online(state, server_id).await {
        Ok(server) => server,
        Err(_) => {
            fleet_repo::insert_result(
                &state.db_write,
                &state.config.log_dir.join("operations"),
                operation_id,
                server_id,
                None,
                None,
                FleetResultStatus::Skipped,
            )
            .await?;
            return Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Skipped,
            });
        }
    };
    let lock_token = format!("fleet-prune-{operation_id}-{server_id}");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let locked = apps_repo::acquire_server_locks(
        &state.db_write,
        server_id,
        &lock_token,
        now + PRUNE_LOCK_TTL_SECS,
    )
    .await?;
    if !locked {
        tulis_hasil_gagal(
            state,
            operation_id,
            server_id,
            "Prune dilewati karena deployment masih berjalan.",
        )
        .await?;
        return Ok(TargetOutcome {
            server_id: server_id.to_string(),
            status: FleetResultStatus::Skipped,
        });
    }
    let session = match buka_ssh(state, &server).await {
        Ok(session) => session,
        Err(_) => {
            fleet_repo::insert_result(
                &state.db_write,
                &state.config.log_dir.join("operations"),
                operation_id,
                server_id,
                None,
                None,
                FleetResultStatus::Skipped,
            )
            .await?;
            apps_repo::release_server_locks(&state.db_write, server_id, &lock_token).await?;
            return Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Skipped,
            });
        }
    };
    let result = prune_server(state, operation_id, server_id, &server, &session).await;
    let _ = session.close().await;
    apps_repo::release_server_locks(&state.db_write, server_id, &lock_token).await?;
    result
}

async fn prune_server(
    state: &AppState,
    operation_id: &str,
    server_id: &str,
    _server: &ServerRow,
    session: &ssh::SshSession,
) -> Result<TargetOutcome> {
    let forward = match docker::establish(session, &state.config.runtime_dir, server_id).await {
        Ok(forward) => forward,
        Err(_) => {
            fleet_repo::insert_result(
                &state.db_write,
                &state.config.log_dir.join("operations"),
                operation_id,
                server_id,
                None,
                None,
                FleetResultStatus::Skipped,
            )
            .await?;
            return Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Skipped,
            });
        }
    };
    let result = async {
        let client = docker::connect(forward.socket_path())
            .map_err(|_| anyhow!("Docker tidak terjangkau"))?;
        docker::ping(&client)
            .await
            .map_err(|_| anyhow!("Docker tidak merespons"))?;
        let images = docker::list_images(&client)
            .await
            .map_err(|_| anyhow!("daftar image gagal"))?;
        let containers = docker::list_containers_with_label(&client, "platform.deployment")
            .await
            .map_err(|_| anyhow!("daftar container gagal"))?;
        let apps = apps_repo::list_by_server(&state.db_read, server_id).await?;
        let mut deployments = Vec::new();
        for app in apps {
            deployments.extend(deployments_repo::list_by_app(&state.db_read, &app.id).await?);
        }
        let protected_containers: HashSet<String> = containers
            .iter()
            .filter_map(|container| container.labels.get("platform.digest").cloned())
            .collect();
        let image_digests: Vec<String> = images
            .iter()
            .flat_map(|image| image.repo_digests.iter().cloned())
            .collect();
        let candidates =
            retention::kandidat_penghapusan(&deployments, &image_digests, &protected_containers);
        let mut removed = 0_u64;
        let mut failed = 0_u64;
        for digest in candidates {
            let renewed = apps_repo::renew_server_lock(
                &state.db_write,
                server_id,
                &format!("fleet-prune-{operation_id}-{server_id}"),
                time::OffsetDateTime::now_utc().unix_timestamp() + PRUNE_LOCK_TTL_SECS,
            )
            .await?;
            if !renewed {
                return Err(anyhow!("lease prune hilang sebelum penghapusan image"));
            }
            match docker::remove_image(&client, &digest).await {
                Ok(_) => removed += 1,
                Err(err) => {
                    failed += 1;
                    tracing::warn!(error = ?err, server_id, "gagal menghapus satu image kandidat prune");
                }
            }
        }
        if failed > 0 {
            return Err(anyhow!("{failed} image gagal dihapus; {removed} image berhasil dihapus"));
        }
        Ok::<String, anyhow::Error>(format!("Image tidak terpakai dihapus: {removed}."))
    }
    .await;
    docker::close(session, forward).await;
    match result {
        Ok(message) => {
            let path = tulis_hasil(state, operation_id, server_id, &message).await?;
            fleet_repo::insert_result(
                &state.db_write,
                &state.config.log_dir.join("operations"),
                operation_id,
                server_id,
                Some(0),
                Some(&path.to_string_lossy()),
                FleetResultStatus::Succeeded,
            )
            .await?;
            Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Succeeded,
            })
        }
        Err(err) => {
            tracing::warn!(error = %err, server_id, "prune server gagal");
            fleet_repo::insert_result(
                &state.db_write,
                &state.config.log_dir.join("operations"),
                operation_id,
                server_id,
                None,
                None,
                FleetResultStatus::Failed,
            )
            .await?;
            Ok(TargetOutcome {
                server_id: server_id.to_string(),
                status: FleetResultStatus::Failed,
            })
        }
    }
}

async fn server_online(state: &AppState, server_id: &str) -> Result<ServerRow> {
    let server = repo::find_by_id(&state.db_read, server_id)
        .await?
        .context("server operasi tidak ditemukan")?;
    if server.status != "online" || server.host_key_fingerprint.is_none() {
        return Err(anyhow!("server tidak online"));
    }
    Ok(server)
}

async fn buka_ssh(state: &AppState, server: &ServerRow) -> Result<ssh::SshSession> {
    let fingerprint = server
        .host_key_fingerprint
        .clone()
        .context("fingerprint server kosong")?;
    let key = state
        .crypto
        .decrypt(&server.ssh_key_encrypted)
        .context("dekripsi kunci SSH operasi")?;
    let outcome = ssh::connect(
        &server.host,
        server.port as u16,
        &server.ssh_user,
        &key,
        &state.config.runtime_dir,
        HostKeyMode::Strict {
            expected_fingerprint: fingerprint,
        },
    )
    .await
    .map_err(|_| anyhow!("koneksi SSH operasi gagal"))?;
    match outcome {
        ssh::ConnectOutcome::Established(session) => Ok(session),
        ssh::ConnectOutcome::TofuPending { session, .. } => {
            let _ = session.close().await;
            Err(anyhow!("host key belum dikonfirmasi"))
        }
    }
}

async fn tulis_hasil(
    state: &AppState,
    operation_id: &str,
    server_id: &str,
    text: &str,
) -> Result<PathBuf> {
    let dir = state.config.log_dir.join("operations").join(operation_id);
    tokio::fs::create_dir_all(&dir)
        .await
        .context("bikin direktori output operasi")?;
    tokio::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .await
        .context("set izin direktori output operasi")?;
    let path = dir.join(format!("{server_id}.out"));
    let bytes = text.as_bytes();
    let (bounded, truncated) = fleet::bounded_output(bytes);
    let mut content = bounded;
    if truncated {
        content.push_str("\n[Keluaran dipotong karena melewati batas ukuran.]\n");
    }
    tokio::fs::write(&path, content.as_bytes())
        .await
        .context("simpan output operasi")?;
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .await
        .context("set izin file output operasi")?;
    Ok(path)
}

async fn tulis_hasil_gagal(
    state: &AppState,
    operation_id: &str,
    server_id: &str,
    message: &str,
) -> Result<()> {
    let path = tulis_hasil(state, operation_id, server_id, message).await?;
    fleet_repo::insert_result(
        &state.db_write,
        &state.config.log_dir.join("operations"),
        operation_id,
        server_id,
        None,
        Some(&path.to_string_lossy()),
        FleetResultStatus::Failed,
    )
    .await
}

fn publish(
    state: &AppState,
    operation_id: &str,
    status: &str,
    server_id: Option<String>,
    message: Option<String>,
) {
    state.fleet_events.publish(
        operation_id,
        FleetEvent {
            operation_id: operation_id.to_string(),
            status: status.to_string(),
            server_id,
            message,
        },
    );
}

/// Eksekusi satu command Docker container dan mengembalikan output terbatas.
/// Dipakai endpoint `POST /fleet/exec`; operasi ini tidak disimpan sebagai
/// fleet job karena SSE menunggu satu sesi remote yang pendek.
/// Hapus output operasi yang lebih tua dari 30 hari. Isi output bisa memuat
/// secret dari perintah operator, jadi retensi file mengikuti kebijakan log
/// dan tidak pernah dilakukan lewat SQLite.
pub async fn sapu_output_lama(state: &AppState) {
    let root = state.config.log_dir.join("operations");
    let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
        return;
    };
    let batas = std::time::Duration::from_secs(30 * 24 * 60 * 60);
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified.elapsed().is_ok_and(|umur| umur > batas)
            && let Err(err) = tokio::fs::remove_dir_all(entry.path()).await
        {
            tracing::warn!(error = %err, "gagal menghapus output operasi lama");
        }
    }
}

pub async fn exec_container_once(
    state: &AppState,
    server_id: &str,
    container_id: &str,
    command: &str,
) -> Result<(String, i64, bool)> {
    let server = server_online(state, server_id).await?;
    let session = buka_ssh(state, &server).await?;
    let forward = docker::establish(&session, &state.config.runtime_dir, server_id)
        .await
        .map_err(|_| anyhow!("forward Docker exec gagal"))?;
    let result = async {
        let client = docker::connect(forward.socket_path())
            .map_err(|_| anyhow!("Docker tidak terjangkau"))?;
        let options = bollard::exec::CreateExecOptions::<String> {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ]),
            ..Default::default()
        };
        let created = client
            .create_exec(container_id, options)
            .await
            .context("buat sesi exec container")?;
        let started = client
            .start_exec(
                &created.id,
                Some(bollard::exec::StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: Some(OUTPUT_MAX_BYTES),
                }),
            )
            .await
            .context("mulai sesi exec container")?;
        let mut output = match started {
            bollard::exec::StartExecResults::Attached { output, input: _ } => output,
            bollard::exec::StartExecResults::Detached => {
                return Err(anyhow!("sesi exec container tidak terpasang"));
            }
        };
        let session_result = tokio::time::timeout(
            Duration::from_secs(crate::fleet::EXEC_TIMEOUT_SECS),
            async {
                let mut bytes = Vec::new();
                let mut truncated = false;
                while let Some(item) = output.next().await {
                    let chunk = item.context("stream exec container gagal")?;
                    let remaining = OUTPUT_MAX_BYTES.saturating_sub(bytes.len());
                    if remaining == 0 {
                        truncated = true;
                        break;
                    }
                    let chunk_bytes = chunk.to_string();
                    if chunk_bytes.len() > remaining {
                        bytes.extend_from_slice(&chunk_bytes.as_bytes()[..remaining]);
                        truncated = true;
                        break;
                    }
                    bytes.extend_from_slice(chunk_bytes.as_bytes());
                }
                Ok::<_, anyhow::Error>((bytes, truncated))
            },
        )
        .await
        .map_err(|_| anyhow!("exec container melewati batas waktu"))??;
        let inspect = client
            .inspect_exec(&created.id)
            .await
            .context("baca status exec container")?;
        let (text, truncated_again) = fleet::bounded_output(&session_result.0);
        Ok::<_, anyhow::Error>((
            text,
            inspect.exit_code.unwrap_or(-1),
            session_result.1 || truncated_again,
        ))
    }
    .await;
    docker::close(&session, forward).await;
    let _ = session.close().await;
    result
}
