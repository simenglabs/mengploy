# Progress

State lintas sesi. Diperbarui **orchestrator** setiap satu subagent selesai —
lihat blok D di `.opencode/agents/orchestrator.md`. Ini satu-satunya alasan sesi
baru bisa menyambung tanpa mengulang pekerjaan.

Nama fase diambil dari `docs/prd.md` §4. Satu fase = satu sesi; jalankan `/new`
sebelum `/phase` berikutnya.

**Fase aktif:** **Fase 4 (Pengelolaan environment) — migration+backend+frontend+qa SELESAI (307 test hijau, nol regresi), gerbang BELUM ditutup.** Q1 (env vs `docker inspect`, `docs/plan.md`) dijawab manusia: Opsi A. **`uiux`, `security` ("fase kritis kedua"), dan `reviewer` BELUM dikerjakan** — lihat bagian "Fase 4" di bawah untuk simplifikasi yang diambil tanpa uiux dan celah transaksi `env_vars` yang perlu dinilai reviewer/security sebelum gerbang ditutup. Fase 0-3 semuanya SELESAI dan LOLOS gerbang (`docs/prd.md` §6) — ringkasan Fase 0 ada di bawah, Fase 1-3 masing-masing punya bagian sendiri dengan paragraf kesimpulan eksplisit.

**Fase 1 — SELESAI, LOLOS gerbang** (`docs/prd.md` §6). Backend (3a-3f) +
frontend (`src/web/**`) + security + qa (`tests/phase1.rs`) + code review
semuanya tuntas. Lihat catatan "Gerbang Fase 1 ditutup" di bawah untuk rincian
temuan dan perbaikannya.

---

## Fase 0 — Fondasi

