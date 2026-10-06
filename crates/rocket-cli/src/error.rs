//! CLI errors and their exit codes (Go: `exitError`, `APIError` handling in
//! `main`). Exit codes: 0 ok, 1 error, 2 partial failure or failed job,
//! 3 confirmation required.

use rocket_client::{ClientError, ErrorCode, PathsError};

pub const EXIT_ERROR: i32 = 1;
pub const EXIT_PARTIAL: i32 = 2;
pub const EXIT_CONFIRMATION: i32 = 3;

#[derive(Debug)]
pub enum CliError {
    /// Exit with this code and print nothing (Go: `exitError` with no text).
    Exit(i32),
    /// A plain message.
    Message(String),
    /// Discovery found no `rocket.yaml` (Go: `errors.Is(err, ErrNoManifest)`).
    NoManifest(String),
    /// A daemon or transport failure.
    Client(ClientError),
}

impl CliError {
    pub fn msg(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }

    /// Whether this is [`CliError::NoManifest`].
    pub fn is_no_manifest(&self) -> bool {
        matches!(self, Self::NoManifest(_))
    }

    /// The text printed after `rocket: `; `None` for a silent exit.
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Exit(_) => None,
            Self::Message(m) | Self::NoManifest(m) => Some(m.clone()),
            Self::Client(e) => Some(e.to_string()),
        }
    }

    /// `(exit code, JSON "code")`.
    pub fn codes(&self) -> (i32, String) {
        match self {
            Self::Exit(c) => (*c, "error".into()),
            Self::Client(e) => match e.code() {
                Some(ErrorCode::ConfirmationRequired) => {
                    (EXIT_CONFIRMATION, "confirmation_required".into())
                }
                Some(code) if !code.as_str().is_empty() => (EXIT_ERROR, code.as_str().to_owned()),
                _ => (EXIT_ERROR, "error".into()),
            },
            _ => (EXIT_ERROR, "error".into()),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text().unwrap_or_default())
    }
}

impl std::error::Error for CliError {}

impl From<ClientError> for CliError {
    fn from(e: ClientError) -> Self {
        Self::Client(e)
    }
}

impl From<PathsError> for CliError {
    fn from(e: PathsError) -> Self {
        Self::Message(e.to_string())
    }
}

impl From<rocket_manifest::Error> for CliError {
    fn from(e: rocket_manifest::Error) -> Self {
        if e.is_no_manifest() {
            Self::NoManifest(e.to_string())
        } else {
            Self::Message(e.to_string())
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::Message(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: ErrorCode) -> CliError {
        CliError::Client(ClientError::Api {
            status: 400,
            code,
            message: "boom".into(),
        })
    }

    #[test]
    fn exit_codes_and_json_codes() {
        assert_eq!(CliError::msg("x").codes(), (1, "error".into()));
        assert_eq!(CliError::Exit(2).codes().0, 2);
        assert_eq!(CliError::Exit(2).text(), None);
        assert_eq!(api(ErrorCode::Invalid).codes(), (1, "invalid".into()));
        assert_eq!(api(ErrorCode::NotFound).codes(), (1, "not_found".into()));
        assert_eq!(
            api(ErrorCode::ConfirmationRequired).codes(),
            (3, "confirmation_required".into())
        );
        assert_eq!(
            api(ErrorCode::Other(String::new())).codes(),
            (1, "error".into())
        );
        assert_eq!(api(ErrorCode::Invalid).text().as_deref(), Some("boom"));
    }

    #[test]
    fn manifest_errors_keep_their_no_manifest_identity() {
        let e: CliError = rocket_manifest::Error::NoManifest { start: "/x".into() }.into();
        assert!(e.is_no_manifest());
        assert_eq!(
            e.text().unwrap(),
            "no rocket.yaml found in /x or any parent directory"
        );
        let e: CliError = rocket_manifest::Error::Parse("bad".into()).into();
        assert!(!e.is_no_manifest());
    }
}
