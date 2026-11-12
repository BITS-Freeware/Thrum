//! Geist3 server: accounts, device registry, key packages, message relay.

pub mod account;
pub mod accounts;
pub mod auth;
pub mod blobs;
pub mod config;
pub mod db;
pub mod error;
pub mod frames;
pub mod hub;
pub mod messages;
pub mod ratelimit;
pub mod routes;
pub mod state;

use axum::Router;
use state::AppState;

/// Build the application router for `state`.
pub fn build_router(state: AppState) -> Router {
    routes::routes(state)
}

/// Serve the application on `listener` until it is stopped. Used by the
/// server binary and by test harnesses.
pub async fn serve(state: AppState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(
        listener,
        routes::routes(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
}
