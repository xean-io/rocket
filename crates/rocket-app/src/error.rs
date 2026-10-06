//! Application errors (Go: the `ErrInvalid`/`ErrNotFound`/... sentinels).
//!
//! Go wraps the sentinels with `fmt.Errorf("%w: detail", ErrX)`, so the text
//! the daemon puts in the API `error` field starts with the sentinel text
//! (`invalid request: ...`). The `Display` impl reproduces that, and
//! [`AppError::code`] is the API `code` field the Go server derives with
//! `errors.Is` in `internal/adapters/api/server.go` (`writeErr`).

use rocket_domain::ports;

/// Result alias for use cases.
pub type Result<T> = std::result::Result<T, AppError>;

/// An error returned by a use case.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// Go `ErrInvalid` (HTTP 400, code `invalid`).
    #[error("invalid request: {0}")]
    Invalid(String),
    /// Go `ErrNotFound` (HTTP 404, code `not_found`).
    #[error("not found: {0}")]
    NotFound(String),
    /// Go `ErrConflict` (HTTP 409, code `conflict`).
    #[error("conflict: {0}")]
    Conflict(String),
    /// Go `ErrConfirmation` (HTTP 428, code `confirmation_required`).
    #[error("confirmation required: {0}")]
    ConfirmationRequired(String),
    /// Go's `context.Canceled`: the operation was canceled through its
    /// [`CancellationToken`](tokio_util::sync::CancellationToken). Code `internal`.
    #[error("context canceled")]
    Canceled,
    /// Anything else (Go: an unwrapped error, HTTP 500, code `internal`).
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid(detail.into())
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::NotFound(detail.into())
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::Conflict(detail.into())
    }

    pub fn confirmation_required(detail: impl Into<String>) -> Self {
        Self::ConfirmationRequired(detail.into())
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal(detail.into())
    }

    /// The API error `code`: `invalid | not_found | conflict |
    /// confirmation_required | internal` (`unauthorized` is the API layer's).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::ConfirmationRequired(_) => "confirmation_required",
            Self::Canceled | Self::Internal(_) => "internal",
        }
    }

    /// The HTTP status the Go server pairs with [`code`](Self::code).
    pub fn http_status(&self) -> u16 {
        match self {
            Self::Invalid(_) => 400,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::ConfirmationRequired(_) => 428,
            Self::Canceled | Self::Internal(_) => 500,
        }
    }
}

impl From<ports::Error> for AppError {
    fn from(err: ports::Error) -> Self {
        Self::Internal(err.to_string())
    }
}
