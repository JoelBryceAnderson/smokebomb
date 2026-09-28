//! Route table. Everything versioned lives under `/v1`.

mod auth;
mod devices;
mod health;
mod rolls;
mod themes;

use axum::routing::{get, post};
use axum::Router;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health))
        .route("/health/db", get(health::database))
        .route("/v1/auth/register", post(auth::register))
        .route("/v1/auth/login", post(auth::login))
        .route("/v1/auth/me", get(auth::me))
        .route("/v1/devices", post(devices::register))
        .route("/v1/devices/{serial}", get(devices::get))
        .route("/v1/devices/{serial}/rolls", get(rolls::list_for_device))
        .route("/v1/rolls", post(rolls::submit))
        .route("/v1/rolls/verify", post(rolls::verify))
        .route("/v1/themes", get(themes::list))
}
