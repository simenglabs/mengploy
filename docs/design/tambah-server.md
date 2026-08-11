# Spesifikasi Desain: Tambah Server (Wizard 3 Langkah)

Spesifikasi antarmuka untuk alur pendaftaran server target baru pada `mengdep` Fase 1. Alur ini dibagi menjadi 3 langkah berurutan: Formulir Kredensial SSH, Checklist Verifikasi via SSE, dan Konfigurasi Registry Opsional.

## 1. Tujuan
Memandu pengguna mendaftarkan VPS baru secara aman tanpa membocorkan private key atau token. Alur verifikasi harus transparan, menampilkan kemajuan langkah demi langkah, menangani kegagalan jaringan/konfigurasi secara terperinci, dan mengonfirmasi identitas server target (TOFU host key) sebelum menyimpan sidik jari secara permanen.

## 2. Token Visual Sistem Desain
Menggunakan token visual dari `src/web/styles.rs`:
*   Garis tepi elemen: `--color-border` (`#444`)
*   Teks peringatan: `--color-warning` (`#fc3`)
*   Teks kegagalan/error: `--color-danger` (`#f55`)
*   Teks sukses: `--color-success` (`#6c6`)
*   Font utama: `--font-mono`

## 3. Layout (Sketsa ASCII)

### Langkah 1: Formulir Kredensial SSH (GET /servers/baru)
```text
+-----------------------------------------------------------------+
| MENGDEP | [Armada...]                                  [Keluar] |
+-----------------------------------------------------------------+
|                                                                 |
|  Tambah Server Baru (Langkah 1/3: Kredensial SSH)               |
|                                                                 |
|  Nama Server  : [ vps-sg-1           ]                          |
|  Host / IP    : [ 128.199.12.34      ] (Tanpa http:// atau :port)|
|  Port SSH     : [ 22                 ]                          |
|  Pengguna SSH : [ root               ]                          |
|  Kunci Privat : +--------------------------------------------+  |
|                 | -----BEGIN OPENSSH PRIVATE KEY-----        |  |
|                 | ...                                        |  |
|                 +--------------------------------------------+  |
|                                                                 |
|  [ Lanjutkan ke Verifikasi ]                                    |
|                                                                 |
+-----------------------------------------------------------------+
```

### Langkah 2: Checklist Verifikasi (GET /servers/{id}/verifikasi)
```text
+-----------------------------------------------------------------+
| MENGDEP | [Armada...]                                  [Keluar] |
+-----------------------------------------------------------------+
|                                                                 |
|  Verifikasi Server (Langkah 2/3: Pemeriksaan Sistem)            |
|                                                                 |
|  [*] Langkah 1: Membangun Koneksi SSH...                        |
|      Sidik jari host key belum terdaftar:                       |
|      SHA256:abc123xyz789...                                     |
|      Apakah Anda mempercayai host ini?                          |
|      [ Ya, Terima & Simpan ]                                    |
|                                                                 |
|  [ ] Langkah 2: Pemeriksaan Lingkungan Docker                   |
|  [ ] Langkah 3: Pemeriksaan Akses Registry                      |
|                                                                 |
+-----------------------------------------------------------------+
```

### Langkah 3: Konfigurasi Registry (GET /servers/{id}/registry)
```text
+-----------------------------------------------------------------+
| MENGDEP | [Armada...]                                  [Keluar] |
+-----------------------------------------------------------------+
|                                                                 |
|  Tautkan Registry (Langkah 3/3: Opsional)                       |
|                                                                 |
|  Pilih Registry Tersimpan:                                      |
|  ( ) ghcr.io (user: mengdep-deployer)                           |
|  ( ) Baru...                                                    |
|                                                                 |
|  Host Registry: [ ghcr.io            ]                          |
|  Username     : [ mengdep-deployer   ]                          |
|  Token Akses  : [ ****************** ]                          |
|                                                                 |
|  [ Tautkan Registry ]            [ Lewati & Selesai ]           |
|                                                                 |
+-----------------------------------------------------------------+
```

