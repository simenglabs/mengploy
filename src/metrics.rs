//! Domain metrik Fase 6: parsing host, koreksi statistik Docker, alert,
//! downsampling, dan tipe data dashboard. I/O berada di worker/repository.

use std::collections::HashMap;

use anyhow::{Context, Result};

pub const RES_RAW: &str = "raw";
pub const RES_MIN: &str = "min";
pub const RES_HOUR: &str = "hour";
pub const RETENSI_RAW_SECS: i64 = 6 * 60 * 60;
pub const RETENSI_MIN_SECS: i64 = 7 * 24 * 60 * 60;
pub const RETENSI_HOUR_SECS: i64 = 365 * 24 * 60 * 60;
pub const DISK_ALERT_THRESHOLD: f64 = 0.80;
pub const RESTART_ALERT_DELTA: i64 = 3;
pub const RESOURCE_SPIKE_THRESHOLD: f64 = 1.30;
pub const RESOURCE_SPIKE_DELAY_SECS: i64 = 10 * 60;
pub const RESOURCE_SPIKE_WINDOW_SECS: i64 = 60 * 60;

#[derive(Debug, Clone, PartialEq)]
pub struct CpuCounters {
    pub total: u64,
    pub idle: u64,
    pub iowait: u64,
}

#[derive(Debug, Clone)]
pub struct HostSampleInput<'a> {
    pub proc_stat: &'a str,
    pub proc_meminfo: &'a str,
    pub proc_loadavg: &'a str,
    pub df_output: &'a str,
    pub cpu_cores: i64,
    pub previous_cpu: Option<&'a CpuCounters>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostSample {
    pub cpu_percent: f64,
    pub mem_used: i64,
    pub mem_total: i64,
    pub load1: f64,
    pub disk_used: i64,
    pub disk_total: i64,
}

pub fn parse_host_sample(input: &HostSampleInput<'_>) -> Result<(HostSample, CpuCounters)> {
    let current_cpu = parse_proc_stat(input.proc_stat)?;
    let cpu_percent = cpu_percent(input.previous_cpu, &current_cpu, input.cpu_cores);
    let (mem_total, mem_available) = parse_meminfo(input.proc_meminfo)?;
    // Host `MemAvailable` sudah memperhitungkan cache yang dapat direclaim.
    // `inactive_file` hanya dikurangi pada statistik cgroup Docker.
    let mem_used = (mem_total - mem_available).max(0);
    let load1 = input
        .proc_loadavg
        .split_whitespace()
        .next()
        .context("load average 1 menit tidak tersedia")?
        .parse::<f64>()
        .context("load average 1 menit bukan angka")?;
    let (disk_used, disk_total) = parse_df(input.df_output)?;
    Ok((
        HostSample {
            cpu_percent,
            mem_used,
            mem_total,
            load1,
            disk_used,
            disk_total,
        },
        current_cpu,
    ))
}

fn parse_proc_stat(value: &str) -> Result<CpuCounters> {
    let line = value
        .lines()
        .find(|line| line.starts_with("cpu "))
        .context("baris cpu /proc/stat tidak tersedia")?;
    let fields = line.split_whitespace().skip(1).map(|value| {
        value
            .parse::<u64>()
            .context("counter /proc/stat bukan angka")
    });
    let values: Vec<u64> = fields.collect::<Result<Vec<_>>>()?;
    if values.len() < 5 {
        return Err(anyhow::anyhow!("counter CPU /proc/stat tidak lengkap"));
    }
    // Linux mendefinisikan user,nice,system,idle,iowait,irq,softirq,steal,
    // guest,guest_nice. Guest time sudah termasuk user/nice, sehingga tidak
    // dijumlahkan dua kali; seluruh field non-guest tetap menjadi total.
    let total = values
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 8 && *index != 9)
        .map(|(_, value)| *value)
        .sum();
    Ok(CpuCounters {
        total,
        idle: values[3],
        iowait: values[4],
    })
}

fn parse_meminfo(value: &str) -> Result<(i64, i64)> {
    let mut values = HashMap::new();
    for line in value.lines() {
        let Some((key, raw)) = line.split_once(':') else {
            continue;
        };
        let amount = raw
            .split_whitespace()
            .next()
            .with_context(|| format!("nilai meminfo {key} tidak tersedia"))?
            .parse::<i64>()
            .with_context(|| format!("nilai meminfo {key} bukan angka"))?;
        values.insert(key, amount * 1024);
    }
    let total = *values.get("MemTotal").context("MemTotal tidak tersedia")?;
    let available = *values
        .get("MemAvailable")
        .context("MemAvailable tidak tersedia")?;
    Ok((total, available))
}

fn parse_df(value: &str) -> Result<(i64, i64)> {
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let Some(used) = fields.next().and_then(|v| v.parse::<i64>().ok()) else {
            continue;
        };
        let Some(total) = fields.next().and_then(|v| v.parse::<i64>().ok()) else {
            continue;
        };
        return Ok((used, total));
    }
    Err(anyhow::anyhow!("baris data df tidak tersedia"))
}

