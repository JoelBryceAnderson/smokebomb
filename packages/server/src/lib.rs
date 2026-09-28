//! Smokebomb backend.
//!
//! Axum + SQLx on PostgreSQL. See `docs/API.md` for the REST surface and
//! `migrations/` for the schema.

pub mod api;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod models;

use std::sync::Arc;

use axum::Router;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<config::Config>,
}

impl AppState {
    pub fn new(db: PgPool, config: config::Config) -> Self {
        Self {
            db,
            config: Arc::new(config),
        }
    }
}

pub fn router(state: AppState) -> Router {
    api::routes()
        .layer(TraceLayer::new_for_http())
        // TODO: restrict origins once the web verification page has a domain.
        .layer(CorsLayer::permissive())
        .with_state(state)
}
