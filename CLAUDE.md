# CLAUDE.md — Server Console (nama sementara)

Dokumen ini adalah kontrak kerja. Kalau ada permintaan yang bertentangan dengan
bagian **Jangkar** atau **Non-Goals**, tolak dulu dan tanyakan, jangan langsung kerjakan.

---

## 1. Apa yang dibangun

Sebuah **konsol server pribadi yang kebetulan bisa deploy**.

Satu binary Rust + satu file SQLite yang mengelola beberapa VPS Linux lewat SSH:
melihat kesehatan server, mengoperasikan container, dan men-deploy image OCI
yang sudah dibangun oleh CI.

Ini **bukan** Vercel self-hosted, bukan PaaS, bukan pengganti Coolify.

## 2. Jangkar (satu-satunya alat uji fitur)

> **Berapa banyak sesi SSH manual yang dihilangkan fitur ini?**

Kalau jawabannya nol, fitur itu tidak masuk. Tidak ada pengecualian.
Kalau sebuah ide terdengar bagus tapi tidak lulus tes ini, catat di `IDEAS.md`, jangan bangun.

## 3. Non-Goals (jangan bangun, titik)

- Membangun image. **CI yang build.** Tidak ada clone repo, tidak ada `docker build`,
  tidak ada cache layer, tidak ada manajemen build secret di platform ini.
- Framework detection / Nixpacks / Railpack.
- Kubernetes, Docker Swarm, orkestrasi multi-node.
- Multi-tenant, organisasi, RBAC, GitHub App.
- Redis, PostgreSQL, object storage (S3/R2), message broker.
- Managed database sebagai resource kelas satu (backup/restore/upgrade Postgres dsb).
- Preview deployment per PR. (Tapi pasang wildcard DNS + wildcard TLS supaya pintunya terbuka.)
- Terminal web penuh, file browser, editor config. Cukup "pintu darurat" di Fase 4.
- Grafana kecil-kecilan. Query bebas bukan urusan kita.

## 4. Stack (sudah diputuskan, jangan diperdebatkan ulang)

| Bagian | Pilihan |
|---|---|
| Bahasa | Rust (edition 2021+) |
| Web + SSE | `axum` (`axum::response::sse`) |
| Template | `maud` + HTMX. **Tanpa WASM, tanpa Leptos/Dioxus.** |
| DB | SQLite via `sqlx` (compile-time checked query) |
| SSH | crate `openssh` (bungkus ssh sistem, ControlMaster multiplexing) |
| Docker API | `bollard` — **baru mulai Fase 5**, sebelum itu shell-out via SSH |
| Enkripsi | crate `age` |
| Password | `argon2` |
| Proxy di target | Traefik, Docker label discovery |
| Job queue | tabel `jobs` di SQLite, tanpa crate eksternal (~80 baris) |
| Log runtime | file di disk, **bukan** tabel SQLite |
| Migrasi DB | `sqlx::migrate!` + direktori `migrations/` |

`sqlx::query!` butuh metadata saat compile. Jalankan `cargo sqlx prepare` dan
**commit direktori `.sqlx/`**. Tanpa ini CI patah di build pertama, karena runner
tidak punya `DATABASE_URL`.

JS yang boleh ada: HTMX, dan `xterm.js` hanya kalau butuh ANSI color di viewer log.

## 5. Invariants (aturan yang tidak boleh dilanggar)

1. **Kegagalan deploy tidak boleh membuat keadaan lebih buruk dari sebelum deploy.**
   Container lama tetap melayani traffic sampai yang baru terbukti sehat.
2. **Server adalah sumber kebenaran, bukan database.** Selalu rekonsiliasi dengan
   `docker ps --filter label=platform.app=...`, jangan percaya tebakan SQLite.
3. **Jangan pernah mengambil tindakan destruktif karena server tidak terjangkau.**
   "Server mati" dan "jaringan putus" tidak bisa dibedakan. Tidak ada auto-rollback.
4. **Tampilkan drift, jangan perbaiki otomatis.** Tidak ada auto-healing.
5. **Semua container jalan dengan `--restart unless-stopped`.** Control plane mati
   tidak boleh berarti aplikasi mati.
