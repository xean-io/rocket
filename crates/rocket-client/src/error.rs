use std::path::PathBuf;
use std::time::Duration;

/// The `code` field of an API error body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    /// 400: bad body, unknown service/group/env, invalid ttl or rocket.yaml.
    Invalid,
    /// 401: TCP listener only; missing or wrong bearer token.
    Unauthorized,
    /// 404: unknown project, service or job.
    NotFound,
    /// 409: name registered elsewhere, project still running.
    Conflict,
    /// 428: deploy refused until confirmed (the CLI exits 3).
    ConfirmationRequired,
    /// 500.
    Internal,
    /// Any other wire value (empty when the body carried none).
    Other(String),
}

impl ErrorCode {
    pub fn from_wire(code: &str) -> Self {
        match code {
            "invalid" => Self::Invalid,
            "unauthorized" => Self::Unauthorized,
            "not_found" => Self::NotFound,
            "conflict" => Self::Conflict,
            "confirmation_required" => Self::ConfirmationRequired,
            "internal" => Self::Internal,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Invalid => "invalid",
            Self::Unauthorized => "unauthorized",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::ConfirmationRequired => "confirmation_required",
            Self::Internal => "internal",
            Self::Other(s) => s,
        }
    }

    /// The documented code for an HTTP status, used when a JSON error body
    /// has an empty `code`.
    pub(crate) fn from_status(status: u16) -> Self {
        match status {
            400 => Self::Invalid,
            401 => Self::Unauthorized,
            404 => Self::NotFound,
            409 => Self::Conflict,
            428 => Self::ConfirmationRequired,
            500 => Self::Internal,
            _ => Self::Other(String::new()),
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// A non-2xx daemon response.
    #[error("{message}")]
    Api {
        status: u16,
        code: ErrorCode,
        message: String,
    },
    /// The daemon could not be reached (socket missing, connection refused).
    #[error("cannot connect to rocketd at {target}: {source}")]
    Connect {
        target: String,
        #[source]
        source: std::io::Error,
    },
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// HTTP-level failure after the connection was established.
    #[error("http error: {0}")]
    Http(String),
    /// A response or event payload did not match the contract.
    #[error("cannot decode daemon response: {0}")]
    Decode(#[from] serde_json::Error),
    /// A stream broke the protocol (e.g. closed before its terminal event).
    #[error("{0}")]
    Protocol(String),
    /// `daemon.json` is missing, unreadable or malformed.
    #[error("daemon.json at {path}: {reason}")]
    DaemonInfo { path: PathBuf, reason: String },
    /// The TCP listener is not available (no `http` in daemon.json, bad URL).
    #[error("invalid TCP endpoint: {0}")]
    Endpoint(String),
    /// The rocket binary could not be found or launched.
    #[error("{0}")]
    Launch(String),
    #[error("rocketd did not start within {waited:?}; see {log}")]
    DaemonStartTimeout { waited: Duration, log: PathBuf },
    #[error(transparent)]
    Paths(#[from] crate::paths::PathsError),
}

impl ClientError {
    /// The API error code, when this is an [`ClientError::Api`] error.
    pub fn code(&self) -> Option<&ErrorCode> {
        match self {
            Self::Api { code, .. } => Some(code),
            _ => None,
        }
    }
}
