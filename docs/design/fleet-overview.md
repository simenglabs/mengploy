# Spesifikasi Desain: Overview Armada (Fleet Overview)

Spesifikasi antarmuka untuk halaman Overview Armada (daftar server) dan Fleet Strip pendukung pada `mengdep` Fase 1.

## 1. Tujuan
Menyediakan antarmuka terpusat bagi operator untuk memantau status seluruh server target (3-8 VPS), melihat informasi dasar OS/Docker, dan menavigasi ke detail server atau alur penambahan server baru. Halaman ini juga menyertakan komponen "Fleet Strip" yang selalu menempel di setiap halaman terlindungi sebagai status bar armada yang cepat dipindai.

## 2. Token Visual Sistem Desain
Menggunakan token visual yang telah dikunci di `src/web/styles.rs` tanpa membuat token baru. Pemetaan status server ke token warna yang sudah ada adalah sebagai berikut:

| Status Server | Visual / Kelas CSS | Token Warna | Kegunaan |
| :--- | :--- | :--- | :--- |
| `pending` | Badge Abu-abu | `--color-text-muted` (`#888`) | Menunggu verifikasi pertama kali dijalankan |
| `verifying` | Badge Kuning berkedip | `--color-warning` (`#fc3`) | Proses verifikasi sedang berlangsung |
| `online` | Badge Hijau | `--color-success` (`#6c6`) | Koneksi & Docker sehat |
| `unreachable` | Badge Merah | `--color-danger` (`#f55`) | Gagal terhubung/terjadi error berturut-turut |

## 3. Layout (Sketsa ASCII)

### A. Layout Halaman Overview Armada (GET /servers)
```text
+-----------------------------------------------------------------+
25: | MENGDEP [Fase 1]  | [Armada: VPS-1 (ONLINE) | VPS-2 (ALERT)] [Keluar] |
26: |                   +---------------------------------------------+
27: | > Dashboard       |                                             |
28: | > Server          |  Armada Server             [+ Tambah Server] |
29: |                   |                                             |
30: |                   |  +---------------------------------------+  |
31: |                   |  | Nama  | Host   | Status    | Docker  |   |
32: |                   |  |-------|--------|-----------|---------|   |
33: |                   |  | VPS-1 | 1.1.1.1| [ONLINE]  | v24.0.7 |   |
34: |                   |  | VPS-2 | 2.2.2.2| [ALERT]   | -       |   |
35: |                   |  +---------------------------------------+  |
36: |                   |                                             |
37: +-----------------------------------------------------------------+
```

### B. Fleet Strip (Menempel di Bagian Atas / Header)
```text
+-----------------------------------------------------------------+
| MENGDEP | VPS-1 [o] | VPS-2 [x] | VPS-3 [-]          [Keluar] |
+-----------------------------------------------------------------+
```

## 4. Komponen & Enam State

### 4.1 Tabel Armada (Fleet Table)
Komponen utama halaman `/servers` untuk menampilkan daftar server dari database (`ServerRingkas`).

1. **Default**
   * **Kondisi**: Server terdaftar di database dan berhasil dimuat.
   * **Perilaku**: Menampilkan tabel dengan kolom: Nama Server, Host, Status (Badge), Versi Docker, Info OS, dan Terakhir Dilihat. Nama server berupa tautan aktif menuju `/servers/{id}`.
   * **Visual**:
     * Kolom status menampilkan badge dengan label kapital (`ONLINE`, `UNREACHABLE`, `PENDING`, `VERIFYING`).
     * Baris server yang `unreachable` memiliki latar belakang sedikit lebih redup dan warna teks nama server ditandai dengan warna `--color-danger` (`#f55`) sebagai peringatan visual. Detail error (`last_error_kind`) ditampilkan berupa teks kecil di bawah nama host.

2. **Loading**
   * **Kondisi**: Browser memproses permintaan halaman `/servers`.
   * **Perilaku**: Menggunakan indikator pemuatan browser native. Tidak ada pemuatan asinkron untuk seluruh halaman ini karena backend merender HTML penuh.
   * **Visual**: Tampilan layar bertahan pada kondisi sebelumnya hingga halaman selesai dimuat.

3. **Empty**
   * **Kondisi**: Tidak ada server yang terdaftar di database (`servers` kosong). Response tetap `200 OK`.
   * **Perilaku**: Menyembunyikan tabel dan menampilkan panel kosong (empty state) dengan ajakan tindakan (CTA) yang menonjol untuk menambahkan server pertama.
   * **Visual**: Menampilkan kotak bergaris putus-putus dengan teks: `[!] Belum ada server terdaftar. Daftarkan server pertama Anda untuk mulai mengelola container.` dan di bawahnya terdapat tombol "+ Tambah Server" yang menonjol.

