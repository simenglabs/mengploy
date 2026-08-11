//! Detail deployment + timeline SSE — `docs/design/deployment-detail.md`.

use maud::{Markup, html};

use crate::deployments::{DeploymentRingkas, StatusDeployment};
use crate::events::DeploymentEvent;
use crate::web::fleet::format_epoch_opt;
use crate::web::layout::{app_shell, base_page};

/// Badge status non-warna-saja, pola sama `web::fleet::badge`.
pub fn badge_deployment(status: StatusDeployment) -> Markup {
    let (class, label) = match status {
        StatusDeployment::Queued => ("pending", "ANTRE"),
        StatusDeployment::Pulling => ("verifying", "MENARIK IMAGE"),
        StatusDeployment::Starting => ("verifying", "MEMULAI"),
        StatusDeployment::Checking => ("verifying", "HEALTH CHECK"),
        StatusDeployment::Live => ("online", "LIVE"),
        StatusDeployment::Failed => ("unreachable", "GAGAL"),
        StatusDeployment::Cancelled => ("unreachable", "DIBATALKAN"),
        StatusDeployment::Unknown => ("unreachable", "TIDAK DIKETAHUI"),
    };
    html! {
        span class=(format!("status-badge {class}")) aria-label=(format!("Status: {label}")) {
            (label)
        }
    }
}

pub fn render_deployment_detail(
    dep: &DeploymentRingkas,
    app_name: &str,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.detail-title-row {
            h1 { "Deployment " code { (commit_pendek(&dep.id)) } }
            (badge_deployment(dep.status))
        }

        @if dep.status.selesai() {
            form method="post" action=(format!("/deployments/{}/rollback", dep.id)) {
                input type="hidden" name="csrf_token" value=(csrf_token);
                button.btn type="submit" { "Rollback ke deployment ini" }
            }
        }

        @if dep.status == StatusDeployment::Unknown {
            div.alert.alert-warning {
                "Status deployment tidak diketahui karena control plane kehilangan kepastian. Rollback hanya dijalankan setelah konfirmasi eksplisit."
            }
        }

        div id="deployment-timeline"
            hx-ext="sse"
            sse-connect=(format!("/events/deploy/{}", dep.id))
            sse-swap="message"
        {
            (fragmen_isi(dep, app_name))
        }
    };

    base_page(
        &format!("Deployment {} - Mengploy", app_name),
        app_shell(Some(csrf_token), strip, content),
    )
}

/// Fragmen yang di-swap SSE — dipanggil ulang setiap `DeploymentEvent`
/// (`routes::events::deploy_stream`), TIDAK pernah butuh `csrf_token`
/// (halaman ini tidak punya form mutasi).
pub fn render_deployment_fragmen(dep: &DeploymentRingkas, app_name: &str) -> Markup {
    fragmen_isi(dep, app_name)
}

fn fragmen_isi(dep: &DeploymentRingkas, app_name: &str) -> Markup {
    html! {
        div.detail-grid {
            section.detail-card aria-labelledby="judul-info" {
                h2 id="judul-info" { "Info" }
                div.detail-row { span { "App" } span { (app_name) } }
                div.detail-row { span { "Status" } span { (badge_deployment(dep.status)) } }
                div.detail-row { span { "Commit" } span { code { (commit_pendek(&dep.commit_sha)) } } }
                @if let Some(git_ref) = &dep.git_ref {
                    div.detail-row { span { "Ref" } span { (git_ref) } }
                }
                div.detail-row { span { "Image Digest" } span { code { (dep.image_digest) } } }
                div.detail-row { span { "Dibuat" } span { (format_epoch_opt(Some(dep.created_at))) } }
                @if let Some(selesai) = dep.finished_at {
                    div.detail-row { span { "Selesai" } span { (format_epoch_opt(Some(selesai))) } }
                }
            }

            section.detail-card aria-labelledby="judul-log" {
                h2 id="judul-log" { "Log" }
                a href=(format!("/deployments/{}/log", dep.id)) { "Lihat log lengkap" }
            }

            @if let Some(kind) = &dep.error_kind {
                section.detail-card.metric-placeholder aria-labelledby="judul-error" {
                    h2 id="judul-error" { "Kegagalan: " (kind) }
                    @if let Some(detail) = &dep.error_detail {
                        pre { (detail) }
                    }
                }
            }
        }
    }
}

fn commit_pendek(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Job SUDAH selesai (state akhir) — dipakai `routes::events::deploy_stream`
/// menutup SSE tepat setelah event terakhir diteruskan, pola sama
/// `routes::events::job_selesai` (Fase 1).
pub fn job_selesai(event: &DeploymentEvent) -> bool {
    event.status.selesai()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(status: StatusDeployment) -> DeploymentRingkas {
        DeploymentRingkas {
            id: "dep-1234567890".to_string(),
            app_id: "app-1".to_string(),
            commit_sha: "abcdef1234567890".to_string(),
            git_ref: Some("main".to_string()),
            image_digest: format!("ghcr.io/org/api@sha256:{}", "a".repeat(64)),
            status,
            container_id: None,
            env_version_id: None,
            error_kind: None,
            error_detail: None,
            started_at: None,
            finished_at: None,
            created_at: 0,
        }
    }

    #[test]
    fn detail_menautkan_ke_viewer_log_lengkap() {
        let markup = render_deployment_detail(&dep(StatusDeployment::Live), "api", "tok", None)
            .into_string();
        assert!(markup.contains(r#"href="/deployments/dep-1234567890/log""#));
        assert!(markup.contains("Lihat log lengkap"));
    }

    #[test]
    fn detail_menyambungkan_sse_ke_deployment_id() {
        let markup = render_deployment_detail(&dep(StatusDeployment::Live), "api", "tok", None)
            .into_string();
        assert!(markup.contains(r#"sse-connect="/events/deploy/dep-1234567890""#));
    }

    #[test]
    fn fragmen_menampilkan_error_kind_kalau_gagal() {
        let mut d = dep(StatusDeployment::Failed);
        d.error_kind = Some("health_no_response".to_string());
        d.error_detail = Some("detail gagal".to_string());
        let markup = render_deployment_fragmen(&d, "api").into_string();
        assert!(markup.contains("health_no_response"));
        assert!(markup.contains("detail gagal"));
    }

    #[test]
    fn fragmen_tanpa_error_tidak_menampilkan_kartu_kegagalan() {
        let markup = render_deployment_fragmen(&dep(StatusDeployment::Live), "api").into_string();
        assert!(!markup.contains("Kegagalan:"));
    }

    #[test]
    fn job_selesai_benar_untuk_status_akhir_saja() {
        assert!(!job_selesai(&DeploymentEvent {
            status: StatusDeployment::Pulling,
            pesan: None
        }));
        assert!(job_selesai(&DeploymentEvent {
            status: StatusDeployment::Live,
            pesan: None
        }));
    }
}
