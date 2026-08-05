use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Typed application error → JSON response. Message is included as-is; if that
/// becomes a leak in production, gate `self.to_string()` on `APP_ENV`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    /// Unauthorized with a specific message (parity with the Node auth middleware).
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    BadGateway(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Auth(_) => StatusCode::UNAUTHORIZED,
            AppError::BadGateway(_) => StatusCode::BAD_GATEWAY,
            AppError::Config(_) | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<axum::extract::multipart::MultipartError> for AppError {
    fn from(e: axum::extract::multipart::MultipartError) -> Self {
        AppError::BadRequest(format!("Invalid upload: {e}"))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("internal error: {}", self);
        }
        // Parity with the Node error handler: real message in dev, generic in production.
        let expose = std::env::var("APP_ENV").as_deref() != Ok("production");
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR && !expose {
            "Internal server error".to_string()
        } else {
            self.to_string()
        };
        (status, Json(json!({ "success": false, "message": message }))).into_response()
    }
}
