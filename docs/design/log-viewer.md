# Spesifikasi Desain: Viewer Log (Log Viewer)

Spesifikasi antarmuka untuk komponen Viewer Log pada `mengdep` Fase 3. Komponen ini digunakan di dua tempat: detail log deployment (`GET /deployments/{id}/log`) dan tab log runtime di halaman detail aplikasi (`GET /apps/{id}/logs`).

## 1. Tujuan
Menyediakan antarmuka terpusat bagi operator untuk memantau keluaran teks log (baik proses deploy asinkron maupun container yang sedang berjalan) secara real-time. Antarmuka ini dirancang agar padat informasi, mendukung pemindaian cepat di malam hari, dapat dicari di sisi server, mendukung pembungkusan baris (wrapping), pelacakan otomatis (auto-follow), serta dapat diunduh (khusus log deploy).

## 2. Token Visual Sistem Desain
Menggunakan token visual yang sudah didefinisikan di `src/web/styles.rs`. Kami menambahkan satu token baru khusus untuk area konsol log guna meningkatkan kenyamanan membaca baris teks panjang di latar belakang yang sangat gelap:

*   Latar halaman: `--color-bg-page` (`#111`)
*   Latar panel kontrol/toolbar: `--color-bg-input` (`#1a1a1a`)
*   Garis tepi/border: `--color-border` (`#444`)
*   Teks utama: `--color-text-main` (`#ddd`)
*   Teks redup (untuk stempel waktu/timestamp): `--color-text-muted` (`#888`)
*   Warna sukses (status aktif/streaming): `--color-success` (`#6c6`)
*   Warna peringatan (terputus/reconnecting): `--color-warning` (`#fc3`)
*   Warna bahaya/kegagalan: `--color-danger` (`#f55`)
*   Font monospace: `--font-mono` (`14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace`)
*   **Token Baru**: `--color-bg-log` (`#070707`). *Alasan*: Memberikan kontras kedalaman ekstra antara area luar shell aplikasi dengan area dalam kontainer log, meniru visual terminal fisik (xterm) untuk mengurangi kelelahan mata operator saat melakukan debugging darurat.

## 3. Layout (Sketsa ASCII)

### Tampilan Utama Viewer Log (Deploy & Runtime)
```text
+-----------------------------------------------------------------+
| MENGDEP | [Armada...]                                  [Keluar] |
+-----------------------------------------------------------------+
| < Kembali ke App (api)                                          |
|                                                                 |
| Log Deployment: dep_9a8b7c                                      |
|                                                                 |
| +-------------------------------------------------------------+ |
| | [Peringatan: Teks log berasal dari aplikasi pengguna dan    | |
| |  dapat memuat nilai sensitif/secret.]                       | |
| +-------------------------------------------------------------+ |
| | Cari log...     [Cari] [Batal] | [x] Wrap | [x] Follow | [Unduh] |
| +-------------------------------------------------------------+ |
| | 12:00:01 | [info] Memulai proses pull image...              | |
| | 12:00:02 | [info] Menarik layer sha256:abc...               | |
| | 12:00:05 | [info] Image berhasil ditarik.                   | |
| | 12:00:06 | [info] Membuat container baru...                 | |
| |          |                                                  | |
| |          |                                                  | |
| |                                     [ Kembali ke Bawah \/ ] | |
| +-------------------------------------------------------------+ |
+-----------------------------------------------------------------+
```

## 4. Komponen & State Antarmuka

Komponen Viewer Log terdiri dari **Toolbar Kontrol** di bagian atas (kotak cari, toggle wrap, toggle follow, tombol unduh) dan **Area Konsol Monospace** di bawahnya. 

Berikut adalah spesifikasi perilaku antarmuka untuk 13 state wajib:

### 4.1 Default / Sudah Selesai
*   **Kondisi**: Pengguna membuka log deployment yang sudah selesai (`live`, `failed`, atau `cancelled`), atau log runtime aplikasi saat container dalam kondisi diam (tidak ada streaming aktif).
*   **Perilaku**: Halaman memuat histori log dari disk (untuk deploy) secara statis tanpa membuka koneksi Server-Sent Events (SSE). Toolbar pencarian, toggle wrap, dan unduh aktif. Toggle follow dinonaktifkan secara visual (karena tidak ada data baru yang akan masuk).
*   **Visual**: Area log menampilkan baris-baris log statis. Indikator status bertuliskan `[ARsip]` berwarna `--color-text-muted`.

### 4.2 Sedang Streaming
*   **Kondisi**: Proses deployment sedang berjalan, atau log runtime container aktif yang sedang memancarkan data secara real-time via SSE.
*   **Perilaku**: Koneksi SSE terbuka. Baris log baru ditambahkan ke bagian bawah area konsol secara dinamis. Pelacakan otomatis (auto-follow) aktif.
*   **Visual**: Indikator status berkedip dengan tulisan `[*] STREAMING` berwarna `--color-success`. Baris log baru mengalir masuk secara halus.
*   **Sunyi (Silence)**: Jika tidak ada log baru yang dicetak oleh aplikasi pengguna selama beberapa menit, indikator tetap bertuliskan `[*] STREAMING` berwarna hijau (bukan kuning/merah) untuk menegaskan koneksi masih sehat dan terbuka.

### 4.3 Memuat (Loading)
*   **Kondisi**: Sebelum isi awal data log berhasil ditarik dari server atau sebelum koneksi SSE pertama kali terjalin.
*   **Perilaku**: Area konsol menampilkan indikator pemuatan statis.
*   **Visual**: Menampilkan baris teks: `[*] Memuat log dari server...` berwarna `--color-warning` dengan latar `--color-bg-log`.

### 4.4 Kosong (Empty)
Tergantung pada penyebab kosongnya log, antarmuka menyajikan informasi yang spesifik untuk membantu operator mengambil langkah berikutnya:

#### A. Belum Ada Keluaran (Deployment Baru Dimulai)
*   **Kondisi**: Deployment baru saja diinisialisasi dan prosesor belum sempat menulis baris log pertamanya.
*   **Pesan**: `[i] Menunggu keluaran log pertama dari server...`
*   **Visual**: Teks berwarna `--color-text-muted` di dalam area konsol.

#### B. File Belum Dibuat / Inisialisasi Gagal
*   **Kondisi**: Proses deploy gagal pada tahap sangat awal sebelum file log berhasil diinisialisasi di disk.
*   **Pesan**: `[x] File log tidak ditemukan. Proses deploy kemungkinan gagal sebelum logging dimulai. Langkah perbaikan: Periksa status server target Anda atau jalankan deploy ulang.`
*   **Visual**: Teks berwarna `--color-danger` di dalam kotak peringatan.

#### C. Log Sudah Tersapu Retensi 30 Hari
*   **Kondisi**: Pengguna mengakses log deployment yang telah melewati masa retensi 30 hari.
*   **Pesan**: `[i] Log sudah tidak tersedia karena telah melewati batas retensi penyimpanan selama 30 hari. Langkah perbaikan: Untuk mengaudit aktivitas lama, silakan merujuk pada catatan eksternal sistem CI/CD Anda.`
*   **Visual**: Teks berwarna `--color-text-muted`. Tombol unduh dinonaktifkan.

### 4.5 Terpotong (Truncated)
*   **Kondisi**: File log deploy di disk control plane mencapai batas keras 8 MiB (`truncated=1`).
*   **Perilaku**: Backend berhenti menulis data baru ke disk dan menyisipkan penanda pemotongan. Deploy sendiri tetap berjalan normal hingga selesai.
*   **Visual**: Di bagian bawah area konsol, muncul baris penanda berwarna `--color-warning` dengan latar `--color-bg-input` dan border kuning:
    `--- [!] LOG TERPOTONG: Ukuran log telah melampaui batas maksimum 8 MiB. Proses deploy tetap berjalan normal di server target. Aliran data real-time masih disiarkan secara langsung di tab ini, namun tidak lagi disimpan ke disk. ---`

