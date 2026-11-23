//! Conversation endpoints: create, list, rename, send.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thrum_core::UserName;
use thrum_core::protocol::ServerFrame;

use crate::auth;
use crate::error::ApiError;
use crate::frames::encode_payload;
use crate::messages;
use crate::state::AppState;

/// Longest group name a server accepts.
pub const MAX_NAME_BYTES: usize = 256;

#[derive(Debug, Deserialize)]
pub struct CreateConversationBody {
    /// Usernames to include besides the caller.
    pub members: Vec<String>,
    /// Optional group name, visible to the server as routing metadata.
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct ConversationInfo {
    pub id: String,
    pub name: String,
    /// Username of the conversation admin (the creator).
    pub admin: String,
    pub members: Vec<String>,
}

/// `POST /v1/conversations`: create a conversation with the caller as its
/// admin and the listed members. A block between any two of them refuses
/// the creation.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateConversationBody>,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let name = validate_name(&body.name)?;
    let mut member_ids = Vec::new();
    for raw in &body.members {
        let name_part =
            UserName::new(raw).map_err(|_| ApiError::bad_request("invalid_username"))?;
        let id =
            sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE lower(username) = lower($1)")
                .bind(name_part.as_str())
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
    let mut everyone = member_ids.clone();
    everyone.push(authed.user_id);
    blocked_among(&state, &everyone, &[]).await?;
    if group_too_large(state.cfg.max_group_members, member_ids.len() + 1) {
        return Err(ApiError::bad_request("group_too_large"));
    }
    let conversation_id =
        messages::create_conversation(&state.db, authed.user_id, &member_ids, &name).await?;
    Ok((
        StatusCode::CREATED,
        Json(conversation_info(&state, &conversation_id).await?),
    ))
}

/// `PATCH /v1/conversations/{conversation_id}/name`: rename the group.
/// Admin only.
pub async fn rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(body): Json<RenameBody>,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let name = validate_name(&body.name)?;
    require_admin(&state, &conversation_id, authed.user_id).await?;
    sqlx::query("UPDATE conversations SET name = $1 WHERE id = $2")
        .bind(&name)
        .bind(&conversation_id)
        .execute(&state.db)
        .await?;
    Ok(Json(conversation_info(&state, &conversation_id).await?))
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub name: String,
}

