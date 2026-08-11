-- Migrasi 0006: Fase 5 — rekonsiliasi dan delivery notifikasi.
-- Kolom lock serta indeks deployments(app_id, created_at) sudah ada sejak 0003.
-- expected_json/observed_json hanya metadata non-secret; tidak menyimpan log,
-- credential, environment, URL webhook, atau stderr mentah.

CREATE TABLE reconciliation_findings (
    id               TEXT PRIMARY KEY,
    app_id           TEXT NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    server_id        TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    deployment_id    TEXT REFERENCES deployments (id) ON DELETE SET NULL,
    kind             TEXT NOT NULL,
    severity         TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'open'
                     CHECK (status IN ('open', 'acknowledged', 'resolved')),
    fingerprint      TEXT NOT NULL,
    expected_json    TEXT,
    observed_json    TEXT,
    first_seen_at    INTEGER NOT NULL,
    last_seen_at     INTEGER NOT NULL,
    acknowledged_at  INTEGER,
    resolved_at      INTEGER,
    UNIQUE (server_id, fingerprint)
);

CREATE INDEX idx_reconciliation_findings_app_status
    ON reconciliation_findings (app_id, status);
CREATE INDEX idx_reconciliation_findings_server_status
    ON reconciliation_findings (server_id, status);
CREATE INDEX idx_reconciliation_findings_last_seen
    ON reconciliation_findings (last_seen_at);

CREATE TABLE notification_deliveries (
    id                TEXT PRIMARY KEY,
    event_id          TEXT NOT NULL,
    event_type        TEXT NOT NULL,
    app_id            TEXT REFERENCES apps (id) ON DELETE SET NULL,
    payload_json      TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'queued'
                      CHECK (status IN ('queued', 'sending', 'delivered', 'failed')),
    attempts          INTEGER NOT NULL DEFAULT 0,
    next_attempt_at   INTEGER NOT NULL,
    last_status_code  INTEGER,
    last_error_kind   TEXT,
    created_at        INTEGER NOT NULL,
    delivered_at     INTEGER,
    UNIQUE (event_id, event_type)
);

CREATE INDEX idx_notification_deliveries_status_next
    ON notification_deliveries (status, next_attempt_at);
