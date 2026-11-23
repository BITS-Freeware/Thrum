//! Conversation and message storage: fan-out, catch-up, expiry.

use sqlx::AnyPool;
use thrum_core::protocol::ServerFrame;
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
/// `queue_cap` bounds the bytes of the sending account's messages that
/// some device has not received yet, this one included; zero means no cap.
/// The check runs under a lock on the sender's account row, so parallel
/// sends cannot pass it together.
///
/// Returns the message id and the device ids the message was queued for.
pub async fn store_message(
    db: &AnyPool,
    conversation_id: &str,
    sender: (i64, &str),
    payload: &[u8],
    queue_cap: u64,
) -> Result<(String, Vec<String>), ApiError> {
    let (sender_user, sender_device) = sender;
    let message_id = Uuid::new_v4().to_string();
    let created_at = now();
    let internal = || ApiError::internal();
    let mut tx = db.begin().await.map_err(|_| internal())?;
    if queue_cap > 0 {
        sqlx::query("UPDATE users SET created_at = created_at WHERE id = $1")
            .bind(sender_user)
            .execute(&mut *tx)
            .await
            .map_err(|_| internal())?;
        let queued: i64 = sqlx::query_scalar(
            "SELECT CAST(COALESCE(SUM(LENGTH(m.payload)), 0) AS BIGINT)
             FROM messages m JOIN devices d ON d.id = m.sender_device
             WHERE d.user_id = $1
               AND EXISTS (
                   SELECT 1 FROM deliveries dl
                   WHERE dl.message_id = m.id AND dl.delivered_at IS NULL
               )",
        )
        .bind(sender_user)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| internal())?;
        if (queued.max(0) as u64).saturating_add(payload.len() as u64) > queue_cap {
            return Err(ApiError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "queue_full",
            ));
        }
    }
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

/// How many queued messages catch-up loads at once.
pub const CATCH_UP_PAGE: i64 = 100;

/// Where a catch-up page ended: the last message's creation time and
/// insertion order. The next page starts after it.
pub type QueueCursor = (i64, i64);

/// One page of a device's undelivered messages, oldest first, after
/// `after`, as frames, with the cursor of the page's last message.
pub async fn undelivered_page(
    db: &AnyPool,
    backend: crate::db::Backend,
    device_id: &str,
    after: Option<QueueCursor>,
) -> Result<(Vec<ServerFrame>, Option<QueueCursor>), sqlx::Error> {
    // Stable catch-up order: creation time, then insertion order. SQLite
    // exposes insertion order as rowid; PostgreSQL uses the messages.seq
    // identity column from its migration set.
    let tiebreak = match backend {
        crate::db::Backend::Sqlite => "m.rowid",
        crate::db::Backend::Postgres => "m.seq",
    };
    let (after_time, after_order) = after.unwrap_or((i64::MIN, i64::MIN));
    let rows = sqlx::query_as::<_, (String, String, Vec<u8>, i64, String, i64)>(&format!(
        "SELECT m.id, m.conversation_id, m.payload, m.created_at,
                COALESCE(m.sender_name, u.username), CAST({tiebreak} AS BIGINT)
         FROM deliveries d
         JOIN messages m ON m.id = d.message_id
         JOIN devices sd ON sd.id = m.sender_device
         JOIN users u ON u.id = sd.user_id
         WHERE d.device_id = $1 AND d.delivered_at IS NULL
           AND (m.created_at > $2 OR (m.created_at = $2 AND {tiebreak} > $3))
         ORDER BY m.created_at, {tiebreak}
         LIMIT $4"
    ))
    .bind(device_id)
    .bind(after_time)
    .bind(after_order)
    .bind(CATCH_UP_PAGE)
    .fetch_all(db)
    .await?;
    let cursor = rows.last().map(|row| (row.3, row.5));
    let frames = rows
        .into_iter()
        .map(
            |(message_id, conversation_id, payload, created_at, sender_username, _)| {
                ServerFrame::Message {
                    conversation_id,
                    message_id,
                    sender_username,
                    payload_base64: encode_payload(&payload),
                    created_at,
                }
            },
        )
        .collect();
    Ok((frames, cursor))
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