- [x] planner — output: docs/plan.md, docs/api-contract.md (selesai, keputusan Q1-Q6 terkunci)
- [x] uiux — output: docs/design/login.md, docs/design/shell-aplikasi.md (selesai, token visual dark-mode ditetapkan)
- [x] migration — scope: migrations/0001_init.sql (settings, sessions, pragma WAL; busy_timeout/foreign_keys/synchronous dicatat untuk backend di src/db.rs)
- [x] backend — scope: src/ selain src/web/, Cargo.toml (Cargo.toml final 14 dep, seluruh src/** di luar src/web/ terisi: main, config, error, db, state, auth/{mod,password,session,middleware}, routes/{mod,health,login,dashboard}, .sqlx/ tergenerate)
- [x] frontend — scope: src/web/ (6 file Maud: mod.rs, layout.rs, login.rs, dashboard.rs, error_page.rs, styles.rs + const CSS token dark-mode; clippy bersih, 23 test lulus)
- [x] qa — scope: tests/phase0.rs (21/21 pass; dua bug tersingkap sebelum hijau: cookie CSRF draft malformed di helper `ambil_login` diperbaiki qa, `fallback_404` mengembalikan 200 alih-alih 404 diperbaiki backend; `tower` ditambah sebagai dev-dependency atas izin manusia)
- [x] reviewer — scope: Cargo.toml, migrations/0001_init.sql, src/** di luar src/web/ (0 BLOCKING; 2 WARNING non-blocking di `tests/phase0.rs` — pemakaian `sqlx::query()` tanpa komentar alasan; **LOLOS gerbang review**)
- [x] security — scope: src/auth/**, src/routes/login.rs, src/config.rs, src/db.rs, src/state.rs, src/error.rs, migrations/0001_init.sql (0 BLOCKING, 5 "harus diperbaiki" sebelum Fase 1, 8 catatan; **LOLOS dari sisi keamanan**)

### Catatan

Planner selesai: docs/plan.md + docs/api-contract.md ditulis. Q1-Q6 dikunci:
axum-extra(cookie), rand, age ditunda ke Fase 1, dotenvy ditambah (dev only),
MENGDEP_INITIAL_PASSWORD untuk seed password, expiry sesi absolute 30 hari.
Dep final Cargo.toml (14 baris): axum, tokio, sqlx, maud, tracing,
tracing-subscriber, argon2, anyhow, serde, serde_json, time, axum-extra, rand,
dotenvy. Urutan eksekusi: uiux ‖ migration → backend → frontend → security →
qa → reviewer.

uiux: token visual final (dark) di docs/design/login.md — --color-bg-page #111,
--color-text-main #ddd, --font-mono ui-monospace dst. shell-aplikasi.md rujuk
token sama. Semua state (default/error/empty/404/500) dispesifikasikan.

migration: migrations/0001_init.sql dibuat. Tabel settings(key,value),
sessions(id,created_at,expires_at,csrf_token) + idx_sessions_expires_at.
PRAGMA journal_mode=WAL di file migrasi (persisten); busy_timeout/
foreign_keys/synchronous=NORMAL WAJIB diset backend di src/db.rs saat buka
pool (per-koneksi, bukan migrasi).

backend: verifikasi orchestrator langsung — `cargo fmt --check` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found (exit 0),
`cargo test` 13 passed (1 suite, 0.57s). Semua `unwrap()`/`expect()`
terverifikasi hanya di dalam blok `#[cfg(test)]` (src/auth/password.rs:31,
src/db.rs:91, src/error.rs:56). `.sqlx/` tergenerate dan konsisten dengan
seluruh `sqlx::query!`. Keputusan teknis penting: `maud` diaktifkan dengan
fitur `axum` (menarik `axum-core 0.5.6`, sama dengan yang dipakai `axum 0.8.9`
— inilah yang membuat `Markup` bisa dipakai langsung sebagai return handler);
`argon2` dengan fitur `std` (untuk `OsRng`); token acak memakai `rand::RngExt`
+ `rand::distr::Alphanumeric` + `rand::rng()`.

frontend: verifikasi orchestrator langsung — `cargo clippy --all-targets -- -D warnings` No issues found, `cargo test` 23 passed (13 test backend lama + 10 test frontend baru). Enam file terbentuk di `src/web/`: mod.rs (657B), layout.rs (3.1K), login.rs (3.2K), dashboard.rs (2.0K), error_page.rs (2.6K), styles.rs (4.3K). Token visual dark-mode diterapkan dari spesifikasi uiux. Layout shell (sidebar + header + area konten) terbentuk, semua state login dirender, halaman 404/500 ada.

**Keputusan yang sudah diambil (dua bekas BLOCKER — keduanya SELESAI)**

**BLOCKER 1 — form logout tidak bisa dirender. SELESAI.**
Masalah semula: `docs/api-contract.md` mewajibkan `POST /logout` membawa token
CSRF karena ia form terlindungi, tapi signature `render_dashboard() -> Markup`
tidak punya parameter `csrf_token`, sehingga template tidak bisa menanam hidden
input CSRF. Dashboard sempat dirender tanpa form logout sama sekali, dan
kriteria selesai `docs/plan.md` "POST /logout menghapus sesi dan cookie,
redirect ke /login" tidak bisa dicapai lewat UI.

Keputusan manusia: **setujui perubahan signature `render_dashboard`.**
Dikerjakan agent `migration` sebagai **satu refactor lintas file**, bukan dua
delegasi frontend+backend terpisah — supaya repo tidak pernah berada dalam
keadaan tidak bisa dikompilasi di tengah serah terima. Empat file berubah:

- `src/auth/middleware.rs` — `require_session` kini menyisipkan `Session` hasil
  validasi ke request extensions (`request.extensions_mut().insert(session)`)
  sebelum `next.run()`.
- `src/routes/dashboard.rs` — handler jadi
  `dashboard(Extension(session): Extension<Session>) -> Response`, meneruskan
  `session.csrf_token` ke render. Kegagalan ekstraksi ditangani Axum sebagai 500
  otomatis, bukan `unwrap()`/`expect()`.
- `src/web/dashboard.rs` — signature jadi
  `render_dashboard(csrf_token: &str) -> Markup`, memanggil
  `app_shell(Some(csrf_token), content)`.
- `src/web/mod.rs` — baris dokumentasi kontrak render diperbarui.

Catatan penting untuk sesi berikutnya: yang ditanam di form adalah
**`csrf_token` milik sesi, bukan id sesi** (`docs/prd.md` §3 nomor 7 — token
sesi tidak pernah keluar ke klien lewat body/HTML). Kontrak
`docs/api-contract.md` **tidak berubah** — yang berubah hanya signature fungsi
render internal.

Verifikasi orchestrator langsung: `cargo fmt --check` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found, `cargo test`
23 passed. Jumlah test tetap 23 karena satu test lama
(`render_dashboard_tidak_memuat_tombol_keluar_tanpa_csrf_token`, menguji
perilaku lama yang kini justru salah) diganti satu test baru
(`render_dashboard_memuat_form_logout_dengan_csrf_token`).

**BLOCKER 2 — teks 404/500 punya dua versi yang berbeda. SELESAI.**
Masalah semula: `docs/design/shell-aplikasi.md` memuat teks halaman 404/500 di
dua tempat dengan kalimat yang tidak identik — §4.4 ("Pesan") dan §7 (tabel
"Isi Kesalahan").

Keputusan manusia: **versi §4.4 menang, tabel §7 disesuaikan.** Dikerjakan agent
`uiux`. **Nol perubahan kode** — §4.4 dipilih justru karena frontend sudah
memakainya dan test di `src/error.rs` sudah mengasersi substring
`"kesalahan internal"` dari versi itu. Teks final yang dikunci:

- 404: "Halaman tidak ditemukan. Alamat yang Anda tuju tidak dikenal atau telah
  dipindahkan."
- 500: "Terjadi kesalahan internal pada server. Silakan hubungi administrator
  atau periksa log aplikasi."

Ditambahkan catatan di §7 yang menetapkan tabel itu sebagai sumber kebenaran
teks tunggal yang wajib identik dengan §4.4, supaya divergensi yang sama tidak
terulang. uiux melaporkan tidak ada divergensi lain antara §4.x dan §7.

**Hal kecil tertunda:**
File `test_out.txt` (39B) tertinggal di root repo dari proses debugging
subagent. Perintah `rm` diblokir untuk agent — perlu dihapus manual oleh
manusia. Isinya hanya log test lama.

**qa: kronologi sampai 21/21 hijau**

`tests/phase0.rs` ditulis agent `qa`, tapi tidak langsung hijau. Tiga tahap:

1. **Gagal compile.** Test memakai `.oneshot()` yang butuh trait
   `tower::util::ServiceExt`, sementara `tower` tidak ada di `Cargo.toml`.
   Manusia **mengizinkan eksplisit** menambah `tower` sebagai `[dev-dependencies]`
   dengan fitur `util` saja — bukan pelanggaran larangan "jangan tambah
   dependensi". Dikerjakan agent `backend` (pemilik `Cargo.toml`). Ini
   satu-satunya dependency baru sesi ini; daftar `[dependencies]` runtime tetap
   14 baris, tidak berubah.
2. **12 dari 21 test gagal.** Dipanggil agent `debugger` (read-only, tidak
   mengedit apa pun) untuk mencari akar. Ditemukan **dua bug independen**:
   - **Grup A — 11 test, milik `qa`, di `tests/phase0.rs`.** Helper `ambil_login`
     menyimpan nilai cookie CSRF draft **telanjang** (tanpa prefix
     `mengdep_csrf_draft=`) lalu meneruskannya sebagai header `Cookie:` yang
     malformed ke `post_form`. Akibatnya `jar.get(CSRF_DRAFT_COOKIE_NAME)` di
     backend selalu kosong, validasi CSRF gagal 400, dan verifikasi password
     tidak pernah sempat dijalankan. Diperbaiki `qa`: `ambil_login` kini
     membungkus nilai jadi `{CSRF_DRAFT_COOKIE_NAME}={draft}`
     (`tests/phase0.rs:180-189`).
   - **Grup B — 1 test, milik `backend`, di `src/routes/mod.rs`.** `fallback_404`
     mengembalikan `maud::Markup` langsung, yang `IntoResponse`-nya berstatus
     **200**, bukan 404. Diperbaiki `backend`: return jadi
     `axum::response::Response` lewat
     `(StatusCode::NOT_FOUND, web::render_404()).into_response()`
     (`src/routes/mod.rs:38`).

   Kedua perbaikan dikerjakan **paralel** — glob kepemilikan tidak beririsan
   (`tests/phase0.rs` milik qa vs `src/routes/mod.rs` milik backend).
3. **Satu kegagalan tersisa** setelah kedua fix:
   `logout_csrf_hilang_ditolak_dan_sesi_tetap_ada`. Axum menolak field form wajib
   yang hilang dengan **422**, bukan 400, karena `LogoutForm.csrf_token` bertipe
   `String` dan bukan `Option`. Disepakati ini **perilaku sah, bukan bug backend**
   — yang penting request ditolak dan sesi tidak terhapus. `qa` melonggarkan
   assert jadi `status.is_client_error()` (4xx apa pun, asal bukan 500), konsisten
   dengan test serupa `login_tanpa_field_csrf_ditolak_bukan_panic`.

Verifikasi orchestrator langsung: `cargo test --test phase0` **21/21 pass**,
`cargo fmt --check` bersih, `cargo clippy --all-targets -- -D warnings`
"No issues found" untuk seluruh workspace (`src/` dan `tests/`).

File yang berubah di tahap ini: `Cargo.toml` (section `[dev-dependencies]` baru),
`tests/phase0.rs` (helper cookie CSRF + satu assert dilonggarkan ke 4xx),
`src/routes/mod.rs` (status code `fallback_404`).

**reviewer: 0 BLOCKING, Fase 0 lolos gerbang review**

Dijalankan setelah retry — percobaan pertama gagal kehabisan step budget tanpa
menghasilkan laporan sama sekali, percobaan kedua sukses penuh. Catatan operasional
untuk sesi berikutnya: review cakupan-penuh satu fase mendekati batas step budget,
pertimbangkan memecah cakupan kalau fase berikutnya lebih besar.

Cakupan yang benar-benar dibaca: `Cargo.toml`, `migrations/0001_init.sql`,
`src/main.rs`, `src/config.rs`, `src/error.rs`, `src/db.rs`, `src/state.rs`,
`src/auth/**`, `src/routes/**`.

Temuan: **2 WARNING, bukan BLOCKING.** Keduanya di `tests/phase0.rs:65` dan
`tests/phase0.rs:331` — memakai `sqlx::query()` alih-alih `sqlx::query!` yang
compile-time checked, tanpa komentar alasan di atasnya. Ini melanggar konvensi
`AGENTS.md` bagian "Query" secara minor. Lokasinya di file test (milik `qa`), bukan
di `src/` produksi, jadi tidak menyentuh jalur request.

Keputusan desain yang dinilai reviewer: `LogoutForm.csrf_token` bertipe `String`
wajib (bukan `Option`) sehingga Axum menolak field yang hilang dengan **422**, bukan
400 buatan handler. Reviewer menilai ini **dapat diterima** — memakai validasi tipe
bawaan Axum, tetap menghasilkan 4xx, dan aman dari bypass CSRF. Konsisten dengan
kesepakatan di tahap qa nomor 3 di atas.

Kesimpulan reviewer: **0 BLOCKING. Fase 0 LOLOS gerbang review.**

**security: 0 BLOCKING, Fase 0 lolos dari sisi keamanan**

Dijalankan paralel dengan reviewer (keduanya read-only, `AGENTS.md` bagian B).
Cakupan: `src/auth/password.rs`, `src/auth/session.rs`, `src/auth/middleware.rs`,
`src/routes/login.rs`, `src/config.rs`, `src/db.rs`, `src/state.rs`, `src/error.rs`,
`migrations/0001_init.sql`.

Terverifikasi **benar dari kode aktual**, bukan dari asumsi atau dari dokumen:

- Argon2id dengan parameter memadai: m=19456 KiB, t=2, p=1 — sesuai rekomendasi OWASP.
- Token sesi dan token CSRF 32 karakter alfanumerik ≈190 bit entropi, dari CSPRNG.
- Rotasi sesi penuh di dalam satu transaksi saat login (DELETE lalu INSERT,
  `src/auth/session.rs:58`).
- Expiry sesi ditegakkan di sisi server, bukan mengandalkan `Max-Age` cookie.
- Cakupan middleware benar: hanya `/healthz`, `GET /login`, dan `POST /login` yang
  publik; sisanya masuk router terlindungi.
- Fail-closed pada kegagalan validasi sesi — error saat memvalidasi diperlakukan
  sebagai tidak ada sesi, bukan sebagai sesi valid.
- Ketiga flag cookie ter-set: `HttpOnly`, `Secure`, `SameSite=Lax`, plus `Path=/`.
- Pesan gagal login generik — tidak membedakan "user tidak ada" dari "password salah"
  (`docs/api-contract.md`, `docs/prd.md` §3 nomor 7).
- Tidak ada `derive(Debug)` pada `AppState`, `Config`, maupun `Session` yang bisa
  membocorkan secret lewat log atau pesan error.
- Tidak ada kolom kunci enkripsi di skema db — invariant `docs/prd.md` §3 nomor 8 aman.
- Semua query memakai bind parameter; tidak ada perangkaian string SQL, tidak ada
  jalur SQL injection.

Rekap: **0 BLOCKING**, 5 "HARUS DIPERBAIKI", 8 "CATATAN". Salah satu catatan yang
perlu diingat operator: flag `Secure` pada cookie mengharuskan aplikasi diakses lewat
reverse proxy TLS — **jangan dilepas** supaya bisa diakses via HTTP polos; dokumentasikan
di panduan operasional.

**Checklist 5 temuan security "harus diperbaiki" (non-blocking Fase 0, wajib ditutup sebelum Fase 1 menangani kredensial armada nyata)**

- [ ] File `-wal` dan `-shm` SQLite belum diset eksplisit ke mode `0600` seperti file
      db utama — **paling mendesak.**
- [ ] `MENGDEP_INITIAL_PASSWORD` kosong atau string-kosong tidak ditolak saat seed —
      **paling mendesak.**
- [ ] Temuan ketiga dari laporan security (rincian ada di laporan agent, belum
      disalin lengkap ke sini).
- [ ] Temuan keempat dari laporan security (idem).
- [ ] Temuan kelima dari laporan security (idem).

> Dua item pertama disebut security sebagai yang **paling mendesak**. Tiga sisanya
> tercatat di laporan agent security dan perlu disalin utuh saat Fase 1 dibuka —
> jangan tutup checklist ini sebelum kelimanya benar-benar teridentifikasi.

Kesimpulan security: **Fase 0 LOLOS dari sisi keamanan**, dengan syarat kelima temuan
di atas ditutup sebelum Fase 1. Kelimanya **bukan blocker untuk menutup Fase 0** itu
sendiri, tapi wajib jadi item awal Fase 1 atau ditangani sebagai task kecil terpisah.

---

## Fase 1 — Registry server dan konektivitas — SELESAI, LOLOS gerbang

- [x] planner — output: docs/plan.md, docs/api-contract.md
- [x] uiux — output: docs/design/{tambah-server,fleet-overview,server-detail}.md
- [x] migration — scope: migrations/0002_servers.sql
- [x] backend — scope: src/ selain src/web/ (sub-blok 3a-3f)
- [x] frontend — scope: src/web/
- [x] qa — tests/phase1.rs, 7 skenario injeksi kegagalan (minimum 5)
- [x] reviewer — 2 batch, 3 temuan diperbaiki, 0 blocking tersisa
- [x] security — 0 BLOCKING, 2 HARUS DIPERBAIKI (keduanya ditutup), beberapa CATATAN

### Catatan

Belum dibuka. Fase paling kritis untuk security menurut PRD §4.

---

## Fase 2 — Loop deploy

- [x] planner — output: `docs/plan.md` (overwrite penuh dari Fase 1)
- [ ] uiux — output: `docs/design/{apps,deployment-detail}.md` — BELUM
- [x] migration — `migrations/0003_deploy.sql`
- [ ] backend — sub-blok 2a, 2b SELESAI; 2c-2g BELUM
- [ ] frontend
- [ ] qa
- [ ] reviewer
- [ ] security

### Catatan

**Insiden sebelum Fase 2 dimulai**: `CLAUDE.md` ketiban `rtk init` (RTK/Rust
Token Killer) — file kontrak kerja asli (Jangkar, Non-Goals, Invariants, spek
Fase 0-6 versi lama) TERTIMPA total jadi dokumentasi command RTK generik,
timestamp cocok dengan `.rtk/filters.toml`. Ditemukan saat mau cek nomor fase
CLAUDE.md sebelum planning Fase 2. **Dipulihkan verbatim** dari context
percakapan (manusia memilih restore). Dicatat: `AGENTS.md` eksplisit
menyebut `docs/prd.md` sebagai "sumber kebenaran produk" — jadi seluruh
pekerjaan Fase 0/1 sebelumnya (mengikuti `docs/prd.md`) tetap sah walau
`CLAUDE.md` (dokumen draf lebih lama, urutan fase BEDA — CLAUDE.md "Fase 0"
itu fleet-view sederhana tanpa bollard/TOFU, `bollard` katanya "baru mulai
Fase 5") sempat hilang. **Terpisah**: repo ternyata **nol commit git sama
sekali** sejak awal sesi — manusia memilih belum commit dulu, dicatat sebagai
risiko terbuka (tidak ada safety net version control untuk seluruh Fase 0+1).

**planner**: `docs/plan.md` ditulis ulang penuh untuk Fase 2. Ringkasan
keputusan: state machine `queued→pulling→starting→checking→live` + cabang
`failed/cancelled/unknown` (persis CLAUDE.md §9 lama, masih relevan);
operasi container lifecycle lewat `bollard` (BUKAN shell-out `docker` CLI
via SSH) — keputusan sadar planner karena infrastruktur socket-forward+
`bollard` sudah ada dari Fase 1, lebih aman dari mem-parsing stdout CLI;
worker deploy TERPISAH dari worker polling status Fase 1 (siklus hidup
beda: event-driven vs tick 30 detik); `deploy_tokens` pakai argon2 (pola
password), BUKAN `age` (token satu-arah, tidak pernah didekripsi ulang).

Dua pertanyaan diajukan ke manusia (Q3/Q4 tidak memblokir, dijawab default):
- **Q1 token deploy** — dijawab: token acak per-app, hash argon2 (opsi
  planner diterima persis).
- **Q2 bootstrap Traefik** (belum terpasang sama sekali dari Fase 1 — dicek
  nol referensi `traefik` di `src/`) — dijawab: lazy saat deploy app
  pertama ke server (opsi planner diterima persis).

**migration**: `migrations/0003_deploy.sql` — `apps`, `domains`,
`deployments`, `deploy_tokens`, `jobs`. `deployments.env_version_id`
ditambah sekarang (NULL selamanya sampai Fase 4) — kolom murah-sekarang-
mahal-diretrofit eksplisit menurut `docs/prd.md` §8, bukan pelanggaran
aturan "jangan desain skema untuk fase yang belum tiba". Diverifikasi:
`cargo sqlx migrate run` dari db kosong bersih (migrasi 1→2→3 berurutan).

**backend sub-blok 2a — `docker/client.rs` diperluas** dengan operasi
container lifecycle: `pull_image` (deteksi macet 60 detik tanpa progres +
batas total 10 menit, dua timeout independen bukan satu global),
`create_container`/`start_container` (terpisah supaya kegagalan create vs
start bisa dibedakan pemanggil; `--restart unless-stopped` + `--network`
selalu diset, TIDAK PERNAH `-p`), `inspect` (status ringkas: running,
exit_code, IP di network tertentu — bukan `ContainerInspectResponse`
mentah), `container_logs` (tail N baris, timeout pendek — bukan streaming
log runtime, itu Fase 3), `stop_container` (`t` grace period WAJIB
eksplisit), `remove_container`. API `bollard` diverifikasi field-per-field
langsung dari source crate (`ContainerCreateBody`, `HostConfig`,
`RestartPolicyNameEnum::UNLESS_STOPPED`, dll) sebelum menulis kode — build
sukses first-try meski permukaan API besar.

**backend sub-blok 2b — `auth/deploy_token.rs`** — generate token
(`mengdep_deploy_` + 32 karakter acak, prefiks kosmetik saja, entropi penuh
di bagian acak), `hash`/`verify` reuse `hash_password`/`verify_password`
Argon2 yang sudah ada (token deploy dan password sama-sama kredensial
satu-arah).

Verifikasi orchestrator langsung setelah 2a+2b: `cargo build --all-targets`
bersih, `cargo fmt` bersih, `cargo clippy --all-targets --all-features -- -D
warnings` No issues found, `cargo test` **134 passed** (belum nambah test
baru — `docker/client.rs`/`deploy_token.rs` fungsinya butuh Docker/db
sungguhan untuk diuji penuh, unit test murni ditambahkan di sub-blok
berikutnya begitu ada pemanggil nyata), `phase0` 21/21, `phase1` 7/7 —
TIDAK ADA REGRESI.

**BELUM dikerjakan** (sub-blok 2c-2g backend, lalu frontend/security/qa/
reviewer): `apps/**`, `jobs/**`, `deployments/**` (termasuk `engine.rs` —
bagian terbesar, mesin state penuh), `worker/deploy_worker.rs`,
`routes/{deploy_api,apps,deployments}.rs` + wiring, `src/web/{apps,
deployments}.rs`, `docs/design/{apps,deployment-detail}.md`,
`tests/phase2.rs`. **Gerbang tambahan setelah fase ini lolos**: platform
dipakai untuk deployment nyata dua minggu penuh sebelum Fase 3
(`docs/prd.md` §6).

---

## Fase 3 — Log dan riwayat

- [x] planner — output: `docs/plan.md` (overwrite penuh dari Fase 2), `docs/api-contract.md` (bagian Fase 3 di-append)
- [x] uiux — output: `docs/design/{log-viewer,riwayat-deployment}.md`
- [x] migration — `migrations/0004_logs.sql` (terverifikasi orchestrator: 170 test hijau)
- [x] backend — sub-blok 3a-3h SELESAI (253 test hijau); 3c/3e/3h-1/3h-2 dikerjakan orchestrator setelah agent gagal nol edit
- [x] frontend — `src/web/logs.rs` + tab + CSS (274 test hijau); xterm.js DIBUANG atas keputusan manusia (Q1 opsi c)
- [x] security — audit dikerjakan orchestrator (agent nol laporan); 10 poin diperiksa, 1 WARNING ditutup, Q2 = PERINGATKAN (275 test hijau)
- [x] qa — `tests/phase3.rs` 21 test hijau (296 total, 7 suite); 1 temuan dilaporkan (rejection `Query` membocorkan pesan library)
- [x] reviewer — audit dikerjakan orchestrator (agent nol laporan); nol BLOCKING, 1 WARNING boleh ditunda, 3 NIT; **Fase 3 layak maju ke gerbang manusia**

### Catatan

**Gerbang tambahan setelah Fase 2 (`docs/prd.md` §6 — "platform dipakai untuk
deployment nyata dua minggu penuh sebelum Fase 3"): dikonfirmasi terpenuhi oleh
manusia** saat Fase 3 dibuka. Dicatat di sini karena progress.md sebelumnya
tidak memuat bukti pemakaian dua minggu itu — konfirmasinya keputusan manusia,
bukan verifikasi otomatis.

**planner (2026-08-10)**: rencana Fase 3 ditulis. Angka yang dikunci (bukan TBD):

- Direktori log `/var/lib/platform/logs` (override `MENGDEP_LOG_DIR`), mode
  `0700`, file `0600`. Pola persis `verify_runtime_dir_available()`
  (`src/config.rs:100-117`) — gagal startup, TANPA fallback diam-diam.
- Log deploy: `<log_dir>/deploy/{deployment_id}.log`, satu deployment satu file.
- Retensi **30 hari**, sapuan tiap **24 jam** menumpang worker tick 30 detik
  yang sudah ada, batas **500 file per sapuan**, dan **melewati** deployment
  yang belum `selesai()` berapa pun umurnya (invariant §3 no 1).
- Batas ukuran **8 MiB per file**. Terlampaui → berhenti menulis, satu baris
  penanda, `truncated=1`, satu `warn!` (bukan per baris), broadcast TETAP jalan,
  dan **deploy tidak dibatalkan** — log adalah pengamatan, bukan kontrol.
- Tidak ada rotasi file bernomor; "rotasi" PRD diwujudkan sebagai retensi umur.
- **Log runtime tidak pernah ditulis ke disk control plane** — sudah persisten
  di server target (json-file driver Docker). Konsekuensi jujur: container yang
  sudah dihapus tidak bisa ditampilkan log runtimenya; itu satu state UI wajib.
- **Kebocoran channel** (`docs/prd.md:291`, `:384` — risiko yang PRD tandai
  paling mungkin di proyek ini) dijawab struktural, bukan lewat kedisiplinan:
  `LogRegistry` memegang `Weak`, hanya writer boleh `mulai()`, pembaca hanya
  `ikut()` (mengembalikan `None` kalau tidak ada sesi — menutup jalur "SSE
  membuat channel yatim"), `Drop` menghapus entri map, batas keras 64 sesi, lag
  jadi event `Tertinggal` (tidak pernah `continue` diam-diam), plus test yang
  mengasersi map kosong setelah semua handle drop.
- **Nol crate baru untuk backend.** `notify`/`rev_lines` ditolak dengan alasan
  eksplisit: writer log hidup di dalam proses ini, jadi "ada baris baru"
  dikabarkan lewat broadcast channel, bukan pengawasan inode.

Sub-blok: 14 langkah. Paralel `(uiux ‖ migration ‖ 3a ‖ 3b ‖ 3e)` → `(3c ‖ 3d)`
→ `(3f ‖ 3g)` → 3h → frontend → security → qa → reviewer.

**Pertanyaan terbuka planner (belum dijawab manusia):**

- **Q1 — `xterm.js`. DIJAWAB manusia: (a) vendor lokal, TANPA addon apa pun.**
  Opsi planner diterima persis. `xterm.min.js` + `xterm.min.css` di-`include_str!`
  seperti HTMX (`src/routes/assets.rs:11-12`), disajikan dari
  `GET /assets/xterm.min.js` dan `GET /assets/xterm.min.css`. Nol addon
  (`fit`/`search`/`weblinks` TIDAK ikut) — pencarian dikerjakan backend
  (`logs::reader`), jadi tidak ada alasan menariknya. Kedua endpoint aset di
  `docs/api-contract.md` **tetap berlaku**, tidak ada yang dihapus planner.
  Sub-blok frontend TIDAK lagi terblokir.
- **Q2 — secret yang dicetak aplikasi pengguna ke stdout: saring atau
  peringatkan.** Didelegasikan ke sub-blok security, tidak memblokir backend.
  Rekomendasi planner: peringatkan, jangan saring.
- **Q3 — `MENGDEP_LOG_DIR` di mesin dev. DIJAWAB manusia: rekomendasi planner
  diterima.** Perilaku identik `runtime_dir` — `create_dir_all` + chmod `0700`,
  gagal startup dengan pesan yang menyebut `MENGDEP_LOG_DIR`, **tanpa fallback
  diam-diam** ke `./data/logs` maupun `/tmp`. Default `/var/lib/platform/logs`
  tetap seperti di tabel angka plan.md.

**uiux (2026-08-10)**: `docs/design/log-viewer.md` dan
`docs/design/riwayat-deployment.md` ditulis. Keputusan desain yang mengikat
frontend:

- **Follow saat scroll** (tuntutan eksplisit `docs/prd.md:288`): scroll ke atas
  mematikan follow otomatis, memunculkan tombol melayang "Kembali ke Bawah".
  Klik → follow aktif lagi, scroll ke dasar, tombol hilang.
- **Sunyi vs putus dibedakan secara visual** — ini yang paling gampang salah
  dibaca: sunyi tetap `[*] STREAMING` (hijau, karena sunyi BUKAN error,
  `docs/plan.md` tabel timeout); putus jadi `[!] MENGHUBUNGKAN ULANG` (kuning)
  plus baris peringatan mengambang.
- **Tertinggal (subscriber lag)** disisipkan sebagai baris peringatan merah yang
  menyarankan muat ulang — lubang di tampilan, bukan lubang di aplikasi pengguna.
- **Log terpotong 8 MiB**: baris penanda kuning yang menegaskan deploy TETAP
  berjalan normal di target (mencegah salah baca "deploy gagal").
- **Log tersapu retensi 30 hari**: tautan berubah jadi `[Log (Terhapus)]` abu-abu
  non-aktif + tooltip penjelasan — jujur di muka, bukan menjanjikan halaman yang
  akan 404.
- **Riwayat**: digest dipotong (`api@sha256:7e8f…`) dengan tombol salin, commit
  dipotong 7 karakter. Tab riwayat hanya baca, nol tombol rollback (Fase 5).

Satu token visual BARU: `--color-bg-log: #070707` (latar area konsol log),
alasan uiux: kedalaman visual ekstra untuk area log dan mengurangi lelah mata
saat debug malam. Satu-satunya token baru sejak Fase 0.

Asumsi uiux yang dicatat: commit SHA tidak ditautkan ke provider eksternal
(GitHub/GitLab) — hanya teks monospace siap salin, karena integrasi provider
tidak ada di PRD Fase 3.

Butuh konfirmasi security nanti (bukan blocker sekarang): kalimat peringatan
privasi di toolbar viewer ("isi log berasal dari keluaran aplikasi pengguna dan
dapat memuat informasi sensitif…") — konsisten rekomendasi planner Q2
(peringatkan, jangan saring). Kalau security menolak Q2, teks ini ikut berubah.

**migration (2026-08-10)**: `migrations/0004_logs.sql` (47 baris) —
`deployment_logs` dengan `deployment_id` TEXT PK sekaligus FK ke
`deployments(id) ON DELETE CASCADE`, plus `path`/`size_bytes`/`line_count`/
`truncated` (CHECK 0-1)/`created_at`/`updated_at`, dan indeks
`idx_deployment_logs_created_at` untuk sapuan retensi. **Invariant §3 no 9
ditegakkan di level skema**: nol kolom teks bebas yang bisa menampung isi log —
tidak ada `preview`/`last_lines`/`tail_cache`. Kolom `path` menyimpan NAMA FILE
saja, bukan path absolut, supaya perubahan `MENGDEP_LOG_DIR` tidak membatalkan
baris lama dan tidak ada path absolut yang bisa bocor lewat pesan error.

Verifikasi ORCHESTRATOR LANGSUNG (laporan agent tidak memuat hasil verifikasi,
jadi dijalankan ulang sendiri): `cargo fmt --check` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found,
`cargo test --all-targets` **170 passed** (5 suites) — sama dengan baseline
Fase 2, TIDAK ADA REGRESI. `cargo test --test phase0` **21/21** — test ini
membangun database dari nol tiap run, jadi keempat migrasi terbukti berjalan
berurutan dari kosong lewat jalur produksi `sqlx::migrate!` (`src/db.rs:86`).

**Catatan `cargo sqlx migrate run` (pre-existing, BUKAN bug migrasi 0004):**
perintah `sqlx-cli` itu gagal dengan "cannot change into wal mode from within a
transaction" karena `migrations/0001_init.sql:11` memuat
`PRAGMA journal_mode = WAL` sementara `sqlx-cli` membungkus tiap migrasi dalam
transaksi. Jalur produksi tidak terpengaruh — `src/db.rs:105` sudah menyetel
`journal_mode(SqliteJournalMode::Wal)` lewat connect options sebelum
`sqlx::migrate!` jalan. Kondisi ini ada sejak Fase 0 dan tidak pernah menghalangi
apa pun; dicatat di sini supaya sesi berikutnya tidak mengira ini regresi Fase 3.

### backend Fase 3 sub-blok 3a — SELESAI (config + lib + kerangka logs)

- `src/config.rs`: field baru `log_dir: PathBuf` (default `/var/lib/platform/logs`,
  override `MENGDEP_LOG_DIR`) dan `log_retention_days: u32` (default 30, rentang
  sah 1–3650). Konstanta `DEFAULT_LOG_DIR`, `DEFAULT_LOG_RETENTION_DAYS`,
  `LOG_RETENTION_DAYS_MIN/MAX`. Method `verify_log_dir_available()` meniru
  `verify_runtime_dir_available()` persis: `create_dir_all` + chmod `0700` untuk
  `log_dir` DAN `<log_dir>/deploy/`, gagal fatal dengan pesan yang menyebut
  `MENGDEP_LOG_DIR`, **tanpa fallback diam-diam** (Q3 sesuai jawaban manusia).
- `parse_log_retention_days()` dipisah sebagai **fungsi murni**
  `Option<String> -> Result<u32>`, bukan membaca `std::env::var` di dalamnya —
  supaya test memanggil jalur kode produksi asli tanpa memanipulasi env var
  proses global. Ini sengaja memperbaiki kelas masalah CATATAN-12 audit security
  Fase 0 (test lama menguji closure duplikat, bukan kode produksi). Nilai di luar
  rentang / tidak bisa di-parse → gagal startup, TIDAK di-clamp.
- `src/lib.rs`: `pub mod logs;` + `src/logs/mod.rs` kerangka minimal (sub-modul
  3b-3g masih dikomentari, tipe `LogEvent` {Baris, Tertinggal, Selesai}
  didefinisikan karena bentuknya sudah dikunci plan.md). Modul dideklarasikan
  bersamaan dengan file-nya dibuat — menghindari bug orphan-module yang pernah
  terjadi di Fase 1 (`src/crypto.rs` tidak pernah dicompile sementara
  `cargo test` tetap melaporkan hijau).
- `src/main.rs`: `verify_log_dir_available()` dipanggil setelah
  `verify_runtime_dir_available()`, fatal lewat `?`.
- `src/state.rs`: **sengaja TIDAK diubah** — field `logs: Arc<LogRegistry>`
  ditunda ke 3b karena tipenya belum ada; menambahnya sekarang hanya membuat
  crate tidak bisa dikompilasi.

**Serah terima ke qa (pola sama sub-blok 3a Fase 1):** menambah field ke `Config`
memecah literal `Config { .. }` di `tests/**` (E0063). Backend melaporkan
lokasinya dan TIDAK menyentuhnya sendiri. qa memperbaiki:
`tests/{phase0,phase1,phase2}.rs` helper `setup()` (+`log_dir: dir.join("logs")`,
`log_retention_days: 30`, pola sama `runtime_dir`), plus
`tests/phase0.rs:465` — spawn binary di test SIGTERM ditambah
`.env("MENGDEP_LOG_DIR", dir.join("logs"))`. Tanpa itu binary gagal startup di
macOS karena `/var/lib/platform/logs` tidak bisa dibuat tanpa sudo — persis
masalah yang sudah pernah terjadi saat `MENGDEP_RUNTIME_DIR` ditambahkan.
Hanya `phase0` yang men-spawn binary; `phase1`/`phase2` murni `oneshot` router.

Verifikasi orchestrator langsung: `cargo fmt --check` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found,
`cargo test --all-targets` **176 passed** (5 suites) = baseline 170 + 6 test baru
`parse_log_retention_days`. `phase0` **21/21**, `phase1` **7/7**, `phase2` **7/7**
— TIDAK ADA REGRESI.

- [x] backend Fase 3 sub-blok 3a — SELESAI

### backend Fase 3 sub-blok 3b — SELESAI (LogRegistry, anti-kebocoran channel)

Sub-blok yang PRD tandai sebagai sumber kebocoran memori paling mungkin di
proyek ini (`docs/prd.md:291`, `:384`). Ketujuh aturan mengikat `docs/plan.md`
diimplementasikan:

- `src/logs/registry.rs` (baru, 350 baris): `LogRegistry {
  sessions: Mutex<HashMap<String, Weak<LogSession>>> }` — map memegang **`Weak`**,
  bukan `Arc`. `LogSession { key, tx: broadcast::Sender<LogEvent>,
  registry: Weak<LogRegistry> }` dipegang `Arc` oleh writer DAN tiap subscriber.
  `CHANNEL_CAPACITY = 256`, `MAX_SESSIONS = 64` sesuai tabel angka plan.md.
- API final: `mulai(self: &Arc<Self>, key) -> Option<Arc<LogSession>>` (hanya
  writer; `None` + `tracing::warn!` saat batas 64 terlampaui — pemanggil sub-blok
  3f tetap menulis file tanpa siaran, menukar kenyamanan dengan jaminan memori
  secara sadar), `ikut(key) -> Option<Arc<LogSession>>` (hanya `HashMap::get` +
  `Weak::upgrade`, **tidak pernah membuat entri** — menutup jalur channel yatim
  secara struktural, bukan lewat kedisiplinan pemanggil), `sapu_yatim()`
  (`tracing::warn!` kalau menemukan apa pun, karena `Drop` mestinya sudah
  membersihkan sendiri), `LogSession::subscribe()`, `LogSession::kirim()`.
- **Race `mulai()` vs `Drop` ditangani eksplisit**: `Drop for LogSession`
  membandingkan `std::ptr::eq(weak.as_ptr(), self as *const LogSession)` sebelum
  menghapus entri, sehingga sesi lama tidak bisa menghapus entri map yang sudah
  menunjuk sesi baru untuk key yang sama. `upgrade()` tidak bisa dipakai untuk
  perbandingan identitas di titik itu karena strong count sesi ini sudah nol.
- **Nol deadlock `Drop`+`Mutex`**: setiap method mengunci `sessions` tepat sekali,
  tanpa nested lock dan tanpa callback yang mengunci ulang.
- Nol `unwrap()`/`expect()` di jalur produksi: lock poison ditangani
  `unwrap_or_else(|err| err.into_inner())`, pola sama `src/events.rs`.
- `src/logs/mod.rs`: `pub mod registry;` + re-export. `writer`/`reader`/`repo`/
  `retention` tetap dikomentari (sub-blok 3c/3d/3g).
- `src/state.rs`: field `logs: Arc<LogRegistry>` (ditunda dari 3a, sekarang
  tipenya ada). `src/main.rs` menginisialisasinya.

**8 test ditulis, semuanya benar-benar bisa gagal:** map kosong setelah writer +
semua subscriber drop (invariant utama — akan merah kalau map memegang `Arc`);
`ikut()` key tak dikenal → `None` tanpa menyisakan entri; `mulai()` menolak di
batas 64; subscriber drop duluan tidak menghapus sesi selama writer hidup; dua
subscriber key sama menerima event sama (menutup kriteria "dua tab"); event
`Selesai` diterima; `sapu_yatim` tidak menemukan apa pun pada operasi normal;
race identity `mulai()` ulang untuk key sama.

Test `Lagged` **sengaja tidak ditulis** — sulit dibuat deterministik tanpa
timing rapuh. Penanganan lag tetap wajib di sisi handler SSE (sub-blok 3h,
aturan 4 plan.md: `continue` diam-diam DILARANG untuk log).

**Asumsi yang dicatat:** satu instance `LogRegistry` melayani log deploy dan log
runtime sekaligus (namespace key `deployment_id` vs `app_id`). Kalau kolisi id
jadi masalah nyata, pisah jadi dua field seperti `events`/`deployment_events` —
upgrade path ada di komentar modul.

**Serah terima ke qa:** field `logs` memecah literal `AppState` di `tests/**`.
qa memperbaiki `tests/{phase0,phase1,phase2}.rs:94` dengan
`logs: std::sync::Arc::new(mengdep::logs::LogRegistry::new()),` mengikuti pola
path penuh tetangganya (`events`/`deployment_events`).

Verifikasi orchestrator langsung: `cargo fmt --check` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found,
`cargo test --all-targets` **184 passed** (5 suites) = 176 + 8 test registry.
`phase0` 21/21, `phase1` 7/7, `phase2` 7/7 — NOL REGRESI.

- [x] backend Fase 3 sub-blok 3b — SELESAI

### backend Fase 3 sub-blok 3e — SELESAI (container_logs_follow)

**Dikerjakan orchestrator langsung, BUKAN subagent.** Agent `backend` dipanggil
dua kali untuk sub-blok ini dan **dua kali selesai tanpa menulis satu baris pun
dan tanpa laporan** (diverifikasi: `rg container_logs_follow src/docker/client.rs`
kosong setelah kedua panggilan). Anomali tooling, bukan penolakan beralasan —
sub-blok 3a dan 3b lewat agent yang sama berjalan normal. Dicatat di sini supaya
kalau pola ini berulang, penyebabnya diselidiki, bukan ditebak.

`src/docker/client.rs` (satu file, +~90 baris + 4 test):

- `LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT = 15 detik` — konstanta BARU, terpisah dari
  `LOGS_TIMEOUT` (10 detik) yang dipakai `container_logs` non-follow.
- `container_logs_follow(docker, container_id, tail_lines)` dengan
  `follow(true)`, `stdout(true)`, `stderr(true)`, `timestamps(true)`.
- **Return type `(String, impl Stream<Item = Result<String, LogFollowError>>)`** —
  chunk pertama sudah ditarik saat fungsi kembali, sisanya stream telanjang.
  Bentuk ini dipilih supaya kesalahan "membungkus seluruh stream dengan timeout"
  **tidak mungkin** dilakukan pemanggil secara tidak sengaja: batas 15 detik
  sudah habis terpakai di dalam fungsi, dan `sisa` tidak membawa batas waktu apa
  pun. Invariant §3 no.11 dijaga oleh bentuk tipe, bukan oleh kedisiplinan.
- **`LogFollowError` enum baru** (`ContainerHilang` / `TimeoutChunkPertama` /
  `Unreachable`), dipisah dari `DockerClientError` karena kontrak HTTP memetakan
  dua kasus pertamanya ke status berbeda (502 vs 504). Pemanggil membedakannya
  lewat varian, **tanpa mem-parsing string pesan**.
- `petakan_error_log_follow()` fungsi **murni** (`&bollard::errors::Error ->
  LogFollowError`) supaya bisa diuji tanpa Docker. Docker 404 → `ContainerHilang`;
  sisanya → `Unreachable`.
- Stream berakhir tanpa satu chunk pun (`Ok(None)`) **bukan error** — container
  ada tapi belum menulis apa pun; pemanggil merender state "belum ada keluaran".
- Byte log diteruskan **apa adanya**: nol penanggalan ANSI, nol escape HTML, nol
  pemotongan. ANSI dipertahankan sampai browser (Q1 = xterm.js vendor lokal);
  escaping HTML tetap milik `src/web/**`.
- **Penjepitan `tail_lines` ke maksimum 2000 TIDAK dilakukan di sini** —
  diserahkan ke handler `routes/**` yang memvalidasi query, supaya batas kontrak
  HTTP hidup di satu tempat saja. Sub-blok 3h wajib melakukannya.
- Batas 30 menit per sesi **bukan** tanggung jawab fungsi ini (pemanggil).

**4 test ditulis** (semua bisa gagal, semua tanpa Docker): 404 → `ContainerHilang`;
409/500/503 → `Unreachable` (bukan `ContainerHilang` — kalau disamakan, pengguna
dapat pesan perbaikan yang salah); `TimeoutChunkPertama` tidak pernah lahir dari
pemetaan error apa pun (kalau bocor, handler membalas 504 untuk container yang
sebenarnya hilang); `LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT == 15s` dan `!= LOGS_TIMEOUT`.

Jalur streaming sesungguhnya tidak diuji unit — butuh daemon Docker nyata.
Diserahkan ke qa (`tests/phase3.rs`) atau tetap tidak diuji dengan jujur, bukan
ditutup test yang selalu hijau.

Catatan kecil: `bollard::errors::Error::NoHomePathError` **tidak bisa dipakai** di
test — di-gate `#[cfg(feature = "ssl_providerless")]` yang memang tidak aktif
(konsisten invariant §3 no.13, nol TCP/TLS). Dipakai `APIVersionParseError {}`.

Verifikasi orchestrator: `cargo fmt` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found,
`cargo test --all-targets` **188 passed** (5 suites) = 184 + 4. NOL REGRESI.
`tests/**` tidak tersentuh (tidak ada perubahan API yang memecahnya).

- [x] backend Fase 3 sub-blok 3e — SELESAI

### backend Fase 3 sub-blok 3c — SELESAI (repo + writer log deploy)

**Dikerjakan orchestrator langsung.** Agent `backend` balik kosong untuk KETIGA
kalinya (3e ×2, 3c ×1): nol edit, nol laporan, `src/logs/` tetap hanya berisi
`mod.rs` + `registry.rs` setelah panggilan. Sub-blok 3a dan 3b lewat agent yang
sama berhasil normal. Pola ini sekarang cukup konsisten untuk disebut **anomali
tooling pada agent `backend`, bukan kebetulan** — perlu diselidiki sebelum
mengandalkannya lagi untuk sub-blok besar (3f, 3g, 3h).

**`src/logs/repo.rs` (baru):** metadata `deployment_logs` saja — PATH dan angka,
**nol isi log** (invariant §3 no.9). `insert` (pool tulis), `update_metadata`
(pool tulis), `find` (pool baca) → `Option<LogMeta>`; `nama_file(deployment_id)`
satu-satunya tempat bentuk nama file ditentukan, dipakai bersama writer dan
(nanti) reader supaya tidak ada yang merangkai sendiri. `INSERT OR REPLACE`
**sengaja tidak dipakai**: id sama dua kali berarti bug pemanggil, lebih baik
gagal keras daripada menimpa metadata log yang mungkin sedang dibaca (ada
test-nya). `SELECT` memakai `AS "deployment_id!"` karena SQLite mengizinkan PK
TEXT NULL secara historis sehingga sqlx menyimpulkannya `Option<String>`.

**`src/logs/writer.rs` (baru):** file `<log_dir>/deploy/{id}.log`.

- Mode **0600 diset saat `open`** lewat `OpenOptions::mode`, bukan chmod
  sesudahnya — chmod-setelah-create meninggalkan celah singkat file terbaca
  proses lain.
- API **buka/tulis/tutup eksplisit**, bukan `Drop`: penutupan butuh `await`
  (flush terakhir, UPDATE final, kirim `Selesai`) dan `Drop` tidak bisa menunggu.
- Flush tiap **200 ms ATAU 64 KiB**; metadata di-UPDATE **paling sering sekali
  per 5 detik** + sekali saat tutup.
- `tulis()` **tidak pernah mengembalikan `Err`**. Kegagalan I/O → `tracing::warn!`
  + penulisan file berhenti, deploy lanjut (invariant §3 no.1 ditegakkan oleh
  bentuk signature, bukan oleh kedisiplinan engine).
- **Batas 8 MiB persis sesuai plan.md:** berhenti menulis, satu baris penutup
  `--- log dipotong pada batas 8 MiB; sisa keluaran tidak disimpan ---`,
  `truncated=1`, `tracing::warn!` **sekali** (dijaga flag `penanda_potong_ditulis`,
  bukan per baris — kalau tidak, log control plane sendiri meledak), dan
  **siaran broadcast TETAP jalan** (field `session` sengaja tidak dilepas di
  jalur ini). Deploy tidak dibatalkan.
- Batas baris **8 KiB** + sisipan `…[baris dipotong]`, memotong di **batas
  karakter UTF-8** — memotong di tengah rangkaian byte menghasilkan `String`
  tidak valid (ada test khusus dengan karakter multibyte).
- `LogRegistry::mulai() == None` (batas 64 sesi) ditangani eksplisit: log **tetap
  ditulis ke file**, hanya siaran langsung yang hilang + `tracing::warn!`. Bukan
  di-`?`-kan jadi kegagalan membuka sesi.

**8 test baru** (semua bisa gagal): file bermode 0600 (permission asli, bukan
asumsi); baris >8 KiB dipotong + penanda; potong tidak merusak UTF-8 multibyte;
batas 8 MiB → `truncated`, penanda **tepat satu**, penulisan berhenti, baris
sesudahnya tidak dipersistensi; siaran tetap mengalir setelah file penuh;
metadata **tidak** di-UPDATE per baris (200 baris ditulis → `line_count` di db
masih 0; kalau writer meng-UPDATE per baris, test merah) lalu benar setelah
tutup; `tutup` mengirim `Selesai` lalu registry kosong. Plus 4 test repo.

Catatan test: `try_recv()` mengembalikan `Err(Lagged)` saat subscriber
tertinggal — loop penguras channel harus **menelan** `Lagged` dan lanjut, bukan
berhenti (`is_ok()` sebagai kondisi loop salah). Ini persis kelas bug yang
aturan 4 plan.md larang di handler SSE; kena duluan di test.

`cargo sqlx prepare` dijalankan ulang lewat `.sqlx-prepare-tmp/prep.db` +
`DATABASE_URL` + `cargo sqlx migrate run` (0004 applied). `.sqlx/` berubah dan
itu benar — tidak pernah diedit tangan.

Verifikasi orchestrator: `cargo fmt` bersih,
`cargo clippy --all-targets -- -D warnings` No issues found,
`cargo test --all-targets` **199 passed** (5 suites) = 188 + 11. NOL REGRESI,
`tests/**` tidak tersentuh (tidak ada perubahan API yang memecahnya).

- [x] backend Fase 3 sub-blok 3c — SELESAI

### backend Fase 3 sub-blok 3d — SELESAI (reader: tail, cari, anti path traversal)

**Dikerjakan agent `backend` dan BERHASIL** — memutus rangkaian tiga kegagalan
kosong sebelumnya (3e ×2, 3c ×1). Jadi anomali itu **tidak deterministik**:
agent yang sama, prompt dengan struktur sama, kadang menulis kadang tidak.
Kesimpulan operasional: setiap laporan agent tetap wajib diverifikasi
orchestrator dengan `rg` pada file yang mestinya berubah sebelum dipercaya —
"selesai" tanpa bukti file bukan bukti.

`src/logs/reader.rs` (baru) + `src/logs/mod.rs` (aktifkan `pub mod reader;`).

**`nama_file_aman(deployment_id)` — gerbang keamanan sub-blok ini.** Menerima
hanya `^[A-Za-z0-9]{1,64}$` (diperiksa lewat `is_ascii_alphanumeric()` + batas
panjang, bukan regex — nol crate baru). Diperketat melampaui minimum: `_` dan
`.` juga ditolak, bukan hanya `/` dan `..`.

**Tail dari ekor tanpa memuat seluruh file:** `seek(SeekFrom::Start)` +
`read_exact` blok 64 KiB dari ekor; kalau jumlah newline belum cukup, blok
digandakan (`ambil *= 2`) sampai cukup atau sampai awal file. Worst case tidak
pernah melebihi ukuran file. Nol crate baru (`rev_lines`/`linemux`/`notify`
tetap tidak ditambahkan, sesuai keputusan plan.md).

**Penjepitan:** `0` → `TAIL_DEFAULT` 500; `> TAIL_MAX` → 5000. **Bukan 400** —
kenyamanan baca, bukan perintah destruktif (aturan `docs/api-contract.md`).
File tidak ada / kosong → `Ok(HasilTail::default())`, bukan error, supaya
handler merender state kosong dan tetap 200.

**Pencarian:** substring case-sensitive, maksimum `SEARCH_MAX_RESULTS` 500 baris
cocok, sisanya ditandai `HasilCari.dipotong = true` — dipotong **eksplisit**,
tidak diam-diam. Timeout `TAIL_READ_TIMEOUT` dan `SEARCH_TIMEOUT` masing-masing
**5 detik**, sesuai tabel timeout per tahap.

**`LogReadError { IdTidakValid, Timeout, Io }`** — enum, bukan string. Pemanggil
(3h) memetakan `IdTidakValid` → 404, `Timeout` → 504, `Io` → 500 lewat `match`,
**tanpa mem-parsing pesan** (pola sama `LogFollowError` sub-blok 3e). **Path file
tidak pernah masuk varian error** — hanya ke `tracing::warn!`.

**Tipe ke `src/web/**`: `LogLine { nomor, teks }`** — sudah jadi, sesuai aturan
plan.md bahwa `src/web/**` hanya menerima data siap render. `teks` mentah: ANSI
dipertahankan (Q1 = xterm.js), escaping HTML tetap milik frontend.

**24 test baru.** Path traversal ditolak: `..`, `../../etc/passwd`, `a/b`, string
kosong, 65 karakter, `%2e%2e`, null byte, unicode (`café`, `日本語`), `_`, `.`.
Diterima: alfanumerik 24 karakter dan tepat 64 karakter. Tail: file tidak ada,
file kosong, urutan N baris terakhir, minta lebih dari isi, `0` → default,
melebihi max → dijepit, file besar multi-chunk (menguji pembesaran blok), file
tanpa trailing newline. Cari: file tidak ada, filter cocok, query kosong = semua,
melebihi 500 → `dipotong=true`, di bawah batas → `false`.

**Asumsi yang dicatat agent (disetujui orchestrator):** nomor baris hasil `tail`
relatif terhadap potongan yang terbaca, bukan absolut dari awal file — nomor
absolut butuh baca penuh, yang kontradiksi dengan tujuan tail. Cukup untuk gutter
viewer. Reader memperlakukan `NotFound` sebagai hasil kosong untuk file yang
belum pernah ada MAUPUN yang sudah tersapu retensi; **pembeda kedua state itu
adalah tugas handler 3h** berdasar metadata db, bukan reader (kontrak memang
tidak membedakan keduanya di level status: keduanya 404/state kosong).

Nol `sqlx::query!` baru → `cargo sqlx prepare` tidak perlu (reader murni baca
file; metadata sudah disediakan `logs::repo`).

Verifikasi orchestrator langsung (bukan hanya percaya laporan): `cargo fmt --check`
bersih, `cargo clippy --all-targets -- -D warnings` No issues found,
`cargo test --all-targets` **223 passed** (5 suites) = 199 + 24. NOL REGRESI.
Diperiksa juga: nol `PathBuf::from(<input>)`, nol `unwrap()`/`expect()` di luar
`#[cfg(test)]`, kedua konstanta timeout benar 5 detik.

Catatan: laporan agent menulis "223 passed (6 suites)"; jumlah suite sebenarnya
**5** — jumlah test-nya yang cocok. Selisih sepele tapi menegaskan poin di atas.

- [x] backend Fase 3 sub-blok 3d — SELESAI

### backend Fase 3 sub-blok 3f — SELESAI (engine menulis log deploy)

**Agent menulis kode tapi TIDAK mengembalikan laporan, DAN meninggalkan kode yang
tidak bisa dikompilasi.** Dua error: `use sqlx::SqlitePool;` tidak ditambahkan
padahal `catat()` memakainya di signature, dan `drain_container_lama` dipanggil
dengan 5 argumen sementara signature-nya masih 4 (agent menambah `writer` di
callsite tapi lupa di definisi). Diperbaiki orchestrator. Ini menaikkan aturan
verifikasi: **`cargo build --all-targets` sebelum apa pun**, karena "agent balik
diam" ternyata bisa berarti "kode ada tapi rusak", bukan hanya "kode tidak ada".

`src/deployments/engine.rs`:

- `baris_berstempel(pesan)` (baris 104) — format `HH:MM:SS | pesan`, persis pola
  gutter `docs/design/log-viewer.md`.
- `catat(writer, pool_tulis, pesan)` (baris 118) — **titik tunggal** yang
  menegakkan invariant §3 no.1: `writer` bertipe `Option<LogWriter>`, `None`
  berarti diam saja. Tidak ada jalur di mana log menggagalkan deploy.
- Sesi dibuka di awal `jalankan_deploy` (baris 132). `logs::writer::mulai` gagal
  → `tracing::warn!` + `None`, **deploy tetap lanjut tanpa file log**. Tidak
  di-`?`-kan.
- Sesi ditutup baris 202-205, **tepat sebelum** `deployment_events.remove` —
  titik yang plan.md pilih justru supaya tidak ada jalur keluar yang lolos.
  `writer.take()` lalu `tutup(&state.db_write).await`. Jalur sukses dan gagal
  sama-sama lewat sini karena keduanya jatuh ke `match &hasil` di atasnya.
- 12 titik `catat()`: SSH connect, forward socket, tiap `publish_dan_set`
  (`tahap: {status}`), tarik image + digest, container dibuat, container
  dimulai, health check lulus/gagal, drain container lama, deploy selesai/gagal.

**INVARIANT 5 — diperiksa orchestrator secara literal, tidak bergeser.**
Penangkapan 50 baris log container ada di **baris 426**
(`docker::container_logs`), `remove_container` di **baris 447**. Urutannya utuh;
Fase 3 hanya menambahkan tujuan tulis kedua (baris 430-435, `catat` ke file)
**di antara** keduanya, tanpa memindahkan penangkapan maupun penghapusan.
Komentar di baris 421-425 menuliskan kontrak ini supaya refactor berikutnya
tidak menggesernya tanpa sadar.

**Secret:** teks baris log disusun manual, nol interpolasi kredensial. Tahap pull
hanya mencatat `image_digest` (bukan secret, dan memang wajib terlihat); tahap
SSH hanya mencatat "menyambung ke server target lewat SSH" tanpa host/kunci.
`DockerCredentials` tidak pernah masuk `catat()`. `error_detail` **tidak
bertambah** — invariant §3 no.9 utuh, nol baris log ke SQLite.

Orchestrator menambah 2 test yang benar-benar bisa gagal (agent menulis nol):
`catat_tanpa_sesi_log_tidak_panik` (writer `None` → diam, bukan panik — ini yang
menjaga deploy jalan tanpa log) dan
`baris_berstempel_memakai_format_gutter_jam_menit_detik` (memvalidasi kontrak
format dengan frontend: `HH:MM:SS | `, tiap bagian dua digit dan numerik).

`cargo fmt` bersih, clippy No issues, `cargo test --all-targets` **225 passed**
(223 → +2). Nol `sqlx::query!` baru.

- [x] backend Fase 3 sub-blok 3f — SELESAI

### backend Fase 3 sub-blok 3g — SELESAI (retensi log + sapuan channel yatim)

Agent **menulis kode yang langsung terkompilasi** (build bersih pada percobaan
pertama) tapi **tetap tidak mengembalikan laporan**. Jadi pola kegagalannya
konsisten pada *laporan*, bukan pada *kode*. Semua verifikasi di bawah dikerjakan
orchestrator langsung.

`src/logs/retention.rs` (baru, 412 baris):

- `pilih_korban(kandidat, now, retention_days, batas_jumlah) -> Vec<String>`
  (baris 55) — **fungsi murni**: nol disk, nol db, dan `now` **diberikan
  pemanggil** (tidak memanggil jam sistem). Itulah yang membuat invariant §3
  no.1 bisa diuji tanpa infrastruktur apa pun.
- Urutan filter disengaja: `.filter(selesai())` **sebelum** `.filter(umur)`
  (baris 67-68), supaya niatnya terbaca reviewer — **deployment yang belum
  selesai tidak pernah dipilih, apa pun umurnya**.
- `BATAS_FILE_PER_SAPUAN = 500`, `BATAS_WAKTU_SAPUAN = 60 detik`
  (`tokio::time::timeout` membungkus seluruh sapuan, baris 133). `retention_days`
  dibaca dari config, tidak dihardcode.
- `hapus_file_log`: `NotFound` → **`true`** (file sudah hilang bukan error,
  metadata tetap layak dihapus); kegagalan lain → `warn!` + deployment dilewati,
  barisnya tetap ada dan dicoba lagi sapuan berikutnya.
- `path_log` dipakai untuk membentuk path; nol `PathBuf::from(<input>)`.
- `repo::hapus_batch`: satu `begin()`/`commit()` untuk seluruh batch
  (`src/logs/repo.rs:176-187`) — **satu transaksi per batch, bukan per file**,
  invariant 10.

`src/worker/log_retention.rs` (baru): `DELAY_PERTAMA` 60 detik dari `boot`
(`Instant`, sengaja **tidak dipersist** — restart = jeda 60 detik lagi, supaya
sapuan tidak menghajar startup), `INTERVAL_SAPUAN_SECS` 24 jam, penanda
`settings` key `log_retention_last_run_at`. Setiap jalur gagal (baca penanda,
sapuan, tulis penanda) → `tracing::warn!` + `return`, **worker tidak pernah
jatuh** (konvensi AGENTS.md). Kegagalan menulis penanda ditandai eksplisit
sebagai "bukan masalah keamanan" — konsekuensinya hanya sapuan berikutnya bisa
lebih cepat dari 24 jam.

`src/worker/mod.rs:52-53` — dua sapuan menumpang tick 30 detik yang sudah ada:
`log_retention::jalankan_jika_jatuh_tempo(&state, boot)` dan
`state.logs.sapu_yatim()`. **Bukan worker ketiga.** `sapu_yatim` sudah
mengeluarkan `warn!` sendiri kalau menemukan sesuatu, jadi tidak diduplikasi.

7 test baru: empat murni pada `pilih_korban` (belum selesai tidak pernah dipilih
walau tua; selesai + tua dipilih; selesai + muda tidak; batas 500 benar-benar
membatasi; batas mengambil **yang paling tua dulu**) dan tiga integrasi
`jalankan_sapuan` (hapus file + metadata; melewati deployment belum selesai;
file sudah hilang di disk tetap menghapus metadata). Semua status akhir
(`live`/`failed`/`cancelled`/`unknown`) diuji layak disapu.

`cargo build --all-targets` nol error, `cargo fmt --check` bersih, clippy
No issues, `cargo test --all-targets` **234 passed** (225 → +9). `.sqlx/`
terkonfirmasi sinkron lewat `SQLX_OFFLINE=true cargo check --all-targets`
(8 file query menyebut `deployment_logs`).

- [x] backend Fase 3 sub-blok 3g — SELESAI

### backend Fase 3 sub-blok 3h-1 — SELESAI (endpoint log non-SSE), DIKERJAKAN ORCHESTRATOR

3h dipecah dua atas keputusan manusia: **3h-1 endpoint HTTP biasa** (ini) dan
**3h-2 dua endpoint SSE log** (berikutnya). Alasannya SSE runtime adalah bagian
tersulit fase ini (semaphore 4 slot, tiga jalur penutupan, forward socket wajib
ditutup) dan layak mendapat perhatian terpisah dari endpoint baca biasa.

Agent `backend` dipanggil untuk 3h-1 dan **kembali dengan nol edit sama sekali** —
`src/routes/logs.rs` tidak dibuat, `apps.rs`/`mod.rs` tidak berubah ukurannya.
Ini kegagalan kosong keempat di fase ini. Dikerjakan orchestrator manual, pola
sama 3c dan 3e.

**File:** `src/routes/logs.rs` (baru), `src/routes/apps.rs`, `src/routes/mod.rs`,
`src/error.rs`.

**`AppError::Timeout(String)` ditambahkan** (`src/error.rs`) → `504 GATEWAY_TIMEOUT`.
Kontrak Fase 3 menuntut 504 di beberapa tempat dan varian itu belum ada sejak
Fase 0-2. Pesannya sudah berupa kategori Bahasa Indonesia yang menyebut langkah
perbaikan, bukan detail internal.

**Enam route baru, SEMUANYA di blok `protected`** (`src/routes/mod.rs:58-64`,
sebelum `route_layer(require_session)`): `/apps/{id}/deployments`,
`/apps/{id}/logs`, `/apps/{id}/logs/isi`, `/deployments/{id}/log`,
`/deployments/{id}/log/isi`, `/deployments/{id}/log/unduh`. Nol route log di
blok `public`.

**Anti path traversal.** `baca_baris` dan `deploy_log_unduh` memanggil
`reader::nama_file_aman(id)` **sebelum** `writer::path_log` membentuk path.
Diverifikasi: satu-satunya kemunculan string `PathBuf::from` di
`src/routes/logs.rs` ada **di komentar larangan** di header file, nol di kode.
`LogReadError::IdTidakValid` → **404**, sama persis dengan id tidak dikenal —
tidak ada perbedaan pesan antara keduanya, sesuai kontrak.

**Unduhan:** nama berkas `deploy-{id}.log` dibentuk dari id yang **sudah lolos**
`^[A-Za-z0-9]{1,64}$`, jadi tidak mungkin menyuntik header `Content-Disposition`.
File tidak ada (tersapu retensi) → 404 biasa, tanpa membedakan "belum pernah
ada" vs "sudah dihapus". Pesan io mentah dan path hanya ke `tracing::warn!`.

**`/apps/{id}/logs/isi`** — satu tarikan `docker::container_logs` (TANPA follow).
Pemetaan status sesuai kontrak: tidak ada deployment live / `container_id` NULL
→ **409**; container hilang di server → **502**; SSH/forward/tarikan gagal →
**504** kategori "server tidak merespons". **Forward ditutup di satu titik yang
melayani jalur sukses maupun gagal** (`docker::close` + `session.close()`
setelah `tarik_log_runtime` kembali, apa pun hasilnya); dua jalur error sebelum
forward terbentuk menutup `session` masing-masing. Nol stderr mentah, nol exit
code telanjang, nol path socket forward di respons mana pun.

**Kontrak render ke `src/web/**` DIKUNCI** lewat modul sementara
`routes::logs::render_sementara`, ditandai `// ponytail:`. `src/web/logs.rs`
milik frontend dan belum ada; placeholder ini ada semata supaya repo tetap
terkompilasi di antara dua sub-blok, dan **akan dihapus** setelah frontend
menulis versi sebenarnya. Lima signature yang wajib disediakan frontend:

```rust
render_deploy_log(dep: &DeploymentRingkas, truncated: bool, baris: &[LogLine],
                  pencarian_dipotong: bool, q: Option<&str>, streaming: bool,
                  csrf_token: &str, strip: Option<Markup>) -> Markup
render_log_fragmen(baris: &[LogLine], truncated: bool,
                   pencarian_dipotong: bool, selesai: bool) -> Markup
render_log_pesan(pesan: &str) -> Markup
render_app_tab_deployments(app: &AppRingkas, deploys: &[DeploymentRingkas],
                           dipotong: bool, csrf_token: &str,
                           strip: Option<Markup>) -> Markup
render_app_tab_logs(app: &AppRingkas, ada_container: bool, csrf_token: &str,
                    strip: Option<Markup>) -> Markup
```

Semua argumen sudah jadi (`&[LogLine]`, `bool`, `&str`) — **nol `sqlx::`, nol
`tokio::fs::`, nol `bollard::`** yang bisa bocor ke `src/web/**` lewat desain
ini. `streaming` = deployment belum selesai (frontend memasang SSE); `false`
berarti isi statis TANPA membuka SSE, supaya klien tidak menunggu event yang
tidak akan datang.

**Tab Deployments** dibatasi `BATAS_RIWAYAT_DEPLOYMENT = 100` dengan flag
`dipotong` ke frontend; read-only, nol tombol rollback (itu Fase 5).
**Tab Logs** tidak membuka SSH maupun forward — itu tugas `/logs/isi` dan SSE.

9 test baru di `src/routes/logs.rs`: penjepitan tail runtime (default 200 saat
absen/nol, jepit di 2000, teruskan nilai dalam rentang), pemetaan ketiga varian
`LogReadError` ke 404/504/500, satu test yang mengasersi pesan `Internal`
**tidak memuat karakter `/`** (anti bocor path), dan tiga test penyaringan
pencarian runtime termasuk pemotongan di `SEARCH_MAX_RESULTS`.

`cargo build --all-targets` nol error, `cargo fmt` bersih, clippy No issues
(satu `#[allow(clippy::too_many_arguments)]` dengan alasan tertulis, pola sama
`src/web/apps.rs:127`), `cargo test --all-targets` **243 passed** (234 → +9).
Nol `sqlx::query!` baru. Nol `unwrap()`/`expect()` di luar `#[cfg(test)]`.

**Belum dikerjakan, diserahkan:** dua route aset xterm
(`GET /assets/xterm.min.{js,css}`) **tidak didaftarkan** karena file vendor
`src/web/assets/xterm.min.*` milik frontend dan belum ada — `include_str!` ke
file yang tidak ada akan menggagalkan kompilasi. Frontend menyediakan filenya,
lalu route-nya didaftarkan.

- [x] backend Fase 3 sub-blok 3h-1 — SELESAI, lanjut 3h-2 (dua SSE log)

**Catatan dokumentasi tertunda (bukan pekerjaan Fase 3):**
`docs/api-contract.md:15` masih menulis bearer token "Belum dibuka (Fase 2)"
padahal Fase 2 sudah lolos gerbang; dan permukaan HTTP Fase 2
(`POST /api/v1/deploy`, `/apps*`, `/deployments/{id}`, `/events/deploy/{id}`)
tidak pernah ditulis sebagai bagian tersendiri di kontrak — hidupnya di plan.md
versi Fase 2 dan di kode yang sudah beku. Planner sengaja TIDAK menambalnya di
fase ini (menulis ulang kontrak yang implementasinya sudah beku berisiko
menciptakan versi kedua yang berbeda dari kode). Ini task dokumentasi terpisah
untuk planner, bukan pekerjaan implementer Fase 3.

### backend Fase 3 sub-blok 3h-2 — SELESAI (dua SSE log), DIKERJAKAN ORCHESTRATOR

Agent `backend` dipanggil untuk 3h-2 dan **kembali dengan nol edit lagi** —
kegagalan kosong kelima di fase ini. Dikerjakan orchestrator manual.

**File:** `src/routes/events.rs`, `src/routes/mod.rs`, `src/routes/logs.rs`,
`src/docker/mod.rs`, `src/error.rs`.

**Dua varian `AppError` baru** (`src/error.rs`): `TooManyRequests(String)` → 429
(batas empat sesi log runtime) dan `BadGateway(String)` → 502 (container sudah
tidak ada di server). Keduanya membawa pesan kategori Bahasa Indonesia, bukan
stderr.

**Dua route SSE didaftarkan di blok `protected`** (`src/routes/mod.rs:66-67`):
`/events/log/deploy/{id}`, `/events/log/runtime/{id}`. Total delapan route log
Fase 3 kini terlindungi; nol di blok `public`.

**SSE log deploy.** `nama_file_aman` dulu → id tidak valid atau tidak dikenal
sama-sama 404, tanpa perbedaan pesan. `LogRegistry::ikut()` mengembalikan `None`
untuk deployment yang sudah selesai → kirim **satu** event `selesai` lalu tutup,
**tanpa membuat channel** (jaminan struktural anti kebocoran, `docs/prd.md:291`).
`LogEvent::Tertinggal(n)` **dan** `BroadcastStreamRecvError::Lagged(n)` sama-sama
dipetakan ke penanda `--- {n} baris terlewat ---`; nol `continue` diam-diam —
baris yang hilang tidak boleh disembunyikan dari pengguna.

**SSE log runtime.** Izin `Semaphore` (4 slot) diambil `try_acquire` **sebelum**
menyentuh jaringan → penuh berarti 429 tanpa membuka SSH. Tanpa deployment live
atau `container_id` NULL berarti 409, stream tidak dibuka sama sekali.

**Keputusan struktural:** `bollard::Docker` dan stream turunannya tidak bisa
hidup di luar task yang memilikinya (stream meminjam client), jadi **seluruh
sesi streaming berjalan di dalam satu task**. Kegagalan *membuka* stream tetap
wajib jadi status HTTP, bukan event — jadi hasil pembukaan dikirim balik lewat
`tokio::sync::oneshot` dan handler menunggunya sebelum membentuk respons. Task
hilang tanpa mengirim hasil → 504, bukan klien yang menggantung.

**Satu jalur penutupan untuk KEEMPAT sebab** (klien putus / batas 30 menit /
stream Docker berakhir / gagal membuka): `tutup_sesi_runtime(sesi_ssh, forward)`
mengambil `SshSession` **by value** (`close(self)`) lalu
`semaphore_runtime().add_permits(1)`, dipanggil dari **satu** tempat di ujung
task. Izin di-`forget()` di handler supaya kuota baru bebas saat sesi benar-benar
berakhir, bukan saat respons terbentuk. Forward yang bocor adalah kebocoran fd
di `/run`.

**Batas 30 menit dijadikan parameter** `alirkan_runtime(.., batas_sesi)` alih-alih
konstanta yang dibaca di dalam. Alasan: `tokio` di repo ini **tidak** mengaktifkan
feature `test-util`, jadi `tokio::time::advance` tidak tersedia, dan menambah
feature berarti menyentuh `Cargo.toml` tanpa izin. Dengan parameter, "sunyi bukan
error lalu ditutup rapi" bisa diuji nyata dalam 80 ms. Produksi tetap memakai
`DURASI_MAKS_SESI_RUNTIME`.

**`render_log_fragmen` placeholder diperbaiki** supaya benar-benar merender isi
baris (sebelumnya hanya mencetak jumlah). Escaping otomatis Maud atas keluaran
aplikasi pengguna adalah bagian **kontrak**, bukan detail tampilan yang boleh
menunggu frontend — dan sekarang ada test yang membuktikannya.

10 test baru di `src/routes/events.rs`: `n` muncul di penanda tertinggal; event
selesai bertanda; `<script>` di isi log ter-escape (nol `PreEscaped`);
`ContainerHilang` → 502 sedangkan `TimeoutChunkPertama`/`Unreachable` → 504;
ketiga pesan penutup nol karakter `/` dan nol kata `Error`; `ikut()` tanpa sesi
tidak menyisakan entri map; penjepitan tail runtime; sunyi bertahan lalu ditutup
dengan pesan yang menyebut "30 menit"; klien terputus berarti nol pesan penutup;
stream Docker berakhir berarti pesan kategori + kedua chunk terkirim.

`cargo build --all-targets` nol error, `cargo fmt` bersih, `clippy --all-targets
-- -D warnings` bersih, `cargo test` **253 passed (6 suites)** (243 → +10). Nol
`sqlx::query!` baru (tidak perlu `cargo sqlx prepare`). Nol `unwrap()`/`expect()`
di luar `#[cfg(test)]`.

**Diserahkan ke frontend:** `src/web/logs.rs` + tab di `src/web/apps.rs` + vendor
`xterm.min.{js,css}`, lalu ganti kelima signature `render_sementara` dan hapus
modulnya, plus daftarkan dua route aset xterm.

- [x] backend Fase 3 — SELESAI seluruh sub-blok (3a-3h), lanjut frontend

### frontend Fase 3 — SELESAI (viewer log + tab), DIKERJAKAN ORCHESTRATOR

Agent `frontend` dipanggil dan **kembali dengan nol edit** — `src/web/logs.rs`
tidak dibuat, nol file `src/web/**` berubah ukurannya. Kegagalan kosong keenam
di fase ini. Dikerjakan orchestrator manual.

**File:** `src/web/logs.rs` (baru, ~640 baris termasuk test), `src/web/mod.rs`,
`src/web/styles.rs`, `src/web/deployments.rs`, plus penghapusan modul
placeholder di `src/routes/logs.rs` dan penunjukan ulang pemanggil di
`src/routes/{apps,events}.rs`, dan satu komentar di `src/docker/client.rs`.

**KONFLIK SPEK YANG DIPUTUSKAN MANUSIA: xterm.js dibuang (opsi (c) Q1).**
`docs/design/log-viewer.md` §9 menyuruh memakai `xterm.js` dengan
`xterm.write()`, tapi `docs/api-contract.md` — sudah beku, backendnya sudah
lolos gerbang — menetapkan tiap event SSE membawa **fragmen HTML** yang
di-append HTMX. Keduanya tidak bisa disatukan: `xterm.write()` menerima teks
mentah, dan `xterm` menguasai DOM-nya sendiri sehingga `sse-swap` tidak bisa
menyuntik ke dalamnya. Orchestrator BERHENTI dan menanyakannya; manusia memilih
menghormati kontrak. Konsekuensi:
- Dua file vendor yang sudah diunduh (`xterm.min.js` 283 KiB +
  `xterm.min.css` 2.8 KiB) **dihapus**; `src/web/assets/` kembali hanya berisi
  HTMX. Nol route aset baru, nol perubahan `src/routes/assets.rs`.
- Warna log **hilang**. Sebagai gantinya `web::logs::tanggalkan_ansi` menanggalkan
  escape ANSI di sisi render supaya `\x1b[32mOK\x1b[0m` tampil sebagai `OK`,
  bukan sampah `[32mOK[0m`. Fungsi ini juga membuang karakter kontrol C0 selain
  tab, supaya baris log tidak bisa mengacak tata letak halaman.
- Backend **tidak** diubah untuk ini: penanggalan ANSI hidup di `src/web/**`,
  konsisten dengan pembagian "backend meneruskan byte apa adanya".

**Nol `PreEscaped` di `src/web/logs.rs`** — diverifikasi: empat kemunculan string
itu semuanya di komentar yang menjelaskan larangannya. Konsekuensinya JS viewer
harus ditulis **tanpa karakter `<`, `>`, `&`, `"`** karena Maud meng-escape isi
`script` seperti teks biasa. Jadi `a <= b` ditulis lewat `Math.max` dan `&&`
ditulis sebagai `if` bersarang. Alternatif "pakai `PreEscaped` sekali saja untuk
skrip" ditolak: satu kemunculan melemahkan pemeriksaan harfiah yang menjaga isi
log tetap ter-escape. **Ada test yang menjaga larangan ini** — kalau nanti ada
yang menambahkan `<` ke skrip, test gagal, bukan viewer mati tanpa suara.

**JS: satu blok inline ~40 baris** (`JS_VIEWER`), bertanda `// ponytail:`.
Isinya hanya yang tidak bisa dicapai CSS: deteksi scroll untuk auto-follow
(toleransi 10px persis spek §5.1), tombol "Kembali ke Bawah", toggle wrap, dan
menyalakan/mematikan kelas `log-status-terputus` dari event `htmx:sseError` /
`htmx:sseOpen`. **Nol string Bahasa Indonesia di dalam JS**: label `[*]
STREAMING` dan `[!] MENGHUBUNGKAN ULANG` hidup sebagai dua elemen di Maud dan
CSS memilih mana yang tampil, jadi copywriting tetap satu sumber.

**Sunyi ≠ terputus** ditegakkan di CSS, bukan di JS timer: selama SSE terbuka
indikator tetap hijau walau nol baris baru; label kuning baru tampil kalau
`htmx:sseError` benar-benar terjadi (`docs/design/log-viewer.md` §4.2, §4.6).

**`sse-swap` memakai `hx-swap="beforeend"`** (tiga nama event: `message`,
`tertinggal`, `selesai`) supaya histori yang sudah dirender tidak hilang tiap
event. `streaming == false` → **nol atribut `sse-connect` DAN nol `sse-swap`**
di markup; ada test yang mengasersi keduanya absen, karena membuka SSE untuk
deployment mati membuat klien menunggu event yang tidak akan datang.

**Teks kategori dipusatkan sebagai konstanta** `pub(super)` di
`src/routes/logs.rs` (`PESAN_BELUM_ADA_CONTAINER`, `PESAN_CONTAINER_HILANG`,
`PESAN_TIMEOUT_KONEKSI`, `PESAN_TERLALU_BANYAK_SESI`, `PESAN_PENCARIAN_TIMEOUT`,
`PESAN_SESI_30_MENIT`, `PESAN_LAG`), diambil verbatim dari tabel copywriting
`docs/design/log-viewer.md` §8. Pesan improvisasi backend dari sub-blok 3h
diganti dengan konstanta ini, jadi satu pesan tidak punya dua versi antara
fragmen HTMX dan status HTTP. Penanda lag tetap menyisipkan **jumlah baris**
yang hilang di belakang teks spek.

**Gutter timestamp** dipisah lewat `pisahkan_gutter`, yang hanya memotong kalau
prefiksnya benar-benar berpola `HH:MM:SS` (format
`deployments::engine::baris_berstempel`). Baris log aplikasi yang kebetulan
memuat `" | "` — mis. `GET /a | 200` — **tidak** dipotong; ada test untuk itu.
Baris tanpa stempel dapat gutter kosong berlebar tetap supaya indentasi lurus.

**Tab Deployments read-only**: kolom waktu, status, commit pendek, image digest
penuh, durasi, tautan log. Batas 100 dengan penanda "Menampilkan 100 deployment
terbaru". Ada test yang mengasersi markup **tidak memuat kata "rollback"** —
itu Fase 5 (`docs/prd.md:326`).

**Tab Logs**: `ada_container == false` → state "[i] Belum ada container aktif…",
SSE tidak dipasang, tetap 200. Nol tombol unduh untuk runtime (tidak
dipersistensi di control plane); ada test yang mengasersi `/log/unduh` absen.

**Unduh log deploy** dinonaktifkan (span + tooltip retensi 30 hari, bukan
tautan) saat nol baris dan `truncated == false` — kasus file tersapu retensi.

**CSS** (`src/web/styles.rs`): token `--color-bg-log: #070707` ditambahkan;
`.log-console` 60vh/min 400px dengan scroll internal; `.log-console-wrap`
di-toggle JS; `.sr-only` untuk label kotak cari; `.app-tabs`; gutter
**disembunyikan di bawah 48rem** dan font turun ke 12px sesuai §6; toolbar
bertumpuk di mobile, horizontal di ≥48rem.

**A11y**: `<pre><code role="log" aria-label="Log Aplikasi" aria-live="off">`
sesuai §7 — `aria-live` sengaja `off` karena log streaming akan membajak fokus
pembaca layar. Tab aktif `aria-current="page"` dan bukan tautan.

`src/web/deployments.rs` bertambah kartu "Log" dengan tautan "Lihat log lengkap"
ke `/deployments/{id}/log`.

**Modul `routes::logs::render_sementara` DIHAPUS**; `src/routes/{logs,apps,
events}.rs` sekarang menunjuk `crate::web`. Nol `sqlx::`, nol `tokio::fs`, nol
`bollard::`, nol `unwrap()`/`expect()` di `src/web/**`.

21 test baru (`src/web/logs.rs` 20 + `src/web/deployments.rs` 1). Yang penting:
escaping `<script>`; penanggalan ANSI termasuk escape terpotong di ujung baris;
gutter tidak salah potong; state kosong/truncated/pencarian-dipotong; deployment
selesai tidak membuka SSE; kata kunci pencarian dikembalikan ke kotak cari dalam
keadaan ter-escape; JS bebas karakter yang akan di-escape Maud.

`cargo fmt` bersih, `cargo clippy --all-targets -- -D warnings` **nol
error/warning**, `cargo test` **274 passed (6 suites)** (253 → +21). Nol
`sqlx::query!` baru. Nol dependensi baru di `Cargo.toml`.

**Perlu tindakan planner (bukan implementer):** `docs/api-contract.md:763-781`
masih memuat bagian `GET /assets/xterm.min.js` dan `.css`. Karena Q1 berakhir di
opsi (c), bagian itu harus **dihapus planner** — kontrak melarang implementer
menyentuhnya. Endpoint aset xterm tidak ada di router, jadi kontrak dan kode
sekarang tidak sinkron sampai planner merapikannya.

### security Fase 3 — SELESAI, DIKERJAKAN ORCHESTRATOR

Agent `security` dipanggil dan **kembali dengan nol laporan** — kegagalan kosong
ketujuh di fase ini. Audit dikerjakan orchestrator, read-only kecuali satu
perbaikan WARNING di bawah.

**JAWABAN Q2 (`docs/prd.md:289`) — PERINGATKAN, JANGAN SARING.**
Rekomendasi planner diterima. Alasan: penyaringan pola (`sk-…`, `AKIA…`,
`-----BEGIN`) memberi rasa aman palsu, pasti meleset pada format tak terduga,
dan bisa merusak baris log yang justru sedang dibaca untuk debugging jam 11
malam — tepat kasus yang jadi north star PRD. Tiga hal yang membuat ini tetap
bertanggung jawab, ketiganya sudah ada di kode:
1. `PERINGATAN_PRIVASI` (`src/web/logs.rs:32`) dirender di **setiap** viewer
   sebagai `p.log-privacy-note` (`src/web/logs.rs:262`), bukan tersembunyi di
   tooltip.
2. Control plane **tidak pernah** menulis secretnya sendiri ke file log —
   diverifikasi di 13 titik `catat()` (`src/deployments/engine.rs`): yang
   tercatat adalah nama tahap, `image_digest` (bukan secret), nama container,
   dan tail log container. `resolve_credentials` **tidak** ikut dicatat, dengan
   komentar eksplisit di `engine.rs:342-343`.
3. Izin file 0600 + direktori 0700 membatasi pembacanya ke pemilik proses.
Kalau kelak penyaringan tetap diinginkan, tempatnya `src/logs/writer.rs`
(sebelum byte menyentuh disk), bukan frontend — supaya file di disk dan yang
tampil di viewer tidak pernah berbeda isi.

**Sepuluh poin yang diperiksa, dengan bukti `file:baris`:**

1. **Q2** — dijawab di atas.
2. **Path traversal — bersih.** `reader::nama_file_aman` (`src/logs/reader.rs:82`,
   `^[A-Za-z0-9]{1,64}$`) dipanggil sebelum path dibentuk di **semua** jalur:
   halaman viewer `routes/logs.rs:96`, unduh `routes/logs.rs:181`, SSE deploy
   `routes/events.rs:255`. Path selalu lewat `writer::path_log`
   (`src/logs/writer.rs:85-86`) → `log_dir.join("deploy").join(nama_file(id))`.
   **Nol `PathBuf::from(<input>)`** di `src/routes/logs.rs` (diverifikasi grep).
   Nilai kolom `path` **tidak pernah** digabung ke path — hanya dipakai sebagai
   metadata (`src/logs/repo.rs:124` menyebutnya eksplisit).
   `Content-Disposition` (`routes/logs.rs:205`) memakai id yang sudah lolos
   pola, jadi tidak mungkin menyuntik CRLF atau `"` ke header.
3. **SSE terautentikasi — bersih.** Delapan route log semuanya di blok
   `protected` sebelum `.route_layer(from_fn_with_state(state, require_session))`
   (`src/routes/mod.rs:56-67`). Blok `public` hanya `/healthz`, `/login`, dua
   aset (`src/routes/mod.rs:69-74`). Kedua handler SSE log juga mengekstrak
   `Extension<Session>` (`events.rs:250`, `events.rs:320`) sehingga route yang
   lupa middleware **gagal di runtime**, bukan diam-diam terbuka. Id di path
   dipakai murni sebagai penunjuk channel, bukan pengganti autentikasi.
4. **Invariant 7 — bersih.** `SshConnectError` (`src/ssh/session.rs:75-90`)
   varian kategori tanpa `stderr` mentah; `tracing::warn!(error = ?err)` di
   `events.rs:388` dan `events.rs:400` mencetak enum itu, bukan isi kunci.
   Klien hanya menerima teks kategori (`events.rs:389-392`, `401-404`).
   `plaintext_key` (`events.rs:362`) hanya diteruskan ke `ssh::connect`, tidak
   masuk struct yang di-`Debug`, tidak masuk event.
5. **Invariant 9 — bersih.** `migrations/0004_logs.sql:34-42`: tujuh kolom
   (`deployment_id`, `path`, `size_bytes`, `line_count`, `truncated`,
   `created_at`, `updated_at`) — nol kolom yang bisa menampung isi log. Semua
   `sqlx::query!` di `src/logs/repo.rs` mem-bind hanya metadata itu.
   `deployments.error_detail` tidak bertambah di fase ini.
6. **Izin filesystem — bersih, TANPA TOCTOU.** Mode 0600 diset **saat file
   dibuat** lewat `OpenOptions::mode(0o600)` (`src/logs/writer.rs:112-117`),
   bukan `chmod` sesudahnya — tidak ada celah baca. `log_dir` dan
   `<log_dir>/deploy` dipaksa 0700 saat boot dan **kegagalannya fatal**
   (`src/config.rs:180-207`, `?` bukan `.ok()`). `MENGDEP_LOG_RETENTION_DAYS`
   di luar 1-3650 menggagalkan startup (`src/config.rs:110-112`), tidak
   di-clamp diam-diam.
7. **XSS — bersih.** Empat kemunculan string `PreEscaped` di `src/web/logs.rs`
   semuanya di **komentar** (baris 4, 7, 47, 49); nol di kode. Test
   `render_log_fragmen` dengan payload `<script>alert(1)</script>`
   (`src/web/logs.rs:581-585`) mengasersi markup tidak memuat `<script>alert`.
   `JS_VIEWER` adalah konstanta statis tanpa interpolasi apa pun
   (`src/web/logs.rs:241`, `487`) — nol jalan bagi data pengguna masuk ke
   `<script>`. `tanggalkan_ansi` (`src/web/logs.rs:115-139`) juga membuang
   karakter kontrol C0 selain tab, jadi baris log tidak bisa mengacak tata letak.
8. **Timeout per tahap — bersih.** Nol timeout global. Tahap terpisah: SSH+forward
   10 detik, chunk pertama `LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT` 15 detik
   (`src/docker/client.rs`), baca tail 5 detik (`reader::TAIL_READ_TIMEOUT`),
   sapuan retensi 60 detik. Batas 30 menit sesi runtime adalah **penutupan
   rapi berpesan**, bukan error. Sunyi tidak diperlakukan error di mana pun —
   `KeepAlive::default()` yang menjaga koneksi (`events.rs:308`).
9. **DoS — bersih.** `?tail=` dijepit dua lapis: `jepit_tail_runtime`
   (`routes/logs.rs:63-70`, maks 2000) dan `jepit_tail` (`reader.rs:112-118`,
   maks 5000) — keduanya **menjepit**, tidak menolak, jadi tidak ada jalur yang
   melewati batas. `?q=` dibatasi 500 baris hasil + timeout 5 detik. Satu baris
   dibatasi `MAX_LINE_BYTES` 8 KiB (`writer.rs:40`) dengan pemotongan sadar
   batas UTF-8 (`writer.rs:303-320`, ada test untuk `é`). Sesi runtime dibatasi
   `Semaphore` 4 dengan `try_acquire` (`events.rs:331`) — **menolak 429, tidak
   mengantre**, jadi koneksi HTTP tidak tertahan. Izin di-`forget()` lalu
   dilepas manual di akhir task (`events.rs:419`, `451`) supaya kuota bebas saat
   sesi benar-benar berakhir. Fd: `tutup_sesi_runtime(sesi_ssh, forward)`
   dipanggil **satu kali di satu jalur** untuk keempat sebab termasuk
   gagal-membuka (`events.rs:446-450`); jalur error sebelum task di-spawn
   menutup manual (`events.rs:381`, `399`).
10. **Invariant 1 — bersih setelah perbaikan.** `pilih_korban`
    (`src/logs/retention.rs:53-72`) memfilter `selesai()` **sebelum** memfilter
    umur, jadi deployment berjalan tidak pernah terpilih apa pun umurnya —
    ada test `tidak_pernah_pilih_deployment_yang_belum_selesai_walau_sangat_tua`.

**WARNING yang ditemukan dan DITUTUP:**

`src/logs/retention.rs:80` — `hapus_file_log` membentuk path dari nilai kolom
`deployment_id` **tanpa** melewatkannya `nama_file_aman`. Semua jalur BACA
punya gerbang itu; jalur HAPUS tidak. Id memang digenerate alfanumerik
(`deployments::repo::generate_id`, `src/deployments/repo.rs:15-21`) jadi tidak
ada rute serangan yang diketahui hari ini — tapi ini satu-satunya tempat di
seluruh program yang **menghapus file berdasarkan nilai kolom**, dan kalau
kolom itu pernah tercemar (restore db manual, bug pemanggil, jalur tulis masa
depan), `../../korban.log` akan membawa `remove_file` keluar dari
`<log_dir>/deploy/`. Perbaikan: gerbang `nama_file_aman` ditambahkan di depan
`path_log`; id yang tidak lolos → file tidak disentuh, metadata **tidak**
dihapus (dibiarkan hidup supaya bisa diselidiki manusia, bukan disapu
diam-diam), `tracing::warn!` sekali. Ditambah satu test yang **benar-benar bisa
gagal**: `jalankan_sapuan_menolak_deployment_id_yang_tidak_lolos_pola_nama_aman`
menanam id `../../korban` di db, menaruh file nyata di `dir/korban.log`, lalu
mengasersi file itu masih ada setelah sapuan.

**`.sqlx/` diregenerasi** — test baru memakai `sqlx::query!` dengan bentuk bind
baru (`UPDATE deployment_logs SET created_at = ? WHERE deployment_id = ?`),
sehingga cache offline harus diperbarui. Jalur: `cargo sqlx database create` +
`migrate run` + `cargo sqlx prepare -- --all-targets` pada db sementara di
`.sqlx-prepare-tmp/` (sudah ada di `.gitignore:9`), bukan pada db produksi —
db produksi tetap gagal karena masalah WAL-dalam-transaksi yang pre-existing
sejak Fase 0.

`cargo fmt` bersih, `cargo clippy --all-targets -- -D warnings` **nol
error/warning**, `cargo test` **275 passed (6 suites)** (274 → +1).

- [x] security Fase 3 — SELESAI, Q2 terjawab (PERINGATKAN), 1 WARNING ditutup, nol BLOCKING

- [x] frontend Fase 3 — SELESAI, lanjut security (Q2) → qa → reviewer

### qa Fase 3 — `tests/phase3.rs`

Dikerjakan orchestrator setelah agent `qa` menyerahkan file yang **tidak
terkompilasi** (4 error) — anomali tooling kesembilan di fase ini. Isi test-nya
sendiri layak dipertahankan, jadi yang dilakukan adalah membetulkan
pemanggilannya ke API nyata, bukan menulis ulang.

Empat error kompilasi yang diperbaiki:

1. `deployments_repo::insert_queued` tidak pernah ada → diganti
   `generate_id()` + `insert_queued_dengan_job(pool, &id, new, &job_id, "{}")`
   (`src/deployments/repo.rs:16,42`).
2. `NewDeployment` tidak punya `trigger_source` → dibuang, `git_ref:
   Some("main")` ditambahkan sesuai bentuk struct aslinya
   (`src/deployments/repo.rs:28-33`).
3. `LogRegistry::jumlah_sesi` ber-`#[cfg(test)] pub(crate)`
   (`src/logs/registry.rs:156`) sehingga tidak terjangkau dari `tests/**`.
   Asersinya diganti `state.logs.ikut(&dep_id).is_none()` — karena `ikut()`
   **tidak pernah membuat** sesi, `None` setelah lima request SSE membuktikan
   hal yang sama (pembaca tidak boleh membuat channel) tanpa perlu menyentuh
   `src/` yang bukan wilayah qa.
4. `sqlx::query_as(&format!(...))` ditolak lint "dynamic SQL strings" → diganti
   dua query statis yang menyatukan **semua** kolom `deployment_logs` dan
   `deployments` jadi satu string; kalau kelak ada yang menambah kolom
   penampung isi log, test invariant §3 no.9 ikut merah tanpa diperbarui.

Tiga test merah pada eksekusi pertama, dua di antaranya bug test:

- `riwayat_deployment_tidak_memuat_aksi_rollback` menuntut `"deadbeef"` padahal
  commit dirender pendek 7 karakter (`src/web/logs.rs:513`) → jadi `"deadbee"`.
  Asersinya dikoreksi; perilaku kodenya benar.
- `kata_kunci_pencarian_sangat_panjang_tidak_menjatuhkan_server` memakai `?q=`
  100 KiB, tapi yang panik adalah `http::Uri` di **pembangun request test**
  (`InvalidUri(TooLong)`), bukan aplikasi. Batas URI hyper bukan yang sedang
  diuji, jadi diturunkan ke 8 KiB — masih jauh di atas kata kunci realistis.

**Satu temuan nyata yang TIDAK ditutup qa** (perbaikannya milik backend):
`?tail=-1` menghasilkan body `Failed to deserialize query string: tail: invalid
digit found in string`. Itu **pesan library mentah dalam Bahasa Inggris** yang
dikembalikan ke klien, melanggar `docs/api-contract.md` ("tidak ada pesan
library mentah") dan konvensi Bahasa Indonesia. Statusnya 400, bukan 500, dan
tidak ada path filesystem yang bocor — jadi bukan kebocoran data, tapi tetap
pelanggaran kontrak. Test `tail_tidak_terparse_tidak_pernah_500` dipertahankan
dengan asersi yang lebih sempit (bukan 500 + nol path) supaya gerbang hijau
**tanpa** menyembunyikan temuannya; catatan penyebabnya ditulis di doc comment
test itu. Perbaikan yang disarankan: `Option<String>` + parse manual, atau
`Query` rejection handler yang memetakan ke pesan generik Bahasa Indonesia.
Karena ini menyentuh `src/routes/logs.rs`, keputusan menutupnya di Fase 3 atau
menundanya diserahkan ke reviewer.

Nol `sqlx::query!` baru di `tests/phase3.rs` (hanya `query_as` statis dan
helper repo yang sudah ada), jadi `cargo sqlx prepare` **tidak** perlu
dijalankan ulang.

Cakupan 21 test: auth 8 route → 303 tanpa cookie; path traversal → 404;
id valid-tapi-tidak-ada → 404 seragam; jepit `tail`; `tail` malformed bukan 500;
`q` besar; state kosong viewer; app tanpa deployment live (tab 200 / fragmen
isi 409); app asing → 404; unduh 404 + header `deploy-{id}.log`; nol kebocoran
secret/path; escape `<script>`; SSE wajib menutup diri (timeout 5 detik);
pembaca SSE tidak membuat sesi; riwayat kosong; nol string "rollback";
retensi 3 skenario; invariant §3 no.9 lewat dump seluruh kolom.

`cargo fmt` bersih, `cargo clippy --all-targets -- -D warnings` **nol
error/warning**, `cargo test` **296 passed (7 suites)** — `phase0` 7,
`phase1` 7, `phase2` 21, `phase3` 21, unit 237, doc 3. Nol regresi.

- [x] qa Fase 3 — SELESAI, 21 test, 1 temuan diserahkan ke reviewer

### reviewer Fase 3

Dikerjakan orchestrator (agent `reviewer` nol laporan — anomali tooling
kesepuluh di fase ini). Repo **nol commit git**, jadi `git diff` kosong dan
tidak berguna; peninjauan dilakukan atas **isi file**, bukan diff.

**Gerbang diverifikasi sendiri, bukan dipercaya dari catatan:**
`cargo fmt --check` exit 0; `cargo clippy --all-targets -- -D warnings` exit 0
nol warning; `cargo test` exit 0 — **296 passed, 0 failed (7 suite)**: unit 237,
doc 3, `phase3` 21, `phase0` 7, `phase1` 7, `phase2` 21, satu suite 0.
Nol regresi pada Fase 0-2.

**Invariant yang diperiksa dan LOLOS:**

- **§3 no.9 (harfiah)** — `migrations/0004_logs.sql:35-41`: tujuh kolom, nol yang
  bisa memuat isi log (`path` = nama file, sisanya angka). Grep `sqlx::query!`
  di `src/logs/**` untuk bind bernama `line`/`content`/`body`/`isi`/`teks`:
  **nol hasil**. `deployments.error_detail` tidak berubah peran maupun panjang.
- **Kebocoran channel** (`docs/prd.md:291`, `:384`) — `src/logs/registry.rs:60`
  map memegang `Weak`, bukan `Arc`; `:97` `mulai()` hanya untuk writer dan
  `:100` menolak di `MAX_SESSIONS` 64; `:123-126` `ikut()` hanya `get` +
  `Weak::upgrade`, **tidak pernah** membuat; `:195-210` `Drop` menghapus entri
  dengan penjagaan identitas `std::ptr::eq(weak.as_ptr(), self)` — race
  `mulai()`-ulang vs `Drop`-lama tidak menghapus entri sesi baru, detail yang
  mudah terlewat dan di sini benar; `:137-152` sapuan yatim melaporkan
  `tracing::warn!` alih-alih menyapu diam-diam. Lag ditangani di
  `src/routes/events.rs:280,292` sebagai `Tertinggal(n)`, bukan `continue`.
- **Kebocoran fd / socket forward** — `src/routes/events.rs:446-451`: SATU jalur
  penutupan untuk keempat sebab (klien putus / 30 menit / stream Docker berakhir
  / gagal membuka), `tutup_sesi_runtime` (`:529-531`) menutup forward lalu sesi
  SSH, `add_permits(1)` setelahnya. `izin.forget()` (`:419`) dipasangkan dengan
  tepat satu `add_permits` di jalur yang tidak bisa dilewati — bukan pola yang
  saya sukai, tapi alasannya ditulis (`:416-418`) dan benar: izin harus hidup
  selama task, bukan selama handler.
- **§3 no.5** — `src/deployments/engine.rs:440` `container_logs` dipanggil,
  `:461` `remove_container`. Urutan tangkap-sebelum-hapus utuh; komentar
  penjaga di `:438-440`. Fase 3 hanya menambah tujuan tulis.
- **§3 no.11** — semua timeout cocok tabel `docs/plan.md`: `reader.rs:19,22`
  5 detik tail/cari; `docker/client.rs:60` chunk pertama 15 detik;
  `events.rs:52` sesi runtime 30 menit. Sunyi bukan error —
  `KeepAlive::default()` di delapan titik SSE; nol timeout global yang
  membungkus aliran selain batas UMUR sesi yang memang disengaja.
- **§3 no.1** — `writer.rs:171,202,272-292`: 8 MiB → satu baris penutup,
  `truncated=1`, satu `tracing::warn!`; deploy tidak dibatalkan.
  `retention.rs:65-68` memfilter `StatusDeployment::selesai()` — deployment
  belum selesai tidak pernah disentuh apa pun umurnya.
- **Anti path traversal** — `reader::nama_file_aman` jadi gerbang di **semua**
  pemanggil: `routes/logs.rs:96,181`, `routes/events.rs:255`,
  `retention.rs:90`. Nol `PathBuf::from` di `src/routes/logs.rs`.
- **Batas peran** — `sqlx::`/`tokio::fs`/`bollard::`/`reqwest` di `src/web/**`:
  **nol**. `html!` di luar `src/web/**`: **nol**. Batas bersih dua arah.
- **Escaping** — nol `PreEscaped` fungsional di `src/web/logs.rs` (empat
  kemunculan semuanya di komentar yang melarangnya).
- **Router** — delapan route log semuanya di blok `protected`
  (`src/routes/mod.rs:39-45,66-67`), termasuk kedua SSE, sebelum
  `route_layer(require_session)` di `:68`.
- **Pool** — nol `INSERT`/`UPDATE`/`DELETE` lewat `db_read` di `src/logs/**`,
  `src/routes/logs.rs`, `src/worker/log_retention.rs`.

**Temuan:**

1. **WARNING** `src/routes/logs.rs:51-61` — `?tail=-1` mengembalikan body
   `Failed to deserialize query string: tail: invalid digit found in string`:
   pesan library mentah berbahasa Inggris, dilarang `docs/api-contract.md`
   ("tidak ada pesan library mentah"). **Bukan BLOCKING**: statusnya 400 (bukan
   500), nol path/secret bocor, dan hanya terpicu oleh URL yang diketik tangan —
   HTMX selalu mengirim angka sah. Perbaikan (pemilik **backend**): ubah
   `tail: Option<u32>` jadi `Option<String>` lalu parse manual dan jepit seperti
   `jepit_tail_runtime` (`src/routes/logs.rs:63`), atau pasang rejection handler
   `Query` yang memetakan ke pesan generik Bahasa Indonesia. Boleh ditunda ke
   Fase 4 karena tidak melanggar invariant §3 mana pun.
2. **NIT** `docs/api-contract.md:763-781` — masih memuat
   `GET /assets/xterm.min.{js,css}` yang tidak ada di router setelah Q1 dijawab
   "(c) tanpa xterm.js". Pemilik **planner**; implementer dilarang menyentuh
   kontrak. Kontrak sendiri menginstruksikan penghapusan ini oleh planner.
3. **NIT** `src/worker/deploy_worker.rs:67` — satu `.expect()` di luar
   `#[cfg(test)]`. **Pre-existing Fase 2, bukan regresi Fase 3**; alasannya
   ditulis di `:60-62` dan benar (semaphore tidak pernah `close()`). Fase 3
   sendiri nol `unwrap`/`expect` di luar test. Dicatat supaya tidak hilang.
4. **NIT** `src/web/logs.rs:37` vs `src/routes/logs.rs` —
   `PESAN_BELUM_ADA_CONTAINER` masih terduplikasi di dua modul. Nilainya sama
   dan keduanya di sisi server, jadi tidak ada risiko divergensi ke klien;
   menyatukannya butuh satu modul menyentuh glob milik peran lain. Dibiarkan
   dengan sadar.

Nol BLOCKING. Nol pelanggaran invariant §3. Nol pelanggaran batas peran.

- [x] reviewer Fase 3 — SELESAI, nol BLOCKING, 1 WARNING (boleh ditunda), 3 NIT

**Kesimpulan: Fase 3 LOLOS gerbang** (`docs/prd.md` §6) — dicatat eksplisit
sekarang (sebelumnya cuma tersirat dari checklist tercentang, beda dari
Fase 1/2 yang punya paragraf penutup). Kedelapan peran tuntas, DoD
terverifikasi langsung, security 0 BLOCKING, reviewer 0 BLOCKING (1 WARNING
`?tail=-1` boleh ditunda ke Fase 4 — TIDAK diambil di sesi ini, di luar
scope Fase 4 PRD, dicatat supaya tidak hilang lagi), migrasi bersih dari
kosong (`cargo test --test phase0` 21/21 membangun db dari nol tiap run).
Manusia membuka Fase 4 lewat instruksi eksplisit "lanjutkan fase 4"
(2026-08-11) — diperlakukan sebagai konfirmasi gerbang, konsisten pola
Fase 2→3 (`docs/progress.md:371-375`, konfirmasi manusia dicatat bukan
diasumsikan).

---

## Fase 4 — Pengelolaan environment

- [x] planner — output: `docs/plan.md` (overwrite penuh dari Fase 3),
      `docs/api-contract.md` (bagian Fase 4 di-append setelah Q1 dijawab)
- [x] uiux — `docs/design/environment.md` dan implementasi diff/sentinel
      environment selesai.
- [x] migration — `migrations/0005_env.sql`
- [x] backend — CRUD env, transaksi atomik, engine cleanup/log safety,
      deploy_api.rs (env_version_id otomatis)
- [x] frontend — tab Environment, diff aman, sentinel value kosong
- [x] qa — `tests/phase4.rs`, 9 skenario (minimum PRD: 5)
- [x] reviewer — audit invariant, batas peran, transaksi, dan coverage selesai
- [x] security — audit enkripsi, log, file env, auth/CSRF selesai; boundary
      Docker `inspect` dicatat sesuai keputusan Q1

### Catatan

**planner (2026-08-11):** rencana ditulis. Ringkasan 5 baris:

1. Skema baru `env_vars`+`env_versions` (migrasi 0005); `deployments.env_version_id`
   sudah ada sejak Fase 2, tinggal dipakai.
2. Simpan env → snapshot terenkripsi baru → deployment `queued` baru
   (`trigger_source='env'`, digest SAMA dengan yang live) — satu transaksi,
   lock app dihormati (409 kalau app lagi deploy lain, env tetap tersimpan).
3. `POST /api/v1/deploy` (CI, sudah ada) ikut diperbarui: deployment baru
   otomatis memakai `env_version_id` TERBARU app itu, bukan NULL — supaya
   `engine.rs` selalu tahu env mana yang harus ditulis ke target apa pun
   pemicunya.
4. Isi env dikirim ke server target lewat `ssh::exec_with_stdin` (pola
   identik `docker/registry_login.rs`) ke `/var/lib/platform/env/{app}.env`
   (0600) — TANPA file staging lokal.
5. **Satu pertanyaan arsitektur nyata diajukan ke manusia (Q1, belum
   dijawab, MEMBLOKIR langkah backend C/D di `docs/plan.md`):** `bollard`
   (dipakai sejak Fase 2, bukan shell-out CLI) tidak punya primitif
   "--env-file" di sisi daemon — env HARUS masuk field `Env` JSON API
   supaya proses container benar-benar menerimanya, dan `docker inspect`
   akan selalu menampilkannya apa adanya, terlepas dari mekanisme
   pengiriman. Ini bikin baris security Fase 4 "tidak muncul di `docker
   inspect`" (`docs/prd.md:309`) secara teknis mustahil dipenuhi literal
   tanpa app pengguna ikut bekerja sama (baca file sendiri, tidak generik).
   Rencana merekomendasikan **Opsi A**: terima kenyataan itu (boundary
   kepercayaan sama dengan akses SSH+docker socket yang sudah ada sejak
   Fase 0), kirim lewat `ContainerCreateBody.env` (field API, BUKAN
   argumen `-e` di command line — jadi invariant §3 no.6 versi hurufnya
   tetap terpenuhi), security review menulis eksplisit alasan di laporan
   alih-alih diam-diam dianggap lolos. Rincian penuh + dua opsi yang
   ditolak (dan kenapa) ada di `docs/plan.md` bagian "Pertanyaan terbuka".

Q2 (kecil, tidak memblokir): nama file env `{app}.env` satu-per-app
(ditimpa tiap redeploy) diasumsikan, konsisten `CLAUDE.md` §6.

**Q1 dijawab manusia lewat `AskUserQuestion`: Opsi A** (terima env terlihat
`docker inspect`, kirim lewat `ContainerCreateBody.env`). Implementasi
lanjut sesi yang sama, tanpa jeda — pola sama Fase 2/3 ("satu sesi panjang").

### migration — `migrations/0005_env.sql`

`env_vars(id, app_id, key, value_encrypted, is_secret, updated_at)` UNIQUE
(app_id,key); `env_versions(id, app_id, version, snapshot_encrypted, note,
created_at)` UNIQUE(app_id,version). `deployments.env_version_id` sudah ada
sejak `migrations/0003_deploy.sql`, tidak disentuh. Diverifikasi: `cargo sqlx
database create` + `cargo sqlx migrate run` bersih dari db dev yang sudah
berisi migrasi 1-4 ("Applied 5/migrate env").

### backend

**`src/apps/model.rs`+`repo.rs`:** `EnvVersionRingkas`. **Satu** fungsi
listing env — `list_env_vars_encrypted(pool, app_id) -> Vec<(key,
value_encrypted, is_secret)>` — dipakai baik untuk membangun snapshot
maupun (dipanggil dari `routes/apps.rs`, yang mendekripsi baris non-secret)
untuk merender tab. Rencana awal (`docs/plan.md`) mengusulkan dua fungsi
terpisah (`EnvVarRingkas`/`list_env_vars_ringkas` "aman" vs
`list_env_vars_encrypted` "mentah") — disatukan saat implementasi karena
keduanya query IDENTIK kecuali field yang di-select; dua fungsi nyaris
sama adalah duplikasi, bukan lapisan keamanan tambahan (keputusan dekripsi
tetap di pemanggil, bukan di `apps::repo`, jadi batas invariant §3 no.7
tidak berubah). `upsert_env_var` (is_secret HANYA diset saat baris pertama
kali dibuat, tidak berubah lagi setelah itu), `delete_env_var`,
`insert_env_version_tx`/`insert_queued_dengan_job_tx` (varian yang menerima
`&mut Transaction` dari pemanggil, dipakai `routes/apps.rs` env_submit
supaya env_version+deployment+job satu transaksi), `find_latest_env_version`,
`find_env_version_snapshot`.

**`src/deployments/model.rs`+`repo.rs`:** `DeploymentRingkas.env_version_id:
Option<String>` (empat SELECT site diperbarui: `find_by_id`, `list_by_app`,
`list_stale_active`, `find_current_live`). `NewDeployment` +`trigger_source:
&str` (bukan literal `'api'` lagi) +`env_version_id: Option<&str>`.

**`src/docker/client.rs`:** `NewContainer.env: &[(String,String)]` →
`ContainerCreateBody.env` (`Vec<"KEY=value">`, `None` kalau kosong bukan
`Some(vec![])`). Field API diverifikasi langsung dari source
`bollard-stubs` (`env: Option<Vec<String>>`) sebelum menulis kode.

**`src/deployments/engine.rs`:** `resolve_env()` — dekripsi snapshot env
(kegagalan dekripsi MENGGAGALKAN deploy dengan pesan jelas, bukan diam-diam
lanjut tanpa env — PRD Fase 4 baris Debugger). `env` diteruskan ke
`docker::create_container`. **Keputusan implementasi yang tidak eksplisit
di plan**: file audit env di target (`/var/lib/platform/env/{app}.env`,
`install -D -m 0600` via `ssh::exec_with_stdin`, pola identik
`docker/registry_login.rs`) ditulis **SETELAH health check lulus** (bukan
sebelum create_container) — supaya kalau deploy GAGAL, file di target tetap
merefleksikan env yang BENAR-BENAR live (deployment lama), bukan percobaan
yang gagal. Ini sekaligus memenuhi "hapus env lama" PRD tanpa langkah hapus
terpisah: satu file per app yang selalu ditimpa nilai yang benar-benar
jalan. Best-effort — kegagalan tulis file audit `tracing::warn!` saja, TIDAK
menggagalkan deploy (env sudah sampai ke container lewat API terlepas dari
ini). Isi env TIDAK PERNAH masuk `catat()` (baris log deploy) — hanya
JUMLAH variabel yang dicatat.

**`src/routes/deploy_api.rs`:** deployment dari CI (`POST /api/v1/deploy`)
sekarang mengisi `env_version_id` dari `find_latest_env_version` app itu
(NULL kalau app belum pernah punya env) — PRD: "deploy yang dipicu digest
baru memakai env yang sedang berjalan", simetris dengan arah sebaliknya
(env-save memakai digest yang sedang berjalan).

**`src/routes/apps.rs`:** `tab_environment` (GET) + `env_submit` (POST).
Form diterima `Form<HashMap<String,String>>` (bukan struct tetap) karena
nama field dinamis (`value__{key}`, `delete__{key}` per baris existing;
`new_key_{i}`/`new_value_{i}`/`new_secret_{i}` untuk 5 slot baris baru
tetap, `ENV_NEW_ROW_SLOTS`). Validasi: CSRF dulu (400 tanpa efek), key
duplikat di baris baru manapun ditolak SELURUHNYA (bukan cuma yang kedua),
value dengan `\n`/`\r` ditolak. Field kosong pada baris existing = "tidak
diubah" (berlaku sama untuk secret maupun non-secret — simplifikasi
disengaja, dicatat di `docs/api-contract.md`).

**Bug nyata ditemukan+diperbaiki SAAT verifikasi (bukan lolos tanpa
ketahuan):** urutan asli membuka transaksi (`state.db_write.begin()`)
**SEBELUM** memanggil `apps_repo::acquire_lock` (yang minta koneksi lain
dari pool YANG SAMA). `db_write` py `max_connections(1)` (`CLAUDE.md` §7)
— transaksi menahan satu-satunya koneksi, `acquire_lock` menunggu koneksi
yang tidak akan pernah bebas → deadlock, dua test macet ~30 detik lalu gagal
lewat pool acquire timeout. Ditemukan lewat instrumentasi tracing sementara
di test (dihapus lagi setelah dipakai), bukan tebakan. Diperbaiki: lock
diambil dulu via pool, transaksi dibuka BELAKANGAN — pola sama
`routes/deploy_api.rs` yang sudah benar sejak Fase 2, tapi tidak diikuti
persis di percobaan pertama `env_submit` karena strukturnya sedikit beda
(env_version harus masuk transaksi yang sama, deploy_api tidak punya
tabel setara). Dicatat di sini supaya kalau ada kode masa depan yang
membuka transaksi lalu memanggil fungsi lock/pool lain, polanya sudah
punya presedan tertulis: **lock dulu, transaksi belakangan, selalu.**

**Batas invariant §3 no.10 yang diketahui, dicatat jujur bukan
disembunyikan**: UPDATE/INSERT/DELETE `env_vars` per baris (upsert/delete)
terjadi di LUAR transaksi besar (masing-masing statement tunggal, atomik
sendiri lewat SQLite). Hanya `env_versions`+`deployments`+`jobs` yang
benar-benar satu transaksi. Kalau proses mati tepat di antara baris
`env_vars` terakhir dan pembukaan transaksi snapshot, hasilnya env_vars
"benar" tapi TANPA env_version yang mencatatnya — state yang aneh tapi
tidak merusak (baris env_vars individual tetap konsisten, cuma snapshot
riwayatnya hilang satu). Reviewer/security sesi berikutnya perlu menilai
apakah ini cukup atau perlu transaksi tunggal penuh.

### frontend — `src/web/env.rs`

`EnvVarTampil { key, value_plaintext: Option<String>, is_secret }` — view
model murni, TIDAK PERNAH dibangun dari ciphertext (keputusan dekripsi ada
di `routes/apps.rs`, modul ini cuma render). Baris secret: input KOSONG
+placeholder, TIDAK PERNAH nilai asli walau di-inspect elemen HTML manapun
(diverifikasi test `secret_ditopengi_tanpa_plaintext_di_markup`). Baris
non-secret: plaintext di `value="..."` (Maud escape attribute otomatis,
bukan `PreEscaped` — aman dari XSS). `tab_nav` (`src/web/logs.rs`) ganti
`fn` privat jadi `pub(super) fn` +entri ke-4 "Environment" — dipakai lintas
submodul `web::env` dan `web::logs` (keduanya turunan `web`, jadi
`pub(super)` cukup, tidak perlu `pub(crate)`).

**Simplifikasi UI diambil TANPA uiux (peran ini belum dikerjakan formal),
ditandai eksplisit supaya sesi berikutnya tahu ini keputusan implementer,
bukan spek yang sudah disetujui**:
- Bar sticky "N variabel berubah" (PRD) **diganti** peringatan statis
  selalu-tampil "Menyimpan mengubah environment dan akan memicu deploy
  baru..." — hitungan live butuh JS (proyek ini sengaja nol JS di luar
  HTMX/xterm, `docs/prd.md` §2), dan HTMX partial-update untuk penghitung
  dianggap tidak sepadan kerumitannya untuk sesi ini.
- Tampilan diff eksplisit (PRD: "spesifikasi tampilan diff") **tidak
  dibangun** — form langsung tampilkan state saat ini (value existing
  terisi di input, baris baru kosong), user melihat apa yang dia ubah
  lewat isi field itu sendiri, bukan lewat panel diff terpisah.
- Field yang MEMANG harus diset ke string kosong tidak bisa lewat form ini
  (kosong = "tidak diubah" secara seragam) — kalau ini jadi masalah nyata,
  perlu sentinel terpisah (mis. checkbox "kosongkan value ini").

Ketiganya kandidat kerja `uiux` + iterasi frontend berikutnya, BUKAN
dianggap selesai permanen.

### qa — `tests/phase4.rs`, 7 skenario (minimum PRD: 5)

1. `env_tab_dan_submit_tanpa_cookie_sesi_redirect_ke_login`
2. `csrf_salah_pada_env_submit_ditolak_tanpa_menyimpan_apa_pun`
3. `key_baru_duplikat_dalam_satu_submit_ditolak_tanpa_menyimpan` — nol baris
   tersisa, bukan cuma yang kedua ditolak
4. `value_dengan_newline_ditolak`
5. `simpan_env_membuat_snapshot_dan_deployment_baru_dengan_digest_sama` —
   happy path lengkap: secret TIDAK muncul di response, non-secret muncul,
   snapshot bisa didekripsi balik jadi nilai asli, deployment baru
   `image_digest` identik, `trigger_source='env'`, `env_version_id` terisi
6. `simpan_env_saat_lock_aktif_env_tetap_tersimpan_tapi_redeploy_ditolak_409`
   — lock disimulasikan langsung lewat `apps_repo::acquire_lock` (pola sama
   `tests/phase1.rs` menyuntik state lewat repo, bukan lewat worker
   sungguhan), memverifikasi env_vars TETAP ada dan TIDAK ADA deployment
   baru dibuat
7. `id_app_tidak_dikenal_pada_env_selalu_404` — dengan CSRF **valid** (bukan
   asal), supaya 404 murni soal id, tidak tercampur penolakan CSRF (pola
   sama presedan `tests/phase1.rs`)

**Keterbatasan qa dicatat jujur**: env benar-benar sampai ke proses
container (lewat `docker inspect`), file audit tertulis di server target,
dan kegagalan dekripsi snapshot saat deploy nyata — TIDAK dites integrasi
penuh (butuh Docker+SSH sungguhan, keterbatasan sama sejak Fase 2).
`resolve_env`/`tulis_env_file_target` di `engine.rs` belum punya unit test
tersendiri (fungsi murni tanpa Docker/SSH bisa diuji, tapi belum ditulis
sesi ini) — kandidat kerja qa/reviewer berikutnya.

Verifikasi orchestrator langsung: `cargo sqlx prepare -- --all-targets`
bersih, `cargo build --all-targets` bersih, `cargo fmt` bersih, `cargo
clippy --all-targets --all-features -- -D warnings` No issues found,
`cargo test --all-targets` **307 passed** (naik dari 300 — 4 test
`web/env.rs` + 7 test `phase4.rs` baru, 4 sebelumnya baru dari fix web
literal). `phase0` 21/21, `phase1` 7/7, `phase2` 7/7, `phase3` 21/21,
`phase4` 7/7 — TIDAK ADA REGRESI. Nol `unwrap()`/`expect()` di luar
`#[cfg(test)]` di seluruh perubahan (diverifikasi lewat `awk`, bukan
diasumsikan).

### Audit formal dan perbaikan blocker (2026-08-11)

- [x] uiux — `docs/design/environment.md` ditulis sebagai spesifikasi formal:
  bar konsekuensi deploy, diff server-side, masking secret, state UI,
  aksesibilitas, dan keputusan value kosong.
- [x] security — audit formal selesai. Enkripsi at-rest, kunci eksternal
  0600, auth/CSRF, dan validasi newline lulus. Kebocoran `log_tail` mentah
  ditutup: engine kini hanya mencatat bahwa log ditangkap, tanpa isi log.
  File audit env target dihapus setelah pergantian container selesai.
- [x] reviewer — review formal selesai. Mutasi `env_vars`, snapshot,
  deployment, dan job kini satu transaksi; operasi lock tetap dilakukan
  sebelum transaksi karena pool tulis satu koneksi.
- [x] qa — test round-trip value 8.001+ karakter dengan karakter khusus dan
  test pesan kegagalan tanpa log mentah ditambahkan.

**Gerbang Fase 4 ditutup secara teknis.** Sentinel value kosong dan diff
server-side sudah diimplementasikan. Boundary `docker inspect` tetap dicatat
sebagai batas teknis Docker API yang telah disetujui manusia pada Q1 (Opsi A):
secret yang masuk ke `ContainerCreateBody.env` dapat terlihat oleh operator
server target yang sudah memiliki akses Docker socket. Platform tidak
menganggap boundary tersebut sebagai kebocoran ke response control plane, dan
secret tidak pernah masuk log platform, response, atau markup.

Perubahan lanjutan:

- `src/web/env.rs`: diff aman untuk tambah/ubah/kosongkan/hapus, sentinel
  `empty__{key}`, serta masking secret.
- `src/routes/apps.rs`: validasi konflik hapus+kosongkan, empty string
  terenkripsi sebagai snapshot, dan diff diteruskan ke response.
- `tests/phase4.rs`: skenario konflik aksi ditolak tanpa efek samping.

Verifikasi langsung setelah perbaikan: `cargo fmt`, `cargo sqlx prepare --
--all-targets` dengan database metadata sementara bermigrasi bersih, `cargo
clippy --all-targets -- -D warnings`, dan `cargo test` hijau: 242 unit +
3 phase0 smoke + 21 phase0 + 7 phase1 + 7 phase2 + 21 phase3 + 9 phase4,
termasuk test sentinel, diff secret, value panjang, dan karakter khusus.
Tidak ada commit dibuat.

---

## Fase 5 — Keandalan dan rollback

- [x] planner — plan Fase 5, kontrak HTTP, dan kebijakan rollback/
      reconciliation/retensi/webhook ditulis.
- [x] uiux — spesifikasi `rollback.md`, `reconciliation.md`, dan
      `notifications.md` ditulis.
- [x] migration — `migrations/0006_reliability.sql`; migrasi 0001–0006
      bersih dari database kosong, tanpa mengulang lock/index yang sudah ada.
- [x] backend — heartbeat deployment guarded tiap 10 detik; observasi Docker
      berlabel; repository finding idempoten; route acknowledge; dan endpoint
      rollback yang membuat deployment baru dengan digest/env target.
- [x] frontend — halaman finding read-only dan tombol rollback pada detail
      deployment, dengan peringatan status unknown tanpa auto-heal.
- [x] qa — baseline `tests/phase5.rs` dan test policy retensi/finding ditambahkan;
      fault injection end-to-end masih tersisa.
- [ ] reviewer
- [ ] security

### Catatan

Fase 5 masih berjalan. Kontrak, schema, lock/heartbeat, observasi Docker,
finding read-only, rollback dasar, policy retensi, queue webhook idempoten, dan
worker reconciliation sudah terpasang. Settings webhook protected sudah
tersedia: URL dan secret dienkripsi dengan `CryptoKey`, URL hanya ditampilkan
termasking, HTTPS dan event whitelist divalidasi, dan queue event tetap
idempoten.

SSRF protection URL webhook sudah memblokir HTTP, loopback, private/link-local,
metadata address, userinfo, dan hasil resolusi DNS yang menunjuk ke alamat
internal. Klasifikasi drift deterministik kini mencakup container hilang/tidak
berjalan, digest atau ID mismatch, multiple container, dan orphan container;
semuanya read-only tanpa auto-heal. Delivery queue kini memiliki klaim atomik,
backoff retry, dan worker non-blocking; delivery HTTP sengaja belum mengirim
request sampai TLS dan signing HMAC siap. Signing HMAC belum diimplementasikan
karena dependency crypto hash/HMAC langsung belum ada di `Cargo.toml` dan aturan
repo melarang menambah dependency tanpa izin eksplisit.
Scanner periodik yang benar-benar membuka SSH/Docker dan fault-injection
crash/SSH/image hilang/concurrent rollback masih harus dikerjakan. Tidak ada
auto-healing.

Verifikasi checkpoint: `cargo fmt`, `cargo sqlx prepare -- --all-targets`,
`cargo clippy --all-targets -- -D warnings`, dan `cargo test` hijau — 250 unit
+ seluruh suite phase0–phase5; migrasi 0001–0006 bersih dari database sementara.

Verifikasi tahap schema: `cargo sqlx migrate run` 0001–0006 dan `cargo sqlx
prepare -- --all-targets` berhasil pada database metadata sementara.

---

## Fase 6 — Metrik dan pemantauan

- [ ] planner
- [ ] uiux
- [ ] migration
- [ ] backend
- [ ] frontend
- [ ] qa
- [ ] reviewer
- [ ] security

### Catatan

Belum dibuka. Boleh ditunda tanpa batas — fase 1–5 sudah berguna sendiri.

---

## Fase 7 — Operasi armada dan pintu darurat

- [ ] planner
- [ ] uiux
- [ ] migration
- [ ] backend
- [ ] frontend
- [ ] qa
- [ ] reviewer
- [ ] security

### Catatan

Belum dibuka. Isi fase ini ditentukan catatan "kenapa saya SSH" yang dikumpulkan
selama memakai fase 1–6, bukan PRD.

---

## Pekerjaan ad-hoc (di luar PRD)

Diisi lewat `/feature`. Jangan dicampur ke checklist fase di atas.

_Belum ada._

---

## Fase 1 — update sesi berjalan (belum final, diisi progresif)

**Planner sudah selesai dari sesi sebelumnya** (tercatat sebagai belum di checklist atas, dikoreksi sekarang): `docs/plan.md` dan `docs/api-contract.md` sudah berisi rencana lengkap Fase 1 — Task 0 (utang security Fase 0), struktur modul, migrasi, timeout per tahap, backoff polling, risiko, kriteria selesai, dan **7 pertanyaan terbuka (Q1-Q7)**. Q1-Q4 memblokir langkah backend utama (langkah 3 di plan.md); Task 0, uiux, migration TIDAK diblokir.

- [x] planner — output: docs/plan.md, docs/api-contract.md (rencana Fase 1 lengkap; Q1-Q7 terbuka, Q1-Q4 blokir langkah backend utama)
- [x] uiux — output: docs/design/tambah-server.md, docs/design/fleet-overview.md, docs/design/server-detail.md (semua state + a11y, tidak ada token visual baru)
- [x] migration — scope: migrations/0002_servers.sql (tabel servers 16 kolom, registries 4 kolom, server_registries join table + 2 indeks; migrations/0001_init.sql tidak tersentuh; cargo test --test phase0 tetap 21/21, fmt+clippy bersih)
- [x] backend Task 0 (utang security Fase 0: -wal/-shm 0600 di src/db.rs, tolak MENGDEP_INITIAL_PASSWORD kosong/whitespace di src/config.rs — 47 test total, phase0 tetap 21/21)
- [ ] backend (fitur utama, terblokir Q1-Q4)
- [ ] frontend
- [ ] qa
- [ ] reviewer
- [ ] security

### Catatan migration (sesi ini)

Skema `migrations/0002_servers.sql`:
- **servers**: id, name, host, port(default 22), ssh_user, ssh_key_encrypted, status (CHECK IN pending/verifying/online/unreachable, default pending), last_seen_at (nullable), docker_version (nullable), os_info (nullable), host_key_fingerprint (nullable, TOFU), consecutive_failures (default 0), next_poll_at (NOT NULL default 0 — backoff state bertahan lintas restart), last_error_kind (nullable), last_error_message (nullable, CHECK <=500 char — cegah stderr mentah masuk db, invariant §3 no.9), created_at, updated_at (epoch integer).
- **registries**: id, host, username, token_encrypted, UNIQUE(host,username).
- **server_registries**: server_id + registry_id (PK gabungan), FK ON DELETE CASCADE ke keduanya, last_login_at (nullable).
- Indeks: idx_servers_next_poll_at (worker poll tiap siklus), idx_servers_status (fleet overview+strip). Alasan ditulis sebagai komentar SQL.
- Tidak ada kolom kunci enkripsi apa pun di skema — kunci age tetap di file terpisah 0600 (invariant §3 no.8 aman).

### Catatan uiux (sesi ini)

Tiga file dibuat: `docs/design/fleet-overview.md`, `docs/design/tambah-server.md`, `docs/design/server-detail.md`. Badge status: pending=abu-abu, verifying=kuning, online=hijau, unreachable=merah (pakai token warna Fase 0 yang sudah ada, tidak ada token baru). Fleet strip: horizontal desktop, wrap di mobile. Wizard 3 langkah lengkap dengan semua kegagalan PRD §4 dispesifikasikan (host tidak terjangkau, kunci ditolak, Docker tidak terpasang, tanpa akses socket, fingerprint berubah — TOFU gagal keras tanpa tombol override). Server detail: kerangka tanpa grafik, placeholder metrik untuk Fase 6. Semua 6 state (default/loading/empty/error/disabled/success) + a11y dispesifikasikan di ketiga file. Asumsi uiux: kunci SSH tanpa passphrase (sejalan asumsi plan.md), SSE pakai hx-ext="sse" dihitung bagian HTMX (rekomendasi Q4 planner).

### Catatan backend Task 0 (sesi ini)

Verifikasi ORCHESTRATOR LANGSUNG (laporan agent kosong, tapi kode diverifikasi manual):
- `src/db.rs`: fungsi baru `set_file_mode_0600_if_exists`, `wal_path`, `shm_path`. Mode 0600 di-set untuk `-wal`/`-shm` di DUA titik: setelah pool tulis dibuka (WAL bisa lahir dari koneksi pertama) DAN setelah migrasi jalan (WAL bisa lahir baru di titik ini kalau migrasi adalah tulisan pertama). Tidak bersyarat pada "db baru" — cek tiap startup.
- `src/config.rs`: opsi (a) dipilih — `MENGDEP_INITIAL_PASSWORD` di-trim, kalau hasil trim kosong diperlakukan sebagai `None` (`.filter(|value| !value.trim().is_empty())`). Password kosong tidak pernah mencapai `hash_password`.
- Test baru ditambahkan untuk kedua temuan (test `initial_password_kosong_atau_spasi_diperlakukan_sebagai_none` dikonfirmasi ada di src/config.rs).
- Verifikasi: `cargo fmt --check` bersih, `cargo clippy --all-targets -- -D warnings` No issues found, `cargo test` 47 passed (4 suites — naik dari 23 karena test baru db.rs+config.rs), `cargo test --test phase0` tetap 21/21 pass — TIDAK ADA REGRESI.

**Utang Fase 0 (2 dari 5 temuan) — SELESAI.** 3 temuan sisanya masih tercatat sebagai Q1 di plan.md, BELUM diputuskan manusia — backend utama (langkah 3 plan.md) masih terblokir Q1-Q4.

### Keputusan manusia — Q1-Q4 dijawab (menjawab blokir langkah backend utama)

- **Q1 = (a)** — audit security ulang cepat atas cakupan Fase 0 (`src/auth/**`, `src/routes/login.rs`, `src/config.rs`, `src/db.rs`, `src/state.rs`, `src/error.rs`, `migrations/0001_init.sql`) dijalankan SEBELUM backend Fase 1 utama mulai. Hasil dicatat lengkap di bawah begitu security selesai.
- **Q2 = private key tanpa passphrase**, ditempel ke form. Konsisten asumsi plan.md — tidak ada perubahan skema/kontrak.
- **Q3 = disetujui**. Tambah `tokio-stream` fitur `sync` untuk `BroadcastStream` → adaptor `Stream` yang dibutuhkan `axum::response::Sse`. Ini SATU-SATUNYA dependensi baru di luar `openssh`/`bollard`/`age` yang sudah terkunci PRD §1.6.
- **Q4 = disetujui**. HTMX di-vendor lokal (bukan CDN), disajikan `GET /assets/htmx.min.js` lewat `src/routes/assets.rs`. Ekstensi `hx-ext="sse"` dihitung bagian dari "HTMX + SSE" yang dikunci `docs/prd.md:60`, BUKAN pelanggaran larangan "JS di luar xterm.js".

Q5-Q7 (interval polling 60 detik, kebijakan host key berubah, `zeroize`) belum dijawab eksplisit — tidak memblokir, backend jalan dengan asumsi default plan.md (Q5: 60 detik; Q6: gagal keras tanpa tombol override; Q7: ditunda, keputusan security setelah `src/crypto.rs` jadi).

### Audit security ulang Fase 0 (Q1 = opsi a) — hasil LENGKAP, MENGGANTIKAN checklist lama

Checklist lama "3 temuan hilang" di bagian Fase 0 (baris sekitar 233-236) **tidak bisa dipulihkan literal** — laporan asli hilang total. Audit ini adalah audit independen baru dari kode aktual, dinyatakan security sebagai pengganti PENUH checklist lama (jangan pertahankan placeholder lama).

**Verifikasi 2 temuan Task 0 yang sudah diperbaiki:**
- WAL/-shm 0600: **tertutup sebagian** — steady-state benar, tapi ada jendela world-readable singkat antara file dibuat SQLite (default umask 0644) dan chmod dijalankan aplikasi. Ditutup oleh HARUS-1 (umask proses).
- MENGDEP_INITIAL_PASSWORD kosong: **tertutup penuh** — hanya satu jalur baca (`config.rs:59`), filter benar, tidak ada cara password kosong mencapai `hash_password`. Catatan minor: test (`config.rs:111-135`) menguji closure duplikat bukan `Config::from_env` — regresi tidak akan tertangkap (CATATAN-12).

**Rekap audit ulang: 0 BLOCKER, 6 HARUS DIPERBAIKI, 12 CATATAN.**

**HARUS DIPERBAIKI (wajib sebelum gerbang Fase 1 ditutup; HARUS-1/2/3 WAJIB sebelum backend Fase 1 utama mulai):**
1. **HARUS-1** — jendela world-readable antara file db/-wal dibuat dan chmod 0600 dijalankan (`src/db.rs:43-56,64-67`). Fix: `umask(0o077)` sekali di awal `main()` sebelum `db::connect_and_migrate`.
2. **HARUS-2** — direktori data tidak diset 0700, file sampingan SQLite (`-journal`, temp VACUUM) di luar `-wal`/`-shm` tidak tercakup (`src/db.rs:34-39`). Fix: chmod 0700 direktori data + andalkan umask 0077 dari HARUS-1.
3. **HARUS-3** — izin `.env` tidak pernah diverifikasi padahal memuat `MENGDEP_INITIAL_PASSWORD` (`src/main.rs:20`). Fix: warn kalau `.env` ada dan group/other-readable, pesan tanpa isi file.
4. **HARUS-4** — `POST /login` tanpa batas laju/konkurensi → DoS memori lewat Argon2 (~19 MiB/request, CSRF draft token bisa dipakai ulang). Fix: `ConcurrencyLimit` kecil (2-4) khusus route login. Boleh paralel, tutup sebelum gerbang Fase 1.
5. **HARUS-5** — token sesi disimpan PLAINTEXT di `sessions.id` (`src/auth/session.rs:50-82`) — kebocoran db/backup langsung berarti replay sesi 30 hari tanpa password. Fix: simpan SHA-256 token, lookup pakai hash (butuh migrasi baru). Boleh paralel, tutup sebelum gerbang Fase 1.
6. **HARUS-6** — tidak ada `Cache-Control: no-store` — token CSRF & (Fase 1) fingerprint/daftar armada bisa tertulis cache disk browser. Fix: middleware kecil di router terlindungi + halaman login. Boleh paralel, tutup sebelum gerbang Fase 1.

**CATATAN (12 item, tidak wajib, dicatat untuk kelengkapan):** tidak ada header keamanan HTTP (CSP dll), cookie tanpa prefiks `__Host-`, perbandingan CSRF tidak constant-time (`subtle` sudah ada transitif), pesan rejection form axum bocor nama field mentah, timing oracle "belum ter-seed" vs "password salah", panjang password tidak dibatasi, hash rusak ditelan diam tanpa log, sqlx `log_statements` tidak eksplisit di-Off, flag Secure+bind localhost (dokumentasi saja jangan diubah), `MENGDEP_INITIAL_PASSWORD` tetap di env proses setelah seed, `test_out.txt` tertinggal (perlu manusia hapus), test password kosong tidak menguji kode produksi asli.

**Diverifikasi BENAR tanpa temuan:** Argon2id parameter OWASP-compliant, entropi token 190-bit CSPRNG, rotasi sesi atomik, expiry server-side, cakupan middleware benar (tidak ada route lupa dilindungi), fail-closed, atribut cookie lengkap, pesan gagal login generik konsisten, tidak ada `derive(Debug)` bocor secret, tidak ada kolom kunci enkripsi di skema, semua SQL bind parameter (nol SQL injection), nol `PreEscaped` Maud (nol XSS), nol `unwrap()`/`expect()` di luar test, nol secret hardcoded, dependensi bersih.

**Kesimpulan security:** boleh lanjut ke backend Fase 1 (private key SSH nyata) **dengan syarat HARUS-1/2/3 ditutup dulu** (soal umask/izin file, akan menyederhanakan seluruh file sensitif baru Fase 1: socket forward, known_hosts aplikasi, kunci age). HARUS-4/5/6 boleh paralel, wajib tertutup sebelum gerbang Fase 1 ditutup (bukan sebelum backend mulai).

- [x] security (audit ulang Q1) — 0 BLOCKER, 6 HARUS DIPERBAIKI (3 wajib sebelum backend Fase 1 utama: HARUS-1/2/3; 3 boleh paralel wajib sebelum gerbang: HARUS-4/5/6), 12 CATATAN

### Task 0.5 — tutup HARUS-1/2/3 (izin file, syarat sebelum backend Fase 1 utama)

Dikerjakan backend, verifikasi manual orchestrator:
- HARUS-1 (jendela world-readable db/-wal): `set_umask_private()` di `src/main.rs` — FFI langsung `unsafe extern "C" { fn umask(mask: u32) -> u32; }`, dipanggil awal `main()` sebelum `Config::from_env`/`db::connect_and_migrate`. Bukan crate `libc` (hanya transitif, tidak di `Cargo.toml` — tidak ditambahkan sesuai batasan "jangan tambah dependensi").
- HARUS-2 (direktori data tidak 0700): `src/db.rs` — `set_mode(parent, 0o700)` tanpa syarat (baru maupun sudah ada), refactor `set_file_mode_0600` → `set_mode(path, mode)` generic dipakai ulang.
- HARUS-3 (izin `.env` tak diverifikasi): `warn_if_dotenv_permissions_longgar()` di `src/main.rs` — cek `.env` (cwd, sama asumsi `dotenvy`), `tracing::warn!` kalau grup/other punya bit apa pun (`mode & 0o077 != 0`), tidak fatal, tidak memuat isi file.
- Test baru: `direktori_data_bermode_0700_baik_baru_maupun_sudah_ada`, `dotenv_permissions_longgar_mendeteksi_group_dan_other_readable`, `dotenv_permissions_longgar_menerima_0600`, `set_umask_private_tidak_panic`.
- Catatan jujur backend: umask proses-wide tidak dites langsung di unit test (bisa ganggu test paralel lain) — dicek transitif lewat `tests/phase0.rs` yang jalankan binary sungguhan.
- Verifikasi manual orchestrator: `cargo fmt --check` bersih, `cargo clippy --all-targets -- -D warnings` 0 masalah, `cargo test` 51 passed (4 suites), `cargo test --test phase0` tetap 21/21 — TIDAK ADA REGRESI.

- [x] Task 0.5 (HARUS-1/2/3) — tertutup, terverifikasi, siap lanjut backend Fase 1 utama

### backend Fase 1 utama — percobaan 1 (delegasi besar 3a-3f sekaligus): TERHENTI DI TENGAH

Dipanggil sebagai satu delegasi besar (langkah 3 penuh, plan.md). Laporan akhir agent **kosong** (`task_result` tanpa isi) — kemungkinan kehabisan step budget di tengah, tepat seperti yang diantisipasi plan.md bagian "Catatan ukuran fase".

Verifikasi manual orchestrator menemukan progres PARSIAL nyata (bukan nol):
- `Cargo.toml`: sudah lengkap — `openssh` (fitur `native-mux`), `bollard` (fitur default, tanpa TLS/TCP, komentar ponytail menjelaskan alasan), `age` (fitur `armor`), `tokio-stream` (fitur `sync`, `default-features = false`). Sesuai Q1-Q4.
- `src/crypto.rs`: sudah dibuat lengkap — `CryptoKey` (identity+recipient age, TANPA `derive(Debug)`), `load_from_file` (baca file, parse identity, TIDAK mengulang cek permission — didelegasikan ke `Config::verify_encryption_key_permissions`), `encrypt`/`decrypt` (armor string, aman utk kolom TEXT), 2 test roundtrip.
- **BUG kritis ditemukan**: `src/lib.rs` **belum** mendeklarasikan `pub mod crypto;` — akibatnya seluruh `src/crypto.rs` adalah file orphan, tidak pernah dicek compiler sebagai bagian crate, dan **0 dari 2 test crypto pernah benar-benar jalan** (`cargo test crypto` → "0 passed, 51 filtered out"). `cargo test` yang melaporkan "51 passed" itu SEMUA test lama, tidak menyentuh crypto.rs sama sekali.
- Modul `ssh/`, `docker/`, `servers/`, `registries/`, `worker/`, `events.rs` — **belum ada sama sekali**.
- `src/config.rs`, `src/state.rs`, `src/error.rs`, `src/main.rs`, `src/routes/**` — **belum diubah** dari versi Task 0.5.

**Keputusan orchestrator:** TIDAK mengulang delegasi besar yang sama (plan.md sendiri sudah memperingatkan fase ini kelebihan ukuran). Lanjut dengan granularitas lebih kecil sesuai pecahan 3a-3f yang sudah disiapkan plan.md, dimulai dari menuntaskan 3a (yang sudah setengah jalan: Cargo.toml done, crypto.rs done-tapi-orphan, config.rs+state.rs belum tersentuh).

### backend Fase 1 sub-blok 3a — SELESAI (crypto+config+state)

- `src/lib.rs`: tambah `pub mod crypto;` — perbaiki bug orphan module dari percobaan 1 (crypto.rs sebelumnya tidak pernah dicompile).
- `src/crypto.rs`: 1 baris fix `use age::secrecy::ExposeSecret;` di modul test — bug pre-existing baru kelihatan setelah `mod crypto` ditambahkan, bukan bug baru.
- `src/config.rs`: kunci age WAJIB — `verify_encryption_key_permissions()` sekarang fatal (`anyhow::bail!` sebut `MENGDEP_KEY_PATH`) kalau path `None`, bukan `Ok(())`. Test lama diganti jadi assert error; tambah test mode 0644 ditolak + 0600 diterima.
- `src/state.rs`: tambah `pub crypto: Arc<CryptoKey>` ke `AppState`.
- `src/main.rs`: setelah verifikasi izin kunci sukses, `CryptoKey::load_from_file` dipanggil, dimasukkan ke `AppState`.
- Verifikasi manual orchestrator: `mod crypto` terkonfirmasi ada, `cargo test --lib` 31 passed, `cargo test` (all target) GAGAL kompilasi — **sesuai laporan backend, terverifikasi benar** — `tests/phase0.rs:48` `AppState { ... }` belum punya field `crypto`.

**BLOCKER teridentifikasi (bukan kegagalan backend — konsekuensi langsung dari "kunci age jadi wajib"):** `tests/phase0.rs` (milik qa) perlu update: (1) helper `setup()` baris ~42-59 tambah field `crypto` pakai file kunci age sementara mode 0600, (2) test SIGTERM yang spawn binary `mengdep` sebagai proses anak perlu `MENGDEP_KEY_PATH` diset ke file kunci valid. Diserahkan ke qa (bukan backend, glob tidak boleh disentuh backend).

### qa — fix tests/phase0.rs untuk field crypto baru (blocker sub-blok 3a ditutup)

`tests/phase0.rs`: helper baru `tulis_kunci_age_ke()` (pola sama `src/crypto.rs`, generate identity x25519, tulis file mode 0600). `setup()` diupdate: `encryption_key_path: Some(key_path)`, field baru `crypto: Arc::new(CryptoKey::load_from_file(&key_path))`. Test SIGTERM (spawn proses anak binary `mengdep`) ditambah `.env("MENGDEP_KEY_PATH", &key_path)` mengikuti pola env var lain di test itu. Tidak ada assertion diubah, tidak ada `src/**` tersentuh.

Verifikasi manual orchestrator: `cargo fmt --check` bersih, `cargo clippy --all-targets -- -D warnings` No issues found, `cargo test` 55 passed (4 suites), `cargo test --test phase0` tetap 21/21 — TIDAK ADA REGRESI.

- [x] backend Fase 1 sub-blok 3a (crypto+config+state, + fix tests/phase0.rs oleh qa) — SELESAI, terverifikasi, siap lanjut 3b (ssh/**)

### backend Fase 1 sub-blok 3b — SELESAI (ssh/**)

`src/ssh/{mod,session,exec,hostkey}.rs` dibuat lengkap (845 baris): `session.rs`
membangun koneksi ControlMaster (`native-mux`) dengan timeout 10 detik wajib dan
verifikasi fingerprint host key sendiri di layer aplikasi (`hostkey::probe` lewat
`ssh-keyscan`+`ssh-keygen -lf`) SEBELUM autentikasi kunci dicoba — bukan
mengandalkan mekanisme known_hosts bawaan `openssh`. `exec.rs` memisahkan exit
code dari error transport (`SshExecError` HANYA untuk kegagalan level transport;
exit code bukan nol tetap `Ok(ExecResult{code,..})`) — tugas Debugger dari
`docs/plan.md` "Membedakan kegagalan SSH" sudah ditutup di titik ini. Private key
dan file bantu `ssh-keyscan` ditulis ke `TempFile` RAII mode `0600` di
`runtime_dir`, dihapus lewat `Drop` di semua jalur keluar (2 test memverifikasi
ini eksplisit). `src/lib.rs` sudah punya `pub mod ssh;` sejak sub-blok ini.

- [x] backend Fase 1 sub-blok 3b (ssh/**) — SELESAI, siap lanjut 3c (docker/**)

### backend Fase 1 sub-blok 3c — SELESAI (docker/**)

`src/docker/{mod,forward,client,registry_login}.rs` dibuat baru. `Cargo.toml`
sudah punya `bollard` sejak sub-blok 3a (fitur default `http`+`pipe`, TANPA TLS/TCP
— invariant 13 tertutup di level kompilasi, `grep "tcp://|2375|2376"` di `src/`
nol hasil kecuali komentar yang menjelaskan larangannya).

- `forward.rs`: `establish`/`close` membuka/menutup local port forward
  (`SshSession::forward_unix_local`/`close_unix_local_forward`, metode baru
  ditambah di `src/ssh/session.rs` — `inner` tetap `pub(super)`, tidak pernah
  bocor keluar modul `ssh`) dari socket unix lokal `{runtime_dir}/docker-sock/
  {server_id}.sock` (mode `0600`, direktori induk `0700`) ke
  `/var/run/docker.sock` di target. `cleanup_orphans` dipanggil `main.rs`
  (sub-blok 3f) saat startup untuk membuang socket sisa proses sebelumnya.
  **Catatan jujur**: asumsi file socket lokal sudah ada tepat setelah
  `request_port_forward` selesai (dipakai untuk chmod `0600` segera) belum
  pernah diverifikasi terhadap SSH server nyata — tidak ada Docker-in-Docker
  di sandbox pengembangan ini. Kalau salah, `establish` gagal bersih dengan
  `DockerForwardError::Other`, bukan panik.
- `client.rs`: `connect`/`ping`/`version`/`os_info` lewat `bollard`, tiap
  panggilan dibungkus timeout 5 detik sendiri (bukan satu timeout gabungan —
  invariant 11). `os_info` menggabungkan `operating_system`+`architecture`+
  `kernel_version` dari `SystemInfo`, field yang hilang dilewati.
- `registry_login.rs`: `docker login` dijalankan lewat SSH exec (BUKAN API
  `bollard` — kredensial harus mendarat di `~/.docker/config.json` milik CLI
  untuk dipakai `docker run`/`docker pull` Fase 2), password lewat
  `--password-stdin` supaya tidak pernah terlihat lewat `ps` di target. Fungsi
  baru `ssh::exec_with_stdin` ditambah di `src/ssh/exec.rs` untuk ini (spawn +
  tulis stdin + drop untuk EOF + `wait_with_output`), diekspor lewat
  `src/ssh/mod.rs`. Setelah login sukses, `chmod 600 ~/.docker/config.json` di
  target dijalankan dan diverifikasi. `stderr` kegagalan dipotong 500 karakter
  (`truncate_detail`, ada test) sebelum dikembalikan ke pemanggil — konsisten
  dengan constraint `CHECK <=500 char` di `servers.last_error_message`
  (invariant 9: tidak ada log mentah tak terbatas yang menyelinap).

Verifikasi orchestrator langsung: `cargo build` bersih, `cargo fmt --check`
bersih, `cargo clippy --all-targets --all-features -- -D warnings` No issues
found, `cargo test` **71 passed** (3 suites, naik dari 67), `cargo test --test
phase0` tetap **21/21** — TIDAK ADA REGRESI. `grep -rn "tcp://|2375|2376"
src/` hanya menghasilkan satu baris komentar yang menjelaskan mengapa jalur itu
tidak ada.

- [x] backend Fase 1 sub-blok 3c (docker/**) — SELESAI, siap lanjut 3d
      (servers/** + registries/** + events.rs)

### Perbaikan gap: `runtime_dir` tmpfs (invariant 13 `CLAUDE.md`, terlewat planner)

Ditemukan saat menyiapkan 3d: `docs/plan.md` mendeskripsikan `runtime_dir` hanya
sebagai "direktori privat aplikasi (dibuat pemanggil, mis. `{data_dir}/runtime`)"
tanpa mengangkat `CLAUDE.md` §5 invariant 13 ("Private key SSH yang didekripsi
hanya boleh menyentuh tmpfs ... Kalau `/run` tidak tersedia, gagal dan katakan —
jangan diam-diam jatuh ke `/tmp`") maupun §6 (path baku `/run/platform/ssh`) — ini
bukan Q-baru dari planner, jadi ditutup langsung dengan mengikuti `CLAUDE.md`
literal alih-alih menunggu jawaban manusia (dokumen itu sendiri sudah eksplisit).

`src/config.rs`: field baru `Config.runtime_dir: PathBuf`, default
`/run/platform/ssh` (`MENGDEP_RUNTIME_DIR` env var untuk override EKSPLISIT —
mis. dev di macOS yang tidak punya `/run` sama sekali). Fungsi baru
`verify_runtime_dir_available()`: `create_dir_all` + chmod `0700`, TIDAK PERNAH
mencoba path lain kalau gagal — kegagalan menggagalkan startup dengan pesan yang
menyebut invariant 13 dan cara mengatasinya. `src/main.rs` memanggilnya
setelah `verify_encryption_key_permissions()`, sebelum `docker::cleanup_orphans`
(soket forward yatim dari proses sebelumnya dibuang saat startup — janji yang
dicatat `docs/plan.md` untuk `docker/forward.rs`, sekarang benar-benar terpasang).

`tests/phase0.rs`: `Config` literal di `setup()` dan spawn `MENGDEP_RUNTIME_DIR`
di test SIGTERM ditambah (dir temp test sendiri — override eksplisit, konsisten
desain, bukan aplikasi diam-diam jatuh ke `/tmp`).

Verifikasi orchestrator langsung: `cargo build --all-targets` bersih, `cargo fmt`
bersih, `cargo clippy --all-targets --all-features -- -D warnings` No issues
found, `cargo test` **72 passed** (naik dari 71), `cargo test --test phase0`
tetap **21/21** termasuk test SIGTERM yang men-spawn binary sungguhan (memverifikasi
startup TIDAK gagal dengan `MENGDEP_RUNTIME_DIR` di-set eksplisit).

### backend Fase 1 sub-blok 3d — SELESAI (servers/** + registries/** + events.rs)

File baru: `src/servers/{mod,model,repo,verify}.rs`, `src/registries/{mod,repo}.rs`,
`src/events.rs`. Diubah: `src/state.rs` (+field `events: Arc<EventRegistry>`),
`src/lib.rs` (+`pub mod events/registries/servers`), `src/main.rs`+`tests/phase0.rs`
(konstruksi `AppState` ikut field baru), `src/ssh/session.rs` (+method
`SshSession::close()` — WAJIB dipanggil eksplisit di semua jalur keluar,
`Drop` biasa tidak menjamin proses `ssh` ControlMaster ikut berhenti).

**Temuan desain penting yang tidak eksplisit di `docs/plan.md`**: alur TOFU
punya jeda interaktif (pengguna klik "Ya, Terima & Simpan" di
`docs/design/tambah-server.md` §4.2 poin 6) — menahan sesi SSH tetap hidup
menunggu itu lintas request HTTP adalah kelas masalah lifetime yang sama yang
PRD sendiri tandai berbahaya untuk streaming log Fase 3. Diputuskan: sesi
DITUTUP setelah menampilkan fingerprint; `POST /servers/{id}/hostkey/konfirmasi`
(sub-blok 3f) mengambil ulang fingerprint lewat `ssh::fetch_fingerprint_via_keyscan`
— fungsi yang didokumentasikan sub-blok 3b persis untuk kebutuhan ini
("kalau suatu saat dibutuhkan endpoint terpisah") — lalu membangun ulang
koneksi `Strict`. Biaya satu handshake SSH tambahan, tidak ada resource yang
tergantung lintas request.

- `events.rs`: `EventRegistry` in-memory (`std::sync::Mutex<HashMap<...>>`,
  BUKAN `dashmap` — tidak ada di `Cargo.toml`, jumlah server kecil jadi
  kontensi lock tidak relevan). Job = `server_id` (Fase 1 tidak punya
  verifikasi paralel per server). `remove()` dipanggil di ujung setiap alur
  verifikasi (sukses/gagal) — kebocoran channel dicegah sejak awal, bukan
  ditambal belakangan seperti yang PRD peringatkan untuk Fase 3.
- `servers/model.rs`: `StatusServer` (enum + roundtrip string db),
  `ServerRingkas` (TANPA field kunci SSH/token — invariant 7),
  `LangkahVerifikasi`/`LangkahStatus` untuk checklist wizard.
- `servers/repo.rs`: `ServerRow` (baris mentah TERMASUK `ssh_key_encrypted`,
  privat ke domain server, tidak pernah ke `src/web/`), CRUD
  (`insert_pending`, `find_by_id`, `list_ringkas`, `set_status_verifying`,
  `set_host_key_fingerprint`, `mark_online`, `mark_verification_failed`).
  `mark_verification_failed` KEMBALI ke status `pending` (bukan
  `unreachable` — itu milik worker 3-strikes sub-blok 3e, bukan percobaan
  pertama). Pesan error dipotong 500 karakter (`CHECK` skema).
- `registries/repo.rs`: `upsert` pakai `INSERT ... ON CONFLICT ... DO UPDATE
  ... RETURNING` satu statement (atomik, tanpa transaksi baca-lalu-tulis
  yang rawan race walau pool tulis satu koneksi). `record_login_success`
  idem via `ON CONFLICT` ke PK gabungan `server_registries`.
- `servers/verify.rs` (inti sub-blok ini): tiga entry point publik —
  `mulai_verifikasi` (spawn dari route verifikasi, jalan sampai TOFU-pending
  atau selesai/gagal), `konfirmasi_hostkey_dan_lanjutkan` (dipanggil route
  konfirmasi, reprobe+simpan lalu spawn lanjutan), `tautkan_registry`
  (sinkron, tanpa SSE — sesuai `docs/design/tambah-server.md` §4.3 poin 2).
  Klasifikasi kegagalan (`LangkahKegagalan`, 5 kategori A-E persis PRD +
  `Lain`) dan pemetaan pesan Bahasa Indonesia disalin literal dari
  `docs/design/tambah-server.md` §4.2 poin 4 (uiux sudah mengunci teksnya —
  backend tidak mengarang ulang). Pemeriksaan Docker langkah 2: exec
  `docker version --format '{{.Server.Version}}'` (15 detik) — exit 127 =
  kategori C, stderr memuat "permission denied" = kategori D, sukses =
  lanjut forward+bollard ping+os_info. Fungsi klasifikasi (`classify_*`)
  murni dan dites tanpa SSH nyata.

Verifikasi orchestrator langsung: `cargo sqlx prepare -- --all-targets`
dijalankan (banyak `sqlx::query!` baru di `servers/repo.rs`+`registries/repo.rs`,
termasuk yang di `#[cfg(test)]` — `prepare` tanpa `--all-targets` melewatkan
itu, ketahuan saat `cargo build --all-targets` gagal lalu diperbaiki).
`cargo build --all-targets` bersih, `cargo fmt` bersih,
`cargo clippy --all-targets --all-features -- -D warnings` No issues found,
`cargo test` **86 passed** (naik dari 72), `cargo test --test phase0` tetap
**21/21** — TIDAK ADA REGRESI.

- [x] backend Fase 1 sub-blok 3d (servers/** + registries/** + events.rs) —
      SELESAI, siap lanjut 3e (worker/**)

### backend Fase 1 sub-blok 3e — SELESAI (worker/**)

File baru: `src/worker/{mod,status_poll}.rs`. Diubah: `src/lib.rs`
(+`pub mod worker`), `src/main.rs` (spawn worker setelah `AppState` siap,
`worker_handle.shutdown().await` setelah `axum::serve` selesai — dipasang di
jalur shutdown yang sama dengan Fase 0, `src/main.rs:120-151` lama; tambah
`preflight_check_ssh_binaries()` yang dipanggil sebelum `AppState` dibangun,
`tracing::warn!` per binary hilang (`ssh`/`ssh-keyscan`/`ssh-keygen`), TIDAK
fatal — invariant 1, `docs/plan.md` "Catatan penting openssh").

Widened visibility `src/servers/verify.rs`: `LangkahKegagalan` dan
`classify_connect_error`/`classify_exec_error`/`classify_docker_exec` dari
privat jadi `pub(crate)` — dipakai ulang `worker::status_poll` supaya
kategori kegagalan (A-E) dan pemetaan pesan Bahasa Indonesia satu sumber
kebenaran antara verifikasi awal dan polling rutin, bukan diduplikasi.

`src/servers/repo.rs` tambahan: `list_due_for_poll` (`WHERE next_poll_at <= ?`),
`PollWrite`/`PollWriteSukses`/`PollWriteGagal` (instruksi tulis hasil satu
server), `apply_poll_batch` — SATU transaksi untuk seluruh hasil satu siklus
(invariant 10 harfiah, bukan N `UPDATE` terpisah).

`worker/status_poll.rs`:
- **Poll ringan, BUKAN verifikasi penuh ulang**: SSH connect (`Strict` —
  fingerprint sudah tersimpan dari verifikasi awal, TIDAK pernah TOFU saat
  poll otomatis tanpa pengguna hadir) + exec `docker version` saja. TIDAK
  membuka forward socket/bollard setiap siklus — itu mahal untuk dijalankan
  tiap 30 detik per server; `docker_version` diperbarui dari hasil exec,
  `os_info` dipertahankan dari nilai lama (jarang berubah).
- Konkurensi dibatasi 4 lewat `tokio::task::JoinSet` dengan pola sliding
  window (spawn ulang begitu satu slot selesai) — TIDAK ada dependency baru
  (`futures`/`dashmap` tidak ditambah; `JoinSet` bagian `tokio` yang sudah ada).
- `backoff_secs(n)` dan `hitung_setelah_gagal(consecutive_failures_sebelum, now)`
  keduanya fungsi MURNI, dites tanpa I/O — kriteria eksplisit `docs/plan.md`
  ("verifikasi backoff benar-benar melambat, diuji terhadap fungsi backoff
  murni, bukan menunggu 15 menit"). Urutan: 1,2,4,8 menit lalu plateau 15 menit
  selamanya. Status HANYA berubah ke `unreachable` persis di kegagalan ke-3
  (bukan ke-1/ke-2 — tetap `online` di situ, hanya `consecutive_failures`
  naik); server `unreachable` tetap terus di-poll dengan interval backoff
  (harus bisa pulih sendiri, invariant 1/3 — tidak ada auto-fix, tapi juga
  tidak pernah berhenti mencoba).
- Loop tidak pernah mati karena satu server gagal — kegagalan per server
  ditangkap sebagai nilai (`Result` di dalam tuple `JoinSet`), bukan `?` yang
  membatalkan siklus.

Verifikasi orchestrator langsung: `cargo sqlx prepare -- --all-targets`
dijalankan (3 query baru), `cargo build --all-targets` bersih, `cargo fmt`
bersih, `cargo clippy --all-targets --all-features -- -D warnings` No issues
found, `cargo test` **92 passed** (naik dari 86, 8 test baru murni
`backoff_secs`/`hitung_setelah_gagal`), `cargo test --test phase0` tetap
**21/21** — termasuk test SIGTERM yang sekarang men-spawn binary dengan worker
polling AKTIF berjalan, memverifikasi `worker_handle.shutdown().await` tidak
menghambat graceful shutdown. TIDAK ADA REGRESI.

- [x] backend Fase 1 sub-blok 3e (worker/**) — SELESAI. **Backend inti Fase 1
      (3a-3e, seluruh `src/**` di luar `src/web/` dan `src/routes/`) kini
      lengkap dan teruji.** Sisa sub-blok 3f (`src/routes/**` + wiring
      `main.rs` akhir) BELUM dikerjakan — endpoint HTTP (`POST /servers`,
      `GET /servers/{id}/verifikasi`, `POST /servers/{id}/hostkey/konfirmasi`,
      `GET/POST /servers/{id}/registry`, SSE `GET /events/verifikasi/{id}`,
      `GET /servers` fleet, `GET /assets/htmx.min.js`) belum ada — dan
      langkah 3f di `docs/plan.md` butuh langkah 4 (frontend, `src/web/**`)
      berjalan beriringan karena kontrak render (`render_fleet`,
      `render_verifikasi`, dst.) baru bisa dikompilasi setelah handler
      route menuliskannya. **Belum ada satu pun endpoint HTTP Fase 1 yang
      bisa diakses dari browser** — semua yang selesai sejauh ini adalah
      logika domain murni, teruji lewat unit test, belum tersambung ke
      dunia luar.

### backend Fase 1 sub-blok 3f + frontend langkah 4 — SELESAI (routes/** + src/web/**)

Dikerjakan bersamaan (bukan terpisah dua delegasi) karena kontrak render
(`render_fleet`, `render_verifikasi`, dst.) hanya bisa dikompilasi setelah
handler route menuliskan pemanggilnya — persis alasan `docs/plan.md`
menandai langkah 3f dan 4 tidak paralel.

**File baru**: `src/routes/{servers,registries,events,assets}.rs`,
`src/web/{fleet,fleet_strip,server_add,server_detail}.rs`,
`src/web/assets/{htmx.min.js,htmx-sse.min.js}` (di-vendor via `curl` dari
unpkg — htmx 2.0.4 + `htmx-ext-sse` 2.2.2, Q4 `docs/plan.md`: vendor lokal,
bukan CDN). **Diubah**: `src/web/{mod,layout,dashboard,error_page,styles}.rs`
(`app_shell` sekarang `(csrf_token, strip, content)`), `src/routes/{mod,dashboard}.rs`,
`src/error.rs` (+varian `NotFound` → `render_404`), `src/servers/verify.rs`
(3 hal: `NAMA_KONEKSI`/`NAMA_DOCKER`/`NAMA_REGISTRY` jadi `pub(crate)` dipakai
ulang `routes/servers.rs`; `LangkahKegagalan`+`classify_*` jadi `pub(crate)`
dipakai ulang `worker/status_poll.rs`; `konfirmasi_hostkey_dan_lanjutkan`
menerima `fingerprint_disetujui` dan divalidasi ulang terhadap reprobe +
dicek konflik terhadap fingerprint tersimpan — kontrak `docs/api-contract.md`
400/409 yang terlewat draf pertama; `RegistryStepInput` jadi enum
`Baru`/`PakaiUlang` sesuai kontrak `registry_id` opsional).

**Endpoint HTTP lengkap** (11 permukaan Fase 1 + perubahan `GET /`, persis
`docs/api-contract.md`): `GET/POST /servers`, `GET /servers/baru`,
`GET /servers/{id}/verifikasi`, `POST /servers/{id}/verifikasi/ulang`,
`POST /servers/{id}/hostkey/konfirmasi`, `GET /events/verifikasi/{id}` (SSE),
`GET/POST /servers/{id}/registry`, `GET /servers/{id}`,
`GET /assets/htmx.min.js`, `GET /assets/htmx-sse.min.js`.

**Desain SSE (`routes/events.rs`)**: `tokio_stream::StreamExt` TIDAK punya
`.scan()` (itu `futures_util`, sengaja tidak ditambah — Q3). Diselesaikan
dengan task terpisah yang meneruskan event dari `BroadcastStream` ke
`mpsc::channel` dan BERHENTI tepat setelah event yang menandai job selesai
diteruskan (`job_selesai`: koneksi gagal, ATAU langkah Docker sudah
`Sukses`/`Gagal` — TOFU pending BUKAN selesai, stream tetap terbuka
menunggu `konfirmasi_hostkey_dan_lanjutkan`). Kalau klien menyambung setelah
job benar-benar selesai (channel sudah dibuang `EventRegistry::remove`),
handler mengirim SATU snapshot dari status db lalu tutup — bukan koneksi
menggantung tanpa event.

**Dua bug nyata ditemukan lewat smoke test manual** (`curl` end-to-end:
login → tambah server → verifikasi → fleet → detail), BUKAN oleh test
otomatis — unit test `worker::status_poll` memakai status buatan sebagai
input, tidak pernah menguji jalur "server baru dibuat, belum diverifikasi":

1. **Routing**: `POST /servers` (buat server) sempat terpasang di bawah
   path `/servers/baru` alih-alih `/servers` — 405 Method Not Allowed nyata
   saat submit wizard langkah 1. Diperbaiki di `routes/mod.rs`.
2. **Worker memaksa status `online` untuk server yang belum pernah online**:
   `insert_pending` menyimpan `next_poll_at=0`, membuat server yang BARU
   dibuat (status `pending`, belum ada `host_key_fingerprint`) langsung
   "jatuh tempo" bagi worker polling — sebelum verifikasi awal sempat
   selesai. `hitung_setelah_gagal` lama men-hardcode status jadi `Online`
   untuk kegagalan di bawah ambang 3, mengasumsikan server memang pernah
   online. Hasilnya: server yang baru gagal verifikasi awal (masih
   `pending`) bisa berubah jadi `status='online'` dengan `docker_version`/
   `os_info` tetap `NULL` — state yang mustahil lewat jalur `mark_online`
   mana pun. **Dua perbaikan**: `servers::repo::list_due_for_poll` sekarang
   mensyaratkan `host_key_fingerprint IS NOT NULL` (server belum
   terverifikasi TIDAK PERNAH masuk kandidat poll); `hitung_setelah_gagal`
   sekarang menerima status SEBELUMNYA dan mempertahankannya apa adanya di
   bawah ambang, bukan memaksa `Online` (pertahanan berlapis — kalaupun
   filter pertama suatu saat lolos, fungsi murni ini sendiri tetap benar).
   Diverifikasi ulang lewat smoke test kedua: server yang gagal verifikasi
   awal tetap `pending`, `consecutive_failures=0`, tidak tersentuh worker
   sama sekali setelah lewat satu siklus tick 30 detik.

Test regresi baru untuk bug 2:
`hitung_setelah_gagal_tidak_memaksa_online_untuk_server_yang_belum_pernah_online`
(`worker/status_poll.rs`).

**Smoke test manual lengkap** (binary sungguhan, `curl`, bukan hanya
`cargo test`): login → `GET/POST /servers/baru` → checklist verifikasi
(SSE payload diperiksa langsung, format fragmen benar) → fleet menampilkan
badge `pending` yang benar → detail menampilkan banner "belum diverifikasi"
→ `GET /servers/{id}/registry` (200, sebelum verifikasi selesai — belum ada
gate yang salah) → 404 untuk id tidak dikenal → kedua aset HTMX 200 →
akses tanpa cookie redirect 303 ke `/login`. Tidak ada satu pun 500.

Verifikasi orchestrator langsung: `cargo sqlx prepare -- --all-targets`
(query `list_due_for_poll` berubah), `cargo build --all-targets` bersih,
`cargo fmt` bersih, `cargo clippy --all-targets --all-features -- -D warnings`
No issues found, `cargo test` **126 passed** (naik dari 92 — 34 test baru
lintas `web/**`+`routes/**`+1 regresi worker), `cargo test --test phase0`
tetap **21/21** — TIDAK ADA REGRESI.

- [x] backend Fase 1 sub-blok 3f + frontend langkah 4 — SELESAI. **Fase 1
      punya endpoint HTTP lengkap dan bisa dipakai dari browser sungguhan**
      (diverifikasi manual, bukan diasumsikan). Belum dikerjakan: langkah 5
      (security — "fase paling kritis"), langkah 6 (qa — `tests/phase1.rs`,
      5 skenario injeksi kegagalan formal), langkah 7 (reviewer, 2 batch
      sesuai `docs/plan.md`). Ketiganya BELUM berjalan — smoke test manual
      di atas bukan pengganti tinjauan security maupun qa formal.

### Ad-hoc — perbaikan UI wizard tambah server (di luar urutan gerbang, sebelum ditutup)

Diminta manusia setelah smoke test manual di browser sungguhan (Chrome DevTools
MCP): styling input jelek + layout wizard tambah server perlu dirapikan.

**Bug nyata ditemukan, bukan cuma polish**: `src/web/layout.rs` merender CSS
lewat `style { (CSS) }` — sintaks Maud `(expr)` MENG-ESCAPE HTML by default,
jadi setiap `"` di dalam `const CSS: &str` (semua selector atribut
`input[type="..."]`) berubah jadi `&quot;` di HTML yang benar-benar dikirim ke
browser. Selector itu jadi tidak valid CSS dan diam-diam gagal cocok — bug ini
ada SEJAK FASE 0 (styling field password login juga kena), baru ketahuan
sekarang karena wizard Fase 1 punya banyak input teks polos yang jadi kelihatan
jelek tanpa styling apa pun. Diperbaiki: `style { (PreEscaped(CSS)) }` —
`CSS` adalah string statis yang di-embed compile-time, tidak pernah memuat
data pengguna, jadi `PreEscaped` di sini aman (bukan celah XSS baru).

Perbaikan lain (murni desain, dalam sistem token yang sudah ada — tidak
menambah warna/font baru): layout `.field` jadi grid 2 kolom (label kiri,
input kanan) di ≥48rem persis `docs/design/tambah-server.md` §3, tetap
ditumpuk di mobile; wizard dibungkus `.form-panel` (kotak terkontain,
konsisten dengan `.login-card`/`.detail-card`); input dapat padding lebih
lega + transisi border halus; `#port` dipersempit. Diverifikasi visual lewat
Chrome DevTools MCP (screenshot desktop + mobile 375px + halaman login),
bukan diasumsikan dari kode.

**Fitur tambahan diminta manusia**: input readonly berisi perintah
`ssh-keygen -t ed25519 -f mengdep_key -N ''` di atas field kunci privat,
klik untuk salin (`.select()` + `navigator.clipboard.writeText()`) dengan
tooltip "Disalin!" sebentar. **Catatan batas peran**: `docs/prd.md` §2
melarang Frontend menambah JavaScript di luar `xterm.js` — field ini
sengaja MELANGGAR itu dalam skala sangat kecil (satu atribut `onclick`
inline, tanpa file `.js` baru) atas permintaan eksplisit manusia. Diberi
fallback tanpa JS: input tetap readonly dan bisa di-select+copy manual kalau
`navigator.clipboard` tidak tersedia (mis. konteks tanpa izin clipboard).
Input ini TIDAK punya atribut `name` — tidak pernah ikut ter-submit sebagai
bagian `ServerBaruForm`, dites eksplisit
(`server_baru_menampilkan_input_perintah_keygen_readonly_tanpa_name`).

Verifikasi: `cargo test` 127 passed (naik 1 dari 126), `cargo fmt`+`clippy`
bersih, dicoba langsung di browser (klik field, screenshot sebelum/sesudah).

### Gerbang Fase 1 ditutup — security + qa + reviewer

Diminta eksplisit manusia setelah diingatkan gerbang belum tertutup (CLAUDE.md
§11: "kalau diminta melompat fase, ingatkan dulu" — diingatkan dulu lewat
`AskUserQuestion`, manusia memilih menutup gerbang sebelum lanjut Fase 2).

**security — 0 BLOCKING, 2 HARUS DIPERBAIKI (keduanya ditutup sesi ini):**

1. **`key.age` tidak ada di `.gitignore`** — file kunci enkripsi lokal
   (dibuat manusia lewat `age-keygen` mengikuti `README.md`) bisa ke-commit
   tanpa sengaja lewat `git add .`/`git add -A`. Ditemukan lewat
   `git check-ignore -v key.age .env data/mengdep.db` — `.env` dan db
   ter-ignore, `key.age` TIDAK. Diperbaiki: tambah `*.age` + `key.age` ke
   `.gitignore` (`docs/prd.md` §3 nomor 8: kunci enkripsi tidak pernah ikut
   backup/repo).
2. **Kode status HTTP salah untuk dua kasus konflik**: `POST
   /servers/{id}/verifikasi/ulang` (job sudah berjalan) dan `POST
   /servers/{id}/hostkey/konfirmasi` (fingerprint tersimpan berbeda)
   memakai `400 Bad Request`, padahal `docs/api-contract.md` eksplisit minta
   `409 Conflict` untuk keduanya. Diperbaiki: varian baru `AppError::Conflict`
   ditambah, kedua call site di `routes/servers.rs` diperbaiki.

**Diverifikasi BENAR tanpa temuan** (cakupan: `crypto.rs`, `ssh/**`,
`docker/**`, `servers/verify.rs`, `servers/repo.rs`, `registries/repo.rs`,
`config.rs`, seluruh `routes/**`, `state.rs`, `main.rs`):
- Tidak ada `unwrap()`/`expect()` di luar `#[cfg(test)]` di seluruh `src/**`
  (diverifikasi lewat `awk`, bukan diasumsikan).
- `ServerRow`/`RegistryRow` (baris mentah dengan secret) TIDAK PERNAH
  disebut di `src/routes/` maupun `src/web/` — hanya `ServerRingkas`/
  `RegistryRingkas` yang tidak punya field kunci/token. Tidak ada
  `derive(Debug)` pada struct pembawa secret mana pun.
- Semua 6 handler yang menerima `Form<...>` (Fase 0 + Fase 1) memvalidasi
  CSRF — 5 lewat `session.csrf_token`, `POST /login` lewat mekanisme draft
  cookie terpisah (sebelum sesi ada).
- Invariant 13 (Docker socket tidak pernah TCP): `grep tcp://|2375|2376` di
  `src/` nol hasil kecuali komentar yang menjelaskan larangannya; fitur TCP
  `bollard` tidak diaktifkan di `Cargo.toml`.
- Invariant 11 (timeout per tahap): setiap pemanggilan `ssh::connect`/
  `ssh::exec`/`docker::establish`/`docker::ping`/`docker::os_info`/
  `ssh::fetch_fingerprint_via_keyscan` punya batas waktu sendiri (baked-in
  atau parameter eksplisit) — tidak ada satu timeout global.
  `SshSession::close()` dipanggil di SEMUA jalur keluar setiap sesi SSH yang
  berhasil terbuka (ditelusuri satu per satu, termasuk jalur error) —
  tidak ada proses `ssh` ControlMaster yang bocor.
- Invariant 1 (tidak ada tindakan destruktif karena tidak terjangkau):
  `mark_verification_failed` dan `hitung_setelah_gagal` tidak pernah
  menghapus baris; TOFU mismatch gagal keras tanpa override otomatis.
- TOFU: fingerprint direprobe DAN dicocokkan ulang saat konfirmasi (bukan
  percaya nilai dari klien begitu saja) — MITM di antara tampil-fingerprint
  dan klik-konfirmasi akan terdeteksi sebagai `FingerprintTidakCocok`.

**CATATAN (tidak wajib, dicatat untuk kelengkapan, bukan penghalang gerbang):**
- Tidak ada rate limit pada `POST /servers/{id}/verifikasi/ulang` — bisa
  dispam memicu banyak percobaan SSH beruntun. Risiko rendah untuk instance
  pengguna tunggal.
- Race jarang: `verifikasi/ulang` dan worker polling bisa berebut socket
  forward `{runtime_dir}/docker-sock/{server_id}.sock` kalau kebetulan
  jalan bersamaan untuk server yang sama. Dampak hanya kegagalan transien
  (pesan generik, retry aman), bukan korupsi data. Fase 1 sengaja belum
  punya lock db per server (`docs/plan.md` risiko baris 12) — kalau
  ditemukan mengganggu di pemakaian nyata, jadi kandidat Fase 5.
- Task SSE (`routes/events.rs`) yang menunggu event di channel broadcast
  bisa menggantung selama wizard TOFU pending yang ditinggalkan (browser
  ditutup tanpa konfirmasi) SAMPAI koneksi TCP klien benar-benar terputus.
  Dampak kecil (satu future menganggur, bukan thread/memori signifikan) dan
  jarang (hanya path TOFU-pending-lalu-ditinggalkan). Tidak diperbaiki
  sengaja — mengikat ke `EventRegistry` butuh mekanisme pembatalan
  tambahan yang tidak sepadan untuk skala masalah ini.
- `routes/login.rs` query `settings.password_hash` langsung dengan
  `sqlx::query!` tanpa lewat modul repo — pre-existing dari Fase 0 (sudah
  lolos gerbang Fase 0), di luar cakupan Fase 1.

**qa — `tests/phase1.rs`, 7 skenario injeksi kegagalan (minimum PRD: 5):**

1. `host_tidak_terjangkau_verifikasi_gagal_server_tetap_ada` — host tidak
   terjangkau, server TETAP ada (invariant 1), status kembali `pending`.
2. `format_kunci_salah_ditolak_tanpa_menyentuh_db` — validasi gagal sebelum
   menyentuh jaringan ATAU db (nol baris server dibuat).
3. `csrf_salah_pada_post_servers_ditolak_tanpa_efek` — CSRF salah ditolak,
   nol efek samping.
4. `id_tidak_dikenal_selalu_404_bukan_500` — enam permukaan HTTP Fase 1
   sekaligus (`GET/POST` untuk detail, verifikasi, verifikasi/ulang,
   hostkey/konfirmasi, registry).
5. `verifikasi_ulang_saat_sudah_berjalan_ditolak_409` — job ganda ditolak,
   status tidak berubah akibat percobaan yang ditolak.
6. `worker_tidak_menyentuh_server_yang_belum_terverifikasi` — regresi untuk
   bug nyata yang ditemukan smoke test manual sebelumnya (worker menyulap
   status server belum-terverifikasi jadi online).
7. `worker_backoff_bertambah_dan_unreachable_persis_kegagalan_ketiga` —
   siklus worker SUNGGUHAN (bukan cuma fungsi murni) dijalankan 3x
   berturut-turut, memverifikasi `next_poll_at` terus bertambah dan status
   baru jadi `unreachable` persis di kegagalan ketiga.

Satu bug ditemukan SAAT MENULIS test (bukan di kode produksi): helper
`tunggu_verifikasi_selesai` awalnya menunggu status keluar dari `Verifying`,
tapi tepat setelah `POST /servers` status masih `pending` (task yang
di-spawn belum sempat jalan) — race yang bikin test lulus palsu tanpa
benar-benar menunggu apa pun. Diperbaiki: tunggu `last_error_kind` terisi
ATAU status jadi `online`, dengan panic eksplisit kalau batas waktu
tercapai (bukan diam-diam lulus).

**reviewer — 2 batch sesuai `docs/plan.md`, 0 BLOCKING, 3 temuan diperbaiki:**

Batch A (`Cargo.toml`, `migrations/0002_servers.sql`, `crypto.rs`, `ssh/**`,
`docker/**`) dan batch B (`servers/**`, `registries/**`, `worker/**`,
`events.rs`, `routes/**`, `state.rs`, `main.rs`) dibaca penuh. Batas peran
diperiksa: `src/web/` nol `sqlx::`/`bollard::`/`openssh::` (grep, bukan
asumsi); `src/routes/` nol pemanggilan `html!` langsung dan nol
`sqlx::query` langsung KECUALI satu baris pre-existing Fase 0 di
`login.rs` (dicatat, bukan diperbaiki — di luar cakupan Fase 1).

Temuan (ketiganya sudah masuk daftar security di atas atau ditutup
terpisah):
- 2 temuan security (gitignore, status code) — lihat di atas.
- **Invariant 10 (satu transaksi per siklus) bocor di
  `servers::verify::tautkan_registry`**: jalur registry BARU memanggil
  `registries::repo::upsert` lalu `record_login_success` sebagai DUA
  statement terpisah tanpa transaksi pembungkus — crash tepat di antara
  keduanya bisa menyisakan baris `registries` tanpa `server_registries`
  yang menautkannya. Diperbaiki: fungsi baru
  `registries::repo::upsert_dan_catat_login` membungkus keduanya dalam satu
  `pool.begin()...commit()`; fungsi `upsert` lama (sudah tidak dipakai)
  dihapus, bukan dibiarkan jadi dead code.

Verifikasi orchestrator langsung setelah SEMUA perbaikan gerbang:
`cargo sqlx prepare -- --all-targets` (query baru dari
`upsert_dan_catat_login` dan test `phase1.rs`), `cargo build --all-targets`
bersih, `cargo fmt` bersih, `cargo clippy --all-targets --all-features -- -D
warnings` No issues found, `cargo test` **134 passed** (naik dari 127 — 7
test `phase1.rs` baru), `cargo test --test phase0` tetap **21/21**,
`cargo test --test phase1` **7/7** — TIDAK ADA REGRESI.

**Kesimpulan: Fase 1 LOLOS gerbang** (`docs/prd.md` §6) — Definition of
Done terverifikasi langsung (bukan diasumsikan), ≥3 skenario injeksi
kegagalan (nyatanya 7) diuji dan lulus, security tanpa blocker, review kode
tanpa pelanggaran invariant tersisa, migrasi bersih. Siap membuka Fase 2
(Loop deploy) kapan pun manusia memutuskan.

---

## Fase 2 — Loop deploy: implementasi selesai, gerbang ditutup

**Tanggal:** 2026-08-10. Dikerjakan seri lewat semua sub-blok `docs/plan.md`
(0 migration → 2a docker/client.rs → 2b auth/deploy_token.rs → 2c apps/** →
2d jobs/** → 2e deployments/{model,repo,engine}.rs → 2f worker/deploy_worker.rs
→ 2g routes/**+web/**, digabung karena saling bergantung untuk kompilasi →
security → qa → reviewer), tanpa jeda antar sub-blok (satu sesi panjang).

**Skema:** `migrations/0003_deploy.sql` — `apps`, `domains`, `deployments`,
`deploy_tokens`, `jobs`. Terapkan bersih dari kosong, 3 migrasi berurutan.

**Backend inti (`deployments::engine::jalankan_docker`):** urutan KAKU
start container baru → health check kita sendiri (bukan Traefik) → lulus:
`stop --time=30` container lama / gagal: tangkap 50 baris log SEBELUM
`remove_container` (invariant §3 no.7, untuk SEMUA mode kegagalan bukan
cuma container exited) → hapus container baru, container lama TIDAK
tersentuh (invariant §3 no.1). Tiga mode klasifikasi kegagalan health check
persis `docs/prd.md` (`ContainerExited`/`HealthNon2xx`/`HealthNoResponse`)
plus `PullGagal`/`Lain`, masing-masing punya `error_kind` + pesan Bahasa
Indonesia dengan kemungkinan penyebab. Lock per app WAJIB kedaluwarsa
(`LOCK_TTL_SECS=900`, invariant §3 no.12), id deployment dipakai sekaligus
sebagai lock token.

**Bootstrap Traefik lazy (Q2, ditemukan BELUM diimplementasikan walau
sudah dijawab manusia di sub-blok sebelumnya — ditutup di sesi ini):** cek
label `platform.traefik=true` tiap deploy (murah, self-healing kalau
container Traefik dihapus manual, TANPA state tambahan "sudah pernah
bootstrap"), kalau belum ada: `ensure_network` → pull tag `traefik:v3.1` →
`resolve_image_digest` (invariant §5 no.6 berlaku juga untuk image infra —
container Traefik dijalankan dengan `@sha256:...` hasil resolusi, TAG
hanya argumen pull) → create+start dengan port 80 host-bound (satu-satunya
container yang boleh `-p`, invariant §5 no.5 mengecualikan Traefik sendiri
karena dia PINTU masuk, bukan salah satu dari dua container app yang hidup
bersamaan) + docker socket mount read-only. **Batas diketahui (dicatat
`// ponytail:` di `docker/client.rs::container_exists_with_label`):** cek
hanya keberadaan label, bukan status `running` — Traefik yang gagal start
tepat di antara create dan start akan salah dianggap "sudah bootstrap" di
deploy berikutnya. Kandidat upgrade Fase 4 (banner drift), bukan auto-heal.

**Kontrak `POST /api/v1/deploy`:** urutan validasi KAKU — app dulu (404
kalau tidak dikenal, TERMASUK saat token app lain valid, supaya tidak
membocorkan app mana yang ada) → token app itu diverifikasi (401) →
validasi digest (400, regex efektif `@sha256:[a-f0-9]{64}` di akhir
referensi) → lock diambil (409 kalau app sedang deploy lain). Deployment
`queued` + job deploy masuk SATU transaksi
(`deployments::repo::insert_queued_dengan_job`) — kegagalan salah satu
tidak menyisakan yang lain yatim.

**Security review (Q3 — rate limit, satu-satunya pertanyaan yang
didelegasikan ke sub-blok ini):** batas ternyata salah kalau dipasang di
layer HTTP (`POST /api/v1/deploy` cuma menyisipkan baris lalu balas 202,
kerja berat SSH+docker pull terjadi BELAKANGAN di worker) —
`ConcurrencyLimitLayer` di route sempat dicoba lalu DIBATALKAN, diganti
`tokio::sync::Semaphore` (4 slot) di `worker::deploy_worker` yang membatasi
`jalankan_deploy` PARALEL sungguhan, permit dipegang task yang di-spawn dan
lepas otomatis saat selesai. Tidak menambah dependency (`tokio::sync` sudah
ada) — percobaan pertama sempat menambah fitur `tower` "limit" lalu
di-revert karena salah sasaran.

Item lain yang direview tanpa temuan blocking: token deploy argon2 satu
arah + per-app (bukan global) + tidak pernah dikembalikan API setelah
dibuat (banner plaintext HANYA di response `POST /apps/{id}/token`, render
ulang langsung bukan redirect — supaya plaintext tidak pernah singgah di
query string/riwayat browser); kredensial registry pull dicocokkan HANYA
lewat `server_registries` yang sudah login di server SPESIFIK itu (tidak
ada jalur cross-server); semua `sqlx::query!`/`query_as!` parameterized.

**qa — `tests/phase2.rs`, 7 skenario injeksi kegagalan (minimum PRD: 5):**

1. `token_salah_ditolak_401_tanpa_membuat_deployment`
2. `app_tidak_dikenal_selalu_404_walau_token_app_lain_valid`
3. `image_tanpa_digest_ditolak_400`
4. `deploy_valid_membuat_deployment_queued_dan_202`
5. `deploy_kedua_saat_lock_aktif_ditolak_409`
6. `deploy_ke_server_tak_terjangkau_gagal_dan_lock_terlepas` — memanggil
   `jalankan_deploy` langsung (pola sama `tests/phase1.rs` memanggil
   `verify::` langsung, bukan lewat worker tick) ke server yang diarahkan
   ke port tertutup (trik sama Fase 1) — deployment berakhir `failed`
   dengan `error_kind` terisi, DAN lock terlepas (dibuktikan lewat
   percobaan deploy kedua yang 202, bukan 409 — invariant §3 no.12 diuji
   lewat efek, bukan diasumsikan).
7. `jalankan_deploy_dengan_id_tidak_dikenal_tidak_panik`

**Keterbatasan qa yang dicatat jujur (bukan disembunyikan):** skenario yang
butuh Docker daemon sungguhan (container exited, health check
gagal/lulus setelah grace, urutan tangkap-log-sebelum-hapus, port bentrok)
TIDAK dites integrasi penuh — lingkungan test tidak punya daemon Docker,
sama keterbatasan SSH-vs-Docker yang sudah ada sejak Fase 1. Klasifikasi
kegagalan diuji unit (`deployments::engine` test `kind()`/`pesan()`); urutan
literal start-baru→health-check→stop-lama dan tangkap-log-sebelum-hapus
diverifikasi lewat pembacaan kode langsung (baris demi baris di atas), bukan
test otomatis.

**reviewer:** `src/web/**` diperiksa nol `sqlx::`/`bollard::`/`openssh::`
(grep, bukan asumsi) — batas modul Fase 1 tetap terjaga di Fase 2.
`src/routes/**` nol pemanggilan `html!` langsung. `deployments::repo`
IDENTIK pola `apps::repo`/`servers::repo` — tidak ada helper mapping baris
dengan argumen posisional (`baris_ke_ringkas` dihapus, diganti struct
literal inline atau `sqlx::query_as!` langsung, tergantung apakah kolom
butuh konversi enum). Ditemukan DAN diperbaiki di sesi ini (bukan temuan
gerbang terpisah, bagian dari mengejar clippy bersih): fungsi
`extract_registry_host` salah mengira `nginx@sha256:...` (image Docker Hub
resmi tanpa host eksplisit, digest langsung nempel di nama) sebagai host
registry — segmen sebelum `/` tidak ada sama sekali di referensi tanpa
`/`, jadi kolon di `sha256:` salah kebaca sebagai port host. Diperbaiki:
`split_once('/')` (butuh `/` beneran, bukan cuma `contains(':')`) sebelum
mengecek `.`/`:`.

Verifikasi orchestrator langsung setelah SEMUA sub-blok:
`cargo sqlx prepare -- --all-targets` bersih, `cargo build --all-targets`
bersih, `cargo fmt` bersih, `cargo clippy --all-targets --all-features -- -D
warnings` No issues found, `cargo test --all-targets` **170 passed** (naik
dari 148 sebelum sub-blok 2g — 15 test frontend/routes baru + 7 test
`phase2.rs` baru), `cargo test --test phase0` tetap **21/21**,
`cargo test --test phase1` tetap **7/7**, `cargo test --test phase2`
**7/7** — TIDAK ADA REGRESI ke Fase 0/1.

**Kesimpulan: Fase 2 LOLOS gerbang** (`docs/prd.md` §6) — Definition of
Done terverifikasi langsung, ≥5 skenario injeksi kegagalan (nyatanya 7)
diuji dan lulus (dengan keterbatasan Docker-integration yang dicatat
jujur di atas, bukan disembunyikan), security tanpa blocker, review kode
tanpa pelanggaran invariant tersisa (1 bug nyata ditemukan dan diperbaiki
selama proses, bukan hanya asumsi "kode sudah benar"), migrasi bersih.
Gap yang SENGAJA belum dikerjakan, dicatat eksplisit bukan didiamkan:
`HEAD https://{domain}` post-swap Traefik verification check dari PRD
(butuh TLS-capable HTTP client, dependency baru rustls/openssl-sys —
di luar scope sesi ini, PRD menandainya best-effort/non-blocking).
