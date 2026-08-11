use maud::{Markup, html};

use crate::apps::model::AppRingkas;
use crate::deployments::reconciliation::{FindingRingkas, FindingStatus};
use crate::web::layout::{app_shell, base_page};

pub fn render_reconciliation(
    app: &AppRingkas,
    findings: &[FindingRingkas],
    pesan: Option<&str>,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.detail-title-row { h1 { "Rekonsiliasi: " (app.name) } }
        p { a href=(format!("/apps/{}", app.id)) { "Kembali ke app" } }
        @if let Some(pesan) = pesan {
            div.alert.alert-warning { (pesan) }
        }
        @if findings.is_empty() {
            div.alert.alert-success { "Tidak ada penyimpangan aktif." }
        } @else {
            div.alert.alert-warning {
                "Sistem menemukan kondisi yang perlu diperiksa. Tidak ada perbaikan otomatis yang dilakukan."
            }
            table.fleet-table {
                thead { tr { th { "Kategori" } th { "Severity" } th { "Status" } th { "Terakhir terlihat" } th { "Tindakan" } } }
                tbody {
                    @for finding in findings {
                        tr {
                            td { code { (finding.kind) } }
                            td { (finding.severity) }
                            td { @match finding.status { FindingStatus::Open => { "Aktif" } FindingStatus::Acknowledged => { "Diakui" } FindingStatus::Resolved => { "Pulih" } } }
                            td { (finding.last_seen_at) }
                            td {
                                @if finding.status == FindingStatus::Open {
                                    form method="post" action=(format!("/apps/{}/reconciliation/{}/acknowledge", app.id, finding.id)) {
                                        input type="hidden" name="csrf_token" value=(csrf_token);
                                        button.btn type="submit" { "Akui" }
                                    }
                                } @else { "-" }
                            }
                        }
                    }
                }
            }
        }
    };
    base_page(
        &format!("Rekonsiliasi {} - Mengploy", app.name),
        app_shell(Some(csrf_token), strip, content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_aktif_menegaskan_tidak_ada_perbaikan_otomatis() {
        let app = AppRingkas {
            id: "app".to_string(),
            server_id: "srv".to_string(),
            name: "api".to_string(),
            health_path: "/health".to_string(),
            health_grace_secs: 30,
            port: 8080,
            restart_policy: "unless-stopped".to_string(),
            repo_url: None,
            created_at: 0,
            updated_at: 0,
        };
        let finding = FindingRingkas {
            id: "finding".to_string(),
            app_id: "app".to_string(),
            server_id: "srv".to_string(),
            deployment_id: None,
            kind: "live_digest_mismatch".to_string(),
            severity: "warning".to_string(),
            status: FindingStatus::Open,
            first_seen_at: 1,
            last_seen_at: 2,
        };
        let markup = render_reconciliation(&app, &[finding], None, "csrf", None).into_string();
        assert!(markup.contains("Tidak ada perbaikan otomatis"));
        assert!(!markup.contains("Perbaiki otomatis"));
    }
}
