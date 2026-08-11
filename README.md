# mengploy

**mengploy** adalah konsol web pribadi untuk mengelola beberapa VPS: memeriksa
kesehatan server, membaca log, dan mendeploy container OCI melalui SSH.
Aplikasi ini bukan PaaS multi-tenant dan tidak membutuhkan Rust pada mesin
operator ketika dipasang dari release.

> Nama paket/binary Cargo internal saat ini tetap `mengdep` untuk menjaga
> kompatibilitas test dan instalasi lama. Nama produk yang dipakai pengguna
> adalah **mengploy**. Environment variable `MENGDEP_*` juga tetap menjadi
> kontrak kompatibilitas untuk upgrade; release berikutnya akan memigrasikannya
> secara bertahap.

## Fitur dan stack

- Dashboard web server-side rendered dengan Axum, Maud, HTMX, dan SSE.
- SQLite dengan migrasi otomatis dan dua pool baca/tulis.
- Verifikasi server melalui SSH dan pengelolaan container Docker melalui
  socket forward SSH.
- Enkripsi `age` untuk private key SSH dan token registry.
- Operasi armada, deployment, log, metrik, rekonsiliasi, webhook HMAC, dan
  cleanup kooperatif saat lease deployment hilang.

## Instalasi tanpa Rust

Installer release memilih binary sesuai sistem operasi dan arsitektur, memeriksa
checksum SHA-256, lalu memasang executable `mengploy`. Rust tidak perlu
terpasang.

```bash
curl -fsSL https://raw.githubusercontent.com/simenglabs/mengploy/main/install.sh | sh
```

Perilaku installer:

1. Meminta konfirmasi sebelum mengubah sistem.
2. Mengambil release stabil terbaru dari GitHub.
3. Memilih Linux x86_64, macOS x86_64, atau macOS Apple Silicon.
4. Memverifikasi `checksums.txt` sebelum executable dipasang.
5. Memasang ke `/usr/local/bin` bila dapat ditulis; kalau tidak, memakai
   `~/.local/bin` tanpa meminta Rust atau package manager.

Untuk memasang versi tertentu atau lokasi khusus:

```bash
MENGPLOY_VERSION=v0.1.0 MENGPLOY_INSTALL_DIR="$HOME/.local/bin" \
  curl -fsSL https://raw.githubusercontent.com/simenglabs/mengploy/main/install.sh | sh
```

Pastikan `~/.local/bin` ada di `PATH` jika installer memilih lokasi tersebut.

## Menjalankan mengploy

### 1. Siapkan kunci enkripsi

Kunci ini melindungi private key SSH dan token registry di SQLite. Jangan commit
atau membagikan file ini.

```bash
mkdir -p "$HOME/.config/mengploy"
age-keygen -o "$HOME/.config/mengploy/key.age"
chmod 600 "$HOME/.config/mengploy/key.age"
```

`age-keygen` diperlukan sekali untuk membuat kunci; binary `mengploy` sendiri
tidak membutuhkan Rust.

### 2. Siapkan direktori runtime dan log

Untuk penggunaan lokal/macOS, gunakan path yang dapat ditulis oleh user:

```bash
mkdir -p "$HOME/.local/share/mengploy" "$HOME/.local/state/mengploy" \
  "$HOME/.local/run/mengploy"
chmod 700 "$HOME/.local/share/mengploy" "$HOME/.local/state/mengploy" \
  "$HOME/.local/run/mengploy"
```

Untuk Linux produksi, `MENGDEP_RUNTIME_DIR` sebaiknya menunjuk tmpfs privat
(misalnya `/run/mengploy/ssh`) dan `MENGDEP_LOG_DIR` ke direktori data privat.
Aplikasi tidak memilih fallback diam-diam jika direktori wajib tidak tersedia.

### 3. Jalankan

Contoh foreground dengan konfigurasi lokal:

```bash
export MENGDEP_KEY_PATH="$HOME/.config/mengploy/key.age"
export MENGDEP_DB_PATH="$HOME/.local/share/mengploy/mengploy.db"
export MENGDEP_RUNTIME_DIR="$HOME/.local/run/mengploy"
export MENGDEP_LOG_DIR="$HOME/.local/share/mengploy/logs"
export MENGDEP_LISTEN_ADDR="127.0.0.1:3000"
export MENGDEP_INITIAL_PASSWORD='ganti-dengan-password-kuat'

mkdir -p "$HOME/.local/share/mengploy/logs"
chmod 700 "$HOME/.local/share/mengploy/logs"
mengploy
```

