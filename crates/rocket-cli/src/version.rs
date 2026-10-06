//! The version string (Go: `version.go`).
//!
//! Releases override the Cargo package version at build time with
//! `ROCKET_VERSION=0.2.0 cargo build --release`, the counterpart of GoReleaser's
//! `-ldflags "-X main.version=..."`.

/// Placeholder of builds that carry no release version.
pub const DEV_VERSION: &str = "dev";

/// Prefers the build-time override, then the Cargo package version. An
/// override that is empty or still the `dev` placeholder does not count.
pub fn resolve(build_override: Option<&str>, package: &str) -> String {
    match build_override.map(str::trim) {
        Some(v) if !v.is_empty() && v != DEV_VERSION => v.to_owned(),
        _ => package.to_owned(),
    }
}

/// The version of this binary.
pub fn version() -> &'static str {
    use std::sync::OnceLock;
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| resolve(option_env!("ROCKET_VERSION"), env!("CARGO_PKG_VERSION")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_the_go_ldflags_fallback() {
        let cases = [
            ("override wins (release build)", Some("0.2.0"), "0.2.0"),
            (
                "surrounding whitespace is ignored",
                Some(" 0.2.0\n"),
                "0.2.0",
            ),
            ("pre-release override", Some("0.1.1-rc.1"), "0.1.1-rc.1"),
            ("dev placeholder falls back", Some("dev"), "0.1.1"),
            ("empty override falls back", Some(""), "0.1.1"),
            ("no override", None, "0.1.1"),
        ];
        for (name, over, want) in cases {
            assert_eq!(resolve(over, "0.1.1"), want, "{name}");
        }
    }

    #[test]
    fn version_is_never_empty() {
        assert!(!version().is_empty());
    }
}
