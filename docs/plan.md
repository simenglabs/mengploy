# Rencana implementasi

**Fase:** Fase 4 — Pengelolaan environment (`docs/prd.md:296-313`)

**Keadaan repo:** Fase 3 (Log dan riwayat) selesai — kedelapan peran di
`docs/progress.md` checklist Fase 3 tercentang, reviewer menutup dengan 0
BLOCKING (1 WARNING boleh ditunda, 3 NIT). Belum ada konfirmasi eksplisit
manusia "Fase 3 LOLOS gerbang" (beda dari Fase 1/2 yang punya paragraf
kesimpulan eksplisit) — dicatat, bukan penghalang, karena kriteria gerbang
`docs/prd.md` §6 secara substansi sudah terpenuhi semua.

## Masalah

Env var app saat ini cuma bisa diubah lewat SSH manual (`docker exec` atau
edit file di server). PRD Fase 4: env bisa diedit dari UI, perubahan
menghasilkan deployment baru dengan **digest yang sama** (bukan rebuild),
secret tidak pernah terbaca lagi setelah disimpan.

## Kondisi sekarang

- `deployments.env_version_id` **sudah ada** di skema sejak Fase 2
  (`migrations/0003_deploy.sql:90-91`), sengaja NULL selamanya sampai fase
  ini — kolom murah-sekarang-mahal-diretrofit, bukan pelanggaran "jangan
  desain skema fase depan" (`docs/prd.md` §8 sudah menandainya begitu).
  Belum ada tabel `env_vars`/`env_versions` sama sekali.
- `deployments::repo::insert_queued_dengan_job` (`src/deployments/repo.rs:42-89`)
  meng-INSERT `trigger_source` literal `'api'` dan tidak pernah mengisi
  `env_version_id` (selalu NULL lewat default kolom). `NewDeployment`
  (`src/deployments/repo.rs:28-33`) tidak punya field env_version_id.
- `deployments::engine::jalankan_docker` (`src/deployments/engine.rs:326-465`)
  membuat container lewat `docker::create_container`/`NewContainer`
  (`src/docker/client.rs:191-229`) — `NewContainer` **tidak punya field env
  sama sekali**. Tidak ada jalur menulis apa pun ke server target selain
  socket docker forward dan (Fase 1) `docker login`.
- `crypto::CryptoKey` (`src/crypto.rs:17-58`) sudah `encrypt`/`decrypt`
  generik pakai `age` — dipakai ulang persis, tanpa perubahan.
- `ssh::exec_with_stdin` (`src/ssh/exec.rs:82-129`) sudah dipakai
  `docker::registry_login::login` (`src/docker/registry_login.rs:40-69`)
  untuk mengirim password lewat stdin, **bukan argumen baris perintah** —
  pola identik dipakai untuk menulis isi file env ke server target tanpa
  menyentuh disk lokal sama sekali dan tanpa secret pernah muncul di `ps`.
- Halaman detail app punya 3 tab (Overview/Deployments/Logs) lewat
  `tab_nav()` (`src/web/logs.rs:381-406`) + handler
  `tab_deployments`/`tab_logs` (`src/routes/apps.rs:162-208`). Tab
  Environment mengikuti bentuk identik — tab ke-4.
- `apps_repo::insert_deploy_token`/`DeployTokenRingkas`
  (`src/apps/model.rs:30-35`, `src/apps/repo.rs:143-167`) sudah membangun
  pola "secret ditampilkan sekali saat dibuat, response berikutnya cuma
  metadata" — dipakai ulang bentuknya untuk field secret env (masker
  `••••••••` + tombol Replace, PRD Fase 4 baris Frontend).
- `routes/deploy_api.rs` (`POST /api/v1/deploy`, dari Fase 2) membuat
  deployment baru **tanpa** menyentuh env sama sekali — perlu diperbarui
  supaya env_version_id ikut terisi (lihat "Desain teknis" di bawah),
  bukan cuma jalur "simpan env" yang menyentuhnya.
- Layout on-disk server target sudah dikunci `CLAUDE.md` §6:
  `/var/lib/platform/env/{app}.env` (0600, dihapus setelah pergantian
  container selesai) — **tidak ada file staging lokal**, konten env langsung
  dari memori lewat SSH stdin ke server target.

