---
description: Cari akar penyebab bug lewat debugger dulu, baru serahkan perbaikannya ke agent domain
agent: orchestrator
subtask: false
---

# Perbaiki bug

## Gejala yang dilaporkan

$ARGUMENTS

## Keadaan repo

Perubahan yang belum di-commit:

!`git status --short 2>/dev/null || echo "(repo kosong)"`

Hasil test terakhir:

!`cargo test 2>&1 | tail -40`

Clippy:

!`cargo clippy --all-targets -- -D warnings 2>&1 | tail -20`

## Instruksi

**Urutannya tidak boleh dibalik.**

1. **debugger** — panggil lebih dulu, selalu. Kirimkan gejala di atas, keluaran
   test, dan file yang menurutmu terkait. Tunggu laporannya: akar penyebab, bukti,
   reproduksi minimal, saran perbaikan, observability yang kurang, dan tingkat
   keyakinan.

2. **Evaluasi laporannya.** Kalau keyakinan `rendah`, jangan langsung menyuruh
   memperbaiki — panggil `debugger` lagi dengan konteks tambahan, atau baca sendiri
   bagian yang masih gelap. Perbaikan berdasarkan diagnosis yang salah lebih mahal
   daripada menunggu satu iterasi.

3. **Serahkan ke agent domain** sesuai bagian `DISERAHKAN KE` di laporan debugger,
   mengikuti kepemilikan file di `AGENTS.md`:
   - `src/web/**` → **frontend**
   - `src/**` lainnya, `Cargo.toml` → **backend**
   - `migrations/**` atau refactor lintas file → **migration**
   - `tests/**` → **qa**

   Delegasi wajib memuat **akar penyebab dan bukti** dari debugger, bukan cuma
   gejalanya. Sertakan juga rekomendasi observability dari laporan debugger — itu
   bagian dari perbaikan, bukan tambahan opsional.

4. **qa** — setelah perbaikan selesai, minta test regresi yang **gagal sebelum fix
   dan lolos sesudahnya**. Test yang tidak pernah bisa gagal tidak menutup bug ini.

5. **reviewer** — atas diff perbaikan.

Kalau debugger menyimpulkan bahwa perilaku yang dilaporkan sebenarnya **benar**
(misalnya nilai `None` di siklus pertama karena delta butuh dua sampel), hentikan
alur dan jelaskan itu. Jangan mengubah kode untuk mengakomodasi kesalahpahaman.

Kalau perbaikan menuntut perubahan `docs/api-contract.md`, **berhenti dan tanya** —
itu salah satu kondisi berhenti di system prompt-mu.
