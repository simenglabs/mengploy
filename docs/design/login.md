# Spesifikasi Desain: Layar Login

Spesifikasi antarmuka untuk halaman masuk (login) pengguna tunggal pada `mengdep` Fase 0.

## 1. Tujuan
Memastikan pengguna tunggal dapat masuk ke konsol secara aman dengan memasukkan kata sandi yang benar. Halaman ini harus ringan, aman, dan dapat diakses dengan keyboard atau pembaca layar tanpa bergantung pada JavaScript.

## 2. Token Visual Sistem Desain
Token ini didefinisikan sebagai CSS custom properties (variabel CSS) di `src/web/styles.rs` dan menjadi sumber kebenaran tunggal untuk seluruh aplikasi.

| Nama Token | Nilai CSS | Peran / Penggunaan |
| :--- | :--- | :--- |
| `--color-bg-page` | `#111` | Latar belakang halaman utama |
| `--color-bg-input` | `#1a1a1a` | Latar belakang field input |
| `--color-bg-btn` | `#2a2a2a` | Latar belakang tombol utama |
| `--color-bg-btn-hover` | `#333` | Latar belakang tombol saat diarahkan (hover) |
| `--color-text-main` | `#ddd` | Warna teks utama |
| `--color-text-muted` | `#888` | Warna teks redup/sekunder (misalnya label pendukung) |
| `--color-border` | `#444` | Warna garis tepi (border) elemen |
| `--color-link` | `#6cf` | Warna teks tautan |
| `--color-success` | `#6c6` | Status sukses atau bar normal |
| `--color-warning` | `#fc3` | Status peringatan |
| `--color-danger` | `#f55` | Status bahaya atau pesan kesalahan |
| `--font-mono` | `14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace` | Font monospace konsol |
| `--page-padding` | `2rem` | Padding luar halaman |
| `--max-form-width` | `32rem` | Batas lebar maksimum kontainer formulir |

## 3. Layout (Sketsa ASCII)
Layout menggunakan pendekatan minimalis di mana kontainer berada tepat di tengah layar secara vertikal dan horizontal.

```text
+-------------------------------------------------------------+
|                                                             |
|                                                             |
|                       +-------------+                       |
|                       |   MENGDEP   |                       |
|                       +-------------+                       |
|                                                             |
|                 +-------------------------+                 |
|                 | Masuk ke Konsol         |                 |
|                 |                         |                 |
|                 | Kata Sandi              |                 |
|                 | [*********************] |                 |
|                 |                         |                 |
|                 | [ Masuk ]               |                 |
|                 +-------------------------+                 |
|                                                             |
|                                                             |
+-------------------------------------------------------------+
```

## 4. Komponen & Enam State
Komponen utama halaman ini adalah **Formulir Login**. Berikut adalah detail perilaku formulir untuk masing-masing dari enam state wajib:

### 4.1 Default
*   **Kondisi**: Pengguna pertama kali memuat `/login` tanpa parameter error.
*   **Perilaku**: Field input kata sandi kosong, kursor fokus berada di input tersebut (`autofocus`), tombol "Masuk" aktif.
*   **Visual**: Input menggunakan latar `--color-bg-input` dengan border `--color-border`.

### 4.2 Loading
*   **Kondisi**: Form telah dikirimkan via `POST /login` dan server sedang melakukan kalkulasi hash Argon2.
*   **Perilaku**: Karena Fase 0 berjalan tanpa JavaScript, status memuat (loading) sepenuhnya bergantung pada indikator pemuatan bawaan browser (native browser loading indicator). Sisi klien tidak melakukan penonaktifan elemen secara dinamis.
*   **Visual**: Tampilan layar tidak berubah secara instan, menunggu respons penuh dari server.

### 4.3 Empty
*   **Kondisi**: Halaman dimuat tetapi tidak ada data konfigurasi awal di database (kasus startup pertama).
*   **Perilaku**: Karena ini sistem pengguna tunggal, kredensial dikonfigurasi melalui variabel lingkungan `MENGDEP_INITIAL_PASSWORD` saat startup pertama (`plan.md` Q5). Jika database belum ter-seed dan variabel lingkungan tidak disetel, server gagal berjalan (unreachable). Jika server berjalan, input tetap kosong dengan petunjuk tindakan yang jelas untuk memasukkan kata sandi yang telah di-seed.
*   **Visual**: Form menampilkan pesan petunjuk: "Masukkan kata sandi awal konsol Anda."

### 4.4 Error
Ada dua skenario error yang dapat terjadi berdasarkan `docs/api-contract.md`:

#### A. Kredensial Salah (401 Unauthorized)
*   **Kondisi**: Pengguna memasukkan kata sandi yang salah.
*   **Pesan**: "Kata sandi salah. Silakan coba lagi."
*   **Visual**: Pesan kesalahan berwarna `--color-danger` ditampilkan di atas kolom input kata sandi. Border kolom input berubah menjadi `--color-danger`.
*   **Persyaratan Keamanan**: Pesan bersifat generik dan tidak membocorkan detail verifikasi Argon2.

