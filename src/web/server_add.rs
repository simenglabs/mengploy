//! Wizard tambah server, 3 langkah (`docs/design/tambah-server.md`):
//! 1. `render_server_baru` — form kredensial SSH.
//! 2. `render_verifikasi` / `render_verifikasi_fragmen` — checklist SSE.
//! 3. `render_registry_form` — registry opsional.

use maud::{Markup, html};

use crate::events::VerificationEvent;
use crate::registries::repo::RegistryRingkas;
use crate::servers::model::{LangkahStatus, LangkahVerifikasi, ServerRingkas};
use crate::web::layout::{app_shell, base_page};

// ============================================================
// Langkah 1 — form kredensial SSH
// ============================================================

pub fn render_server_baru(csrf_token: &str, error: Option<&str>, strip: Option<Markup>) -> Markup {
    let content = html! {
        h1 { "Tambah Server Baru (Langkah 1/3: Kredensial SSH)" }
        @if let Some(pesan) = error {
            div.alert.alert-danger { (pesan) }
        }
        (panel_panduan_ssh())
        form.form-panel method="post" action="/servers" {
            input type="hidden" name="csrf_token" value=(csrf_token);
            div.field {
                label for="name" { "Nama Server" }
                input id="name" name="name" type="text" required autofocus;
            }
            div.field {
                label for="host" { "Alamat Host / IP" }
                input id="host" name="host" type="text" required;
                p.field-hint { "Contoh: vps-sg-1.domain.com atau 128.199.12.34 (Tanpa skema URL atau port)" }
            }
            div.field {
                label for="port" { "Port SSH" }
                input id="port" name="port" type="number" min="1" max="65535" value="22";
            }
            div.field {
                label for="ssh_user" { "Pengguna SSH" }
                input id="ssh_user" name="ssh_user" type="text" required;
            }
            div.field {
                label for="ssh_keygen_cmd" { "Belum punya kunci?" }
                div.copy-field {
                    input
                        id="ssh_keygen_cmd"
                        type="text"
                        readonly
                        title="Klik untuk salin perintah"
                        value="ssh-keygen -t ed25519 -f mengploy_key -N ''"
                        onclick="this.select(); this.nextElementSibling.classList.add('show'); clearTimeout(this._t); this._t = setTimeout(() => this.nextElementSibling.classList.remove('show'), 1200); navigator.clipboard && navigator.clipboard.writeText(this.value).catch(() => {})";
                    span.copy-tooltip { "Disalin!" }
                }
                p.field-hint {
                    "Jalankan di terminal lokal, lalu tempel ISI file " code { "mengploy_key" }
                    " (bukan " code { "mengploy_key.pub" } ") ke kolom di bawah."
                }
            }
            div.field {
                label for="ssh_key" { "Kunci Privat SSH" }
                textarea id="ssh_key" name="ssh_key" rows="10" required placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" {}
                p.field-hint {
                    "Harus berupa kunci format OpenSSH (dimulai dengan -----BEGIN OPENSSH PRIVATE KEY-----)."
                }
            }
            div.field-actions {
                button.btn type="submit" { "Lanjutkan ke Verifikasi" }
            }
        }
    };

    base_page(
        "Tambah Server Baru - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

/// Panduan langkah demi langkah menyiapkan akses SSH sebelum mengisi form —
/// penjelasan mengapa public key harus ada di `authorized_keys` server
/// (kesalahan paling umum saat verifikasi: kunci privat valid ditolak karena
/// public key pasangannya belum terdaftar). Perintah yang dipakai `code`
/// memakai komentar `# komentar` yang dibungkus `<code>` saja, bukan blok
/// `<pre>` besar, supaya tetap ringkas di layar.
fn panel_panduan_ssh() -> Markup {
    html! {
        section.guide-panel aria-labelledby="judul-panduan" {
            h2 id="judul-panduan" { "Cara Menyiapkan Akses SSH" }
            ol.guide-list {
                li {
                    strong { "1. Buat pasangan kunci (sekali saja)." }
                    p {
                        "Di terminal lokal Anda, jalankan: "
                        code { "ssh-keygen -t ed25519 -f mengploy_key -N ''" }
                    }
                    p.field-hint {
                        "Hasilnya dua file: " code { "mengploy_key" } " (PRIVAT, rahasia) dan "
                        code { "mengploy_key.pub" } " (PUBLIK, boleh disebar)."
                    }
                }
                li {
                    strong { "2. Daftarkan kunci PUBLIK di server target." }
                    p {
                        "Login sekali ke server Anda lalu tempel isi " code { "mengploy_key.pub" }
                        " ke file " code { "~/.ssh/authorized_keys" } " (untuk user SSH yang sama dengan "
                        "kolom Pengguna SSH di bawah). Cara cepat:"
                    }
                    p {
                        code { "ssh-copy-id -i ~/.ssh/mengploy_key.pub user@alamat-server" }
                    }
                    div.alert.alert-warning {
                        strong { "Mengapa wajib? " }
                        "Server hanya mengizinkan login dengan kunci yang PUBLIK-nya terdaftar di "
                        code { "~/.ssh/authorized_keys" } ". Jangan pernah menaruh kunci privat "
                        "di server — file itu hanya menerima kunci publik."
                    }
                }
                li {
                    strong { "3. Tempel kunci PRIVAT ke form di bawah." }
                    p {
                        "Salin seluruh isi " code { "mengploy_key" } " (dimulai "
                        code { "-----BEGIN OPENSSH PRIVATE KEY-----" } ") ke kolom "
                        "\"Kunci Privat SSH\". Isi nama, alamat, dan user SSH server target, "
                        "lalu lanjutkan."
                    }
                }
            }
        }
    }
}

// ============================================================
// Langkah 2 — checklist verifikasi (SSE)
// ============================================================

pub fn render_verifikasi(
    server: &ServerRingkas,
    event: &VerificationEvent,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        h1 { "Verifikasi Server (Langkah 2/3: Pemeriksaan Sistem)" }
        div hx-ext="sse" sse-connect=(format!("/events/verifikasi/{}", server.id)) sse-swap="message" hx-target="#checklist-container" hx-swap="innerHTML" {
            (checklist_body(&server.id, event, csrf_token))
        }
    };

    base_page(
        "Verifikasi Server - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

/// Payload SSE — persis isi `#checklist-container`, supaya swap HTMX tidak
/// pernah menghasilkan markup yang beda bentuk dari render awal.
pub fn render_verifikasi_fragmen(
    server_id: &str,
    event: &VerificationEvent,
    csrf_token: &str,
) -> Markup {
    checklist_body(server_id, event, csrf_token)
}

fn checklist_body(server_id: &str, event: &VerificationEvent, csrf_token: &str) -> Markup {
    html! {
        div id="checklist-container" {
            ul.verify-checklist {
                @for langkah in &event.langkah {
                    (baris_langkah(langkah))
                }
            }
            @if let Some(fingerprint) = &event.tofu_pending_fingerprint {
                div.tofu-box {
                    p {
                        "Sidik jari host key belum terdaftar di aplikasi. Konfirmasi sidik jari "
                        "berikut untuk melanjutkan:"
                    }
                    p { code.host-key { (fingerprint) } }
                    form method="post" action=(format!("/servers/{server_id}/hostkey/konfirmasi")) {
                        input type="hidden" name="csrf_token" value=(csrf_token);
                        input type="hidden" name="fingerprint" value=(fingerprint);
                        button.btn type="submit" { "Ya, Terima & Simpan" }
                    }
                }
            }
            @if semua_selesai(&event.langkah) {
                p { a.btn href=(format!("/servers/{server_id}/registry")) { "Lanjutkan ke Langkah 3 (Registry)" } }
            }
        }
    }
}

fn semua_selesai(langkah: &[LangkahVerifikasi]) -> bool {
    !langkah.is_empty() && langkah.iter().all(|l| l.status == LangkahStatus::Sukses)
}

fn baris_langkah(langkah: &LangkahVerifikasi) -> Markup {
    let (simbol, kelas) = match langkah.status {
        LangkahStatus::Menunggu => ("[ ]", "todo"),
        LangkahStatus::Berjalan => ("[*]", "running"),
        LangkahStatus::Sukses => ("[o]", "success"),
        LangkahStatus::Gagal => ("[x]", "danger"),
    };

    html! {
        li class=(format!("verify-step {kelas}")) {
            span.verify-step-symbol aria-hidden="true" { (simbol) }
            span.verify-step-name { (langkah.nama) }
            @if let Some(pesan) = &langkah.pesan {
                p.verify-step-message { (pesan) }
            }
        }
    }
}

// ============================================================
// Langkah 3 — registry opsional
// ============================================================

pub fn render_registry_form(
    server: &ServerRingkas,
    registries: &[RegistryRingkas],
    csrf_token: &str,
    error: Option<&str>,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        h1 { "Tautkan Registry (Langkah 3/3 - Opsional)" }
        @if let Some(pesan) = error {
            div.alert.alert-danger { (pesan) }
        }
        form.form-panel method="post" action=(format!("/servers/{}/registry", server.id)) {
            input type="hidden" name="csrf_token" value=(csrf_token);

            @if !registries.is_empty() {
                fieldset {
                    legend { "Pilih Registry Tersimpan" }
                    @for r in registries {
                        div.field-radio {
                            input type="radio" name="registry_id" id=(format!("registry-{}", r.id)) value=(r.id);
                            label for=(format!("registry-{}", r.id)) { (r.host) " (user: " (r.username) ")" }
                        }
                    }
                    div.field-radio {
                        input type="radio" name="registry_id" id="registry-baru" value="" checked;
                        label for="registry-baru" { "Baru..." }
                    }
                }
            }

            div.field {
                label for="host" { "Host Registry" }
                input id="host" name="host" type="text" placeholder="ghcr.io";
            }
            div.field {
                label for="username" { "Username" }
                input id="username" name="username" type="text";
            }
            div.field {
                label for="token" { "Token Akses / Kata Sandi" }
                input id="token" name="token" type="password";
            }

            div.field-actions {
                button.btn type="submit" { "Tautkan Registry" }
                a.btn.btn-secondary href=(format!("/servers/{}", server.id)) { "Lewati & Selesai" }
            }
        }
    };

    base_page(
        "Tautkan Registry - Mengploy",
        app_shell(Some(csrf_token), strip, content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> ServerRingkas {
        ServerRingkas {
            id: "srv-1".to_string(),
            name: "vps-sg-1".to_string(),
            host: "1.2.3.4".to_string(),
            port: 22,
            ssh_user: "root".to_string(),
            status: crate::servers::model::StatusServer::Verifying,
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
    fn server_baru_tidak_pernah_prefill_kunci() {
        let markup =
            render_server_baru("token", Some("Host tidak boleh mengandung skema URL"), None)
                .into_string();
        assert!(markup.contains("Host tidak boleh mengandung skema URL"));
        // "BEGIN OPENSSH" muncul sah di `placeholder`/hint — yang tidak
        // boleh adalah textarea BERISI kunci (mis. dari submit sebelumnya).
        // Maud selalu merender elemen kosong sebagai `<textarea ...></textarea>`
        // tanpa spasi/newline di antaranya; cek itu persis.
        assert!(markup.contains("></textarea>"));
    }

    #[test]
    fn server_baru_menampilkan_input_perintah_keygen_readonly_tanpa_name() {
        let markup = render_server_baru("token", None, None).into_string();
        assert!(markup.contains("ssh-keygen"));
        assert!(markup.contains("readonly"));
        // Tidak ada `name=` — memastikan field bantuan ini TIDAK PERNAH
        // ikut ter-submit sebagai bagian `ServerBaruForm`.
        assert!(!markup.contains(r#"id="ssh_keygen_cmd" name"#));
    }

    #[test]
    fn server_baru_menampilkan_panduan_langkah_ssh_dan_authorized_keys() {
        let markup = render_server_baru("token", None, None).into_string();
        assert!(markup.contains("Cara Menyiapkan Akses SSH"));
        assert!(markup.contains("authorized_keys"));
        assert!(markup.contains("ssh-copy-id"));
        assert!(markup.contains("mengploy_key.pub"));
        assert!(markup.contains("Jangan pernah menaruh kunci privat"));
    }

    #[test]
    fn verifikasi_menyambung_sse_ke_endpoint_yang_benar() {
        let event = VerificationEvent {
            langkah: vec![LangkahVerifikasi {
                nama: "Membangun Koneksi SSH".to_string(),
                status: LangkahStatus::Berjalan,
                pesan: None,
            }],
            tofu_pending_fingerprint: None,
        };
        let markup = render_verifikasi(&server(), &event, "token", None).into_string();
        assert!(markup.contains(r#"sse-connect="/events/verifikasi/srv-1""#));
        assert!(markup.contains("checklist-container"));
    }

    #[test]
    fn verifikasi_menampilkan_kotak_tofu_saat_pending() {
        let event = VerificationEvent {
            langkah: vec![LangkahVerifikasi {
                nama: "Membangun Koneksi SSH".to_string(),
                status: LangkahStatus::Berjalan,
                pesan: None,
            }],
            tofu_pending_fingerprint: Some("SHA256:abc123".to_string()),
        };
        let markup = render_verifikasi(&server(), &event, "token", None).into_string();
        assert!(markup.contains("SHA256:abc123"));
        // Maud meng-escape `&` jadi `&amp;` di teks — cocokkan bentuk yang
        // benar-benar dirender, bukan literal mentah.
        assert!(markup.contains("Ya, Terima &amp; Simpan"));
        assert!(markup.contains(r#"action="/servers/srv-1/hostkey/konfirmasi""#));
    }

    #[test]
    fn verifikasi_menampilkan_tombol_lanjut_saat_semua_sukses() {
        let event = VerificationEvent {
            langkah: vec![
                LangkahVerifikasi {
                    nama: "a".to_string(),
                    status: LangkahStatus::Sukses,
                    pesan: None,
                },
                LangkahVerifikasi {
                    nama: "b".to_string(),
                    status: LangkahStatus::Sukses,
                    pesan: None,
                },
            ],
            tofu_pending_fingerprint: None,
        };
        let markup = render_verifikasi(&server(), &event, "token", None).into_string();
        assert!(markup.contains("Lanjutkan ke Langkah 3 (Registry)"));
    }

    #[test]
    fn verifikasi_tidak_menampilkan_tombol_lanjut_kalau_belum_semua_sukses() {
        let event = VerificationEvent {
            langkah: vec![LangkahVerifikasi {
                nama: "a".to_string(),
                status: LangkahStatus::Berjalan,
                pesan: None,
            }],
            tofu_pending_fingerprint: None,
        };
        let markup = render_verifikasi(&server(), &event, "token", None).into_string();
        assert!(!markup.contains("Lanjutkan ke Langkah 3"));
    }

    #[test]
    fn verifikasi_gagal_menampilkan_pesan_perbaikan() {
        let event = VerificationEvent {
            langkah: vec![LangkahVerifikasi {
                nama: "Membangun Koneksi SSH".to_string(),
                status: LangkahStatus::Gagal,
                pesan: Some(
                    "Gagal terhubung ke host target dalam batas waktu 10 detik.".to_string(),
                ),
            }],
            tofu_pending_fingerprint: None,
        };
        let markup = render_verifikasi(&server(), &event, "token", None).into_string();
        assert!(markup.contains("Gagal terhubung ke host target"));
        assert!(markup.contains("verify-step danger") || markup.contains("danger"));
    }

    #[test]
    fn fragmen_sse_sama_persis_dengan_isi_checklist_container() {
        let event = VerificationEvent {
            langkah: vec![LangkahVerifikasi {
                nama: "a".to_string(),
                status: LangkahStatus::Sukses,
                pesan: None,
            }],
            tofu_pending_fingerprint: None,
        };
        let fragmen = render_verifikasi_fragmen("srv-1", &event, "token").into_string();
        assert!(fragmen.contains("checklist-container"));
        assert!(fragmen.contains("verify-step"));
    }

    #[test]
    fn registry_form_tombol_lewati_navigasi_biasa_bukan_form() {
        let markup = render_registry_form(&server(), &[], "token", None, None).into_string();
        assert!(markup.contains(r#"href="/servers/srv-1""#));
        assert!(markup.contains("Lewati &amp; Selesai"));
    }

    #[test]
    fn registry_form_tidak_pernah_prefill_token() {
        let registries = vec![RegistryRingkas {
            id: "reg-1".to_string(),
            host: "ghcr.io".to_string(),
            username: "deployer".to_string(),
        }];
        let markup =
            render_registry_form(&server(), &registries, "token", None, None).into_string();
        assert!(markup.contains("ghcr.io"));
        assert!(markup.contains("deployer"));
        assert!(markup.contains(r#"type="password""#));
        // Field token tidak pernah berisi value= apa pun
        assert!(!markup.contains(r#"name="token" type="password" value"#));
    }

    #[test]
    fn registry_form_menampilkan_pesan_error() {
        let markup = render_registry_form(
            &server(),
            &[],
            "token",
            Some("Kredensial ditolak oleh host registry."),
            None,
        )
        .into_string();
        assert!(markup.contains("Kredensial ditolak oleh host registry."));
    }
}
