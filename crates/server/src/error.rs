//! HTTP error responses.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// An error that maps directly to an HTTP response with a JSON body of the
/// form `{"error": "<code>"}`. Error codes are stable strings clients can
/// switch on; they never include secrets or user input.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }

    pub fn bad_request(code: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code)
    }

    pub fn unauthorized(code: &'static str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code)
    }

    pub fn conflict(code: &'static str) -> Self {
        Self::new(StatusCode::CONFLICT, code)
    }

    pub fn forbidden(code: &'static str) -> Self {
        Self::new(StatusCode::FORBIDDEN, code)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found")
    }

    pub fn payload_too_large(code: &'static str) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, code)
    }

    pub fn too_many_requests() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    }

    pub fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.code }))).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self::internal()
    }
}

impl From<argon2::password_hash::Error> for ApiError {
    fn from(_: argon2::password_hash::Error) -> Self {
        Self::internal()
    }
}