## Dependensi

Tidak ada dependensi baru. `age`, `sqlx`, `ssh`/`openssh` semua sudah ada
sejak fase sebelumnya.

## Desain teknis yang mengikat

**Alur simpan env (`POST /apps/{id}/env`):**
1. Validasi CSRF + form (key tidak boleh duplikat dalam satu submit, key
   tidak boleh kosong).
2. Enkripsi tiap value baru/berubah dengan `age` (`state.crypto.encrypt`),
   upsert ke `env_vars` (state "sedang diedit").
3. Bangun snapshot JSON `{key: value_plaintext}` dari SELURUH `env_vars` app
   itu (bukan cuma yang berubah), enkripsi SATU KALI jadi
   `env_versions.snapshot_encrypted`, `version` = `MAX(version)+1` per app.
4. Ambil `image_digest` dari deployment `live` app ini SAAT INI
   (`deployments_repo::find_current_live`, sudah ada) — env-triggered
   redeploy TIDAK PERNAH mengganti image, cuma env.
5. INSERT `env_versions` + INSERT `deployments` (status `queued`,
   `trigger_source='env'`, `env_version_id` = versi baru) + INSERT `jobs`
   — **SATU transaksi** (invariant §3 no.10), perluasan
   `insert_queued_dengan_job` yang sudah ada, bukan fungsi baru terpisah.
6. Kalau app sedang terkunci deploy lain (`apps.lock_token` aktif) — tolak
   409, PESAN eksplisit "deploy lain sedang berjalan", env_vars TETAP
   tersimpan (state "diedit" berhasil, hanya redeploy yang ditunda) — user
   bisa coba simpan-dan-deploy lagi.

**Alur normal `POST /api/v1/deploy` (CI, sudah ada sejak Fase 2) berubah
sedikit:** `env_version_id` deployment baru diisi versi env AKTIF TERBARU
app itu (`SELECT MAX(version) FROM env_versions WHERE app_id=?`, NULL kalau
app belum pernah punya env sama sekali) — supaya `engine.rs` PUNYA cara
seragam tahu env mana yang harus ditulis ke target, terlepas dari trigger
apa pun. Ini konsekuensi langsung PRD Fase 4 baris Backend: "Deploy yang
dipicu perubahan env memakai digest yang sedang berjalan" — sebaliknya juga
harus benar: deploy yang dipicu digest baru (CI) memakai env yang sedang
berjalan.

**Di `deployments::engine::jalankan_docker`, SEBELUM `create_container`:**
- Kalau `dep.env_version_id` adalah `Some`, ambil `env_versions.snapshot_encrypted`,
  dekripsi, format `KEY=VALUE\n` per baris (escape tidak perlu — `age`
  membawa byte apa adanya, newline di dalam VALUE jadi tanggung jawab
  validasi form: value dengan newline literal ditolak saat simpan, lihat
  "Kriteria selesai").
- Kirim isi file lewat `ssh::exec_with_stdin` ke server target:
  `install -D -m 0600 /dev/stdin /var/lib/platform/env/{app}.env` (satu
  perintah, sekaligus `mkdir -p` parent dan set mode, tidak butuh langkah
  `mkdir` terpisah) — pola identik `registry_login.rs` tapi tujuannya file,
  bukan `docker login`.
- SETELAH swap sukses (container baru live, container lama sudah di-drain)
  ATAU deploy gagal dan container baru sudah dihapus: hapus file env lama
  dari langkah SEBELUMNYA (deployment env_version yang beda dari yang baru
  saja dipakai) — bukan file yang baru saja ditulis. Kalau app tidak
  pernah ganti env, tidak ada apa pun untuk dihapus.

**Pertanyaan terbuka Q1 (lihat di bawah) menentukan APAKAH env sampai ke
proses container lewat `bollard::ContainerCreateBody.env` (field JSON API,
satu-satunya jalur yang benar-benar membuat proses container melihatnya
sebagai variabel environment) atau lewat mekanisme lain. Bagian di atas
mengasumsikan opsi A (lihat Q1) supaya rencana tetap bisa ditulis — kalau
jawabannya beda, langkah "kirim isi file" tetap berlaku (dibutuhkan untuk
audit/debug operator), tapi `NewContainer` di `docker/client.rs` juga perlu
field `env: &[(String, String)]` diteruskan ke `ContainerCreateBody.env`.**

