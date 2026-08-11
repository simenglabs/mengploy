# Spesifikasi Desain: Environment

**Status:** spesifikasi formal Fase 4  
**Ruang lingkup:** tab `Environment` pada `/apps/{id}/env`, termasuk pengubahan
variabel, konfirmasi perubahan, dan hasil deploy.  
**Sumber implementasi:** `src/web/env.rs` dan `src/routes/apps.rs`.  
**Bahasa UI:** Bahasa Indonesia, kecuali nama `key` dan istilah teknis yang memang
muncul sebagai nilai data.

Dokumen ini memformalkan tiga simplifikasi yang sebelumnya hanya tersirat dalam
implementasi: bar perubahan, diff, dan cara menetapkan value kosong. Spesifikasi
ini mempertahankan batasan proyek: tidak mengandalkan JavaScript kustom; form
HTML biasa boleh dipakai dan HTMX/xterm bukan prasyarat tab ini.

## 1. Tujuan dan prinsip

Tab ini memberi operator cara yang aman untuk menambah, mengganti, dan menghapus
environment variable aplikasi. Setiap penyimpanan membuat snapshot environment
baru dan, bila aplikasi sedang live, membuat deployment baru dengan image digest
yang sama. Menyimpan environment **berarti me-restart/mengganti container aplikasi**,
bukan sekadar menyimpan preferensi di control plane.

Prinsip yang mengikat:

1. Secret tidak pernah ditampilkan kembali sebagai plaintext di HTML, response,
   diff, pesan sukses, log, atau atribut tersembunyi.
2. Nilai non-secret boleh ditampilkan dalam input agar dapat diedit langsung.
3. Nilai pada form existing yang kosong berarti **pertahankan nilai lama**, bukan
   set ke string kosong. Cara set string kosong dijelaskan di §5.
4. Menghapus key adalah tindakan berbeda dari mengosongkan value.
5. Tidak ada informasi penting yang hanya disampaikan lewat warna.
6. Nilai dengan `\n` atau `\r` ditolak; UI tidak meng-escape atau mengubahnya diam-diam.

## 2. Struktur halaman

Urutan halaman dari atas ke bawah:

1. Judul `App: {nama app}` dan navigasi tab, dengan tab `Environment` aktif.
2. Alert hasil operasi atau error, bila ada.
3. **Bar perubahan dan konsekuensi deploy** (§3), selalu terlihat sebelum tabel.
4. Tabel variabel existing: `Key`, `Value`, `Tindakan`.
5. Tabel `Tambah Variabel` dengan lima slot baris kosong (`new_key_0` sampai
   `new_key_4`), mengikuti `ENV_NEW_ROW_SLOTS`.
6. Tombol utama `Simpan & Deploy`.

Key dirender sebagai teks/code, bukan sebagai input yang dapat diedit. Perubahan
nama key dilakukan dengan menghapus key lama lalu menambah key baru, supaya
identitas perubahan dan diff tidak ambigu.

## 3. Bar perubahan (keputusan formal)

### 3.1 Perilaku tanpa JavaScript

Bar perubahan **selalu dirender** pada halaman, termasuk ketika belum ada
perubahan lokal. Ini menggantikan penghitung dinamis yang sebelumnya tidak formal.
Alasannya: form HTML biasa tidak dapat menghitung perubahan saat pengguna
mengetik, dan bar keselamatan tidak boleh bergantung pada JavaScript.

Bar memiliki gaya visual alert yang kontras dan teks wajib:

> **Perhatian:** menyimpan environment akan me-restart aplikasi dan memicu deploy
> baru dengan image yang sama. Periksa perubahan sebelum menyimpan.

Bar harus berada di dalam urutan fokus normal, tidak boleh hanya `position: fixed`
yang menutupi isi, dan tetap terlihat saat halaman digulir. Implementasi boleh
menggunakan `position: sticky` pada viewport, tetapi salinan teks konsekuensi
harus tetap ada di alur konten agar tersedia untuk pembaca layar dan mode cetak.

### 3.2 Tombol dan status bar

- Tombol `Simpan & Deploy` adalah satu-satunya CTA utama untuk menerapkan seluruh
  perubahan form. Tombol diberi `type="submit"` dan berada di akhir form.
