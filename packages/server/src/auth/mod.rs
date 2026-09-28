//! Authentication (stub).
//!
//! Plan: accounts log in with email + password (argon2) and receive a
//! short-lived HS256 JWT plus a refresh token. Devices never use JWTs; they
//! prove identity by signing with their ATECC608 key (see `crate::crypto`).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::ApiError;

/// Authenticated account, extracted from `Authorization: Bearer <jwt>`.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
}

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let _token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        // TODO: validate the JWT against `Config::jwt_secret`.
        Err(ApiError::NotImplemented("JWT validation"))
    }
}
