//! API tests.
//!
//! Tests that need PostgreSQL run only when `DATABASE_URL` is set (CI starts a
//! postgres service); the rest use a lazy pool that never connects.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use smokebomb_core::roll::RollEngine;
use smokebomb_hal::SecureElement;
use smokebomb_hal_simulator::{SimRng, SimSecureElement};
use smokebomb_server::config::Config;
use smokebomb_server::{db, router, AppState};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn config(url: &str) -> Config {
    Config {
        database_url: url.into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        jwt_secret: "test".into(),
        run_migrations: true,
    }
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_default())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn health_and_stubs_without_database() {
    let url = "postgres://nobody@127.0.0.1:1/none";
    let pool = PgPoolOptions::new().connect_lazy(url).unwrap();
    let app = router(AppState::new(pool, config(url)));

    let (status, body) = call(&app, "GET", "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");

    let creds = json!({ "email": "a@b.c", "password": "x" });
    let (status, body) = call(&app, "POST", "/v1/auth/login", Some(creds)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["error"], "not_implemented");
}

#[tokio::test]
async fn register_device_and_verify_simulated_rolls() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let pool = db::connect(&url).await.unwrap();
    db::MIGRATOR.run(&pool).await.unwrap();
    let app = router(AppState::new(pool, config(&url)));

    // Register the simulator's die.
    let mut se = SimSecureElement::new();
    let serial = hex::encode(se.serial().unwrap());
    let (status, device) = call(
        &app,
        "POST",
        "/v1/devices",
        Some(json!({
            "serial": serial,
            "public_key": hex::encode(se.public_key().unwrap()),
            "firmware_version": "0.1.0",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{device}");

    let (status, _) = call(&app, "GET", &format!("/v1/devices/{serial}"), None).await;
    assert_eq!(status, StatusCode::OK);

    // Roll with the firmware's own engine and check the server agrees.
    let mut engine = RollEngine::new(&mut se).unwrap();
    let roll = engine
        .roll(&mut SimRng, &mut se, smokebomb_shared::DieKind::D20, 2, 1_000)
        .unwrap();
    let mut body = json!({
        "device_serial": serial,
        "counter": roll.record.counter,
        "uptime_ms": roll.record.uptime_ms,
        "die": "d20",
        "values": roll.record.values.to_vec(),
        "prev_hash": hex::encode(roll.record.prev_hash),
        "signature": hex::encode(roll.signature),
    });

    let (status, res) = call(&app, "POST", "/v1/rolls/verify", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["valid"], true, "{res}");
    assert_eq!(res["chain"], "genesis");
    assert_eq!(res["digest"], hex::encode(roll.record.digest()));

    // Tampering with a value must invalidate the signature.
    body["values"] = json!([20, 20]);
    let (_, res) = call(&app, "POST", "/v1/rolls/verify", Some(body)).await;
    assert_eq!(res["valid"], false, "{res}");

    // Pass the Pot: signed as raw d6 values, chained to the previous roll.
    let pot = engine
        .roll(
            &mut SimRng,
            &mut se,
            smokebomb_shared::DieKind::PassThePot,
            3,
            2_000,
        )
        .unwrap();
    let mut body = json!({
        "device_serial": serial,
        "counter": pot.record.counter,
        "uptime_ms": pot.record.uptime_ms,
        "die": "pass_the_pot",
        "values": pot.record.values.to_vec(),
        "prev_hash": hex::encode(pot.record.prev_hash),
        "signature": hex::encode(pot.signature),
    });
    let (status, res) = call(&app, "POST", "/v1/rolls/verify", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["valid"], true, "{res}");

    // The same bytes claimed as a d6 roll no longer match the signature.
    body["die"] = json!("d6");
    let (_, res) = call(&app, "POST", "/v1/rolls/verify", Some(body.clone())).await;
    assert_eq!(res["valid"], false, "{res}");

    // Pass the Pot allows at most three dice.
    body["die"] = json!("pass_the_pot");
    body["values"] = json!([1, 2, 3, 4]);
    let (status, _) = call(&app, "POST", "/v1/rolls/verify", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
