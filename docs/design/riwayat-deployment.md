# Spesifikasi Desain: Riwayat Deployment & Navigasi Tab (Deployment History & Tab Navigation)

Spesifikasi antarmuka untuk tab Riwayat Deployment pada detail aplikasi (`GET /apps/{id}/deployments`) serta struktur navigasi tiga tab Aplikasi (Overview, Riwayat Deployment, dan Log Runtime) pada `mengdep` Fase 3.

## 1. Tujuan
Menyediakan catatan historis aktivitas deployment per aplikasi secara transparan kepada operator. Riwayat ini menampilkan status pengaktifan container, durasi, commit Git, digest image kontainer, serta tautan cepat ke detail timeline dan file log. Halaman ini juga berfungsi sebagai pintu masuk investigasi kegagalan deployment lama.

## 2. Token Visual Sistem Desain
Menggunakan token visual yang didefinisikan di `src/web/styles.rs` tanpa membuat token baru:
*   Latar halaman: `--color-bg-page` (`#111`)
*   Latar baris/panel: `--color-bg-input` (`#1a1a1a`)
*   Teks utama: `--color-text-main` (`#ddd`)
*   Teks redup: `--color-text-muted` (`#888`)
*   Garis pembatas/border: `--color-border` (`#444`)
*   Warna link & fokus tab: `--color-link` (`#6cf`)
*   Warna sukses: `--color-success` (`#6c6`)
*   Warna bahaya/kegagalan: `--color-danger` (`#f55`)
*   Warna peringatan: `--color-warning` (`#fc3`)
*   Font: `--font-mono`

---

## 3. Layout (Sketsa ASCII)

### Navigasi Tab & Daftar Riwayat Deployment (`GET /apps/{id}/deployments`)
```text
+-----------------------------------------------------------------+
| MENGDEP | [Armada...]                                  [Keluar] |
+-----------------------------------------------------------------+
| Aplikasi: api                                                   |
|                                                                 |
|  [ Overview ]  [* Riwayat Deployment *]  [ Log Runtime ]        |
|  -------------------------------------------------------------  |
|  [*] Menampilkan 100 riwayat deployment terbaru.                |
|                                                                 |
|  Waktu Mulai      | Status     | Commit  | Digest  | Durasi | Aksi  |
|  -----------------+------------+---------+---------+--------+-------|
|  2026-08-10 12:00 | [LIVE]     | abc9a8b | api@sha | 45s    | [Det] |
|                   |            |         | :7e8f   |        | [Log] |
|  -----------------+------------+---------+---------+--------+-------|
|  2026-08-10 11:30 | [GAGAL]    | fd12345 | api@sha | 1m 12s | [Det] |
|                   | image_not_ |         | :5c3d   |        | [Log] |
|                   | found      |         |         |        |       |
|  -----------------+------------+---------+---------+--------+-------|
|  2026-07-01 09:00 | [LIVE]     | 8b7c6d5 | api@sha | 38s    | [Det] |
|                   |            |         | :1a2b   |        | [Log]*|
|                   |            |         |         |        |       |
|                                                                 |
|  * Log bertanda asterisk [Log]* telah tersapu retensi 30 hari   |
+-----------------------------------------------------------------+
```

---

## 4. Komponen & State Antarmuka

Halaman Detail Aplikasi dirombak menjadi struktur **Navigasi Tiga Tab** yang membagi konten menjadi:
1.  **Overview**: Informasi konfigurasi dasar, daftar domain, dan pengelolaan token deploy (layout as-built Fase 2).
2.  **Riwayat Deployment**: Tabel data historis deployment aplikasi (maksimal 100 terbaru).
3.  **Log Runtime**: Area pemutaran log langsung dari container aktif (spesifikasi `log-viewer.md`).

Berikut adalah spesifikasi perilaku antarmuka untuk masing-masing state wajib di tab Riwayat Deployment:

