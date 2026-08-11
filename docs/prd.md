# PRD — Konsol Server Pribadi

Status: Draft 1
Terakhir diubah: 2026-08-09

---

## 1. Ringkasan produk

### 1.1 Masalah

Mengelola 3–8 VPS berarti SSH ke setiap mesin satu per satu untuk pekerjaan yang berulang: cek disk penuh, prune Docker, lihat kenapa container restart, deploy versi baru, cek log, restart layanan. Tidak ada tempat tunggal untuk melihat keadaan seluruh armada.

### 1.2 North star

**Kurangi jumlah sesi SSH manual menjadi mendekati nol.**

Ini satu-satunya metrik yang menentukan apakah sebuah fitur masuk. Untuk setiap usulan fitur, pertanyaannya: *berapa sesi SSH yang dihilangkan fitur ini?* Kalau jawabannya nol, fitur itu ditolak atau ditunda.

### 1.3 Pengguna

Satu developer. Bukan tim, bukan multi-tenant. Dua mode pemakaian yang sama pentingnya:

- **Pemindaian tenang** — "apakah semuanya baik-baik saja?" Dilakukan sambil lalu, beberapa kali sehari.
- **Debug mendesak** — "apa yang rusak dan kenapa?" Jam 11 malam, dengan kesabaran nol.

### 1.4 Prinsip produk

1. **Kegagalan tidak boleh membuat keadaan lebih buruk.** Deploy yang gagal harus meninggalkan aplikasi persis seperti sebelumnya.
2. **Server adalah sumber kebenaran, bukan database.** Saat ragu, tanya server. Jangan menebak dari state lokal.
3. **Tampilkan penyimpangan, jangan perbaiki otomatis.** Auto-healing membuat tool berubah dari membantu jadi menakutkan.
4. **Control plane mati tidak boleh berarti aplikasi mati.**
5. **Satu primitif: OCI image.** Semua yang di-deploy adalah container. Tidak ada jalur kedua.
6. **Selalu ada pintu darurat.** Kotak "jalankan perintah" dan `exec` ke container membuat sisa kasus tetap tertangani tanpa membangun fitur untuk setiap kemungkinan.

### 1.5 Bukan tujuan (non-goals)

Daftar ini sama mengikatnya dengan daftar fitur. Menambah sesuatu ke sini butuh keputusan sadar, bukan improvisasi di tengah fase.

| Bukan tujuan | Alasan |
|---|---|
| Multi-tenant / RBAC / tim | Pengguna tunggal. Menambah ini menggandakan kompleksitas auth dan data model. |
| Membangun image sendiri | CI yang membangun. Menghapus builder memangkas ~40% pekerjaan. |
| Mengelola database sebagai resource | Backup, restore, dan upgrade versi adalah lubang tanpa dasar. |
| Kubernetes / Docker Swarm | Container biasa cukup untuk skala ini. |
| Object storage / S3 | Backup lokal + rsync sudah cukup. |
| Grafana kecil-kecilan | Kalau butuh query bebas, pasang Grafana. |
| Terminal web penuh / file browser | Ini jalan menuju membangun ulang Cockpit dengan lebih buruk. |
| Preview deployment per PR | Pasang wildcard cert supaya pintunya terbuka, jangan bangun fiturnya. |

### 1.6 Keputusan teknis yang sudah dikunci

Peran mana pun tidak boleh mengubah ini tanpa keputusan eksplisit di luar fase berjalan.

| Area | Keputusan |
|---|---|
| Bahasa | Rust |
| Web framework | Axum |
| Database | SQLite (WAL), `sqlx` |
| Template | Maud + HTMX + SSE. Tanpa WASM, tanpa SPA. |
| Antrean job | Tabel SQLite, worker in-process |
| Koneksi server | SSH via crate `openssh` (ControlMaster) |
| Docker | `bollard` lewat socket yang di-forward SSH |
| Registry | GHCR (dan registry lain lewat konfigurasi) |
| Reverse proxy | Traefik dengan Docker label |
| Enkripsi secret | crate `age`, kunci di file terpisah `0600` |
| Log runtime | File di disk, **bukan** SQLite |
| Identitas image | Digest (`sha256:…`), **bukan** tag |

---

## 2. Peran dan batas tanggung jawab

Sembilan peran. Yang paling sering merusak proyek adalah peran yang melebar; bagian "tidak boleh" sama pentingnya dengan "bertanggung jawab atas".