### 4.6 Terputus Lalu Tersambung Lagi
*   **Kondisi**: Koneksi jaringan antara peramban (browser) dan control plane terputus saat proses streaming log sedang berlangsung, lalu mencoba terhubung kembali.
*   **Perilaku**: HTMX/browser mencoba membangun kembali koneksi SSE.
*   **Visual**:
    *   **Saat terputus/mencoba menyambung**: Indikator status berubah menjadi `[!] MENGHUBUNGKAN ULANG` berwarna `--color-warning`. Tampilkan baris mengambang di bawah area konsol: `[!] Jaringan terputus. Mencoba menghubungkan kembali...`
    *   **Saat berhasil tersambung kembali**: Indikator kembali menjadi `[*] STREAMING` hijau. Baris peringatan mengambang hilang. Pengguna disarankan untuk me-refresh halaman jika ingin memastikan tidak ada baris histori yang terlewat selama masa disconnect.

### 4.7 Tertinggal (Subscriber Lag)
*   **Kondisi**: Browser memproses data terlalu lambat sehingga tertinggal dari buffer antrean siaran in-memory server, yang memicu status `Lagged` dari broadcast channel.
*   **Perilaku**: Backend mengirimkan event penanda khusus `Lagged` yang disisipkan ke area log.
*   **Visual**: Sisipkan baris pembatas berwarna `--color-danger` di dalam konsol log:
    `--- [x] ALIRAN LOG TERTINGGAL: Beberapa baris log terlewat karena aktivitas transfer terlalu padat. Langkah perbaikan: Silakan muat ulang halaman (refresh) untuk mengambil histori log yang utuh dari file disk. ---`

### 4.8 Container Sudah Tidak Ada (Khusus Runtime)
*   **Kondisi**: Container target yang ingin di-tail sudah dihapus atau tidak ditemukan lagi di server target (misalnya karena dideploy ulang dengan container ID baru atau dihapus manual).
*   **Perilaku**: Menghentikan proses pemuatan log. SSE tidak dibuka atau langsung ditutup.
*   **Visual**: Area log menampilkan kotak pesan kegagalan:
    `[x] Container tidak ditemukan di server target. Log runtime tidak dapat ditampilkan lagi. Langkah perbaikan: Silakan periksa tab Riwayat Deployment untuk melihat log deploy terakhir, atau pastikan container dalam keadaan berjalan.`

### 4.9 Belum Ada Container yang Berjalan (Khusus Runtime)
*   **Kondisi**: Tidak ada deployment berstatus `live` untuk aplikasi ini atau `container_id` bernilai `NULL`.
*   **Perilaku**: Tidak ada stream SSE yang dibuka. Status respons HTTP tetap 200.
*   **Visual**: Area log menampilkan pesan informatif:
    `[i] Belum ada container aktif untuk aplikasi ini. Langkah perbaikan: Silakan lakukan deployment pertama Anda untuk melihat log runtime container di sini.`

### 4.10 Terlalu Banyak Sesi Log Terbuka (429, Khusus Runtime)
*   **Kondisi**: Operator membuka lebih dari 4 sesi streaming log runtime secara serentak di berbagai tab peramban (batas Semaphore terlampaui).
*   **Perilaku**: Permintaan SSE ditolak segera dengan status 429.
*   **Visual**: Tampilkan kotak pesan kesalahan berwarna `--color-danger`:
    `[x] Terlalu banyak sesi log runtime aktif terbuka. Aplikasi membatasi maksimal 4 sesi streaming runtime secara bersamaan untuk menghemat memori. Langkah perbaikan: Tutup salah satu tab browser yang sedang memutar log runtime, lalu coba lagi.`

