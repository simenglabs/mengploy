# Spesifikasi Desain: Aplikasi (Apps)

Spesifikasi antarmuka untuk `mengdep` Fase 2 — daftar app, form tambah app, dan
overview detail app (konfigurasi, domain, token deploy, riwayat deployment).
Ditulis sebagai dokumentasi as-built (`src/web/apps.rs`), bukan spec di muka —
Fase 2 disatukan dengan implementasi karena `routes/**` dan `web/**` saling
bergantung untuk kompilasi (beda dari Fase 1 yang bisa memisah sub-blok
uiux/backend/frontend berurutan).

## 1. Tujuan

Tempat operator mendaftarkan app (nama, server tujuan, port, health check),
mengelola domain dan token deploy per app, dan melihat riwayat deployment —
tanpa ini `POST /api/v1/deploy` tidak punya apa pun untuk dituju.

## 2. Token Visual

Sama `src/web/styles.rs` seperti Fase 1 — tidak ada token baru. Reuse
langsung: `.fleet-table`, `.fleet-header`, `.fleet-empty`, `.detail-grid`,
`.detail-card`, `.detail-row`, `.alert`, `.form-panel`, `.field` (semua sudah
ada dari halaman server/registry Fase 1).

## 3. Layout — `GET /apps`

```text
+-----------------------------------------------------------------+
| MENGDEP [Fase 2] | Dashboard Server Apps            [Keluar]    |
+-----------------------------------------------------------------+
|  Aplikasi                                      [+ Tambah App]   |
|  -------------------------------------------------------------  |
|  Nama     | Server      | Port | Health Path                    |
|  api      | vps-sg-1    | 8080 | /health                        |
|  worker   | vps-sg-1    | 9000 | /health                        |
+-----------------------------------------------------------------+
```

**Kosong**: pesan + CTA "+ Tambah App", sama pola `fleet-empty` Fase 1.

## 4. Layout — `GET /apps/baru`

Form satu langkah (bukan wizard — jauh lebih sedikit field dari tambah
server): pilih server (select, WAJIB ada server terdaftar dulu — kalau
kosong tampil alert mengarahkan ke `/servers/baru`), nama app, port
container, health check path (default `/health`), grace period detik
(default `30`). **Tidak ada field restart policy** — invariant §5 no.5
mengunci `unless-stopped` untuk semua container, jadi tidak ada pilihan di
UI sama sekali (`routes/apps.rs::app_baru_submit` hardcode nilainya).

## 5. Layout — `GET /apps/{id}` (Overview)

```text
+-----------------------------------------------------------------+
|  App: api                                                       |
|                                                                  |
|  [Token deploy baru — salin sekarang, tidak akan ditampilkan     |
|   lagi: mengdep_deploy_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]   |
|  (banner HANYA muncul tepat setelah "+ Buat Token" — sekali)     |
|                                                                  |
|  +----------------------+  +----------------------+              |
|  | Konfigurasi          |  | Domain               |              |
|  | Server: vps-sg-1     |  | api.contoh.com        |              |
|  | Port: 8080           |  | [+ Tambah Domain]     |              |
|  | Health Path: /health |  +----------------------+              |
|  | Grace: 30s           |  +----------------------+              |
|  | Restart: unless-stop |  | Token Deploy          |              |
|  +----------------------+  | github-actions —      |              |
|                             |  dibuat ... (terakhir  |              |
|                             |  dipakai ...)          |              |
|                             | [+ Buat Token]         |              |
|                             +----------------------+              |
|                                                                  |
|  +--------------------------------------------------------+     |
|  | Riwayat Deployment                                      |     |
|  | Waktu            | Commit  | Status                     |     |
|  | 2026-08-10 10:00 | abcdef1 | [LIVE]                      |     |
|  | 2026-08-10 09:40 | 1234567 | [GAGAL]                     |     |
|  +--------------------------------------------------------+     |
+-----------------------------------------------------------------+
```

Baris riwayat link ke `/deployments/{id}` (detail deployment).

## 6. State & Komponen

### 6.1 Banner token baru (`token_baru: Option<&str>`)
Invariant §5 no.11 — secret tidak pernah dikembalikan API setelah disimpan.
`POST /apps/{id}/token` me-render ULANG halaman detail LANGSUNG (bukan
redirect) dengan plaintext token disisipkan di response INI SAJA. Reload
halaman berikutnya (`GET /apps/{id}`) tidak akan pernah menampilkannya lagi
— `DeployTokenRingkas` yang dibaca dari db tidak punya field plaintext atau
hash sama sekali.

### 6.2 Domain kosong
"Belum ada domain — Traefik hanya routing lewat label, tanpa domain publik
tidak ada router." — bukan pesan generik "tidak ada data", supaya operator
paham KONSEKUENSI, bukan cuma fakta kosong.

### 6.3 Token kosong
"Belum ada token — CI tidak bisa deploy app ini sampai token dibuat." — sama
alasan: konsekuensi, bukan cuma fakta.

### 6.4 Riwayat deployment kosong
"Belum pernah dideploy." Badge status pakai `web::deployments::badge_deployment`
(non-warna-saja, label kapital + `aria-label`, pola sama badge server Fase 1).

## 7. Aksesibilitas

Sama pola Fase 1: badge status punya `aria-label`, section pakai
`aria-labelledby` menunjuk `h2` di dalamnya, tidak ada informasi yang HANYA
disampaikan lewat warna (badge selalu punya label teks kapital).
