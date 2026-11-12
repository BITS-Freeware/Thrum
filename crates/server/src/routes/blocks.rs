//! Blocking: one user refusing new shared contexts with another.
//!
//! A block prevents creating conversations that would include both users,
//! adding them together, and redeeming each other's invites. Existing
//! conversations keep working: leaving them would need group-wide MLS
//! changes and MLS already lets the blocked party read only what the group
//! sends.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use geist3_core::UserName;
use serde::Deserialize;

use crate::auth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct BlockBody {
    pub username: String,
}

/// `POST /v1/blocks`: block a user. Blocking yourself is meaningless.
pub async fn block(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BlockBody>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let name =
        UserName::new(&body.username).map_err(|_| ApiError::bad_request("invalid_username"))?;
    let target =
        sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE lower(username) = lower($1)")
            .bind(name.as_str())
            .fetch_optional(&state.db)
            .await?
            .map(|(id,)| id)
            .ok_or_else(ApiError::not_found)?;
    if target == authed.user_id {
        return Err(ApiError::bad_request("cannot_block_self"));
    }
    sqlx::query(
        "INSERT INTO blocks (user_id, blocked_user_id, created_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(authed.user_id)
    .bind(target)
    .bind(crate::accounts::now())
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/blocks/{username}`: lift a block.
pub async fn unblock(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let name = UserName::new(&username).map_err(|_| ApiError::bad_request("invalid_username"))?;
    let target =
        sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE lower(username) = lower($1)")
            .bind(name.as_str())
            .fetch_optional(&state.db)
            .await?
            .map(|(id,)| id)
            .ok_or_else(ApiError::not_found)?;
    sqlx::query("DELETE FROM blocks WHERE user_id = $1 AND blocked_user_id = $2")
        .bind(authed.user_id)
        .bind(target)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
