# Kontrak HTTP

> Ditulis oleh agent **planner**, dibaca oleh **frontend**, **backend**, **qa**,
> dan **security**. Hanya diubah kalau ada permukaan HTTP yang berubah.
>
> **Implementer tidak boleh mengubah file ini.** Kalau kontraknya bermasalah,
> lapor dan berhenti — kontrak yang berubah setelah implementasi dimulai adalah
> kondisi berhenti bagi orchestrator.

## Autentikasi

| Mekanisme | Dipakai untuk | Status |
|---|---|---|
| Session cookie | Semua halaman kecuali `/login` | Aktif sejak Fase 0 |
| Bearer token | `POST /api/v1/deploy` | Belum dibuka (Fase 2) |

Route yang butuh login **wajib** masuk router terlindungi, bukan router luar.
Route yang lupa dipasangi middleware auth adalah temuan blocking bagi `security`.

## Aturan yang berlaku untuk semua endpoint

- Semua teks yang dikembalikan dalam **Bahasa Indonesia**.
- Setiap `POST` dari form terlindungi wajib membawa token CSRF.
- Response `4xx`/`5xx` tidak boleh membocorkan detail internal: tidak ada path
  filesystem, tidak ada isi query, tidak ada pesan library mentah.
- **Tidak ada endpoint yang mengembalikan secret** dalam bentuk apa pun — private
  key SSH, hash password, isi file kunci enkripsi, token session, token API
  (`docs/prd.md` §3 nomor 7).
- Pesan gagal login tidak boleh membedakan "user tidak ada" dari "password salah".
- Image dirujuk dengan digest, tidak pernah dengan tag (`docs/prd.md` §3 nomor 4).
  Endpoint yang menerima referensi image wajib menolak tag.
- Setiap operasi jarak jauh punya timeout per tahap, bukan timeout global
  (`docs/prd.md` §3 nomor 11).

---

## Endpoint

Format per endpoint:

```

---

### METHOD /path

Akses     : session cookie | bearer token | publik
Request   : field → tipe → aturan validasi
Sukses    : status code + bentuk response
Error     : status code → kondisi → bentuk response
Efek      : perubahan state, job yang di-enqueue, file yang ditulis
Tidak     : field yang TIDAK PERNAH dikembalikan
dikembalikan
```

---

## Fase 0 — Fondasi

Enam permukaan HTTP. Tidak ada endpoint JSON API di fase ini; semua respons adalah
HTML (Maud) atau redirect. Bearer token belum dibuka (Fase 2). Semua halaman
kecuali `/healthz`, `GET /login`, dan `POST /login` masuk **router terlindungi**.

### GET /healthz

```
Akses     : publik (tidak butuh sesi)
Request   : tidak ada body, tidak ada query
Sukses    : 200 OK — body teks pendek "ok" (bukan HTML shell)
Error     : 500 → proses tidak sehat → body generik tanpa detail internal
Efek      : tidak ada (read-only, tidak sentuh db pada jalur sukses)
Tidak     : tidak pernah mengembalikan versi library, path, atau isi config
dikembalikan
```

Catatan: `/healthz` sengaja publik supaya health check eksternal (systemd,
uptime monitor) tidak perlu sesi. Tidak membocorkan apa pun soal state internal.

### GET /login

```
Akses     : publik (tidak butuh sesi)
Request   : tidak ada body; query opsional untuk menandai state (mis. sesi
            kedaluwarsa) — teks dari docs/design/login.md, bukan detail internal
Sukses    : 200 OK — HTML halaman login (form: field password + hidden CSRF token)
Error     : —
Efek      : set cookie CSRF sekali-pakai / token CSRF ditanam di form (implementer
            putuskan mekanismenya; token CSRF wajib untuk POST /login)
Tidak     : tidak pernah mengembalikan password_hash, token sesi, atau isi settings
dikembalikan
```

Jika sudah punya sesi valid, `GET /login` boleh redirect `303` ke `/`
(opsional, bukan kriteria selesai).

### POST /login

```
Akses     : publik (proses login itu sendiri)
Request   : password → string → wajib, tidak kosong
            csrf_token → string → wajib, cocok dengan token yang ditanam GET /login
            (username TIDAK ADA — pengguna tunggal; lihat plan.md Q5)
Sukses    : 303 See Other → Location: /
            Set-Cookie: <nama>=<token sesi>; HttpOnly; Secure; SameSite=Lax; Path=/
            Sesi baru dibuat di tabel sessions; sesi lama (jika ada) dirotasi/dihapus
Error     : 400 → CSRF token hilang/tidak cocok → re-render login + pesan generik
            401 → kredensial salah → re-render login (atau status 401) dengan pesan
                  GENERIK yang tidak membedakan "user tidak ada" vs "password salah"
            429 → (opsional Fase 0) terlalu banyak percobaan — tidak wajib
Efek      : INSERT baris sessions (pool tulis); hapus sesi lama; set cookie sesi.
            Bila settings belum punya password_hash dan MENGDEP_INITIAL_PASSWORD
            di-seed (plan.md Q5), hashing dilakukan saat startup, bukan di sini.
Tidak     : tidak pernah mengembalikan password_hash, isi token sesi di body,
dikembalikan  detail kegagalan verifikasi Argon2, atau apakah user ada. Token sesi
              HANYA lewat header Set-Cookie, tidak pernah di body/HTML.
```

Pesan gagal generik (Bahasa Indonesia) — teks final dari `docs/design/login.md`.

### POST /logout

```
Akses     : session cookie (router terlindungi)
Request   : csrf_token → string → wajib, cocok (form terlindungi wajib CSRF)
Sukses    : 303 See Other → Location: /login
            Set-Cookie: <nama>=; Max-Age=0 (cookie sesi dihapus)
Error     : 400 → CSRF token hilang/tidak cocok → tolak, tidak menghapus sesi
            401/303 → tidak ada sesi valid → redirect /login (idempoten aman)
Efek      : DELETE baris sessions untuk sesi aktif (pool tulis); clear cookie
Tidak     : tidak mengembalikan apa pun soal sesi yang dihapus
dikembalikan
```

### GET /

