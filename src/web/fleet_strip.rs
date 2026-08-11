//! Fleet strip: status ringkas seluruh armada, menempel di header semua
//! halaman terlindungi (`docs/design/fleet-overview.md` §4.2).

use maud::{Markup, html};

use crate::servers::model::{ServerRingkas, StatusServer};

pub fn render_fleet_strip(servers: &[ServerRingkas]) -> Markup {
    html! {
        @if servers.is_empty() {
            span.fleet-strip-empty {
                "Tanpa server terdaftar "
                a href="/servers/baru" { "[Tambah Server]" }
            }
        } @else {
            ul.fleet-strip {
                @for server in servers {
                    li {
                        a href=(format!("/servers/{}", server.id)) {
                            span class=(dot_class(server.status)) aria-hidden="true" {}
                            span.fleet-strip-name { (server.name) }
                        }
                    }
                }
            }
        }
    }
}

fn dot_class(status: StatusServer) -> &'static str {
    match status {
        StatusServer::Online => "status-dot online",
        StatusServer::Unreachable => "status-dot unreachable",
        StatusServer::Pending => "status-dot pending",
        StatusServer::Verifying => "status-dot verifying",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, status: StatusServer) -> ServerRingkas {
        ServerRingkas {
            id: "id-1".to_string(),
            name: name.to_string(),
            host: "1.2.3.4".to_string(),
            port: 22,
            ssh_user: "root".to_string(),
            status,
            last_seen_at: None,
            docker_version: None,
            os_info: None,
            host_key_fingerprint: None,
            consecutive_failures: 0,
            last_error_kind: None,
            last_error_message: None,
        }
    }

    #[test]
    fn strip_kosong_menampilkan_teks_dan_tautan_tambah() {
        let markup = render_fleet_strip(&[]).into_string();
        assert!(markup.contains("Tanpa server terdaftar"));
        assert!(markup.contains(r#"href="/servers/baru""#));
    }

    #[test]
    fn strip_menampilkan_nama_dan_tautan_per_server() {
        let servers = vec![server("vps-sg-1", StatusServer::Online)];
        let markup = render_fleet_strip(&servers).into_string();
        assert!(markup.contains("vps-sg-1"));
        assert!(markup.contains(r#"href="/servers/id-1""#));
        assert!(markup.contains("status-dot online"));
    }

    #[test]
    fn dot_class_berbeda_untuk_tiap_status() {
        assert_eq!(dot_class(StatusServer::Online), "status-dot online");
        assert_eq!(
            dot_class(StatusServer::Unreachable),
            "status-dot unreachable"
        );
        assert_eq!(dot_class(StatusServer::Pending), "status-dot pending");
        assert_eq!(dot_class(StatusServer::Verifying), "status-dot verifying");
    }
}
