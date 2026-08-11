-- Migrasi 0007: Fase 6 — metrik dan pemantauan.
--
-- Satu tabel per domain memakai kolom `res` ('raw', 'min', 'hour') sebagai
-- tingkat resolusi. Ini sengaja menghindari tiga tabel dengan bentuk identik;
-- rollup tetap terpisah secara logis dan diproteksi PRIMARY KEY yang sama.
-- Nilai `source` disiapkan untuk agen masa depan, tetapi worker saat ini
-- menulis 'ssh' atau 'docker'. Tidak ada daftar proses atau isi environment.

CREATE TABLE metrics_host (
    res         TEXT NOT NULL CHECK (res IN ('raw', 'min', 'hour')),
    ts          INTEGER NOT NULL,
    server_id   TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    cpu_avg     REAL,
    cpu_max     REAL,
    mem_used    INTEGER NOT NULL,
    mem_max     INTEGER NOT NULL,
    mem_total   INTEGER NOT NULL,
    load1       REAL NOT NULL,
    disk_used   INTEGER NOT NULL,
    disk_total  INTEGER NOT NULL,
    source      TEXT NOT NULL,
    PRIMARY KEY (res, ts, server_id)
) WITHOUT ROWID;

CREATE INDEX idx_metrics_host_server_res_ts
    ON metrics_host (server_id, res, ts);

CREATE TABLE metrics_container (
    res           TEXT NOT NULL CHECK (res IN ('raw', 'min', 'hour')),
    ts            INTEGER NOT NULL,
    container_id  TEXT NOT NULL,
    app_id        TEXT REFERENCES apps (id) ON DELETE SET NULL,
    cpu_avg       REAL,
    cpu_max       REAL,
    mem_bytes     INTEGER NOT NULL,
    mem_max       INTEGER NOT NULL,
    mem_limit     INTEGER NOT NULL,
    net_rx        INTEGER NOT NULL,
    net_tx        INTEGER NOT NULL,
    restart_count INTEGER NOT NULL,
    source        TEXT NOT NULL,
    PRIMARY KEY (res, ts, container_id)
) WITHOUT ROWID;

CREATE INDEX idx_metrics_container_app_res_ts
    ON metrics_container (app_id, res, ts);
CREATE INDEX idx_metrics_container_id_res_ts
    ON metrics_container (container_id, res, ts);

CREATE TABLE metric_alerts (
    id             TEXT PRIMARY KEY,
    server_id      TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    app_id         TEXT REFERENCES apps (id) ON DELETE SET NULL,
    container_id   TEXT,
    deployment_id  TEXT REFERENCES deployments (id) ON DELETE SET NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('disk_high', 'restart_loop', 'resource_spike')),
    severity       TEXT NOT NULL CHECK (severity IN ('warning', 'critical')),
    target         TEXT NOT NULL,
    message        TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'resolved')),
    first_seen_at  INTEGER NOT NULL,
    last_seen_at   INTEGER NOT NULL,
    resolved_at    INTEGER,
    UNIQUE (server_id, kind, target)
);

CREATE INDEX idx_metric_alerts_server_status
    ON metric_alerts (server_id, status, last_seen_at);

-- Retensi dijalankan worker metrik, bukan trigger: satu sapuan bisa dibungkus
-- transaksi dan tidak menambah biaya tulis pada setiap sampel.
