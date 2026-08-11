//! Viewer log deploy dan runtime, plus tab Deployments/Logs pada detail app —
//! `docs/design/log-viewer.md` dan `docs/design/riwayat-deployment.md`.
//!
//! **Nol `PreEscaped` di file ini, dan tidak boleh pernah ada.** Isi log adalah
//! keluaran aplikasi pengguna: data tidak tepercaya yang wajib di-escape saat
//! masuk HTML (`docs/api-contract.md` Fase 3, aturan tambahan). Maud
//! meng-escape otomatis; membatalkannya dengan `PreEscaped` adalah temuan
//! blocking.
//!
//! **Keputusan menyimpang dari `docs/design/log-viewer.md` §9 (xterm.js).**
//! Spek uiux meminta `xterm.js` dengan `xterm.write()`, tapi
//! `docs/api-contract.md` — yang sudah beku dan implementasi backendnya sudah
//! lolos gerbang — menetapkan setiap event SSE membawa **fragmen HTML** yang
//! di-append HTMX. Keduanya tidak bisa disatukan: `xterm.write()` menerima teks
//! mentah, dan `xterm` menguasai DOM-nya sendiri sehingga `sse-swap` tidak bisa
//! menyuntik ke dalamnya. Manusia memutuskan kontrak yang menang, yaitu opsi
//! (c) `docs/plan.md` Q1: `<pre>` monospace polos, escape ANSI ditanggalkan di
//! sisi render supaya `\x1b[32m` tidak tampil sebagai sampah `[32m`. Warna log
//! hilang; nol JS di luar HTMX + satu blok kecil untuk auto-follow.
use maud::{Markup, html};

use crate::apps::model::AppRingkas;
use crate::deployments::DeploymentRingkas;
use crate::logs::reader::LogLine;
use crate::web::deployments::badge_deployment;
use crate::web::fleet::format_epoch_opt;
use crate::web::layout::{app_shell, base_page};

/// Teks final dari `docs/design/log-viewer.md` §8. Dikumpulkan sebagai konstanta
/// supaya test bisa mengasersi teks yang benar-benar sama dengan spek, bukan
/// parafrase yang mirip.
const PERINGATAN_PRIVASI: &str = "Peringatan: Seluruh isi log berikut berasal dari keluaran aplikasi pengguna dan dapat memuat informasi sensitif seperti kunci enkripsi, token, atau kata sandi yang dicetak secara sengaja atau tidak sengaja oleh aplikasi Anda.";
const PESAN_MENUNGGU: &str = "[i] Menunggu keluaran log pertama dari server...";
const PESAN_TERPOTONG: &str = "--- [!] LOG TERPOTONG: Ukuran log telah melampaui batas maksimum 8 MiB. Proses deploy tetap berjalan normal di server target. Aliran data real-time masih disiarkan secara langsung di tab ini, namun tidak lagi disimpan ke disk. ---";
const PESAN_PENCARIAN_DIPOTONG: &str = "--- [i] HASIL DIPOTONG: Ditemukan lebih dari 500 baris yang cocok. Silakan persempit kata kunci pencarian Anda untuk hasil yang lebih spesifik. ---";
const PESAN_JARINGAN_TERPUTUS: &str = "[!] Jaringan terputus. Mencoba menghubungkan kembali...";
pub(crate) const PESAN_BELUM_ADA_CONTAINER: &str = "[i] Belum ada container aktif untuk aplikasi ini. Langkah perbaikan: Silakan lakukan deployment pertama Anda untuk melihat log runtime container di sini.";
const TOMBOL_KEMBALI_KE_BAWAH: &str = "Kembali ke Bawah \\/";

