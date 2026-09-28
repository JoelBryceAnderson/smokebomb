//! Account endpoints (stubs). See `crate::auth` for the plan.

use axum::Json;
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::models::User;

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // fields are read once registration is implemented
pub struct Credentials {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

pub async fn register(Json(_body): Json<Credentials>) -> ApiResult<Json<User>> {
    Err(ApiError::NotImplemented("registration"))
}

pub async fn login(Json(_body): Json<Credentials>) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented("login"))
}

pub async fn me(_user: AuthUser) -> ApiResult<Json<User>> {
    Err(ApiError::NotImplemented("current user"))
}
