//! Session token generation and bearer authentication.

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::state::AppState;

/// Resolve a raw bearer token to a live device. Shared by the HTTP bearer
/// path and the WebSocket auth frame.
pub async fn authenticate_token(state: &AppState, token: &str) -> Result<AuthedDevice, ApiError> {
    let hash = token_hash(token);
    let row = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT u.id, u.username, d.id
         FROM devices d JOIN users u ON u.id = d.user_id
         WHERE d.token_hash = ?1 AND d.revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::unauthorized("invalid_token"))?;
    Ok(AuthedDevice {
        user_id: row.0,
        username: row.1,
        device_id: row.2,
    })
}

/// The authenticated caller: one device of one user.
#[derive(Debug, Clone)]
pub struct AuthedDevice {
    pub user_id: i64,
    pub username: String,
    pub device_id: String,
}

/// Generate a new 256-bit session token, hex encoded.
pub fn new_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex_encode(&bytes)
}

/// SHA-256 of a session token. Tokens are stored hashed, never in plaintext.
pub fn token_hash(token: &str) -> String {
    hex_encode(&Sha256::digest(token.as_bytes()))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Resolve the `Authorization: Bearer <token>` header to a live device.
pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthedDevice, ApiError> {
    let token = bearer_token(headers)?;
    authenticate_token(state, &token).await
}

fn bearer_token(headers: &HeaderMap) -> Result<String, ApiError> {
    let value = headers
        .get(AUTHORIZATION)
        .ok_or_else(|| ApiError::unauthorized("missing_token"))?
        .to_str()
        .map_err(|_| ApiError::unauthorized("missing_token"))?;
    value
        .strip_prefix("Bearer ")
        .map(str::to_string)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::unauthorized("missing_token"))
}
