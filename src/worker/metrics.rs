//! Worker metrik Fase 6. Satu siklus mengambil host lewat satu perintah SSH,
//! membaca stats container lewat Docker API ter-forward, lalu menulis seluruh
//! hasil siklus dalam satu transaksi. Kegagalan satu server membuat gap data;
//! worker ini tidak mengubah status konektivitas server.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::apps::repo as apps_repo;
use crate::deployments::repo as deployments_repo;
use crate::docker;
use crate::metrics::{self, AlertKind, ContainerSample, HostSample};
use crate::metrics_repo;
use crate::servers::repo::{self, ServerRow};
use crate::ssh::{self, HostKeyMode};
use crate::state::AppState;

use super::WorkerHandle;

const TICK_INTERVAL: Duration = Duration::from_secs(60);
const REMOTE_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONCURRENT_SERVERS: usize = 4;

struct ServerObservation {
    row: ServerRow,
    host: HostSample,
    cpu_counters: metrics::CpuCounters,
    containers: Vec<(
        docker::ContainerObservation,
        ContainerSample,
        Option<String>,
    )>,
}

pub fn spawn(state: AppState) -> WorkerHandle {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        let mut previous_cpu: HashMap<String, metrics::CpuCounters> = HashMap::new();
        let mut last_rollup_at = 0_i64;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let now = now_epoch();
                    jalankan_siklus(&state, now, &mut previous_cpu).await;
                    if now - last_rollup_at >= 60 {
                        if let Err(err) = metrics_repo::rollup_and_retain(&state.db_write, now).await {
                            tracing::warn!(error = %err, "gagal menjalankan rollup dan retensi metrik");
                        }
                        last_rollup_at = now;
                    }
                }
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

async fn jalankan_siklus(
    state: &AppState,
    now: i64,
    previous_cpu: &mut HashMap<String, metrics::CpuCounters>,
) {
    let servers = match repo::list_online_for_metrics(&state.db_read).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "gagal membaca server untuk worker metrik");
            return;
        }
    };
    let mut tasks: JoinSet<Result<ServerObservation>> = JoinSet::new();
    let mut remaining = servers.into_iter();
    for row in remaining.by_ref().take(MAX_CONCURRENT_SERVERS) {
        let previous = previous_cpu.get(&row.id).cloned();
        spawn_server(&mut tasks, state.clone(), row, previous);
    }
    let mut observations = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(observation)) => {
                observations.push(observation);
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "pengumpulan metrik server gagal; gap dibiarkan")
            }
            Err(err) => tracing::warn!(error = %err, "task metrik server dibatalkan"),
        }
        if let Some(row) = remaining.next() {
            let previous = previous_cpu.get(&row.id).cloned();
            spawn_server(&mut tasks, state.clone(), row, previous);
        }
    }

    match simpan_ciklus(state, now, &observations).await {
        Ok(()) => {
            for observation in &observations {
                previous_cpu.insert(observation.row.id.clone(), observation.cpu_counters.clone());
            }
        }
        Err(err) => tracing::warn!(error = %err, "gagal menyimpan satu transaksi siklus metrik"),
    }
}

fn spawn_server(
    tasks: &mut JoinSet<Result<ServerObservation>>,
    state: AppState,
    row: ServerRow,
    previous: Option<metrics::CpuCounters>,
) {
    tasks.spawn(async move { observasi_server(&state, row, previous.as_ref()).await });
}

