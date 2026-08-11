# Changelog

Semua perubahan penting pada **mengploy** dicatat di sini.
Format mengikuti prinsip Keep a Changelog dan versi release memakai Semantic
Versioning.

## [Unreleased]

- Menyiapkan branding produk `mengploy` tanpa memutus crate/binary dan
  environment variable `MENGDEP_*` yang sudah digunakan.
- Menambahkan installer `curl | sh` dengan verifikasi SHA-256.
- Menambahkan CI dan automatic GitHub Release untuk Linux serta macOS.
- Memperbarui panduan menjalankan aplikasi tanpa Rust pada mesin operator.

## [0.1.0] - 2026-08-11

### Ditambahkan

- Konsol web SQLite untuk registry server, deployment container, log, metrik,
  rekonsiliasi, notifikasi webhook HMAC, dan operasi armada.
- Enkripsi `age` untuk private key SSH dan token registry.
- Migrasi database otomatis saat startup.
- Hardening Fase 7 untuk lease deployment, handoff `live`, output operasi,
  cleanup kooperatif, dan hasil parsial per target.
- Test suite lintas fase dengan validasi `cargo fmt`, Clippy, SQLx, dan Cargo
  test.

### Kompatibilitas

- Nama crate dan executable Cargo masih `mengdep` pada release awal ini.
- `MENGDEP_*`, path database, dan file kunci lama tetap didukung.
- Installer memasang executable dengan nama produk `mengploy`.