### Planner

**Bertanggung jawab atas:** memecah fase jadi tugas berurutan, menetapkan kriteria masuk dan keluar fase, memutuskan urutan pengerjaan, menolak pekerjaan yang di luar cakupan fase, menjadi gerbang akhir sebelum fase dinyatakan selesai.

**Tidak boleh:** menulis kode produksi, menambah fitur yang tidak ada di PRD ini, memulai fase berikutnya sebelum fase sekarang lulus gerbang.

**Keluaran:** daftar tugas berurutan per fase, catatan keputusan, laporan gerbang fase.

### UI/UX

**Bertanggung jawab atas:** spesifikasi tiap layar termasuk **semua state** (kosong, memuat, sebagian terisi, error, tidak terjangkau), seluruh teks antarmuka, kontrak interaksi (apa yang terjadi saat diklik, apa yang muncul saat gagal), hierarki informasi.

**Tidak boleh:** menulis template atau CSS, menentukan bentuk endpoint API, menambah layar yang tidak ada di fase berjalan.

**Keluaran:** spesifikasi layar per fase, tabel state, daftar teks antarmuka.

**Aturan tetap:** setiap layar wajib punya spesifikasi state kosong dan state error sebelum Frontend mulai. Ini yang paling sering dilewat dan paling mahal ditambal belakangan.

### Migration

**Bertanggung jawab atas:** skema SQLite, file migrasi berurutan, indeks, pragma, kebijakan retensi dan rollup data, seed data untuk pengembangan.

**Tidak boleh:** menulis query domain (itu milik Backend), mengubah migrasi yang sudah dijalankan (selalu buat migrasi baru), mendesain skema untuk kebutuhan fase yang belum tiba.

**Keluaran:** file migrasi, diagram skema, catatan indeks dan alasannya.

**Kenapa peran terpisah:** kesalahan skema adalah kesalahan paling mahal dalam proyek ini. Memisahkannya memaksa skema ditinjau sendiri, bukan diselipkan di tengah pekerjaan fitur.

### Backend

**Bertanggung jawab atas:** handler Axum, logika domain, worker antrean, lapisan SSH, klien Docker, mesin state deployment, pengumpul metrik, endpoint SSE.

**Tidak boleh:** mengubah skema langsung (minta ke Migration), menulis template, memutuskan teks antarmuka, memilih tindakan destruktif tanpa persetujuan Security.

**Keluaran:** kode Rust, kontrak API terdokumentasi, unit test untuk logika murni.

### Frontend

**Bertanggung jawab atas:** template Maud, atribut HTMX, langganan SSE, CSS, keadaan interaktif, aksesibilitas dasar (fokus keyboard terlihat, kontras).

**Tidak boleh:** menambah JavaScript di luar `xterm.js` untuk viewer log, mengubah bentuk respons API (minta ke Backend), mengarang teks yang tidak diberikan UI/UX.

**Keluaran:** template dan CSS, catatan perilaku HTMX.

### Security

**Bertanggung jawab atas:** peninjauan penanganan kredensial, cakupan token, jalur data secret dari input sampai ke server target, verifikasi signature webhook, keamanan sesi, izin file, permukaan serangan tiap fase.

**Tidak boleh:** memblokir fase karena risiko teoretis tanpa skenario konkret, menuntut fitur keamanan yang tidak relevan untuk instance pengguna tunggal.

**Keluaran:** catatan tinjauan per fase dengan temuan yang diberi tingkat (blocker / harus diperbaiki / catat saja).

### QA

**Bertanggung jawab atas:** rencana uji per fase, uji integrasi terhadap server nyata (atau container Docker-in-Docker), **injeksi kegagalan**, verifikasi setiap state UI benar-benar bisa dicapai.

**Tidak boleh:** memperbaiki bug yang ditemukan (serahkan ke Debugger), meloloskan fase yang jalur kegagalannya belum diuji.

**Keluaran:** rencana uji, hasil eksekusi, laporan bug dengan langkah reproduksi.

**Aturan tetap:** setiap fase wajib punya minimal tiga skenario injeksi kegagalan. Jalur bahagia saja tidak pernah cukup untuk tool deployment.

### Debugger

