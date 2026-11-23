//! Key package upload and one-time fetch.
//!
//! The server stores key packages and hands them out; a fetch consumes a
//! one-time package, because MLS key packages are single-use. On upload it
//! checks one thing inside: that the package is for the uploading device,
//! and that it carries the signature key the device registered with its
//! first package. Clients compare group leaves with that key, so a member
//! cannot add a leaf of their own under someone else's device.

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

/// Header that marks an upload as the device's last-resort key package.
const ROLE_HEADER: &str = "x-thrum-key-package-role";

/// `POST /v1/key-packages`: store a key package for the caller's device.
///
/// A package is one-time unless `x-thrum-key-package-role: last-resort`.
/// One-time stock is capped. A new last-resort package replaces the old one
/// and is not counted toward the cap.
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
        return Err(ApiError::payload_too_large("key_package_too_large"));
    }
    bind_signature_key(&state, &authed, &body).await?;
    let last_resort = headers
        .get(ROLE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("last-resort"));
    // Check and insert under a lock on the device row, so parallel uploads
    // cannot pass the cap together or leave two last-resort packages.
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE devices SET label = label WHERE id = $1")
        .bind(&authed.device_id)
        .execute(&mut *tx)
        .await?;
    if last_resort {
        sqlx::query("DELETE FROM key_packages WHERE device_id = $1 AND last_resort = 1")
            .bind(&authed.device_id)
            .execute(&mut *tx)
            .await?;
    } else {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM key_packages WHERE device_id = $1 AND last_resort = 0",
        )
        .bind(&authed.device_id)
        .fetch_one(&mut *tx)
        .await?;
        let cap = state.cfg.key_package_max_per_device;
        if cap > 0 && count >= i64::from(cap) {
            return Err(ApiError::conflict("key_package_stock_full"));
        }
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO key_packages (id, device_id, content, created_at, last_resort)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(&authed.device_id)
    .bind(&body[..])
    .bind(accounts::now())
    .bind(i64::from(last_resort))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// Check that an uploaded key package is for the uploading device and
/// carries its registered signature key, registering the key on the first
/// upload.
async fn bind_signature_key(
    state: &AppState,
    authed: &auth::AuthedDevice,
    package: &[u8],
) -> Result<(), ApiError> {
    let (identity, signature_key) = thrum_core::mls::key_package_binding(package)
        .map_err(|_| ApiError::bad_request("invalid_key_package"))?;
    let expected_device = identity
        .split_once('#')
        .filter(|(username, device_id)| {
            username.eq_ignore_ascii_case(&authed.username) && *device_id == authed.device_id
        })
        .is_some();
    if !expected_device {
        return Err(ApiError::bad_request("key_package_identity_mismatch"));
    }
    sqlx::query("UPDATE devices SET signature_key = $1 WHERE id = $2 AND signature_key IS NULL")
        .bind(&signature_key)
        .bind(&authed.device_id)
        .execute(&state.db)
        .await?;
    let bound: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT signature_key FROM devices WHERE id = $1")
            .bind(&authed.device_id)
            .fetch_one(&state.db)
            .await?;
    if bound.as_deref() != Some(signature_key.as_slice()) {
        return Err(ApiError::conflict("signature_key_mismatch"));
    }
    Ok(())
}

