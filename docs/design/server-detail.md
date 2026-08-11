# Spesifikasi Desain: Detail Server (Server Detail)

Spesifikasi antarmuka untuk kerangka halaman Detail Server pada `mengdep` Fase 1.

## 1. Tujuan
Menyediakan informasi detail mengenai status operasional, konfigurasi dasar, dan integrasi registry dari sebuah server tertentu yang dipilih operator. Halaman ini juga berfungsi sebagai ruang diagnostik untuk mendeteksi penyebab kegagalan server yang statusnya tidak terjangkau (`unreachable`) atau masih tertunda (`pending`/`verifying`).

## 2. Token Visual Sistem Desain
Menggunakan token visual yang sudah didefinisikan di `src/web/styles.rs`:
*   Latar belakang panel informasi: `--color-bg-input` (`#1a1a1a`)
*   Garis tepi pembatas: `--color-border` (`#444`)
*   Warna teks redup: `--color-text-muted` (`#888`)
*   Garis batas peringatan/error: `--color-danger` (`#f55`) dan `--color-warning` (`#fc3`)
*   Font monospace: `--font-mono`

## 3. Layout (Sketsa ASCII)
```text
+-----------------------------------------------------------------+
| MENGDEP | [Armada...]                                  [Keluar] |
+-----------------------------------------------------------------+
|                                                                 |
|  Detail Server: VPS-SG-1                   [Status: ONLINE]     |
|  -------------------------------------------------------------  |
|                                                                 |
|  +---------------------------+   +---------------------------+  |
|  | Kredensial & Jaringan     |   | Spesifikasi & Lingkungan  |  |
|  | Host: 128.199.12.34       |   | Docker Versi: v24.0.7     |  |
|  | Port: 22                  |   | OS Info     : Ubuntu 22.04|  |
|  | User: root                |   | Sidik Jari  : SHA256:abc..|  |
|  +---------------------------+   +---------------------------+  |
|                                                                 |
|  +-----------------------------------------------------------+  |
|  | Registry Terintegrasi                                     |  |
|  | ghcr.io (User: mengdep-deployer)                          |  |
|  +-----------------------------------------------------------+  |
|                                                                 |
|  +-----------------------------------------------------------+  |
|  | Metrik Kinerja (Fase 6)                                   |  |
|  | [!] Metrik kinerja server akan tersedia pada Fase 6.      |  |
|  +-----------------------------------------------------------+  |
|                                                                 |
+-----------------------------------------------------------------+
```

## 4. Komponen & Enam State

### 4.1 Panel Informasi Server
Komponen utama halaman `/servers/{id}` yang menampilkan detail data `ServerRingkas`.

1. **Default (Online / Sehat)**
   * **Kondisi**: Server dalam status `online` dan data berhasil dimuat penuh.
   * **Perilaku**: Menampilkan informasi lengkap yang terbagi dalam dua kolom panel kartu:
     * *Kredensial & Jaringan*: Alamat host/IP, Port SSH, Pengguna SSH (private key **TIDAK PERNAH** ditampilkan).
     * *Spesifikasi & Lingkungan*: Versi Docker target, Info Sistem Operasi (OS), Sidik Jari Host Key (sebagai informasi publik non-secret).
     * *Registry Terintegrasi*: Daftar host registry dan username yang tertaut ke server ini.
   * **Visual**: Teks berwarna `--color-text-main`, status berlabel hijau `ONLINE`.

2. **Loading**
   * **Kondisi**: Browser memuat data detail server dari database.
   * **Perilaku**: HTML dirender penuh dari server secara sinkron.
   * **Visual**: Mengikuti pemuatan halaman bawaan browser.

3. **Empty**
   * **Kondisi**: Data konfigurasi server di database tidak ditemukan untuk ID yang diberikan.
   * **Perilaku**: Menampilkan halaman kesalahan 404 (Halaman Tidak Ditemukan).
   * **Visual**: Pesan kesalahan dari `error_page.rs` dengan teks: `[!] Server tidak ditemukan. ID server tidak dikenal atau telah dihapus.`

