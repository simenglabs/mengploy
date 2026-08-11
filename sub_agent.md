# Bootstrap Prompt — Sistem Multi-Agent OpenCode

> Cara pakai: jalankan `opencode` di root proyek, pastikan model aktif adalah
> `omniroute/combo`, lalu paste seluruh isi di bawah garis ini.

---

Kamu bertugas membangun sistem multi-agent OpenCode untuk repository ini, dari nol sampai siap pakai. Ikuti fase-fase di bawah **secara berurutan**. Jangan lompat fase. Jangan menulis file apa pun sebelum Fase 2 selesai.

## Aturan mutlak

1. **Jangan mengarang path, command, atau konvensi.** Setiap glob permission, setiap command test, setiap nama folder yang kamu tulis harus berasal dari hasil investigasi nyata di Fase 0 atau dari jawaban saya di Fase 1. Kalau kamu tidak yakin, tanya — jangan tebak.
2. **Jangan pakai field yang sudah deprecated.** `tools` dan `maxSteps` tidak boleh dipakai. Gunakan `permission` dan `steps`.
3. **Jangan bikin agent yang tidak diminta.** Kalau menurutmu ada agent yang perlu ditambah, usulkan di akhir sebagai rekomendasi terpisah, jangan langsung dibuat.
4. Semua prompt agent ditulis dalam **Bahasa Indonesia**, kecuali istilah teknis.

---

## FASE 0 — Investigasi repo

Lakukan sendiri, jangan tanya saya dulu. Kumpulkan:

- Bahasa, framework, dan versinya (baca `package.json` / `go.mod` / `requirements.txt` / `composer.json` / `cargo.toml`, mana pun yang ada)
- Struktur folder sampai kedalaman 3 level. Catat mana yang jelas frontend, backend, shared, config, test.
- Script yang tersedia: build, test, typecheck, lint, format, migrate
- Apakah ada test suite yang benar-benar jalan? Berapa lama kira-kira?
- Sistem desain: ada file token, tema, config Tailwind, atau folder komponen bersama?
- Layer database dan tooling migrasi
- Apakah monorepo? Kalau ya, petakan tiap package.
- Apakah sudah ada `AGENTS.md`, `CLAUDE.md`, `.cursor/rules/`, atau `.opencode/`?

Rangkum temuanmu dalam maksimal 15 baris. Jangan dump isi file.

## FASE 1 — Klarifikasi

Berdasarkan hasil Fase 0, ajukan **maksimal 6 pertanyaan** tentang hal yang benar-benar tidak bisa kamu simpulkan sendiri. Prioritaskan yang paling berdampak ke pemisahan permission:

- Batas kepemilikan folder antara frontend dan backend (terutama file yang ambigu seperti type/schema bersama)
- Command mana yang wajib dijalankan agent sebelum melapor selesai
- Path mana yang **tidak boleh** disentuh agent mana pun (secret, generated file, vendor)
- Apakah proyek ini sudah production (memengaruhi ketatnya permission migrasi)
- Solo atau tim

Berhenti di sini. Tunggu jawaban saya sebelum lanjut.

---

## FASE 2 — Rencana

Setelah saya jawab, tulis rencana singkat: daftar file yang akan kamu buat beserta satu baris isi masing-masing, plus tabel glob permission per agent. Tunggu saya bilang "lanjut" sebelum menulis file.

---

## FASE 3 — Generate file

Setelah saya setujui, buat semua file berikut.

### 3.1 `AGENTS.md` di root

Maksimal 120 baris. Isinya hanya yang setiap agent butuh dan tidak bisa disimpulkan dari kode dalam 3 tool call:

- Deskripsi proyek dalam 2 kalimat
- Peta struktur folder + siapa pemiliknya
- Command wajib (build, test, typecheck, lint) dengan sintaks persis
- Konvensi kode yang non-obvious (pola penamaan, penanganan error, struktur import)
- Larangan keras (file yang tidak boleh diedit, pola yang dilarang)
- Definition of done: apa yang harus benar sebelum sebuah task disebut selesai

