use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Error yang bisa dikembalikan handler. Pesan untuk klien sengaja generik
/// untuk error internal; detailnya hanya masuk log.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(String),
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("works-server error: {0}")]
    Works(String),
    #[error("not implemented")]
    NotImplemented,

    /* Token aksi. Kodenya dibedakan karena seller-app bercabang atasnya:
     * yang satu "sudah diproses", yang lain "minta saran baru". */
    #[error("token aksi tidak sah")]
    TokenInvalid,
    #[error("token aksi kedaluwarsa")]
    TokenExpired,
    #[error("token aksi sudah dipakai")]
    TokenUsed,
    #[error("jumlah {diminta} melebihi batas {batas}")]
    QtyAboveCap { diminta: i64, batas: i64 },
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", self.to_string()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            AppError::NotImplemented => (
                StatusCode::NOT_IMPLEMENTED,
                "not_implemented",
                self.to_string(),
            ),
            AppError::TokenInvalid => (StatusCode::UNAUTHORIZED, "token_invalid", self.to_string()),
            AppError::TokenExpired => (StatusCode::GONE, "token_expired", self.to_string()),
            // 409: permintaannya sah, tapi sudah pernah dijalankan.
            AppError::TokenUsed => (StatusCode::CONFLICT, "token_used", self.to_string()),
            AppError::QtyAboveCap { .. } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "qty_above_cap",
                self.to_string(),
            ),
            AppError::Works(_) => {
                tracing::error!(error = %self, "upstream error");
                (
                    StatusCode::BAD_GATEWAY,
                    "upstream_error",
                    "upstream error".into(),
                )
            }
            AppError::Config(_) => {
                tracing::error!(error = %self, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".into(),
                )
            }
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}
