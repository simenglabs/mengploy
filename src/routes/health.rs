//! `GET /healthz` — publik, read-only, tidak menyentuh db pada jalur sukses.

pub async fn healthz() -> &'static str {
    "ok"
}