**Bertanggung jawab atas:** mereproduksi bug dari QA, menemukan akar masalah, memperbaiki, dan **menambahkan observability platform itu sendiri** (log terstruktur, konteks error, trace) supaya kelas bug yang sama lebih cepat ketahuan lain kali.

**Tidak boleh:** menambal gejala tanpa akar masalah, memperbaiki dengan mengubah cakupan, menyembunyikan error dengan `unwrap_or_default()`.

**Keluaran:** perbaikan disertai uji regresi, catatan akar masalah.

### Reviewer

**Bertanggung jawab atas:** gerbang kode sebelum masuk fase berikutnya. Memeriksa terhadap invariant di bagian 3, konvensi Rust, penanganan error, batas peran (apakah Frontend diam-diam menulis logika domain?), dan apakah pekerjaan ini benar-benar milik fase ini.

**Tidak boleh:** menulis ulang kode sendiri (kembalikan ke pemiliknya), meloloskan pelanggaran invariant dengan alasan apa pun.

**Keluaran:** catatan review, keputusan lolos atau tidak.

### Urutan serah terima dalam satu fase

```
Planner
   ├─> UI/UX ──────┐
   └─> Migration ──┤
                   ├─> Backend ──> Frontend
                                      │
                          Security ───┤
                             QA ──────┤
                                      ├─> Debugger (kalau ada temuan)
                                      └─> Reviewer ──> Planner (gerbang)
```

UI/UX dan Migration berjalan paralel dan **harus selesai sebelum Backend mulai**. Ini bukan formalitas — keduanya adalah keputusan yang paling mahal diubah belakangan.

---

## 3. Invariant lintas fase

Reviewer memeriksa daftar ini di setiap gerbang fase. Pelanggaran adalah blocker, tanpa pengecualian.

1. Tidak pernah ada tindakan destruktif karena server tidak terjangkau.
2. Container selalu dijalankan dengan `--restart unless-stopped`.
3. Container selalu diberi label `platform.app`, `platform.deployment`, `platform.digest`.
4. Image selalu dirujuk dengan digest, tidak pernah dengan tag.
5. Log container yang gagal ditangkap **sebelum** container dihapus.
6. Env var dikirim lewat `--env-file` (`0600`), tidak pernah lewat `-e`.
7. Nilai secret tidak pernah dikembalikan ke klien setelah disimpan.
8. Kunci enkripsi tidak pernah berada di dalam database atau di direktori backup.
9. Baris log tidak pernah ditulis ke SQLite.
10. Setiap tulisan ke SQLite dalam satu siklus dibungkus satu transaksi.
11. Setiap operasi jarak jauh punya timeout per tahap, bukan timeout global.
12. Setiap kunci (lock) punya waktu kedaluwarsa.
13. Docker socket tidak pernah diekspos lewat TCP.
14. Health check menembak IP container langsung, tidak lewat domain atau proxy.

---

## 4. Fase

Ringkasan urutan dan alasannya:

| Fase | Nama | Kenapa di sini |
|---|---|---|
| 0 | Fondasi | Semua fase butuh ini |
| 1 | Registry server dan konektivitas | Deploy butuh server terdaftar. Ini juga inti north star. |
| 2 | Loop deploy | Prioritas utama pengguna |
| 3 | Log dan riwayat | Yang membuat platform terasa hidup |
| 4 | Pengelolaan environment | Butuh loop deploy sudah stabil |
| 5 | Keandalan dan rollback | Butuh riwayat dari fase 3 |
| 6 | Metrik dan pemantauan | Beban tulis berbeda, jangan campur lebih awal |
| 7 | Operasi armada dan pintu darurat | Menyerang north star paling langsung, tapi butuh semua di atas |

---

### Fase 0 — Fondasi

**Tujuan.** Satu binary yang bisa dijalankan, menyimpan state, dan melayani halaman terautentikasi. Belum ada fitur produk.

