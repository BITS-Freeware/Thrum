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
/// still signed in, and the signature key each one registered (base64, or
/// null before its first key package). Clients compare this with the MLS
/// roster to drop revoked devices and to check the keys of group leaves.
/// Unknown usernames are 404.
pub async fn list_active(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use base64::Engine as _;
    auth::authenticate(&state, &headers).await?;
    let rows = sqlx::query_as::<_, (String, Option<Vec<u8>>)>(
        "SELECT d.id, d.signature_key FROM devices d
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
    let mut rows = rows;
    for (device_id, key) in rows.iter_mut() {
        if key.is_none() {
            *key = backfill_signature_key(&state, &username, device_id).await?;
        }
    }
    let keys: serde_json::Map<String, serde_json::Value> = rows
        .iter()
        .map(|(id, key)| {
            let value = key.as_ref().map_or(serde_json::Value::Null, |key| {
                base64::engine::general_purpose::STANDARD.encode(key).into()
            });
            (id.clone(), value)
        })
        .collect();
    Ok(Json(serde_json::json!({
        "devices": rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
        "signature_keys": keys,
    })))
}

/// Bind the signature key of a device that uploaded key packages before
/// keys were recorded, from one of its stored packages.
async fn backfill_signature_key(
    state: &AppState,
    username: &str,
    device_id: &str,
) -> Result<Option<Vec<u8>>, ApiError> {
    let stored: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM key_packages WHERE device_id = $1 LIMIT 1")
            .bind(device_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(package) = stored else {
        return Ok(None);
    };
    let Ok((identity, signature_key)) = thrum_core::mls::key_package_binding(&package) else {
        return Ok(None);
    };
    let matches = identity
        .split_once('#')
        .is_some_and(|(user, device)| user.eq_ignore_ascii_case(username) && device == device_id);
    if !matches {
        return Ok(None);
    }
    sqlx::query("UPDATE devices SET signature_key = $1 WHERE id = $2 AND signature_key IS NULL")
        .bind(&signature_key)
        .bind(device_id)
        .execute(&state.db)
        .await?;
    let bound: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT signature_key FROM devices WHERE id = $1")
            .bind(device_id)
            .fetch_one(&state.db)
            .await?;
    Ok(bound)
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
    sqlx::query("UPDATE devices SET revoked_at = $1, token_hash = 'revoked:' || id WHERE id = $2 AND revoked_at IS NULL")
        .bind(accounts::now())
        .bind(&device_id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM key_packages WHERE device_id = $1")
        .bind(&device_id)
        .execute(&state.db)
        .await?;
    state.hub.disconnect(&[device_id]);
    Ok(StatusCode::NO_CONTENT)
}