async fn observasi_server(
    state: &AppState,
    row: ServerRow,
    previous_cpu: Option<&metrics::CpuCounters>,
) -> Result<ServerObservation> {
    let fingerprint = row
        .host_key_fingerprint
        .clone()
        .context("fingerprint host key belum tersedia")?;
    let key = state
        .crypto
        .decrypt(&row.ssh_key_encrypted)
        .map_err(|_| anyhow!("kunci SSH scanner metrik tidak bisa didekripsi"))?;
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
    .map_err(|_| anyhow!("koneksi SSH metrik gagal"))?
    {
        ssh::ConnectOutcome::Established(session) => session,
        ssh::ConnectOutcome::TofuPending { session, .. } => {
            let _ = session.close().await;
            return Err(anyhow!("host key metrik tidak konsisten"));
        }
    };

    let command = "printf '%s\\n' '__MENGDEP_STAT__'; cat /proc/stat; printf '%s\\n' '__MENGDEP_MEM__'; cat /proc/meminfo; printf '%s\\n' '__MENGDEP_LOAD__'; cat /proc/loadavg; printf '%s\\n' '__MENGDEP_DF__'; df -B1 --output=used,size /; printf '%s\\n' '__MENGDEP_CORES__'; nproc";
    let output = match ssh::exec(&session, "sh", &["-c", command], REMOTE_COMMAND_TIMEOUT).await {
        Ok(output) => output,
        Err(_) => {
            let _ = session.close().await;
            return Err(anyhow!("perintah metrik host gagal di transport SSH"));
        }
    };
    if !output.success() {
        let _ = session.close().await;
        return Err(anyhow!(
            "perintah metrik host selesai dengan exit code {}",
            output.code
        ));
    }
    let input = match parse_marked_output(&output.stdout) {
        Ok(input) => input,
        Err(err) => {
            let _ = session.close().await;
            return Err(err);
        }
    };
    let cores = match input.cores.trim().parse::<i64>() {
        Ok(cores) => cores,
        Err(err) => {
            let _ = session.close().await;
            return Err(anyhow!("jumlah core host bukan angka: {err}"));
        }
    };
    let (host, counter) = match metrics::parse_host_sample(&metrics::HostSampleInput {
        proc_stat: &input.stat,
        proc_meminfo: &input.mem,
        proc_loadavg: &input.load,
        df_output: &input.df,
        cpu_cores: cores,
        previous_cpu,
    }) {
        Ok(result) => result,
        Err(err) => {
            let _ = session.close().await;
            return Err(err);
        }
    };

    let forward = match docker::establish(&session, &state.config.runtime_dir, &row.id).await {
        Ok(forward) => forward,
        Err(_) => {
            let _ = session.close().await;
            return Err(anyhow!("forward Docker metrik gagal"));
        }
    };
    let result = async {
        let client = docker::connect(forward.socket_path())
            .map_err(|_| anyhow!("koneksi Docker metrik gagal"))?;
        docker::ping(&client)
            .await
            .map_err(|_| anyhow!("ping Docker metrik gagal"))?;
        let containers = docker::list_containers_with_label(&client, "platform.deployment")
            .await
            .map_err(|_| anyhow!("daftar container metrik gagal"))?;
        let apps = apps_repo::list_by_server(&state.db_read, &row.id).await?;
        let app_by_name: HashMap<String, String> =
            apps.into_iter().map(|app| (app.name, app.id)).collect();
        let mut samples = Vec::new();
        for container in containers {
            let stats = match docker::stats(&client, &container.id).await {
                Ok(stats) => stats,
                Err(err) => {
                    tracing::warn!(error = ?err, container_id = %container.id, "stats container gagal; container dilewati");
                    continue;
                }
            };
            let sample = metrics::container_sample(&metrics::ContainerStatsInput {
                cpu_delta: stats.cpu_delta,
                system_delta: stats.system_delta,
                online_cpus: stats.online_cpus,
                memory_usage: stats.memory_usage,
                inactive_file: stats.inactive_file,
                memory_max: stats.memory_max,
                memory_limit: stats.memory_limit,
                net_rx: stats.net_rx,
                net_tx: stats.net_tx,
                restart_count: stats.restart_count,
            });
            let app_id = container
                .labels
                .get("platform.app")
                .and_then(|name| app_by_name.get(name))
                .cloned();
            samples.push((container, sample, app_id));
        }
        Ok(samples)
    }.await;
    docker::close(&session, forward).await;
    let _ = session.close().await;
    result.map(|containers| ServerObservation {
        row,
        host,
        cpu_counters: counter,
        containers,
    })
}

struct MarkedOutput {
    stat: String,
    mem: String,
    load: String,
    df: String,
    cores: String,
}