6. **Referensi image selalu `@sha256:` digest, tidak pernah tag.**
7. **Tangkap log container yang gagal SEBELUM menghapusnya.** Selalu, tanpa kecuali.
8. **Jangan expose Docker socket lewat TCP.** Selalu lewat SSH.
9. **Kunci enkripsi tidak pernah berada di dalam database, dan tidak pernah ikut backup
   ke direktori yang sama.**
10. **Env var tidak pernah lewat `docker run -e`.** Selalu `--env-file` dengan permission 0600.
11. **Secret tidak pernah dikembalikan oleh API setelah disimpan.** Tampilkan `••••••••` + "Replace".
12. **Semua endpoint mutasi butuh auth.** Tidak ada "nanti saja".
13. **Private key SSH yang didekripsi hanya boleh menyentuh tmpfs.** Ditulis ke
    `/run/platform/ssh/` (0600), dihapus segera setelah ControlMaster berdiri.
    Tidak pernah ke disk persisten. Kalau `/run` tidak tersedia, gagal dan katakan —
    jangan diam-diam jatuh ke `/tmp`.

## 6. Layout on-disk (control plane)

```
/var/lib/platform/
├── db.sqlite
├── key.age                       (0600, JANGAN di-backup ke direktori ini)
├── logs/{deployment_id}.log
├── backups/db-YYYY-MM-DD.sqlite
└── env/                          (staging file env sebelum dikirim ke target)
```

Kunci SSH tidak punya tempat di sini — disimpan terenkripsi di kolom
`servers.ssh_key_encrypted`. Saat dipakai (Invariant 13):
```
/run/platform/ssh/{server_id}     (tmpfs, 0700 dir / 0600 file, umur hidup detik)
```
Satu kali tulis per server per uptime, bukan per perintah — ControlMaster
menahan koneksinya setelah itu.

Di server target:
```
/var/lib/platform/env/{app}.env   (0600, dihapus setelah pergantian container selesai)
```

## 7. Pragma SQLite wajib

```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
```

**Pola dua pool:** satu `SqlitePool` tulis dengan `max_connections(1)`, satu pool baca
dengan beberapa koneksi. Serialkan tulisan di level aplikasi.

## 8. Skema target (dibangun bertahap, tapi kolomnya disiapkan sejak awal)

```sql
users        id, username, password_hash, created_at
api_tokens   id, name, token_hash, created_at, last_used_at
             -- Bearer utk POST /api/v1/deploy. Simpan argon2 hash, bukan plaintext.

servers      id, name, host, port, ssh_user, ssh_key_encrypted, status,
             last_seen_at, consecutive_failures, docker_version, notes

apps         id, server_id, name, image_repo, health_path, health_grace_secs,
             port, restart_policy, lock_token, lock_expires_at, created_at

registries   id, host, username, token_encrypted           -- terpisah dari git, sengaja
git_providers id, kind(github|gitlab), host, auth_kind(pat|app),
             token_encrypted, app_installation_id          -- auth_kind murah sekarang, mahal nanti

env_vars     id, app_id, key, value_encrypted, is_secret, updated_at
env_versions id, app_id, version, snapshot_encrypted, created_at, note
             UNIQUE (app_id, version)

domains      id, app_id, host, tls_enabled

deployments  id, app_id, commit_sha, image_digest, image_tag, status,
             triggered_by, source, env_version_id, container_id,
             started_at, finished_at, heartbeat_at, log_path, error_stage, error_detail
             -- container_id: handle konkret utk drain + capture log (Invariant 7).
             -- Label saja tidak cukup saat container sudah exited.

jobs         id, kind, payload, status, run_at, started_at, attempts, last_error

metrics_host      res, ts, server_id, cpu_avg, cpu_max, mem_used, mem_max, mem_total,
                  load1, disk_used, disk_total, source
                  PRIMARY KEY (res, ts, server_id)  WITHOUT ROWID
metrics_container res, ts, container_id, app_id, cpu_avg, cpu_max,
                  mem_bytes, mem_max, mem_limit, net_rx, net_tx, source
                  PRIMARY KEY (res, ts, container_id) WITHOUT ROWID
-- res: 'raw' | 'min' | 'hour'. Retensi = DELETE WHERE res = ? AND ts < ?
-- max disimpan sejak awal, bukan hanya avg (alasannya di Fase 5).
```