## Perubahan per file

| File | Perubahan | Pemilik |
|---|---|---|
| `migrations/0005_env.sql` | `env_vars(id, app_id, key, value_encrypted, is_secret, updated_at)` UNIQUE(app_id,key); `env_versions(id, app_id, version, snapshot_encrypted, note, created_at)` UNIQUE(app_id,version). Tidak ada perubahan `deployments` — `env_version_id` sudah ada sejak 0003. | migration |
| `src/apps/model.rs` | `EnvVarRingkas` (id, key, is_secret, updated_at — **tanpa** value_encrypted, pola sama `DeployTokenRingkas`), `EnvVersionRingkas` (id, version, note, created_at). | backend |
| `src/apps/repo.rs` | `upsert_env_var`, `list_env_vars_ringkas`, `delete_env_var`, `insert_env_version` (snapshot), `find_latest_env_version`, `find_env_version_by_id`. | backend |
| `src/deployments/model.rs`, `src/deployments/repo.rs` | `NewDeployment` +`env_version_id: Option<&str>` +`trigger_source: &str` (bukan literal `'api'` lagi — dua pemanggil beda nilai: `deploy_api.rs`='api', env save='env'); `insert_queued_dengan_job` ikut parameter baru. | backend |
| `src/deployments/engine.rs` | Tulis env file ke target sebelum `create_container` (lihat "Desain teknis"), hapus file env versi sebelumnya setelah swap kelar (sukses maupun gagal). | backend |
| `src/docker/client.rs` | **Tergantung jawaban Q1.** Opsi A: tidak berubah (env tidak pernah masuk `ContainerCreateBody`, aplikasi baca dari file ter-mount — TAPI ini butuh bind mount + app baca file sendiri, lihat Q1 opsi ditolak). Opsi B (direkomendasikan): `NewContainer` +`env: &[(String,String)]`, diteruskan ke `ContainerCreateBody.env`. | backend |
| `src/routes/deploy_api.rs` | Isi `env_version_id` dari `find_latest_env_version` app sebelum `insert_queued_dengan_job`. | backend |
| `src/routes/apps.rs` | Handler baru: `tab_environment` (GET, render tabel+form), `env_submit` (POST, alur "Desain teknis"). | backend |
| `src/routes/mod.rs` | Wiring dua route baru, di blok `protected`. | backend |
| `src/web/logs.rs` (fungsi `tab_nav`) | Tambah entri ke-4 `("environment", "Environment", ...)`. | frontend |
| `src/web/env.rs` (baru) | `render_app_tab_environment`: tabel env (key, value bertopeng kalau `is_secret`, tombol Replace), baris tambah inline, bar sticky "N variabel berubah" + tombol simpan-dan-deploy, tampilan diff (key ditambah/diubah/dihapus dibanding versi env aktif). | frontend |
| `docs/design/environment.md` (baru) | Spesifikasi tampilan diff (termasuk cara menampilkan perubahan value secret TANPA membocorkan nilainya — pola: tampilkan "(diubah)" bukan nilai lama/baru), bar perubahan yang mustahil dilewatkan. | uiux |
| `docs/api-contract.md` | Append bagian Fase 4: `GET/POST /apps/{id}/env`. Sebutkan eksplisit field yang TIDAK PERNAH dikembalikan (value_encrypted, plaintext value). | planner |
| `tests/phase4.rs` (baru) | ≥5 skenario injeksi kegagalan (lihat "Kriteria selesai"). | qa |

Sembilan baris kode (di luar migration/design/contract/test) — dalam
ambang "5-6 task idealnya" `planner.md` kalau dihitung per-peran backend
(5 sub-perubahan backend, tapi saling terikat erat, tidak berdiri sendiri
seperti sub-blok Fase 2/3 — tidak diusulkan dipecah subtask terpisah).

## Urutan eksekusi