---

## 4. Komponen & Enam State

### 4.1 Formulir Kredensial SSH (Langkah 1)
Mengelola input data dasar server dan kunci privat SSH.

1. **Default**
   * **Kondisi**: Pengguna membuka `GET /servers/baru` pertama kali.
   * **Perilaku**: Form kosong, kecuali input Port yang langsung terisi nilai default `22`. Fokus keyboard berada pada input "Nama Server". Textarea "Kunci Privat" kosong dan menampilkan placeholder instruksi.
   * **Visual**: Input normal menggunakan border `--color-border`.

2. **Loading**
   * **Kondisi**: Menunggu pengiriman formulir via `POST /servers` dan enkripsi kunci oleh backend menggunakan `age` sebelum menyimpan draft.
   * **Perilaku**: Dikelola secara sinkron oleh browser.
   * **Visual**: Tampilan tombol "Lanjutkan ke Verifikasi" menunjukkan status menunggu respon.

3. **Empty**
   * **Kondisi**: Pengguna mengirimkan formulir dengan field wajib yang dikosongkan.
   * **Perilaku**: Form ditolak di sisi server (jika bypass HTML5 validation) dan dirender ulang dengan error per field.
   * **Visual**: Field yang kosong ditandai dengan border `--color-danger`. Pesan error "Wajib diisi" ditampilkan di bawah field bersangkutan.

4. **Error**
   * **Kondisi**: Validasi field gagal di backend berdasarkan aturan `docs/api-contract.md` (misalnya skema URL atau port di luar 1-65535).
   * **Perilaku**: Formulir di-render ulang dengan menampilkan pesan kegagalan spesifik Bahasa Indonesia.
   * **Persyaratan Keamanan Kritis**: Textarea kunci privat (`ssh_key`) **TIDAK BOLEH** di-prefill dengan nilai yang salah tersebut. Kolom textarea wajib dikosongkan penuh demi keamanan data sensitif di sisi browser.
   * **Visual & Pesan**:
     * *Host salah*: `Host tidak boleh mengandung skema URL (http://) atau gabungan port (:22). Masukkan alamat IP atau nama domain saja. Langkah perbaikan: Hapus 'http://' atau ':port' dari input Host.`
     * *Port salah*: `Port harus berupa angka bulat dalam rentang 1 - 65535. Langkah perbaikan: Ganti dengan port SSH server target yang benar.`
     * *Kunci privat salah*: `Format kunci privat tidak valid. Langkah perbaikan: Pastikan Anda menyalin seluruh teks kunci termasuk baris pembuka dan penutup OpenSSH.`

5. **Disabled**
   * **Kondisi**: Server tidak dapat memproses penyimpanan karena database terkunci penuh atau kunci enkripsi age tidak terkonfigurasi di server control plane.
   * **Perilaku**: Seluruh input dinonaktifkan (`disabled`). Tombol submit dimatikan.
   * **Visual**: Kontainer form direndahkan opacity-nya, tombol kirim menampilkan teks "Pendaftaran Ditangguhkan". Pesan kesalahan kritis ditampilkan di atas formulir: `[x] Kunci enkripsi aplikasi tidak terkonfigurasi. Langkah perbaikan: Hubungi administrator untuk mengonfigurasi file enkripsi.`

6. **Success**
   * **Kondisi**: Validasi berhasil, baris server disimpan dengan status `pending`, dan kunci privat dienkripsi.
   * **Perilaku**: Pengguna dialihkan ke `GET /servers/{id}/verifikasi` via redirect `303 See Other`.
   * **Visual**: Browser memuat halaman verifikasi Langkah 2.

---

### 4.2 Checklist Verifikasi (Langkah 2)
Pembaruan kemajuan sistem secara asinkron menggunakan Server-Sent Events (SSE) yang terhubung ke `/events/verifikasi/{job_id}`.

