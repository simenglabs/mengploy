//! Shell aplikasi: kerangka HTML dasar, dan tata letak sidebar + header +
//! area konten yang dipakai bersama oleh dashboard, fleet, wizard, detail
//! server, dan halaman error (`docs/design/shell-aplikasi.md`).

use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::web::CSS;

/// Kerangka HTML dasar: `<head>` + `<style>` inline + isi `<body>` + HTMX
/// (core + ekstensi SSE) di-vendor lokal (Q4 `docs/plan.md`) lewat
/// `GET /assets/htmx.min.js` dan `GET /assets/htmx-sse.min.js`. Dimuat di
/// semua halaman — ukurannya kecil (~59 KB gabungan) dan progressive
/// enhancement tidak mengganggu halaman yang tidak memakainya.
pub fn base_page(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="id" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (PreEscaped(CSS)) }
                script src="/assets/htmx.min.js" {}
                script src="/assets/htmx-sse.min.js" {}
            }
            body {
                (body)
            }
        }
    }
}

/// Tata letak shell pasca-login: sidebar navigasi + header status + area
/// konten utama. Dipakai semua halaman terlindungi supaya tampilan tetap
/// konsisten (`docs/design/shell-aplikasi.md` §4.4).
///
/// `csrf_token` opsional: tombol "Keluar" hanya dirender kalau tersedia.
/// `strip` opsional (`docs/design/fleet-overview.md` §4.2 poin 4) —
/// `None` dipakai halaman error 404/500 (sesi gagal dimuat / bukan
/// konteksnya) supaya kegagalan mengambil ringkasan armada tidak pernah
/// menjatuhkan seluruh halaman.
pub fn app_shell(csrf_token: Option<&str>, strip: Option<Markup>, content: Markup) -> Markup {
    html! {
        div.app-layout {
            aside.sidebar {
                div.brand { "MENGPLOY " span.phase-tag { "[Fase 7]" } }
                nav {
                    ul {
                        li { a href="/" { "Dashboard" } }
                        li { a href="/servers" { "Server" } }
                        li { a href="/apps" { "Apps" } }
                        li { a href="/fleet" { "Operasi Armada" } }
                    }
                }
            }
            div.main-column {
                header.app-header {
                    div.fleet-strip-slot {
                        @if let Some(strip) = strip {
                            (strip)
                        }
                    }
                    div.header-actions {
                        span.status-active { "Status: Aktif" }
                        @if let Some(token) = csrf_token {
                            form.form-logout method="post" action="/logout" {
                                input type="hidden" name="csrf_token" value=(token);
                                button.btn type="submit" { "Keluar" }
                            }
                        }
                    }
                }
                main.app-content {
                    (content)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_page_menyertakan_lang_id_dan_style_dan_htmx() {
        let markup = base_page("Judul Uji", html! { p { "isi" } }).into_string();
        assert!(markup.contains(r#"lang="id""#));
        assert!(markup.contains("<style>"));
        assert!(markup.contains("Judul Uji"));
        assert!(markup.contains("/assets/htmx.min.js"));
        assert!(markup.contains("/assets/htmx-sse.min.js"));
    }

    #[test]
    fn app_shell_sembunyikan_tombol_keluar_tanpa_csrf_token() {
        let markup = app_shell(None, None, html! { p { "konten" } }).into_string();
        assert!(!markup.contains("form-logout"));
        assert!(markup.contains("Status: Aktif"));
    }

    #[test]
    fn app_shell_tampilkan_tombol_keluar_dengan_csrf_token() {
        let markup = app_shell(Some("token-abc"), None, html! { p { "konten" } }).into_string();
        assert!(markup.contains("form-logout"));
        assert!(markup.contains("token-abc"));
        assert!(markup.contains("Keluar"));
    }

    #[test]
    fn app_shell_menyisipkan_strip_kalau_ada() {
        let strip = html! { span { "STRIP-PENANDA" } };
        let markup = app_shell(None, Some(strip), html! { p { "konten" } }).into_string();
        assert!(markup.contains("STRIP-PENANDA"));
    }

    #[test]
    fn app_shell_tanpa_strip_tidak_meninggalkan_bekas() {
        let markup = app_shell(None, None, html! { p { "konten" } }).into_string();
        assert!(markup.contains("fleet-strip-slot"));
    }
}
