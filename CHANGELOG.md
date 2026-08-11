# Changelog

Semua perubahan penting pada **mengploy** dicatat di sini.
Format mengikuti prinsip Keep a Changelog dan versi release memakai Semantic
Versioning.

## [0.1.3] - 2026-08-11

### Diperbaiki

- **Verifikasi server selalu gagal `auth_rejected` saat private key di-paste
  dengan line-ending CRLF.** OpenSSH menolak file kunci yang barisnya
  berakhiran `\r\n` (`invalid format`), sehingga kunci yang sebenarnya sudah
  benar dan terdaftar di `authorized_keys` tetap ditolak. Normalisasi kunci
  kini mengubah `\r\n` (dan `\r` klasik) menjadi `\n` sebelum trim; ditambah
  3 test unit (CRLF, CR klasik, dan jaminan kunci LF yang sudah benar tidak
  berubah).
- **Host key server dilaporkan "berubah" secara palsu sehingga server jadi
  `unreachable`.** Pemilihan host key memakai baris pertama output
  `ssh-keyscan` yang berisi beberapa kunci (RSA/ECDSA/ED25519) dengan urutan
  tidak deterministik antar pemanggilan; saat verifikasi tersimpan fingerprint
  RSA tetapi polling berikutnya kebetulan mendapat ECDSA, aplikasi menganggap
  host key berubah padahal server sama. Kini host key dipilih dengan urutan
  preferensi tetap **ed25519 → ecdsa → rsa** (fallback baris pertama bila
  ketiganya tidak ada), sehingga fingerprint stabil antar siklus; ditambah 5
  test unit untuk urutan, kelipatan kunci, dan fallback.

## [0.1.2] - 2026-08-11

## [0.1.2] - 2026-08-11

### Ditambahkan

- **Tombol Hapus Server di halaman Detail Server.** `POST /servers/{id}/hapus`
  menghapus server beserta seluruh data terkait (app, deployment, log, metrik,
  alert, tautan registry, lock fleet) dalam satu transaksi, dengan konfirmasi
  browser sebelum submit. Id tidak dikenal → 404; hanya bisa lewat form POST
  ber-CSRF, tidak ada route GET yang menghapus.
- **Panduan langkah menyiapkan akses SSH di wizard Tambah Server.** Tiga
  langkah: buat pasangan kunci (`ssh-keygen`), daftarkan kunci PUBLIK ke
  `~/.ssh/authorized_keys` server target (`ssh-copy-id`), lalu tempel kunci
  PRIVAT ke form — termasuk peringatan bahwa server hanya menerima kunci
  publik di `authorized_keys`, bukan kunci privat.

### Diperbaiki

- **Private key SSH valid ditolak server saat verifikasi.** Penyimpanan kunci
  memakai `.trim()` yang membuang newline penutup `-----END OPENSSH PRIVATE
  KEY-----`; OpenSSH menolak file kunci tanpa newline akhir (`invalid format`)
  sehingga verifikasi selalu gagal dengan "Kunci privat ditolak" meskipun
  public key sudah benar di `authorized_keys`. Kini kunci dinormalisasi:
  whitespace tepi dibuang tetapi newline akhir dijamin ada; ditambah test unit
  untuk tiga kasus (newline ada, hilang, whitespace tepi).

## [0.1.1] - 2026-08-11

### Diperbaiki

- **404 saat masuk ke halaman Verifikasi Server.** Tombol "Jalankan Verifikasi"
  dan "Lihat Progres Verifikasi" di halaman Detail Server memakai href relatif
  (`verifikasi`) sehingga browser meresolusi ke `/servers/verifikasi` yang tidak
  punya route. Kini memakai href absolut `/servers/{id}/verifikasi`; ditambah
  test regresi yang melarang href relatif muncul lagi.

### Ditambahkan

- Installer `curl | sh` kini otomatis menulis `INSTALL_DIR` ke PATH pada shell
  config user: prioritas `~/.zshrc`, fallback `~/.bashrc`, dan membuat
  `~/.bashrc` bila keduanya tidak ada. Idempoten (tidak menumpuk duplikat) dan
  me-source config otomatis bila shell aktif cocok (bash↔`.bashrc`,
  zsh↔`.zshrc`).
- Target release dipersempit ke Linux x86_64 dan macOS Apple Silicon (arm64/M1).

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