1. **Default**
   * **Kondisi**: Pertama kali halaman dimuat. Mulai menghubungkan SSE.
   * **Perilaku**: Menampilkan daftar langkah verifikasi dengan status loading awal.
   * **Visual**: Ikon animasi berputar di samping langkah "Koneksi SSH".

2. **Loading**
   * **Kondisi**: Proses verifikasi sedang berjalan (status `verifying`).
   * **Perilaku**: SSE mengirimkan data real-time, memperbarui checklist secara bertahap tanpa reload halaman.
   * **Visual**: Sub-cek Docker yang berjalan diperbarui satu demi satu.

3. **Empty**
   * **Kondisi**: Tidak ada status tersimpan atau job terputus sebelum mulai.
   * **Perilaku**: Mengajukan verifikasi ulang.
   * **Visual**: Menampilkan tombol "Mulai Verifikasi" jika status server macet di `pending`.

4. **Error (Penanganan Kegagalan Spesifik Fase 1)**
   * **Kondisi**: Salah satu tahap verifikasi mengalami kegagalan.
   * **Perilaku**: Verifikasi berhenti. SSE mengirimkan fragmen error. Tampilkan pesan kesalahan terperinci dan tombol "Verifikasi Ulang" (retry).
   * **Spesifikasi Kegagalan Wajib**:
     * **A. Host Tidak Terjangkau (Timeout 10s)**
       * *Pesan*: `[x] Gagal terhubung ke host target dalam batas waktu 10 detik. Langkah perbaikan: Periksa apakah IP/Host sudah benar, port SSH terbuka, dan firewall memperbolehkan koneksi masuk.`
     * **B. Autentikasi Kunci Ditolak**
       * *Pesan*: `[x] Kunci privat ditolak oleh server target. Langkah perbaikan: Pastikan public key yang sesuai telah didaftarkan pada file '~/.ssh/authorized_keys' pengguna SSH di server target.`
     * **C. Docker Tidak Terpasang**
       * *Pesan*: `[x] Binary Docker tidak ditemukan di server target. Langkah perbaikan: Masuk ke server Anda via terminal luar dan jalankan instalasi Docker Engine terlebih dahulu.`
     * **D. Pengguna Tanpa Akses Docker Socket**
       * *Pesan*: `[x] Pengguna SSH tidak memiliki izin untuk mengakses Unix socket Docker. Langkah perbaikan: Tambahkan pengguna SSH tersebut ke dalam grup OS 'docker' di server target dengan perintah 'usermod -aG docker <user>', lalu verifikasi ulang.`
     * **E. Fingerprint Host Key Berubah (Gagal Keras TOFU)**
       * *Pesan*: `[x] PERINGATAN KEAMANAN: Sidik jari host key yang ditawarkan server berbeda dengan yang telah disimpan sebelumnya! Langkah perbaikan: Jika Anda sengaja mengganti/menginstal ulang server target, Anda harus mendaftarkannya kembali sebagai server baru dengan nama berbeda. Aplikasi menolak menimpa sidik jari tersimpan demi mencegah serangan Man-in-the-Middle.`
       * *Visual*: Menampilkan nilai sidik jari tersimpan (Lama) dan sidik jari yang ditawarkan sekarang (Baru) secara berdampingan dalam kotak merah `--color-danger`. Tidak ada tombol untuk memperbarui sidik jari secara otomatis.
   * **Visual**: Garis tepi checklist berubah menjadi `--color-danger`, status ikon menjadi silang merah `[x]`.

5. **Disabled**
   * **Kondisi**: Mencoba memicu verifikasi ulang saat job sebelumnya masih berjalan (status `409 Conflict`).
   * **Perilaku**: Menolak memulai job baru. Menampilkan pesan peringatan.
   * **Visual**: Tombol "Verifikasi Ulang" dinonaktifkan (`disabled`). Menampilkan teks: `Verifikasi sedang berjalan. Silakan tunggu hingga proses selesai.` dengan warna `--color-warning`.

