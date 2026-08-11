---
description: Kerjakan satu fase dari PRD sampai lolos review
agent: orchestrator
subtask: false
---

Kerjakan Fase $ARGUMENTS.

PRD: @docs/prd.md
Plan: @docs/plan.md
Progress: @docs/progress.md
Branch: !`git branch --show-current 2>/dev/null || echo "(belum ada commit)"`
Status: !`git status --short 2>/dev/null || echo "(repo kosong)"`

Langkah:

1. Baca progress.md. Kalau fase ini sudah ada tahap yang selesai,
   LANJUTKAN dari situ — jangan ulang dari awal.
2. Kalau plan.md belum memuat fase ini, panggil @planner dulu lalu
   BERHENTI. Laporkan ke saya untuk review sebelum implementasi.
3. Kerjakan hanya task milik fase ini. Task fase lain diabaikan
   sepenuhnya, sekalipun terlihat sepele.
4. Update progress.md setiap satu subagent selesai.
5. Patuhi kondisi berhenti di system prompt-mu.
