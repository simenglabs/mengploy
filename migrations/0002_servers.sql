-- Migrasi 0002: Fase 1 — server, registry, dan verifikasi konektivitas.
--
-- Tiga tabel baru:
--   servers          — satu baris per VPS yang dikelola
--   registries       — satu baris per Docker registry (ghcr.io, dst.)
--   server_registries — join server ↔ registry (many-to-many)
--
-- Kolom terenkripsi (ssh_key_encrypted, token_encrypted) menyimpan CIPHERTEXT
-- hasil enkripsi age, BUKAN kunci enkripsi itu sendiri. Kunci age selalu di
-- file terpisah mode 0600 di luar db (invariant PRD §3 nomor 8).
--
-- Kolom metrik, aplikasi, deployment, dan env TIDAK ada di sini — itu Fase 2/4/6.
-- PRD §4 melarang mendesain skema untuk fase yang belum tiba.

-- ============================================================
-- servers: satu baris per VPS yang terdaftar di sistem.
-- id                   = token acak buram (RNG kripto-aman, bukan auto-increment)
-- name                 = nama tampilan (user-facing)
-- host                 = hostname atau IP, tanpa skema URL
-- port                 = port SSH, default 22
-- ssh_user             = nama pengguna SSH
-- ssh_key_encrypted    = private key OpenSSH terenkripsi age (ciphertext armor)
-- status               = state machine: pending → verifying → online / unreachable
-- last_seen_at         = epoch detik terakhir server berhasil di-poll (NULL = belum pernah)
-- docker_version       = versi Docker terdeteksi (NULL = belum/sudah tidak terdeteksi)
-- os_info              = ringkasan OS dari remote (NULL = belum terbaca)
-- host_key_fingerprint = fingerprint host key server target (NULL = belum dikonfirmasi TOFU)
-- consecutive_failures = jumlah kegagalan polling berturut-turut (reset ke 0 saat sukses)
-- next_poll_at         = epoch detik kapan worker boleh mem-poll lagi (0 = segera)
-- last_error_kind      = kategori kegagalan terakhir (NULL = tidak ada error)
-- last_error_message   = pesan pendek yang sudah dipetakan (NULL = tidak ada error, MAX 500 karakter
--                        — BUKAN stderr mentah, invariant PRD §3 nomor 9)
-- created_at           = epoch detik saat baris dibuat
-- updated_at           = epoch detik saat baris terakhir diubah
-- ============================================================
CREATE TABLE servers (
    id                    TEXT    PRIMARY KEY,
    name                  TEXT    NOT NULL,
    host                  TEXT    NOT NULL,
    port                  INTEGER NOT NULL DEFAULT 22,
    ssh_user              TEXT    NOT NULL,
    ssh_key_encrypted     TEXT    NOT NULL,
    status                TEXT    NOT NULL DEFAULT 'pending'
                              CHECK (status IN ('pending', 'verifying', 'online', 'unreachable')),
    last_seen_at          INTEGER,
    docker_version        TEXT,
    os_info               TEXT,
    host_key_fingerprint  TEXT,
    consecutive_failures  INTEGER NOT NULL DEFAULT 0,
    next_poll_at          INTEGER NOT NULL DEFAULT 0,
    last_error_kind       TEXT,
    last_error_message    TEXT    CHECK (last_error_message IS NULL OR length(last_error_message) <= 500),
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);

-- Indeks: worker SELECT server dengan next_poll_at <= now ORDER BY next_poll_at
-- LIMIT N. Tanpa indeks ini, worker melakukan full scan setiap 30 detik.
CREATE INDEX idx_servers_next_poll_at ON servers (next_poll_at);

-- Indeks: fleet overview dan fleet strip SELECT ... WHERE status = ?.
-- Tanpa indeks ini, setiap render halaman melakukan full scan.
CREATE INDEX idx_servers_status ON servers (status);

-- ============================================================
-- registries: satu baris per Docker registry (ghcr.io, Docker Hub, dst.).
-- id              = token acak buram
-- host            = hostname registry (tanpa skema URL)
-- username        = nama pengguna registry
-- token_encrypted = token/password registry terenkripsi age (ciphertext armor)
-- UNIQUE(host, username) = tidak ada duplikat diam-diam
-- ============================================================
CREATE TABLE registries (
    id              TEXT PRIMARY KEY,
    host            TEXT NOT NULL,
    username        TEXT NOT NULL,
    token_encrypted TEXT NOT NULL,
    UNIQUE (host, username)
);

-- ============================================================
-- server_registries: join server ↔ registry (many-to-many).
-- PRIMARY KEY gabungan (server_id, registry_id).
-- FOREIGN KEY ON DELETE CASCADE: kalau server atau registry dihapus,
-- baris join ikut terhapus otomatis.
-- last_login_at = epoch detik terakhir docker login berhasil di server ini
-- untuk registry ini (NULL = belum pernah login).
-- ============================================================
CREATE TABLE server_registries (
    server_id    TEXT    NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    registry_id  TEXT    NOT NULL REFERENCES registries (id) ON DELETE CASCADE,
    last_login_at INTEGER,
    PRIMARY KEY (server_id, registry_id)
);
