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
use crate::client_ip::{self, TrustedNet};
use crate::error::ApiError;
use crate::ratelimit::{Limit, account_key};
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
    headers: HeaderMap,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, ApiError> {
    state.limiter.check(
        client_addr(&state, addr.ip(), &headers),
        Limit::Register,
        state.cfg.register_per_minute,
    )?;
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
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<Json<accounts::Session>, ApiError> {
    state.limiter.check(
        client_addr(&state, addr.ip(), &headers),
        Limit::Login,
        state.cfg.login_per_minute,
    )?;
    let client = client_addr(&state, addr.ip(), &headers);
    let account = account_key(&body.username);
    state.limiter.check_login(&account, client, &state.cfg)?;
    match accounts::login(
        &state.db,
        &body.username,
        &body.password,
        &body.device_label,
        state.cfg.max_devices_per_account,
    )
    .await
    {
        Ok(session) => {
            state.limiter.clear_login_failures(&account, client);
            Ok(Json(session))
        }
        Err(error) if error.code == "invalid_credentials" => {
            state
                .limiter
                .record_login_failure(&account, client, &state.cfg);
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// The address rate limits see: the socket address, or the forwarded client
/// when the socket is a configured trusted proxy.
fn client_addr(state: &AppState, peer: std::net::IpAddr, headers: &HeaderMap) -> std::net::IpAddr {
    let trusted: Vec<TrustedNet> = state
        .cfg
        .trusted_proxies
        .iter()
        .filter_map(|entry| TrustedNet::parse(entry))
        .collect();
    client_ip::client_ip(peer, headers, &trusted)
}

/// `DELETE /v1/sessions/current`: log out this device.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    sqlx::query("UPDATE devices SET revoked_at = $1, token_hash = 'revoked:' || id WHERE id = $2 AND revoked_at IS NULL")
        .bind(accounts::now())
        .bind(&authed.device_id)
        .execute(&state.db)
        .await?;
    state.hub.disconnect(&[authed.device_id]);
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
///
/// The password check counts toward the same per-address lockout and
/// account ceiling as login, so a stolen token is not an unlimited password
/// oracle.
pub async fn delete_account(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<account::DeleteAccountBody>,
) -> Result<StatusCode, ApiError> {
    let authed = auth::authenticate(&state, &headers).await?;
    let client = client_addr(&state, addr.ip(), &headers);
    let account = account_key(&authed.username);
    state.limiter.check_login(&account, client, &state.cfg)?;
    let limiter = state.limiter.clone();
    let cfg = state.cfg.clone();
    let result = account::delete_account(State(state), authed, Json(body)).await;
    if matches!(&result, Err(error) if error.code == "invalid_credentials") {
        limiter.record_login_failure(&account, client, &cfg);
    }
    result
}
