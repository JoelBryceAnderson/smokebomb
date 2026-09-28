use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// API error, rendered as `{"error": "<code>", "message": "..."}`.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("authentication required")]
    Unauthorized,
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ApiError::NotImplemented(_) => (StatusCode::NOT_IMPLEMENTED, "not_implemented"),
            ApiError::Database(sqlx::Error::RowNotFound) => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::Database(e) => {
                tracing::error!("database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        let message = match &self {
            ApiError::Database(_) => "internal error".to_string(),
            other => other.to_string(),
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

/// Decode a fixed-length hex field from a request body.
pub fn hex_field<const N: usize>(name: &str, value: &str) -> ApiResult<[u8; N]> {
    let bytes = hex::decode(value).map_err(|_| ApiError::BadRequest(format!("{name} must be hex")))?;
    bytes
        .try_into()
        .map_err(|_| ApiError::BadRequest(format!("{name} must be {N} bytes")))
}