**Definition of done.** `cargo run` menghasilkan server yang bisa login, punya database ter-migrasi, menulis log terstruktur, dan mati dengan bersih saat SIGTERM.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Tetapkan struktur direktori dan batas modul. Kunci daftar dependensi awal. |
| **Migration** | Setup `sqlx` migrate. Migrasi 0001: tabel `settings`, `sessions`. Pragma WAL, `busy_timeout`, `foreign_keys`, `synchronous=NORMAL`. Pola dua pool (tulis `max_connections(1)`, baca banyak). |
| **Backend** | Skeleton Axum. Muat konfigurasi dari env dan file. Login pengguna tunggal dengan hash Argon2. Middleware sesi berbasis cookie. Endpoint `/healthz`. Graceful shutdown. `tracing` dengan keluaran JSON. |
| **Frontend** | Layout dasar: sidebar, header, area konten. Token CSS dari arah visual. Halaman login. Halaman error 404 dan 500. |
| **UI/UX** | Spesifikasi layar login dan shell aplikasi. Tetapkan token warna, tipografi, dan spasi final. Tulis teks untuk error autentikasi. |
| **Security** | Tinjau: hash password, flag cookie (`HttpOnly`, `Secure`, `SameSite=Lax`), rotasi sesi setelah login, izin file `0600` pada db dan kunci. Pastikan kunci enkripsi dimuat dari luar db. |
| **QA** | Verifikasi: login salah ditolak, sesi kedaluwarsa, database terbuat dari nol, migrasi idempoten, SIGTERM tidak merusak db. |
| **Debugger** | Siapkan konteks error dengan `anyhow` plus tipe error domain. Pastikan setiap error tercatat dengan cukup konteks untuk ditelusuri. |
| **Reviewer** | Periksa struktur modul tidak bocor (handler tidak berisi logika domain), tidak ada `unwrap()` di jalur request. |

**Tidak dikerjakan:** apa pun yang menyentuh server jarak jauh.

---

### Fase 1 — Registry server dan konektivitas

**Tujuan.** Bisa mendaftarkan server, memverifikasi kesiapannya, dan melihat seluruh armada dalam satu layar.

**Definition of done.** Dari UI, pengguna menambahkan server dengan kredensial SSH, sistem memverifikasi konektivitas dan Docker, lalu server muncul di fleet strip dengan status yang diperbarui otomatis.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Urutkan: koneksi → verifikasi → login registry → polling status. Tetapkan interval polling dan kebijakan backoff. |
| **Migration** | Migrasi 0002: `servers` (id, name, host, port, ssh_user, ssh_key_encrypted, status, last_seen_at, docker_version, os_info), `registries` (id, host, username, token_encrypted), `server_registries`. |
| **Backend** | Pool koneksi `openssh` dengan ControlMaster. Uji koneksi dengan timeout 10 detik. Deteksi Docker dan versinya. Forward socket Docker, sambungkan `bollard`, verifikasi ping. Kelola `docker login` di server target. Job polling status dengan backoff eksponensial (1, 2, 4, 8, berhenti di 15 menit). Tandai `unreachable` setelah tiga kegagalan berturut-turut. |
| **Frontend** | Layar fleet overview dengan tabel server. Fleet strip yang menempel di semua halaman. Alur tambah server tiga langkah dengan checklist verifikasi yang terisi langsung lewat SSE. Layar detail server (kerangka, belum ada grafik). |
| **UI/UX** | Spesifikasi alur tambah server termasuk **setiap** kegagalan: host tidak terjangkau, autentikasi ditolak, Docker tidak terpasang, pengguna tanpa akses Docker, registry login gagal. Setiap pesan error harus menyebut langkah perbaikannya. State kosong fleet. Perilaku baris server tidak terjangkau. |
| **Security** | **Fase paling kritis untuk Security.** Tinjau: penyimpanan private key SSH terenkripsi, kunci tidak pernah dikembalikan ke klien, verifikasi host key (putuskan kebijakan TOFU dan tampilkan fingerprint), token registry terpisah dari token kode, izin `~/.docker/config.json` di target, socket forward tidak bocor ke jaringan. |
| **QA** | Injeksi kegagalan: matikan server di tengah polling, putuskan jaringan saat verifikasi, berikan key salah, berikan host tanpa Docker, hentikan daemon Docker saat terhubung. Verifikasi backoff benar-benar melambat dan tidak membanjiri server mati. |
| **Debugger** | Pastikan error SSH bisa dibedakan antara gagal koneksi dan gagal perintah remote — ini kelemahan bawaan crate `openssh` dan harus ditangani eksplisit dengan memisahkan exit code dan stderr. |
| **Reviewer** | Invariant 1, 13. Pastikan tidak ada perintah remote tanpa timeout. |

**Tidak dikerjakan:** metrik (fase 6), aplikasi, deploy.

---

### Fase 2 — Loop deploy

