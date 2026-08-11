# Spesifikasi Desain: Shell Aplikasi (Dashboard Kosong & Halaman Error)

Spesifikasi antarmuka untuk kerangka aplikasi (shell) utama pasca-login pada `mengdep` Fase 0.

## 1. Tujuan
Menyediakan struktur tata letak (layout) yang konsisten untuk seluruh halaman aplikasi setelah pengguna berhasil masuk. Shell ini terdiri dari Sidebar untuk navigasi dasar (logout), Header untuk status sistem, dan Area Konten Utama yang bersifat fleksibel. Pada Fase 0, area konten diisi dengan panel placeholder inisialisasi sistem. Dokumen ini juga menspesifikasikan halaman kesalahan (error) 404 dan 500.

## 2. Token Visual
Shell aplikasi menggunakan token visual yang sama dengan yang didefinisikan di `docs/design/login.md` sebagai sumber kebenaran tunggal:
*   Latar halaman: `--color-bg-page` (`#111`)
*   Teks utama: `--color-text-main` (`#ddd`)
*   Teks redup: `--color-text-muted` (`#888`)
*   Garis tepi/border: `--color-border` (`#444`)
*   Tautan: `--color-link` (`#6cf`)
*   Warna sukses: `--color-success` (`#6c6`)
*   Warna bahaya/kesalahan: `--color-danger` (`#f55`)
*   Font: `--font-mono` (`14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace`)
*   Padding halaman: `--page-padding` (`2rem`)

## 3. Layout (Sketsa ASCII)
Layout terbagi menjadi dua kolom utama pada layar desktop: Sidebar di sisi kiri dan area konten di sisi kanan yang memiliki Header di bagian atas.

```text
+-----------------------------------------------------------------+
| MENGDEP [Fase 0]  | Status: Aktif                      [Keluar] |
|                   +---------------------------------------------+
| > Dashboard       |                                             |
|                   |  +---------------------------------------+  |
|                   |  | SISTEM INisialisasi: SIAP             |  |
|                   |  |                                       |  |
|                   |  | Database SQLite berhasil dimigrasi.   |  |
|                   |  | Konsol siap digunakan untuk Fase 1.   |  |
|                   |  +---------------------------------------+  |
|                   |                                             |
+-----------------------------------------------------------------+
```

## 4. Komponen & Enam State
Shell ini menampung tiga komponen utama: **Sidebar**, **Header**, dan **Area Konten Utama** (termasuk Re-render Halaman Error).

### 4.1 Default (Normal)
*   **Kondisi**: Pengguna mengakses `/` dengan cookie sesi yang valid.
*   **Perilaku**: Layout shell dirender penuh. Sidebar menampilkan navigasi aktif ke "Dashboard" dan tombol "Keluar" (Logout). Header menampilkan status sistem "Aktif". Area Konten Utama memuat panel informasi kesiapan sistem.
*   **Visual**: Teks berwarna `--color-text-main`, garis pembatas antar kolom berwarna `--color-border`.

### 4.2 Loading
*   **Kondisi**: Pengguna berpindah halaman atau mengirim form logout.
*   **Perilaku**: Tidak ada status memuat dinamis Fase 0. Transisi antar halaman dikelola penuh oleh browser secara sinkron.
*   **Visual**: Browser menampilkan indikator pemuatan bawaan miliknya.

### 4.3 Empty
*   **Kondisi**: Database kosong dari data operasional (karena Fase 0 memang belum mengelola data server/armada).
*   **Perilaku**: Area Konten Utama menampilkan kartu placeholder yang menjelaskan bahwa sistem berada di Fase 0 (Fondasi) dan belum ada server terdaftar. Wajib ada ajakan tindakan (CTA) yang jelas.
*   **Visual**: Kartu petunjuk menampilkan teks: "Belum ada server terdaftar. Daftarkan server pertama Anda pada Fase 1 nanti."

### 4.4 Error
Skenario kesalahan di-render menggunakan layout shell yang sama melalui `src/web/error_page.rs` untuk menjaga konsistensi:

#### A. Kesalahan 404 (Halaman Tidak Ditemukan)
*   **Kondisi**: Pengguna mengakses URL yang tidak terdaftar di router.
*   **Pesan**: "Halaman tidak ditemukan. Alamat yang Anda tuju tidak dikenal atau telah dipindahkan."
*   **Visual**: Area Konten Utama menampilkan pesan kesalahan dengan simbol `[!]` berwarna `--color-warning` (`#fc3`) dan petunjuk untuk kembali ke halaman utama.

#### B. Kesalahan 500 (Kesalahan Internal Server)
*   **Kondisi**: Terjadi kepanikan (panic) di backend atau koneksi database SQLite terputus.
*   **Pesan**: "Terjadi kesalahan internal pada server. Silakan hubungi administrator atau periksa log aplikasi."
*   **Visual**: Area Konten Utama menampilkan kotak kegagalan kritis dengan simbol `[x]` berwarna `--color-danger` (`#f55`). Detail kesalahan internal tidak dibocorkan di antarmuka.

