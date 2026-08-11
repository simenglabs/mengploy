-- Koreksi Fase 6: seluruh histori metrik memakai timestamp sebagai awalan
-- primary key dan metrik container baru terikat langsung pada server.
-- SQLite mengharuskan rebuild untuk mengubah PRIMARY KEY pada WITHOUT ROWID.

CREATE TABLE metrics_host_v2 (
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
    PRIMARY KEY (ts, res, server_id)
) WITHOUT ROWID;

INSERT INTO metrics_host_v2
    (res, ts, server_id, cpu_avg, cpu_max, mem_used, mem_max, mem_total,
     load1, disk_used, disk_total, source)
SELECT res, ts, server_id, cpu_avg, cpu_max, mem_used, mem_max, mem_total,
       load1, disk_used, disk_total, source
FROM metrics_host;

DROP INDEX idx_metrics_host_server_res_ts;
ALTER TABLE metrics_host RENAME TO metrics_host_legacy;
ALTER TABLE metrics_host_v2 RENAME TO metrics_host;
CREATE INDEX idx_metrics_host_server_res_ts ON metrics_host (server_id, res, ts);

CREATE TABLE metrics_container_v2 (
    res           TEXT NOT NULL CHECK (res IN ('raw', 'min', 'hour')),
    ts            INTEGER NOT NULL,
    server_id     TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
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
    PRIMARY KEY (ts, res, server_id, container_id)
) WITHOUT ROWID;

-- Baris lama tidak memuat server_id dan tidak boleh ditebak. Arsipkan agar
-- data tidak dihapus diam-diam; histori ini ikut retensi tetapi tidak dipakai
-- dashboard sampai ada identitas server yang dapat diverifikasi.
DROP INDEX idx_metrics_container_app_res_ts;
DROP INDEX idx_metrics_container_id_res_ts;
ALTER TABLE metrics_container RENAME TO metrics_container_legacy;
ALTER TABLE metrics_container_v2 RENAME TO metrics_container;
CREATE INDEX idx_metrics_container_server_res_ts
    ON metrics_container (server_id, res, ts);
CREATE INDEX idx_metrics_container_app_res_ts
    ON metrics_container (app_id, res, ts);
CREATE INDEX idx_metrics_container_id_res_ts
    ON metrics_container (container_id, res, ts);
