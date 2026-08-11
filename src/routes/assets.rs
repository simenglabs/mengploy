//! `GET /assets/htmx.min.js`, `GET /assets/htmx-sse.min.js` — aset statis
//! HTMX di-vendor lokal, di-embed ke binary saat kompilasi (Q4
//! `docs/plan.md`: vendor, bukan CDN — menghindari dependensi jaringan
//! eksternal di halaman terlindungi). Publik (tidak memuat data pengguna
//! sama sekali), daftar aset TETAP saat kompilasi — tidak ada path param,
//! tidak ada kemungkinan path traversal (`docs/api-contract.md`).

use axum::http::header;
use axum::response::{IntoResponse, Response};

const HTMX_JS: &str = include_str!("../web/assets/htmx.min.js");
const HTMX_SSE_JS: &str = include_str!("../web/assets/htmx-sse.min.js");

pub async fn htmx_js() -> Response {
    ([(header::CONTENT_TYPE, "application/javascript")], HTMX_JS).into_response()
}

pub async fn htmx_sse_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        HTMX_SSE_JS,
    )
        .into_response()
}
