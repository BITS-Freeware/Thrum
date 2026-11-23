//! Data export and account deletion.

use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::StreamExt;
use serde_json::json;
use sqlx::Row;

use crate::accounts;
use crate::auth;
use crate::error::ApiError;
use crate::state::AppState;

/// `GET /v1/account/export` — download everything the server knows about the
/// caller as a single JSON document.  Message payloads are ciphertext.
pub async fn export(
    state: AppState,
    authed: auth::AuthedDevice,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = authed.user_id;
    let db = &state.db;

    // Account info.
    let user_row = sqlx::query("SELECT id, username, created_at FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await?;
    let account = json!({
        "id": user_row.get::<i64, _>("id"),
        "username": user_row.get::<String, _>("username"),
        "created_at": user_row.get::<i64, _>("created_at"),
    });

    // Devices.
    let devices_rows =
        sqlx::query("SELECT id, label, created_at, revoked_at FROM devices WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
    let devices: Vec<serde_json::Value> = devices_rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "label": r.get::<String, _>("label"),
                "created_at": r.get::<i64, _>("created_at"),
                "revoked_at": r.try_get::<Option<i64>, _>("revoked_at").ok().flatten(),
            })
        })
        .collect();

    // Conversations the user belongs to.
    let conv_rows = sqlx::query(
        r#"
        SELECT c.id, c.name, c.created_at, cm.joined_at
        FROM conversations c
        JOIN conversation_members cm ON cm.conversation_id = c.id
        WHERE cm.user_id = $1
        ORDER BY c.created_at
        "#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;

    let mut conversations = Vec::new();
    for conv in &conv_rows {
        let conv_id: String = conv.get("id");
        let name: String = conv.get("name");
        let created_at: i64 = conv.get("created_at");
        let joined_at: i64 = conv.get("joined_at");

        // Members list (usernames).
        let members = sqlx::query(
            r#"
            SELECT u.username
            FROM conversation_members cm2
            JOIN users u ON u.id = cm2.user_id
            WHERE cm2.conversation_id = $1
            ORDER BY cm2.joined_at
            "#,
        )
        .bind(&conv_id)
        .fetch_all(db)
        .await?;
        let members_list: Vec<String> = members.iter().map(|r| r.get("username")).collect();

        // Owner.
        let owner = sqlx::query(
            "SELECT u.username FROM conversations c2 JOIN users u ON u.id = c2.owner_user_id WHERE c2.id = $1",
        )
        .bind(&conv_id)
        .fetch_optional(db)
        .await?
        .map(|r| r.get::<String, _>("username"));

        conversations.push(json!({
            "id": conv_id,
            "name": name,
            "created_at": created_at,
            "joined_at": joined_at,
            "members": members_list,
            "owner": owner,
        }));
    }

    // Blob records.
    let blob_rows = sqlx::query(
        r#"SELECT id, size, created_at, expires_at FROM blobs WHERE uploader_id = $1 ORDER BY created_at"#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    let blobs: Vec<serde_json::Value> = blob_rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "size": r.get::<i64, _>("size"),
                "created_at": r.get::<i64, _>("created_at"),
                "expires_at": r.try_get::<Option<i64>, _>("expires_at").ok().flatten(),
            })
        })
        .collect();

    let document = json!({
        "format": "thrum-export",
        "version": 1,
        "exported_at": accounts::now(),
        "account": account,
        "devices": devices,
        "conversations": conversations,
        "blobs": blobs,
    });
    // The messages go last, one page at a time, so a long history is never
    // held in memory at once.
    let mut head = serde_json::to_string(&document).map_err(|_| ApiError::internal())?;
    head.pop(); // the closing brace
    head.push_str(",\"messages\":[");
    let pages = futures_util::stream::unfold(
        Some(ExportCursor {
            state,
            user_id,
            after: None,
            first: true,
        }),
        |cursor| async move {
            let mut cursor = cursor?;
            match export_page(&cursor.state, cursor.user_id, cursor.after).await {
                Err(error) => {
                    eprintln!("export page failed: {error}");
                    Some((Err(std::io::Error::other("export failed")), None))
                }
                Ok((page, next)) => {
                    let last = (page.len() as i64) < EXPORT_PAGE;
                    let mut chunk = String::new();
                    for message in page {
                        if !cursor.first {
                            chunk.push(',');
                        }
                        cursor.first = false;
                        chunk.push_str(&message.to_string());
                    }
                    if last {
                        chunk.push_str("]}");
                        Some((Ok(Bytes::from(chunk)), None))
                    } else {
                        cursor.after = next;
                        Some((Ok(Bytes::from(chunk)), Some(cursor)))
                    }
                }
            }
        },
    );
    let body = futures_util::stream::once(async move { Ok(Bytes::from(head)) }).chain(pages);

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"thrum-export.json\"",
            ),
        ],
        Body::from_stream(body),
    ))
}

