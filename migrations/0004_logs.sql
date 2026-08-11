-- Migrasi 0004: Fase 3 — log deploy dan metadata retensi.
--
-- Satu tabel baru:
--   deployment_logs — metadata file log per deployment (path + ukuran + baris)
--
-- INVARIANT §3 NO.9: baris log TIDAK PERNAH masuk SQLite. Tabel ini hanya
-- menyimpan path file dan metadata — tidak ada kolom teks bebas yang bisa
-- menampung isi log. Kolom 'path' menyimpan nama file saja (relatif terhadap
-- <log_dir>/deploy/), bukan path absolut: kalau MENGDEP_LOG_DIR berubah,
-- baris lama tetap benar, dan tidak ada path absolut yang bisa bocor ke
-- klien lewat pesan error.
--
-- Kolom metrik, env var, dan fase lain TIDAK ada di sini — PRD §4 melarang
-- mendesain skema untuk fase yang belum tiba.

-- ============================================================
-- deployment_logs: satu baris per deployment yang sudah punya file log.
-- deployment_id = PK sekaligus FK ke deployments.id. Satu deployment tepat
--                 satu file log; deploy ulang menghasilkan deployment baru,
--                 jadi tidak pernah ada dua log untuk satu id.
-- path          = nama file saja, relatif terhadap <log_dir>/deploy/
--                 (mis. "{deployment_id}.log"). BUKAN path absolut —
--                 kalau MENGDEP_LOG_DIR berubah, baris lama tetap benar,
--                 dan tidak ada path absolut yang bisa bocor ke klien.
-- size_bytes    = ukuran file dalam byte (di-UPDATE berkala oleh writer,
--                 bukan per baris — hindari beban pool tulis max_connections=1)
-- line_count    = jumlah baris dalam file
-- truncated     = penanda 0/1: 1 kalau writer berhenti karena batas 8 MiB
--                 terlampaui. Deploy tetap jalan sampai selesai — log adalah
--                 pengamatan, bukan kontrol (docs/prd.md §3 no.1).
-- created_at    = epoch detik saat file log dibuat (awal deploy)
-- updated_at    = epoch detik saat metadata terakhir di-UPDATE
-- ============================================================
CREATE TABLE deployment_logs (
    deployment_id  TEXT    PRIMARY KEY REFERENCES deployments (id) ON DELETE CASCADE,
    path           TEXT    NOT NULL,
    size_bytes     INTEGER NOT NULL DEFAULT 0,
    line_count     INTEGER NOT NULL DEFAULT 0,
    truncated      INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

-- Indeks: sapuan retensi SELECT deployment_id FROM deployment_logs
-- WHERE created_at < ? LIMIT 500 — satu-satunya query rentang di tabel ini.
-- Tanpa indeks ini, retensi melakukan full scan tiap 24 jam.
CREATE INDEX idx_deployment_logs_created_at ON deployment_logs (created_at);
