# Spesifikasi Desain: Detail Deployment

Spesifikasi antarmuka untuk `mengdep` Fase 2 — timeline satu deployment,
`GET /deployments/{id}` + `GET /events/deploy/{id}` (SSE). Dokumentasi
as-built (`src/web/deployments.rs`), sama alasan penggabungan sub-blok
seperti `docs/design/apps.md`.

## 1. Tujuan

Jawab pertanyaan "kenapa deploy ini gagal / masih di tahap apa?" tanpa SSH —
Jangkar CLAUDE.md persis. Timeline mendorong status TERBARU lewat SSE, jadi
operator tidak perlu me-refresh halaman manual saat menunggu.

## 2. Layout

```text
+-----------------------------------------------------------------+
|  Deployment dep-1234        [MENARIK IMAGE]                     |
|                                                                  |
|  +---------------------------+                                  |
|  | Info                      |                                  |
|  | App: api                  |                                  |
|  | Status: [MENARIK IMAGE]   |                                  |
|  | Commit: abcdef1            |                                  |
|  | Ref: main                  |                                  |
|  | Image Digest: ghcr.io/...  |                                  |
|  | Dibuat: 2026-08-10 10:00  |                                  |
|  +---------------------------+                                  |
+-----------------------------------------------------------------+
```

Kalau gagal, kartu KEDUA muncul (`error_kind` sebagai judul, `error_detail`
sebagai isi — pesan Bahasa Indonesia final dari `DeployKegagalan::pesan()`,
sudah termasuk kemungkinan penyebab dan langkah perbaikan):

```text
  +--------------------------------------------------------+
  | Kegagalan: health_no_response                          |
  | Container berjalan tapi tidak merespons health check   |
  | sama sekali. Kemungkinan besar: aplikasi bind ke        |
  | 127.0.0.1, seharusnya ke 0.0.0.0, atau port salah.      |
  +--------------------------------------------------------+
```

## 3. Timeline SSE

Seluruh `.detail-grid` (kartu Info + kartu Kegagalan kalau ada) dibungkus
`<div id="deployment-timeline" hx-ext="sse" sse-connect="/events/deploy/{id}"
sse-swap="message">` — pola PERSIS `render_verifikasi`/`render_verifikasi_fragmen`
Fase 1, hanya beda payload (`DeploymentEvent` vs `VerificationEvent`).

**Snapshot awal benar walau SSE gagal tersambung**: render pertama HTML
halaman sudah memuat fragmen dari status db saat ini (`fragmen_isi`
dipanggil langsung di `render_deployment_detail`, BUKAN menunggu event SSE
pertama). SSE murni dorongan pembaruan, bukan satu-satunya sumber data.

**Job selesai, stream ditutup**: `routes::events::deploy_stream` — kalau
`dep.status.selesai()` sudah `true` SEBELUM klien menyambung, kirim SATU
snapshot lalu tutup (tidak membuka koneksi menggantung tanpa event yang
akan pernah datang). Kalau belum selesai, forward tiap `DeploymentEvent`
dari `AppState.deployment_events`, BACA ULANG baris `deployments` tiap
event (bukan hanya meneruskan payload broadcast) supaya fragmen SELALU
punya `error_kind`/`error_detail` terbaru — payload broadcast sendiri hanya
bawa `status` + `pesan` ringkas, bukan baris penuh.

## 4. State badge (`badge_deployment`)

| Status internal | Label | Kelas warna |
|---|---|---|
| `queued` | ANTRE | pending (kuning) |
| `pulling` | MENARIK IMAGE | verifying (biru) |
| `starting` | MEMULAI | verifying (biru) |
| `checking` | HEALTH CHECK | verifying (biru) |
| `live` | LIVE | online (hijau) |
| `failed` | GAGAL | unreachable (merah) |
| `cancelled` | DIBATALKAN | unreachable (merah) |
| `unknown` | TIDAK DIKETAHUI | unreachable (merah) |

Non-warna-saja — setiap badge punya `aria-label="Status: {LABEL}"`, sama
pola `web::fleet::badge` Fase 1.

## 5. Yang SENGAJA belum ada (di luar scope Fase 2)

- **Log runtime streaming** — Fase 3 (`docs/prd.md` §"Fase 2 — Log runtime
  streaming"). Halaman ini hanya punya `error_detail` (dipotong 500 karakter,
  hasil `DeployKegagalan::pesan()`), bukan log container penuh/live.
- **Tombol rollback** — Fase 3 juga (butuh `env_versions`, belum ada di Fase 2).
- **Grafik metrik sebelum/sesudah deploy** — Fase 5.