6. **Success (Termasuk Konfirmasi TOFU)**
   * **Kondisi**:
     * **TOFU Terbaca**: Saat pertama kali terhubung, sidik jari host key ditampilkan. User harus menyetujui secara eksplisit sebelum lanjut ke cek Docker.
     * **Verifikasi Selesai**: Seluruh langkah sukses.
   * **Perilaku**:
     * *TOFU*: Menampilkan kotak dialog konfirmasi dengan tombol "Ya, Terima & Simpan". Tombol ini mengirimkan `POST /servers/{id}/hostkey/konfirmasi`.
     * *Selesai*: Setelah langkah 2 sukses sepenuhnya, tampilkan tombol "Lanjutkan ke Langkah 3 (Registry)".
   * **Visual**: Ikon sukses hijau centang `[o]` di samping setiap langkah.

---

### 4.3 Formulir Kredensial Registry (Langkah 3 - Opsional)
Mengatur login ke registry kontainer jarak jauh agar server target dapat menarik image privat.

1. **Default**
   * **Kondisi**: Memuat halaman `GET /servers/{id}/registry` pertama kali.
   * **Perilaku**: Menampilkan pilihan registry yang telah tersimpan di database (jika ada) berupa daftar radio button, opsi "Baru", dan kolom input kosong untuk pendaftaran registry baru.
   * **Visual**: Input kosong. Di bagian bawah terdapat dua tombol berdampingan: "Tautkan Registry" (tombol utama) dan "Lewati & Selesai" (tombol sekunder).

2. **Loading**
   * **Kondisi**: Server sedang menjalankan `docker login` di target via SSH pasca submit.
   * **Perilaku**: Dikelola secara sinkron oleh browser.
   * **Visual**: Menampilkan teks pemuatan pasif pada tombol submit.

3. **Empty**
   * **Kondisi**: Menekan "Tautkan Registry" saat memilih opsi "Baru" tetapi kolom host/username/token kosong.
   * **Perilaku**: Server mendeteksi kegagalan validasi.
   * **Visual**: Field kosong diberi border `--color-danger`.

4. **Error**
   * **Kondisi**: Kegagalan `docker login` di target (mis. kredensial ditolak oleh host registry) atau timeout.
   * **Perilaku**: Form di-render ulang dengan field token / password dikosongkan.
   * **Visual & Pesan**:
     * *Kredensial Ditolak (422)*: `Kredensial ditolak oleh host registry. Username atau token yang Anda masukkan salah. Langkah perbaikan: Periksa kembali token akses Anda di registry dan pastikan izin read/write sudah benar.`
     * *Timeout Jaringan (504)*: `Batas waktu koneksi ke registry terlampaui. Langkah perbaikan: Periksa apakah server target dapat mengakses internet luar atau apakah registry sedang mengalami gangguan.`

5. **Disabled**
   * **Kondisi**: Tidak ada.
   * **Visual**: Tidak ada.

6. **Success**
   * **Kondisi**: Sukses login ke registry dan izin file target `config.json` diatur ke `0600`.
   * **Perilaku**: Menyimpan relasi server-registry, mengalihkan pengguna ke halaman detail server `GET /servers/{id}`.
   * **Visual**: Browser memuat halaman detail server.

---

## 5. Responsif

* **Lebar Desktop (>= 48rem)**:
  * Formulir input terbagi dalam kolom berjejer (Label di sebelah kiri, kolom input di sebelah kanan).
  * Checklist verifikasi disusun berdampingan dengan kotak detail sidik jari host key.
* **Lebar Mobile (< 48rem)**:
  * Layout form ditumpuk secara vertikal (Label di atas input). Input lebar penuh.
  * Checklist verifikasi ditumpuk di atas kotak konfirmasi sidik jari.
  * Target sentuh tombol minimal `44px x 44px`. Padding form menyusut menjadi `1rem` tanpa scroll horizontal.

## 6. Aksesibilitas (a11y)

