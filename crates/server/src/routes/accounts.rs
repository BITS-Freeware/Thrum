//! Account and session endpoints.

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use thrum_core::UserName;

use crate::account;
use crate::accounts;
use crate::auth;
use crate::error::ApiError;
use crate::ratelimit::Limit;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserRef {
    pub id: i64,
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
    /// An optional name the client gives the device it runs on.
    #[serde(default)]
    pub device_label: String,
}

/// `POST /v1/accounts`: create an account.
pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .limiter
        .check(addr.ip(), Limit::Register, state.cfg.register_per_minute)?;
    let name =
        UserName::new(&body.username).map_err(|_| ApiError::bad_request("invalid_username"))?;
    accounts::validate_password(&body.password)?;
    let user_id = accounts::create_user(&state.db, &name, &body.password).await?;
    Ok((
        StatusCode::CREATED,
        Json(UserRef {
            id: user_id,
            username: name.as_str().to_string(),
        }),
    ))
}

/// `POST /v1/sessions`: log in, creating a new device with its own token.
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<LoginBody>,
) -> Result<Json<accounts::Session>, ApiError> {
    state
        .limiter
        .check(addr.ip(), Limit::Login, state.cfg.login_per_minute)?;
    let session = accounts::login(
        &state.db,
        &body.username,
        &body.password,
        &body.device_label,
    )
    .await?;
    Ok(Json(session))
}

/// `DELETE /v1/sessions/current`: log out this device.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    sqlx::query("UPDATE devices SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL")
        .bind(accounts::now())
        .bind(&authed.device_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/account/export`: download a JSON export of the caller's data.
pub async fn export(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    account::export(state, authed).await
}

/// `DELETE /v1/account`: password-confirmed account deletion.
pub async fn delete_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<account::DeleteAccountBody>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    account::delete_account(State(state), authed, Json(body)).await
}
