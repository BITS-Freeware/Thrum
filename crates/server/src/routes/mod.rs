//! HTTP routes.

pub mod accounts;
pub mod devices;
pub mod keypackages;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::state::AppState;

/// All application routes, bound to `state`.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/accounts", post(accounts::register))
        .route("/v1/sessions", post(accounts::login))
        .route("/v1/sessions/current", delete(accounts::logout))
        .route("/v1/devices", get(devices::list))
        .route("/v1/devices/{device_id}", delete(devices::revoke))
        .route(
            "/v1/devices/{device_id}/key-package",
            get(keypackages::fetch),
        )
        .route("/v1/key-packages", post(keypackages::upload))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}
