use maud::{Markup, html};

use crate::web::layout::{app_shell, base_page};

pub fn render_notification_settings(
    enabled: bool,
    masked_url: Option<String>,
    events: &[String],
    pesan: Option<&str>,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        h1 { "Pengaturan Notifikasi" }
        @if let Some(pesan) = pesan { div.alert.alert-success { (pesan) } }
        @if let Some(url) = masked_url { p { "URL tersimpan: " (url) } } @else { p { "Belum ada webhook tersimpan." } }
        form method="post" action="/settings/notifications" {
            input type="hidden" name="csrf_token" value=(csrf_token);
            label { input type="checkbox" name="enabled" value="1" checked[enabled]; " Aktif" }
            label { "URL HTTPS"; input type="url" name="url" placeholder="https://contoh.invalid/webhook"; }
            label { "Secret baru (kosongkan untuk mempertahankan)"; input type="password" name="secret" autocomplete="new-password"; }
            label { "Event (pisahkan koma)"; input type="text" name="events" value=(events.join(",")); }
            button.btn type="submit" { "Simpan" }
        }
    };
    base_page(
        "Pengaturan Notifikasi - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_tidak_pernah_dirender() {
        let markup = render_notification_settings(
            true,
            Some("https://example.invalid/••••".to_string()),
            &[],
            None,
            "csrf",
            None,
        )
        .into_string();
        assert!(!markup.contains("secret-rahasia"));
        assert!(markup.contains("URL tersimpan"));
    }
}
