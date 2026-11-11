//! Geist3 server: accounts, conversation membership and encrypted message relay.

use axum::{Router, routing::get};

/// Build the application router.
pub fn build_router() -> Router {
    Router::new().route("/healthz", get(healthz))
}

async fn healthz() -> &'static str {
    "ok"
}
