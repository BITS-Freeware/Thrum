//! Account creation and login: password handling and device sessions.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand::RngCore;
use sqlx::AnyPool;
use std::sync::OnceLock;
use thrum_core::UserName;
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
pub async fn create_user(db: &AnyPool, name: &UserName, password: &str) -> Result<i64, ApiError> {
    let password = password.to_string();
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| ApiError::internal())??;
    // A deleted account's name stays retired. The answer is the same as for
    // a name in use, so it does not tell anyone that an account was deleted.
    let retired: Option<String> =
        sqlx::query_scalar("SELECT username FROM retired_usernames WHERE username = $1")
            .bind(name.as_str())
            .fetch_optional(db)
            .await?;
    if retired.is_some() {
        return Err(ApiError::conflict("username_taken"));
    }
    let result = sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (username, password_hash, created_at) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name.as_str())
    .bind(&password_hash)
    .bind(now())
    .fetch_one(db)
    .await;
    match result {
        Ok(id) => Ok(id),
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

/// Check a password against a stored PHC hash string, off the async
/// workers: an Argon2 run takes long enough to stall other requests.
pub async fn check_password(password: &str, stored: &str) -> Result<(), ApiError> {
    let password = password.to_string();
    let stored = stored.to_string();
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&stored).map_err(|_| ApiError::internal())?;
        verify_password(&password, &parsed)
    })
    .await
    .map_err(|_| ApiError::internal())?
}

/// Check a password against a stored PHC hash string.
fn verify_password(password: &str, stored: &PasswordHash<'_>) -> Result<(), ApiError> {
    Argon2::default()
        .verify_password(password.as_bytes(), stored)
        .map_err(|_| ApiError::unauthorized("invalid_credentials"))
}

/// Create a device with its own session token for `user_id`.
pub async fn create_device(
    db: &AnyPool,
    user_id: i64,
    label: &str,
    max_devices: u32,
) -> Result<(String, String), ApiError> {
    let device_id = Uuid::new_v4().to_string();
    let token = auth::new_session_token();
    // Count and insert under a lock on the account row, so parallel logins
    // cannot pass the device cap together.
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE users SET created_at = created_at WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if max_devices > 0 {
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM devices WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if active >= i64::from(max_devices) {
            return Err(ApiError::conflict("too_many_devices"));
        }
    }
    sqlx::query("INSERT INTO devices (id, user_id, label, token_hash, created_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(&device_id)
        .bind(user_id)
        .bind(label)
        .bind(auth::token_hash(&token))
        .bind(now())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((device_id, token))
}

/// Longest device label a server accepts, in bytes.
pub const MAX_DEVICE_LABEL_BYTES: usize = 64;

/// The session created by a successful login.
#[derive(Debug, serde::Serialize)]
pub struct Session {
    pub token: String,
    pub device_id: String,
    pub username: String,
}

/// Verify credentials and start a new device session, unless the account
/// already has `max_devices` signed-in devices (zero means no cap).
///
/// Unknown names and wrong passwords return the same error and take the
/// same time, so logins do not reveal whether an account exists.
pub async fn login(
    db: &AnyPool,
    username: &str,
    password: &str,
    device_label: &str,
    max_devices: u32,
) -> Result<Session, ApiError> {
    if device_label.len() > MAX_DEVICE_LABEL_BYTES {
        return Err(ApiError::bad_request("device_label_too_long"));
    }
    let invalid = || ApiError::unauthorized("invalid_credentials");
    let name = UserName::new(username).map_err(|_| invalid())?;
    let row = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, password_hash FROM users WHERE lower(username) = lower($1)",
    )
    .bind(name.as_str())
    .fetch_optional(db)
    .await?;
    let Some((user_id, stored)) = row else {
        let _ = check_password(password, dummy_hash()).await;
        return Err(invalid());
    };
    check_password(password, &stored).await?;
    let (device_id, token) = create_device(db, user_id, device_label, max_devices).await?;
    Ok(Session {
        token,
        device_id,
        username: name.as_str().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Passwords must be stored as Argon2id PHC strings (threat model,
    /// curious server operator): a database leak must not hand out
    /// reversible or weakly hashed passwords.
    #[test]
    fn password_hashes_are_argon2id() {
        let hash = hash_password("correct horse battery staple").expect("hash");
        let parsed = PasswordHash::new(&hash).expect("parseable PHC string");
        assert_eq!(
            parsed.algorithm.as_str(),
            "argon2id",
            "stored hash must be argon2id, got {hash}"
        );
        verify_password("correct horse battery staple", &parsed).expect("verifies");
        assert!(verify_password("wrong password", &parsed).is_err());
        // Two hashes of the same password must not share a salt.
        let other = hash_password("correct horse battery staple").expect("hash");
        assert_ne!(hash, other, "salts must differ");
    }

    /// The dummy hash used for unknown usernames must be an Argon2id PHC
    /// string too, so the timing path is the same as a real login.
    #[test]
    fn dummy_hash_is_a_real_argon2id_string() {
        let parsed = PasswordHash::new(dummy_hash()).expect("parseable PHC string");
        assert_eq!(parsed.algorithm.as_str(), "argon2id");
        assert!(verify_password("anything", &parsed).is_err());
    }
}
