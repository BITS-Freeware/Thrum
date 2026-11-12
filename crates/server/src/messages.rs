//! Conversation and message storage: fan-out, catch-up, expiry.

use thrum_core::protocol::ServerFrame;
use sqlx::AnyPool;
use uuid::Uuid;

use crate::accounts::now;
use crate::error::ApiError;
use crate::frames::encode_payload;

/// Create a conversation with `creator_id` (its admin) and every listed
/// member user id. Returns the conversation id.
pub async fn create_conversation(
    db: &AnyPool,
    creator_id: i64,
    member_ids: &[i64],
    name: &str,
) -> Result<String, sqlx::Error> {
    let conversation_id = Uuid::new_v4().to_string();
    let started_at = now();
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO conversations (id, created_at, name, owner_user_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(&conversation_id)
    .bind(started_at)
    .bind(name)
    .bind(creator_id)
    .execute(&mut *tx)
    .await?;
    for user_id in std::iter::once(creator_id).chain(member_ids.iter().copied()) {
        sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, joined_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(&conversation_id)
            .bind(user_id)
            .bind(started_at)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(conversation_id)
}

/// Store an opaque message and fan delivery rows out to every live device
/// of every member except the sending device.
///
/// Returns the message id and the device ids the message was queued for.
pub async fn store_message(
    db: &AnyPool,
    conversation_id: &str,
    sender_device: &str,
    payload: &[u8],
) -> Result<(String, Vec<String>), ApiError> {
    let message_id = Uuid::new_v4().to_string();
    let created_at = now();
    let internal = || ApiError::internal();
    let mut tx = db.begin().await.map_err(|_| internal())?;
    sqlx::query("INSERT INTO messages (id, conversation_id, sender_device, payload, created_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(&message_id)
        .bind(conversation_id)
        .bind(sender_device)
        .bind(payload)
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(|_| internal())?;
    let recipients = sqlx::query_as::<_, (String,)>(
        "SELECT d.id FROM conversation_members m
         JOIN devices d ON d.user_id = m.user_id
         WHERE m.conversation_id = $1 AND d.revoked_at IS NULL AND d.id != $2",
    )
    .bind(conversation_id)
    .bind(sender_device)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| internal())?;
    for (device_id,) in &recipients {
        sqlx::query(
            "INSERT INTO deliveries (message_id, device_id, queued_at) VALUES ($1, $2, $3)",
        )
        .bind(&message_id)
        .bind(device_id)
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(|_| internal())?;
    }
    tx.commit().await.map_err(|_| internal())?;
    Ok((
        message_id,
        recipients
            .into_iter()
            .map(|(device_id,)| device_id)
            .collect(),
    ))
}

/// Delete undelivered deliveries past their keep-time, then message rows
/// that no longer have any undelivered delivery. The server keeps no
/// delivered history: clients store what they need.
pub async fn cleanup_expired(db: &AnyPool, keep_older_than: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM deliveries WHERE delivered_at IS NULL AND queued_at < $1")
        .bind(keep_older_than)
        .execute(db)
        .await?;
    sqlx::query(
        "DELETE FROM messages WHERE NOT EXISTS (
             SELECT 1 FROM deliveries d
             WHERE d.message_id = messages.id AND d.delivered_at IS NULL
         )",
    )
    .execute(db)
    .await?;
    Ok(())
}

/// All undelivered messages for one device, oldest first, as frames.
pub async fn undelivered_for_device(
    db: &AnyPool,
    backend: crate::db::Backend,
    device_id: &str,
) -> Result<Vec<ServerFrame>, sqlx::Error> {
    // Stable catch-up order: creation time, then insertion order. SQLite
    // exposes insertion order as rowid; PostgreSQL uses the messages.seq
    // identity column from its migration set.
    let tiebreak = match backend {
        crate::db::Backend::Sqlite => "m.rowid",
        crate::db::Backend::Postgres => "m.seq",
    };
    let rows = sqlx::query_as::<_, (String, String, Vec<u8>, i64, String)>(&format!(
        "SELECT m.id, m.conversation_id, m.payload, m.created_at, u.username
         FROM deliveries d
         JOIN messages m ON m.id = d.message_id
         JOIN devices sd ON sd.id = m.sender_device
         JOIN users u ON u.id = sd.user_id
         WHERE d.device_id = $1 AND d.delivered_at IS NULL
         ORDER BY m.created_at, {tiebreak}"
    ))
    .bind(device_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(message_id, conversation_id, payload, created_at, sender_username)| {
                ServerFrame::Message {
                    conversation_id,
                    message_id,
                    sender_username,
                    payload_base64: encode_payload(&payload),
                    created_at,
                }
            },
        )
        .collect())
}

/// Mark one delivery delivered; true when a pending row was updated.
pub async fn mark_delivered(
    db: &AnyPool,
    message_id: &str,
    device_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE deliveries SET delivered_at = $1
         WHERE message_id = $2 AND device_id = $3 AND delivered_at IS NULL",
    )
    .bind(now())
    .bind(message_id)
    .bind(device_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}