**Tujuan.** Satu perintah dari CI menghasilkan container baru yang melayani traffic, tanpa downtime.

**Definition of done.** `POST /api/v1/deploy` dengan digest menyebabkan image ditarik, container baru dijalankan, health check lolos, traffic pindah, container lama berhenti. Gagal di tahap mana pun meninggalkan container lama tetap melayani.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Definisikan mesin state deployment: `queued → pulling → starting → checking → live`, dengan cabang `failed`, `cancelled`, `unknown`. Tetapkan timeout tiap tahap. |
| **Migration** | Migrasi 0003: `apps` (id, server_id, name, health_path, health_grace_seconds, port, restart_policy), `domains`, `deployments` (id, app_id, image_digest, image_ref, commit_sha, status, stage, trigger_source, heartbeat_at, started_at, finished_at, error_kind, error_detail), `deploy_tokens`. |
| **Backend** | Endpoint deploy dengan autentikasi bearer. Worker antrean yang mengambil job. Pull image dengan deteksi kemacetan (batal kalau tidak ada progres byte 60 detik). Jalankan container dengan label wajib dan `--restart unless-stopped`. Health check ke IP container langsung dengan grace period terpisah. Konfigurasi label Traefik. Hentikan container lama setelah baru sehat. Tangkap log container yang gagal sebelum menghapusnya. |
| **Frontend** | Daftar aplikasi. Detail aplikasi tab Overview. Layar detail deployment dengan timeline tahap yang bergerak langsung lewat SSE. Alur tambah aplikasi. |
| **UI/UX** | Spesifikasi timeline tahap termasuk tampilan setiap kegagalan. **Bedakan tiga mode kegagalan health check** dengan pesan berbeda: container keluar (tampilkan exit code dan 50 baris terakhir), container jalan tapi balas non-2xx (tampilkan body), container jalan tapi tidak merespons (sarankan bind ke `0.0.0.0`, bukan `127.0.0.1`). |
| **Security** | Tinjau: pembuatan dan penyimpanan deploy token, token per aplikasi bukan global, rate limit endpoint deploy, validasi format digest (tolak tag), pastikan endpoint deploy tidak bisa dipakai menarik image sembarangan dari registry mana pun. |
| **QA** | Injeksi kegagalan: image tidak ada, pull terputus di tengah, container keluar segera, health check selalu gagal, health check lolos setelah grace period habis, dua deploy bersamaan untuk app yang sama, port sudah dipakai. Verifikasi container lama tetap hidup di semua kasus gagal. |
| **Debugger** | Pastikan setiap kegagalan menyimpan `error_kind` yang bisa dikategorikan, bukan hanya string. Ini yang membuat UI bisa memberi saran spesifik. |
| **Reviewer** | Invariant 1–6, 11, 14. Periksa urutan tangkap-log-sebelum-hapus benar-benar tidak bisa dilewati. |

**Tidak dikerjakan:** rollback (fase 5), env dari UI (fase 4), streaming log runtime (fase 3).

---

### Fase 3 — Log dan riwayat

**Tujuan.** Semua yang perlu dilihat saat sesuatu rusak tersedia tanpa SSH.

**Definition of done.** Log deploy streaming langsung, log runtime bisa di-tail dan dicari, riwayat deployment lengkap dengan digest dan commit.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Tetapkan kebijakan retensi log (default 30 hari) dan batas ukuran file. |
| **Migration** | Migrasi 0004: `deployment_logs` menyimpan **path file dan metadata saja**, tidak pernah baris log. Kolom: deployment_id, path, size_bytes, line_count, truncated. |
| **Backend** | Tulis log deploy ke `/var/lib/platform/logs/{deployment_id}.log`. Satu `tokio::sync::broadcast` per deployment aktif, disimpan di `DashMap<DeploymentId, Sender>`, dengan writer paralel yang mem-persist ke file. Endpoint SSE untuk log langsung. Tail file untuk histori. Stream log runtime dari `docker logs --follow --tail` lewat socket yang di-forward. Job rotasi dan pembersihan log. |
| **Frontend** | Viewer log sebagai komponen utama: monospace, gutter timestamp, warna ANSI (`xterm.js`), toggle wrap, toggle follow, pencarian, unduh. Tab Deployments dengan daftar riwayat. Tab Logs pada detail aplikasi. |
| **UI/UX** | Spesifikasi viewer log untuk semua kondisi: sedang streaming, streaming terputus lalu tersambung lagi, log kosong, log terpotong karena batas ukuran, container sudah tidak ada. Perilaku follow saat pengguna men-scroll ke atas (harus berhenti follow, dengan tombol "kembali ke bawah"). |
| **Security** | Tinjau: log bisa memuat secret yang tercetak aplikasi — putuskan apakah menyaring pola umum atau memperingatkan pengguna. Pastikan path log tidak bisa ditembus traversal. Pastikan endpoint SSE terautentikasi. |
| **QA** | Injeksi kegagalan: putuskan koneksi SSE di tengah stream, deploy menghasilkan 100 ribu baris, container dihapus saat log sedang di-tail, reload halaman di tengah deploy, buka dua tab pada deployment yang sama. |
| **Debugger** | Kelola lifetime broadcast channel — pastikan channel dibersihkan saat deployment selesai dan tidak bocor saat klien terputus mendadak. Ini sumber kebocoran memori paling mungkin di proyek ini. |
| **Reviewer** | Invariant 9. Periksa tidak ada baris log yang menyentuh SQLite di jalur mana pun. |

