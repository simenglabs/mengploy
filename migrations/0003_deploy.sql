-- Migrasi 0003: Fase 2 — loop deploy.
--
-- Lima tabel baru:
--   apps           — satu baris per aplikasi yang di-deploy ke satu server
--   domains        — domain publik yang diarahkan ke satu app (opsional, banyak per app)
--   deployments    — riwayat + state machine setiap percobaan deploy
--   deploy_tokens  — kredensial POST /api/v1/deploy, satu per app (bukan global)
--   jobs           — antrean kerja worker deploy (tabel SQLite, tanpa crate eksternal)
--
-- Tidak ada kolom secret plaintext atau kunci enkripsi di sini (invariant §3 no.8).
-- Kolom metrik dan env var TIDAK ada — itu Fase 4/6. PRD §4 melarang mendesain
-- skema untuk fase yang belum tiba, KECUALI kolom yang eksplisit ditandai murah
-- sekarang/mahal-kalau-diretrofit di docs/prd.md §8 (deployments.env_version_id
-- di bawah adalah satu-satunya kolom semacam itu di migrasi ini).

-- ============================================================
-- apps: satu baris per aplikasi (satu app = satu server, Fase 2 belum
-- mendukung multi-server per app — bukan non-goal permanen, cuma belum perlu).
-- id                   = token acak buram
-- server_id            = FK ke servers.id — app WAJIB terikat ke satu server
-- name                 = nama aplikasi, dipakai sebagai path POST /api/v1/deploy
--                        dan sebagai bagian nama container (--name {app}-{deployment_id})
-- health_path          = path HTTP untuk health check (mis. /healthz)
-- health_grace_secs    = detik toleransi sebelum health check mulai dianggap gagal
-- port                 = port yang didengarkan container di dalam network platform
-- restart_policy       = kebijakan restart docker (default 'unless-stopped' — invariant §5 no.5)
-- lock_token           = token lock deploy aktif (NULL = tidak terkunci)
-- lock_expires_at      = kedaluwarsa lock (invariant §3 no.12 — WAJIB ada kedaluwarsa)
-- created_at/updated_at = epoch detik
-- ============================================================
CREATE TABLE apps (
    id                TEXT    PRIMARY KEY,
    server_id         TEXT    NOT NULL REFERENCES servers (id),
    name              TEXT    NOT NULL UNIQUE,
    health_path       TEXT    NOT NULL DEFAULT '/',
    health_grace_secs INTEGER NOT NULL DEFAULT 30,
    port              INTEGER NOT NULL,
    restart_policy    TEXT    NOT NULL DEFAULT 'unless-stopped',
    lock_token        TEXT,
    lock_expires_at   INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

-- Indeks: worker klaim lock cek app by id+lock_expires_at tiap deploy; fleet
-- app view SELECT ... WHERE server_id = ?.
CREATE INDEX idx_apps_server_id ON apps (server_id);

-- ============================================================
-- domains: domain publik yang diarahkan ke satu app (banyak domain per app
-- mungkin, mis. domain utama + alias). tls_enabled dibaca label Traefik saat
-- container dijalankan (Fase 2 belum mengelola provisioning sertifikat TLS
-- itu sendiri — tanggung jawab Traefik/ACME di server target).
-- ============================================================
CREATE TABLE domains (
    id          TEXT    PRIMARY KEY,
    app_id      TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    host        TEXT    NOT NULL UNIQUE,
    tls_enabled INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_domains_app_id ON domains (app_id);

-- ============================================================
-- deployments: satu baris per percobaan deploy — riwayat lengkap + state
-- machine aktif.
-- status        = queued -> pulling -> starting -> checking -> live
--                 cabang: failed | cancelled | unknown
-- stage         = label tahap yang lebih rinci dari status untuk UI (mis.
--                 'menarik image', 'menjalankan container') — status adalah
--                 sumber kebenaran mesin, stage murni tampilan.
-- trigger_source = 'api' (POST /api/v1/deploy) — kolom disiapkan untuk
--                 sumber lain di masa depan (mis. UI manual), TIDAK dipakai
--                 selain 'api' di Fase 2.
-- heartbeat_at  = worker meng-update ini selama deployment aktif; heartbeat
--                 basi saat boot -> status disulap 'unknown', BUKAN ditebak.
-- container_id  = id container docker konkret (bukan cuma label) — dipakai
--                 untuk drain + tangkap log SETELAH container mungkin sudah
--                 exited, saat label saja tidak lagi cukup (docs/prd.md §8).
-- error_kind    = kategori pendek (container_exited|health_non_2xx|
--                 health_no_response|pull_gagal|...), BUKAN log mentah.
-- error_detail  = pesan pendek yang sudah dipetakan, MAX 500 karakter —
--                 sama pola servers.last_error_message (invariant §3 no.9,
--                 log runtime TIDAK PERNAH masuk sini).
-- env_version_id = kolom murah-sekarang-mahal-diretrofit (docs/prd.md §8) —
--                 TIDAK dipakai sampai Fase 4 (env_versions belum ada),
--                 sengaja NULL selamanya di Fase 2.
-- ============================================================
CREATE TABLE deployments (
    id             TEXT    PRIMARY KEY,
    app_id         TEXT    NOT NULL REFERENCES apps (id),
    commit_sha     TEXT    NOT NULL,
    git_ref        TEXT,
    image_digest   TEXT    NOT NULL,
    status         TEXT    NOT NULL DEFAULT 'queued'
                       CHECK (status IN ('queued', 'pulling', 'starting', 'checking',
                                         'live', 'failed', 'cancelled', 'unknown')),
    stage          TEXT,
    trigger_source TEXT    NOT NULL DEFAULT 'api',
    container_id   TEXT,
    env_version_id TEXT,
    heartbeat_at   INTEGER,
    started_at     INTEGER,
    finished_at    INTEGER,
    error_kind     TEXT,
    error_detail   TEXT    CHECK (error_detail IS NULL OR length(error_detail) <= 500),
    created_at     INTEGER NOT NULL
);

-- Indeks: riwayat per app terurut waktu (tab deployments UI); worker boot
-- mencari deployment aktif dengan heartbeat basi.
CREATE INDEX idx_deployments_app_id_created_at ON deployments (app_id, created_at);
CREATE INDEX idx_deployments_status ON deployments (status);

-- ============================================================
-- deploy_tokens: kredensial POST /api/v1/deploy, satu per app (invariant:
-- token per aplikasi, BUKAN token global — docs/prd.md §4 Security).
-- token_hash = hash argon2 (pola sama settings.password_hash), BUKAN
-- ciphertext age — token deploy hanya perlu diverifikasi, tidak pernah
-- didekripsi kembali.
-- ============================================================
CREATE TABLE deploy_tokens (
    id           TEXT    PRIMARY KEY,
    app_id       TEXT    NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE INDEX idx_deploy_tokens_app_id ON deploy_tokens (app_id);

-- ============================================================
-- jobs: antrean kerja worker deploy. Tabel SQLite polos, tanpa crate queue
-- eksternal (CLAUDE.md §4 — "~80 baris").
-- kind         = jenis job ('deploy' — satu-satunya kind Fase 2)
-- payload_json = data job (mis. deployment_id) sebagai JSON teks
-- status       = queued -> running -> done | failed
-- run_at       = epoch detik kapan job boleh diklaim (mendukung retry
--                dengan delay di masa depan; Fase 2 selalu run_at = now)
-- attempts     = jumlah percobaan klaim
-- ============================================================
CREATE TABLE jobs (
    id           TEXT    PRIMARY KEY,
    kind         TEXT    NOT NULL,
    payload_json TEXT    NOT NULL,
    status       TEXT    NOT NULL DEFAULT 'queued'
                     CHECK (status IN ('queued', 'running', 'done', 'failed')),
    run_at       INTEGER NOT NULL,
    started_at   INTEGER,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    created_at   INTEGER NOT NULL
);

-- Indeks: worker SELECT job dengan status='queued' AND run_at <= now setiap
-- siklus klaim.
CREATE INDEX idx_jobs_status_run_at ON jobs (status, run_at);
