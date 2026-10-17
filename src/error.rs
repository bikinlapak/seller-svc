use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Errors a handler may return. Client-facing messages are deliberately
/// generic for internal failures; the detail only reaches the log.
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

    /* Action tokens. These get distinct codes because seller-app branches on
     * them: one means "already done", the other "ask for a fresh suggestion". */
    #[error("invalid action token")]
    TokenInvalid,
    #[error("action token expired")]
    TokenExpired,
    #[error("action token already used")]
    TokenUsed,
    #[error("quantity {requested} exceeds the cap of {cap}")]
    QtyAboveCap { requested: i64, cap: i64 },
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
            // 409: the request is valid, it has simply already been carried out.
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