```
Akses     : session cookie (router terlindungi)
Request   : tidak ada
Sukses    : 200 OK — HTML shell aplikasi kosong (sidebar, header, area konten),
            dirender src/web/dashboard.rs + layout.rs. Belum ada fitur produk.
Error     : 303 See Other → Location: /login → bila tidak ada sesi valid atau
            sesi kedaluwarsa (expires_at lewat) → diperlakukan seolah tidak ada
Efek      : membaca sesi (pool baca) untuk validasi; tidak menulis
Tidak     : tidak pernah menampilkan token sesi, password_hash, atau isi settings
dikembalikan  sensitif di HTML
```

### 404 & 500 (bukan route, tapi permukaan HTTP)

```
Akses     : mengikuti konteks (fallback handler + error mapping)
404       : path tidak dikenal → 404 Not Found → HTML dari src/web/error_page.rs
500       : error tak tertangani → 500 Internal Server Error → HTML generik dari
            src/web/error_page.rs
Error     : —
Efek      : tidak ada
Tidak     : 500 tidak pernah membocorkan pesan library mentah, path filesystem,
dikembalikan  isi query, backtrace, atau nilai secret. error.rs memetakan ke pesan
              generik Bahasa Indonesia; detail hanya ke tracing (tanpa secret).
```

---

## Fase 1 — Registry server dan konektivitas

Sebelas permukaan HTTP baru. Bagian Fase 0 di atas **tetap berlaku tanpa
perubahan**; `GET /` hanya bertambah isi (fleet strip), bentuk responsnya tetap
HTML shell.

Masih **tidak ada endpoint JSON API**. Semua respons adalah HTML (Maud), fragmen
HTML (untuk HTMX), redirect, atau `text/event-stream`. Bearer token **tetap belum
dibuka** — itu Fase 2. Semua endpoint di bawah **kecuali `GET /assets/*`** masuk
router terlindungi (`src/routes/mod.rs:22-25`).

### Aturan tambahan yang berlaku untuk seluruh Fase 1

- **Private key SSH dan token registry tidak pernah dikembalikan dalam bentuk apa
  pun** — tidak plaintext, tidak ciphertext `age`, tidak sebagian, tidak sebagai
  panjang karakter, tidak sebagai fingerprint turunan kunci privat
  (`docs/prd.md` §3 nomor 7). Field `ssh_key_encrypted` dan `token_encrypted`
  **tidak pernah** masuk view-model. Setelah disimpan, satu-satunya cara mengubah
  kunci adalah mengirim kunci baru.
- **Fingerprint host key BUKAN secret** dan memang wajib ditampilkan
  (`docs/prd.md:245`). Yang ditampilkan adalah fingerprint **host key server
  target**, bukan turunan private key pengguna.
- **Pesan error tidak pernah memuat `stderr` mentah** dari SSH atau Docker. Setiap
  kegagalan dipetakan ke kategori berpesan tetap dari `docs/design/tambah-server.md`,
  dan setiap pesan menyebut langkah perbaikannya (`docs/prd.md:244`).