/// Auto-follow, toggle wrap, dan indikator sambungan — satu-satunya JS di luar
/// HTMX pada fase ini (`docs/design/log-viewer.md` §5.1, §5.2, §4.6). Perilaku
/// ini tidak bisa dicapai dengan CSS: ia butuh membaca `scrollTop` dan bereaksi
/// terhadap event scroll serta event SSE.
///
/// **Ditulis tanpa karakter `<`, `>`, `&`, dan `"` dengan sengaja.** Maud
/// meng-escape isi elemen `script` seperti teks biasa, dan JS yang memuat
/// karakter itu akan rusak jadi `&lt;`. Alternatifnya adalah `PreEscaped`, yang
/// tidak dipakai di file ini karena kehadirannya satu kali saja melemahkan
/// pemeriksaan "nol `PreEscaped`" yang menjaga isi log tetap ter-escape.
/// Karena itu perbandingan `a <= b` ditulis lewat `Math.max`, dan `&&` ditulis
/// sebagai `if` bersarang. Ada test yang menjaganya.
///
/// Nol string Bahasa Indonesia di sini: label status hidup sebagai dua elemen
/// di HTML dan JS hanya menyalakan/mematikan kelasnya, jadi copywriting tetap
/// satu sumber di template.
// ponytail: satu blok inline tanpa modul/bundler, batasnya semua interaksi
// viewer harus muat di bawah ~40 baris; upgrade saat viewer butuh state klien
// yang lebih dari "di dasar atau tidak" dan "tersambung atau tidak".
const JS_VIEWER: &str = "
(function () {
  var konsol = document.getElementById('log-console');
  if (!konsol) { return; }
  var follow = document.getElementById('follow-checkbox');
  var tombol = document.getElementById('back-to-bottom-btn');
  var wrap = document.getElementById('wrap-checkbox');
  var status = document.getElementById('log-status');
  function jarakDariDasar() {
    return Math.max(0, konsol.scrollHeight - konsol.scrollTop - konsol.clientHeight - 10);
  }
  function keDasar() { konsol.scrollTop = konsol.scrollHeight; }
  konsol.addEventListener('scroll', function () {
    if (jarakDariDasar() === 0) {
      if (tombol) { tombol.hidden = true; }
      return;
    }
    if (follow) { follow.checked = false; }
    if (tombol) { tombol.hidden = false; }
  });
  if (tombol) {
    tombol.addEventListener('click', function () {
      if (follow) { follow.checked = true; }
      keDasar();
      tombol.hidden = true;
    });
  }
  if (wrap) {
    wrap.addEventListener('change', function () {
      konsol.classList.toggle('log-console-wrap', wrap.checked);
    });
  }
  document.body.addEventListener('htmx:sseMessage', function () {
    if (!follow) { return; }
    if (follow.checked) { keDasar(); }
  });
  if (status) {
    document.body.addEventListener('htmx:sseError', function () {
      status.classList.add('log-status-terputus');
    });
    document.body.addEventListener('htmx:sseOpen', function () {
      status.classList.remove('log-status-terputus');
    });
  }
})();
";

/// Tanggalkan escape ANSI (CSI `\x1b[...m` dan kerabatnya) dari satu baris log.
///
/// Backend sengaja meneruskan byte log apa adanya (`docs/plan.md`: "warna ANSI
/// tidak diproses backend sama sekali"), jadi penanggalan terjadi di sisi
/// render. Tanpa ini, `\x1b[32mOK\x1b[0m` tampil sebagai `[32mOK[0m` — sampah
/// yang justru mempersulit pembacaan saat sesuatu sedang rusak.
///
/// Juga membuang karakter kontrol C0 lain kecuali tab, supaya baris log tidak
/// bisa memindahkan kursor atau mengacak tata letak halaman.
fn tanggalkan_ansi(teks: &str) -> String {
    let mut keluar = String::with_capacity(teks.len());
    let mut sisa = teks.chars().peekable();
    while let Some(c) = sisa.next() {
        if c == '\x1b' {
            // CSI: `\x1b[` + parameter + satu byte akhir di rentang 0x40..=0x7e.
            if sisa.peek() == Some(&'[') {
                sisa.next();
                for lanjut in sisa.by_ref() {
                    if ('\x40'..='\x7e').contains(&lanjut) {
                        break;
                    }
                }
            } else {
                // Escape dua-karakter (mis. `\x1bM`) — buang penandanya saja.
                sisa.next();
            }
            continue;
        }
        if c == '\t' || !c.is_control() {
            keluar.push(c);
        }
    }
    keluar
}