### 4.11 Server Tidak Merespons / Timeout Tahap (504)
*   **Kondisi**: Kegagalan koneksi SSH atau Docker socket forward ke server target mengalami timeout (10 detik untuk koneksi, 15 detik untuk chunk pertama).
*   **Perilaku**: Stream ditutup dengan kode error 504.
*   **Visual**: Menampilkan pesan kesalahan di area log:
    `[x] Batas waktu koneksi ke server target terlampaui saat mencoba menarik log. Langkah perbaikan: Pastikan server target dalam keadaan aktif, jaringan stabil, dan Docker Engine berjalan dengan normal.`

### 4.12 Sesi Dihentikan Setelah 30 Menit (Khusus Runtime)
*   **Kondisi**: Batas durasi maksimum streaming log runtime (30 menit) tercapai.
*   **Perilaku**: Server menutup koneksi SSE secara rapi.
*   **Visual**: Di bagian bawah log, tampilkan baris penutup:
    `--- [i] SESI LOG SELESAI: Aliran log otomatis dihentikan setelah 30 menit demi menghemat bandwidth. Langkah perbaikan: Silakan muat ulang halaman ini untuk memulai sesi streaming baru. ---`

### 4.13 Error / 404
*   **Kondisi**: ID deployment atau ID aplikasi tidak dikenal atau tidak memenuhi pola regex aman `^[A-Za-z0-9]{1,64}$`.
*   **Perilaku**: Menampilkan halaman kesalahan 404 standar melalui `error_page.rs`.
*   **Visual**: Sesuai dengan spesifikasi shell aplikasi `/` dengan pesan kesalahan: `[!] Data log tidak ditemukan. ID yang diminta tidak valid atau telah dihapus.`

---

## 5. Kontrak Interaksi & Fitur

### 5.1 Perilaku Follow Saat Scroll (Auto-Follow)
*   **Aturan Dasar**: Saat halaman pertama kali dimuat atau streaming sedang aktif, opsi **Follow** tercentang secara default. Konsol akan otomatis men-scroll ke baris terbawah setiap kali ada baris baru masuk.
*   **Scroll ke Atas**: Jika pengguna secara sengaja men-scroll ke atas (membaca baris lama), fitur **Follow** otomatis dimatikan (tidak tercentang) untuk mencegah teks melompat-lompat.
*   **Tombol "Kembali ke Bawah"**: Begitu follow mati secara otomatis akibat scroll ke atas, sebuah tombol melayang (`position: absolute`) dengan label `[ Kembali ke Bawah \/ ]` akan muncul di sudut kanan bawah area konsol log.
*   **Klik Tombol**: Mengklik tombol tersebut akan mengaktifkan kembali centang **Follow**, meluncurkan (scroll) area log kembali ke baris paling bawah secara instan, dan menyembunyikan kembali tombol tersebut.
*   **Kehilangan Tombol**: Tombol melayang akan otomatis hilang jika pengguna secara manual men-scroll kembali area log hingga mencapai baris paling bawah.

### 5.2 Toggle Wrap (Pembungkusan Baris)
*   **Wrap Mati (Default)**: Teks log yang panjang akan memanjang secara horizontal tanpa dibatasi lebar kontainer. Pengguna dapat men-scroll secara horizontal di dalam area konsol. Baris-baris log tetap utuh satu baris per baris.
*   **Wrap Aktif**: Teks log yang melebihi lebar area konsol akan dibungkus ke baris berikutnya secara otomatis (`white-space: pre-wrap; word-break: break-all;`). Tidak ada scroll horizontal di area konsol.
*   **Kontrol**: Disediakan checkbox `[ ] Wrap` di toolbar. Mengklik checkbox langsung mengubah gaya CSS area konsol secara instan tanpa memuat ulang data.