### 4.1 Default (Ada Isi)
*   **Kondisi**: Aplikasi memiliki data riwayat deployment (1 hingga 100 entri).
*   **Perilaku**: Menampilkan tabel dengan kolom:
    *   *Waktu Mulai*: Tanggal & jam pembuatan deployment dalam format Bahasa Indonesia (`YYYY-MM-DD HH:MM`). Menglink ke halaman `/deployments/{id}`.
    *   *Status*: Menggunakan komponen `badge_deployment` (non-warna-saja, label kapital + `aria-label` deskriptif).
    *   *Commit*: 7 karakter pertama dari `commit_sha` Git, ditampilkan dalam tag `<code>` monospace.
    *   *Digest*: Nama repositori singkat + 7 karakter pertama dari hash SHA256 image (contoh: `api@sha256:7e8f...`), dilengkapi dengan tombol ikon copy.
    *   *Durasi*: Selisih waktu dari `created_at` hingga `finished_at`. Jika deployment masih berjalan, durasi ditandai dengan ikon animasi berjalan `[*]`. Jika terputus tanpa kepastian, durasi ditandai `-`.
    *   *Aksi*: Menampilkan tautan `[Detail]` ke detail timeline (`/deployments/{id}`) dan `[Log]` ke berkas log (`/deployments/{id}/log`).
*   **Visual**: Teks rapi di dalam tabel. Baris yang berstatus `Failed` (Gagal) ditandai dengan latar belakang baris merah sangat tipis (`rgba(255, 85, 85, 0.03)`) untuk kemudahan pemindaian mata.

### 4.2 Empty (Kosong)
*   **Kondisi**: Aplikasi baru terdaftar dan belum pernah dipicu deployment sama sekali (baik sukses maupun gagal).
*   **Perilaku**: Menampilkan panel informasi kosong dengan ajakan tindakan (CTA) yang jelas tentang langkah memicu deployment pertama.
*   **Visual**: Menggunakan pola kotak putus-putus `.fleet-empty` dengan isi pesan:
    `[i] Belum ada riwayat deployment untuk aplikasi ini. Untuk memicu deployment pertama Anda, pastikan Anda telah membuat Token Deploy di tab Overview dan mengonfigurasikannya pada sistem CI/CD Anda.`

### 4.3 Memuat (Loading)
*   **Kondisi**: Browser memohon data riwayat dari database SQLite.
*   **Perilaku**: Render halaman secara sinkron dari server.
*   **Visual**: Mengikuti pemuatan halaman bawaan peramban (browser).

### 4.4 Error
*   **Kondisi**: Terjadi kegagalan pemanggilan database atau kegagalan internal server lainnya.
*   **Perilaku**: Mengalihkan pengguna ke Halaman Error 500 standar melalui `error_page.rs`.
*   **Visual**: Kotak peringatan merah `--color-danger` dengan instruksi perbaikan:
    `[x] Gagal memuat riwayat deployment aplikasi. Langkah perbaikan: Silakan muat ulang halaman ini atau hubungi administrator jika masalah berlanjut.`

### 4.5 State "Menampilkan 100 Terbaru"
*   **Kondisi**: Jumlah deployment di database melebihi 100 entri.
*   **Perilaku**: Menampilkan baris status statis di atas tabel riwayat yang menginformasikan batasan tampilan tanpa paging.
*   **Visual**: Kotak baris teks dengan border `--color-border` berisikan teks peringatan:
    `[*] Menampilkan 100 riwayat deployment terbaru. Catatan audit yang lebih tua dari batas ini telah diarsipkan secara otomatis.`

---

## 5. Spesifikasi Perilaku & Interaksi Khusus

### 5.1 Penanganan Status Deployment `unknown` (Tidak Diketahui)
*   **Kondisi**: Worker kehilangan koneksi ke target server saat proses health check berlangsung, sehingga tidak dapat memverifikasi status akhir kontainer (apakah sukses `live` atau kembali `failed`).
*   **Visual**: Badge status menampilkan teks merah `TIDAK DIKETAHUI` dengan ikon `[?]`.
*   **Teks Penjelas**: Di bawah baris status yang terpengaruh, atau jika pengguna mengklik ikon tanda tanya, tampilkan informasi bantuan:
    `Status kontainer tidak dapat dipastikan oleh control plane karena server target sempat terputus dari jaringan saat verifikasi akhir dilakukan. Sistem menolak mengambil tindakan spekulatif demi mencegah downtime. Langkah perbaikan: Silakan periksa tab Log Runtime untuk melihat apakah container berjalan dengan sehat, atau picu deploy ulang.`