/// Messages an export loads at once.
const EXPORT_PAGE: i64 = 100;

/// Where the export of one caller's messages has got to.
struct ExportCursor {
    state: AppState,
    user_id: i64,
    after: Option<(i64, i64)>,
    first: bool,
}

/// One page of the messages in the caller's conversations that the caller
/// sent or was sent (not what others were sent before the caller joined),
/// oldest first, after `after`.
async fn export_page(
    state: &AppState,
    user_id: i64,
    after: Option<(i64, i64)>,
) -> Result<(Vec<serde_json::Value>, Option<(i64, i64)>), sqlx::Error> {
    let tiebreak = match state.backend {
        crate::db::Backend::Sqlite => "m.rowid",
        crate::db::Backend::Postgres => "m.seq",
    };
    let (after_time, after_order) = after.unwrap_or((i64::MIN, i64::MIN));
    let rows = sqlx::query(&format!(
        r#"
        SELECT m.id, m.conversation_id, m.created_at, m.payload,
               COALESCE(m.sender_name, u.username) AS sender_name,
               CAST({tiebreak} AS BIGINT) AS ord
        FROM messages m
        JOIN conversation_members cm
          ON cm.conversation_id = m.conversation_id AND cm.user_id = $1
        JOIN devices d ON d.id = m.sender_device
        JOIN users u ON u.id = d.user_id
        WHERE (
              d.user_id = $1
              OR EXISTS (
                  SELECT 1 FROM deliveries dl
                  JOIN devices mine ON mine.id = dl.device_id
                  WHERE dl.message_id = m.id AND mine.user_id = $1
              )
          )
          AND (m.created_at > $2 OR (m.created_at = $2 AND {tiebreak} > $3))
        ORDER BY m.created_at, {tiebreak}
        LIMIT $4
        "#
    ))
    .bind(user_id)
    .bind(after_time)
    .bind(after_order)
    .bind(EXPORT_PAGE)
    .fetch_all(&state.db)
    .await?;
    let next = rows
        .last()
        .map(|row| (row.get::<i64, _>("created_at"), row.get::<i64, _>("ord")));
    let page = rows
        .iter()
        .map(|row| {
            let payload: Vec<u8> = row.get("payload");
            json!({
                "id": row.get::<String, _>("id"),
                "conversation_id": row.get::<String, _>("conversation_id"),
                "sender": row.get::<String, _>("sender_name"),
                "created_at": row.get::<i64, _>("created_at"),
                "payload": STANDARD.encode(payload),
            })
        })
        .collect();
    Ok((page, next))
}

