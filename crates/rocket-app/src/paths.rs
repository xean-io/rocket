//! Lexical path helpers matching Go's `filepath.Join` / `filepath.Clean`.

use std::path::{Component, Path, PathBuf};

/// Go `filepath.Clean`: collapses `.`, `..` and repeated separators without
/// touching the filesystem. An empty path cleans to `.`.
pub(crate) fn clean(path: &Path) -> PathBuf {
    let mut out: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(comp),
            },
            other => out.push(other),
        }
    }
    if out.is_empty() {
        return PathBuf::from(".");
    }
    out.iter().collect()
}

/// Go `filepath.Join(a, b)`: joins and cleans; empty elements are ignored.
pub(crate) fn join(a: &str, b: &str) -> PathBuf {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => PathBuf::new(),
        (true, false) => clean(Path::new(b)),
        (false, true) => clean(Path::new(a)),
        (false, false) => clean(&Path::new(a).join(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_matches_go() {
        assert_eq!(join("/code/api", ""), PathBuf::from("/code/api"));
        assert_eq!(
            join("/code/api", "apps/web"),
            PathBuf::from("/code/api/apps/web")
        );
        assert_eq!(join("/code/api", "./x/../y"), PathBuf::from("/code/api/y"));
        assert_eq!(join("/code", "../../.."), PathBuf::from("/"));
        assert_eq!(join("", ""), PathBuf::new());
    }
}