/// Pisahkan stempel waktu di awal baris dari isinya, supaya gutter timestamp
/// bisa dirender di kolom sendiri (`docs/design/log-viewer.md` §5.3).
///
/// Writer log deploy menulis `HH:MM:SS | pesan`
/// (`deployments::engine::baris_berstempel`). Baris yang tidak berpola itu —
/// termasuk log runtime dan cetakan multi-baris aplikasi — mengembalikan gutter
/// `None`, dan template mengisinya dengan spasi lebar tetap supaya indentasi
/// teks di sebelah kanan tetap lurus.
fn pisahkan_gutter(teks: &str) -> (Option<&str>, &str) {
    let Some((depan, sisa)) = teks.split_once(" | ") else {
        return (None, teks);
    };
    let berpola_jam = depan.len() == 8
        && depan.as_bytes()[2] == b':'
        && depan.as_bytes()[5] == b':'
        && depan
            .as_bytes()
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || i == 5 || b.is_ascii_digit());
    if berpola_jam {
        (Some(depan), sisa)
    } else {
        (None, teks)
    }
}

/// Area konsol: daftar baris + penanda status. Dipakai halaman penuh maupun
/// fragmen HTMX, supaya keduanya tidak bisa berbeda tampilan.
fn baris_konsol(baris: &[LogLine], truncated: bool, pencarian_dipotong: bool) -> Markup {
    html! {
        @if baris.is_empty() {
            div.log-line.log-line-info { span.log-gutter {} span.log-text { (PESAN_MENUNGGU) } }
        }
        @for b in baris {
            @let bersih = tanggalkan_ansi(&b.teks);
            @let (jam, isi) = pisahkan_gutter(&bersih);
            div.log-line {
                span.log-gutter { @if let Some(jam) = jam { (jam) } }
                span.log-text { (isi) }
            }
        }
        @if pencarian_dipotong {
            div.log-line.log-line-info { span.log-gutter {} span.log-text { (PESAN_PENCARIAN_DIPOTONG) } }
        }
        @if truncated {
            div.log-line.log-line-warning { span.log-gutter {} span.log-text { (PESAN_TERPOTONG) } }
        }
    }
}

/// Halaman viewer log deploy. `streaming` = deployment belum selesai, artinya
/// halaman memasang langganan SSE ke `/events/log/deploy/{id}`; `false` berarti
/// isi statis TANPA membuka SSE sama sekali — klien tidak menunggu event yang
/// tidak akan datang.
// Pola sama `web::apps::render_app_detail` (`src/web/apps.rs:127`): halaman
// penuh memang butuh banyak potongan data, dan membungkusnya jadi struct hanya
// untuk memuaskan lint akan menambah tipe yang tidak dipakai di tempat lain.
#[allow(clippy::too_many_arguments)]
pub fn render_deploy_log(
    dep: &DeploymentRingkas,
    truncated: bool,
    baris: &[LogLine],
    pencarian_dipotong: bool,
    q: Option<&str>,
    streaming: bool,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let url_isi = format!("/deployments/{}/log/isi", dep.id);
    let ada_file = !baris.is_empty() || truncated;
    let content = html! {
        div.log-back { a href=(format!("/deployments/{}", dep.id)) { "< Kembali ke Deployment" } }

        div.detail-title-row {
            h1 { "Log Deployment: " code { (dep.id) } }
            (badge_deployment(dep.status))
        }

        (toolbar(
            &url_isi,
            q,
            streaming,
            Some(&format!("/deployments/{}/log/unduh", dep.id)),
            ada_file,
        ))

        @if streaming {
            div.log-console-shell
                hx-ext="sse"
                sse-connect=(format!("/events/log/deploy/{}", dep.id))
            {
                (konsol(baris, truncated, pencarian_dipotong, true))
            }
        } @else {
            div.log-console-shell {
                (konsol(baris, truncated, pencarian_dipotong, false))
            }
        }

        script { (JS_VIEWER) }
    };

    base_page(
        &format!("Log Deployment {} - Mengploy", dep.id),
        app_shell(Some(csrf_token), strip, content),
    )
}

