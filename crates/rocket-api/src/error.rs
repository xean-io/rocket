//! `{error, code}` bodies and the AppError -> HTTP status mapping.

use crate::json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rocket_app::AppError;
use rocket_domain::api::ErrorBody;

/// A use-case error on its way out as an HTTP response (Go: `writeErr`).
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        json_response(
            status,
            json::pretty(&ErrorBody {
                error: self.0.to_string(),
                code: self.0.code().to_string(),
            }),
        )
    }
}

/// A response with an `application/json` body.
pub(crate) fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Go `http.Error`: plain text with a trailing newline and `nosniff`.
pub(crate) fn text_response(status: StatusCode, text: &str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        format!("{text}\n"),
    )
        .into_response()
}