#### B. Token CSRF Invalid/Hilang (400 Bad Request)
*   **Kondisi**: Formulir dikirimkan tanpa token CSRF yang valid (misalnya cookie kedaluwarsa atau manipulasi permintaan).
*   **Pesan**: "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi."
*   **Visual**: Kotak peringatan berwarna `--color-danger` ditampilkan di atas formulir.

### 4.5 Disabled
*   **Kondisi**: State di mana kontrol input dimatikan.
*   **Perilaku**: Pada layar login, tidak ada kondisi di mana formulir dinonaktifkan secara sengaja oleh sistem lokal selama server aktif. Jika server tidak terjangkau (unreachable), browser akan menampilkan error koneksi bawaannya sendiri.
*   **Visual**: Tidak ada styling disabled khusus yang dirancang karena kontrol harus selalu dapat menerima masukan saat halaman berhasil dimuat.

### 4.6 Success
*   **Kondisi**: Kata sandi cocok, server memproses sesi baru.
*   **Perilaku**: Server merespons dengan status `303 See Other` menuju lokasi `/` dan menyetel header `Set-Cookie` dengan token sesi.
*   **Visual**: Browser langsung dialihkan ke dashboard utama `/`. Tidak ada layar perantara sukses yang ditampilkan di `/login`.

## 5. Responsif
*   **Lebar Sempit (< 36rem / 576px)**: Padding halaman menyusut menjadi `1rem`. Kontainer formulir melebar memenuhi layar hingga batas padding kiri dan kanan. Tidak ada scroll horizontal.
*   **Lebar Lebar (>= 36rem)**: Lebar kontainer dibatasi pada `--max-form-width` (`32rem`) dan diposisikan di tengah secara horizontal dengan margin otomatis (`margin: 0 auto`).

## 6. Aksesibilitas (a11y)
*   **Bahasa**: Dokumen HTML menggunakan atribut `lang="id"`.
*   **Pelabelan**: Kolom input wajib memiliki elemen `<label for="password">Kata Sandi</label>` yang eksplisit.
*   **Fokus Keyboard**: Elemen input kata sandi memiliki atribut `autofocus` agar pengguna dapat langsung mengetik saat halaman dimuat. Indikator fokus keyboard (`outline`) wajib terlihat jelas dengan warna kontras `--color-link` ketika elemen difokuskan.
*   **Kontras Warna**: Teks `--color-text-main` (`#ddd`) di atas latar `--color-bg-page` (`#111`) menghasilkan rasio kontras 11.6:1 (memenuhi syarat WCAG AA & AAA yang minimal 4.5:1 / 7:1). Pesan error `--color-danger` (`#f55`) di atas `#111` menghasilkan rasio kontras 5.2:1 (memenuhi WCAG AA).
*   **Penyampaian Status**: Status kesalahan tidak boleh disampaikan melalui warna merah border saja. Wajib menyertakan teks pesan kesalahan tertulis yang dapat dibaca oleh pembaca layar (screen reader) dan simbol tanda silang `[x]` di awal pesan.

## 7. Copywriting
Semua teks antarmuka dalam Bahasa Indonesia yang lugas dan tanpa tanda seru.

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Judul Halaman (`<title>`) | Masuk - Mengdep |
| Logo / Nama Aplikasi | MENGDEP |
| Sub-judul Form | Masuk ke Konsol |
| Label Input Kata Sandi | Kata Sandi |
| Placeholder Input | Masukkan kata sandi |
| Teks Tombol Submit | Masuk |
| Pesan Kredensial Salah | [x] Kata sandi salah. Silakan coba lagi. |
| Pesan CSRF Invalid | [x] Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi. |

## 8. Catatan Implementasi untuk Frontend
*   Formulir harus menggunakan method `POST` ke `/login`.
*   Wajib menanam `<input type="hidden" name="csrf_token" value="...">` di dalam tag `<form>`.
*   Input kata sandi menggunakan `<input type="password" id="password" name="password" required autofocus autocomplete="current-password">`.
*   Jangan gunakan skrip JavaScript apa pun untuk validasi klien pada Fase 0. Serahkan validasi langsung ke browser (`required`) dan penanganan error ke backend via HTML re-render.
*   Gunakan layout flexbox atau grid minimal untuk memposisikan kontainer formulir di tengah layar:
    ```css
    body {
      background-color: var(--color-bg-page);
      color: var(--color-text-main);
      font: var(--font-mono);
      display: grid;
      place-items: center;
      min-height: 100vh;
      margin: 0;
      padding: var(--page-padding);
      box-sizing: border-box;
    }
    ```
