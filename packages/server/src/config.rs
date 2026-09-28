use std::net::SocketAddr;

use anyhow::Context;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: SocketAddr,
    /// HMAC secret for session JWTs. Required outside development.
    pub jwt_secret: String,
    pub run_migrations: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://smokebomb:smokebomb@localhost:5432/smokebomb".into()),
            bind_addr: std::env::var("BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8080".into())
                .parse()
                .context("BIND_ADDR must be host:port")?,
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-only-insecure-secret".into()),
            run_migrations: std::env::var("RUN_MIGRATIONS").map(|v| v != "0").unwrap_or(true),
        })
    }
}
