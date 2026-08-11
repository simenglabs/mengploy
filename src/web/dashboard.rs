//! Render shell dashboard pasca-login (`docs/design/shell-aplikasi.md`,
//! `docs/api-contract.md` "GET / (perubahan, bukan endpoint baru)").

use maud::{Markup, html};

use crate::web::layout::{app_shell, base_page};

/// `jumlah_server` menentukan isi placeholder: Fase 0 selalu bilang "belum
/// ada server", Fase 1 menyesuaikan dengan keadaan armada nyata
/// (`docs/api-contract.md:459`: placeholder lama diganti isi yang sesuai).
pub fn render_dashboard(strip: Option<Markup>, csrf_token: &str, jumlah_server: usize) -> Markup {
    let content = html! {
        div.card-placeholder {
            h2 { "SISTEM INITIALISASI: SIAP" }
            @if jumlah_server == 0 {
                p {
                    "Sistem berada pada Fase 1 (Registry server dan konektivitas). "
                    "Belum ada server terdaftar."
                }
                p { a.btn href="/servers/baru" { "+ Tambah Server Pertama" } }
            } @else {
                p {
                    (jumlah_server) " server terdaftar. Lihat "
                    a href="/servers" { "Armada Server" }
                    " untuk status lengkap."
                }
            }
        }
    };

    base_page(
        "Dashboard — mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_dashboard_tanpa_server_memuat_ajakan_tambah_server() {
        let markup = render_dashboard(None, "test-token-placeholder", 0).into_string();
        assert!(markup.contains("Belum ada server terdaftar"));
        assert!(markup.contains(r#"href="/servers/baru""#));
        assert!(markup.contains("Status: Aktif"));
    }

    #[test]
    fn render_dashboard_dengan_server_menampilkan_jumlah() {
        let markup = render_dashboard(None, "token", 3).into_string();
        assert!(markup.contains("3 server terdaftar"));
        assert!(!markup.contains("Belum ada server terdaftar"));
    }

    #[test]
    fn render_dashboard_memuat_form_logout_dengan_csrf_token() {
        let token = "token_uji_1234567890abcdef";
        let markup = render_dashboard(None, token, 0).into_string();
        assert!(
            markup.contains(r#"action="/logout""#),
            "form logout harus punya action /logout"
        );
        assert!(
            markup.contains(r#"method="post""#),
            "form logout harus pakai method post"
        );
        assert!(
            markup.contains(&format!(r#"name="csrf_token" value="{token}""#)),
            "hidden input csrf_token harus berisi token yang dioper, bukan id sesi"
        );
        assert!(markup.contains(">Keluar<"), "tombol Keluar harus dirender");
        assert!(
            !markup.contains("mengdep_session"),
            "id sesi tidak boleh muncul di HTML"
        );
    }
}
