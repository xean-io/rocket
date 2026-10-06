//! Bearer-token guard for the TCP listener (Go: `daemon.RequireToken`). The
//! unix socket is not guarded: its 0600 mode is the access control.

use crate::error::json_response;
use crate::json;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use rocket_domain::api::ErrorBody;
use std::sync::Arc;

/// Constant-time equality (Go `subtle.ConstantTimeCompare`): only the length
/// is allowed to short-circuit.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Rejects every request lacking `Authorization: Bearer <token>`: all
/// methods, all paths (including unknown ones) and SSE streams.
pub fn require_token(router: Router, token: &str) -> Router {
    let token: Arc<[u8]> = Arc::from(token.as_bytes());
    router.layer(middleware::from_fn_with_state(token, guard))
}

async fn guard(State(token): State<Arc<[u8]>>, req: Request, next: Next) -> Response {
    let authorized = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.as_bytes().strip_prefix(b"Bearer "))
        .is_some_and(|got| ct_eq(got, &token));
    if authorized {
        return next.run(req).await;
    }
    let mut resp = json_response(
        StatusCode::UNAUTHORIZED,
        json::compact_html_line(&ErrorBody {
            error: "missing or invalid bearer token (see daemon.json)".into(),
            code: "unauthorized".into(),
        }),
    );
    resp.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Bearer realm=\"rocketd\""),
    );
    resp
}