4. **Error (Penanganan Server Bermasalah)**
   Ada beberapa skenario tampilan berdasarkan status server target:

   #### A. Server Berstatus `unreachable` (Tidak Terjangkau)
   * **Kondisi**: Server gagal dihubungi 3 kali berturut-turut oleh worker.
   * **Perilaku**: Halaman menampilkan status `TIDAK TERJANGKAU` dengan badge merah. Di bagian paling atas detail, tampilkan panel peringatan berisi kategori kegagalan terakhir (`last_error_kind`).
   * **Visual**: Kotak peringatan bergaris tepi `--color-danger` (`#f55`) dengan teks kesalahan besar di dalamnya.
   * **Pesan Kegagalan & Tindakan Perbaikan**: Menampilkan deskripsi kesalahan yang cocok dengan salah satu kategori kegagalan Fase 1 (mis. *Host Tidak Terjangkau*, *Autentikasi Kunci Ditolak*, *Pengguna Tanpa Akses Docker Socket*) beserta langkah perbaikannya (sesuai spesifikasi `docs/design/tambah-server.md` §4.2 nomor 4). Di bawah pesan kesalahan terdapat tombol "Mulai Verifikasi Ulang" yang mengarah ke `POST /servers/{id}/verifikasi/ulang`.

   #### B. Server Berstatus `pending` atau `verifying` (Belum Selesai Verifikasi)
   * **Kondisi**: Server baru didaftarkan tetapi belum melewati verifikasi awal secara sukses.
   * **Perilaku**: Data seperti Versi Docker, OS Info, dan Registry tertulis `Belum terverifikasi` atau `Sedang diverifikasi`.
   * **Visual**: Menampilkan banner info `--color-warning` (`#fc3`) yang memandu pengguna untuk masuk ke alur verifikasi.
   * **Teks & Aksi**:
     * Untuk status `pending`: `[!] Server ini belum diverifikasi. Silakan jalankan proses pemeriksaan sistem.` Di samping teks tersebut terdapat tombol "Jalankan Verifikasi" yang mengarah ke `GET /servers/{id}/verifikasi`.
     * Untuk status `verifying`: `[*] Proses verifikasi sistem sedang berlangsung.` Di samping teks tersebut terdapat tombol "Lihat Progres Verifikasi" yang mengarah langsung ke halaman checklist real-time `GET /servers/{id}/verifikasi`.

5. **Disabled**
   * **Kondisi**: Sesuai `docs/plan.md`, tidak ada aksi destruktif atau pengeditan data server pada Fase 1. Tombol hapus dan ubah sengaja tidak dibangun.
   * **Perilaku**: Tidak ada kontrol input yang dapat dimanipulasi pengguna pada halaman ini.
   * **Visual**: Tidak ada tombol "Edit" atau "Hapus" yang ditampilkan di layar.

6. **Success**
   * **Kondisi**: Halaman detail berhasil dimuat dan menampilkan data paling mutakhir dari hasil polling terakhir.
   * **Perilaku**: Pengguna dapat melihat seluruh riwayat info sistem dengan benar.
   * **Visual**: Semua kartu informasi terisi rapi.

---

### 4.2 Kartu Placeholder Metrik Kinerja (Non-Goals Fase 1)
Menampilkan placeholder visual untuk grafik metrik yang baru akan diimplementasikan pada Fase 6.

* **Kondisi**: Selalu ditampilkan di bagian bawah halaman detail server selama Fase 1 hingga Fase 5 berjalan.
* **Perilaku**: Kartu ini tidak memuat skrip visualisasi grafik, canvas, atau metrik real-time apa pun.
* **Visual**: Kotak kosong dengan garis tepi `--color-border` berisikan teks penjelas statis.
* **Pesan**: `[i] Metrik kinerja server (CPU, Memori, Disk) akan tersedia pada Fase 6.` menggunakan teks berwarna `--color-text-muted` (`#888`).

## 5. Responsif