Kolom yang **murah sekarang, mahal kalau diretrofit**: `apps.server_id`,
`deployments.env_version_id`, `deployments.heartbeat_at`, `metrics_*.source`,
`git_providers.auth_kind`.

**Tidak ada tabel `build_logs`.** Log adalah stream append-only → filesystem.

## 9. State machine deployment

```
queued → pulling → starting → checking → live
              ↘ failed
              ↘ cancelled
              ↘ unknown
```

`unknown` bukan `failed`. `unknown` artinya kita tidak tahu. Jangan pura-pura tahu.

**Timeout per tahap** (bukan satu timeout global):

| Tahap | Timeout |
|---|---|
| Koneksi SSH | 10 detik |
| Pull image | 10 menit, ATAU 60 detik tanpa progres byte |
| Start container | 30 detik |
| Health check | `apps.health_grace_secs` + threshold, dipisah |
| Drain container lama | 30 detik |

**Lock per app** wajib punya kedaluwarsa:
```sql
UPDATE apps SET lock_token = ?, lock_expires_at = ?
WHERE id = ? AND (lock_expires_at IS NULL OR lock_expires_at < ?)
```

**Saat boot:** cari deployment berstatus berjalan dengan `heartbeat_at` basi →
tandai `unknown` → rekonsiliasi dengan bertanya ke server. Jangan menebak.

## 10. Label container (wajib, ini fondasi rekonsiliasi)

```
--name {app}-{deployment_id}
--network platform                 # TIDAK PERNAH -p. Dua container hidup bersamaan.
--restart unless-stopped
--env-file /var/lib/platform/env/{app}.env
--label platform.app=<app_name>
--label platform.deployment=<deployment_id>
--label platform.digest=sha256:...
--label traefik.enable=true
--label traefik.http.routers.<app>.rule=Host(`...`)
--label traefik.http.services.<app>.loadbalancer.server.port=<apps.port>
--label traefik.http.services.<app>.loadbalancer.healthcheck.path=<apps.health_path>
--label traefik.http.services.<app>.loadbalancer.healthcheck.interval=2s
```

**Urutan pergantian container** (tanpa ini Invariant 1 bocor — dua container
dengan router Traefik yang sama akan menerima traffic bergantian):

1. Start container baru. Traefik melihatnya, tapi belum mengirim traffic karena
   healthcheck-nya belum lulus.
2. Health check milik kita sendiri, menembak IP container langsung.
3. Gagal → tangkap log → hapus container. Traffic tidak pernah menyentuhnya.
4. Lulus → `docker stop --time=30` container lama. Flag `--time` wajib eksplisit;
   default `docker stop` adalah 10 detik dan bentrok dengan tabel drain di §9.
5. `HEAD https://{domain}` sekali sesudah swap. **Ini bukan health check** — ini
   menangkap label Traefik yang salah, kasus di mana deployment `live` tapi publik
   dapat 404. Gagal → tandai warn di deployment, **jangan** rollback (Invariant 3).

---

# FASE

Aturan main: **satu fase selesai dan dipakai sungguhan minimal 1 minggu sebelum
fase berikutnya dimulai.** Tidak ada fase paralel.

---

## Fase 0 — Fleet view (read-only)

**Kenapa duluan:** ini yang paling sering kamu SSH-kan. Berguna di hari pertama
tanpa satu baris kode deployment.

**Prasyarat manual:** selama 2 minggu, catat setiap kali kamu SSH ke server dan
kenapa. Satu baris per kejadian. Daftar itu adalah spesifikasi produk yang sebenarnya
dan lebih jujur daripada dokumen ini. Kalau urutannya ternyata berbeda dari fase di
bawah, **ikuti daftar itu, bukan dokumen ini.**

**Scope:**
- Auth: single user, argon2, session cookie, `SameSite=Lax`, `Secure`, `HttpOnly`.
  CSRF token untuk semua form.
- Tabel `servers`, form "add server" (host, user, private key), tes koneksi SSH.
- Poll tiap 60 detik: `cat /proc/stat /proc/meminfo /proc/loadavg; df -B1 /`
  plus `docker ps --format`.