* **Bahasa**: Dokumen menggunakan atribut `lang="id"`.
* **Semantik Form**: Setiap kontrol input memiliki elemen `<label>` yang merujuk pada `id` input yang benar menggunakan atribut `for`.
* **Keyboard Focus**: Fokus berpindah secara berurutan sesuai alur form. Area textarea kunci privat dapat dilewati menggunakan tombol Tab dengan benar. Indikator fokus outline terlihat tebal dengan warna `--color-link` (`#6cf`).
* **Visual Status**: Keberhasilan, peringatan, dan kegagalan checklist ditandai dengan simbol teks pendukung selain warna:
  * Sukses: simbol `[o]` berwarna hijau
  * Proses: simbol `[*]` berwarna kuning
  * Gagal: simbol `[x]` berwarna merah
  * Belum berjalan: simbol `[ ]` berwarna abu-abu
* **Konfirmasi Destruktif / Peringatan**: Kotak peringatan sidik jari berubah (TOFU) menggunakan kontras tinggi dan memaksa fokus keyboard pertama kali ke teks penjelas sebelum tombol "Lanjutkan".

## 7. Copywriting

| Kunci / Elemen | Teks Final |
| :--- | :--- |
| Judul Form Langkah 1 | Tambah Server Baru (Langkah 1/3) |
| Label Nama | Nama Server |
| Label Host | Alamat Host / IP |
| Hint Host | Contoh: vps-sg-1.domain.com atau 128.199.12.34 (Tanpa skema URL atau port) |
| Label Port | Port SSH |
| Label User | Pengguna SSH |
| Label Kunci | Kunci Privat SSH |
| Hint Kunci | Harus berupa kunci format OpenSSH (dimulai dengan -----BEGIN OPENSSH PRIVATE KEY-----). |
| Tombol Submit Langkah 1 | Lanjutkan ke Verifikasi |
| Judul Verifikasi Langkah 2 | Verifikasi Server (Langkah 2/3) |
| Teks Menunggu Konfirmasi Hostkey | Sidik jari host key belum terdaftar di aplikasi. Konfirmasi sidik jari berikut untuk melanjutkan: |
| Tombol Terima Hostkey | Ya, Terima & Simpan |
| Judul Form Registry Langkah 3 | Tautkan Registry (Langkah 3/3 - Opsional) |
| Opsi Registry Baru | Baru... |
| Label Host Registry | Host Registry |
| Label Username Registry | Username |
| Label Token Registry | Token Akses / Kata Sandi |
| Tombol Submit Langkah 3 | Tautkan Registry |
| Tombol Lewati Langkah 3 | Lewati & Selesai |

## 8. Catatan Implementasi untuk Frontend

* Fungsi render yang harus disediakan di `src/web/server_add.rs`:
  * `render_server_baru(csrf_token: &str, error: Option<&str>) -> Markup`
  * `render_verifikasi(server: &ServerRingkas, langkah: &[LangkahVerifikasi], csrf_token: &str) -> Markup`
  * `render_verifikasi_fragmen(langkah: &[LangkahVerifikasi]) -> Markup`
  * `render_registry_form(server: &ServerRingkas, csrf_token: &str, error: Option<&str>) -> Markup`
* Alur Langkah 2 Verifikasi wajib menggunakan ekstensi HTMX SSE (`hx-ext="sse"`) untuk mendengarkan `/events/verifikasi/{job_id}`:
  ```html
  <div hx-ext="sse" sse-connect="/events/verifikasi/JOB_ID" sse-swap="message" hx-target="#checklist-container">
    <div id="checklist-container">
      <!-- Fragmen dirender awal dari backend via render_verifikasi -->
    </div>
  </div>
  ```
* Validasi input client-side menggunakan atribut HTML5 bawaan: `required`, `pattern` untuk host, `min="1" max="65535"` untuk port.
* Hindari menyimpan state kunci privat atau token registry di `sessionStorage` atau `localStorage` browser. Seluruh transmisi form wajib langsung dipos ke server dan dibersihkan dari memori peramban pasca pengiriman.