* **Lebar Desktop (>= 48rem)**:
  * Informasi Kredensial dan Spesifikasi disusun berdampingan menjadi 2 kolom.
  * Kartu Registry dan Placeholder Metrik mengambil lebar penuh.
* **Lebar Mobile (< 48rem)**:
  * Seluruh kartu ditumpuk secara vertikal menjadi 1 kolom.
  * Padding area konten menyusut menjadi `1rem` tanpa scroll horizontal.
  * Ukuran teks sidik jari host key yang panjang dibungkus otomatis (`word-break: break-all`) agar tidak merusak lebar kontainer mobile.

## 6. Aksesibilitas (a11y)

* **Bahasa**: Halaman menggunakan bahasa Indonesia (`lang="id"`).
* **Semantik**: Menggunakan elemen `<section>` yang diberi label judul yang jelas menggunakan `aria-labelledby` atau tag `<h2>` untuk membedakan kategori informasi bagi pembaca layar.
* **Kontras**: Teks status `unreachable` merah (`#f55`) dan warning kuning (`#fc3`) memenuhi syarat rasio kontras minimal WCAG AA terhadap latar belakang `#111`.
* **Keterbacaan Sidik Jari**: Kode sidik jari SSH host key menggunakan elemen `<code class="host-key">` agar dapat disalin dengan mudah dan dibacakan per huruf/angka secara jelas oleh pembaca layar.

## 7. Copywriting

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Judul Halaman | Detail Server {nama} - Mengdep |
| Label Judul Utama | Detail Server: {nama} |
| Judul Seksi Jaringan | Kredensial & Jaringan |
| Label Host | Alamat Host / IP |
| Label Port | Port SSH |
| Label User | Pengguna SSH |
| Judul Seksi OS | Spesifikasi & Lingkungan |
| Label Docker | Versi Docker |
| Label OS | Informasi OS |
| Label Sidik Jari | Sidik Jari Host Key |
| Judul Seksi Registry | Registry Terintegrasi |
| Teks Tanpa Registry | Tidak ada registry yang ditautkan ke server ini. |
| Judul Seksi Metrik | Metrik Kinerja (Fase 6) |
| Isi Placeholder Metrik | [i] Metrik kinerja server (CPU, Memori, Disk, Sparkline) akan tersedia pada Fase 6. |
| Teks Info Pending | [!] Server ini belum diverifikasi. Silakan jalankan proses pemeriksaan sistem. |
| Tombol Info Pending | Jalankan Verifikasi |
| Teks Info Verifying | [*] Proses verifikasi sistem sedang berlangsung. |
| Tombol Info Verifying | Lihat Progres Verifikasi |

## 8. Catatan Implementasi untuk Frontend

* Halaman detail server dirender melalui fungsi `render_server_detail(server: &ServerRingkas, csrf_token: &str) -> Markup`.
* Kelas CSS yang digunakan untuk membingkus panel detail:
  ```css
  .detail-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1.5rem;
    margin-bottom: 1.5rem;
  }
  .detail-card {
    background-color: var(--color-bg-input);
    border: 1px solid var(--color-border);
    padding: 1.5rem;
  }
  .detail-card h2 {
    margin-top: 0;
    font-size: 1.1rem;
    border-bottom: 1px solid var(--color-border);
    padding-bottom: 0.5rem;
    margin-bottom: 1rem;
  }
  .detail-row {
    display: flex;
    justify-content: space-between;
    margin-bottom: 0.75rem;
  }
  .detail-row span:first-child {
    color: var(--color-text-muted);
  }
  .host-key {
    background-color: var(--color-bg-page);
    padding: 0.2rem 0.4rem;
    font-size: 0.9em;
    word-break: break-all;
  }
  @media (max-width: 48rem) {
    .detail-grid {
      grid-template-columns: 1fr;
    }
  }
  ```
* Tombol "Mulai Verifikasi Ulang" pada status `unreachable` wajib dibungkus dalam form `POST` ke `/servers/{id}/verifikasi/ulang` dengan token CSRF tersembunyi.
