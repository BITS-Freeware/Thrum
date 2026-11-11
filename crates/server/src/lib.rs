//! Thrum server: accounts, device registry, key packages, message relay.

pub mod accounts;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod ratelimit;
pub mod routes;
pub mod state;

use axum::Router;
use state::AppState;

/// Build the application router for `state`.
pub fn build_router(state: AppState) -> Router {
    routes::routes(state)
}