Jangan tulis hal yang jelas dari melihat kode. Jangan tulis tutorial framework.

### 3.2 Agent files di `.opencode/agents/`

Buat 10 file berikut. Spesifikasi frontmatter yang valid — **jangan pakai field di luar daftar ini**:

```
description   (WAJIB, satu baris, diawali kata kerja, jelaskan kapan dipakai)
mode          primary | subagent | all
model         string, format provider/model-id
temperature   0.0-1.0
top_p         0.0-1.0 (opsional, jangan dipakai bareng temperature)
steps         integer (batas iterasi)
permission    object
hidden        boolean (hanya untuk mode subagent)
color         hex atau nama tema
disable       boolean
```

Permission key yang tersedia: `read`, `edit`, `glob`, `grep`, `list`, `bash`, `task`, `external_directory`, `todowrite`, `webfetch`, `websearch`, `lsp`, `skill`, `question`, `doom_loop`. Nilainya `"allow"` / `"ask"` / `"deny"`, atau object pattern→nilai untuk `read`, `edit`, `glob`, `grep`, `list`, `bash`, `task`, `external_directory`, `lsp`, `skill`.

**Aturan urutan pattern: rule terakhir yang cocok yang menang.** Jadi selalu taruh `"*"` paling atas, lalu pengecualian di bawahnya.

Model per agent (pakai persis ID ini):

| File | mode | model | temp |
|---|---|---|---|
| `orchestrator.md` | primary | `cc/claude-opus-5` | 0.1 |
| `planner.md` | subagent | `cc/claude-opus-4-8` | 0.1 |
| `uiux.md` | subagent | `antigravity/gemini-3.5-flash-high` | 0.4 |
| `frontend.md` | subagent | `antigravity/gemini-3.5-flash-high` | 0.1 |
| `backend.md` | subagent | `cc/claude-sonnet-5` | 0.1 |
| `migration.md` | subagent | `cmd/xiaomi/mimo-v2.5-pro` | 0.1 |
| `qa.md` | subagent | `cmd/deepseek/deepseek-v4-flash` | 0.2 |
| `reviewer.md` | subagent | `cmd/deepseek/deepseek-v4-flash` | 0.1 |
| `security.md` | subagent | `cc/claude-opus-4-8` | 0.1 |
| `debugger.md` | subagent | `cmd/deepseek/deepseek-v4-flash` | 0.1 |

Peran dan batasan tiap agent:

**orchestrator** — Koordinator. **Tidak menulis kode** (`edit: deny`). Punya `permission.task` yang menyebut eksplisit sembilan subagent lain: `"*": "deny"` dulu, lalu masing-masing `allow`. Prompt-nya harus memuat alur wajib: planner → uiux (kalau ada perubahan UI) → implementer → qa → reviewer → security, plus aturan bahwa setiap delegasi harus menyertakan daftar file yang boleh disentuh dan referensi ke `docs/plan.md`.

**planner** — Investigasi lalu tulis `docs/plan.md` dan `docs/api-contract.md`. Edit hanya di `docs/**`. Bash hanya read-only git. Tidak implementasi.

**uiux** — Tulis spec ke `docs/design/**`. Tidak menulis kode produksi. Prompt wajib memuat kewajiban mendefinisikan **semua state** tiap komponen (default, loading, empty, error, disabled, success), perilaku responsif, aksesibilitas, dan copywriting. Ambil token dari sistem desain yang sudah ada, jangan karang nilai baru.

**frontend** / **backend** — Implementer. Edit dibatasi glob sesuai hasil Fase 0/1. Bash: `"*": "ask"` lalu test/typecheck di-`allow`. Wajib menjalankan verifikasi sebelum melapor. Format laporan akhir: file yang diubah, keputusan teknis yang diambil, yang belum selesai, asumsi yang dipakai.