fn parse_marked_output(value: &str) -> Result<MarkedOutput> {
    fn block<'a>(value: &'a str, start: &str, end: &str) -> Result<&'a str> {
        let after = value
            .split_once(start)
            .map(|(_, v)| v)
            .context("marker metrik host hilang")?;
        after
            .split_once(end)
            .map(|(v, _)| v)
            .context("marker metrik host berikutnya hilang")
    }
    Ok(MarkedOutput {
        stat: block(value, "__MENGDEP_STAT__\n", "__MENGDEP_MEM__")?.to_string(),
        mem: block(value, "__MENGDEP_MEM__\n", "__MENGDEP_LOAD__")?.to_string(),
        load: block(value, "__MENGDEP_LOAD__\n", "__MENGDEP_DF__")?.to_string(),
        df: block(value, "__MENGDEP_DF__\n", "__MENGDEP_CORES__")?.to_string(),
        cores: value
            .split_once("__MENGDEP_CORES__\n")
            .map(|(_, v)| v)
            .context("marker core hilang")?
            .to_string(),
    })
}

async fn simpan_ciklus(
    state: &AppState,
    now: i64,
    observations: &[ServerObservation],
) -> Result<()> {
    let mut host_writes = Vec::new();
    let mut container_writes = Vec::new();
    let mut alerts = Vec::new();
    let mut server_ids = Vec::new();

    for observation in observations {
        server_ids.push(observation.row.id.as_str());
        host_writes.push(metrics::HostMetricWrite {
            server_id: &observation.row.id,
            sample: &observation.host,
        });
        if metrics::disk_alert(&observation.host).is_some() {
            alerts.push(metrics::AlertWrite {
                server_id: &observation.row.id,
                app_id: None,
                container_id: None,
                deployment_id: None,
                kind: AlertKind::DiskHigh,
                severity: "critical",
                target: "root",
                message: "Disk host terpakai 80% atau lebih.",
            });
        }
        for (container, sample, app_id) in &observation.containers {
            container_writes.push(metrics::ContainerMetricWrite {
                server_id: &observation.row.id,
                container_id: &container.id,
                app_id: app_id.as_deref(),
                sample,
            });
            let prior = sqlx::query!(
                "SELECT restart_count FROM metrics_container
                 WHERE server_id = ? AND container_id = ? AND res = 'raw'
                 ORDER BY ts DESC LIMIT 1",
                observation.row.id,
                container.id,
            )
            .fetch_optional(&state.db_read)
            .await
            .context("baca restart container sebelumnya")?
            .map(|row| row.restart_count);
            if metrics::restart_alert(prior, sample.restart_count).is_some() {
                alerts.push(metrics::AlertWrite {
                    server_id: &observation.row.id,
                    app_id: app_id.as_deref(),
                    container_id: Some(&container.id),
                    deployment_id: container
                        .labels
                        .get("platform.deployment")
                        .map(String::as_str),
                    kind: AlertKind::RestartLoop,
                    severity: "warning",
                    target: &container.id,
                    message: "Container mengalami restart berulang.",
                });
            }
            let deployment = match container.labels.get("platform.deployment") {
                Some(deployment_id) => {
                    deployments_repo::find_by_id(&state.db_read, deployment_id).await?
                }
                None => None,
            };
            let baseline = match deployment.as_ref().and_then(|value| value.started_at) {
                Some(started_at) => {
                    metrics_repo::baseline_before_deployment(
                        &state.db_read,
                        app_id.as_deref().unwrap_or(""),
                        started_at,
                    )
                    .await?
                }
                None => None,
            };
            if metrics::resource_spike_alert(
                deployment.as_ref().and_then(|value| value.started_at),
                now,
                baseline,
                (sample.cpu_percent, sample.mem_bytes as f64),
            )
            .is_some()
            {
                alerts.push(metrics::AlertWrite {
                    server_id: &observation.row.id,
                    app_id: app_id.as_deref(),
                    container_id: Some(&container.id),
                    deployment_id: container
                        .labels
                        .get("platform.deployment")
                        .map(String::as_str),
                    kind: AlertKind::ResourceSpike,
                    severity: "warning",
                    target: &container.id,
                    message: "Pemakaian resource naik lebih dari 30% setelah deployment.",
                });
            }
        }
    }

    metrics_repo::insert_cycle(
        &state.db_write,
        now,
        &server_ids,
        &host_writes,
        &container_writes,
        &alerts,
    )
    .await
}

fn now_epoch() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}
