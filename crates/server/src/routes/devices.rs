//! Device listing and revocation endpoints.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use serde::Serialize;
use sqlx::Row;

use crate::accounts;
use crate::auth::{self, AuthedDevice};
use crate::error::ApiError;
use crate::state::AppState;

/// A device as the user sees it.
#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub label: String,
    pub created_at: i64,
    pub active: bool,
}

/// `GET /v1/devices`: the caller's devices, oldest first.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DeviceInfo>>, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, label, created_at, revoked_at FROM devices
         WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(authed.user_id)
    .fetch_all(&state.db)
    .await?;
    let devices = rows
        .into_iter()
        .map(|row| DeviceInfo {
            id: row.get("id"),
            label: row.get("label"),
            created_at: row.get("created_at"),
            active: row.get::<Option<i64>, _>("revoked_at").is_none(),
        })
        .collect();
    Ok(Json(devices))
}

/// `GET /v1/users/{username}/devices`: ids of a user's devices that are
/// still signed in. Clients compare this with the MLS roster to drop
/// revoked devices. Unknown usernames are 404.
pub async fn list_active(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth::authenticate(&state, &headers).await?;
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT d.id FROM devices d
         JOIN users u ON u.id = d.user_id
         WHERE lower(u.username) = lower($1) AND d.revoked_at IS NULL
         ORDER BY d.created_at, d.id",
    )
    .bind(&username)
    .fetch_all(&state.db)
    .await?;
    let known =
        sqlx::query_as::<_, (i64,)>("SELECT 1 FROM users WHERE lower(username) = lower($1)")
            .bind(&username)
            .fetch_optional(&state.db)
            .await?;
    if known.is_none() {
        return Err(ApiError::not_found());
    }
    Ok(Json(
        serde_json::json!({ "devices": rows.into_iter().map(|(id,)| id).collect::<Vec<_>>() }),
    ))
}

/// `DELETE /v1/devices/{device_id}`: revoke one of the caller's devices.
/// Idempotent; unknown devices and other users' devices are both 404.
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let authed: AuthedDevice = auth::authenticate(&state, &headers).await?;
    let owner = sqlx::query_as::<_, (i64,)>("SELECT user_id FROM devices WHERE id = $1")
        .bind(&device_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if owner.0 != authed.user_id {
        return Err(ApiError::not_found());
    }
    sqlx::query("UPDATE devices SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL")
        .bind(accounts::now())
        .bind(&device_id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM key_packages WHERE device_id = $1")
        .bind(&device_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