### 5.3 Gutter Timestamp (Stempel Waktu)
*   **Format**: Stempel waktu ditampilkan di kolom kiri (gutter) terpisah dari isi log dengan warna teks redup `--color-text-muted` (contoh: `12:04:55 |`).
*   **Baris Tanpa Timestamp**: Jika sebuah baris log tidak diawali dengan stempel waktu dari aplikasi (misal, cetakan multi-baris kosong), gutter timestamp dikosongkan dengan karakter spasi kosong lebar tetap, menjaga agar indentasi teks log di sebelah kanan tetap lurus dan rapi.

### 5.4 Pencarian (Server-side Search)
*   **Cara Kerja**: Kotak input `Cari log...` disediakan di toolbar. Pencarian dieksekusi di sisi server untuk menghemat CPU browser.
*   **Pengiriman**: Mengetik kata kunci dan menekan Enter (atau mengklik tombol `[Cari]`) akan memicu request HTMX `GET /deployments/{id}/log/isi?q=KATA_KUNCI` yang akan mengganti konten area konsol secara parsial.
*   **Pembersihan**: Menekan tombol `[Batal]` atau mengosongkan input cari dan menekan enter akan memicu pemuatan ulang log tanpa filter (`q` kosong).
*   **Batas Hasil Pencarian**: Hasil pencarian dibatasi maksimal 500 baris cocok teratas. Jika melebihi batas tersebut, tampilkan penanda di akhir hasil:
    `--- [i] HASIL DIPOTONG: Ditemukan lebih dari 500 baris yang cocok. Silakan persempit kata kunci pencarian Anda untuk hasil yang lebih spesifik. ---`
*   **Pencarian Terlalu Lama (Timeout 5s)**: Jika query pencarian file log memakan waktu lebih dari 5 detik, hentikan proses dan tampilkan fragmen kesalahan:
    `[x] Pencarian terlalu lama. Kata kunci yang Anda masukkan menghasilkan pencarian yang lambat. Langkah perbaikan: Silakan masukkan kata kunci yang lebih spesifik.`

### 5.5 Unduh (Download - Hanya Log Deploy)
*   **Aturan**: Tombol `[Unduh]` hanya tersedia pada halaman detail log deploy (`GET /deployments/{id}/log`). Untuk log runtime container, tombol ini sengaja ditiadakan karena data tidak disimpan di server control plane.
*   **Nama Berkas**: File yang diunduh diformat sebagai plain text UTF-8 dengan nama berkas otomatis `deploy-{id}.log`.
*   **Pembersihan Retensi**: Jika file sudah dihapus oleh job retensi 30 hari di server, tombol unduh dinonaktifkan dengan tooltip/teks pembantu: `Berkas log telah dihapus berdasarkan aturan retensi 30 hari.`

### 5.6 Peringatan Isi Log (Privacy Warning)
*   **Visual**: Tampilkan kartu kecil statis berwarna `--color-text-muted` di atas toolbar kontrol sebelum log dimulai.
*   **Teks**:
    `Peringatan: Seluruh isi log berikut berasal dari keluaran aplikasi pengguna dan dapat memuat informasi sensitif seperti kunci enkripsi, token, atau kata sandi yang dicetak secara sengaja atau tidak sengaja oleh aplikasi Anda.`

---

## 6. Responsif per Breakpoint

*   **Layar Desktop (>= 48rem / 768px)**:
    *   Toolbar disusun horizontal dalam satu baris (Kotak cari di kiri, opsi Wrap + Follow + Tombol Unduh di kanan).
    *   Area konsol log memiliki padding kiri/kanan `1.5rem` dengan font berukuran `14px`.
*   **Layar Mobile (< 48rem)**:
    *   Toolbar disusun bertumpuk secara vertikal (Baris 1: Kotak cari + tombol cari, Baris 2: Opsi Wrap + Follow + Unduh disusun sejajar).
    *   Untuk menghemat ruang horizontal, stempel waktu (gutter timestamp) disembunyikan secara default, menyisakan teks log utama agar tidak terpotong atau terlalu banyak terbungkus.
    *   Ukuran font teks log diturunkan menjadi `12px` untuk mengoptimalkan jumlah karakter yang terlihat per baris.
    *   Scroll horizontal pada area konsol diisolasi (`overflow-x: auto`) agar tidak menyebabkan scroll horizontal pada seluruh halaman web.