/// `DELETE /v1/account`: password-confirmed account deletion.
///
/// Signs the account's devices out, then deletes in one transaction, in
/// foreign-key-safe order: blobs, invites, messages, devices, ownership
/// transfer, user row, memberless conversations. Blob objects go after
/// the commit (decision 0010).
pub async fn delete_account(
    State(state): State<AppState>,
    authed: auth::AuthedDevice,
    Json(body): Json<DeleteAccountBody>,
) -> Result<StatusCode, ApiError> {
    let user_id = authed.user_id;

    // Verify password — a live token alone is not enough.
    let user_row = sqlx::query("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    let stored: String = user_row.get("password_hash");
    accounts::check_password(&body.password, &stored).await?;

    // Sign every device out first, so no new request of this account
    // starts while the deletion runs, and close their sockets.
    let device_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM devices WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;
    sqlx::query("UPDATE devices SET revoked_at = $1, token_hash = 'revoked:' || id WHERE user_id = $2 AND revoked_at IS NULL")
        .bind(accounts::now())
        .bind(user_id)
        .execute(&state.db)
        .await?;
    state.hub.disconnect(&device_ids);

    // Every database step runs in one transaction, so a failure leaves the
    // account as it was. Requests of this account that were already
    // running finish before it starts or fail after it commits: SQLite has
    // one writer, and on PostgreSQL the account and device rows are locked
    // against the key-share locks that writes referencing them take.
    let mut tx = state.db.begin().await?;
    match state.backend {
        crate::db::Backend::Sqlite => {
            sqlx::query("UPDATE users SET created_at = created_at WHERE id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        crate::db::Backend::Postgres => {
            sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
            sqlx::query("SELECT id FROM devices WHERE user_id = $1 FOR UPDATE")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        }
    }
    let db = &mut *tx;

    // 1. Blob rows. Their objects are deleted after the commit; until the
    //    store confirms each one, a tombstone keeps its id from being
    //    claimed again and lets the sweeper retry.
    let blob_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM blobs WHERE uploader_id = $1")
        .bind(user_id)
        .fetch_all(&mut *db)
        .await?;
    sqlx::query(
        "INSERT INTO blob_tombstones (id, created_at)
         SELECT id, $2 FROM blobs WHERE uploader_id = $1
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .bind(accounts::now())
    .execute(&mut *db)
    .await?;
    sqlx::query("DELETE FROM blobs WHERE uploader_id = $1")
        .bind(user_id)
        .execute(&mut *db)
        .await?;
    // 2. Invites the user minted.
    sqlx::query("DELETE FROM invites WHERE created_by = $1")
        .bind(user_id)
        .execute(&mut *db)
        .await?;

    // 3. Messages sent by the user's devices. Those some member has not
    //    received yet may be commits that member needs, so they stay until
    //    delivered or expired, moved to the placeholder sender with the
    //    user's name kept for the frame. The rest go (cascades deliveries).
    sqlx::query(
        "UPDATE messages
         SET sender_device = 'departed',
             sender_name = (SELECT username FROM users WHERE id = $1)
         WHERE sender_device IN (SELECT id FROM devices WHERE user_id = $1)
           AND EXISTS (
               SELECT 1 FROM deliveries d
               JOIN devices r ON r.id = d.device_id
               WHERE d.message_id = messages.id
                 AND d.delivered_at IS NULL
                 AND r.user_id <> $1
           )",
    )
    .bind(user_id)
    .execute(&mut *db)
    .await?;
    sqlx::query(
        "DELETE FROM messages WHERE sender_device IN (SELECT id FROM devices WHERE user_id = $1)",
    )
    .bind(user_id)
    .execute(&mut *db)
    .await?;

    // 4. Devices (cascade key packages, deliveries).
    sqlx::query("DELETE FROM devices WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *db)
        .await?;

    // 5. Transfer ownership of conversations the user owned.
    //    Earliest remaining member takes it; NULL when no member remains.
    sqlx::query(
        r#"
        UPDATE conversations
        SET owner_user_id = (
            SELECT cm.user_id
            FROM conversation_members cm
            WHERE cm.conversation_id = conversations.id
              AND cm.user_id <> $1
            ORDER BY cm.joined_at
            LIMIT 1
        )
        WHERE owner_user_id = $1
        "#,
    )
    .bind(user_id)
    .execute(&mut *db)
    .await?;

    // 6. Retire the name, so nobody can register it and inherit the trust
    //    others placed in it, then delete the user row (cascade
    //    memberships, blocks).
    sqlx::query(
        "INSERT INTO retired_usernames (username, retired_at) \
         SELECT username, $2 FROM users WHERE id = $1 \
         ON CONFLICT (username) DO NOTHING",
    )
    .bind(user_id)
    .bind(accounts::now())
    .execute(&mut *db)
    .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&mut *db)
        .await?;

    // 7. Delete memberless conversations (cascade messages, invites).
    sqlx::query(
        r#"DELETE FROM conversations WHERE NOT EXISTS (
            SELECT 1 FROM conversation_members cm
            WHERE cm.conversation_id = conversations.id
        )"#,
    )
    .execute(&mut *db)
    .await?;

    tx.commit().await?;
    // A device that connected between the sign-out and the commit.
    state.hub.disconnect(&device_ids);

    for blob_id in &blob_ids {
        if crate::blobs::delete_object(state.blob_store.as_ref(), blob_id)
            .await
            .is_ok()
        {
            let _ = sqlx::query("DELETE FROM blob_tombstones WHERE id = $1")
                .bind(blob_id)
                .execute(&state.db)
                .await;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

use axum::Json;
use axum::extract::State;

#[derive(Debug, serde::Deserialize)]
pub struct DeleteAccountBody {
    pub password: String,
}
