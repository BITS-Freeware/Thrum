//! HTTP routes.

pub mod accounts;
pub mod blobs;
pub mod blocks;
pub mod conversations;
pub mod devices;
pub mod invites;
pub mod keypackages;
pub mod members;
pub mod ws;

use axum::Router;
use axum::routing::{delete, get, patch, post};

use crate::state::AppState;

/// All application routes, bound to `state`.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/accounts", post(accounts::register))
        .route("/v1/sessions", post(accounts::login))
        .route("/v1/sessions/current", delete(accounts::logout))
        .route("/v1/account/export", get(accounts::export))
        .route("/v1/account", delete(accounts::delete_account))
        .route("/v1/devices", get(devices::list))
        .route("/v1/devices/{device_id}", delete(devices::revoke))
        .route(
            "/v1/devices/{device_id}/key-package",
            get(keypackages::fetch),
        )
        .route("/v1/key-packages", post(keypackages::upload))
        .route("/v1/key-packages/count", get(keypackages::count))
        .route(
            "/v1/users/{username}/key-package-devices",
            get(keypackages::devices_with_key_packages),
        )
        .route(
            "/v1/conversations/{conversation_id}/members",
            post(members::add_members),
        )
        .route(
            "/v1/conversations/{conversation_id}/members/{username}",
            delete(members::remove_member),
        )
        .route(
            "/v1/conversations",
            get(conversations::list).post(conversations::create),
        )
        .route(
            "/v1/conversations/{conversation_id}/messages",
            post(conversations::send_message),
        )
        .route(
            "/v1/conversations/{conversation_id}/name",
            patch(conversations::rename),
        )
        .route(
            "/v1/conversations/{conversation_id}/invites",
            post(invites::create),
        )
        .route(
            "/v1/conversations/{conversation_id}/invites/{token}",
            delete(invites::revoke),
        )
        .route("/v1/invites/redeem", post(invites::redeem))
        .route("/v1/blocks", post(blocks::block))
        .route("/v1/blocks/{username}", delete(blocks::unblock))
        // The upload cap lives in the handler so it can answer with the
        // stable `too_large` error code; the body limit only has to sit
        // above it so the handler, not axum, sees the oversized body.
        .route(
            "/v1/blobs",
            post(blobs::upload).layer(axum::extract::DefaultBodyLimit::max(
                state.cfg.attachment_max_bytes + 64 * 1024,
            )),
        )
        .route("/v1/blobs/{blob_id}", get(blobs::fetch))
        .route("/v1/ws", get(ws::ws_handler))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}