/// Toolbar kontrol: kotak cari (HTMX, pencarian di sisi server), toggle wrap,
/// toggle follow, tombol unduh. `url_unduh` `None` untuk log runtime — runtime
/// tidak dipersistensi di control plane, jadi tidak ada yang bisa diunduh
/// (`docs/design/log-viewer.md` §5.5).
fn toolbar(
    url_isi: &str,
    q: Option<&str>,
    streaming: bool,
    url_unduh: Option<&str>,
    ada_file: bool,
) -> Markup {
    html! {
        p.log-privacy-note { (PERINGATAN_PRIVASI) }

        div.log-toolbar {
            form.log-search
                hx-get=(url_isi)
                hx-target="#log-console"
                hx-swap="innerHTML"
                hx-trigger="submit"
            {
                label.sr-only for="log-q" { "Cari log" }
                input id="log-q" name="q" type="search" placeholder="Cari log..."
                    value=[q];
                button.btn type="submit" { "Cari" }
                button.btn.btn-secondary type="button"
                    hx-get=(url_isi)
                    hx-target="#log-console"
                    hx-swap="innerHTML" { "Batal" }
            }

            div.log-toggles {
                label.log-toggle {
                    input id="wrap-checkbox" type="checkbox"; " Wrap"
                }
                label.log-toggle {
                    input id="follow-checkbox" type="checkbox" checked[streaming] disabled[!streaming];
                    " Follow"
                }
                @if let Some(url) = url_unduh {
                    @if ada_file {
                        a.btn href=(url) { "Unduh" }
                    } @else {
                        span.log-download-disabled
                            title="Berkas log telah dihapus berdasarkan aturan retensi 30 hari." {
                            "Unduh"
                        }
                    }
                }
            }
        }
    }
}

/// Kerangka area konsol: `<pre><code role="log">` dengan `aria-live="off"`
/// (`docs/design/log-viewer.md` §7 — log streaming bisa ribuan baris, `polite`
/// akan membajak fokus pembaca layar), penanda status, dan tombol melayang.
///
/// `sse-swap` memakai `beforeend` supaya histori yang sudah dirender **tidak
/// hilang** tiap event; `sse-swap` seluruh isi akan membuang baris lama.
fn konsol(baris: &[LogLine], truncated: bool, pencarian_dipotong: bool, streaming: bool) -> Markup {
    html! {
        div.log-status-row {
            @if streaming {
                // Dua label hidup berdampingan; CSS memilih yang tampil
                // berdasarkan kelas `log-status-terputus`, jadi copywriting
                // tetap satu sumber di Maud dan JS tidak membentuk teks apa pun.
                span.log-status.log-status-streaming id="log-status" {
                    span.log-status-sehat { "[*] STREAMING" }
                    span.log-status-putus { "[!] MENGHUBUNGKAN ULANG" }
                    span.log-status-putus-detail { (PESAN_JARINGAN_TERPUTUS) }
                }
            } @else {
                span.log-status.log-status-arsip { "[ARSIP]" }
            }
        }

        pre.log-console id="log-console"
            sse-swap=[streaming.then_some("message,tertinggal,selesai")]
            hx-swap="beforeend"
        {
            code role="log" aria-label="Log Aplikasi" aria-live="off" {
                (baris_konsol(baris, truncated, pencarian_dipotong))
            }
        }

        button.log-back-to-bottom id="back-to-bottom-btn" type="button" hidden {
            "[ " (TOMBOL_KEMBALI_KE_BAWAH) " ]"
        }
    }
}

/// Fragmen HTML daftar baris (tanpa app shell) — dipakai HTMX untuk ganti
/// `tail`, pencarian, dan muat ulang isi. `selesai` tidak mengubah isi baris,
/// hanya dipakai pemanggil SSE sebagai penanda event terakhir.
pub fn render_log_fragmen(
    baris: &[LogLine],
    truncated: bool,
    pencarian_dipotong: bool,
    selesai: bool,
) -> Markup {
    html! {
        (baris_konsol(baris, truncated, pencarian_dipotong))
        @if selesai {
            div.log-line.log-line-info { span.log-gutter {} span.log-text { "--- [i] Aliran log selesai. ---" } }
        }
    }
}

