-- Payload operasi dipisah dari `targets` agar target tetap dapat dibaca
-- sebagai metadata dan perintah tidak perlu dipaksakan masuk array target.
ALTER TABLE fleet_operations
    ADD COLUMN payload_json TEXT NOT NULL DEFAULT '{}';