---

### Fase 4 — Pengelolaan environment

**Tujuan.** Mengubah konfigurasi aplikasi tanpa SSH dan tanpa commit.

**Definition of done.** Env bisa diedit dari UI, perubahan menghasilkan deployment baru dengan digest yang sama, secret tidak pernah terbaca lagi setelah disimpan.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Putuskan perilaku env saat rollback (default: pakai env terbaru, tampilkan diff, sediakan opsi env asli). |
| **Migration** | Migrasi 0005: `env_vars` (app_id, key, value_encrypted, is_secret, updated_at), `env_versions` (app_id, version, snapshot_encrypted, note, created_at). Tambah `env_version_id` ke `deployments`. |
| **Backend** | CRUD env dengan enkripsi `age`. Buat snapshot versi baru setiap perubahan disimpan. Tulis `--env-file` dengan izin `0600` di server target sebelum menjalankan container. Hapus file env lama setelah pergantian selesai. Deploy yang dipicu perubahan env memakai digest yang sedang berjalan. |
| **Frontend** | Tab Environment dengan tabel dan baris tambah inline. Bar sticky "3 variabel berubah" dengan tombol simpan-dan-deploy. Tampilan diff. Field secret bertopeng dengan tombol Replace. |
| **UI/UX** | Buat konsekuensi eksplisit: menyimpan env berarti me-restart aplikasi. Bar perubahan harus mustahil dilewatkan. Spesifikasi tampilan diff, termasuk cara menampilkan perubahan nilai secret tanpa membocorkan nilainya. |
| **Security** | **Fase kritis kedua untuk Security.** Tinjau: enkripsi at rest, kunci di luar database, secret tidak pernah ada di respons API, tidak pernah tercatat di log, tidak muncul di `docker inspect`, file env dihapus setelah dipakai, snapshot lama tetap terenkripsi. |
| **QA** | Injeksi kegagalan: deploy gagal setelah env diubah (env versi mana yang aktif?), simpan env saat deploy sedang berjalan, key duplikat, nilai dengan newline dan karakter khusus, nilai sangat panjang. |
| **Debugger** | Pastikan kegagalan dekripsi memberi pesan yang jelas (kunci hilang atau salah), bukan panic. |
| **Reviewer** | Invariant 6, 7, 8. Cari kebocoran secret di setiap jalur log dan respons. |

---

### Fase 5 — Keandalan dan rollback

**Tujuan.** Sistem tetap benar saat control plane crash, koneksi putus, atau deploy macet.