/// Fragmen satu pesan kategori (state kosong / container hilang / server tidak
/// merespons / lag / sesi 30 menit). Teks final datang dari pemanggil, yang
/// mengambilnya dari `docs/design/log-viewer.md` §8 — file ini tidak menebak
/// pesan, supaya tidak ada dua sumber teks yang bisa berbeda.
pub fn render_log_pesan(pesan: &str) -> Markup {
    let kelas = if pesan.starts_with("[x]") || pesan.starts_with("--- [x]") {
        "log-line log-line-danger"
    } else if pesan.starts_with("[!]") || pesan.starts_with("--- [!]") {
        "log-line log-line-warning"
    } else {
        "log-line log-line-info"
    };
    html! {
        div class=(kelas) {
            span.log-gutter {}
            span.log-text { (pesan) }
        }
    }
}

/// Empat tab detail app (Environment ditambah Fase 4). Overview adalah
/// halaman `/apps/{id}` yang sudah ada dan isinya TIDAK berubah di fase ini.
pub(super) fn tab_nav(app_id: &str, aktif: &str) -> Markup {
    let tabs = [
        ("overview", "Overview", format!("/apps/{app_id}")),
        (
            "deployments",
            "Deployments",
            format!("/apps/{app_id}/deployments"),
        ),
        ("logs", "Logs", format!("/apps/{app_id}/logs")),
        ("environment", "Environment", format!("/apps/{app_id}/env")),
        (
            "reconciliation",
            "Rekonsiliasi",
            format!("/apps/{app_id}/reconciliation"),
        ),
    ];
    html! {
        nav.app-tabs aria-label="Tab detail app" {
            ul {
                @for (kunci, label, url) in &tabs {
                    li {
                        @if *kunci == aktif {
                            span.app-tab.app-tab-aktif aria-current="page" { (label) }
                        } @else {
                            a.app-tab href=(url) { (label) }
                        }
                    }
                }
            }
        }
    }
}

/// Tab Deployments — riwayat satu app (`docs/design/riwayat-deployment.md`).
/// `dipotong` = riwayat melebihi 100 terbaru. **Read-only**: nol tombol
/// rollback, itu Fase 5 (`docs/prd.md:326`).
pub fn render_app_tab_deployments(
    app: &AppRingkas,
    deploys: &[DeploymentRingkas],
    dipotong: bool,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.detail-title-row { h1 { "App: " (app.name) } }
        (tab_nav(&app.id, "deployments"))

        section.detail-card aria-labelledby="judul-riwayat" {
            h2 id="judul-riwayat" { "Riwayat Deployment" }
            @if deploys.is_empty() {
                p { "Belum pernah dideploy. Langkah perbaikan: Jalankan deploy pertama dari CI Anda dengan token deploy app ini." }
            } @else {
                table.fleet-table {
                    thead {
                        tr {
                            th scope="col" { "Waktu" }
                            th scope="col" { "Status" }
                            th scope="col" { "Commit" }
                            th scope="col" { "Image Digest" }
                            th scope="col" { "Durasi" }
                            th scope="col" { "Log" }
                        }
                    }
                    tbody {
                        @for d in deploys {
                            tr {
                                td { a href=(format!("/deployments/{}", d.id)) { (format_epoch_opt(Some(d.created_at))) } }
                                td { (badge_deployment(d.status)) }
                                td { code { (commit_pendek(&d.commit_sha)) } }
                                td { code.digest-cell { (d.image_digest) } }
                                td { (durasi(d)) }
                                td { a href=(format!("/deployments/{}/log", d.id)) { "Lihat log" } }
                            }
                        }
                    }
                }
                @if dipotong {
                    p.log-privacy-note { "Menampilkan 100 deployment terbaru." }
                }
            }
        }
    };

    base_page(
        &format!("Deployments {} - Mengploy", app.name),
        app_shell(Some(csrf_token), strip, content),
    )
}

