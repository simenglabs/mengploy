//! Detail server dan panel metrik Fase 6.

use maud::{Markup, html};

use crate::metrics::{ContainerMetricPoint, DeploymentMarker, MetricDashboard};
use crate::registries::repo::RegistryRingkas;
use crate::servers::model::{ServerRingkas, StatusServer};
use crate::web::fleet::{badge, format_epoch_opt};
use crate::web::layout::{app_shell, base_page};

pub fn render_server_detail(
    server: &ServerRingkas,
    registries_tertaut: &[RegistryRingkas],
    metrics: &MetricDashboard,
    range_hours: u32,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.detail-title-row {
            h1 { "Detail Server: " (server.name) }
            (badge(server.status))
        }
        @match server.status {
            StatusServer::Unreachable => (panel_unreachable(server, csrf_token)),
            StatusServer::Pending => (panel_pending(&server.id)),
            StatusServer::Verifying => (panel_verifying(&server.id)),
            StatusServer::Online => {}
        }
        div.detail-grid {
            section.detail-card aria-labelledby="judul-jaringan" {
                h2 id="judul-jaringan" { "Kredensial & Jaringan" }
                div.detail-row { span { "Alamat Host / IP" } span { (server.host) } }
                div.detail-row { span { "Port SSH" } span { (server.port) } }
                div.detail-row { span { "Pengguna SSH" } span { (server.ssh_user) } }
                div.detail-row { span { "Terakhir Dilihat" } span { (format_epoch_opt(server.last_seen_at)) } }
            }
            section.detail-card aria-labelledby="judul-os" {
                h2 id="judul-os" { "Spesifikasi & Lingkungan" }
                div.detail-row { span { "Versi Docker" } span { (belum_atau(&server.docker_version, server.status)) } }
                div.detail-row { span { "Informasi OS" } span { (belum_atau(&server.os_info, server.status)) } }
                div.detail-row {
                    span { "Sidik Jari Host Key" }
                    span { @match &server.host_key_fingerprint {
                        Some(fp) => code.host-key { (fp) },
                        None => "-",
                    }}
                }
            }
        }
        section.detail-card aria-labelledby="judul-registry" {
            h2 id="judul-registry" { "Registry Terintegrasi" }
            @if registries_tertaut.is_empty() {
                p { "Tidak ada registry yang ditautkan ke server ini." }
            } @else {
                ul { @for r in registries_tertaut { li { (r.host) " (User: " (r.username) ")" } } }
            }
        }
        (panel_metrik(metrics, range_hours, &server.id))
    };
    base_page(
        &format!("Detail Server {} - Mengploy", server.name),
        app_shell(Some(csrf_token), strip, content),
    )
}

fn panel_metrik(metrics: &MetricDashboard, range_hours: u32, server_id: &str) -> Markup {
    let ada_data = !metrics.host.is_empty() || !metrics.containers.is_empty();
    html! {
        section.detail-card.metrics-panel aria-labelledby="judul-metrik" {
            div.metrics-header {
                div { h2 id="judul-metrik" { "Metrik Kinerja" } p.metrics-caption { "Host dan container • data historis tanpa daftar proses atau environment" } }
                nav.metrics-range aria-label="Rentang metrik" {
                    @for (hours, label) in [(1_u32, "1j"), (6, "6j"), (24, "24j"), (168, "7h")] {
                        a class=(if hours == range_hours { "range-active" } else { "" }) href=(format!("/servers/{server_id}?range={hours}")) { (label) }
                    }
                }
            }
            @if !ada_data {
                div.metrics-empty { strong { "Belum ada sampel metrik" } p { "Worker akan menampilkan grafik setelah pengumpulan pertama berhasil. Server tidak terjangkau membuat celah data, bukan nilai nol." } }
            } @else {
                div.metric-cards {
                    (metric_card("CPU host", "persen", metrics.host.iter().map(|p| p.cpu_avg.unwrap_or(0.0)).collect()))
                    (metric_card("Memori host", "byte", metrics.host.iter().map(|p| p.mem_used as f64).collect()))
                    (metric_card("Disk host", "persen", metrics.host.iter().map(|p| if p.disk_total > 0 { p.disk_used as f64 / p.disk_total as f64 * 100.0 } else { 0.0 }).collect()))
                }
                section.metric-chart aria-labelledby="judul-grafik-host" {
                    h3 id="judul-grafik-host" { "Sampel host" }
                    (render_host_chart(metrics))
                }
                section.metric-chart aria-labelledby="judul-grafik-container" {
                    h3 id="judul-grafik-container" { "Container" }
                    @if metrics.containers.is_empty() { p.metrics-empty { "Belum ada container berlabel platform." } }
                    @else { @for container in group_containers(metrics) { div.container-chart { h4 { "Container " (container.0) } (render_container_chart(&container.1)) } } }
                }
            }
            @if !visible_deployments(metrics).is_empty() {
                p.deployment-markers { "Penanda deployment: " @for marker in visible_deployments(metrics) { code { (format_epoch_opt(Some(marker.ts))) } " " (marker.label) "  " } }
            }
            section.alert-panel aria-labelledby="judul-alert" {
                h3 id="judul-alert" { "Alert aktif" }
                @if metrics.alerts.is_empty() { p { "Tidak ada alert aktif. Alert yang pulih tidak lagi ditampilkan." } }
                @else { ul.alert-list { @for alert in &metrics.alerts { li class=(format!("alert-item alert-{}", alert.severity)) { strong { (format_alert_kind(&alert.kind)) } " — " (alert.message) " (" (alert.target) ")" } } } }
            }
        }
    }
}

