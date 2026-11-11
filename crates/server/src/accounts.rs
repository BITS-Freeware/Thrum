//! Account creation and login: password handling and device sessions.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use thrum_core::UserName;
use rand::RngCore;
use sqlx::SqlitePool;
use std::sync::OnceLock;
use uuid::Uuid;

use crate::auth;
use crate::error::ApiError;

/// Minimum and maximum accepted password length in bytes.
pub const PASSWORD_MIN_LEN: usize = 8;
pub const PASSWORD_MAX_LEN: usize = 1024;

/// Unix time in seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs() as i64
}

/// Validate a password: length bounds, no NUL bytes.
pub fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < PASSWORD_MIN_LEN
        || password.len() > PASSWORD_MAX_LEN
        || password.contains('\0')
    {
        return Err(ApiError::bad_request("invalid_password"));
    }
    Ok(())
}

/// A random 16-byte salt, base64 encoded for the PHC string.
fn random_salt() -> SaltString {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    SaltString::encode_b64(&bytes[..]).expect("salt encoding")
}

/// Hash a password with Argon2id (default parameters of the argon2 crate).
fn hash_password(password: &str) -> Result<String, ApiError> {
    let salt = random_salt();
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// A PHC-format hash to burn time on when the username does not exist, so
/// that "unknown user" and "wrong password" take the same time. Generated
/// once at first use; it verifies no password anyone registers.
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = random_salt();
        Argon2::default()
            .hash_password(b"not a real password", &salt)
            .expect("dummy hash")
            .to_string()
    })
}

/// Insert a user with the given canonical name and password.
/// Fails with `username_taken` when the name is already in use.
pub async fn create_user(
    db: &SqlitePool,
    name: &UserName,
    password: &str,
) -> Result<i64, ApiError> {
    let password_hash = hash_password(password)?;
    let result =
        sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES (?1, ?2, ?3)")
            .bind(name.as_str())
            .bind(&password_hash)
            .bind(now())
            .execute(db)
            .await;
    match result {
        Ok(inserted) => Ok(inserted.last_insert_rowid()),
        Err(error) => {
            let unique = error
                .as_database_error()
                .is_some_and(|db_error| db_error.is_unique_violation());
            if unique {
                Err(ApiError::conflict("username_taken"))
            } else {
                Err(error.into())
            }
        }
    }
}

/// Check a password against a stored PHC hash string.
fn verify_password(password: &str, stored: &PasswordHash<'_>) -> Result<(), ApiError> {
    Argon2::default()
        .verify_password(password.as_bytes(), stored)
        .map_err(|_| ApiError::unauthorized("invalid_credentials"))
}

/// Create a device with its own session token for `user_id`.
pub async fn create_device(
    db: &SqlitePool,
    user_id: i64,
    label: &str,
) -> Result<(String, String), ApiError> {
    let device_id = Uuid::new_v4().to_string();
    let token = auth::new_session_token();
    sqlx::query("INSERT INTO devices (id, user_id, label, token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5)")
        .bind(&device_id)
        .bind(user_id)
        .bind(label)
        .bind(auth::token_hash(&token))
        .bind(now())
        .execute(db)
        .await?;
    Ok((device_id, token))
}

/// The session created by a successful login.
#[derive(Debug, serde::Serialize)]
pub struct Session {
    pub token: String,
    pub device_id: String,
    pub username: String,
}

/// Verify credentials and start a new device session.
///
/// Unknown names and wrong passwords return the same error and take the
/// same time, so logins do not reveal whether an account exists.
pub async fn login(
    db: &SqlitePool,
    username: &str,
    password: &str,
    device_label: &str,
) -> Result<Session, ApiError> {
    let invalid = || ApiError::unauthorized("invalid_credentials");
    let name = UserName::new(username).map_err(|_| invalid())?;
    let row = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, password_hash FROM users WHERE username = ?1",
    )
    .bind(name.as_str())
    .fetch_optional(db)
    .await?;
    let Some((user_id, stored)) = row else {
        let parsed = PasswordHash::new(dummy_hash()).map_err(|_| ApiError::internal())?;
        let _ = verify_password(password, &parsed);
        return Err(invalid());
    };
    let parsed = PasswordHash::new(&stored).map_err(|_| ApiError::internal())?;
    verify_password(password, &parsed)?;
    let (device_id, token) = create_device(db, user_id, device_label).await?;
    Ok(Session {
        token,
        device_id,
        username: name.as_str().to_string(),
    })
}