/// Tab Logs (halaman). `ada_container` `false` → state "belum ada container
/// yang berjalan", SSE TIDAK dipasang, status tetap 200.
pub fn render_app_tab_logs(
    app: &AppRingkas,
    ada_container: bool,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let url_isi = format!("/apps/{}/logs/isi", app.id);
    let content = html! {
        div.detail-title-row { h1 { "App: " (app.name) } }
        (tab_nav(&app.id, "logs"))

        @if ada_container {
            // Tanpa tombol unduh: log runtime tidak dipersistensi di control
            // plane, jadi tidak ada berkas yang bisa diunduh.
            (toolbar(&url_isi, None, true, None, false))
            div.log-console-shell
                hx-ext="sse"
                sse-connect=(format!("/events/log/runtime/{}", app.id))
            {
                (konsol(&[], false, false, true))
            }
            script { (JS_VIEWER) }
        } @else {
            section.detail-card aria-labelledby="judul-log-runtime" {
                h2 id="judul-log-runtime" { "Log Runtime" }
                (render_log_pesan(PESAN_BELUM_ADA_CONTAINER))
            }
        }
    };

    base_page(
        &format!("Log Runtime: {} - Mengploy", app.name),
        app_shell(Some(csrf_token), strip, content),
    )
}

fn durasi(dep: &DeploymentRingkas) -> String {
    match (dep.started_at, dep.finished_at) {
        (Some(mulai), Some(selesai)) if selesai >= mulai => {
            let detik = selesai - mulai;
            format!("{}m {}s", detik / 60, detik % 60)
        }
        (Some(_), None) if !dep.status.selesai() => "berjalan".to_string(),
        _ => "-".to_string(),
    }
}