- Backoff eksponensial saat gagal: 1, 2, 4, 8 menit, berhenti di 15.
  Tandai `unreachable` setelah 3 kegagalan berturut-turut, tampilkan `last_seen_at`.
- Satu halaman: tabel semua server — disk, RAM, load, jumlah container, terakhir terhubung.
  **Jumlah container = semua container, bukan hanya `label=platform.app`.** Disk penuh
  hampir selalu ulah container yang bukan milik platform ini.
- CPU dari `/proc/stat` adalah penghitung kumulatif. Hitung delta terhadap sampel
  sebelumnya, simpan sampel terakhir per server di memori (belum perlu tabel metrik).

**Selesai kalau:** kamu bisa menjawab "server mana yang disknya hampir penuh?"
tanpa membuka terminal.

**Jangan:** grafik, histori metrik, aksi apa pun (masih read-only).

---

## Fase 1 — Deploy loop paling tipis

**Scope:**
- `POST /api/v1/deploy` — Bearer token, body:
  ```json
  {"app":"api","image":"ghcr.io/kamu/api@sha256:...","commit":"<sha>","ref":"main"}
  ```
- Tabel `apps`, `registries`, `deployments`, `jobs`.
- Alur worker: klaim job → `docker pull` → jalankan container baru dengan label →
  health check → hentikan yang lama → tandai `live`.
- Health check **menembak IP container di dalam docker network secara langsung**,
  bukan lewat domain publik atau proxy. Kalau lewat domain, kamu ikut menguji DNS,
  TLS, dan proxy sekaligus, dan saat gagal kamu tidak tahu mana yang rusak.
- `docker login` di server target dikelola sebagai bagian dari alur "add server",
  bukan langkah manual.
  **PAT registry harus terpisah dari PAT git, scope `read:packages` saja.**
  Alasannya: kredensial registry mendarat di server produksi, bukan cuma di control plane.
  Untuk GitLab pakai Deploy Token dengan scope `read_registry`.
- Traefik dijalankan di server target saat pendaftaran; routing lewat label.
- Satu halaman daftar app + riwayat deployment. Boleh jelek.
- Notifikasi webhook (Telegram/Discord): **hanya saat gagal dan saat pulih.**
  Jangan saat berhasil — notifikasi terlalu sering akan diabaikan dalam dua minggu.

**Klasifikasi kegagalan health check** (tiga mode, penyebabnya beda total):

| Gejala | Kemungkinan besar | Tampilkan |
|---|---|---|
| Container exited | env var salah / dependency hilang | exit code + 50 baris log terakhir |
| Jalan, balas 5xx | koneksi DB / migrasi | body respons |
| Jalan, tanpa respons | port salah / bind ke `127.0.0.1` bukan `0.0.0.0` | tebakan itu, tulis eksplisit |

**Selesai kalau:** `git push` → CI build → app di server baru jalan, dengan riwayat tercatat.

**Jangan:** UI bagus, rollback, log streaming, metrik.

**Setelah fase ini: pakai 2 minggu tanpa menambah apa pun.** Catat apa yang bikin kesal.
Daftar itu menentukan urutan fase berikutnya, bukan dokumen ini.

---

## Fase 2 — Log runtime streaming

**Scope:**
- `docker logs --follow --tail 200` lewat SSH → di-pipe ke SSE.
- Satu `tokio::sync::broadcast` channel per deployment aktif, disimpan di
  `DashMap<DeploymentId, Sender>`.
- Writer paralel mem-persist tiap baris ke `logs/{deployment_id}.log` supaya
  reload halaman tetap dapat riwayat.
- Tail untuk SSE = seek biasa ke file, bukan query DB.
- Rotasi = hapus file lebih tua dari 30 hari.
- HTMX `hx-ext="sse"` menyambung langsung ke endpoint SSE.

**Peringatan:** bagian tersulit proyek ini di Rust bukan SSH atau Docker, tapi
**manajemen lifetime pada streaming log**. Proses panjang menulis ke satu channel,
entah berapa browser berlangganan, koneksi bisa putus kapan saja. Rancang
disconnect handling di awal — meretrofitnya menyakitkan.

**Selesai kalau:** kamu bisa lihat kenapa container crash tanpa SSH.

---

## Fase 3 — Env management + rollback