### 4.5 Disabled
*   **Kondisi**: Tombol "Keluar" dinonaktifkan.
*   **Perilaku**: Sesuai Invariant 3, tidak ada aksi destruktif yang dapat dilakukan jika server tidak terjangkau. Namun untuk tombol "Keluar", kontrol ini hanya tidak aktif ketika browser sedang mengirimkan request POST logout.
*   **Visual**: Tombol "Keluar" tidak berubah warna tetapi tidak merespons klik ganda selama proses pengiriman form.

### 4.6 Success
*   **Kondisi**: Formulir logout dikirimkan dan disetujui server.
*   **Perilaku**: Sesi dihapus, cookie kedaluwarsa diset, pengguna dialihkan ke `/login`.
*   **Visual**: Browser mengarahkan ke halaman login.

## 5. Responsif
*   **Lebar Layar Desktop (>= 48rem / 768px)**: Sidebar di sebelah kiri dengan lebar tetap `16rem`. Area konten mengambil sisa ruang horizontal.
*   **Lebar Layar Mobile (< 48rem)**: Sidebar berpindah ke atas secara vertikal (stacked) atau berubah menjadi baris menu horizontal ringkas untuk menghemat ruang. Susunan menjadi:
    1.  Header Aplikasi (Logo MENGDEP & Tombol Keluar) di bagian paling atas.
    2.  Menu navigasi horizontal di bawah header.
    3.  Area Konten Utama yang melebar penuh dengan padding kiri/kanan menyusut menjadi `1rem`. Scroll horizontal pada halaman dicegah dengan membungkus teks panjang secara otomatis (`word-break: break-all`).

## 6. Aksesibilitas (a11y)
*   **Struktur Semantik**: Layout wajib menggunakan tag HTML5 yang tepat: `<aside>` untuk sidebar, `<header>` untuk bagian atas, `<main>` untuk area konten, dan `<nav>` untuk daftar menu.
*   **Navigasi Keyboard**: Fokus keyboard bergerak berurutan secara logis: tombol "Keluar" -> menu navigasi -> konten utama.
*   **Tombol Keluar**: Karena logout memicu perubahan state server (`POST /logout`), tombol ini wajib berupa elemen `<button type="submit">` di dalam formulir, bukan sekadar tag tautan `<a>`, untuk mendukung eksekusi keyboard standar (tombol Enter).
*   **Bahasa Konten**: Tag HTML utama wajib disetel ke `lang="id"`.
*   **Kontras**: Teks status dan pesan kesalahan memenuhi syarat kontras WCAG AA terhadap latar belakang `#111`.

## 7. Copywriting
Teks antarmuka menggunakan Bahasa Indonesia dengan nada lugas dan informatif. Tabel ini merupakan sumber kebenaran tunggal dan wajib identik dengan pesan kesalahan pada bagian §4.4.

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Logo / Nama Aplikasi | MENGDEP |
| Label Status Sistem | Status: Aktif |
| Teks Tombol Keluar | Keluar |
| Judul Menu Utama | Dashboard |
| Judul Kartu Placeholder | SISTEM INITIALISASI: SIAP |
| Teks Kartu Placeholder | Sistem berada pada Fase 0 (Fondasi). Database SQLite berhasil dikonfigurasi dan sistem siap menerima fungsionalitas konektivitas pada Fase 1. |
| Judul Kesalahan 404 | [!] Halaman Tidak Ditemukan |
| Isi Kesalahan 404 | Halaman tidak ditemukan. Alamat yang Anda tuju tidak dikenal atau telah dipindahkan. |
| Judul Kesalahan 500 | [x] Kesalahan Internal Server |
| Isi Kesalahan 500 | Terjadi kesalahan internal pada server. Silakan hubungi administrator atau periksa log aplikasi. |
| Teks Tautan Kembali | Kembali ke Dashboard |

## 8. Catatan Implementasi untuk Frontend
*   Layout utama dibentuk menggunakan CSS Grid untuk desktop:
    ```css
    .app-layout {
      display: grid;
      grid-template-columns: 16rem 1fr;
      min-height: 100vh;
    }
    ```
*   Pada mobile, ubah menjadi layout flex vertikal:
    ```css
    @media (max-width: 48rem) {
      .app-layout {
        display: flex;
        flex-direction: column;
      }
    }
    ```
*   Tombol "Keluar" wajib dibungkus dalam form POST ke `/logout` lengkap dengan token CSRF tersembunyi:
    ```html
    <form action="/logout" method="POST" style="margin: 0;">
      <input type="hidden" name="csrf_token" value="...">
      <button type="submit" class="btn-logout">Keluar</button>
    </form>
    ```
*   Untuk halaman error, backend akan merender template `src/web/error_page.rs` yang disisipkan ke dalam layout shell yang sama dengan melampirkan pesan kesalahan yang sesuai.
