//! Thrum server: accounts, device registry, key packages, message relay.

pub mod account;
pub mod accounts;
pub mod auth;
pub mod blobs;
pub mod client_ip;
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
use axum::body::Body;
use axum::http::{HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;
use state::AppState;

/// Content-Security-Policy for the API and the static web client.
///
/// `'wasm-unsafe-eval'` is required to compile the WebAssembly module.
/// Everything else stays on this origin.
const CONTENT_SECURITY_POLICY: &str = "\
default-src 'self'; \
script-src 'self' 'wasm-unsafe-eval'; \
style-src 'self'; \
img-src 'self' data:; \
connect-src 'self'; \
font-src 'self'; \
object-src 'none'; \
base-uri 'self'; \
frame-ancestors 'none'; \
form-action 'self'";

/// Build the application router for `state`.
pub fn build_router(state: AppState) -> Router {
    routes::routes(state)
}

/// Add the security headers every response should carry, including static
/// files served as a fallback beside the API routes.
pub fn with_security_headers(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(security_headers))
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        axum::http::header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    response
}

/// Serve the application on `listener` until it is stopped. Used by the
/// server binary and by test harnesses.
pub async fn serve(state: AppState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(
        listener,
        with_security_headers(routes::routes(state))
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
}
