# Spesifikasi UI: Rollback

Rollback selalu membuat deployment baru; histori deployment target tidak diubah.
Dialog menampilkan digest live saat ini dan digest target, commit, status image,
dan peringatan bahwa operasi dapat mengganti container aplikasi.

Environment default adalah snapshot deployment target. Operator dapat memilih
snapshot target, environment terbaru, atau versi historis milik app yang sama.
Diff secret hanya menampilkan `(secret diubah)`, `(secret diisi)`, `(secret menjadi
kosong)`, atau `dihapus`; plaintext, panjang, hash, prefix, dan suffix dilarang.

State wajib: kosong, image tidak tersedia, environment tidak tersedia, lock aktif,
loading, sukses queued, gagal, dan unknown. Tombol konfirmasi memakai CSRF,
memiliki label accessible, dan tidak menyatakan rollback selesai sampai status
deployment menjadi `live`. Tidak ada retry otomatis atau perbaikan otomatis.