**Definition of done.** Rollback dalam hitungan detik. Control plane yang di-restart di tengah deploy memulihkan diri dengan bertanya ke server, bukan menebak.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Tetapkan kebijakan retensi image (5 terakhir per aplikasi). Definisikan aturan rekonsiliasi: apa yang ditampilkan, apa yang tidak pernah diperbaiki otomatis. |
| **Migration** | Migrasi 0006: kolom `lock_token` dan `lock_expires_at` pada `apps`. Tabel `reconciliation_findings`. Indeks pada `deployments(app_id, created_at)`. |
| **Backend** | Kunci deploy per aplikasi dengan kedaluwarsa. Heartbeat worker tiap 10 detik. Saat boot, tandai deployment dengan heartbeat basi sebagai `unknown` lalu rekonsiliasi dengan membaca label container di server. Rollback: jalankan ulang tahap deploy dengan digest dan versi env pilihan. Job rekonsiliasi periodik. Pembersihan image yang menghormati retensi. Notifikasi webhook saat gagal dan saat pulih. |
| **Frontend** | Tombol dan dialog rollback dengan perbandingan digest dan peringatan perubahan env. Banner penyimpangan dari rekonsiliasi. Halaman pengaturan notifikasi. |
| **UI/UX** | Spesifikasi dialog rollback termasuk pilihan env. Spesifikasi banner penyimpangan — harus informatif, tidak menakutkan, dan **tidak menawarkan perbaikan otomatis**. Tampilan deployment berstatus `unknown`. |
| **Security** | Tinjau: URL webhook disimpan terenkripsi, payload notifikasi tidak memuat secret, endpoint rollback terautentikasi dan tidak bisa dipicu lintas aplikasi. |
| **QA** | Injeksi kegagalan: bunuh control plane di setiap tahap deploy lalu restart, putuskan SSH tepat setelah `docker run` dikirim, rollback ke deployment yang image-nya sudah dihapus, dua rollback bersamaan, ubah container manual di server lalu jalankan rekonsiliasi. |
| **Debugger** | Fokus pada kasus SSH terputus setelah perintah terkirim — verifikasi sistem benar-benar bertanya ke server dan tidak pernah berasumsi. |
| **Reviewer** | Invariant 1, 2, 3, 12. Periksa tidak ada jalur yang melakukan perbaikan otomatis. |

---

### Fase 6 — Metrik dan pemantauan

**Tujuan.** Tahu keadaan sumber daya tiap server dan tiap aplikasi, dikorelasikan dengan deployment.

