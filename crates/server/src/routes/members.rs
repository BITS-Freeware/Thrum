//! Conversation membership changes on the server side.
//!
//! The MLS layer lives entirely in clients; the server only tracks which
//! users are in a conversation so it can fan payloads out to their devices.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use thrum_core::UserName;
use serde::Deserialize;

use crate::auth;
use crate::error::ApiError;
use crate::routes::conversations::{ConversationInfo, conversation_members};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AddMembersBody {
    /// Usernames to add to the conversation.
    pub members: Vec<String>,
}

/// `POST /v1/conversations/{conversation_id}/members`: add users to the
/// conversation. The caller must be a member.
pub async fn add_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    axum::Json(body): axum::Json<AddMembersBody>,
) -> Result<axum::response::Response, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    require_member(&state, &conversation_id, authed.user_id).await?;

    let mut added = Vec::new();
    for raw in &body.members {
        let name = UserName::new(raw).map_err(|_| ApiError::bad_request("invalid_username"))?;
        let id = sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE username = ?1")
            .bind(name.as_str())
            .fetch_optional(&state.db)
            .await?
            .map(|(id,)| id)
            .ok_or_else(|| ApiError::bad_request("unknown_member"))?;
        added.push(id);
    }

    for user_id in added {
        sqlx::query(
            "INSERT OR IGNORE INTO conversation_members (conversation_id, user_id, joined_at)
             VALUES (?1, ?2, ?3)",
        )
        .bind(&conversation_id)
        .bind(user_id)
        .bind(crate::accounts::now())
        .execute(&state.db)
        .await?;
    }

    let members = conversation_members(&state.db, &conversation_id).await?;
    Ok((
        StatusCode::OK,
        Json(ConversationInfo {
            id: conversation_id,
            members,
        }),
    )
        .into_response())
}

/// `DELETE /v1/conversations/{conversation_id}/members/{username}`: remove a
/// member (or leave, when removing yourself). The caller must be a member.
pub async fn remove_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation_id, username)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    require_member(&state, &conversation_id, authed.user_id).await?;

    let name = UserName::new(&username).map_err(|_| ApiError::bad_request("invalid_username"))?;
    let user_id = sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE username = ?1")
        .bind(name.as_str())
        .fetch_optional(&state.db)
        .await?
        .map(|(id,)| id)
        .ok_or_else(ApiError::not_found)?;

    let removed =
        sqlx::query("DELETE FROM conversation_members WHERE conversation_id = ?1 AND user_id = ?2")
            .bind(&conversation_id)
            .bind(user_id)
            .execute(&state.db)
            .await?
            .rows_affected();

    if removed == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Fail with 404 when `user_id` is not a member of the conversation.
async fn require_member(
    state: &AppState,
    conversation_id: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    let member = sqlx::query_as::<_, (i64,)>(
        "SELECT 1 FROM conversation_members WHERE conversation_id = ?1 AND user_id = ?2",
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    if member.is_none() {
        return Err(ApiError::not_found());
    }
    Ok(())
}
