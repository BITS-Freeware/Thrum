//! Encrypted attachment blob upload and fetch.
//!
//! Blobs are opaque ciphertext to the server. The uploader picks a fresh
//! 128-bit id before encrypting, because the per-file key inside the MLS
//! message is bound to that id as AES-GCM associated data. The id is a
//! bearer capability: it appears only inside encrypted messages, so anyone
//! who can name it may fetch the ciphertext, which is useless without the
//! key.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::Error as SqlxError;

use crate::accounts;
use crate::auth;
use crate::blobs;
use crate::error::ApiError;
use crate::state::AppState;

/// The request header carrying the client-chosen blob id.
const BLOB_ID_HEADER: &str = "x-thrum-blob-id";

/// `POST /v1/blobs`: store an encrypted blob under a client-chosen id.
///
/// The size cap and the per-user daily quota are operator configuration:
/// oversized uploads answer `413 too_large`, an exhausted daily quota
/// answers `429 quota_exceeded`.
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let raw_id = headers
        .get(BLOB_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("bad_blob_id"))?;
    let id =
        blobs::normalize_blob_id(raw_id).ok_or_else(|| ApiError::bad_request("bad_blob_id"))?;

    if body.is_empty() {
        return Err(ApiError::bad_request("empty_blob"));
    }
    if body.len() > state.cfg.attachment_max_bytes {
        return Err(ApiError::payload_too_large("too_large"));
    }

    let day_start = accounts::now() - accounts::now() % 86_400;
    let used = blobs::get_daily_bytes_since(&state.db, authed.user_id, day_start).await?;
    if used.saturating_add(body.len() as u64) > state.cfg.daily_upload_bytes {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "quota_exceeded",
        ));
    }

    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs WHERE id = $1")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    if taken > 0 {
        // Someone already registered this capability. Rejecting keeps one
        // user from replacing another user's blob.
        return Err(ApiError::conflict("blob_exists"));
    }

    state.blob_store.put(&id, &body).await.map_err(|error| {
        eprintln!(
            "blob store put failed for {}: {}",
            crate::blobs::log_ref(&id),
            crate::blobs::log_error(&error, &id)
        );
        ApiError::internal()
    })?;

    let expires_at = accounts::now() + state.cfg.blob_retention_seconds;
    match record_upload(
        &state,
        &id,
        authed.user_id,
        body.len(),
        day_start,
        expires_at,
    )
    .await
    {
        Ok(()) => {}
        Err(Recorded::OverQuota) => {
            let _ = state.blob_store.delete(&id).await;
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "quota_exceeded",
            ));
        }
        Err(Recorded::Failed(error)) => {
            // Do not leave an unreferenced object behind.
            let _ = state.blob_store.delete(&id).await;
            if matches!(error, SqlxError::Database(_)) {
                // Almost certainly the pre-check losing a race.
                return Err(ApiError::conflict("blob_exists"));
            }
            eprintln!(
                "blob insert failed for {}: {}",
                crate::blobs::log_ref(&id),
                crate::blobs::log_error(&error, &id)
            );
            return Err(error.into());
        }
    }

    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// Why an upload could not be recorded.
enum Recorded {
    OverQuota,
    Failed(SqlxError),
}

impl From<SqlxError> for Recorded {
    fn from(error: SqlxError) -> Self {
        Self::Failed(error)
    }
}

/// Check the day's quota again and insert the blob row, under a lock on
/// the uploader's account row, so parallel uploads cannot pass the quota
/// together. The check before storing the object only saves work.
async fn record_upload(
    state: &AppState,
    id: &str,
    user_id: i64,
    size: usize,
    day_start: i64,
    expires_at: i64,
) -> Result<(), Recorded> {
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET created_at = created_at WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let used: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(size), 0) AS BIGINT) FROM blobs
         WHERE uploader_id = $1 AND created_at >= $2",
    )
    .bind(user_id)
    .bind(day_start)
    .fetch_one(&mut *tx)
    .await?;
    if (used.max(0) as u64).saturating_add(size as u64) > state.cfg.daily_upload_bytes {
        return Err(Recorded::OverQuota);
    }
    sqlx::query(
        "INSERT INTO blobs (id, uploader_id, size, expires_at, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(size as i64)
    .bind(expires_at)
    .bind(accounts::now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `GET /v1/blobs/{id}`: fetch an encrypted blob by id. Any authenticated
/// device may fetch; ids travel only inside encrypted messages. Expired
/// blobs answer `404` exactly like unknown ones.
pub async fn fetch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
) -> Result<Response, ApiError> {
    auth::authenticate(&state, &headers).await?;
    let id = blobs::normalize_blob_id(&raw_id).ok_or_else(ApiError::not_found)?;

    let expires_at: (i64,) = sqlx::query_as("SELECT expires_at FROM blobs WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if expires_at.0 <= accounts::now() {
        return Err(ApiError::not_found());
    }

    let bytes = state
        .blob_store
        .get(&id)
        .await
        .map_err(|error| {
            eprintln!(
                "blob store get failed for {}: {}",
                crate::blobs::log_ref(&id),
                crate::blobs::log_error(&error, &id)
            );
            ApiError::internal()
        })?
        .ok_or_else(|| {
            // The row survived but the object did not; drop the dead row so
            // the next sweep does not trip over it either.
            eprintln!(
                "blob row {} has no stored object",
                crate::blobs::log_ref(&id)
            );
            ApiError::not_found()
        })?;

    Ok((
        StatusCode::OK,
        [("content-type", "application/octet-stream")],
        bytes,
    )
        .into_response())
}
