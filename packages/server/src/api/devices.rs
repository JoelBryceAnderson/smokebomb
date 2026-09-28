//! Device registry: serial -> public key, owner and firmware version.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{hex_field, ApiResult};
use crate::models::Device;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterDevice {
    /// 9-byte ATECC608 serial, hex.
    pub serial: String,
    /// 64-byte raw P-256 public key (X || Y), hex.
    pub public_key: String,
    pub firmware_version: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceView {
    pub id: Uuid,
    pub serial: String,
    pub public_key: String,
    pub owner_name: Option<String>,
    pub firmware_version: String,
    pub registered_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

impl From<Device> for DeviceView {
    fn from(d: Device) -> Self {
        Self {
            id: d.id,
            serial: d.serial,
            public_key: hex::encode(d.public_key),
            owner_name: d.owner_name,
            firmware_version: d.firmware_version,
            registered_at: d.registered_at,
            last_seen_at: d.last_seen_at,
        }
    }
}

/// Register a die, or refresh its firmware version if already known.
///
/// TODO: this must only accept keys attested during factory provisioning
/// (signed by the Smokebomb manufacturing CA); today anyone can register.
pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterDevice>,
) -> ApiResult<(StatusCode, Json<DeviceView>)> {
    let serial: [u8; 9] = hex_field("serial", &body.serial)?;
    let public_key: [u8; 64] = hex_field("public_key", &body.public_key)?;

    let device = sqlx::query_as::<_, Device>(
        r#"
        INSERT INTO devices (serial, public_key, firmware_version)
        VALUES ($1, $2, $3)
        ON CONFLICT (serial) DO UPDATE
            SET firmware_version = EXCLUDED.firmware_version,
                last_seen_at = now()
        RETURNING *
        "#,
    )
    .bind(hex::encode(serial))
    .bind(public_key.as_slice())
    .bind(&body.firmware_version)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(device.into())))
}

pub async fn get(State(state): State<AppState>, Path(serial): Path<String>) -> ApiResult<Json<DeviceView>> {
    let serial: [u8; 9] = hex_field("serial", &serial)?;
    let device = sqlx::query_as::<_, Device>("SELECT * FROM devices WHERE serial = $1")
        .bind(hex::encode(serial))
        .fetch_one(&state.db)
        .await?;
    Ok(Json(device.into()))
}
