//! Render halaman login (`docs/design/login.md`).

use maud::{Markup, html};

use crate::web::layout::base_page;

/// Pesan petunjuk default saat form belum pernah dikirim (state "Empty" /
/// "Default" — `docs/design/login.md` §4.1, §4.3).
const PETUNJUK_DEFAULT: &str = "Masukkan kata sandi awal konsol Anda.";

/// Render halaman login. `error` diisi backend dengan pesan generik Bahasa
/// Indonesia (mis. "Kata sandi salah. Silakan coba lagi." atau pesan CSRF
/// tidak valid) kalau ada kegagalan; `None` untuk kunjungan pertama.
/// `csrf_token` wajib ditanam sebagai hidden input pada form.
pub fn render_login(error: Option<&str>, csrf_token: &str) -> Markup {
    let has_error = error.is_some();
    let body = html! {
        div.login-page {
            div.login-container {
                p.login-logo { "MENGPLOY" }
                div.login-card {
                    h1 { "Masuk ke Konsol" }
                    @if let Some(msg) = error {
                        div.alert.alert-danger role="alert" { "[x] " (msg) }
                    } @else {
                        p.field-hint { (PETUNJUK_DEFAULT) }
                    }
                    form method="post" action="/login" {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        div.field {
                            label for="password" { "Kata Sandi" }
                            input
                                type="password"
                                id="password"
                                name="password"
                                class=[has_error.then_some("field-error")]
                                placeholder="Masukkan kata sandi"
                                required
                                autofocus
                                autocomplete="current-password";
                        }
                        button.btn type="submit" { "Masuk" }
                    }
                }
            }
        }
    };

    base_page("Masuk - Mengploy", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_login_tanpa_error_tampilkan_petunjuk_default() {
        // Nama kelas CSS ".alert-danger" tetap muncul di <style>, jadi cek
        // elemen alert yang benar-benar dirender, bukan nama kelas.
        let markup = render_login(None, "csrf-xyz").into_string();
        assert!(markup.contains(PETUNJUK_DEFAULT));
        assert!(markup.contains(r#"value="csrf-xyz""#));
        assert!(!markup.contains(r#"role="alert""#));
    }

    #[test]
    fn render_login_dengan_error_tampilkan_pesan_generik() {
        let markup =
            render_login(Some("Kata sandi salah. Silakan coba lagi."), "csrf-xyz").into_string();
        assert!(markup.contains("[x] Kata sandi salah. Silakan coba lagi."));
        assert!(markup.contains("alert-danger"));
        assert!(markup.contains("field-error"));
    }

    #[test]
    fn render_login_tidak_pernah_bocorkan_csrf_ke_teks_tampak() {
        // Token hanya boleh muncul sebagai atribut value hidden input, bukan
        // teks yang terlihat pengguna.
        let markup = render_login(None, "rahasia-token").into_string();
        assert!(markup.contains(r#"type="hidden" name="csrf_token" value="rahasia-token""#));
    }
}
