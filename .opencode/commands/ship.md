---
description: Jalankan fmt, clippy, dan test lalu rangkum apa yang siap di-commit
agent: orchestrator
subtask: false
---

# Cek kesiapan commit

## Catatan tambahan

$ARGUMENTS

## Hasil verifikasi

Format:

!`cargo fmt --check 2>&1 | tail -20 || echo "ADA DIFF FORMATTING — jalankan cargo fmt"`

Clippy:

!`cargo clippy --all-targets -- -D warnings 2>&1 | tail -30`

Test:

!`cargo test 2>&1 | tail -25`

## Perubahan yang ada

File yang tersentuh:

!`git status --short 2>/dev/null || echo "(repo kosong)"`

Statistik diff:

!`git diff --stat HEAD 2>/dev/null || echo "(belum ada commit pembanding)"`

## Pemeriksaan tambahan

`unwrap()`/`expect()` di luar test:

!`grep -rn "unwrap()\|expect(" src/ 2>/dev/null | grep -v "cfg(test)" | tail -20 || echo "(tidak ada, atau src/ belum ada)"`

Query sqlx di kode: !`grep -rc "sqlx::query!" src/ 2>/dev/null | tail -10 || echo "(tidak ada)"`

Cache sqlx tersimpan: !`ls .sqlx/*.json 2>/dev/null | wc -l | tr -d ' '` file

Progress fase:

@docs/progress.md

## Instruksi

Perintah ini **hanya melaporkan**. Jangan panggil subagent mana pun kecuali
diperlukan untuk menjelaskan sebuah kegagalan, dan jangan mengubah kode.

Baca keluaran di atas lalu rangkum:

1. **Status tiap gate** — fmt, clippy, test. Lolos atau gagal. Kalau gagal, kutip
   baris error yang relevan dan sebutkan agent mana yang harus menanganinya.
2. **`cargo sqlx prepare`** — kalau ada `sqlx::query!` yang berubah di diff,
   ingatkan bahwa perintah itu harus dijalankan dan `.sqlx/` ikut di-commit.
3. **`unwrap()`/`expect()`** — kalau hasil grep di atas menemukan sesuatu di luar
   `#[cfg(test)]`, itu pelanggaran Definition of done. Sebutkan `file:baris`.
4. **Ringkasan perubahan** — kelompokkan file yang tersentuh per domain (frontend /
   backend / migrasi / test / dokumen) dan sebutkan intinya satu baris per kelompok.
5. **Batas kepemilikan** — file yang tersentuh di luar glob pemiliknya menurut
   `AGENTS.md`. Ini gejala delegasi yang bocor, dan perlu dilihat manusia.
6. **Yang perlu diperhatikan sebelum commit** — migrasi lama yang terlihat berubah,
   dependensi baru di `Cargo.toml`, `.sqlx/` yang diedit tangan, atau apa pun yang
   menyerupai secret.
7. **Saran pesan commit** — Bahasa Indonesia, subjek maksimal 50 karakter, plus
   badan singkat kalau alasannya tidak terbaca dari diff.

Tutup dengan **SIAP COMMIT** atau **BELUM SIAP: <alasan>**.

Kamu tidak menjalankan `git add` maupun `git commit`. Itu dikerjakan manusia.