1. **migration** (0005) — berdiri sendiri.
2. **uiux** (`docs/design/environment.md`) ‖ **backend langkah A**
   (`apps/model.rs`+`apps/repo.rs` CRUD env_vars/env_versions) — paralel,
   tidak saling bergantung.
3. **backend langkah B** (`deployments/repo.rs`+`model.rs` env_version_id
   +trigger_source parametrik) — butuh migration (1) selesai untuk kolom,
   TIDAK butuh langkah A.
4. **backend langkah C** (`deployments/engine.rs` tulis/hapus file env,
   `docker/client.rs` kalau Q1=opsi B, `routes/deploy_api.rs`) — butuh
   langkah A+B selesai (butuh `find_latest_env_version`, `NewDeployment`
   baru).
5. **backend langkah D** (`routes/apps.rs` tab+submit, `routes/mod.rs`
   wiring) — butuh A+B+C.
6. **frontend** (`web/env.rs`, `web/logs.rs::tab_nav`) — butuh uiux (2) dan
   bentuk data langkah A, bisa mulai SETELAH langkah A selesai (tidak perlu
   menunggu C/D, cuma render dari tipe yang sudah ada + endpoint kontrak
   dari planner).
7. **qa** (`tests/phase4.rs`) — butuh D+frontend selesai.
8. **security** — fase kritis kedua (PRD eksplisit), cakupan penuh: enkripsi
   at rest, kunci di luar db, secret tidak pernah di respons API, tidak
   tercatat log, tidak muncul `docker inspect` (atau alasan tertulis kenapa
   itu di luar kendali platform — lihat Q1), file env terhapus setelah
   dipakai, snapshot lama tetap terenkripsi.
9. **reviewer** — invariant §3 no.6, 7, 8. Cari kebocoran secret di setiap
   jalur log dan respons.

## Risiko