**Scope:**
- `env_vars` (state saat ini, yang diedit) dan `env_versions` (snapshot beku,
  dirujuk tiap deployment). Tanpa pemisahan ini, rollback ke deployment lama
  akan memakai env hari ini.
- **Tombol "Save" di halaman env adalah tombol deploy.** Buat konsekuensinya
  eksplisit di UI: "3 variabel berubah — redeploy sekarang?" Jangan sembunyikan.
- Perubahan env menghasilkan record `deployments` baru dengan `image_digest`
  yang sama persis. Riwayat jadi jujur.
- Enkripsi `value_encrypted` dan `snapshot_encrypted` dengan `age`,
  kunci di `key.age` (0600) di luar database.
- Env sampai ke container lewat `--env-file`, file 0600 di server target,
  dihapus setelah pergantian container selesai.
- Rollback = jalankan ulang tahap "pull dan jalankan" dengan `image_digest`
  dari deployment sebelumnya.
- **Rollback memakai env terbaru secara default**, tapi tampilkan diff kalau berbeda
  dari snapshot deployment itu, dengan opsi "pakai env asli". Alasannya: kalau env
  ikut mundur otomatis, rollback setelah rotasi kredensial akan menjalankan app
  dengan password yang sudah dicabut. Jangan diam-diam memilihkan salah satunya.
- Retensi: simpan 5–10 image terakhir per app, sisanya prune.

**Selesai kalau:** rollback selesai dalam hitungan detik dan kamu tahu persis
env apa yang jalan.

---

## Fase 4 — Operasi lintas server (pembeda utama)

**Kenapa ini penting:** ini yang paling langsung menyerang keluhan asli
("tanpa perlu SSH satu per satu"), dan justru paling jarang dilakukan tool lain
dengan baik. Coolify dan Portainer keduanya berorientasi per-resource.

**Scope:**
- Prune di semua server sekaligus, **menghormati kebijakan retensi rollback**
  (jangan hapus 5 image terakhir per app).
- Satu tabel disk untuk semua server.
- Restart beberapa app di beberapa server dalam satu aksi, dengan konfirmasi
  yang menyebut jumlah target.
- Job rekonsiliasi periodik: bandingkan apa yang SQLite kira berjalan dengan
  apa yang benar-benar berjalan. **Tampilkan banner drift, jangan perbaiki otomatis.**
- **Pintu darurat yang disengaja:** satu kotak "jalankan perintah" per server,
  dan `docker exec` sekali pakai ke dalam container. Dengan ini, sisa 20% kasus
  tertangani tanpa perlu mengantisipasi semuanya — dan kamu bisa berhenti
  menambah fitur dengan tenang.

**Selesai kalau:** minggu penuh tanpa `ssh` dari terminal.

**Jangan:** terminal web persisten, file browser, editor config. Itu jalan menuju
membangun ulang Cockpit dengan lebih buruk.

---

## Fase 5 — Metrik (opsional, hanya kalau masih semangat)

**Peringatan scope:** bagian ini kira-kira sebesar seluruh bagian deployment.
Ini menggandakan proyek. Masuki hanya dengan sadar.

**Scope:**
- Baru di sini `bollard` masuk, lewat local port forward `openssh` ke Docker socket
  di server target. Endpoint `/containers/{id}/stats?stream=false&one-shot=true`
  (bukan `docker stats`, yang sengaja menunggu dua sampel >1 detik per panggilan).
- **Satu transaksi per siklus poll.** 3 server × 10 container = ~33 baris → 1 transaksi,
  bukan 33 fsync.
- **Downsampling sejak hari pertama** (menambahkannya saat tabel sudah 2 GB =
  migrasi yang menyakitkan):

  | Tingkat | Interval | Retensi |
  |---|---|---|
  | Mentah | 15 detik | 6 jam |
  | Menit | 1 menit | 7 hari |
  | Jam | 1 jam | 1 tahun |

- Rollup jalan tiap menit. **Simpan `max` bukan hanya `avg`** — lonjakan CPU 3 detik
  yang membunuh app akan hilang total kalau cuma menyimpan rata-rata.
