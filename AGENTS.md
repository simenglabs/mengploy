# AGENTS.md — mengdep

Konsol web pengguna tunggal untuk mengelola 3–8 VPS: cek status armada, deploy
container, baca log, tanpa SSH manual. Rust + Axum + SQLite, satu binary.

Sumber kebenaran produk: `docs/prd.md`. Stack terkunci di §1.6, invariant lintas
fase di §3, batas peran di §2. Jangan mengubah keputusan di sana tanpa keputusan
eksplisit manusia di luar fase berjalan.

## Keadaan repo sekarang

`src/main.rs` masih `println!("Hello, world!")`. `Cargo.toml` nol dependensi.
Belum ada `migrations/`, `tests/`, `.sqlx/`. Fase 0 PRD **belum dikerjakan**.
Kalau kamu membaca ini dan menemukan modul yang disebut di bawah belum ada,
itu benar — bukan kesalahan pembacaanmu.

## Peta folder dan pemilik

| Path | Isi | Pemilik |
|---|---|---|
| `src/web/**` | Template Maud, `const CSS`, handler render, atribut HTMX | frontend |
| `src/**` (selain `src/web/**`) | Logika domain, wiring Axum, query sqlx, worker, SSH, crypto | backend |
| `Cargo.toml` | Dependensi | backend |
| `migrations/**` | File SQL berurutan | migration |
| `tests/**` | Integration test | qa |
| `docs/plan.md`, `docs/api-contract.md` | Rencana dan kontrak HTTP | planner |
| `docs/design/**` | Spesifikasi antarmuka | uiux |
| `docs/progress.md` | State lintas sesi | orchestrator |
| `docs/prd.md` | PRD | **manusia saja** |

`src/web/**` adalah batas yang **dibuat sengaja** supaya frontend dan backend
tidak pernah menyentuh file yang sama. Semua yang merender HTML tinggal di
`src/web/`; semua yang lain di luar. Planner wajib menegakkan batas ini saat
membagi task. Kalau satu task butuh keduanya, pecah jadi dua task.

Unit test tinggal di modul `#[cfg(test)]` di dalam file sumbernya. `tests/`
hanya untuk integration test.

## Command wajib

Tidak ada Makefile, tidak ada script npm. Hanya cargo:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Ketiganya wajib hijau sebelum agent mana pun melapor selesai. Urutannya tidak
boleh dibalik — `fmt` dulu supaya clippy tidak melaporkan hal yang sudah
ditangani formatter.

Kalau ada `sqlx::query!` yang berubah, tambahkan:

```bash
cargo sqlx prepare
```

`.sqlx/` ikut di-commit tapi **tidak pernah diedit dengan tangan**.

## Konvensi

- **Bahasa.** Semua teks UI, pesan error, komentar, dan nama test dalam Bahasa
  Indonesia. Nama simbol kode dalam bahasa Inggris.
- **Error.** `anyhow::Result` dengan `.context()` yang menyebut operasinya:
  `.context("buka pool tulis")`, bukan `.context("failed")`. Tipe error domain
  untuk kegagalan yang perlu dibedakan pemanggil.
- **Tidak ada `unwrap()`/`expect()`** di luar `#[cfg(test)]`. Termasuk di jalur
  startup — pakai `?` dan biarkan `main` melaporkannya.
- **Import** diurutkan: crate eksternal → `std` → `crate::`, dipisah baris kosong.
- **Query.** `sqlx::query!` (compile-time checked), bukan `sqlx::query()`. Kalau
  `query!` benar-benar tidak bisa dipakai, tulis alasannya sebagai komentar di
  atas query itu.
- **Dua pool.** Pool tulis `max_connections(1)` untuk INSERT/UPDATE/DELETE, pool
  baca untuk SELECT. Menulis lewat pool baca adalah bug, bukan preferensi.
- **Loop latar belakang tidak boleh mati karena satu error.** Catat lewat
  `tracing::warn!` dan lanjutkan iterasi berikutnya.
- **SSH.** Exit code bukan nol bukan error transport. Pisahkan `code` dan
  `stderr`; pemanggil yang memutuskan artinya.
- **Penyederhanaan yang disengaja** ditandai `// ponytail: <batasnya>, upgrade
  saat <kondisi>`.

## Larangan keras

- **Jangan edit `docs/prd.md`.** Itu milik manusia.
- **Jangan edit file migrasi yang sudah ada.** Checksum `sqlx::migrate!` akan
  tidak cocok. Selalu tambah `migrations/NNNN_nama.sql` baru.
- **Jangan edit `.sqlx/**` dengan tangan.** Regenerasi lewat `cargo sqlx prepare`.
- **Jangan edit `Cargo.lock`** langsung.
- **Jangan tambah dependensi** tanpa izin eksplisit manusia. Stack final di
  `docs/prd.md` §1.6.
- **Jangan jalankan git yang menulis** — `add`, `commit`, `push`, `reset`,
  `checkout`, `rebase`, `stash`. Commit selalu manual oleh manusia.
- **Jangan bangun Non-Goals** `docs/prd.md` §1.5: multi-tenant, RBAC, build image
  sendiri, Kubernetes, object storage, terminal web penuh, preview per PR.
- **Jangan bikin abstraksi yang tidak diminta.** Trait dengan satu implementor,
  builder untuk struct tiga field, config untuk nilai yang tidak pernah berubah —
  semuanya ditolak review.
- **Jangan menyentuh file di luar glob kepemilikanmu.** Catat kebutuhannya di
  laporan dan serahkan ke pemiliknya.

## Invariant yang paling sering kesenggol

Daftar lengkap di `docs/prd.md` §3. Yang ini dicek di setiap review:

- Secret tidak pernah dikembalikan ke klien setelah disimpan — termasuk lewat
  pesan error, `tracing::`, dan `Debug` yang diturunkan otomatis.
- Kunci enkripsi tidak pernah di dalam database atau direktori backup.
- Baris log tidak pernah ditulis ke SQLite.
- Tidak ada tindakan destruktif karena server tidak terjangkau.
- Setiap operasi jarak jauh punya timeout per tahap, bukan timeout global.
- Env var lewat `--env-file` mode `0600`, tidak pernah lewat `-e`.
- Image dirujuk dengan digest, tidak pernah dengan tag.

## Definition of done

Sebuah task selesai kalau **semuanya** benar:

- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` hijau
- [ ] `cargo sqlx prepare` dijalankan kalau ada `sqlx::query!` yang berubah
- [ ] Tidak ada `unwrap()`/`expect()` di luar `#[cfg(test)]`
- [ ] Logika non-trivial punya minimal satu test yang benar-benar bisa gagal
- [ ] Tidak ada file yang tersentuh di luar glob kepemilikan agent
- [ ] Kriteria selesai di `docs/plan.md` untuk task ini terpenuhi
- [ ] Laporan akhir memuat: file yang diubah, keputusan teknis, yang belum
      selesai, asumsi yang dipakai
