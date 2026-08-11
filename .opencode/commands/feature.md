---
description: Jalankan alur fitur penuh untuk permintaan ad-hoc di luar PRD — planner, uiux bila ada UI, implementer, qa, reviewer, security
agent: orchestrator
subtask: false
---

# Fitur ad-hoc

## Permintaan

$ARGUMENTS

## Konteks repo

Branch: !`git branch --show-current 2>/dev/null || echo "(belum ada commit)"`

Status kerja:

!`git status --short 2>/dev/null || echo "(repo kosong)"`

Commit terakhir: !`git log --oneline -3 2>/dev/null || echo "(belum ada commit)"`

## Rencana dan kontrak yang ada sekarang

@docs/plan.md

@docs/api-contract.md

## Instruksi

Permintaan ini **di luar PRD**. Sebelum apa pun, lakukan dua pengecekan:

1. **Cek non-goals.** Kalau permintaan ini ada di `docs/prd.md` §1.5, tolak dan
   jelaskan alasannya. Jangan dikerjakan.
2. **Cek fase.** Kalau permintaan ini sebenarnya milik fase PRD yang belum dibuka,
   katakan fase mana, lalu tanya apakah saya mau tetap melanjutkannya sekarang.
   Jangan menyelundupkan pekerjaan fase lanjut lewat jalur ad-hoc.
3. **Cek north star** (`docs/prd.md` §1.2): berapa sesi SSH manual yang dihilangkan
   permintaan ini? Kalau jawabannya nol, katakan itu sebelum mulai.

Kalau ketiganya lolos, jalankan alur wajib dari system prompt-mu:

1. **planner** — tulis ulang `docs/plan.md` untuk permintaan di atas. Kalau ada
   permukaan HTTP baru, tulis juga `docs/api-contract.md`. Jangan lanjut sebelum
   file itu ada.
2. **uiux** — panggil **hanya** kalau permintaan menyentuh `src/web/**`. Hasilnya
   `docs/design/<fitur>.md`.
3. **Implementer** — `frontend` untuk `src/web/**`, `backend` untuk `src/**` di luar
   `src/web/**` dan `Cargo.toml`, `migration` untuk `migrations/**` atau refactor
   lintas file. Kalau butuh lebih dari satu wilayah, **delegasi terpisah** —
   jangan digabung. Patuhi aturan paralelisasi di system prompt-mu.
4. **qa** — setelah implementer melapor selesai.
5. **reviewer** dan **security** — paralel, keduanya read-only.

Setiap delegasi wajib memuat keempat hal dari blok C system prompt-mu: tujuan task,
daftar file yang boleh disentuh, referensi ke `docs/plan.md` dan
`docs/api-contract.md`, serta definition of done.

Update `docs/progress.md` setiap satu subagent selesai — catat di bawah bagian
khusus untuk pekerjaan ad-hoc, jangan dicampur ke checklist fase PRD.

Jangan menjalankan git yang menulis. Commit dikerjakan manusia.