*   **Mitigasi Destruktif**: Sesuai Prinsip Produk 3, sistem tidak menawarkan tombol perbaikan/penghapusan otomatis pada state ini. Pengguna sepenuhnya diarahkan untuk memeriksa status fisik via log runtime.

### 5.2 Tampilan Baris Deployment Gagal (`Failed`)
*   **Penyajian**: Jika deployment berstatus `failed`, baris tabel akan menampilkan label kategori kegagalan (`error_kind`) tepat di bawah badge status dengan teks redup (misalnya, `image_not_found` atau `health_check_timeout`).
*   **Navigasi Investigasi**: Pengguna diarahkan ke tautan `[Log]` di baris tersebut untuk membaca log detail penuh atau `[Detail]` untuk melihat timeline kegagalan per tahap.

### 5.3 Penanganan Log yang Tersapu Retensi 30 Hari
*   **Kondisi**: Tanggal `created_at` deployment lebih lama dari 30 hari, sehingga file log fisiknya di disk telah dihapus oleh job pembersih latar belakang.
*   **Perilaku**: Tautan `[Log]` di kolom Aksi diubah perilakunya secara jujur untuk mencegah error 404:
    1.  Teks tautan berubah dari `[Log]` menjadi `[Log (Terhapus)]`.
    2.  Tautan dinonaktifkan (`disabled` secara visual dan fungsional, tidak dapat diklik atau fokus keyboard dilewati).
    3.  Menampilkan atribut tooltip `title="Log telah dihapus secara permanen setelah melewati masa retensi 30 hari."` saat kursor diarahkan ke elemen.

### 5.4 Pemotongan Commit SHA dan Image Digest
*   **Commit SHA**: Ditampilkan ringkas sebanyak 7 karakter (misal `abc9a8b`) menggunakan elemen `<code>`. Saat elemen diklik, teks lengkap SHA disalin ke clipboard dan memunculkan tooltip kecil konfirmasi `Tersalin`.
*   **Image Digest**: Teks digest kontainer image sangat panjang (misal `ghcr.io/username/app@sha256:7e8f1a2b...`). Pada tabel, teks ini dipotong secara visual menjadi format: `[nama-repo]@sha256:[7-karakter-hash]...` (contoh: `api@sha256:7e8f...`). Sebuah tombol salin berlogo ikon papan klip (clipboard) diletakkan di samping teks untuk menyalin nilai digest lengkap ke clipboard pengguna.

---

## 6. Navigasi Tiga Tab & Perilaku Mobile

*   **Pola Navigasi**: Tab berupa baris menu horizontal di bawah judul aplikasi. Elemen aktif ditandai dengan garis bawah tebal berwarna `--color-link` dan teks berwarna putih, sedangkan tab tidak aktif berwarna `--color-text-muted`.
*   **Perilaku Layar Sempit / Mobile (< 48rem)**:
    *   Tabel riwayat menyembunyikan kolom *Digest* dan *Durasi* untuk mencegah penyempitan kolom yang membuat teks bertumpuk tidak beraturan. Kolom *Waktu*, *Commit*, *Status*, dan *Aaksi* tetap dipertahankan.
    *   Menu tab horizontal dapat di-scroll secara horizontal (`overflow-x: auto`) dengan scrollbar yang disembunyikan agar navigasi tetap rapi dan tidak memotong teks label tab di layar kecil.
    *   Target ketuk tautan aksi (`[Detail]`, `[Log]`) diperlebar menjadi minimum `44px x 44px` agar mudah ditekan oleh jari pengguna.

---

## 7. Aksesibilitas (a11y)