**Definition of done.** Grafik CPU, memori, disk per host dan per container, dengan penanda deployment, dan tiga alert yang benar-benar berguna.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Kunci kebijakan downsampling sebelum kode ditulis. Tetapkan tiga alert yang dibangun dan tolak sisanya. |
| **Migration** | Migrasi 0007: `metrics_host` dan `metrics_container` dengan `PRIMARY KEY (ts, …) WITHOUT ROWID`. Tabel rollup menit dan jam. Kolom `source` untuk menampung agen di masa depan. Job retensi: mentah 6 jam, menit 7 hari, jam 1 tahun. |
| **Backend** | Poll host lewat satu perintah SSH (`/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, `df`). Hitung delta CPU dari penghitung kumulatif. Poll container lewat `/containers/{id}/stats?stream=false&one-shot=true`. **Kurangi `inactive_file` dari memory usage.** Kalikan CPU dengan jumlah core. Satu transaksi per siklus poll. Job rollup tiap menit menyimpan `avg` dan `max`. Tiga alert: disk di atas 80%, container restart berulang, kenaikan sumber daya lebih dari 30% setelah deploy. |
| **Frontend** | Panel metrik pada detail server. Grafik dengan garis penanda deployment. Sparkline pada fleet strip. Pemilih rentang waktu. Tampilan alert. |
| **UI/UX** | Spesifikasi grafik: sumbu, satuan, perilaku saat data kosong atau berlubang (server sempat tidak terjangkau). Penanda deployment harus terbaca jelas — ini nilai utama fitur ini. Format alert. |
| **Security** | Tinjau: endpoint metrik tidak membocorkan daftar proses atau isi lingkungan sistem. Kalau endpoint `POST /api/v1/metrics` dibuat untuk agen masa depan, pastikan terautentikasi sejak awal. |
| **QA** | Verifikasi: nilai CPU dan memori cocok dengan `docker stats` di server (ini uji paling penting di fase ini). Injeksi: server tidak terjangkau di tengah pengumpulan, container hilang antar poll, lonjakan tiba-tiba, database tumbuh selama seminggu. |
| **Debugger** | Verifikasi rollup tidak menghilangkan lonjakan — inilah alasan `max` disimpan bersama `avg`. |
| **Reviewer** | Invariant 10. Periksa tidak ada penulisan metrik per-baris. Periksa retensi benar-benar berjalan. |

---

### Fase 7 — Operasi armada dan pintu darurat

**Tujuan.** Menyerang north star secara langsung: bertindak pada banyak server sekaligus, dan menutup sisa kasus dengan pintu darurat.

**Definition of done.** Prune di semua server dari satu tombol. Tabel disk seluruh armada. Jalankan perintah di banyak server. `exec` ke container.

| Peran | Pekerjaan |
|---|---|
| **Planner** | Tinjau catatan "kenapa saya SSH" yang dikumpulkan selama memakai fase 1–6. **Daftar itu menentukan isi fase ini**, bukan PRD ini. Tolak apa pun yang tidak ada di daftar. |
| **Migration** | Migrasi 0008: `fleet_operations` (id, kind, targets, status, created_at), `fleet_operation_results` (operation_id, server_id, exit_code, output_path). |
| **Backend** | Eksekusi paralel lintas server dengan batas konkurensi. Kumpulkan hasil per server. Prune yang menghormati retensi image rollback. Agregasi penggunaan disk. `exec` sekali pakai ke container lewat SSE dua arah. |
| **Frontend** | Layar fleet actions: tabel disk armada, panel prune dengan multi-select dan estimasi ruang, kotak jalankan perintah dengan hasil per server yang bisa dilipat. Terminal `exec` untuk container. |
| **UI/UX** | Spesifikasi hasil parsial — operasi yang berhasil di 3 server dan gagal di 1 adalah kasus normal, bukan error. Dialog konfirmasi untuk operasi destruktif harus menyebut server mana saja yang terpengaruh. |
| **Security** | **Fase kritis ketiga untuk Security.** Kotak jalankan perintah adalah eksekusi kode arbitrer sebagai root di banyak mesin. Tinjau: perlukah konfirmasi ulang, apakah perintah dicatat ke audit log, apakah sesi `exec` punya batas waktu, apakah riwayat perintah disimpan (dan apakah itu diinginkan). |
| **QA** | Injeksi: satu server gagal di tengah operasi armada, perintah berjalan sangat lama, perintah menghasilkan keluaran sangat besar, prune saat deploy sedang berjalan (harus dicegah). |
| **Debugger** | Pastikan kegagalan pada satu server tidak membatalkan operasi di server lain. |
| **Reviewer** | Invariant 1. Periksa prune tidak pernah menghapus image yang dibutuhkan rollback. |

---

## 5. Risiko

| Risiko | Dampak | Mitigasi |
|---|---|---|
| Berhenti di 70% — deploy jalan tapi UI setengah jadi, lalu kembali ke `compose` manual | Tinggi | Fase 0–2 harus selesai dan **dipakai dua minggu tanpa menambah fitur** sebelum fase 3 dimulai |
| Cakupan membengkak, terutama di fase 6 | Tinggi | Setiap fitur diuji dengan pertanyaan "berapa sesi SSH yang dihilangkan?" |
| Fase 6 menggandakan ukuran proyek | Sedang | Fase 6 boleh ditunda tanpa batas; fase 1–5 sudah berguna sendiri |
| Skema berubah setelah data terkumpul | Sedang | Peran Migration terpisah, dan kolom `server_id`, `env_version_id`, `source` sudah disiapkan sejak awal |
| Kebocoran memori pada broadcast channel log | Sedang | Debugger memiliki tugas eksplisit di fase 3 |
| Rust plus Axum plus sqlx plus SSH async sekaligus untuk pertama kali | Sedang | Alokasikan waktu ekstra di fase 0 dan 1 |

---

## 6. Gerbang antar fase

Planner tidak boleh membuka fase berikutnya sebelum semua ini terpenuhi:

- [ ] Definition of done fase terpenuhi dan diverifikasi langsung, bukan diasumsikan
- [ ] Minimal tiga skenario injeksi kegagalan diuji dan lulus
- [ ] Setiap layar punya state kosong dan state error yang benar-benar bisa dicapai
- [ ] Tinjauan Security selesai, tanpa temuan blocker tersisa
- [ ] Review kode selesai, tanpa pelanggaran invariant
- [ ] Migrasi berjalan bersih dari database kosong
- [ ] Tidak ada `unwrap()` atau `expect()` di jalur request

**Gerbang tambahan setelah fase 2:** platform dipakai untuk deployment nyata selama dua minggu penuh sebelum fase 3 dimulai. Ini bukan formalitas — dua minggu memakainya akan mengubah prioritas fase berikutnya lebih akurat daripada perencanaan mana pun.