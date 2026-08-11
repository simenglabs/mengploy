# Spesifikasi UI: Notifikasi

Halaman pengaturan menampilkan status webhook, nama, event aktif, status delivery
terakhir, dan waktu terakhir. URL dimasking dan signing secret tidak pernah
ditampilkan ulang. Rotasi secret menerima secret baru melalui form.

Event minimum: `deployment.failed`, `deployment.recovered`,
`reconciliation.drift_detected`, dan `reconciliation.drift_resolved`.
Delivery berjalan melalui queue, bukan request deploy. State mencakup kosong,
nonaktif, queued, delivered, retrying, failed, dan error konfigurasi.
Payload hanya metadata opaque, digest, kategori generik, dan timestamp; tidak
boleh memuat environment, credential, token, path, stderr, atau log.
