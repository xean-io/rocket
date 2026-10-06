//! dotenv parsing (Go: `dotenv.go`).

use crate::convert::is_env_var_name;
use rocket_domain::ports::{self, EnvSource};
use std::collections::BTreeMap;
use std::path::Path;

/// Parses `KEY=VALUE` lines: optional `export `, single or double quotes
/// (`\n` is expanded inside double quotes only), `#` comments and trailing
/// ` #` comments on unquoted values. Malformed lines are ignored.
pub fn parse_dotenv(data: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(data);
    let mut out = BTreeMap::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if !is_env_var_name(k) {
            continue;
        }
        let mut v = v.trim().to_string();
        let quote = v.chars().next().filter(|c| *c == '"' || *c == '\'');
        let closing = quote.and_then(|q| v[1..].find(q));
        match (quote, closing) {
            (Some(q), Some(end)) if v.len() >= 2 => {
                v = v[1..1 + end].to_string();
                if q == '"' {
                    v = v.replace("\\n", "\n");
                }
            }
            _ => {
                if let Some(i) = v.find(" #") {
                    v = v[..i].trim().to_string();
                }
            }
        }
        out.insert(k.to_string(), v);
    }
    out
}

/// Reads dotenv files relative to a project root; implements
/// [`EnvSource`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvFiles;

impl EnvSource for EnvFiles {
    /// Merges the files in order (later wins); missing files are skipped.
    fn dotenv(&self, root: &Path, files: &[String]) -> ports::Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for f in files {
            let path = Path::new(f);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            match std::fs::read(&path) {
                Ok(data) => out.extend(parse_dotenv(&data)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(out)
    }
}