- **Invariant §3 no.6** ("env var lewat `--env-file` 0600, tidak pernah
  `-e`") ditulis untuk model lama shell-out `docker run` — arsitektur
  sekarang (sejak Fase 2) memakai `bollard` API langsung, yang TIDAK
  mengenal konsep "--env-file" (itu murni konversi sisi CLI `docker`,
  bukan primitif daemon). Ini bukan risiko implementasi biasa, ini
  **pertanyaan arsitektur** — lihat Q1.
- **Invariant §3 no.7** (secret tidak pernah dikembalikan) — `EnvVarRingkas`
  sengaja tanpa field value, pola sama `DeployTokenRingkas`
  (`src/apps/model.rs:30-35`) yang sudah lolos gerbang security Fase 2.
- **Invariant §3 no.9** (baris log tidak pernah ke SQLite) — isi env TIDAK
  PERNAH masuk `deployments_repo::set_status`/log baris `catat()` di
  `engine.rs`; hanya "menulis file env" (tanpa isi) yang boleh dicatat.
- **Concurrent save vs deploy aktif** (skenario qa PRD: "simpan env saat
  deploy sedang berjalan") — ditangani lewat `apps.lock_token` yang sudah
  ada (invariant §3 no.12); env_vars TETAP tersimpan, redeploy ditolak 409
  kalau lock aktif (lihat "Desain teknis" langkah 6).
- **Race file env di target**: kalau dua redeploy app yang sama berhasil
  lolos lock berurutan cepat (lock dilepas lalu diambil lagi), penghapusan
  "file env versi sebelumnya" harus membandingkan `env_version_id`
  eksplisit (bukan "hapus lalu tulis"), supaya deploy B yang sedang
  berjalan tidak kehilangan file env-nya karena deploy A membersihkan.
  Engine.rs SELALU menulis file BARU dulu (menimpa target yang sama
  `{app}.env` kalau env_version_id sama, atau path sama tertimpa kalau
  beda — satu app = satu nama file, jadi "tulis" selalu idempoten aman,
  "hapus" HANYA dipanggil kalau ternyata tidak ada deployment aktif lain
  yang masih memakai path itu — pola sama `drain_container_lama`
  memeriksa `find_current_live` sebelum bertindak).

## Kriteria selesai

- Env bisa ditambah/diedit/dihapus dari UI tab Environment.
- Simpan menghasilkan deployment baru dengan `image_digest` IDENTIK
  deployment live sebelumnya, `env_version_id` baru, `trigger_source='env'`.
- `GET` mana pun (tab Environment, detail app) TIDAK PERNAH mengembalikan
  plaintext value — field secret bertopeng `••••••••` + tombol Replace,
  field non-secret boleh ditampilkan (bukan credential).
- Key duplikat dalam satu submit form ditolak sebelum menyentuh db.
- Value dengan newline literal ditolak dengan pesan jelas (format
  `KEY=VALUE` per baris tidak bisa merepresentasikan newline di tengah
  value tanpa ambiguitas) — batasan didokumentasikan di UI, bukan
  di-escape diam-diam.
- Value sangat panjang (uji dengan >8000 karakter, mis. sertifikat PEM)
  tersimpan dan ter-roundtrip enkripsi/dekripsi utuh — tidak ada
  pemotongan diam-diam (beda dari `error_detail` yang memang sengaja
  dipotong 500 karakter, env value BUKAN pesan error).
- Deploy gagal setelah env diubah → `deployments.env_version_id` deployment
  yang gagal itu tetap tercatat persis versi mana yang dipakai (bukan versi
  yang "sedang diedit" saat itu, kalau ada save lain menyusul).
- File `/var/lib/platform/env/{app}.env` di server target bermode 0600,
  dan versi env LAMA (bukan yang sedang dipakai deployment aktif) terhapus
  setelah pergantian container selesai.
- `unwrap()`/`expect()` nol di luar `#[cfg(test)]` di seluruh perubahan.
- `cargo fmt`/`clippy -D warnings`/`cargo test --all-targets` hijau, nol
  regresi phase0-3.

## Yang sengaja tidak dikerjakan

- **Dialog/tombol rollback** — itu Fase 5 (`docs/prd.md:316-332`). Fase ini
  cuma memastikan `env_version_id` terisi benar di setiap deployment supaya
  Fase 5 punya data yang bisa dipakai, tanpa retrofit skema.
- **Diff terhadap versi historis sembarang** — PRD cuma minta diff "apa yang
  berubah dibanding versi AKTIF saat ini" (form belum-disimpan vs env yang
  sedang berjalan), bukan diff antar dua `env_versions` masa lalu manapun.
- **Import/export env massal (.env file upload)** — tidak diminta PRD,
  form tambah tetap satu-per-satu baris inline.

## Pertanyaan terbuka

**Q1 — cara env sampai ke proses container, vs invariant §3 no.6 dan baris
security Fase 4 "tidak muncul di `docker inspect`". Ini pertanyaan
arsitektur nyata, bukan detail implementasi — perlu dijawab manusia
sebelum langkah backend C/D (lihat "Urutan eksekusi") dikerjakan.**

Docker Engine API (dipakai lewat `bollard` sejak Fase 2, BUKAN shell-out
CLI) tidak punya primitif "baca env dari file" di sisi daemon. Satu-satunya
cara proses di dalam container benar-benar melihat variabel environment
adalah field `Env` di body `POST /containers/create` — dan `docker inspect`
SELALU menampilkan isi field itu apa adanya, terlepas dari bagaimana
klien (CLI `--env-file` ATAU `-e`) mengisinya. Bahkan kalau proyek ini
kembali shell-out `docker run --env-file ...` lewat SSH (membalik
keputusan Fase 2), hasil akhirnya di `docker inspect` tetap sama persis —
`--env-file` cuma kenyamanan sisi klien, bukan mekanisme yang
menyembunyikan nilai dari daemon.

- **Opsi A (direkomendasikan):** Terima bahwa env akan terlihat lewat
  `docker inspect` di server TARGET. Kirim lewat `bollard`
  `ContainerCreateBody.env` (field API, bukan argumen shell — invariant §3
  no.6 versi hurufnya "tidak pernah lewat `-e`" tetap terpenuhi literal,
  karena tidak ada baris perintah `-e` sama sekali). Boundary keamanan
  nyata: siapa pun yang punya akses `docker inspect` di server target
  sudah punya akses SSH+docker socket ke server ITU — batas kepercayaan
  yang sama dengan siapa pun yang bisa membaca `key.age` atau
  `servers.ssh_key_encrypted` kalau db+kunci bocor bersamaan. Ini boundary
  yang SUDAH ada sejak Fase 0 (siapa pun dengan akses server = akses penuh
  ke apa pun yang jalan di situ), bukan permukaan baru. Security review
  (langkah 8) tetap wajib menulis eksplisit "secret muncul di `docker
  inspect` server target, ini batas fisik Docker Engine API, bukan bug"
  di laporan — supaya tidak diam-diam dianggap terpenuhi.
- **Opsi B (ditolak, dicatat kenapa):** File env ter-mount ke container,
  app baca sendiri (dotenv-style) tanpa lewat `Env` API sama sekali. Butuh
  kerja sama image aplikasi (baca file, bukan `std::env::var`/`os.environ`)
  — mustahil digeneralisasi untuk image sembarang dari CI mana pun
  (`docs/prd.md` non-goal: platform tidak pernah tahu isi/struktur image).
- **Opsi C (ditolak, dicatat kenapa):** Kembali shell-out `docker run
  --env-file` lewat SSH untuk langkah create container saja, mundur dari
  keputusan sadar Fase 2 pindah ke `bollard`. Tidak menyelesaikan apa pun
  — hasil `docker inspect` identik Opsi A (lihat penjelasan di atas), cuma
  menambah kompleksitas (dua jalur berbeda: bollard untuk lifecycle,
  SSH-CLI untuk create) tanpa manfaat keamanan nyata.

Rencana di atas ("Desain teknis") ditulis mengasumsikan **Opsi A** dipilih
supaya rencana tetap konkret — kalau manusia memilih lain, langkah
`docker/client.rs` di tabel "Perubahan per file" berubah, sisanya (tulis
file env ke target untuk keperluan audit/`docker exec` manual operator,
hapus setelah swap) tetap relevan tanpa perubahan.

**Q2 (kecil, tidak memblokir) — nama file env di target selalu
`{app}.env` (satu file per app, ditimpa tiap redeploy) atau
`{app}-{env_version}.env` (satu file per versi, cocok pola log
`{deployment_id}.log`)?** Rencana di atas mengasumsikan **satu file per
app, ditimpa** — konsisten `CLAUDE.md` §6 (`/var/lib/platform/env/{app}.env`
tanpa versi di nama) dan lebih sederhana (nol sampah file lama menumpuk di
target kalau proses hapus gagal transien). Kalau ini salah asumsi, ganti
jadi per-versi murni masalah penamaan, tidak mengubah desain lain.

---

## Fase 5 — Keandalan dan rollback

Fase ini memakai reconciliation berbasis label Docker secara periodik dan saat
boot. Reconciliation hanya membaca kondisi server, membuka atau menyelesaikan
finding idempoten, dan tidak pernah memperbaiki container secara otomatis.

Rollback selalu membuat deployment baru dengan `trigger_source='rollback'`.
Digest diambil dari deployment target di database. Default environment adalah
snapshot deployment target; UI juga dapat memilih env terbaru atau versi historis
milik app yang sama. Lima digest deployment terakhir, deployment live/aktif/
unknown, dan image yang sedang dipakai container selalu dilindungi dari retensi.
Server tidak terjangkau berarti cleanup dilewati.

Urutan implementasi:

1. migrasi 0006, kontrak HTTP, dan spesifikasi UI/UX;
2. lock/heartbeat guarded dan klaim job atomik;
3. observasi Docker terstruktur dan reconciliation findings;
4. rollback melalui engine deployment yang sama;
5. retensi image;
6. webhook terenkripsi dengan delivery queue, retry, HMAC, dan payload tanpa
secret;
7. UI, QA fault injection, security, reviewer, dan gerbang fase.

Kriteria wajib: crash control plane di setiap tahap menjadi `unknown` tanpa
tebakan; SSH putus setelah command diverifikasi lewat pembacaan ulang server;
dua rollback bersamaan hanya menerima satu lock; image hilang gagal aman;
drift manual menjadi finding tanpa auto-heal; webhook failure tidak menggagalkan
deploy; migrasi bersih; dan `cargo fmt`, `cargo clippy --all-targets -- -D
warnings`, `cargo test --all-targets` hijau.