4. **Error**
   * **Kondisi**: Gagal mengambil data server dari database (misalnya koneksi pool baca SQLite bermasalah).
   * **Perilaku**: Menampilkan halaman kesalahan internal server 500.
   * **Visual**: Kotak kegagalan kritis dari `error_page.rs` dengan teks: `[x] Gagal memuat daftar server. Silakan hubungi administrator.`

5. **Disabled**
   * **Kondisi**: Tidak berlaku untuk tampilan tabel umum. Navigasi tidak dapat dinonaktifkan secara sengaja.
   * **Visual**: Tidak ada.

6. **Success**
   * **Kondisi**: Halaman berhasil dimuat penuh dengan data terupdate.
   * **Perilaku**: Pengguna dapat melihat daftar server secara utuh.
   * **Visual**: Tampilan tabel normal tanpa pesan kesalahan.

---

### 4.2 Komponen Fleet Strip
Komponen ringkas yang disisipkan ke dalam `app_shell` (via `render_fleet_strip`) yang menempel di semua halaman terlindungi.

1. **Default**
   * **Kondisi**: Ada 1 hingga 8 server terdaftar.
   * **Perilaku**: Menampilkan daftar nama server beserta badge status bulat kecil secara horizontal di bar header aplikasi di sebelah logo. Setiap item server adalah tautan cepat ke `/servers/{id}`.
   * **Visual**:
     * Badge status: Bulat hijau untuk `online`, bulat merah untuk `unreachable`, bulat abu-abu untuk `pending`, bulat kuning untuk `verifying`.
     * Teks nama server dipotong dengan ellipsis (`text-overflow: ellipsis`) jika terlalu panjang.

2. **Loading**
   * **Kondisi**: Halaman sedang dimuat.
   * **Perilaku**: Dirender di sisi server secara sinkron, tidak ada status memuat dinamis.
   * **Visual**: Mengikuti render HTML halaman induk.

3. **Empty**
   * **Kondisi**: Belum ada server yang terdaftar.
   * **Perilaku**: Menampilkan teks keterangan di area header.
   * **Visual**: Menampilkan teks redup: `Tanpa server terdaftar` dengan tautan `[Tambah Server]` di sampingnya.

4. **Error**
   * **Kondisi**: Sesi tidak valid atau gagal kueri data ringkas armada.
   * **Perilaku**: Halaman dialihkan ke `/login` atau merender error 500 (jika kegagalan db). Pada error 500, Fleet Strip tidak dirender (`app_shell` menerima `Option<Markup>` sebagai `None`).
   * **Visual**: Kosong (tidak tampil).

5. **Disabled**
   * **Kondisi**: Tidak ada status disabled untuk navigasi strip.
   * **Visual**: Tidak ada.

6. **Success**
   * **Kondisi**: Status armada berhasil dimuat dan terupdate.
   * **Perilaku**: Informasi status server akurat sesuai hasil polling terakhir.
   * **Visual**: Badge warna yang cocok di samping nama server.

---

### 4.3 Perilaku Baris Server Tidak Terjangkau (Unreachable)
Server yang gagal dihubungi sebanyak 3 kali berturut-turut oleh worker status polling akan ditandai sebagai `unreachable`. Perilaku khusus pada baris tabel adalah:
* **Visual Spesifik**: Teks nama server berwarna `--color-danger` (`#f55`). Status tertulis `TIDAK TERJANGKAU` dengan badge merah.
* **Indikator Kegagalan**: Menampilkan teks kecil di bawah hostname berisi keterangan jumlah kegagalan berturut-turut, misalnya: `(Gagal: {n}x)`.
* **Detail Kegagalan Terakhir**: Di bawah nama host, tampilkan kategori kesalahan terakhir (`last_error_kind`) dengan warna `--color-warning` (`#fc3`) untuk membantu diagnosis cepat tanpa harus masuk ke halaman detail. Contoh: `[Masalah: Autentikasi Ditolak]`.
* **Aksi**: Baris tetap dapat diklik menuju `/servers/{id}` untuk melihat data historis sebelum server mati atau untuk memicu verifikasi ulang.

## 5. Responsif

* **Lebar Layar Desktop (>= 48rem / 768px)**:
  * Tabel fleet menampilkan seluruh kolom secara horizontal.
  * Fleet Strip menampilkan hingga 8 server secara horizontal berdampingan di header. Jika nama terlalu panjang, teks terpotong rapi.
