//! Conversation membership changes on the server side.
//!
//! The MLS layer lives entirely in clients; the server only tracks which
//! users are in a conversation so it can fan payloads out to their devices.
//! The conversation's admin (its creator) decides membership.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;
use thrum_core::UserName;

use crate::auth;
use crate::error::ApiError;
use crate::routes::conversations::{
    blocked_among, conversation_info, group_too_large, lock_conversation, member_ids, require_admin,
};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AddMembersBody {
    /// Usernames to add to the conversation.
    pub members: Vec<String>,
}

/// `POST /v1/conversations/{conversation_id}/members`: add users to the
/// conversation. Admin only; a block between a newcomer and anyone in the
/// group refuses the add. Adding someone the admin removed earlier lifts
/// that removal.
pub async fn add_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    axum::Json(body): axum::Json<AddMembersBody>,
) -> Result<axum::response::Response, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    require_admin(&state, &conversation_id, authed.user_id).await?;

    let mut added = Vec::new();
    for raw in &body.members {
        let name = UserName::new(raw).map_err(|_| ApiError::bad_request("invalid_username"))?;
        let id =
            sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE lower(username) = lower($1)")
                .bind(name.as_str())
                .fetch_optional(&state.db)
                .await?
                .map(|(id,)| id)
                .ok_or_else(|| ApiError::bad_request("unknown_member"))?;
        if !added.contains(&id) {
            added.push(id);
        }
    }
    let current_ids = member_ids(&state, &conversation_id).await?;
    let newcomers: Vec<i64> = added
        .iter()
        .copied()
        .filter(|id| !current_ids.contains(id))
        .collect();
    blocked_among(&state, &newcomers, &current_ids).await?;

    // Count and insert under the conversation's lock, so overlapping adds
    // and redemptions cannot pass the cap together.
    let mut tx = state.db.begin().await?;
    if !lock_conversation(&mut tx, &conversation_id).await? {
        return Err(ApiError::not_found());
    }
    let current: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM conversation_members WHERE conversation_id = $1")
            .bind(&conversation_id)
            .fetch_all(&mut *tx)
            .await?;
    let extra = added.iter().filter(|id| !current.contains(id)).count();
    if group_too_large(state.cfg.max_group_members, current.len() + extra) {
        return Err(ApiError::bad_request("group_too_large"));
    }
    for user_id in added {
        sqlx::query(
            "DELETE FROM conversation_removals WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(&conversation_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO conversation_members (conversation_id, user_id, joined_at)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(&conversation_id)
        .bind(user_id)
        .bind(crate::accounts::now())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let info = conversation_info(&state, &conversation_id).await?;
    Ok((StatusCode::OK, Json(info)).into_response())
}

/// `DELETE /v1/conversations/{conversation_id}/members/{username}`: remove a
/// member. Admin only; the admin cannot remove themselves, so a group never
/// loses its admin through its own commands.
pub async fn remove_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation_id, username)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    require_admin(&state, &conversation_id, authed.user_id).await?;

    let name = UserName::new(&username).map_err(|_| ApiError::bad_request("invalid_username"))?;
    let user_id =
        sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE lower(username) = lower($1)")
            .bind(name.as_str())
            .fetch_optional(&state.db)
            .await?
            .map(|(id,)| id)
            .ok_or_else(ApiError::not_found)?;
    if user_id == authed.user_id {
        return Err(ApiError::bad_request("admin_cannot_leave"));
    }

    let removed =
        sqlx::query("DELETE FROM conversation_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(&conversation_id)
            .bind(user_id)
            .execute(&state.db)
            .await?
            .rows_affected();

    if removed == 0 {
        return Err(ApiError::not_found());
    }
    // Remember the removal, so an invite cannot undo it; only the admin
    // adding them again does.
    sqlx::query(
        "INSERT INTO conversation_removals (conversation_id, user_id, removed_at)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(&conversation_id)
    .bind(user_id)
    .bind(crate::accounts::now())
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
