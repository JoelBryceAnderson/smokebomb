//! Theme store catalog. Downloads are served from `asset_url` (object
//! storage/CDN); purchase flow is not implemented yet.

use axum::extract::State;
use axum::Json;

use crate::error::ApiResult;
use crate::models::Theme;
use crate::AppState;

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<Theme>>> {
    let themes = sqlx::query_as::<_, Theme>(
        r#"
        SELECT id, slug, name, description, kind, price_cents, asset_url,
               asset_size, min_firmware, created_at
        FROM themes
        WHERE published
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(themes))
}