- Dua koreksi wajib, tanpa ini angkamu salah:
  - **Memori cgroup termasuk page cache.** Kurangi
    `stats.memory_stats.stats.inactive_file` dari usage. Tanpa ini app yang membaca
    banyak file terlihat pakai 900 MB padahal 200 MB.
  - **CPU dikali jumlah core.** `Δtotal_usage / Δsystem_cpu_usage × ncpu × 100`.
    Tanpa pengali core, app yang menghabiskan 2 core di mesin 4-core terlihat 50%.

**Yang layak dibangun (dan cukup — jangan lebih):**
1. **Alert disk 80%** + tombol prune. Ini pembunuh nomor satu VPS kecil.
2. **Deteksi container restart berulang.** Angka restart yang naik lebih berguna
   daripada grafik CPU mana pun.
3. **Perbandingan sebelum/sesudah deploy.** Setelah deployment stabil 10 menit,
   bandingkan rata-rata memori dan CPU dengan 10 menit sebelumnya. Naik >30% →
   tandai di riwayat deployment.

**Nilai jual satu-satunya di sini adalah korelasi yang Grafana tidak punya:**
garis vertikal "deploy #47" di grafik memori. Kalau butuh query bebas, pasang
Grafana — kita tidak akan menang di situ.

---

## Fase 6 — Self-hosting

Control plane jadi salah satu app di daftarnya sendiri, dengan satu pengecualian:
dia tidak boleh mematikan dirinya di tengah proses (pakai proses pengganti singkat
atau systemd `ExecStartPre` untuk swap binary).

Backup sebagai job internal, bukan cron eksternal:
```sql
VACUUM INTO '/var/lib/platform/backups/db-YYYY-MM-DD.sqlite'
```
Simpan 7 hari terakhir. Rsync ke salah satu server target (koneksi SSH sudah ada,
tidak ada dependensi baru).

**`key.age` dicadangkan manual sekali, terpisah, dan tidak pernah ke direktori backup.**
Isinya tidak pernah berubah.

---

## 11. Cara kerja dengan asisten (aturan untuk Claude)

- Kerjakan **satu fase saja** per sesi. Kalau diminta melompat fase, ingatkan dulu.
- Sebelum menulis kode, sebutkan file apa saja yang akan disentuh. Kalau lebih dari
  3 file untuk satu fitur, berhenti dan diskusikan dulu.
- Tidak ada abstraksi yang tidak diminta: tidak ada trait dengan satu implementor,
  tidak ada builder untuk struct 3 field, tidak ada config untuk nilai yang tidak
  pernah berubah.
- Setiap logika non-trivial (state machine, rekonsiliasi, perhitungan metrik,
  parsing `/proc/stat`) meninggalkan **satu test yang bisa dijalankan**.
- Query pakai `sqlx::query!` (compile-time checked). Kalau tidak bisa, jelaskan kenapa.
- Kalau sebuah permintaan melanggar bagian **Invariants** atau **Non-Goals**,
  tolak dan sebutkan nomor aturannya.
- Penyederhanaan yang disengaja ditandai komentar `// ponytail: <batasnya>, upgrade saat <kondisi>`.

## 12. Risiko yang sudah diketahui

- **Berhenti di 70%.** Deploy jalan, UI setengah jadi, error handling belum lengkap,
  lalu diam-diam kembali ke `compose pull && up` saat buru-buru. Mitigasi: fase
  sekecil mungkin, dipakai sungguhan sebelum lanjut.
- **Estimasi waktu.** Kalau belum pernah menggabung Axum + sqlx + SSH async + SSE
  sekaligus, Fase 1 bukan satu akhir pekan. Lebihkan 3–5×.
- **Scope creep lewat "sekalian saja".** Filternya cuma satu: berapa sesi SSH yang hilang.
- **`openssh` tidak bisa membedakan dengan andal antara kegagalan koneksi SSH dan
  error dari program di remote.** Pesan errornya lebih buruk daripada implementasi
  native. Pisahkan exit code dan stderr sendiri, sejak Fase 1.

## 13. Riset satu jam sebelum mulai

Lihat **Cockpit** (sisi host) dan **Portainer** (sisi container). Keduanya lemah persis
di tempat proyek ini kuat: deployment dan riwayatnya. Catat di mana mereka bikin kesal —
itu mempertajam daftar fitur lebih cepat daripada merancang dari nol.