fn metric_card(label: &str, unit: &str, values: Vec<f64>) -> Markup {
    let latest = values.last().copied().unwrap_or(0.0);
    let scale = values.iter().copied().fold(0.0_f64, f64::max);
    let spark = values
        .iter()
        .map(|value| {
            spark_char(if scale > 0.0 {
                value / scale * 100.0
            } else {
                0.0
            })
        })
        .collect::<String>();
    html! { div.metric-card { span.metric-card-label { (label) } strong.metric-card-value { (format_metric(latest, unit)) } code.sparkline aria-label=(format!("Sparkline {label}")) { (spark) } } }
}

fn group_containers(metrics: &MetricDashboard) -> Vec<(String, Vec<&ContainerMetricPoint>)> {
    let mut groups: Vec<(String, Vec<&ContainerMetricPoint>)> = Vec::new();
    for point in &metrics.containers {
        if let Some(group) = groups.iter_mut().find(|(id, _)| id == &point.container_id) {
            group.1.push(point);
        } else {
            groups.push((point.container_id.clone(), vec![point]));
        }
    }
    groups
}

fn render_host_chart(metrics: &MetricDashboard) -> Markup {
    let cpu = metrics
        .host
        .iter()
        .map(|p| p.cpu_avg.unwrap_or(0.0))
        .collect::<Vec<_>>();
    let memory = metrics
        .host
        .iter()
        .map(|p| p.mem_used as f64)
        .collect::<Vec<_>>();
    let disk = metrics
        .host
        .iter()
        .map(|p| {
            if p.disk_total > 0 {
                p.disk_used as f64 / p.disk_total as f64 * 100.0
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    html! {
        div.chart-wrap {
            svg.chart-svg viewBox="0 0 100 40" role="img" aria-label="Grafik host; setiap seri dinormalisasi terhadap rentangnya" {
                line x1="0" y1="39" x2="100" y2="39" class="chart-axis" {}
                polyline points=(svg_points(&cpu)) class="chart-line chart-cpu" {}
                polyline points=(svg_points(&memory)) class="chart-line chart-memory" {}
                polyline points=(svg_points(&disk)) class="chart-line chart-disk" {}
                @for point in deployment_positions(metrics) { line x1=(point) y1="0" x2=(point) y2="40" class="chart-deployment" {} }
            }
            p.chart-legend { "CPU (hijau), Memori (kuning), Disk (biru); tiap seri dinormalisasi 0–100% • garis vertikal = deployment" }
            pre.chart-data { (render_host_summary(metrics)) }
        }
    }
}

fn render_container_chart(points: &[&ContainerMetricPoint]) -> Markup {
    let cpu = points
        .iter()
        .map(|p| p.cpu_avg.unwrap_or(0.0))
        .collect::<Vec<_>>();
    let memory = points
        .iter()
        .map(|p| p.mem_bytes as f64)
        .collect::<Vec<_>>();
    html! { div.chart-wrap { svg.chart-svg viewBox="0 0 100 40" role="img" aria-label="Grafik CPU dan memori container" { line x1="0" y1="39" x2="100" y2="39" class="chart-axis" {} polyline points=(svg_points(&cpu)) class="chart-line chart-cpu" {} polyline points=(svg_points(&memory)) class="chart-line chart-memory" {} } pre.chart-data { (render_container_summary(points)) } } }
}

fn svg_points(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let max = values.iter().copied().fold(0.0_f64, f64::max);
    let denominator = (values.len().saturating_sub(1)).max(1) as f64;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = index as f64 / denominator * 100.0;
            let y = 39.0 - if max > 0.0 { value / max * 36.0 } else { 0.0 };
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn deployment_positions(metrics: &MetricDashboard) -> Vec<String> {
    let Some(first) = metrics.host.first().map(|p| p.ts) else {
        return Vec::new();
    };
    let Some(last) = metrics.host.last().map(|p| p.ts) else {
        return Vec::new();
    };
    if last <= first {
        return Vec::new();
    }
    let span = (last - first) as f64;
    metrics
        .deployments
        .iter()
        .filter_map(|deployment| {
            if deployment.ts < first || deployment.ts > last {
                None
            } else {
                Some(((deployment.ts - first) as f64 / span * 100.0).to_string())
            }
        })
        .collect()
}

fn visible_deployments(metrics: &MetricDashboard) -> Vec<&DeploymentMarker> {
    let (Some(first), Some(last)) = (
        metrics.host.first().map(|p| p.ts),
        metrics.host.last().map(|p| p.ts),
    ) else {
        return Vec::new();
    };
    metrics
        .deployments
        .iter()
        .filter(|deployment| deployment.ts >= first && deployment.ts <= last)
        .collect()
}

fn render_host_summary(metrics: &MetricDashboard) -> String {
    metrics
        .host
        .iter()
        .enumerate()
        .map(|(index, point)| {
            format!(
                "{:02} CPU {:>6.1}% max {:>6.1}% RAM {:>10} B Disk {:>6.1}%",
                index + 1,
                point.cpu_avg.unwrap_or(0.0),
                point.cpu_max.unwrap_or(0.0),
                point.mem_used,
                if point.disk_total > 0 {
                    point.disk_used as f64 / point.disk_total as f64 * 100.0
                } else {
                    0.0
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_container_summary(points: &[&ContainerMetricPoint]) -> String {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            format!(
                "{:02} CPU {:>6.1}% max {:>6.1}% RAM {:>10} B restart {}",
                index + 1,
                point.cpu_avg.unwrap_or(0.0),
                point.cpu_max.unwrap_or(0.0),
                point.mem_bytes,
                point.restart_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn spark_char(value: f64) -> char {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    LEVELS[(value.clamp(0.0, 100.0) / 100.0 * 7.0).round() as usize]
}
fn format_metric(value: f64, unit: &str) -> String {
    if unit == "persen" {
        format!("{value:.1}%")
    } else {
        format!("{value:.0} {unit}")
    }
}
fn format_alert_kind(kind: &str) -> &str {
    match kind {
        "disk_high" => "Disk tinggi",
        "restart_loop" => "Restart berulang",
        "resource_spike" => "Lonjakan resource",
        _ => "Alert metrik",
    }
}
fn belum_atau(value: &Option<String>, status: StatusServer) -> String {
    match value {
        Some(v) => v.clone(),
        None => {
            if status == StatusServer::Verifying {
                "Sedang diverifikasi".to_string()
            } else {
                "Belum terverifikasi".to_string()
            }
        }
    }
}

fn panel_unreachable(server: &ServerRingkas, csrf_token: &str) -> Markup {
    html! { div.error-box.danger { h2 { "[x] Server Tidak Terjangkau" } @if let Some(pesan) = &server.last_error_message { p { (pesan) } } @else { p { "Server gagal dihubungi berturut-turut oleh worker polling." } } form method="post" action=(format!("/servers/{}/verifikasi/ulang", server.id)) { input type="hidden" name="csrf_token" value=(csrf_token); button.btn type="submit" { "Mulai Verifikasi Ulang" } } } }
}
// href memakai path ABSOLUT `/servers/{id}/verifikasi` — href relatif
// "verifikasi" dari halaman `/servers/{id}` meresolusi ke `/servers/verifikasi`
// yang tidak punya route → 404 (regresi pernah terjadi, dijaga test di bawah).
fn panel_pending(server_id: &str) -> Markup {
    html! { div.alert.alert-warning { span { "[!] Server ini belum diverifikasi. Silakan jalankan proses pemeriksaan sistem." } " " a.btn href=(format!("/servers/{server_id}/verifikasi")) { "Jalankan Verifikasi" } } }
}
fn panel_verifying(server_id: &str) -> Markup {
    html! { div.alert.alert-warning { span { "[*] Proses verifikasi sistem sedang berlangsung." } " " a.btn href=(format!("/servers/{server_id}/verifikasi")) { "Lihat Progres Verifikasi" } } }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(status: StatusServer) -> ServerRingkas {
        ServerRingkas {
            id: "srv-1".to_string(),
            name: "vps-sg-1".to_string(),
            host: "1.2.3.4".to_string(),
            port: 22,
            ssh_user: "root".to_string(),
            status,
            last_seen_at: None,
            docker_version: None,
            os_info: None,
            host_key_fingerprint: Some("SHA256:abc123".to_string()),
            consecutive_failures: 3,
            last_error_kind: Some("host_unreachable".to_string()),
            last_error_message: Some("Gagal terhubung ke host target.".to_string()),
        }
    }
    #[test]
    fn detail_online_menampilkan_panel_metrik_dan_rentang() {
        let markup = render_server_detail(
            &server(StatusServer::Online),
            &[],
            &MetricDashboard::default(),
            6,
            "token",
            None,
        )
        .into_string();
        assert!(markup.contains("Metrik Kinerja"));
        assert!(markup.contains("Belum ada sampel metrik"));
        assert!(markup.contains("range-active"));
    }
    #[test]
    fn detail_pending_dan_verifying_memakai_href_absolut_verifikasi() {
        // Regresi: href relatif "verifikasi" dari /servers/{id} meresolusi ke
        // /servers/verifikasi yang 404. Link harus absolut.
        let pending = render_server_detail(
            &server(StatusServer::Pending),
            &[],
            &MetricDashboard::default(),
            6,
            "tok",
            None,
        )
        .into_string();
        assert!(
            pending.contains(r#"href="/servers/srv-1/verifikasi""#),
            "link pending harus absolut: {pending}"
        );
        assert!(
            !pending.contains(r#"href="verifikasi""#),
            "href relatif verifikasi dilarang: {pending}"
        );

        let verifying = render_server_detail(
            &server(StatusServer::Verifying),
            &[],
            &MetricDashboard::default(),
            6,
            "tok",
            None,
        )
        .into_string();
        assert!(
            verifying.contains(r#"href="/servers/srv-1/verifikasi""#),
            "link verifying harus absolut: {verifying}"
        );
        assert!(
            !verifying.contains(r#"href="verifikasi""#),
            "href relatif verifikasi dilarang: {verifying}"
        );
    }

    #[test]
    fn detail_unreachable_menampilkan_pesan_dan_tombol_verifikasi_ulang() {
        let markup = render_server_detail(
            &server(StatusServer::Unreachable),
            &[],
            &MetricDashboard::default(),
            6,
            "tok",
            None,
        )
        .into_string();
        assert!(markup.contains("Gagal terhubung ke host target."));
        assert!(markup.contains(r#"action="/servers/srv-1/verifikasi/ulang"#));
    }
    #[test]
    fn detail_tanpa_private_key_tidak_pernah_muncul() {
        let markup = render_server_detail(
            &server(StatusServer::Online),
            &[],
            &MetricDashboard::default(),
            6,
            "tok",
            None,
        )
        .into_string();
        assert!(!markup.contains("PRIVATE KEY"));
    }
    #[test]
    fn grafik_ssr_memakai_svg_dan_memfilter_marker() {
        let dashboard = MetricDashboard {
            host: vec![
                crate::metrics::HostMetricPoint {
                    ts: 100,
                    cpu_avg: Some(10.0),
                    cpu_max: Some(20.0),
                    mem_used: 10,
                    mem_total: 100,
                    load1: 0.1,
                    disk_used: 10,
                    disk_total: 100,
                },
                crate::metrics::HostMetricPoint {
                    ts: 200,
                    cpu_avg: Some(20.0),
                    cpu_max: Some(30.0),
                    mem_used: 20,
                    mem_total: 100,
                    load1: 0.2,
                    disk_used: 20,
                    disk_total: 100,
                },
            ],
            deployments: vec![
                DeploymentMarker {
                    ts: 50,
                    label: "di luar".to_string(),
                },
                DeploymentMarker {
                    ts: 150,
                    label: "di dalam".to_string(),
                },
            ],
            ..MetricDashboard::default()
        };
        let markup = render_server_detail(
            &server(StatusServer::Online),
            &[],
            &dashboard,
            6,
            "tok",
            None,
        )
        .into_string();
        assert!(markup.contains("chart-svg"));
        assert!(markup.contains("di dalam"));
        assert!(!markup.contains("di luar"));
    }
    #[test]
    fn detail_registry_terisi_menampilkan_host_dan_username() {
        let registries = vec![RegistryRingkas {
            id: "reg-1".to_string(),
            host: "ghcr.io".to_string(),
            username: "mengdep-deployer".to_string(),
        }];
        let markup = render_server_detail(
            &server(StatusServer::Online),
            &registries,
            &MetricDashboard::default(),
            6,
            "tok",
            None,
        )
        .into_string();
        assert!(markup.contains("ghcr.io"));
        assert!(markup.contains("mengdep-deployer"));
    }
}