---

## 7. Aksesibilitas (a11y)

*   **Semantic**: Area konsol dibungkus dalam tag `<pre>` dan elemen `<code role="log" aria-label="Log Aplikasi" aria-live="off">`.
*   **Mengapa `aria-live="off"`?**: Kami secara sengaja menonaktifkan `aria-live` otomatis karena log streaming real-time dapat menghasilkan ribuan baris teks per detik. Jika disetel ke `polite` atau `assertive`, pembaca layar akan terus-menerus membacakan teks baru yang masuk tanpa henti, membajak fokus pengguna tunanetra dan membuat aplikasi tidak dapat digunakan.
*   **Navigasi Keyboard**: Kontrol toolbar (input cari, checkbox wrap/follow, tombol unduh) dapat diakses penuh via tombol Tab dengan fokus outline `--color-link` yang tebal dan jelas. Area log sendiri dapat di-scroll menggunakan tombol arah (Arrow keys) keyboard setelah difokuskan.
*   **Kontras**: Teks stempel waktu abu-abu `--color-text-muted` (`#888`) di atas latar `#070707` menghasilkan rasio kontras 4.8:1, memenuhi WCAG AA. Teks log utama `--color-text-main` (`#ddd`) di atas latar `#070707` menghasilkan rasio kontras 12.3:1 (sangat tinggi/mudah dibaca).

---

## 8. Copywriting

Semua teks antarmuka dalam Bahasa Indonesia yang lugas dan informatif:

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Judul Halaman Log Deploy | Log Deployment {id} - Mengdep |
| Judul Halaman Log Runtime | Log Runtime: {app_name} - Mengdep |
| Label Peringatan Privasi | Peringatan: Seluruh isi log berikut berasal dari keluaran aplikasi pengguna dan dapat memuat informasi sensitif seperti kunci enkripsi, token, atau kata sandi yang dicetak secara sengaja atau tidak sengaja oleh aplikasi Anda. |
| Placeholder Cari | Cari log... |
| Tombol Cari | Cari |
| Tombol Batal Cari | Batal |
| Label Wrap Checkbox | Wrap |
| Label Follow Checkbox | Follow |
| Tombol Unduh | Unduh |
| Tombol Kembali Ke Bawah | Kembali ke Bawah \/ |
| Indikator Arsip | [ARSIP] |
| Indikator Streaming | [*] STREAMING |
| Indikator Menghubungkan | [!] MENGHUBUNGKAN ULANG |
| Pesan Memuat | [*] Memuat log dari server... |
| Pesan Menunggu | [i] Menunggu keluaran log pertama dari server... |
| Pesan Log Terpotong | --- [!] LOG TERPOTONG: Ukuran log telah melampaui batas maksimum 8 MiB. Proses deploy tetap berjalan normal di server target. Aliran data real-time masih disiarkan secara langsung di tab ini, namun tidak lagi disimpan ke disk. --- |
| Pesan Jaringan Terputus | [!] Jaringan terputus. Mencoba menghubungkan kembali... |
| Pesan Sesi Selesai 30 Menit | --- [i] SESI LOG SELESAI: Aliran log otomatis dihentikan setelah 30 menit demi menghemat bandwidth. Langkah perbaikan: Silakan muat ulang halaman ini untuk memulai sesi streaming baru. --- |
| Pesan Lag | --- [x] ALIRAN LOG TERTINGGAL: Beberapa baris log terlewat karena aktivitas transfer terlalu padat. Langkah perbaikan: Silakan muat ulang halaman (refresh) untuk mengambil histori log yang utuh dari file disk. --- |
| Pesan Pencarian Terpotong | --- [i] HASIL DIPOTONG: Ditemukan lebih dari 500 baris yang cocok. Silakan persempit kata kunci pencarian Anda untuk hasil yang lebih spesifik. --- |
| Pesan Pencarian Timeout | [x] Pencarian terlalu lama. Kata kunci yang Anda masukkan menghasilkan pencarian yang lambat. Langkah perbaikan: Silakan masukkan kata kunci yang lebih spesifik. |
| Pesan 429 Terlalu Banyak Sesi | [x] Terlalu banyak sesi log runtime aktif terbuka. Aplikasi membatasi maksimal 4 sesi streaming runtime secara bersamaan untuk menghemat memori. Langkah perbaikan: Tutup salah satu tab browser yang sedang memutar log runtime, lalu coba lagi. |
| Pesan 504 Timeout Koneksi | [x] Batas waktu koneksi ke server target terlampaui saat mencoba menarik log. Langkah perbaikan: Pastikan server target dalam keadaan aktif, jaringan stabil, dan Docker Engine berjalan dengan normal. |
| Pesan Container Tidak Ada | [x] Container tidak ditemukan di server target. Log runtime tidak dapat ditampilkan lagi. Langkah perbaikan: Silakan periksa tab Riwayat Deployment untuk melihat log deploy terakhir, atau pastikan container dalam keadaan berjalan. |
| Pesan Belum Ada Container | [i] Belum ada container aktif untuk aplikasi ini. Langkah perbaikan: Silakan lakukan deployment pertama Anda untuk melihat log runtime container di sini. |
| Pesan 404 Tidak Ditemukan | [!] Data log tidak ditemukan. ID yang diminta tidak valid atau telah dihapus. |

