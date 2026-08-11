//! UI Fase 7: operasi armada dan pintu darurat.

use maud::{Markup, html};

use crate::fleet::{DiskSummary, FleetOperationResultSummary, FleetOperationSummary};
use crate::servers::model::ServerRingkas;
use crate::web::fleet::format_epoch_opt;
use crate::web::layout::{app_shell, base_page};

pub type ExecResultTampil = (String, String, (String, i64, bool));

pub fn render_fleet_actions(
    servers: &[ServerRingkas],
    disks: &[DiskSummary],
    operations: &[FleetOperationSummary],
    csrf_token: &str,
    exec_result: Option<ExecResultTampil>,
    results: &[FleetOperationResultSummary],
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.fleet-header {
            h1 { "Operasi Armada" }
        }
        section.detail-card aria-labelledby="judul-disk-armada" {
            h2 id="judul-disk-armada" { "Disk seluruh armada" }
            @if disks.is_empty() {
                p { "Belum ada data disk. Metrik host akan muncul setelah server online dipindai." }
            } @else {
                table.fleet-table {
                    thead { tr {
                        th scope="col" { "Server" }
                        th scope="col" { "Status" }
                        th scope="col" { "Terpakai" }
                        th scope="col" { "Sampel" }
                    }}
                    tbody { @for disk in disks { tr {
                        td { (disk.server_name) }
                        td { (disk.status) }
                        td { (format_disk(disk.used_bytes, disk.total_bytes)) }
                        td { (format_epoch_opt(disk.sampled_at)) }
                    }}}
                }
            }
        }
        section.detail-grid {
            section.detail-card aria-labelledby="judul-command-armada" {
                h2 id="judul-command-armada" { "Jalankan perintah di banyak server" }
                p.field-hint { "Perintah dijalankan sebagai pengguna SSH server. Hasil tiap server dipisahkan dan dibatasi." }
                form method="post" action="/fleet/command" {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    textarea name="command" rows="4" required placeholder="mis. uptime"
                        aria-label="Perintah remote" {}
                    div.fleet-target-list {
                        @for server in servers { label {
                            input type="checkbox" name="server_id" value=(server.id)
                                disabled[server.status != crate::servers::model::StatusServer::Online];
                            " " (server.name) " (" (server.status.as_db_str()) ")"
                        }}
                    }
                    label { input type="checkbox" name="confirm" value="jalankan" required; " Saya paham perintah ini berjalan di server terpilih." }
                    button.btn type="submit" { "Jalankan di server terpilih" }
                }
            }
            section.detail-card aria-labelledby="judul-prune-armada" {
                h2 id="judul-prune-armada" { "Bersihkan image tidak terpakai" }
                p.field-hint { "Image live, lima deployment sukses terakhir, deployment aktif, unknown, dan container berlabel selalu dilindungi." }
                form method="post" action="/fleet/prune" {
                    input type="hidden" name="csrf_token" value=(csrf_token);
                    div.fleet-target-list {
                        @for server in servers { label {
                            input type="checkbox" name="server_id" value=(server.id)
                                disabled[server.status != crate::servers::model::StatusServer::Online];
                            " " (server.name)
                        }}
                    }
                    label { input type="checkbox" name="confirm" value="prune" required; " Saya paham image yang tidak terlindungi akan dihapus." }
                    button.btn type="submit" { "Prune server terpilih" }
                }
            }
        }
        @if let Some((server_id, container_id, (output, code, truncated))) = exec_result {
            section.detail-card aria-labelledby="judul-exec-hasil" {
                h2 id="judul-exec-hasil" { "Hasil exec container" }
                p { "Server: " (server_id) " — container: " (container_id) " — exit code: " (code) }
                @if truncated { div.alert.alert-warning { "Keluaran dipotong karena melewati batas ukuran." } }
                pre.fleet-output { (output) }
            }
        }
        section.detail-card aria-labelledby="judul-exec" {
            h2 id="judul-exec" { "Pintu darurat: exec satu kali ke container" }
            p.field-hint { "Sesi singkat, bukan terminal web penuh. Perintah dibatasi waktu dan ukuran keluaran." }
            @for server in servers {
                @if server.status == crate::servers::model::StatusServer::Online {
                    form method="post" action=(format!("/fleet/exec/{}", server.id)) class="fleet-exec-form" {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        input type="text" name="container_id" placeholder="ID container" required aria-label=(format!("ID container di {}", server.name));
                        input type="text" name="command" placeholder="mis. env" required aria-label=(format!("Perintah exec di {}", server.name));
                        label { input type="checkbox" name="confirm" value="exec" required; " Konfirmasi" }
                        button.btn type="submit" { "Exec di " (server.name) }
                    }
                }
            }
        }
        section.detail-card aria-labelledby="judul-riwayat-operasi" {
            h2 id="judul-riwayat-operasi" { "Riwayat operasi" }
            @if operations.is_empty() {
                p { "Belum ada operasi armada." }
            } @else {
                ul {
                    @for operation in operations { li {
                        a href=(format!("/fleet/operations/{}", operation.id)) { (operation.kind) " — " (operation.status) }
                        " (" (format_epoch_opt(Some(operation.created_at))) ")"
                    }}
                }
            }
        }
        @if !results.is_empty() {
            section.detail-card aria-labelledby="judul-hasil-server" {
                h2 id="judul-hasil-server" { "Hasil per server" }
                ul { @for result in results { li {
                    (result.server_id) " — " (result.status)
                    @if let Some(code) = result.exit_code { " (exit " (code) ")" }
                    @if result.output_path.is_some() { " — output tersedia di log operasi privat" }
                }}}
            }
        }
    };
    base_page(
        "Operasi Armada - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

fn format_disk(used: Option<i64>, total: Option<i64>) -> String {
    match (used, total) {
        (Some(used), Some(total)) if total > 0 => {
            format!(
                "{}% ({used} / {total} byte)",
                used.saturating_mul(100) / total
            )
        }
        _ => "Belum ada sampel".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halaman_kosong_menampilkan_state_kosong() {
        let markup = render_fleet_actions(&[], &[], &[], "csrf", None, &[], None).into_string();
        assert!(markup.contains("Belum ada data disk"));
        assert!(markup.contains("Belum ada operasi armada"));
    }

    #[test]
    fn output_exec_dirender_apa_adanya_sebagai_teks() {
        let markup = render_fleet_actions(
            &[],
            &[],
            &[],
            "csrf",
            Some((
                "srv".to_string(),
                "ctr".to_string(),
                ("<b>".to_string(), 0, false),
            )),
            &[],
            None,
        )
        .into_string();
        assert!(markup.contains("&lt;b&gt;"));
        assert!(!markup.contains("<b>"));
    }

    #[test]
    fn format_disk_menangani_data_kosong_dan_data_valid() {
        assert_eq!(format_disk(None, None), "Belum ada sampel");
        assert!(format_disk(Some(50), Some(100)).contains("50%"));
    }
}
