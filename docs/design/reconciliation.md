# Spesifikasi UI: Rekonsiliasi

Rekonsiliasi membaca label container dan state Docker lalu mencatat finding.
Finding tidak pernah menghentikan, menghapus, membuat, atau mengadopsi container.
Container manual tidak otomatis menjadi deployment `live`.

Banner menyebut kategori, waktu observasi terakhir, deployment/container metadata
aman, dan tindakan manual yang disarankan. Banner tidak menyediakan tombol
perbaikan otomatis. State harus mencakup tidak ada finding, finding aktif,
finding pulih, server tidak terjangkau, dan deployment `unknown`.

Kategori yang stabil: `live_container_missing`, `live_digest_mismatch`,
`live_container_id_mismatch`, `multiple_live_containers`, `orphan_platform_container`,
`deployment_without_container`, `server_unreachable`, dan `image_missing`.
Payload UI tidak boleh berisi secret, env, path filesystem, atau output Docker mentah.
