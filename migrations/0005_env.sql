-- Migrasi 0005: Fase 4 — pengelolaan environment.
--
-- Dua tabel baru:
--   env_vars     — state env "sedang diedit" saat ini, satu baris per key
--   env_versions — snapshot beku env pada satu titik waktu, dirujuk
--                  deployments.env_version_id (kolom itu sudah ada sejak
--                  migrations/0003_deploy.sql, sengaja NULL sampai sekarang)
--
-- INVARIANT §3 NO.7/NO.8: value_encrypted dan snapshot_encrypted adalah
-- ciphertext armor `age` (pola sama servers.ssh_key_encrypted,
-- registries.token_encrypted) — plaintext TIDAK PERNAH masuk kolom apa pun
-- di sini, dan kunci dekripsinya (key.age) tetap di luar database
-- sepenuhnya, tidak berubah dari fase-fase sebelumnya.

-- ============================================================
-- env_vars: state env "sedang diedit" — SATU baris per (app_id, key).
-- key            = nama variabel, unik per app (UPPER_SNAKE_CASE by
--                  konvensi, tidak ditegakkan CHECK — validasi di backend)
-- value_encrypted = ciphertext armor age dari plaintext value
-- is_secret      = 0/1, ditentukan SEKALI saat baris dibuat (tidak bisa
--                  diubah lewat edit — lihat docs/plan.md "Desain teknis"):
--                  menentukan apakah UI menopengi value ini
-- updated_at     = epoch detik terakhir value ini berubah
-- ============================================================
CREATE TABLE env_vars (
    id              TEXT    PRIMARY KEY,
    app_id          TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    key             TEXT    NOT NULL,
    value_encrypted TEXT    NOT NULL,
    is_secret       INTEGER NOT NULL DEFAULT 0 CHECK (is_secret IN (0, 1)),
    updated_at      INTEGER NOT NULL,
    UNIQUE (app_id, key)
);

CREATE INDEX idx_env_vars_app_id ON env_vars (app_id);

-- ============================================================
-- env_versions: snapshot BEKU env_vars pada satu titik waktu — dibuat
-- SEKALI tiap kali env disimpan (bukan tiap kali satu key berubah), berisi
-- SELURUH key milik app itu (bukan cuma yang berubah), supaya satu
-- deployment bisa merujuk satu env_version_id dan tahu PERSIS env lengkap
-- apa yang jalan, terlepas dari perubahan env_vars sesudahnya.
-- version           = nomor urut per app, mulai dari 1
-- snapshot_encrypted = ciphertext armor age dari JSON {"KEY":"value",...}
--                      seluruh env app ini pada saat snapshot dibuat
-- note              = opsional, bisa NULL (mis. "redeploy dari CI", diisi
--                      backend, bukan input pengguna bebas)
-- ============================================================
CREATE TABLE env_versions (
    id                  TEXT    PRIMARY KEY,
    app_id              TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    version             INTEGER NOT NULL,
    snapshot_encrypted  TEXT    NOT NULL,
    note                TEXT,
    created_at          INTEGER NOT NULL,
    UNIQUE (app_id, version)
);

-- Indeks: cari versi TERBARU per app (MAX(version) WHERE app_id = ?) dan
-- lookup by id dari deployments.env_version_id (PK sudah cukup untuk itu).
CREATE INDEX idx_env_versions_app_id ON env_versions (app_id);