Buka <http://127.0.0.1:3000>. `MENGDEP_INITIAL_PASSWORD` hanya dipakai untuk
seed login pertama; hapus dari environment setelah password tersimpan.
Migrasi SQLite dijalankan otomatis saat startup.

> Jika binary dari Cargo masih dipakai, perintah pengembangnya adalah
> `cargo run`. Binary Cargo tersebut bernama `mengdep`; ini hanya alias teknis
> selama masa kompatibilitas, bukan nama produk.

## Konfigurasi

| Variable | Wajib | Default | Keterangan |
|---|---:|---|---|
| `MENGDEP_LISTEN_ADDR` | tidak | `127.0.0.1:3000` | alamat dan port HTTP |
| `MENGDEP_DB_PATH` | tidak | `./data/mengdep.db` | file SQLite |
| `MENGDEP_KEY_PATH` | ya | — | identity `age`, wajib mode `0600` |
| `MENGDEP_RUNTIME_DIR` | tidak | `/run/platform/ssh` | direktori privat/tmpfs untuk material SSH dan socket forward |
| `MENGDEP_LOG_DIR` | tidak | `/var/lib/platform/logs` | direktori log deploy, mode `0700` |
| `MENGDEP_LOG_RETENTION_DAYS` | tidak | `30` | rentang `1`–`3650` |
| `MENGDEP_INITIAL_PASSWORD` | sekali | — | seed password pertama, hapus setelah login pertama |
| `RUST_LOG` | tidak | `info` | filter log terstruktur; hanya relevan saat menjalankan binary |

## Development dan validasi

Prasyarat pengembang: Rust stable edition 2024, SQLite CLI, `age-keygen`, dan
binary sistem `ssh`, `ssh-keyscan`, serta `ssh-keygen`.

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Jika query `sqlx::query!` berubah, siapkan database segar lalu regenerasi cache:

```bash
rm -f .sqlx-dev.db
for file in migrations/*.sql; do sqlite3 .sqlx-dev.db ".read $file"; done
DATABASE_URL=sqlite://.sqlx-dev.db cargo sqlx prepare -- --all-targets
rm -f .sqlx-dev.db
```

`.sqlx/` ikut dikelola Git dan tidak boleh diedit manual.

## CI/CD dan release

- `.github/workflows/ci.yml` menjalankan format check, Clippy, test, dan build
  pada setiap push serta pull request.
- `.github/workflows/release.yml` membangun release ketika tag `v*` dipush,
  lalu membuat GitHub Release berisi archive dan `checksums.txt`.
- Target release saat ini: Linux x86_64, macOS x86_64, dan macOS Apple Silicon.
- Release memaketkan executable sebagai `mengploy`, sehingga instalasi runtime
  tidak membutuhkan Rust.

Contoh membuat release setelah CI hijau:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Workflow membutuhkan permission `contents: write` pada job release; token
bawaan GitHub Actions dipakai, tidak ada secret tambahan untuk upload release.

## Upgrade dari nama lama

Data SQLite dan seluruh `MENGDEP_*` tetap dipertahankan. Saat upgrade, jangan
menghapus file kunci `age` dan jangan membuat kunci baru kecuali memang ingin
kehilangan akses ke secret terenkripsi lama. Installer baru hanya mengganti
binary; konfigurasi data tetap dikelola operator.

## Status proyek

Fase fitur 0–7 telah diimplementasikan dan divalidasi oleh test suite. Hardening
Fase 7 mencakup cleanup lease deployment, handoff `live` bersyarat, pembacaan
output yang menolak symlink pada komponen akhir, dan partial result per target.

Limitasi yang diketahui:

- Belum ada fault-injection end-to-end dengan SSH/Docker nyata; test remote
  menggunakan jalur deterministik dan host/port yang sengaja gagal.
- `O_NOFOLLOW` melindungi komponen file akhir; traversal descriptor penuh untuk
  seluruh pohon filesystem belum diterapkan.
- Operasi SSH/Docker panjang dibatalkan secara kooperatif di checkpoint, bukan
  diputus paksa di tengah system call.

Lihat `CHANGELOG.md` untuk perubahan per versi dan `docs/prd.md` untuk sumber
kebenaran spesifikasi produk.
