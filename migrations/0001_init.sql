-- Migrasi 0001: Fondasi Fase 0
-- Tabel settings (key-value pengguna tunggal), sessions (autentikasi cookie).
-- 
-- Catatan pragma:
--   journal_mode=WAL dijalankan di sini karena bersifat database-level dan persisten.
--   busy_timeout, foreign_keys=ON, synchronous=NORMAL bersifat per-koneksi —
--   backend WAJIB mengaturnya saat membuka pool di src/db.rs, bukan di sini.
--   Pragma di bawah hanya berlaku selama migrasi berjalan, tidak untuk koneksi
--   aplikasi di kemudian hari.

PRAGMA journal_mode = WAL;

-- ============================================================
-- settings: key-value persisten untuk konfigurasi pengguna tunggal.
-- Menyimpan password_hash (Argon2), penanda setup, dan konfigurasi umum.
-- Backend menulis password_hash saat startup kalau belum ada (Q5).
-- ============================================================
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- ============================================================
-- sessions: baris per sesi aktif.
-- id         = token sesi acak buram (RNG kripto-aman, bukan auto-increment)
-- created_at = epoch detik saat sesi dibuat
-- expires_at = epoch detik saat sesi kedaluwarsa (created_at + 30 hari,
--              dihitung backend; Q6: absolute expiry, tanpa idle timeout)
-- csrf_token = token CSRF terikat sesi ini (dikirim di form, divalidasi POST)
-- ============================================================
CREATE TABLE sessions (
    id          TEXT    PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    csrf_token  TEXT    NOT NULL
);

CREATE INDEX idx_sessions_expires_at ON sessions (expires_at);
