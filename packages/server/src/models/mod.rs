//! Database row types.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Device {
    pub id: Uuid,
    /// ATECC608 serial, lowercase hex (18 chars).
    pub serial: String,
    /// Raw P-256 public key, X || Y.
    pub public_key: Vec<u8>,
    pub owner_id: Option<Uuid>,
    pub owner_name: Option<String>,
    pub firmware_version: String,
    pub registered_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Roll {
    pub id: Uuid,
    pub device_id: Uuid,
    pub session_id: Option<Uuid>,
    pub counter: i64,
    pub die_sides: i16,
    pub dice: Vec<i16>,
    pub digest: Vec<u8>,
    pub prev_hash: Vec<u8>,
    pub signature: Vec<u8>,
    pub device_uptime_ms: i64,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Session {
    pub id: Uuid,
    pub join_code: String,
    pub name: String,
    pub organizer_id: Option<Uuid>,
    pub locked: bool,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Theme {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub price_cents: i32,
    pub asset_url: String,
    pub asset_size: i64,
    pub min_firmware: String,
    pub created_at: DateTime<Utc>,
}
