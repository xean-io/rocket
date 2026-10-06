//! The bits of Go's `path/filepath` the scaffold relies on.

use std::path::{Component, Path, PathBuf};

/// Lexical `filepath.Clean`: drops `.` and empty elements, resolves `..`
/// against preceding elements; an empty result is `.`.
pub fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut depth = 0usize; // normal components currently in `out`
    let mut rooted = false;
    for c in path.components() {
        match c {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => {
                rooted = true;
                out.push(Component::RootDir.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if !rooted {
                    out.push("..");
                }
            }
            Component::Normal(n) => {
                out.push(n);
                depth += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// `filepath.Abs`: absolute and cleaned, relative paths resolved against the
/// working directory.
pub fn abs(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(clean(path))
    } else {
        Ok(clean(&std::env::current_dir()?.join(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_like_go() {
        for (input, want) in [
            ("a/./b/../c", "a/c"),
            ("/a/../..", "/"),
            ("../a", "../a"),
            ("a/..", "."),
            ("", "."),
            ("/x//y/", "/x/y"),
            ("./infra.yml", "infra.yml"),
        ] {
            assert_eq!(clean(Path::new(input)), Path::new(want), "{input}");
        }
    }
}
