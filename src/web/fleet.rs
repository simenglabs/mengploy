//! Overview armada: `GET /servers` (`docs/design/fleet-overview.md` §4.1).

use maud::{Markup, html};

use crate::servers::model::{ServerRingkas, StatusServer};
use crate::web::layout::{app_shell, base_page};

pub fn render_fleet(servers: &[ServerRingkas], csrf_token: &str, strip: Option<Markup>) -> Markup {
    let _ = csrf_token; // Fase 1 tidak punya aksi mutasi di halaman ini (tidak ada hapus/ubah).

    let content = html! {
        div.fleet-header {
            h1 { "Armada Server" }
            a.btn href="/servers/baru" { "+ Tambah Server" }
        }
        @if servers.is_empty() {
            div.fleet-empty {
                p { "[!] Belum ada server terdaftar. Daftarkan server pertama Anda untuk mulai mengelola container." }
                a.btn href="/servers/baru" { "+ Tambah Server" }
            }
        } @else {
            table.fleet-table {
                thead {
                    tr {
                        th scope="col" { "Nama" }
                        th scope="col" { "Host / IP" }
                        th scope="col" { "Status" }
                        th scope="col" { "Docker" }
                        th scope="col" { "OS" }
                        th scope="col" { "Terakhir Dilihat" }
                    }
                }
                tbody {
                    @for server in servers {
                        (baris_server(server))
                    }
                }
            }
        }
    };

    base_page(
        "Overview Armada - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

fn baris_server(server: &ServerRingkas) -> Markup {
    let row_class = if server.status == StatusServer::Unreachable {
        "unreachable-row"
    } else {
        ""
    };
    let name_class = if server.status == StatusServer::Unreachable {
        "name-danger"
    } else {
        ""
    };

    html! {
        tr class=(row_class) {
            td {
                a class=(name_class) href=(format!("/servers/{}", server.id)) { (server.name) }
                @if server.status == StatusServer::Unreachable {
                    div.row-detail {
                        "(Gagal: " (server.consecutive_failures) "x)"
                    }
                    @if let Some(kind) = &server.last_error_kind {
                        div.row-detail.warning { "[Masalah: " (kind) "]" }
                    }
                }
            }
            td { (server.host) }
            td { (badge(server.status)) }
            td { (server.docker_version.clone().unwrap_or_else(|| "-".to_string())) }
            td { (server.os_info.clone().unwrap_or_else(|| "-".to_string())) }
            td { (format_epoch_opt(server.last_seen_at)) }
        }
    }
}

/// Badge status non-warna-saja: label kapital + `aria-label` untuk pembaca
/// layar (`docs/design/fleet-overview.md` §6).
pub fn badge(status: StatusServer) -> Markup {
    let (class, label) = match status {
        StatusServer::Pending => ("pending", "MENUNGGU"),
        StatusServer::Verifying => ("verifying", "VERIFIKASI"),
        StatusServer::Online => ("online", "ONLINE"),
        StatusServer::Unreachable => ("unreachable", "TIDAK TERJANGKAU"),
    };
    html! {
        span class=(format!("status-badge {class}")) aria-label=(format!("Status: {label}")) {
            (label)
        }
    }
}

pub fn format_epoch_opt(epoch: Option<i64>) -> String {
    match epoch {
        Some(e) => format_epoch(e),
        None => "-".to_string(),
    }
}

fn format_epoch(epoch: i64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(epoch) {
        Ok(dt) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02} UTC",
            dt.year(),
            u8::from(dt.month()),
            dt.day(),
            dt.hour(),
            dt.minute()
        ),
        Err(_) => "-".to_string(),
    }
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
            last_seen_at: Some(0),
            docker_version: Some("24.0.7".to_string()),
            os_info: Some("Ubuntu 22.04".to_string()),
            host_key_fingerprint: None,
            consecutive_failures: 2,
            last_error_kind: Some("host_unreachable".to_string()),
            last_error_message: None,
        }
    }

    #[test]
    fn fleet_kosong_menampilkan_state_kosong_dan_cta() {
        let markup = render_fleet(&[], "token", None).into_string();
        assert!(markup.contains("Belum ada server terdaftar"));
        assert!(markup.contains("+ Tambah Server"));
        // `!contains("fleet-table")` gagal karena CSS embedded (`<style>`)
        // selalu memuat definisi kelas `.fleet-table` — cek elemen `<table`
        // yang sebenarnya, bukan substring kelas CSS.
        assert!(!markup.contains("<table"));
    }

    #[test]
    fn fleet_dengan_server_menampilkan_tabel() {
        let servers = vec![server(StatusServer::Online)];
        let markup = render_fleet(&servers, "token", None).into_string();
        assert!(markup.contains("vps-sg-1"));
        assert!(markup.contains("24.0.7"));
        assert!(markup.contains("Ubuntu 22.04"));
        assert!(markup.contains(r#"href="/servers/srv-1""#));
    }

    #[test]
    fn baris_unreachable_menampilkan_hitung_gagal_dan_kategori() {
        let servers = vec![server(StatusServer::Unreachable)];
        let markup = render_fleet(&servers, "token", None).into_string();
        assert!(markup.contains("unreachable-row"));
        assert!(markup.contains("Gagal: 2x"));
        assert!(markup.contains("Masalah: host_unreachable"));
    }

    #[test]
    fn badge_menyertakan_aria_label_tidak_hanya_warna() {
        let markup = badge(StatusServer::Online).into_string();
        assert!(markup.contains("aria-label=\"Status: ONLINE\""));
        assert!(markup.contains("ONLINE"));
    }

    #[test]
    fn format_epoch_opt_none_jadi_strip() {
        assert_eq!(format_epoch_opt(None), "-");
    }
}
