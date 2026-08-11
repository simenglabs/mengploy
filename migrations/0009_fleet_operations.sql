-- Migrasi 0009: Fase 7 — operasi armada dan pintu darurat.
-- `targets` berisi JSON array id server; output perintah selalu berada di
-- file privat control plane dan kolom output_path hanya metadata lokasi.
CREATE TABLE fleet_operations (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('command', 'prune', 'exec')),
    targets    TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'queued'
               CHECK (status IN ('queued', 'running', 'partial', 'succeeded', 'failed')),
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_fleet_operations_created_at
    ON fleet_operations (created_at DESC);

CREATE TABLE fleet_operation_results (
    operation_id TEXT NOT NULL REFERENCES fleet_operations (id) ON DELETE CASCADE,
    server_id    TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    exit_code    INTEGER,
    output_path  TEXT,
    status       TEXT NOT NULL CHECK (status IN ('succeeded', 'failed', 'skipped')),
    PRIMARY KEY (operation_id, server_id)
) WITHOUT ROWID;

CREATE INDEX idx_fleet_operation_results_operation
    ON fleet_operation_results (operation_id);