pub fn cpu_percent(previous: Option<&CpuCounters>, current: &CpuCounters, cores: i64) -> f64 {
    let Some(previous) = previous else {
        return 0.0;
    };
    let total_delta = current.total.saturating_sub(previous.total);
    let idle_delta = current
        .idle
        .saturating_sub(previous.idle)
        .saturating_add(current.iowait.saturating_sub(previous.iowait));
    if total_delta == 0 || cores <= 0 {
        return 0.0;
    }
    let busy_delta = total_delta.saturating_sub(idle_delta);
    ((busy_delta as f64 / total_delta as f64) * cores as f64 * 100.0)
        .clamp(0.0, cores as f64 * 100.0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerSample {
    pub cpu_percent: f64,
    pub mem_bytes: i64,
    pub mem_max: i64,
    pub mem_limit: i64,
    pub net_rx: i64,
    pub net_tx: i64,
    pub restart_count: i64,
}

pub struct ContainerStatsInput {
    pub cpu_delta: u64,
    pub system_delta: u64,
    pub online_cpus: u32,
    pub memory_usage: u64,
    pub inactive_file: u64,
    pub memory_max: u64,
    pub memory_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub restart_count: i64,
}

pub fn container_sample(input: &ContainerStatsInput) -> ContainerSample {
    let used = input.memory_usage.saturating_sub(input.inactive_file);
    let cores = input.online_cpus.max(1);
    let cpu_percent = if input.system_delta == 0 {
        0.0
    } else {
        (input.cpu_delta as f64 / input.system_delta as f64 * f64::from(cores) * 100.0)
            .clamp(0.0, f64::from(cores) * 100.0)
    };
    ContainerSample {
        cpu_percent,
        mem_bytes: i64::try_from(used).unwrap_or(i64::MAX),
        mem_max: i64::try_from(input.memory_max).unwrap_or(i64::MAX),
        mem_limit: i64::try_from(input.memory_limit).unwrap_or(i64::MAX),
        net_rx: i64::try_from(input.net_rx).unwrap_or(i64::MAX),
        net_tx: i64::try_from(input.net_tx).unwrap_or(i64::MAX),
        restart_count: input.restart_count,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    DiskHigh,
    RestartLoop,
    ResourceSpike,
}

impl AlertKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::DiskHigh => "disk_high",
            Self::RestartLoop => "restart_loop",
            Self::ResourceSpike => "resource_spike",
        }
    }
}

pub fn disk_alert(sample: &HostSample) -> Option<AlertKind> {
    (sample.disk_total > 0
        && sample.disk_used as f64 / sample.disk_total as f64 >= DISK_ALERT_THRESHOLD)
        .then_some(AlertKind::DiskHigh)
}

pub fn restart_alert(previous: Option<i64>, current: i64) -> Option<AlertKind> {
    previous
        .is_some_and(|old| current.saturating_sub(old) >= RESTART_ALERT_DELTA)
        .then_some(AlertKind::RestartLoop)
}

pub fn resource_spike_alert(
    deployment_started_at: Option<i64>,
    now: i64,
    baseline: Option<(f64, f64)>,
    current: (f64, f64),
) -> Option<AlertKind> {
    let started = deployment_started_at?;
    if now < started + RESOURCE_SPIKE_DELAY_SECS || now > started + RESOURCE_SPIKE_WINDOW_SECS {
        return None;
    }
    let (old_cpu, old_mem) = baseline?;
    let cpu_spike = old_cpu > 0.0 && current.0 > old_cpu * RESOURCE_SPIKE_THRESHOLD;
    let mem_spike = old_mem > 0.0 && current.1 > old_mem * RESOURCE_SPIKE_THRESHOLD;
    (cpu_spike || mem_spike).then_some(AlertKind::ResourceSpike)
}

pub fn bucket_start(ts: i64, seconds: i64) -> i64 {
    ts.div_euclid(seconds) * seconds
}

pub fn rollup<T>(samples: &[T], value: impl Fn(&T) -> f64) -> Option<(f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let mut total = 0.0;
    let mut max = f64::MIN;
    for sample in samples {
        let current = value(sample);
        total += current;
        max = max.max(current);
    }
    Some((total / samples.len() as f64, max))
}

#[derive(Debug, Clone)]
pub struct HostMetricPoint {
    pub ts: i64,
    pub cpu_avg: Option<f64>,
    pub cpu_max: Option<f64>,
    pub mem_used: i64,
    pub mem_total: i64,
    pub load1: f64,
    pub disk_used: i64,
    pub disk_total: i64,
}

#[derive(Debug, Clone)]
pub struct ContainerMetricPoint {
    pub ts: i64,
    pub server_id: String,
    pub container_id: String,
    pub app_id: Option<String>,
    pub cpu_avg: Option<f64>,
    pub cpu_max: Option<f64>,
    pub mem_bytes: i64,
    pub mem_max: i64,
    pub mem_limit: i64,
    pub net_rx: i64,
    pub net_tx: i64,
    pub restart_count: i64,
}

