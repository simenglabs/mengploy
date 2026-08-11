---
description: Tinjau diff yang belum di-commit lewat reviewer dan security secara paralel, tanpa mengubah kode
agent: orchestrator
subtask: false
---

# Review perubahan yang belum di-commit

## Fokus tambahan dari manusia

$ARGUMENTS

## Diff yang ditinjau

File yang tersentuh:

!`git status --short 2>/dev/null || echo "(repo kosong)"`

Diff belum di-stage:

!`git diff 2>/dev/null || echo "(tidak ada diff)"`

Diff yang sudah di-stage:

!`git diff --cached 2>/dev/null || echo "(tidak ada yang di-stage)"`

Statistik:

!`git diff --stat HEAD 2>/dev/null || git status --short 2>/dev/null || echo "(tidak ada)"`

## Instruksi

Perintah ini **tidak mengubah kode**. Jangan panggil `frontend`, `backend`,
`migration`, `qa`, atau `planner`. Hanya dua agent, dan **jalankan keduanya
paralel** — keduanya read-only, jadi tidak ada risiko tabrakan:

1. **reviewer** — tinjau diff di atas terhadap invariant `docs/prd.md` §3, konvensi
   `AGENTS.md`, penanganan error, dan batas kepemilikan agent.
2. **security** — audit diff yang sama untuk kerentanan yang bisa dieksploitasi.
   Wajib dipanggil, tanpa pengecualian, bahkan untuk perubahan yang terlihat
   kosmetik.

Sertakan ke keduanya: daftar file yang berubah, fokus tambahan dari manusia kalau
ada, dan pengingat batas wilayah masing-masing supaya temuannya tidak duplikat.

Kalau tidak ada diff sama sekali, hentikan dan beri tahu bahwa tidak ada yang
ditinjau. Jangan meninjau kode yang sudah di-commit lama.

## Keluaran

Gabungkan hasil keduanya jadi satu daftar, diurutkan berdasarkan severitas
(CRITICAL dan BLOCKING dulu). Untuk setiap temuan sebutkan `file:baris`, masalahnya,
fix yang disarankan, dan agent mana yang memilikinya. Kalau reviewer dan security
melaporkan hal yang sama, gabungkan jadi satu baris.

Tutup dengan satu kalimat penilaian: **siap di-commit** atau **ada N blocking yang
harus diselesaikan dulu**. Jangan memperbaiki apa pun sendiri.
