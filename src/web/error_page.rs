//! Render halaman 404 & 500 (`docs/design/shell-aplikasi.md` §4.4).
//!
//! Dirender lewat layout shell yang sama dengan dashboard supaya konsisten.
//! Tidak pernah membocorkan pesan library mentah, path filesystem, isi
//! query, atau backtrace — hanya teks generik Bahasa Indonesia dari desain.

use maud::{Markup, html};

use crate::web::layout::{app_shell, base_page};

/// Render halaman 404 (halaman tidak ditemukan).
///
/// `strip` opsional — pemanggil yang tahu sesi valid dan armada bisa
/// dimuat boleh menyisipkannya; kalau tidak (mis. sesi sudah gagal
/// sebelum sempat query armada), `None` aman (`docs/design/fleet-overview.md`
/// §4.2 poin 4).
///
/// Catatan: `docs/design/shell-aplikasi.md` memuat dua kalimat isi yang
/// berbeda untuk 404/500 — satu di §4.4 ("Pesan"), satu lagi di tabel
/// copywriting §7 ("Isi Kesalahan"). Keduanya tidak identik. Dipilih teks
/// §4.4 di sini karena cocok dengan asersi test backend yang sudah ada di
/// `src/error.rs` (tidak boleh diubah — bukan milik frontend).
pub fn render_404(strip: Option<Markup>) -> Markup {
    let content = html! {
        div.error-box.warning {
            h2 { "[!] Halaman Tidak Ditemukan" }
            p {
                "Halaman tidak ditemukan. Alamat yang Anda tuju tidak dikenal "
                "atau telah dipindahkan."
            }
            p { a href="/" { "Kembali ke Dashboard" } }
        }
    };

    base_page(
        "Tidak ditemukan — mengploy",
        app_shell(None, strip, content),
    )
}

/// Render halaman 500 (kesalahan internal server). Tidak pernah menerima
/// atau menampilkan detail error asli — hanya teks generik. Selalu tanpa
/// strip — kegagalan internal berarti tidak ada jaminan query ringkasan
/// armada aman dijalankan.
pub fn render_500() -> Markup {
    let content = html! {
        div.error-box.danger {
            h2 { "[x] Kesalahan Internal Server" }
            p {
                "Terjadi kesalahan internal pada server. Silakan hubungi "
                "administrator atau periksa log aplikasi."
            }
            p { a href="/" { "Kembali ke Dashboard" } }
        }
    };

    base_page(
        "Kesalahan server — mengploy",
        app_shell(None, None, content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_404_memuat_pesan_desain() {
        let markup = render_404(None).into_string();
        assert!(markup.contains("Halaman Tidak Ditemukan"));
        assert!(markup.contains("Kembali ke Dashboard"));
    }

    #[test]
    fn render_500_tidak_bocorkan_detail_dan_tampilkan_pesan_generik() {
        let markup = render_500().into_string();
        assert!(markup.contains("Kesalahan Internal Server"));
        assert!(markup.contains("Terjadi kesalahan internal pada server"));
        // Jaga-jaga: tidak boleh ada indikasi path/backtrace tercampur teks statis.
        assert!(!markup.contains(".rs:"));
    }
}