/// `GET /v1/conversations`: the caller's conversations with their members.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConversationInfo>>, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let ids = sqlx::query_as::<_, (String,)>(
        "SELECT conversation_id FROM conversation_members WHERE user_id = $1 ORDER BY conversation_id",
    )
    .bind(authed.user_id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for (conversation_id,) in ids {
        out.push(conversation_info(&state, &conversation_id).await?);
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
    state.limiter.check_keyed(
        crate::ratelimit::Limit::Send,
        &authed.device_id,
        state.cfg.messages_per_device_per_minute,
    )?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty_message"));
    }
    if body.len() > state.cfg.message_max_bytes {
        return Err(ApiError::payload_too_large("message_too_large"));
    }
    let member = sqlx::query_as::<_, (i64,)>(
        "SELECT 1 FROM conversation_members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(&conversation_id)
    .bind(authed.user_id)
    .fetch_optional(&state.db)
    .await?;
    if member.is_none() {
        return Err(ApiError::not_found());
    }
    let (message_id, recipients) = messages::store_message(
        &state.db,
        &conversation_id,
        (authed.user_id, &authed.device_id),
        &body,
        state.cfg.queued_bytes_per_account,
    )
    .await?;
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

/// True when `size` is over the operator's member cap. A cap of zero means
/// there is no limit.
pub(crate) fn group_too_large(max_members: u32, size: usize) -> bool {
    max_members > 0 && size > max_members as usize
}

/// Validate a group name: bounded length, no control characters.
fn validate_name(raw: &str) -> Result<String, ApiError> {
    if raw.len() > MAX_NAME_BYTES {
        return Err(ApiError::bad_request("name_too_long"));
    }
    if raw.chars().any(char::is_control) {
        return Err(ApiError::bad_request("invalid_name"));
    }
    Ok(raw.trim().to_string())
}

/// Fail with 403 when any newcomer and any other person in the resulting
/// group block each other, either way, or two newcomers do (decision 0007).
pub(crate) async fn blocked_among(
    state: &AppState,
    newcomers: &[i64],
    others: &[i64],
) -> Result<(), ApiError> {
    if newcomers.is_empty() {
        return Ok(());
    }
    let placeholders = (1..=newcomers.len())
        .map(|n| format!("${n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT user_id, blocked_user_id FROM blocks
         WHERE user_id IN ({placeholders}) OR blocked_user_id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, (i64, i64)>(&sql);
    for id in newcomers {
        query = query.bind(id);
    }
    let pairs = query.fetch_all(&state.db).await?;
    let in_group = |id: i64| newcomers.contains(&id) || others.contains(&id);
    let joins_newcomer = |a: i64, b: i64| {
        (newcomers.contains(&a) && in_group(b)) || (newcomers.contains(&b) && in_group(a))
    };
    if pairs.iter().any(|&(a, b)| a != b && joins_newcomer(a, b)) {
        return Err(ApiError::forbidden("blocked"));
    }
    Ok(())
}

/// Hold the conversation's row for the rest of `tx`, so membership
/// changes that overlap are counted one after another. PostgreSQL locks the
/// row; SQLite takes its write lock at this first write. False when there
/// is no such conversation.
pub(crate) async fn lock_conversation(
    tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    conversation_id: &str,
) -> Result<bool, sqlx::Error> {
    Ok(
        sqlx::query("UPDATE conversations SET name = name WHERE id = $1")
            .bind(conversation_id)
            .execute(&mut **tx)
            .await?
            .rows_affected()
            > 0,
    )
}

/// The user ids of a conversation's members.
pub(crate) async fn member_ids(
    state: &AppState,
    conversation_id: &str,
) -> Result<Vec<i64>, ApiError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM conversation_members WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .fetch_all(&state.db)
    .await?)
}

/// Fail with 403 when `user_id` is not the conversation's admin.
pub(crate) async fn require_admin(
    state: &AppState,
    conversation_id: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    let owner =
        sqlx::query_as::<_, (i64,)>("SELECT owner_user_id FROM conversations WHERE id = $1")
            .bind(conversation_id)
            .fetch_optional(&state.db)
            .await?
            .map(|(owner,)| owner);
    match owner {
        Some(owner) if owner == user_id => Ok(()),
        Some(_) => Err(ApiError::forbidden("not_admin")),
        None => Err(ApiError::not_found()),
    }
}

/// One conversation's full info: name, admin, sorted member usernames.
pub(crate) async fn conversation_info(
    state: &AppState,
    conversation_id: &str,
) -> Result<ConversationInfo, ApiError> {
    let (name, owner_id) = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT name, owner_user_id FROM conversations WHERE id = $1",
    )
    .bind(conversation_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let admin = match owner_id {
        Some(owner_id) => {
            sqlx::query_as::<_, (String,)>("SELECT username FROM users WHERE id = $1")
                .bind(owner_id)
                .fetch_optional(&state.db)
                .await?
                .map(|(username,)| username)
                .unwrap_or_default()
        }
        None => String::new(),
    };
    let members = conversation_members(&state.db, conversation_id).await?;
    Ok(ConversationInfo {
        id: conversation_id.to_string(),
        name,
        admin,
        members,
    })
}

/// Canonical member usernames of one conversation, sorted.
pub(crate) async fn conversation_members(
    db: &sqlx::AnyPool,
    conversation_id: &str,
) -> Result<Vec<String>, ApiError> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT u.username FROM conversation_members m
         JOIN users u ON u.id = m.user_id
         WHERE m.conversation_id = $1 ORDER BY u.username",
    )
    .bind(conversation_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(username,)| username).collect())
}