- Saat belum ada perubahan, submit tetap sah: server tidak membuat snapshot atau
  deployment baru dan hanya merender ulang halaman. UI tidak boleh menyatakan
  bahwa deploy terjadi.
- Saat ada perubahan yang disubmit, bar pada response berikutnya berubah menjadi
  alert hasil, bukan klaim bahwa deploy sudah selesai.
- Jangan gunakan teks `N variabel berubah` sebagai fakta sebelum server melakukan
  perhitungan. Bila pada masa depan ditambahkan preview dinamis, hitungan hanya
  boleh ditampilkan sebagai `Perkiraan: N perubahan belum disimpan` dan harus
  diperbarui untuk tambah, edit, kosongkan, serta hapus.

Dengan demikian bar mustahil terlewatkan secara semantik (selalu ada dan menyebut
restart/deploy) meskipun penghitung perubahan live belum tersedia.

## 4. Model perubahan dan diff

### 4.1 Sumber pembanding

Diff membandingkan **draft yang akan disubmit** dengan environment aktif yang
dimuat ketika halaman dibuka. Diff bukan perbandingan dengan versi historis
sembarang dan bukan diff image. Jika halaman dimuat ulang setelah penyimpanan,
state terbaru menjadi baseline baru.

Karena tidak ada JavaScript kustom, diff formal disajikan sebagai **ringkasan
server-side pada response hasil submit**. Pada fase lanjutan yang menambahkan
preview, ringkasan yang sama dapat dirender sebelum submit tanpa mengubah aturan
atau labelnya.

### 4.2 Kategori diff

Ringkasan memakai heading `Perubahan environment yang diterapkan` (atau
`Perubahan environment yang diminta` bila operasi gagal) dan daftar berlabel:

| Kategori | Format UI | Makna |
|---|---|---|
| Ditambahkan | `+ KEY` — `nilai baru` | Key tidak ada pada baseline dan akan dibuat. |
| Diubah | `~ KEY` — `nilai lama` → `nilai baru` | Key ada dan value berbeda. |
| Dikosongkan | `~ KEY` — `nilai lama` → `(kosong)` | Key tetap ada dengan value string kosong. |
| Dihapus | `− KEY` — `dihapus` | Key tidak lagi ada pada snapshot baru. |
| Tidak berubah | Tidak ditampilkan dalam daftar perubahan | Tidak menghasilkan operasi atau deploy baru. |

Jumlah pada heading adalah jumlah **key yang berubah**, bukan jumlah karakter,
field HTML, atau jumlah operasi internal. Satu key yang diedit dan sekaligus
ditandai hapus hanya masuk kategori `Dihapus`.

Jika operasi menghasilkan deployment, ringkasan wajib menyebut:

> `Deployment baru dijadwalkan dengan image yang sama. Environment akan aktif
> setelah container baru sehat.`

Jika aplikasi belum pernah dideploy:

> `Environment disimpan. App ini belum pernah dideploy, jadi belum ada deployment
> untuk diterapkan.`

### 4.3 Diff secret tanpa kebocoran

Untuk key `secret`, **nilai lama dan nilai baru tidak pernah ditulis**. Formatnya:

- ditambahkan: `+ API_TOKEN` — `(secret diisi)`;
- diubah/diganti: `~ API_TOKEN` — `(secret diubah)`;
- dikosongkan: `~ API_TOKEN` — `(secret menjadi kosong)`;
- dihapus: `− API_TOKEN` — `dihapus`.

Jangan menampilkan panjang value, jumlah karakter berubah, checksum, prefix,
suffix, preview, `value` attribute, tooltip, `title`, `aria-label`, atau teks
alternatif yang dapat membantu menebak secret. Label `secret` dan status
perubahannya cukup untuk memahami dampak operasi.

Pada tabel sebelum submit, field secret selalu berupa input kosong dengan
placeholder:

> `•••••••• (kosongkan untuk tidak mengganti)`

Masker bukan value yang dikirim browser. Tidak ada tombol `Tampilkan`.
Penggantian secret dilakukan dengan mengetik nilai baru lalu menyimpan.

## 5. Cara mengosongkan value (keputusan formal)

Ada tiga tindakan yang dibedakan secara eksplisit:

### 5.1 Pertahankan value lama

Biarkan input `Value` existing kosong dan jangan centang `Hapus`. Server
mengartikan field kosong sebagai `tidak diubah`. Ini berlaku untuk non-secret dan
secret, sehingga reload tidak menghapus credential secara tidak sengaja.

Helper text wajib dekat field, atau menjadi deskripsi yang dihubungkan dengan
`aria-describedby`:

> `Kosongkan untuk mempertahankan nilai saat ini.`

Untuk secret, gunakan placeholder yang juga menyatakan aturan ini.

### 5.2 Set value menjadi string kosong

Untuk mengosongkan value tetapi **mempertahankan key**, operator mencentang
kontrol tambahan `Set value menjadi kosong` pada baris tersebut, lalu menyimpan.
Kontrol ini adalah sentinel eksplisit; input Value dibiarkan kosong. Saat kontrol
aktif, aturan `tidak diubah` tidak berlaku dan diff menampilkan `(kosong)`.

Kontrol tidak boleh aktif bersamaan dengan `Hapus`. Jika keduanya dikirim, server
menolak submit dengan error yang menyebut key dan meminta operator memilih satu
tindakan.

Catatan implementasi: bila versi HTML saat ini belum memiliki checkbox sentinel,
UI harus setidaknya menyediakan alur setara yang eksplisit sebelum fitur
string-kosong dinyatakan lengkap. Jangan memakai placeholder atau spasi sebagai
sentinel, dan jangan menyuruh operator menghapus lalu membuat ulang key karena
itu mengubah makna diff serta metadata secret.

### 5.3 Menghapus key

Centang `Hapus` untuk menghapus key beserta value-nya dari environment. Konfirmasi
teks pada label harus berbunyi `Hapus KEY dari environment`, bukan sekadar
`hapus`, agar konteks tersedia bagi pembaca layar. Hapus mengalahkan isi input
Value hanya jika tidak ada konflik sentinel; konflik wajib ditolak, bukan dipilih
secara diam-diam.

## 6. State UI

### 6.1 Loading / default

- Saat GET memuat data, tampilkan judul, navigasi, bar konsekuensi, lalu tabel.
- Jika data belum tersedia karena proses loading server-side, gunakan teks
  `Memuat environment…`; jangan tampilkan tabel kosong seolah data berhasil
  dimuat.
- Tombol submit tidak perlu spinner client-side. Setelah diklik, browser menunggu
  response server dan tidak boleh mengirim ulang otomatis.

### 6.2 Empty

Jika tidak ada variable existing, tampilkan:

> `Belum ada environment variable.`

Tetap tampilkan bar konsekuensi, tabel lima slot `Tambah Variabel`, dan CTA
`Simpan & Deploy`. Tambahkan penjelasan bahwa baris baru yang disimpan akan
menjadi environment aplikasi. Empty bukan error.

### 6.3 Success

Alert sukses harus spesifik terhadap hasil:

- live dan deploy dibuat: `Environment disimpan — deploy baru dengan image yang
  sama sedang berjalan.`
- belum pernah live: pesan pada §4.2.
- submit tanpa perubahan: `Tidak ada perubahan untuk disimpan.`

Success tidak berarti container baru sudah sehat. Link ke tab Deployments boleh
disediakan, tetapi jangan menyebut aplikasi sudah restart selesai sebelum status
deployment menyatakan live.

### 6.4 Error

Error ditampilkan di atas bar sebagai alert dengan teks jelas dan tetap
mempertahankan input non-secret yang dikirim agar operator tidak kehilangan kerja.
Jangan memantulkan nilai secret di pesan error.

Kasus minimum:

- CSRF/sesi: `Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan
  coba lagi.`
- key duplikat: sebut key yang duplikat dan minta edit baris existing.
- newline: `Nilai untuk KEY tidak boleh mengandung baris baru.`
- konflik `Hapus` + `Set value menjadi kosong`: minta pilih satu tindakan.
- deploy sedang berjalan: `Environment disimpan, tetapi deploy baru belum dapat
  dijadwalkan karena deploy lain sedang berjalan. Coba simpan lagi setelah selesai.`