---

## 9. Catatan Implementasi untuk Frontend

*   Gunakan library `xterm.js` untuk merender string teks ANSI berwarna secara efisien di dalam area konsol. Jangan gunakan addon xterm lainnya.
*   Area konsol dibungkus dalam div dengan ukuran tinggi tetap (misal, `height: 60vh; min-height: 400px;`) yang mendukung scroll internal (`overflow-y: auto;`).
*   **Deteksi Scroll untuk Auto-Follow**:
    Implementasikan listener event scroll sederhana di JS / HTMX untuk mendeteksi scroll manual ke atas:
    ```javascript
    const logConsole = document.getElementById('log-console');
    const followCheckbox = document.getElementById('follow-checkbox');
    const backToBottomBtn = document.getElementById('back-to-bottom-btn');

    logConsole.addEventListener('scroll', () => {
      // Toleransi 10px dari batas bawah
      const isAtBottom = logConsole.scrollHeight - logConsole.scrollTop <= logConsole.clientHeight + 10;
      if (!isAtBottom) {
        followCheckbox.checked = false;
        backToBottomBtn.classList.remove('hidden');
      } else {
        backToBottomBtn.classList.add('hidden');
      }
    });

    backToBottomBtn.addEventListener('click', () => {
      followCheckbox.checked = true;
      logConsole.scrollTop = logConsole.scrollHeight;
      backToBottomBtn.classList.add('hidden');
    });
    ```
*   Integrasi SSE dengan HTMX dilakukan menggunakan ekstensi `hx-ext="sse"` ke URL `/events/log/deploy/{id}` atau `/events/log/runtime/{id}`. Gunakan attribute `sse-swap` untuk menambahkan data baru ke xterm console (`xterm.write`).
*   Checkbox wrap memicu penggantian kelas CSS:
    ```css
    .log-container-wrap {
      white-space: pre-wrap;
      word-break: break-all;
    }
    .log-container-nowrap {
      white-space: pre;
      overflow-x: auto;
    }
    ```
*   Pastikan textarea cari mengirim data form via HTMX `hx-get` ke `/deployments/{id}/log/isi` dengan parameter `q` dan `tail`. Ketika pencarian dikirim, targetkan response HTMX ke kontainer log untuk re-render isi saja.
*   Peringatan privasi bersifat statis dan langsung di-render oleh mesin Maud.
