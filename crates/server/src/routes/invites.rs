//! Invite links: tokens that let a registered user join a conversation.
//!
//! The admin of a conversation mints an invite token, hands it out out of
//! band, and can revoke it. The token itself is shown once and stored only
//! as a SHA-256 hash, like session tokens. Redeeming an invite adds the
//! caller to the server's membership list; a member's device then pulls the
//! new device into the MLS group on its next sync, which is where the
//! cryptography happens.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::auth;
use crate::error::ApiError;
use crate::routes::conversations::{blocked_between, conversation_info, require_admin};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RedeemBody {
    pub token: String,
}

/// `POST /v1/conversations/{conversation_id}/invites`: mint an invite token.
/// Admin only.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    require_admin(&state, &conversation_id, authed.user_id).await?;
    let token = auth::new_session_token();
    sqlx::query(
        "INSERT INTO invites (id, conversation_id, token_hash, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&conversation_id)
    .bind(auth::token_hash(&token))
    .bind(authed.user_id)
    .bind(crate::accounts::now())
    .execute(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "token": token }))))
}

/// `DELETE /v1/conversations/{conversation_id}/invites/{token}`: revoke an
/// invite token. Admin only; revoking an unknown or already-revoked token
/// is still 204 so clients cannot probe which tokens existed.
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation_id, token)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    require_admin(&state, &conversation_id, authed.user_id).await?;
    sqlx::query(
        "UPDATE invites SET revoked_at = ?1
         WHERE conversation_id = ?2 AND token_hash = ?3 AND revoked_at IS NULL",
    )
    .bind(crate::accounts::now())
    .bind(&conversation_id)
    .bind(auth::token_hash(&token))
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/invites/redeem`: join the conversation the token points to.
/// The caller must not be blocked by the invite's creator (or block them).
pub async fn redeem(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RedeemBody>,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let row = sqlx::query_as::<_, (String, i64)>(
        "SELECT conversation_id, created_by FROM invites
         WHERE token_hash = ?1 AND revoked_at IS NULL",
    )
    .bind(auth::token_hash(&body.token))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let (conversation_id, created_by) = row;
    blocked_between(&state, authed.user_id, created_by).await?;

    sqlx::query(
        "INSERT OR IGNORE INTO conversation_members (conversation_id, user_id, joined_at)
         VALUES (?1, ?2, ?3)",
    )
    .bind(&conversation_id)
    .bind(authed.user_id)
    .bind(crate::accounts::now())
    .execute(&state.db)
    .await?;
    Ok((
        StatusCode::OK,
        Json(conversation_info(&state, &conversation_id).await?),
    ))
}