/// `GET /v1/devices/{device_id}/key-package`: fetch one key package of a
/// device. Requires a logged-in caller. One-time packages are consumed,
/// oldest first. When none remain, the device's last-resort package is
/// returned and kept, so the device can still be added to a conversation.
///
/// A caller is limited per minute overall and per device it fetches. A
/// device also has a ceiling for everyone together: past it, fetches get
/// the last-resort package, so strangers can neither drain the device's
/// stock nor stop others from adding it.
pub async fn fetch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Response, ApiError> {
    use crate::ratelimit::Limit;
    let authed = auth::authenticate(&state, &headers).await?;
    state.limiter.check_keyed(
        Limit::KeyPackageFetchCaller,
        &authed.user_id.to_string(),
        state.cfg.key_package_fetch_per_caller_per_minute,
    )?;
    state.limiter.check_keyed(
        Limit::KeyPackageFetchPair,
        &format!("{}|{device_id}", authed.user_id),
        state.cfg.key_package_fetch_per_caller_device_per_minute,
    )?;
    let over_ceiling = state
        .limiter
        .check_keyed(
            Limit::KeyPackageFetchDevice,
            &device_id,
            state.cfg.key_package_fetch_per_device_per_minute,
        )
        .is_err();
    // Oldest first; the rowid/seq tiebreak keeps multiple packages created
    // in the same second in insertion order (see undelivered_page).
    let tiebreak = match state.backend {
        crate::db::Backend::Sqlite => "kp.rowid",
        crate::db::Backend::Postgres => "kp.seq",
    };
    // Fetches of one device run one after another under its row lock, so
    // concurrent fetches take different one-time packages while any remain.
    let mut tx = state.db.begin().await?;
    let live = sqlx::query("UPDATE devices SET label = label WHERE id = $1 AND revoked_at IS NULL")
        .bind(&device_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if live == 0 {
        return Err(ApiError::not_found());
    }
    let one_time = if over_ceiling {
        None
    } else {
        sqlx::query_scalar::<_, Vec<u8>>(&format!(
            "DELETE FROM key_packages WHERE id = (
                 SELECT kp.id FROM key_packages kp
                 WHERE kp.device_id = $1 AND kp.last_resort = 0
                 ORDER BY kp.created_at, {tiebreak} LIMIT 1
             )
             RETURNING content"
        ))
        .bind(&device_id)
        .fetch_optional(&mut *tx)
        .await?
    };
    let content = match one_time {
        Some(content) => content,
        None => sqlx::query_scalar::<_, Vec<u8>>(&format!(
            "SELECT kp.content FROM key_packages kp
             WHERE kp.device_id = $1 AND kp.last_resort = 1
             ORDER BY kp.created_at DESC, {tiebreak} DESC LIMIT 1"
        ))
        .bind(&device_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            if over_ceiling {
                ApiError::too_many_requests()
            } else {
                ApiError::not_found()
            }
        })?,
    };
    tx.commit().await?;
    Ok((
        StatusCode::OK,
        [(CONTENT_TYPE, "application/octet-stream")],
        content,
    )
        .into_response())
}

/// `GET /v1/key-packages/count`: how many one-time key packages the caller's
/// device still has, and whether a last-resort package is stored. Clients
/// use this to know when to top up. The last-resort package is not part of
/// `count`.
pub async fn count(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let (count, last_resort): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM key_packages kp
             JOIN devices d ON d.id = kp.device_id
             WHERE kp.device_id = $1 AND d.revoked_at IS NULL AND kp.last_resort = 0),
            (SELECT COUNT(*) FROM key_packages kp
             JOIN devices d ON d.id = kp.device_id
             WHERE kp.device_id = $1 AND d.revoked_at IS NULL AND kp.last_resort = 1)",
    )
    .bind(&authed.device_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(
        json!({ "count": count, "last_resort": last_resort > 0 }),
    ))
}

/// `GET /v1/users/{username}/key-package-devices`: the device ids of a user
/// that currently have at least one unconsumed key package. Anyone adding a
/// user to a conversation needs one key package per device.
pub async fn devices_with_key_packages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth::authenticate(&state, &headers).await?;
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT d.id FROM devices d
         JOIN users u ON u.id = d.user_id
         WHERE lower(u.username) = lower($1) AND d.revoked_at IS NULL
           AND EXISTS (SELECT 1 FROM key_packages kp WHERE kp.device_id = d.id)
         ORDER BY d.created_at",
    )
    .bind(&username)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        json!({ "devices": rows.into_iter().map(|(id,)| id).collect::<Vec<_>>() }),
    ))
}
