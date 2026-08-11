-- Lock level server menutup race app baru dibuat saat prune berjalan.
-- Deployment wajib memeriksa lock ini sebelum mengambil lock app.
CREATE TABLE fleet_server_locks (
    server_id    TEXT PRIMARY KEY REFERENCES servers (id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    expires_at   INTEGER NOT NULL
);