* **Lebar Layar Mobile (< 48rem)**:
  * Pada tabel fleet, kolom **Info OS** dan **Terakhir Dilihat** disembunyikan untuk mencegah scroll horizontal. Kolom Nama, Host, Status, dan Docker tetap tampil dengan ukuran yang disesuaikan.
  * Fleet Strip diubah dari susunan horizontal sejajar logo menjadi baris navigasi tersendiri yang terletak tepat di bawah logo/header utama. Jika jumlah server mencapai batas maksimum (8 server), item akan dibungkus ke baris baru (`flex-wrap: wrap`) secara rapi tanpa memicu scroll horizontal halaman. Target sentuh minimal tiap tautan server diatur ke `44px x 44px`.

## 6. Aksesibilitas (a11y)

* **Bahasa**: Seluruh halaman menggunakan bahasa Indonesia (`lang="id"`).
* **Markup Tabel**: Tabel menggunakan tag standar `<table class="fleet-table">`, `<thead>`, `<tbody>`, `<tr>`, `<th>`, dan `<td>` dengan atribut `scope="col"` pada header tabel untuk mempermudah pembaca layar.
* **Status Non-warna**: Indikator status tidak boleh hanya bergantung pada warna badge. Setiap badge harus menyertakan teks keterangan tersembunyi untuk pembaca layar (menggunakan kelas utilitas pembaca layar / `sr-only`) atau teks kapital langsung di sebelah badge.
  * Contoh: `<span class="badge badge-success" aria-label="Status: Online">ONLINE</span>`
* **Navigasi Keyboard**: Seluruh tautan server dalam tabel dan Fleet Strip dapat difokuskan menggunakan keyboard (tombol Tab). Fokus ditunjukkan dengan garis tepi `--color-link` (`#6cf`) yang tebal.

## 7. Copywriting

Semua teks antarmuka dalam bahasa Indonesia yang lugas dan informatif.

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Judul Halaman | Overview Armada - Mengdep |
| Judul Konten Utama | Armada Server |
| Tombol Tambah Server | + Tambah Server |
| Header Kolom 1 | Nama |
| Header Kolom 2 | Host / IP |
| Header Kolom 3 | Status |
| Header Kolom 4 | Docker |
| Header Kolom 5 | OS |
| Header Kolom 6 | Terakhir Dilihat |
| Teks Empty State | [!] Belum ada server terdaftar. Daftarkan server pertama Anda untuk mulai mengelola container. |
| Badge Status - `pending` | MENUNGGU |
| Badge Status - `verifying` | VERIFIKASI |
| Badge Status - `online` | ONLINE |
| Badge Status - `unreachable` | TIDAK TERJANGKAU |
| Label Hitung Kegagalan | Gagal: {n} kali |
| Teks Fleet Strip Kosong | Tanpa server terdaftar |
| Tautan Tambah Cepat Strip | [Tambah Server] |

## 8. Catatan Implementasi untuk Frontend

* Halaman overview armada dirender melalui fungsi `render_fleet(servers: &[ServerRingkas], csrf_token: &str) -> Markup`.
* Template layout shell wajib memanggil `render_fleet_strip(servers: &[ServerRingkas]) -> Markup` untuk disisipkan ke area header jika user sudah login.
* Kelas CSS yang digunakan untuk styling tabel:
  ```css
  .fleet-table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 1.5rem;
  }
  .fleet-table th, .fleet-table td {
    border-bottom: 1px solid var(--color-border);
    padding: 0.75rem 1rem;
    text-align: left;
  }
  .fleet-table tbody tr.unreachable-row {
    background-color: rgba(255, 85, 85, 0.05);
  }
  .status-badge {
    display: inline-block;
    padding: 0.2rem 0.5rem;
    font-size: 0.85em;
    border: 1px solid currentColor;
  }
  .status-badge.online { color: var(--color-success); }
  .status-badge.unreachable { color: var(--color-danger); }
  .status-badge.pending { color: var(--color-text-muted); }
  .status-badge.verifying { 
    color: var(--color-warning);
    animation: pulse 1.5s infinite;
  }
  @keyframes pulse {
    0% { opacity: 0.6; }
    50% { opacity: 1; }
    100% { opacity: 0.6; }
  }
  ```
* Pada tampilan mobile (di dalam `@media (max-width: 48rem)`), sembunyikan kolom OS dan Terakhir Dilihat secara eksplisit:
  ```css
  @media (max-width: 48rem) {
    .fleet-table th:nth-child(5),
    .fleet-table td:nth-child(5),
    .fleet-table th:nth-child(6),
    .fleet-table td:nth-child(6) {
      display: none;
    }
  }
  ```
