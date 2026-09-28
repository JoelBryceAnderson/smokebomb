//! Roll verification and history.

use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smokebomb_shared::{DeviceSerial, DieKind, RollRecord, SessionId};

use crate::crypto;
use crate::error::{hex_field, ApiError, ApiResult};
use crate::models::{Device, Roll};
use crate::AppState;

/// A signed roll as relayed by the phone. Mirrors `smokebomb_shared::SignedRoll`
/// with hex-encoded byte fields.
#[derive(Debug, Deserialize)]
pub struct SignedRollBody {
    pub device_serial: String,
    /// 16-byte session id, hex. Omitted or all zeroes for casual rolls.
    pub session: Option<String>,
    pub counter: u32,
    pub uptime_ms: u64,
    /// `d4` … `d100` or `pass_the_pot`.
    pub die: String,
    /// Raw values in `1..=sides` (d6 values for Pass the Pot).
    pub values: Vec<u8>,
    pub prev_hash: String,
    pub signature: String,
}

impl SignedRollBody {
    fn record(&self) -> ApiResult<RollRecord> {
        let die = DieKind::from_wire(&self.die)
            .ok_or_else(|| ApiError::BadRequest(format!("unsupported die {:?}", self.die)))?;
        if self.values.is_empty() || self.values.len() > die.max_count() {
            return Err(ApiError::BadRequest(format!(
                "{} takes 1 to {} dice",
                die.wire_name(),
                die.max_count()
            )));
        }
        if self.values.iter().any(|&v| v == 0 || v > die.sides()) {
            return Err(ApiError::BadRequest("value out of range for die".into()));
        }
        let values = heapless::Vec::from_slice(&self.values)
            .map_err(|_| ApiError::BadRequest("too many dice".into()))?;
        let session = match &self.session {
            Some(s) => SessionId(hex_field("session", s)?),
            None => SessionId::NONE,
        };
        Ok(RollRecord {
            device: DeviceSerial(hex_field("device_serial", &self.device_serial)?),
            session,
            counter: self.counter,
            uptime_ms: self.uptime_ms,
            die,
            values,
            prev_hash: hex_field("prev_hash", &self.prev_hash)?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainStatus {
    /// The previous roll is on file and this one links to it.
    Linked,
    /// First roll from this device.
    Genesis,
    /// Previous roll not uploaded yet; link can't be checked.
    Unknown,
    /// The previous roll on file has a different digest.
    Broken,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub valid: bool,
    pub digest: String,
    pub chain: ChainStatus,
    pub reason: Option<String>,
}

/// Check a roll's signature against the registered device key and its link
/// to the previous roll on file. Does not store anything.
pub async fn verify(
    State(state): State<AppState>,
    Json(body): Json<SignedRollBody>,
) -> ApiResult<Json<VerifyResponse>> {
    let record = body.record()?;
    let signature: [u8; 64] = hex_field("signature", &body.signature)?;
    let digest = record.digest();

    let device = sqlx::query_as::<_, Device>("SELECT * FROM devices WHERE serial = $1")
        .bind(hex::encode(record.device.0))
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::BadRequest("device is not registered".into()))?;

    let sig_result = crypto::verify_digest(&device.public_key, &digest, &signature);

    let chain = if record.prev_hash == smokebomb_shared::roll::GENESIS_HASH && record.counter == 0 {
        ChainStatus::Genesis
    } else {
        let prev: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT digest FROM rolls WHERE device_id = $1 AND counter = $2")
                .bind(device.id)
                .bind(record.counter as i64 - 1)
                .fetch_optional(&state.db)
                .await?;
        match prev {
            Some(d) if d == record.prev_hash => ChainStatus::Linked,
            Some(_) => ChainStatus::Broken,
            None => ChainStatus::Unknown,
        }
    };

    Ok(Json(VerifyResponse {
        valid: sig_result.is_ok() && !matches!(chain, ChainStatus::Broken),
        digest: hex::encode(digest),
        chain,
        reason: sig_result.err().map(|e| e.to_string()),
    }))
}

/// Upload a roll for history sync. TODO: verify, then insert into `rolls`.
pub async fn submit(Json(_body): Json<SignedRollBody>) -> ApiResult<Json<RollView>> {
    Err(ApiError::NotImplemented("roll upload"))
}

#[derive(Debug, Serialize)]
pub struct RollView {
    pub counter: i64,
    pub die: String,
    pub values: Vec<i16>,
    pub digest: String,
    pub received_at: DateTime<Utc>,
}

pub async fn list_for_device(
    State(state): State<AppState>,
    Path(serial): Path<String>,
) -> ApiResult<Json<Vec<RollView>>> {
    let serial: [u8; 9] = hex_field("serial", &serial)?;
    let rolls = sqlx::query_as::<_, Roll>(
        r#"
        SELECT r.* FROM rolls r
        JOIN devices d ON d.id = r.device_id
        WHERE d.serial = $1
        ORDER BY r.counter DESC
        LIMIT 100
        "#,
    )
    .bind(hex::encode(serial))
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rolls
            .into_iter()
            .map(|r| RollView {
                counter: r.counter,
                die: r.die,
                values: r.dice,
                digest: hex::encode(r.digest),
                received_at: r.received_at,
            })
            .collect(),
    ))
}