fn commit_pendek(sha: &str) -> String {
    sha.chars().take(7).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::deployments::StatusDeployment;

    fn baris(teks: &str) -> Vec<LogLine> {
        vec![LogLine {
            nomor: 1,
            teks: teks.to_string(),
        }]
    }

    fn app() -> AppRingkas {
        AppRingkas {
            id: "app1".to_string(),
            server_id: "srv1".to_string(),
            name: "api".to_string(),
            health_path: "/health".to_string(),
            health_grace_secs: 30,
            port: 8080,
            restart_policy: "unless-stopped".to_string(),
            repo_url: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn dep(status: StatusDeployment) -> DeploymentRingkas {
        DeploymentRingkas {
            id: "dep1".to_string(),
            app_id: "app1".to_string(),
            commit_sha: "abcdef1234567890".to_string(),
            git_ref: Some("main".to_string()),
            image_digest: format!("ghcr.io/org/api@sha256:{}", "a".repeat(64)),
            status,
            container_id: None,
            env_version_id: None,
            error_kind: None,
            error_detail: None,
            started_at: Some(100),
            finished_at: Some(190),
            created_at: 0,
        }
    }

    /// Maud meng-escape isi `script` seperti teks biasa. Kalau JS viewer memuat
    /// `<`, `>`, `&`, atau `"`, ia akan rusak jadi entitas HTML dan seluruh
    /// interaksi viewer mati tanpa suara. Test ini yang menahannya.
    #[test]
    fn js_viewer_tidak_memuat_karakter_yang_akan_dieskape_maud() {
        for c in ['<', '>', '&', '"'] {
            assert!(
                !JS_VIEWER.contains(c),
                "JS viewer memuat '{c}' — Maud akan meng-escape-nya dan skripnya rusak"
            );
        }
        let dirender = html! { script { (JS_VIEWER) } }.into_string();
        assert!(
            !dirender.contains("&amp;") && !dirender.contains("&lt;"),
            "skrip ter-escape saat dirender: {dirender}"
        );
    }

    #[test]
    fn isi_log_dieskape_bukan_dieksekusi() {
        let markup = render_log_fragmen(&baris("<script>alert(1)</script>"), false, false, false)
            .into_string();

        assert!(
            !markup.contains("<script>alert"),
            "isi log adalah data tidak tepercaya: {markup}"
        );
        assert!(markup.contains("&lt;script&gt;"));
    }

    #[test]
    fn escape_ansi_ditanggalkan_bukan_ditampilkan_sebagai_sampah() {
        assert_eq!(tanggalkan_ansi("\x1b[32mOK\x1b[0m"), "OK");
        assert_eq!(tanggalkan_ansi("\x1b[1;31mgagal\x1b[m sisa"), "gagal sisa");
        // Tab dipertahankan (indentasi log punya arti), karakter kontrol lain
        // dibuang supaya tidak bisa mengacak tata letak.
        assert_eq!(tanggalkan_ansi("a\tb\rc"), "a\tbc");
        assert_eq!(tanggalkan_ansi("tanpa escape"), "tanpa escape");
        // Escape yang terpotong di ujung baris tidak boleh membocorkan sisanya.
        assert_eq!(tanggalkan_ansi("x\x1b[3"), "x");
    }

    #[test]
    fn gutter_dipisah_hanya_untuk_baris_berstempel_waktu() {
        assert_eq!(
            pisahkan_gutter("12:00:01 | menarik image"),
            (Some("12:00:01"), "menarik image")
        );
        // Baris log aplikasi yang kebetulan memuat " | " tidak boleh dipotong.
        assert_eq!(pisahkan_gutter("GET /a | 200"), (None, "GET /a | 200"));
        assert_eq!(pisahkan_gutter("tanpa pemisah"), (None, "tanpa pemisah"));
    }

    #[test]
    fn log_kosong_menampilkan_state_menunggu() {
        let markup = render_log_fragmen(&[], false, false, false).into_string();
        assert!(markup.contains("Menunggu keluaran log pertama"));
    }

    #[test]
    fn truncated_menampilkan_penanda_8_mib() {
        let markup = render_log_fragmen(&baris("a"), true, false, false).into_string();
        assert!(markup.contains("LOG TERPOTONG"));
        assert!(markup.contains("8 MiB"));
    }

    #[test]
    fn pencarian_dipotong_menampilkan_penanda_500_baris() {
        let markup = render_log_fragmen(&baris("a"), false, true, false).into_string();
        assert!(markup.contains("HASIL DIPOTONG"));
        assert!(markup.contains("500 baris"));
    }

    #[test]
    fn pesan_kategori_diberi_kelas_sesuai_tingkat() {
        assert!(
            render_log_pesan("[x] gagal")
                .into_string()
                .contains("log-line-danger")
        );
        assert!(
            render_log_pesan("[!] hati-hati")
                .into_string()
                .contains("log-line-warning")
        );
        assert!(
            render_log_pesan("[i] info")
                .into_string()
                .contains("log-line-info")
        );
    }

    #[test]
    fn deployment_berjalan_memasang_sse_dan_follow_aktif() {
        let markup = render_deploy_log(
            &dep(StatusDeployment::Pulling),
            false,
            &baris("mulai"),
            false,
            None,
            true,
            "tok",
            None,
        )
        .into_string();

        assert!(markup.contains("/events/log/deploy/dep1"));
        assert!(markup.contains("[*] STREAMING"));
        // `beforeend`, bukan swap seluruh isi — histori tidak boleh hilang.
        assert!(markup.contains(r#"hx-swap="beforeend""#));
    }

    #[test]
    fn deployment_selesai_tidak_membuka_sse() {
        let markup = render_deploy_log(
            &dep(StatusDeployment::Live),
            false,
            &baris("selesai"),
            false,
            None,
            false,
            "tok",
            None,
        )
        .into_string();

        assert!(
            !markup.contains("sse-connect"),
            "isi statis TIDAK boleh membuka SSE: klien akan menunggu event yang tidak datang"
        );
        assert!(markup.contains("[ARSIP]"));
        assert!(!markup.contains("sse-swap"));
    }

    #[test]
    fn peringatan_privasi_selalu_ada_di_viewer() {
        let markup = render_deploy_log(
            &dep(StatusDeployment::Live),
            false,
            &baris("x"),
            false,
            None,
            false,
            "tok",
            None,
        )
        .into_string();
        assert!(markup.contains("dapat memuat informasi sensitif"));
    }

    #[test]
    fn unduh_dinonaktifkan_saat_berkas_sudah_tersapu_retensi() {
        let ada = render_deploy_log(
            &dep(StatusDeployment::Live),
            false,
            &baris("x"),
            false,
            None,
            false,
            "tok",
            None,
        )
        .into_string();
        assert!(ada.contains("/deployments/dep1/log/unduh"));

        let kosong = render_deploy_log(
            &dep(StatusDeployment::Live),
            false,
            &[],
            false,
            None,
            false,
            "tok",
            None,
        )
        .into_string();
        assert!(!kosong.contains("/deployments/dep1/log/unduh"));
        assert!(kosong.contains("retensi 30 hari"));
    }

    #[test]
    fn kata_kunci_pencarian_dikembalikan_ke_kotak_cari_dalam_keadaan_ter_escape() {
        let markup = render_deploy_log(
            &dep(StatusDeployment::Live),
            false,
            &[],
            false,
            Some("\"><b>x"),
            false,
            "tok",
            None,
        )
        .into_string();

        assert!(!markup.contains("<b>x"), "{markup}");
        assert!(markup.contains("&lt;b&gt;x"));
    }

    #[test]
    fn tab_logs_tanpa_container_tidak_membuka_sse() {
        let markup = render_app_tab_logs(&app(), false, "tok", None).into_string();

        assert!(!markup.contains("sse-connect"));
        assert!(markup.contains("Belum ada container aktif"));
    }

    #[test]
    fn tab_logs_dengan_container_membuka_sse_runtime_tanpa_tombol_unduh() {
        let markup = render_app_tab_logs(&app(), true, "tok", None).into_string();

        assert!(markup.contains("/events/log/runtime/app1"));
        assert!(
            !markup.contains("/log/unduh"),
            "log runtime tidak dipersistensi, jadi tidak ada yang bisa diunduh"
        );
    }

    #[test]
    fn tab_deployments_kosong_menampilkan_state_kosong() {
        let markup = render_app_tab_deployments(&app(), &[], false, "tok", None).into_string();
        assert!(markup.contains("Belum pernah dideploy"));
    }

    #[test]
    fn tab_deployments_menampilkan_digest_commit_dan_tautan_log() {
        let markup =
            render_app_tab_deployments(&app(), &[dep(StatusDeployment::Live)], false, "tok", None)
                .into_string();

        assert!(markup.contains("abcdef1"));
        assert!(markup.contains("sha256:aaaa"));
        assert!(markup.contains("/deployments/dep1/log"));
        assert!(markup.contains("1m 30s"));
        assert!(
            !markup.to_lowercase().contains("rollback"),
            "tab Deployments read-only; rollback adalah Fase 5"
        );
    }

    #[test]
    fn tab_deployments_dipotong_menampilkan_penanda_100_terbaru() {
        let markup =
            render_app_tab_deployments(&app(), &[dep(StatusDeployment::Live)], true, "tok", None)
                .into_string();
        assert!(markup.contains("100 deployment terbaru"));
    }

    #[test]
    fn tab_aktif_ditandai_dan_tidak_jadi_tautan() {
        let markup = render_app_tab_deployments(&app(), &[], false, "tok", None).into_string();
        assert!(markup.contains(r#"aria-current="page""#));
        assert!(markup.contains(r#"href="/apps/app1/logs""#));
        assert!(markup.contains(r#"href="/apps/app1""#));
    }

    #[test]
    fn durasi_dihitung_dari_started_dan_finished() {
        let mut d = dep(StatusDeployment::Live);
        assert_eq!(durasi(&d), "1m 30s");
        d.finished_at = None;
        d.status = StatusDeployment::Pulling;
        assert_eq!(durasi(&d), "berjalan");
        d.started_at = None;
        d.status = StatusDeployment::Queued;
        assert_eq!(durasi(&d), "-");
    }
}
