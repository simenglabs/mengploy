//! Daftar app, detail (Overview), form tambah app — `docs/design/apps.md`.

use maud::{Markup, html};

use crate::apps::model::{AppRingkas, DeployTokenRingkas, DomainRingkas};
use crate::deployments::DeploymentRingkas;
use crate::servers::model::ServerRingkas;
use crate::web::deployments::badge_deployment;
use crate::web::fleet::format_epoch_opt;
use crate::web::layout::{app_shell, base_page};

fn nama_server<'a>(servers: &'a [ServerRingkas], server_id: &str) -> &'a str {
    servers
        .iter()
        .find(|s| s.id == server_id)
        .map(|s| s.name.as_str())
        .unwrap_or("-")
}

pub fn render_apps(
    apps: &[AppRingkas],
    servers: &[ServerRingkas],
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.fleet-header {
            h1 { "Aplikasi" }
            a.btn href="/apps/baru" { "+ Tambah App" }
        }
        @if apps.is_empty() {
            div.fleet-empty {
                p { "[!] Belum ada app terdaftar. Daftarkan app pertama Anda supaya CI bisa deploy." }
                a.btn href="/apps/baru" { "+ Tambah App" }
            }
        } @else {
            table.fleet-table {
                thead {
                    tr {
                        th scope="col" { "Nama" }
                        th scope="col" { "Server" }
                        th scope="col" { "Port" }
                        th scope="col" { "Health Path" }
                    }
                }
                tbody {
                    @for app in apps {
                        tr {
                            td { a href=(format!("/apps/{}", app.id)) { (app.name) } }
                            td { (nama_server(servers, &app.server_id)) }
                            td { (app.port) }
                            td { (app.health_path) }
                        }
                    }
                }
            }
        }
    };

    base_page(
        "Aplikasi - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

pub fn render_app_baru(
    servers: &[ServerRingkas],
    csrf_token: &str,
    error: Option<&str>,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        h1 { "Tambah App Baru" }
        @if let Some(pesan) = error {
            div.alert.alert-danger { (pesan) }
        }
        @if servers.is_empty() {
            div.alert.alert-warning {
                "Belum ada server terdaftar. " a href="/servers/baru" { "Tambahkan server dulu" } "."
            }
        } @else {
            form.form-panel method="post" action="/apps" {
                input type="hidden" name="csrf_token" value=(csrf_token);
                div.field {
                    label for="server_id" { "Server" }
                    select id="server_id" name="server_id" required {
                        @for server in servers {
                            option value=(server.id) { (server.name) }
                        }
                    }
                }
                div.field {
                    label for="name" { "Nama App" }
                    input id="name" name="name" type="text" required autofocus;
                    p.field-hint { "Dipakai sebagai nama container dan harus cocok dengan field \"app\" di POST /api/v1/deploy." }
                }
                div.field {
                    label for="port" { "Port Container" }
                    input id="port" name="port" type="number" min="1" max="65535" required;
                }
                div.field {
                    label for="health_path" { "Health Check Path" }
                    input id="health_path" name="health_path" type="text" value="/health";
                }
                div.field {
                    label for="health_grace_secs" { "Grace Period Health Check (detik)" }
                    input id="health_grace_secs" name="health_grace_secs" type="number" min="0" value="30";
                }
                div.field-actions {
                    button.btn type="submit" { "Simpan App" }
                }
            }
        }
    };

    base_page(
        "Tambah App - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

/// Overview app: konfigurasi, domain, token deploy, riwayat deployment.
/// `token_baru` HANYA `Some` sekali, tepat setelah `POST /apps/{id}/token`
/// sukses (invariant §5 no.11 — token tidak pernah dikembalikan lagi
/// setelah ini, halaman berikutnya hanya menampilkan `DeployTokenRingkas`
/// tanpa hash/plaintext).
#[allow(clippy::too_many_arguments)]
pub fn render_app_detail(
    app: &AppRingkas,
    server_name: &str,
    domains: &[DomainRingkas],
    tokens: &[DeployTokenRingkas],
    deploys: &[DeploymentRingkas],
    token_baru: Option<&str>,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.detail-title-row {
            h1 { "App: " (app.name) }
        }

        @if let Some(token) = token_baru {
            div.alert.alert-warning {
                strong { "Token deploy baru — salin sekarang, tidak akan ditampilkan lagi:" }
                pre { code { (token) } }
            }
        }

        div.detail-grid {
            section.detail-card aria-labelledby="judul-konfig" {
                h2 id="judul-konfig" { "Konfigurasi" }
                div.detail-row { span { "Server" } span { (server_name) } }
                div.detail-row { span { "Port" } span { (app.port) } }
                div.detail-row { span { "Health Path" } span { (app.health_path) } }
                div.detail-row { span { "Grace Period" } span { (app.health_grace_secs) "s" } }
                div.detail-row { span { "Restart Policy" } span { (app.restart_policy) } }
            }

            section.detail-card aria-labelledby="judul-domain" {
                h2 id="judul-domain" { "Domain" }
                @if domains.is_empty() {
                    p { "Belum ada domain — Traefik hanya routing lewat label, tanpa domain publik tidak ada router." }
                } @else {
                    ul {
                        @for d in domains {
                            li { (d.host) }
                        }
                    }
                }
                form method="post" action=(format!("/apps/{}/domain", app.id)) {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    input name="host" type="text" placeholder="app.contoh.com" required;
                    button.btn type="submit" { "+ Tambah Domain" }
                }
            }

            section.detail-card aria-labelledby="judul-token" {
                h2 id="judul-token" { "Token Deploy" }
                @if tokens.is_empty() {
                    p { "Belum ada token — CI tidak bisa deploy app ini sampai token dibuat." }
                } @else {
                    ul {
                        @for t in tokens {
                            li {
                                (t.name) " — dibuat " (format_epoch_opt(Some(t.created_at)))
                                @if let Some(last) = t.last_used_at {
                                    " (terakhir dipakai " (format_epoch_opt(Some(last))) ")"
                                } @else {
                                    " (belum pernah dipakai)"
                                }
                            }
                        }
                    }
                }
                form method="post" action=(format!("/apps/{}/token", app.id)) {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    input name="name" type="text" placeholder="mis. github-actions" required;
                    button.btn type="submit" { "+ Buat Token" }
                }
            }
        }

        section.detail-card aria-labelledby="judul-riwayat" {
            h2 id="judul-riwayat" { "Riwayat Deployment" }
            @if deploys.is_empty() {
                p { "Belum pernah dideploy." }
            } @else {
                table.fleet-table {
                    thead {
                        tr {
                            th scope="col" { "Waktu" }
                            th scope="col" { "Commit" }
                            th scope="col" { "Status" }
                        }
                    }
                    tbody {
                        @for d in deploys {
                            tr {
                                td { a href=(format!("/deployments/{}", d.id)) { (format_epoch_opt(Some(d.created_at))) } }
                                td { code { (commit_pendek(&d.commit_sha)) } }
                                td { (badge_deployment(d.status)) }
                            }
                        }
                    }
                }
            }
        }
    };

    base_page(
        &format!("App {} - Mengploy", app.name),
        app_shell(Some(csrf_token), strip, content),
    )
}

fn commit_pendek(sha: &str) -> String {
    sha.chars().take(7).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppRingkas {
        AppRingkas {
            id: "app-1".to_string(),
            server_id: "srv-1".to_string(),
            name: "api".to_string(),
            health_path: "/health".to_string(),
            health_grace_secs: 30,
            port: 8080,
            restart_policy: "unless-stopped".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn apps_kosong_menampilkan_cta() {
        let markup = render_apps(&[], &[], "tok", None).into_string();
        assert!(markup.contains("Belum ada app terdaftar"));
    }

    #[test]
    fn apps_dengan_data_menampilkan_nama_server() {
        let servers = vec![ServerRingkas {
            id: "srv-1".to_string(),
            name: "vps-sg-1".to_string(),
            host: "1.2.3.4".to_string(),
            port: 22,
            ssh_user: "root".to_string(),
            status: crate::servers::model::StatusServer::Online,
            last_seen_at: None,
            docker_version: None,
            os_info: None,
            host_key_fingerprint: None,
            consecutive_failures: 0,
            last_error_kind: None,
            last_error_message: None,
        }];
        let markup = render_apps(&[app()], &servers, "tok", None).into_string();
        assert!(markup.contains("vps-sg-1"));
        assert!(markup.contains(r#"href="/apps/app-1""#));
    }

    #[test]
    fn detail_token_baru_ditampilkan_sekali_dengan_peringatan() {
        let markup = render_app_detail(
            &app(),
            "vps-sg-1",
            &[],
            &[],
            &[],
            Some("mengdep_deploy_abc"),
            "tok",
            None,
        )
        .into_string();
        assert!(markup.contains("mengdep_deploy_abc"));
        assert!(markup.contains("tidak akan ditampilkan lagi"));
    }

    #[test]
    fn detail_tanpa_token_baru_tidak_menampilkan_apa_pun() {
        let markup =
            render_app_detail(&app(), "vps-sg-1", &[], &[], &[], None, "tok", None).into_string();
        assert!(!markup.contains("tidak akan ditampilkan lagi"));
    }

    #[test]
    fn detail_riwayat_kosong_menampilkan_pesan() {
        let markup =
            render_app_detail(&app(), "vps-sg-1", &[], &[], &[], None, "tok", None).into_string();
        assert!(markup.contains("Belum pernah dideploy"));
    }

    #[test]
    fn commit_pendek_memotong_ke_7_karakter() {
        assert_eq!(commit_pendek("abcdef1234567890"), "abcdef1");
    }
}
