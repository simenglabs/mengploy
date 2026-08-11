//! Tipe error domain untuk handler Axum.
//!
//! `AppError` membungkus semua kegagalan yang bisa terjadi di jalur request
//! dan memetakannya ke response HTTP generik Bahasa Indonesia — tidak pernah
//! membocorkan path filesystem, isi query, atau pesan library mentah
//! (`docs/api-contract.md` aturan umum).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Error domain yang bisa dipetakan langsung ke response HTTP.
///
/// Varian dibedakan supaya pemanggil (handler) bisa memilih status code yang
/// tepat tanpa mengekspos detail internal ke klien.
#[derive(Debug)]
pub enum AppError {
    /// Kredensial salah, token CSRF tidak cocok, atau validasi input gagal.
    /// Pesan sudah aman untuk ditampilkan ke pengguna (Bahasa Indonesia,
    /// generik, tidak membedakan sebab spesifik).
    BadRequest(String),
    /// Tidak ada sesi valid / sesi kedaluwarsa — untuk sesi cookie, middleware
    /// (`auth/middleware.rs`) redirect langsung ke `/login` tanpa lewat sini.
    /// Dipakai `routes::deploy_api` (bearer token) yang butuh 401 eksplisit,
    /// bukan redirect.
    Unauthorized,
    /// `{id}` path param tidak dikenal (`docs/api-contract.md`: "Id yang
    /// tidak dikenal → 404, bukan 403 dan bukan 500").
    NotFound,
    /// Permintaan bertabrakan dengan state yang sudah ada — job verifikasi
    /// yang masih berjalan, atau fingerprint host key tersimpan yang
    /// berbeda (`docs/api-contract.md`: kedua kasus itu eksplisit 409,
    /// bukan 400).
    Conflict(String),
    /// Satu TAHAP operasi melewati batas waktunya (invariant §3 no.11 —
    /// timeout per tahap, bukan timeout global). Dipetakan ke 504 sesuai
    /// `docs/api-contract.md` (mis. pencarian dalam file log 5 detik).
    /// Pesan sudah berupa kategori Bahasa Indonesia yang menyebut langkah
    /// perbaikannya, tanpa detail internal.
    Timeout(String),
    /// Batas jumlah sesi serentak tercapai (mis. empat sesi log runtime,
    /// `docs/plan.md` tabel angka) → 429. Pesan menyebut tindakan yang bisa
    /// dilakukan pengguna, bukan angka internal.
    TooManyRequests(String),
    /// Server target membalas dengan cara yang membuat permintaan tidak bisa
    /// dipenuhi (mis. container sudah tidak ada di sana) → 502. Kategori,
    /// bukan stderr mentah.
    BadGateway(String),
    /// Kegagalan internal (db, IO, dsb). Detail hanya masuk ke `tracing`,
    /// tidak pernah ke body response.
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Sesi tidak valid atau kedaluwarsa.".to_string(),
            )
                .into_response(),
            AppError::NotFound => {
                (StatusCode::NOT_FOUND, crate::web::render_404(None)).into_response()
            }
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg).into_response(),
            AppError::Timeout(msg) => (StatusCode::GATEWAY_TIMEOUT, msg).into_response(),
            AppError::TooManyRequests(msg) => (StatusCode::TOO_MANY_REQUESTS, msg).into_response(),
            AppError::BadGateway(msg) => (StatusCode::BAD_GATEWAY, msg).into_response(),
            AppError::Internal(err) => {
                tracing::error!(error = ?err, "kegagalan internal saat memproses request");
                (StatusCode::INTERNAL_SERVER_ERROR, crate::web::render_500()).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn internal_error_tidak_membocorkan_pesan_asli() {
        let err = AppError::Internal(anyhow::anyhow!("path rahasia /etc/secret bocor di sini"));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("rahasia"));
        assert!(text.contains("kesalahan internal"));
    }
}
