//! Key package upload and one-time fetch.
//!
//! Key packages are opaque MLS bytes to the server: it stores and hands them
//! out without looking inside. A fetch consumes the package, because MLS key
//! packages are single-use.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use uuid::Uuid;

use crate::accounts;
use crate::auth;
use crate::error::ApiError;
use crate::state::AppState;

/// Upper bound on one stored key package. Real MLS key packages are a few
/// hundred bytes; the limit only exists to stop abuse.
pub const MAX_KEY_PACKAGE_BYTES: usize = 64 * 1024;

/// `POST /v1/key-packages`: store a key package for the caller's device.
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty_key_package"));
    }
    if body.len() > MAX_KEY_PACKAGE_BYTES {
        return Err(ApiError::payload_too_large());
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO key_packages (id, device_id, content, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&id)
    .bind(&authed.device_id)
    .bind(&body[..])
    .bind(accounts::now())
    .execute(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// `GET /v1/devices/{device_id}/key-package`: fetch and consume one key
/// package of a device, oldest first. Revoked devices have none.
pub async fn fetch(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Result<Response, ApiError> {
    let content = sqlx::query_scalar::<_, Vec<u8>>(
        "DELETE FROM key_packages WHERE id = (
             SELECT kp.id FROM key_packages kp JOIN devices d ON d.id = kp.device_id
             WHERE kp.device_id = ?1 AND d.revoked_at IS NULL
             ORDER BY kp.created_at, kp.id LIMIT 1
         )
         RETURNING content",
    )
    .bind(&device_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    Ok((
        StatusCode::OK,
        [(CONTENT_TYPE, "application/octet-stream")],
        content,
    )
        .into_response())
}
