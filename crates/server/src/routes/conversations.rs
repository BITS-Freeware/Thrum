//! Conversation endpoints.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use geist3_core::UserName;
use geist3_core::protocol::ServerFrame;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth;
use crate::error::ApiError;
use crate::frames::encode_payload;
use crate::messages;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateConversationBody {
    /// Usernames to include besides the caller.
    pub members: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConversationInfo {
    pub id: String,
    pub members: Vec<String>,
}

/// `POST /v1/conversations`: create a conversation with the caller and the
/// listed members. All members must have accounts.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateConversationBody>,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let mut member_ids = Vec::new();
    for raw in &body.members {
        let name = UserName::new(raw).map_err(|_| ApiError::bad_request("invalid_username"))?;
        let id = sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE username = ?1")
            .bind(name.as_str())
            .fetch_optional(&state.db)
            .await?
            .map(|(id,)| id)
            .ok_or_else(|| ApiError::bad_request("unknown_member"))?;
        if id != authed.user_id {
            member_ids.push(id);
        }
    }
    member_ids.sort_unstable();
    member_ids.dedup();
    let conversation_id =
        messages::create_conversation(&state.db, authed.user_id, &member_ids).await?;
    let members = conversation_members(&state.db, &conversation_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(ConversationInfo {
            id: conversation_id,
            members,
        }),
    ))
}

/// `GET /v1/conversations`: the caller's conversations with their members.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConversationInfo>>, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let ids = sqlx::query_as::<_, (String,)>(
        "SELECT conversation_id FROM conversation_members WHERE user_id = ?1 ORDER BY conversation_id",
    )
    .bind(authed.user_id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for (conversation_id,) in ids {
        let members = conversation_members(&state.db, &conversation_id).await?;
        out.push(ConversationInfo {
            id: conversation_id,
            members,
        });
    }
    Ok(Json(out))
}

/// `POST /v1/conversations/{conversation_id}/messages`: store an opaque
/// payload and deliver it to the other member devices.
pub async fn send_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty_message"));
    }
    if body.len() > state.cfg.message_max_bytes {
        return Err(ApiError::payload_too_large("message_too_large"));
    }
    let member = sqlx::query_as::<_, (i64,)>(
        "SELECT 1 FROM conversation_members WHERE conversation_id = ?1 AND user_id = ?2",
    )
    .bind(&conversation_id)
    .bind(authed.user_id)
    .fetch_optional(&state.db)
    .await?;
    if member.is_none() {
        return Err(ApiError::not_found());
    }
    let (message_id, recipients) =
        messages::store_message(&state.db, &conversation_id, &authed.device_id, &body).await?;
    // Opportunistically drop expired undelivered messages.
    let keep_older_than = crate::accounts::now() - state.cfg.queue_ttl_seconds;
    let _ = messages::cleanup_expired(&state.db, keep_older_than).await;
    state.hub.broadcast(
        &recipients,
        &ServerFrame::Message {
            conversation_id: conversation_id.clone(),
            message_id: message_id.clone(),
            sender_username: authed.username.clone(),
            payload_base64: encode_payload(&body),
            created_at: crate::accounts::now(),
        },
    );
    Ok((StatusCode::CREATED, Json(json!({ "id": message_id }))))
}

/// Canonical member usernames of one conversation, sorted.
async fn conversation_members(
    db: &sqlx::SqlitePool,
    conversation_id: &str,
) -> Result<Vec<String>, ApiError> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT u.username FROM conversation_members m
         JOIN users u ON u.id = m.user_id
         WHERE m.conversation_id = ?1 ORDER BY u.username",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(username,)| username).collect())
}
