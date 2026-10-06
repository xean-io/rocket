//! The error every Tauri command returns to the frontend.

use rocket_client::ClientError;
use serde::Serialize;
use ts_rs::TS;

/// Serialized as `{ code, message, status }`.
///
/// `code` is the rocket API error code verbatim for daemon answers
/// (`invalid`, `not_found`, `conflict`, `confirmation_required`, ...) and one
/// of the local codes below for everything that never reached the daemon:
/// `unreachable`, `io`, `http`, `decode`, `protocol`, `daemon_info`,
/// `endpoint`, `launch`, `daemon_start_timeout`, `paths`, `state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    /// HTTP status of a daemon answer; `null` for local failures.
    pub status: Option<u16>,
}

impl CommandError {
    pub fn local(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            status: None,
        }
    }
}

impl From<ClientError> for CommandError {
    fn from(err: ClientError) -> Self {
        let message = err.to_string();
        match err {
            ClientError::Api {
                status,
                code,
                message,
            } => {
                let code = match code.as_str() {
                    "" => "error",
                    c => c,
                };
                Self {
                    code: code.to_owned(),
                    message,
                    status: Some(status),
                }
            }
            ClientError::Connect { .. } => Self::local("unreachable", message),
            ClientError::Io(_) => Self::local("io", message),
            ClientError::Http(_) => Self::local("http", message),
            ClientError::Decode(_) => Self::local("decode", message),
            ClientError::Protocol(_) => Self::local("protocol", message),
            ClientError::DaemonInfo { .. } => Self::local("daemon_info", message),
            ClientError::Endpoint(_) => Self::local("endpoint", message),
            ClientError::Launch(_) => Self::local("launch", message),
            ClientError::DaemonStartTimeout { .. } => Self::local("daemon_start_timeout", message),
            ClientError::Paths(_) => Self::local("paths", message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket_client::ErrorCode;

    fn api(status: u16, code: ErrorCode) -> ClientError {
        ClientError::Api {
            status,
            code,
            message: "boom".into(),
        }
    }

    #[test]
    fn api_errors_keep_the_rocket_code_and_status() {
        for (status, code, want) in [
            (400, ErrorCode::Invalid, "invalid"),
            (404, ErrorCode::NotFound, "not_found"),
            (409, ErrorCode::Conflict, "conflict"),
            (
                428,
                ErrorCode::ConfirmationRequired,
                "confirmation_required",
            ),
            (500, ErrorCode::Internal, "internal"),
        ] {
            let e = CommandError::from(api(status, code));
            assert_eq!(e.code, want);
            assert_eq!(e.status, Some(status));
            assert_eq!(e.message, "boom");
        }
    }

    #[test]
    fn unknown_and_empty_api_codes() {
        let e = CommandError::from(api(418, ErrorCode::Other("teapot".into())));
        assert_eq!((e.code.as_str(), e.status), ("teapot", Some(418)));
        let e = CommandError::from(api(502, ErrorCode::Other(String::new())));
        assert_eq!(e.code, "error");
    }

    #[test]
    fn local_failures_have_no_status() {
        let connect = ClientError::Connect {
            target: "/tmp/x.sock".into(),
            source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
        };
        let e = CommandError::from(connect);
        assert_eq!(e.code, "unreachable");
        assert_eq!(e.status, None);
        assert!(e.message.contains("/tmp/x.sock"));

        let e = CommandError::from(ClientError::Launch("no binary".into()));
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("launch", "no binary")
        );
        let e = CommandError::from(ClientError::Protocol("cut".into()));
        assert_eq!(e.code, "protocol");
    }

    #[test]
    fn serializes_as_code_message_status() {
        let e = CommandError::from(api(428, ErrorCode::ConfirmationRequired));
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"code": "confirmation_required", "message": "boom", "status": 428})
        );
        let e = CommandError::local("state", "x");
        assert_eq!(
            serde_json::to_value(&e).unwrap()["status"],
            serde_json::Value::Null
        );
    }
}