- kegagalan deploy setelah queue: tampilkan bahwa environment tersimpan dan
  deployment gagal/unknown; arahkan ke tab Deployments. Jangan menyimpulkan env
  baru aktif.
- app tidak ditemukan: halaman 404 standar, bukan error validasi form.

### 6.5 Deployment gagal atau restart belum selesai

Environment version adalah snapshot yang terikat pada deployment tertentu. Jika
container baru gagal, container lama tetap menjadi sumber layanan dan UI harus
menyatakan:

> `Perubahan tersimpan, tetapi belum aktif pada aplikasi yang sedang melayani.
> Periksa detail deployment.`

Tidak ada auto-retry atau tombol perbaikan otomatis pada tab ini. Rollback dengan
pilihan environment adalah ruang lingkup Fase 5.

## 7. Konsekuensi restart dan deploy

Sebelum CTA, bar wajib menjelaskan tiga fakta:

1. menyimpan membuat snapshot environment baru;
2. aplikasi di-restart melalui deployment container baru;
3. image digest tetap sama; yang berubah hanya environment.

Untuk aplikasi live, deployment memakai digest live saat penyimpanan. Environment
baru dianggap aktif hanya setelah container baru sehat dan pergantian selesai.
Untuk aplikasi yang belum pernah dideploy, value disimpan tetapi tidak ada restart.

Tulis hanya metadata operasional ke log/status (jumlah key, versi, status deploy),
tidak pernah plaintext value atau isi secret. Secret pada server target dapat
terlihat oleh pemilik akses Docker/server sesuai batas kepercayaan Docker yang
telah dipilih pada Fase 4; UI control plane tetap wajib masking total.

## 8. Aksesibilitas

- Gunakan heading berurutan: `h1` judul app, `h2` untuk `Tambah Variabel` dan
  ringkasan diff.
- Alert memakai `role="status"` untuk sukses dan `role="alert"` untuk error,
  atau mekanisme server-rendered setara yang dibaca saat halaman dimuat.
- Setiap input memiliki label tekstual yang terhubung lewat `for`/`id`; jangan
  mengandalkan placeholder sebagai label.
- Setiap checkbox menyebut key: `Hapus DB_PASSWORD` dan `Set value DB_PASSWORD
  menjadi kosong`.
- Kolom tabel memakai `th scope="col"`; hubungan label-value harus tetap jelas
  ketika dibaca linear oleh screen reader.
- Bar dan error memiliki kontras memadai, teks terlihat, dan tidak menggunakan
  warna sebagai satu-satunya penanda.
- Fokus keyboard harus mencapai seluruh input, checkbox, tombol, dan link diff;
  sticky bar tidak boleh menutup indikator fokus.
- Jangan menaruh secret di DOM tersembunyi, `aria-*`, URL, history browser,
  `data-*`, atau clipboard otomatis.
- Untuk key sangat panjang atau value non-secret panjang, izinkan pembacaan dan
  pengeditan tanpa pemotongan diam-diam; gunakan wrapping/scroll horizontal yang
  dapat diakses dan pertahankan nama key sebagai konteks.

## 9. Kriteria penerimaan UI

1. Bar konsekuensi selalu terlihat dan menyebut restart, deployment, serta digest
   yang sama.
2. Setiap perubahan dapat diklasifikasikan sebagai tambah, ubah, kosongkan, atau
   hapus pada diff server-side.
3. Diff secret hanya memakai label status `(secret diisi/diubah/menjadi kosong)`;
   tidak ada nilai, panjang, hash, prefix, atau suffix.
4. Operator dapat membedakan pertahankan value lama, set string kosong, dan hapus
   key tanpa memakai spasi atau trik placeholder.
5. Empty, loading, success, error, deploy tertunda, dan deploy gagal memiliki
   pesan eksplisit yang tidak melebih-lebihkan status aplikasi.
6. Seluruh tindakan dapat dilakukan dengan keyboard dan dibaca tanpa informasi
   berbasis warna saja.
7. Form existing tetap menggunakan nama field kontrak saat ini (`value__KEY`,
   `delete__KEY`) dan slot tambah (`new_key_i`, `new_value_i`, `new_secret_i`);
   sentinel string kosong, bila ditambahkan, harus memiliki nama field baru yang
   terdokumentasi dan divalidasi server-side.