*   **Struktur Tabel**: Tabel wajib menggunakan elemen semantik HTML5 (`<table>`, `<thead>`, `<tbody>`, `<tr>`, `<th>`, `<td>`). Baris header kolom didefinisikan menggunakan `<th scope="col">`.
*   **Label Tautan Spesifik**: Tombol aksi di dalam baris pengulangan (loop) tidak boleh hanya tertulis "Detail" atau "Log" secara mentah untuk pembaca layar. Gunakan `aria-label` yang menyertakan informasi commit:
    *   `aria-label="Detail deployment commit abc9a8b"`
    *   `aria-label="Unduh berkas log deployment commit abc9a8b"`
*   **Status Non-Warna**: Informasi keberhasilan atau kegagalan deployment tidak hanya bergantung pada warna badge (hijau/merah), melainkan diperjelas dengan teks label status kapital yang tegas (`LIVE`, `GAGAL`, `DIBATALKAN`).

---

## 8. Copywriting

Semua teks antarmuka dalam Bahasa Indonesia yang lugas dan informatif:

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Judul Halaman Tab Riwayat | Riwayat Deployment: {app_name} - Mengdep |
| Label Tab 1 | Overview |
| Label Tab 2 | Riwayat Deployment |
| Label Tab 3 | Log Runtime |
| Header Baris Tampilan Batas | [*] Menampilkan 100 riwayat deployment terbaru. Catatan audit yang lebih tua dari batas ini telah diarsipkan secara otomatis. |
| Header Kolom Waktu | Waktu Mulai |
| Header Kolom Status | Status |
| Header Kolom Commit | Commit |
| Header Kolom Digest | Image Digest |
| Header Kolom Durasi | Durasi |
| Header Kolom Aksi | Aksi |
| Teks Aksi Detail | [Detail] |
| Teks Aksi Log Aktif | [Log] |
| Teks Aksi Log Terhapus | [Log (Terhapus)] |
| Penjelas Tooltip Log Terhapus | Log telah dihapus secara permanen setelah melewati masa retensi 30 hari. |
| Pesan Riwayat Kosong | [i] Belum ada riwayat deployment untuk aplikasi ini. Untuk memicu deployment pertama Anda, pastikan Anda telah membuat Token Deploy di tab Overview dan mengonfigurasikannya pada sistem CI/CD Anda. |
| Penjelas Status Unknown | Status kontainer tidak dapat dipastikan oleh control plane karena server target sempat terputus dari jaringan saat verifikasi akhir dilakukan. Sistem menolak mengambil tindakan spekulatif demi mencegah downtime. Langkah perbaikan: Silakan periksa tab Log Runtime untuk melihat apakah container berjalan dengan sehat, atau picu deploy ulang. |
| Pesan Konfirmasi Salin Commit | Commit SHA berhasil disalin! |
| Pesan Konfirmasi Salin Digest | Image Digest berhasil disalin! |

---

## 9. Catatan Implementasi untuk Frontend

*   Fungsi render pada `src/web/apps.rs` dirombak untuk menerima parameter `tab` aktif guna memisahkan blok visual yang ditampilkan:
    `pub fn render_app_detail(app: &AppRingkas, server_name: &str, deploys: &[DeploymentRingkas], active_tab: &str, csrf_token: &str) -> Markup`
*   Gunakan transisi CSS sederhana untuk memperhalus perpindahan tab jika dimuat ulang:
    ```css
    .tab-nav {
      display: flex;
      gap: 1.5rem;
      border-bottom: 1px solid var(--color-border);
      margin-bottom: 1.5rem;
      overflow-x: auto;
    }
    .tab-link {
      color: var(--color-text-muted);
      text-decoration: none;
      padding: 0.5rem 0.25rem;
      border-bottom: 2px solid transparent;
      white-space: nowrap;
    }
    .tab-link.active {
      color: var(--color-text-main);
      border-bottom-color: var(--color-link);
      font-weight: bold;
    }
    ```
*   Implementasikan fungsi menyalin teks klip menggunakan Clipboard API bawaan browser untuk commit SHA dan digest pada saat tombol diklik.
*   Jika tab yang aktif adalah `Logs` (Log Runtime), panggil fungsi inisialisasi terminal `xterm.js` dan bangun koneksi SSE sesuai spesifikasi `log-viewer.md`.
