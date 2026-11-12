//! Data export and account deletion.

use argon2::password_hash::PasswordHash;
use argon2::{Argon2, PasswordVerifier};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
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

    // Messages in those conversations.
    let mut messages = Vec::new();
    for conv in &conv_rows {
        let conv_id: String = conv.get("id");

        let msg_rows = sqlx::query(
            r#"
            SELECT m.id, m.created_at, m.payload, u.username AS sender_name
            FROM messages m
            JOIN devices d ON d.id = m.sender_device
            JOIN users u ON u.id = d.user_id
            WHERE m.conversation_id = $1
            ORDER BY m.created_at
            "#,
        )
        .bind(&conv_id)
        .fetch_all(db)
        .await?;

        for row in &msg_rows {
            let payload_bytes: Vec<u8> = row.get("payload");
            messages.push(json!({
                "id": row.get::<String, _>("id"),
                "conversation_id": conv_id.clone(),
                "sender": row.get::<String, _>("sender_name"),
                "created_at": row.get::<i64, _>("created_at"),
                "payload": STANDARD.encode(payload_bytes),
            }));
        }
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
        "messages": messages,
        "blobs": blobs,
    });

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"thrum-export.json\"",
            ),
        ],
        Json(document),
    ))
}

/// `DELETE /v1/account`: password-confirmed account deletion.
///
/// Deletes in foreign-key-safe order: blobs, invites, messages, devices,
/// ownership transfer, user row, memberless conversations.
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
    let parsed = PasswordHash::new(&stored).map_err(|_| ApiError::internal())?;
    Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .map_err(|_| ApiError::unauthorized("invalid_credentials"))?;

    let db = &state.db;

    // 1. Delete blob objects from the store, then blob rows.
    let blob_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM blobs WHERE uploader_id = $1")
        .bind(user_id)
        .fetch_all(db)
        .await?;
    for blob_id in &blob_ids {
        let _ = state.blob_store.delete(blob_id).await;
    }
    sqlx::query("DELETE FROM blobs WHERE uploader_id = $1")
        .bind(user_id)
        .execute(db)
        .await?;

    // 2. Invites the user minted.
    sqlx::query("DELETE FROM invites WHERE created_by = $1")
        .bind(user_id)
        .execute(db)
        .await?;

    // 3. Messages sent by the user's devices (cascades deliveries).
    sqlx::query(
        "DELETE FROM messages WHERE sender_device IN (SELECT id FROM devices WHERE user_id = $1)",
    )
    .bind(user_id)
    .execute(db)
    .await?;

    // 4. Devices (cascade key packages, deliveries).
    sqlx::query("DELETE FROM devices WHERE user_id = $1")
        .bind(user_id)
        .execute(db)
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
    .execute(db)
    .await?;

    // 6. Delete the user row (cascade memberships, blocks).
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(db)
        .await?;

    // 7. Delete memberless conversations (cascade messages, invites).
    sqlx::query(
        r#"DELETE FROM conversations WHERE NOT EXISTS (
            SELECT 1 FROM conversation_members cm
            WHERE cm.conversation_id = conversations.id
        )"#,
    )
    .execute(db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

use axum::extract::State;

#[derive(Debug, serde::Deserialize)]
pub struct DeleteAccountBody {
    pub password: String,
}