- **Kegagalan tidak pernah menghapus apa pun** (`docs/prd.md` §3 nomor 1). Tidak ada
  endpoint di fase ini yang menghapus baris `servers`, `registries`, atau kredensial
  — endpoint hapus/ubah **sengaja tidak dibangun** (lihat `docs/plan.md`, "Yang
  sengaja tidak dikerjakan").
- **Setiap `POST` wajib membawa `csrf_token`** yang cocok dengan `csrf_token` sesi
  aktif (`src/auth/session.rs:20-27`).
- `{id}` pada path adalah id baris `servers`. Id yang tidak dikenal → **404**, bukan
  403 dan bukan 500. Pengguna tunggal, jadi tidak ada kebocoran lintas tenant di
  sini — tapi 404 tetap dipakai supaya konsisten dengan fallback yang sudah ada.
- Timeout per tahap (`docs/prd.md` §3 nomor 11) berlaku di dalam handler; **tidak
  ada satu timeout global** yang membungkus seluruh verifikasi. Tabel batas ada di
  `docs/plan.md`, bagian "Timeout per tahap".

---

### GET /servers

```
Akses     : session cookie (router terlindungi)
Request   : tidak ada body; tidak ada query di Fase 1 (tanpa filter, tanpa paging —
            3-8 server, docs/prd.md:12)
Sukses    : 200 OK — HTML fleet overview: tabel seluruh server (nama, host, status,
            docker_version, os_info, last_seen_at) dari src/web/fleet.rs.
            Nol server → state kosong dari docs/design/fleet-overview.md, tetap 200.
Error     : 303 See Other → Location: /login → tidak ada sesi valid
Efek      : SELECT lewat pool baca; tidak menulis apa pun
Tidak     : ssh_key_encrypted, token_encrypted, isi private key, token registry,
dikembalikan  password_hash, token sesi. Baris tabel hanya memuat field ServerRingkas
              (src/servers/model.rs) yang menurut definisi tidak punya field kunci.
```

### GET /servers/baru

```
Akses     : session cookie (router terlindungi)
Request   : tidak ada
Sukses    : 200 OK — HTML wizard langkah 1: form nama, host, port, ssh_user,
            private key (textarea), plus hidden csrf_token. Dirender
            src/web/server_add.rs.
Error     : 303 See Other → Location: /login → tidak ada sesi valid
Efek      : tidak ada (tidak menulis, tidak menyentuh jaringan)
Tidak     : form TIDAK PERNAH di-prefill dengan kunci yang sudah tersimpan, bahkan
dikembalikan  saat menambah ulang server yang sudah ada
```

### POST /servers

Wizard langkah 1 → 2. Membuat baris server berstatus `pending` lalu memulai job
verifikasi asinkron. Handler **tidak menunggu** verifikasi selesai — progres
mengalir lewat SSE.

```
Akses     : session cookie (router terlindungi)
Request   : csrf_token  → string → wajib, cocok dengan csrf_token sesi
            name        → string → wajib, tidak kosong setelah trim
            host        → string → wajib, hostname atau IP; TOLAK skema URL
                          (http://, ssh://) dan TOLAK "host:port" gabungan
            port        → integer → opsional, default 22, rentang 1-65535
            ssh_user    → string → wajib, tidak kosong
            ssh_key     → string → wajib, harus berbentuk private key OpenSSH.
                          Passphrase: lihat plan.md Q2 — belum diputuskan
Sukses    : 303 See Other → Location: /servers/{id}/verifikasi
            Baris servers dibuat: status='pending', consecutive_failures=0.
            ssh_key dienkripsi age SEBELUM menyentuh db (src/crypto.rs).
            Job verifikasi di-spawn; job_id (token acak buram) dikembalikan lewat
            halaman verifikasi, bukan lewat body endpoint ini.
Error     : 400 → csrf_token hilang/tidak cocok → re-render form + pesan generik
            400 → validasi field gagal → re-render form + pesan spesifik per field
                  (teks dari docs/design/tambah-server.md), TANPA mengembalikan
                  nilai ssh_key yang dikirim
            500 → gagal mengenkripsi (kunci age hilang/tidak terbaca) → HTML 500
                  generik; detail hanya ke tracing, tanpa nilai kunci
Efek      : INSERT servers (pool tulis, satu transaksi). Enkripsi private key.
            Spawn job verifikasi + buat broadcast channel in-memory (src/events.rs).
            TIDAK menyentuh server target di dalam handler ini.
Tidak     : ssh_key (mentah maupun terenkripsi) tidak pernah muncul di response,
dikembalikan  di HTML, di redirect, di query string, maupun di tracing. Kalau
              validasi gagal, textarea kunci dikosongkan — bukan diisi ulang.
```

### GET /servers/{id}/verifikasi

Wizard langkah 2. Halaman checklist. Isi checklist diperbarui lewat SSE, bukan
lewat reload (`docs/prd.md:243`).

```
Akses     : session cookie (router terlindungi)
Request   : id → path param → wajib, harus ada di tabel servers
Sukses    : 200 OK — HTML checklist verifikasi (langkah 1 koneksi, langkah 2 Docker
            dengan empat sub-cek, langkah 3 registry) + fingerprint host key kalau
            sudah terbaca + hidden csrf_token + atribut langganan SSE ke
            /events/verifikasi/{job_id}. Dirender src/web/server_add.rs.
            Status terakhir yang sudah diketahui dirender langsung supaya halaman
            tetap benar kalau SSE gagal tersambung.
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            404 → id tidak dikenal → HTML 404 dari src/web/error_page.rs
Efek      : SELECT lewat pool baca. Tidak memulai job baru — job dimulai oleh
            POST /servers atau POST /servers/{id}/verifikasi/ulang.
Tidak     : private key, token registry, stderr mentah dari ssh/docker, path
dikembalikan  socket forward lokal, path known_hosts aplikasi, path file kunci age
```

### POST /servers/{id}/verifikasi/ulang

```
Akses     : session cookie (router terlindungi)
Request   : csrf_token → string → wajib, cocok
            id → path param → wajib, harus ada
Sukses    : 303 See Other → Location: /servers/{id}/verifikasi
            Job verifikasi baru di-spawn memakai kredensial yang SUDAH tersimpan
            (didekripsi di server, tidak pernah melintas ke klien)
Error     : 400 → csrf_token hilang/tidak cocok → tolak, tidak memulai job
            404 → id tidak dikenal
            409 → job verifikasi untuk server ini sedang berjalan → tolak dengan
                  pesan "verifikasi sedang berjalan", JANGAN memulai job kedua
Efek      : UPDATE servers.status='verifying' (pool tulis, satu transaksi).
            Spawn job. TIDAK menghapus data lama apa pun, termasuk saat percobaan
            sebelumnya gagal (docs/prd.md §3 nomor 1).
Tidak     : kredensial dalam bentuk apa pun
dikembalikan
```

### POST /servers/{id}/hostkey/konfirmasi

Konfirmasi TOFU. Pengguna menyetujui fingerprint yang ditampilkan; baru setelah
itu fingerprint disimpan dan koneksi berikutnya memakai mode ketat.

```
Akses     : session cookie (router terlindungi)
Request   : csrf_token  → string → wajib, cocok
            fingerprint → string → wajib, harus PERSIS SAMA dengan fingerprint yang
                          sedang ditawarkan job verifikasi untuk server ini.
                          Tidak cocok → tolak; ini yang mencegah pengguna
                          menyetujui fingerprint lama saat kunci sudah berganti.
Sukses    : 303 See Other → Location: /servers/{id}/verifikasi
            servers.host_key_fingerprint diisi; verifikasi lanjut ke langkah 2
Error     : 400 → csrf_token hilang/tidak cocok
            400 → fingerprint tidak cocok dengan yang ditawarkan → tolak, tidak
                  menyimpan apa pun
            404 → id tidak dikenal
            409 → server sudah punya host_key_fingerprint yang BERBEDA → tolak.
                  Kebijakan penggantian fingerprint yang sudah tersimpan belum
                  diputuskan (plan.md Q6). Sampai Q6 dijawab, endpoint ini HANYA
                  mengisi fingerprint yang masih kosong — tidak pernah menimpa.
Efek      : UPDATE servers.host_key_fingerprint (pool tulis, satu transaksi).
            Menulis entri known_hosts milik aplikasi (file mode 0600, di luar
            ~/.ssh pengguna sistem).
Tidak     : isi known_hosts mentah, path file known_hosts, private key
dikembalikan
```

### GET /events/verifikasi/{job_id}

Satu-satunya endpoint SSE di Fase 1. **Hanya** untuk progres verifikasi — bukan
viewer log (itu Fase 3).

```
Akses     : session cookie (router terlindungi). Endpoint SSE WAJIB terautentikasi;
            job_id bukan pengganti autentikasi, hanya penunjuk channel.
Request   : job_id → path param → wajib, token acak buram; tidak dikenal → 404
Sukses    : 200 OK — Content-Type: text/event-stream
            Tiap event membawa fragmen HTML checklist (render_verifikasi_fragmen)
            yang di-swap HTMX. Event terakhir menandai job selesai (sukses atau
            gagal) supaya klien berhenti menunggu.
            Stream ditutup oleh server setelah event terakhir.
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            404 → job_id tidak dikenal atau channel-nya sudah dibersihkan
Efek      : subscribe ke broadcast channel IN-MEMORY (src/events.rs). Tidak menulis
            db, tidak menulis file. Channel dibersihkan setelah job selesai dan
            saat klien terputus — jangan sampai bocor (pelajaran docs/prd.md:291).
Tidak     : private key, token registry, stderr mentah, exit code telanjang tanpa
dikembalikan  konteks, path filesystem apa pun. Yang dikirim adalah label langkah +
              status + pesan yang sudah dipetakan ke Bahasa Indonesia.
```

### GET /servers/{id}/registry

Wizard langkah 3. Opsional — server boleh selesai tanpa registry.

```
Akses     : session cookie (router terlindungi)
Request   : id → path param → wajib, harus ada
Sukses    : 200 OK — HTML form registry: host, username, token (input password),
            daftar registry yang sudah ada untuk dipakai ulang (host + username
            saja), hidden csrf_token, plus tombol "lewati". Dirender
            src/web/server_add.rs.
Error     : 303 See Other → Location: /login
            404 → id tidak dikenal
Efek      : SELECT registries (pool baca)
Tidak     : token_encrypted maupun token mentah untuk registry yang sudah ada.
dikembalikan  Registry lama ditampilkan HANYA sebagai host + username. Field token
              tidak pernah di-prefill — memakai ulang registry lama berarti memakai
              token yang sudah tersimpan, bukan menampilkannya kembali.
```

### POST /servers/{id}/registry

```
Akses     : session cookie (router terlindungi)
Request   : csrf_token   → string → wajib, cocok
            registry_id  → string → opsional; kalau diisi, pakai registry yang sudah
                           ada dan field host/username/token DIABAIKAN
            host         → string → wajib kalau registry_id kosong; hostname
                           registry (mis. ghcr.io); TOLAK skema URL
            username     → string → wajib kalau registry_id kosong
            token        → string → wajib kalau registry_id kosong, tidak kosong
Sukses    : 303 See Other → Location: /servers/{id}
            Registry disimpan/dipakai ulang (token dienkripsi age sebelum ke db),
            `docker login` dijalankan di server target, baris server_registries
            dicatat, izin ~/.docker/config.json di target diverifikasi dan
            diperketat ke 0600 kalau lebih longgar (docs/prd.md:245).
Error     : 400 → csrf_token hilang/tidak cocok
            400 → validasi field gagal → re-render form, field token DIKOSONGKAN
            404 → id server atau registry_id tidak dikenal
            422 → `docker login` di target ditolak registry → re-render form dengan
                  pesan kategori "kredensial registry ditolak" + langkah perbaikan.
                  Baris registry TIDAK ditandai rusak dan TIDAK dihapus.
            504 → tahap `docker login` melewati batas waktunya → pesan kategori
                  "registry tidak merespons", tanpa detail internal
Efek      : INSERT/SELECT registries + INSERT server_registries (pool tulis, satu
            transaksi). Menjalankan `docker login` di target lewat SSH dengan
            timeout tahap sendiri. Token diberikan ke `docker login` lewat stdin,
            TIDAK pernah sebagai argumen baris perintah (bocor ke `ps` di target).
Tidak     : token registry (mentah/terenkripsi), stderr `docker login` mentah,
dikembalikan  isi ~/.docker/config.json
```

Catatan: melewati langkah ini (tombol "lewati") adalah **navigasi biasa** ke
`GET /servers/{id}` — tidak ada endpoint terpisah dan tidak ada state yang ditulis.

### GET /servers/{id}

```
Akses     : session cookie (router terlindungi)
Request   : id → path param → wajib, harus ada
Sukses    : 200 OK — HTML detail server (kerangka): nama, host, port, ssh_user,
            status, last_seen_at, docker_version, os_info, host_key_fingerprint,
            daftar registry yang tertaut (host + username saja), kategori kegagalan
            terakhir kalau ada. Dirender src/web/server_detail.rs.
            TANPA grafik metrik apa pun — itu Fase 6 (docs/prd.md:243).
Error     : 303 See Other → Location: /login
            404 → id tidak dikenal
Efek      : SELECT lewat pool baca
Tidak     : ssh_key_encrypted, token_encrypted, private key, token registry,
dikembalikan  stderr mentah, path socket forward, path file kunci age
```

### GET /assets/htmx.min.js

**Keberadaan endpoint ini bergantung pada `docs/plan.md` Q4** (vendor lokal vs CDN).
Kalau Q4 dijawab "CDN", endpoint ini tidak dibuat dan bagian ini dihapus dari
kontrak oleh planner — **bukan** oleh implementer.

```
Akses     : publik (aset statis; tidak memuat data pengguna sama sekali)
Request   : tidak ada
Sukses    : 200 OK — Content-Type: application/javascript, isi file yang di-embed
            ke binary saat kompilasi. Boleh mengirim header cache panjang karena
            path-nya berversi.
Error     : 404 → path aset tidak dikenal
Efek      : tidak ada; tidak menyentuh db
Tidak     : tidak ada data aplikasi apa pun. Endpoint ini TIDAK boleh berubah
dikembalikan  menjadi penyaji file umum — tidak ada path param, tidak ada
              kemungkinan path traversal. Daftar aset bersifat tetap saat kompilasi.
```

### GET / (perubahan, bukan endpoint baru)

```
Akses     : session cookie (router terlindungi) — TIDAK BERUBAH
Request   : tidak ada — TIDAK BERUBAH
Sukses    : 200 OK — HTML shell; sekarang memuat fleet strip yang menempel
            (nama + badge status per server) di semua halaman terlindungi.
            Placeholder "Belum ada server terdaftar … pada Fase 1 nanti"
            (src/web/dashboard.rs:21) diganti isi yang sesuai keadaan armada.
Error     : 303 See Other → Location: /login — TIDAK BERUBAH
Efek      : bertambah satu SELECT ringkas ke servers lewat pool baca
Tidak     : sama seperti Fase 0, ditambah: tidak ada kunci atau token di fleet strip
dikembalikan
```

Fleet strip juga muncul di `/servers`, `/servers/{id}`, dan seluruh halaman
terlindungi lain (`docs/prd.md:243`). Halaman error 404/500 boleh tanpa strip —
`app_shell` menerima strip sebagai `Option`.

---

## Fase 3 — Log dan riwayat

Sembilan permukaan HTTP baru (delapan kalau `docs/plan.md` Q1 dijawab "(c) tanpa
xterm.js"). Bagian Fase 0 dan Fase 1 di atas **tetap berlaku tanpa perubahan**;
permukaan Fase 2 yang sudah berjalan (`POST /api/v1/deploy`, `/apps*`,
`/deployments/{id}`, `/events/deploy/{id}`) juga **tidak berubah** — yang
bertambah hanya isi halaman `GET /deployments/{id}` (tautan ke log) dan
`GET /apps/{id}` (tiga tab).

Catatan keadaan file ini: permukaan Fase 2 **tidak pernah ditulis** sebagai
bagian tersendiri di dokumen ini (bagian terakhir sebelum ini adalah Fase 1) —
kontraknya hidup di `docs/plan.md` versi Fase 2 dan di kode yang sudah lolos
gerbang (`src/routes/{deploy_api,apps,deployments,events}.rs`). Fase 3 **tidak**
menambal celah itu; menuliskan ulang kontrak yang implementasinya sudah beku
berisiko menciptakan versi kedua yang berbeda dari kode. Kalau celah itu perlu
ditutup, itu task dokumentasi tersendiri untuk planner, bukan pekerjaan
implementer Fase 3.

Masih **tidak ada endpoint JSON API baru**. Respons di fase ini adalah HTML
(Maud), fragmen HTML untuk HTMX, `text/event-stream`, atau `text/plain` untuk
unduhan. Bearer token tetap **hanya** untuk `POST /api/v1/deploy` — tidak ada
satu pun endpoint log yang menerima bearer token. Semua endpoint di bawah
**kecuali `GET /assets/*`** masuk router terlindungi (`src/routes/mod.rs:29-59`).

### Aturan tambahan yang berlaku untuk seluruh Fase 3

- **Baris log tidak pernah dilayani dari SQLite** (`docs/prd.md` §3 nomor 9).
  Sumber isi log hanya dua: file di `<log_dir>/deploy/` (log deploy) dan stream
  `docker logs` lewat socket yang di-forward SSH (log runtime). Endpoint mana
  pun yang mengambil isi log dari kolom database adalah temuan blocking.
- **Path file tidak pernah dikembalikan ke klien** — tidak di HTML, tidak di
  pesan error, tidak di header. Klien hanya pernah melihat `deployment_id`.
  `deployment_logs.path` menyimpan nama file saja dan tetap di sisi server.
- **`{id}` selalu divalidasi sebelum path dibentuk**: `^[A-Za-z0-9]{1,64}$`
  (`logs::reader::nama_file_aman`, `docs/plan.md` "Anti path traversal"). Id
  yang tidak lolos pola, atau lolos pola tapi tidak ada di db → **404**, bukan
  400 dan bukan 500. Tidak ada perbedaan pesan antara keduanya.
- **Endpoint SSE wajib terautentikasi** (`docs/prd.md:289`). Tidak ada
  `job_id`/token buram yang boleh menggantikan sesi.
- **Isi log adalah keluaran aplikasi pengguna** dan diperlakukan sebagai data
  tidak tepercaya: selalu di-escape saat masuk HTML (nol `PreEscaped` di
  `src/web/logs.rs`), tidak pernah dieksekusi. Kebijakan penyaringan secret yang
  dicetak aplikasi pengguna menunggu `docs/plan.md` Q2 (rekomendasi planner:
  peringatkan, jangan saring).
- **Control plane tidak pernah menulis secretnya sendiri ke file log** — private
  key SSH, token registry, token deploy, isi kunci `age`. Ini berlaku di writer,
  bukan di viewer.
- Setiap operasi jarak jauh (jalur log runtime) memakai timeout per tahap dari
  tabel `docs/plan.md` "Timeout per tahap" — **tidak ada** satu timeout global
  yang membungkus sesi streaming, dan **sunyi bukan error**.
- Endpoint log tidak punya `POST` sama sekali di fase ini: semuanya baca. Karena
  itu tidak ada `csrf_token` di bagian ini — tidak ada form mutasi baru.

---

### GET /deployments/{id}/log

Halaman viewer log deploy (log yang ditulis control plane selama deployment).

```
Akses     : session cookie (router terlindungi)
Request   : id   → path param → wajib, ^[A-Za-z0-9]{1,64}$ dan ada di deployments
            tail → query, opsional → integer, default 500, maksimum 5000; di luar
                   rentang → dijepit ke maksimum (bukan 400 — ini kenyamanan baca,
                   bukan perintah destruktif)
            q    → query, opsional → string pencarian; kosong/absen = tanpa filter
Sukses    : 200 OK — HTML viewer (src/web/logs.rs): area monospace, gutter
            timestamp, toggle wrap, toggle follow, kotak cari, tombol unduh.
            Isi awal dirender dari FILE (tail), bukan menunggu SSE — reload di
            tengah deploy tetap menampilkan log yang benar.
            Deployment belum selesai → halaman menyertakan atribut langganan SSE
            ke /events/log/deploy/{id}.
            Deployment sudah selesai → isi statis, TANPA membuka SSE.
            File log belum ada / kosong / nol baris → state kosong dari
            docs/design/log-viewer.md, tetap 200.
            deployment_logs.truncated = 1 → penanda "log terpotong pada batas
            8 MiB" ikut dirender (docs/plan.md, batas ukuran).
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            404 → id tidak dikenal ATAU tidak lolos pola id
            500 → file metadata ada tapi tidak terbaca → HTML 500 generik;
                  path dan pesan io mentah HANYA ke tracing
Efek      : SELECT deployment_logs + deployments (pool baca); baca file log
            (batas 5 detik). Tidak menulis apa pun, tidak menyentuh server target.
Tidak     : path file log, path direktori log, isi kunci age, pesan io mentah,
dikembalikan  nama host filesystem control plane
```

### GET /deployments/{id}/log/isi

Fragmen HTML untuk HTMX — dipakai saat mengganti `tail`, menjalankan pencarian,
atau menyegarkan isi tanpa memuat ulang seluruh halaman. Bukan endpoint SSE.

```
Akses     : session cookie (router terlindungi)
Request   : id   → path param → wajib, sama aturannya dengan di atas
            tail → query, opsional → default 500, maksimum 5000
            q    → query, opsional → string; hasil dibatasi 500 baris cocok,
                   selebihnya dipotong dengan penanda eksplisit di fragmen
Sukses    : 200 OK — FRAGMEN HTML (bukan halaman penuh, tanpa app shell):
            daftar baris log + penanda status (kosong / terpotong / hasil
            pencarian dipotong). Dirender src/web/logs.rs.
Error     : 303 See Other → Location: /login
            404 → id tidak dikenal
            504 → pencarian melewati batas 5 detik → fragmen pesan kategori
                  "pencarian terlalu lama, persempit kata kunci" — TANPA detail
                  internal dan TANPA menyebut ukuran file
Efek      : baca file (pool baca hanya untuk metadata). Tidak menulis.
Tidak     : path file, offset byte internal, nama file di disk
dikembalikan
```

### GET /deployments/{id}/log/unduh

```
Akses     : session cookie (router terlindungi)
Request   : id → path param → wajib
Sukses    : 200 OK — Content-Type: text/plain; charset=utf-8
            Content-Disposition: attachment; filename="deploy-{id}.log"
            Isi file log APA ADANYA (termasuk escape ANSI, kalau ada) — nama
            berkas dibentuk dari id yang sudah divalidasi, BUKAN dari nama file
            di disk dan BUKAN dari input klien.
Error     : 303 See Other → Location: /login
            404 → id tidak dikenal ATAU file log tidak ada (deployment lebih tua
                  dari retensi 30 hari dan sudah tersapu) → halaman 404 biasa,
                  tanpa membedakan "belum pernah ada" vs "sudah dihapus retensi"
                  di level status; teks penjelasannya dari docs/design/log-viewer.md
Efek      : baca file. Tidak menulis, tidak menyentuh server target.
Tidak     : path absolut file, header apa pun yang membocorkan layout disk
dikembalikan
```

Log runtime **tidak punya** endpoint unduh — runtime tidak dipersistensi di
control plane (`docs/plan.md`, "Yang sengaja tidak dikerjakan").

### GET /events/log/deploy/{id}

SSE log deploy langsung. Menyiarkan baris yang sedang ditulis writer.

```
Akses     : session cookie (router terlindungi). WAJIB terautentikasi — tidak
            ada token buram yang boleh menggantikan sesi (docs/prd.md:289).
Request   : id → path param → wajib, ^[A-Za-z0-9]{1,64}$ dan ada di deployments
Sukses    : 200 OK — Content-Type: text/event-stream
            Tiap event membawa fragmen HTML satu-atau-beberapa baris log yang
            di-append HTMX (bukan swap seluruh isi — histori yang sudah dirender
            tidak boleh hilang tiap event).
            Event khusus:
              - "tertinggal" → subscriber lag (broadcast Lagged): fragmen penanda
                "--- {n} baris terlewat; muat ulang untuk histori lengkap ---".
                Baris yang hilang TIDAK PERNAH didiamkan (docs/plan.md aturan 4).
              - "selesai"    → deployment berakhir; server menutup stream setelah
                mengirimnya supaya klien berhenti menunggu.
            Tidak ada sesi log aktif (deployment sudah selesai sebelum klien
            menyambung) → kirim SATU event "selesai" lalu tutup, JANGAN membuka
            channel baru (LogRegistry::ikut mengembalikan None; hanya writer
            yang boleh membuat sesi).
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            404 → id tidak dikenal
Efek      : subscribe ke broadcast channel IN-MEMORY (src/logs/registry.rs).
            Tidak menulis db, tidak menulis file, tidak menyentuh server target.
            Channel dibersihkan otomatis saat Arc terakhir drop; entri map
            memegang Weak (docs/prd.md:291, :384 — kebocoran memori paling
            mungkin di proyek ini).
Tidak     : path file, private key, token registry/deploy, stderr internal
dikembalikan  control plane, backtrace. Yang dikirim hanya baris log aplikasi
              + penanda status yang sudah dipetakan ke Bahasa Indonesia.
```

### GET /apps/{id}/deployments

Tab Deployments — riwayat deployment satu aplikasi.

```
Akses     : session cookie (router terlindungi)
Request   : id → path param → wajib, harus ada di tabel apps
Sukses    : 200 OK — HTML halaman detail app dengan tab Deployments aktif:
            waktu, status, commit_sha, image_digest, durasi, tautan ke
            /deployments/{id} dan ke /deployments/{id}/log.
            Batas 100 deployment terbaru; lebih dari itu dirender dengan penanda
            "menampilkan 100 terbaru" (docs/plan.md — tanpa paging di fase ini).
            Nol deployment → state kosong dari docs/design/riwayat-deployment.md,
            tetap 200.
Error     : 303 See Other → Location: /login
            404 → id app tidak dikenal
Efek      : SELECT deployments lewat pool baca (deployments::repo::list_by_app).
            Tidak menulis, tidak menyentuh server target.
Tidak     : deploy token (hash maupun plaintext), ssh_key_encrypted,
dikembalikan  token_encrypted, isi env var (itu Fase 4), path log
```

Tab ini **hanya membaca**. Tidak ada tombol rollback di sini — itu Fase 5
(`docs/prd.md:326`).

### GET /apps/{id}/logs

Tab Logs — viewer log runtime container yang sedang berjalan.

```
Akses     : session cookie (router terlindungi)
Request   : id   → path param → wajib, harus ada di tabel apps
            tail → query, opsional → default 200, maksimum 2000
Sukses    : 200 OK — HTML halaman detail app dengan tab Logs aktif: viewer
            monospace + kotak cari + toggle wrap/follow (TANPA tombol unduh —
            log runtime tidak dipersistensi).
            Ada deployment live dengan container_id → halaman memasang langganan
            SSE ke /events/log/runtime/{id}.
            TIDAK ada deployment live, atau container_id NULL → state
            "belum ada container yang berjalan" dari docs/design/log-viewer.md,
            SSE TIDAK dibuka, tetap 200 (bukan 404, bukan 500).
Error     : 303 See Other → Location: /login
            404 → id app tidak dikenal
Efek      : SELECT apps + deployment live (pool baca). Handler ini sendiri TIDAK
            membuka SSH dan TIDAK membuka socket forward — itu terjadi di
            endpoint SSE.
Tidak     : container_id lengkap tidak diperlakukan sebagai secret tapi juga
dikembalikan  tidak ditampilkan penuh tanpa alasan; private key, token registry,
              path socket forward, path known_hosts, path kunci age — tidak pernah
```

### GET /apps/{id}/logs/isi

Fragmen HTML: satu tarikan `docker logs --tail N` (tanpa `--follow`), opsional
disaring `q`. Dipakai untuk pencarian dan untuk memuat ulang histori runtime.

```
Akses     : session cookie (router terlindungi)
Request   : id   → path param → wajib
            tail → query, opsional → default 200, maksimum 2000
            q    → query, opsional → filter baris; maksimum 500 baris cocok
Sukses    : 200 OK — FRAGMEN HTML daftar baris (tanpa app shell)
Error     : 303 See Other → Location: /login
            404 → id app tidak dikenal
            409 → tidak ada deployment live / container_id NULL → fragmen state
                  "belum ada container yang berjalan"
            502 → container sudah tidak ada di server (Docker membalas 404
                  untuk container itu) → fragmen kategori "container sudah tidak
                  ada; log runtimenya tidak bisa ditampilkan lagi" + saran
                  melihat log deploy terakhir
            504 → salah satu TAHAP melewati batasnya (SSH+forward 10 detik,
                  chunk pertama 15 detik) → fragmen kategori "server tidak
                  merespons", tanpa stderr mentah
Efek      : SSH connect mode Strict → forward socket Docker → satu panggilan
            docker logs (tanpa follow) → forward DITUTUP sebelum handler
            mengembalikan respons. Tidak menulis db, tidak menulis file.
Tidak     : stderr ssh/docker mentah, exit code telanjang tanpa konteks, path
dikembalikan  socket forward, path file kunci sementara, private key
```

### GET /events/log/runtime/{id}

SSE log runtime langsung dari `docker logs --follow` lewat socket yang
di-forward SSH. `{id}` adalah id **app**, bukan id container.

```
Akses     : session cookie (router terlindungi). WAJIB terautentikasi.
Request   : id   → path param → wajib, harus ada di tabel apps
            tail → query, opsional → default 200, maksimum 2000 (histori awal
                   yang diminta ke Docker lewat --tail)
Sukses    : 200 OK — Content-Type: text/event-stream
            Tiap event membawa fragmen HTML baris log yang di-append HTMX.
            Event khusus: "tertinggal" (subscriber lag) dan "selesai".
            Stream ditutup server pada TIGA sebab, ketiganya lewat jalur bersih
            yang sama (tutup forward + tutup sesi SSH + lepas izin Semaphore):
              1. klien terputus
              2. batas 30 menit satu sesi tercapai → event "selesai" dengan
                 pesan "sesi log dihentikan setelah 30 menit; muat ulang untuk
                 melanjutkan"
              3. stream Docker berakhir (container berhenti/dihapus) → event
                 "selesai" dengan pesan kategori yang sesuai
            Sunyi TANPA baris baru BUKAN error — keep-alive SSE yang menjaga
            koneksi, tidak ada timeout global (docs/prd.md §3 nomor 11).
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            404 → id app tidak dikenal
            409 → tidak ada deployment live / container_id NULL → tidak membuka
                  stream sama sekali
            429 → empat sesi log runtime sudah berjalan (batas Semaphore) →
                  pesan "terlalu banyak sesi log terbuka, tutup salah satu tab
                  lalu coba lagi"
            502 → container sudah tidak ada di server target
            504 → tahap SSH/forward (10 detik) atau chunk pertama (15 detik)
                  lewat batas
Efek      : SSH connect Strict + forward socket Docker + stream bollard. TIDAK
            menulis db, TIDAK menulis file log (log runtime tidak dipersistensi
            di control plane), TIDAK mengubah state container apa pun — endpoint
            ini murni membaca. Socket forward WAJIB ditutup di ketiga jalur
            penutupan; forward yang bocor adalah kebocoran fd di /run.
Tidak     : private key, token registry, stderr ssh mentah, path socket forward,
dikembalikan  path known_hosts aplikasi, path file kunci age, nama file log
```

### GET /assets/xterm.min.js dan GET /assets/xterm.min.css

**Keberadaan kedua endpoint ini bergantung pada `docs/plan.md` Q1.** Kalau Q1
dijawab "(c) tanpa xterm.js", keduanya tidak dibuat dan bagian ini dihapus dari
kontrak oleh **planner** — bukan oleh implementer. Kalau Q1 dijawab "(b) CDN"
(ditolak planner), bagian ini juga dihapus.

```
Akses     : publik (aset statis; tidak memuat data pengguna sama sekali)
Request   : tidak ada
Sukses    : 200 OK — Content-Type: application/javascript (js) / text/css (css),
            isi file yang di-embed ke binary saat kompilasi (pola persis
            src/routes/assets.rs:11-24). Boleh mengirim header cache panjang.
Error     : 404 → path aset tidak dikenal
Efek      : tidak ada; tidak menyentuh db, tidak menyentuh server target
Tidak     : tidak ada data aplikasi apa pun. Endpoint ini TIDAK boleh berubah
dikembalikan  menjadi penyaji file umum — tidak ada path param, tidak ada
              kemungkinan path traversal. Daftar aset tetap saat kompilasi, dan
              TIDAK ADA addon xterm (fit/search/weblinks) yang ikut disajikan.
```

### GET /deployments/{id} dan GET /apps/{id} (perubahan, bukan endpoint baru)

```
Akses     : session cookie (router terlindungi) — TIDAK BERUBAH
Request   : tidak ada — TIDAK BERUBAH
Sukses    : 200 OK — HTML; /deployments/{id} bertambah tautan "lihat log
            lengkap" ke /deployments/{id}/log. /apps/{id} sekarang punya tiga
            tab (Overview / Deployments / Logs); Overview tidak berubah isinya.
Error     : sama seperti sebelumnya (303 ke /login, 404 untuk id tidak dikenal)
Efek      : /deployments/{id} bertambah satu SELECT ringkas ke deployment_logs
            (untuk tahu apakah tautan log perlu ditampilkan). Tidak menulis.
Tidak     : sama seperti sebelumnya, ditambah: path file log tidak pernah muncul
dikembalikan  di HTML mana pun
```

---

## Fase 4 — Pengelolaan environment

Kedua endpoint di bawah menambah tab ke-4 (`Environment`) pada `GET /apps/{id}`
(tabnya sendiri tidak berubah). `docs/plan.md` "Pertanyaan terbuka" Q1 dijawab
manusia: env dikirim ke container lewat `bollard` `ContainerCreateBody.env`
(field API) — nilainya akan terlihat lewat `docker inspect` di server target,
batasan Docker Engine API itu sendiri, BUKAN kebocoran platform (didokumentasikan
eksplisit, bukan diam-diam dianggap terpenuhi).

### GET /apps/{id}/env

```
Akses     : session cookie (router terlindungi)
Request   : tidak ada
Sukses    : 200 OK — HTML tabel env var. Baris `is_secret=true` menampilkan
            input KOSONG berplaceholder "•••••••• (kosongkan untuk tidak
            mengganti)" — TIDAK PERNAH nilai aslinya. Baris non-secret
            menampilkan plaintext di dalam input (boleh diedit langsung).
            Baris tambah inline (jumlah tetap, lihat ENV_NEW_ROW_SLOTS di
            src/routes/apps.rs) selalu kosong.
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            404 → id app tidak dikenal
Efek      : SELECT env_vars (dan dekripsi HANYA baris non-secret via
            state.crypto — baris secret tidak pernah menyentuh CryptoKey.decrypt
            di jalur GET ini). Tidak menulis apa pun.
Tidak     : value_encrypted mentah, plaintext baris is_secret=true, isi
dikembalikan  snapshot_encrypted mana pun
```

### POST /apps/{id}/env

```
Akses     : session cookie + CSRF (router terlindungi)
Request   : form-urlencoded dinamis (bukan struct tetap) —
              csrf_token
              value__{key}   — satu per baris env EXISTING; kosong = tidak
                                diubah (berlaku SAMA untuk secret maupun
                                non-secret — field yang MEMANG harus diset ke
                                string kosong tidak bisa lewat form ini,
                                simplifikasi disengaja, lihat docs/plan.md)
              delete__{key}  — checkbox, kalau ada = hapus baris itu
              new_key_{i}, new_value_{i}, new_secret_{i} — i = 0..N-1 slot
                                baris baru (N = ENV_NEW_ROW_SLOTS)
            Validasi: CSRF wajib cocok sesi (400 kalau tidak). Key baru
            duplikat dalam satu submit → 400, TIDAK ADA yang tersimpan (bukan
            cuma yang pertama). Key baru yang sudah ada sebagai env existing →
            400. Value apa pun (existing atau baru) yang memuat `\n`/`\r` →
            400, tidak tersimpan (`KEY=VALUE` per baris tidak bisa
            merepresentasikan newline di tengah value tanpa ambiguitas).
Sukses    : 200 OK — HTML tab Environment dirender ulang dengan banner pesan:
              - app belum pernah dideploy → env tersimpan, tidak ada redeploy
              - app punya deployment live → env tersimpan + deployment BARU
                `queued` dibuat dengan `image_digest` IDENTIK deployment live
                sebelumnya, `trigger_source='env'`, `env_version_id` menunjuk
                snapshot baru
Error     : 303 See Other → Location: /login → tidak ada sesi valid
            400 → CSRF salah / key duplikat / key sudah ada / value memuat
                  baris baru — NOL efek samping di db pada SEMUA kasus ini
            404 → id app tidak dikenal
            409 → app sedang dalam proses deploy lain (lock aktif) — ENV TETAP
                  TERSIMPAN (state "sedang diedit" berhasil), hanya redeploy
                  yang ditunda; body tetap tab Environment dengan pesan yang
                  menyebut ini eksplisit, bukan generik
Efek      : UPDATE/INSERT/DELETE env_vars per baris yang berubah (masing-masing
            statement tunggal, atomik sendiri) → SATU transaksi mencakup INSERT
            env_versions (snapshot SELURUH env app, bukan cuma yang berubah)
            + [kalau ada deployment live DAN lock berhasil diambil] INSERT
            deployments + INSERT jobs. Lock diambil (kalau relevan) SEBELUM
            transaksi dibuka — db_write max_connections(1), transaksi yang
            menahan koneksi lalu memanggil acquire_lock (minta koneksi lain
            dari pool yang sama) macet permanen; urutan ini WAJIB, bukan gaya.
Tidak     : value_encrypted/snapshot_encrypted mentah, plaintext value SECRET
dikembalikan  APA PUN (baik yang lama maupun yang baru disimpan) — hanya
               banner status yang dikembalikan, tidak pernah nilai
```

---

## Fase 5 — Keandalan dan rollback

### POST /apps/{id}/rollback

```
Akses     : session cookie + CSRF, route protected
Request   : csrf_token, deployment_id target, env_version_id pilihan,
            idempotency_key opaque
Validasi  : app dan deployment target harus cocok; env_version harus milik app;
            digest selalu diambil dari deployment target di database; tidak ada
            digest bebas dari client.
Sukses    : 303 ke detail app/deployment; deployment baru dibuat dengan
            trigger_source='rollback', status queued, dan job dalam satu transaksi.
Error     : 303 login, 404 app/deployment tidak dikenal, 409 lock aktif atau
            idempotency conflict, 422 image/env tidak tersedia.
Tidak     : deployment lama diubah, env plaintext/secret dikembalikan, rollback
            otomatis dijalankan oleh reconciliation.
```

### GET /apps/{id}/reconciliation

```
Akses     : session cookie, route protected
Sukses    : 200 HTML berisi finding aktif/terbaru: kategori, severity, status,
            waktu observasi, dan metadata aman.
Tidak     : stderr mentah, path, credential, environment, atau output Docker mentah.
```

### POST /apps/{id}/reconciliation/{finding_id}/acknowledge

```
Akses     : session cookie + CSRF
Sukses    : 303 kembali ke app; hanya mengubah status finding menjadi acknowledged.
Tidak     : tidak menghentikan, menghapus, membuat, atau mengubah container.
```

### GET/POST /settings/notifications

```
GET       : status webhook, event aktif, URL dimasking, dan metadata delivery.
POST      : csrf_token, nama, URL, event, signing secret; URL dan secret wajib
            dienkripsi sebelum disimpan.
Tidak     : URL/secret dikembalikan ulang, payload webhook berisi secret/env/log.
```

### POST /settings/notifications/test

```
Akses     : session cookie + CSRF
Sukses    : delivery test masuk queue; tidak memicu deployment dan tidak memakai
            payload produksi.
```