**migration** — Refactor mekanis skala besar. Tanpa batas `steps`. `git *: deny`. Kalau ketemu ambiguitas desain, catat ke `docs/migration-questions.md` dan lanjut, jangan berhenti bertanya.

**qa** — Adversarial. Prompt-nya harus menyatakan: "tugasmu membuktikan fitur ini rusak, bukan memvalidasi bahwa ia jalan." Edit hanya di folder test. Fokus ke boundary, input kosong, race, dan error path.

**reviewer** — Read-only penuh (`edit: deny`), bash hanya `git diff*` dan `git log*`. Output berupa temuan dengan severity blocking/warning/nit, masing-masing menyebut `file:baris`. Dilarang mengomentari hal yang sudah ditangani formatter.

**security** — Read-only. Fokus: validasi input, authn/authz, kebocoran data, injection, secret hardcoded, dependency rentan, konfigurasi. Output: temuan + severity + lokasi + saran fix konkret.

**debugger** — Read-only untuk kode, boleh bash untuk log dan test. **Dilarang memperbaiki.** Output: root cause, bukti yang mendukungnya, langkah reproduksi minimal, dan saran perbaikan yang diserahkan ke agent domain.

### 3.3 Commands di `.opencode/commands/`

Frontmatter yang valid: `description`, `agent`, `model`, `subtask`. Body-nya template, mendukung `$ARGUMENTS`, `` !`shell command` `` untuk inject output shell, dan `@path` untuk inject isi file.

Buat empat:

- `feature.md` — alur penuh lewat orchestrator, terima deskripsi fitur via `$ARGUMENTS`, inject branch aktif dan `@docs/plan.md`
- `review.md` — hanya reviewer + security terhadap diff yang belum di-commit
- `fix.md` — debugger dulu untuk cari root cause, lalu serahkan ke agent domain
- `ship.md` — typecheck, test, lint, lalu rangkum apa yang siap di-commit

### 3.4 Update `opencode.json`

Jangan tulis ulang dari nol — baca yang sudah ada dan tambahkan:

- Field `instructions` yang menunjuk ke `AGENTS.md`, `docs/api-contract.md`, dan file konvensi lain yang relevan
- Metadata `limit` dan `modalities` untuk tiap model yang saat ini hanya punya `name`
- Blok `permission` global sebagai baseline aman

Pertahankan `plugin` dan `provider` yang sudah ada apa adanya. **Jangan pernah menulis nilai `apiKey` literal** — kalau menemukannya di config, ganti jadi `{env:OMNIROUTE_API_KEY}` dan beri tahu saya.

### 3.5 Folder pendukung

Buat `docs/plan.md`, `docs/api-contract.md`, dan `docs/design/.gitkeep` sebagai placeholder dengan header struktur, supaya agent punya target tulis yang jelas sejak awal.

---

## FASE 4 — Verifikasi

Setelah semua file dibuat, laporkan dalam bentuk checklist:

1. Daftar file yang dibuat beserta jumlah barisnya
2. Tabel: agent → glob edit → apakah ada overlap dengan agent lain (harus nol overlap antara frontend dan backend)
3. Konfirmasi bahwa reviewer dan implementer memakai keluarga model yang berbeda
4. Konfirmasi tidak ada field `tools` atau `maxSteps` di file mana pun
5. Konfirmasi tidak ada secret literal di config
6. Perintah yang harus saya jalankan untuk verifikasi: `opencode models` dan satu tes pemanggilan subagent

Terakhir, tulis maksimal 5 rekomendasi: apa yang menurutmu masih kurang di repo ini agar sistem multi-agent ini benar-benar efektif (biasanya seputar test coverage, kejelasan batas modul, atau konvensi yang belum terdokumentasi). Jangan buatkan file untuk rekomendasi ini — cukup daftar.