use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Migrations are embedded at compile time from `packages/server/migrations`.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new().max_connections(10).connect(url).await
}