#[derive(Debug, Clone)]
pub struct DeploymentMarker {
    pub ts: i64,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct AlertSummary {
    pub kind: String,
    pub severity: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct MetricDashboard {
    pub host: Vec<HostMetricPoint>,
    pub containers: Vec<ContainerMetricPoint>,
    pub deployments: Vec<DeploymentMarker>,
    pub alerts: Vec<AlertSummary>,
}

pub struct HostMetricWrite<'a> {
    pub server_id: &'a str,
    pub sample: &'a HostSample,
}

pub struct ContainerMetricWrite<'a> {
    pub server_id: &'a str,
    pub container_id: &'a str,
    pub app_id: Option<&'a str>,
    pub sample: &'a ContainerSample,
}

pub struct AlertWrite<'a> {
    pub server_id: &'a str,
    pub app_id: Option<&'a str>,
    pub container_id: Option<&'a str>,
    pub deployment_id: Option<&'a str>,
    pub kind: AlertKind,
    pub severity: &'a str,
    pub target: &'a str,
    pub message: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_host_delta_menggunakan_pengali_core() {
        let previous = CpuCounters {
            total: 100,
            idle: 50,
            iowait: 0,
        };
        let current = CpuCounters {
            total: 200,
            idle: 100,
            iowait: 0,
        };
        assert_eq!(cpu_percent(Some(&previous), &current, 4), 200.0);
    }

    #[test]
    fn parser_cpu_mengikutkan_irq_softirq_dan_steal_tanpa_guest_ganda() {
        let (_, counters) = parse_host_sample(&HostSampleInput {
            proc_stat: "cpu 10 20 30 40 5 6 7 8 9 10",
            proc_meminfo: "MemTotal: 1000 kB\nMemAvailable: 500 kB",
            proc_loadavg: "0.1 0.2 0.3 1/10 2",
            df_output: "100 1000",
            cpu_cores: 1,
            previous_cpu: None,
        })
        .expect("sample host valid");
        assert_eq!(counters.total, 126);
    }

    #[test]
    fn parsing_host_memavailable_tidak_double_subtract_cache() {
        let input = HostSampleInput {
            proc_stat: "cpu 100 0 0 100 0\ncpu0 0 0 0 0",
            proc_meminfo: "MemTotal: 1000 kB\nMemAvailable: 400 kB\nInactive(file): 100 kB",
            proc_loadavg: "1.25 0.50 0.10 1/100 10",
            df_output: "800 1000",
            cpu_cores: 2,
            previous_cpu: None,
        };
        let (sample, _) = parse_host_sample(&input).expect("sample host valid");
        assert_eq!(sample.mem_used, 600 * 1024);
    }

    #[test]
    fn koreksi_container_mengurangi_cache_dan_memakai_delta_cpu() {
        let sample = container_sample(&ContainerStatsInput {
            cpu_delta: 2_000,
            system_delta: 10_000,
            online_cpus: 4,
            memory_usage: 900,
            inactive_file: 700,
            memory_max: 900,
            memory_limit: 2_000,
            net_rx: 10,
            net_tx: 20,
            restart_count: 3,
        });
        assert_eq!(sample.mem_bytes, 200);
        assert_eq!(sample.cpu_percent, 80.0);
    }

    #[test]
    fn resource_spike_memiliki_window_setelah_deploy() {
        assert_eq!(
            resource_spike_alert(Some(100), 500, Some((10.0, 100.0)), (14.0, 100.0)),
            None
        );
        assert_eq!(
            resource_spike_alert(Some(100), 800, Some((10.0, 100.0)), (14.0, 100.0)),
            Some(AlertKind::ResourceSpike)
        );
        assert_eq!(
            resource_spike_alert(
                Some(100),
                100 + RESOURCE_SPIKE_WINDOW_SECS + 1,
                Some((10.0, 100.0)),
                (14.0, 100.0)
            ),
            None
        );
    }

    #[test]
    fn tiga_alert_dibatasi_pada_kontrak_fase_6() {
        let host = HostSample {
            cpu_percent: 0.0,
            mem_used: 0,
            mem_total: 100,
            load1: 0.0,
            disk_used: 81,
            disk_total: 100,
        };
        assert_eq!(disk_alert(&host), Some(AlertKind::DiskHigh));
        assert_eq!(restart_alert(Some(1), 4), Some(AlertKind::RestartLoop));
    }

    #[test]
    fn rollup_menyimpan_avg_dan_max() {
        assert_eq!(rollup(&[1.0, 3.0, 2.0], |v| *v), Some((2.0, 3.0)));
    }

    #[test]
    fn bucket_start_stabil() {
        assert_eq!(bucket_start(125, 60), 120);
        assert_eq!(bucket_start(-1, 60), -60);
    }
}
