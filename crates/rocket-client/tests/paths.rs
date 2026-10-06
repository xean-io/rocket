//! `ROCKET_HOME` resolution, file layout and daemon.json parsing. Nothing here
//! reads the process environment or the real `~/.rocket`.

use rocket_client::{DaemonInfo, Paths, PathsError};
use std::ffi::OsStr;
use std::path::Path;

#[test]
fn layout_matches_go_internal_paths() {
    let p = Paths::from_home("/tmp/rk").unwrap();
    assert_eq!(p.home, Path::new("/tmp/rk"));
    assert_eq!(p.socket, Path::new("/tmp/rk/rocketd.sock"));
    assert_eq!(p.db, Path::new("/tmp/rk/state.db"));
    assert_eq!(p.logs, Path::new("/tmp/rk/logs"));
    assert_eq!(p.pid_file, Path::new("/tmp/rk/rocketd.pid"));
    assert_eq!(p.lock_file, Path::new("/tmp/rk/rocketd.lock"));
    assert_eq!(p.daemon_log, Path::new("/tmp/rk/rocketd.log"));
    assert_eq!(p.daemon_json, Path::new("/tmp/rk/daemon.json"));
}

#[test]
fn rocket_home_wins_then_user_home_dot_rocket() {
    let user = Path::new("/Users/me");
    let p = Paths::resolve_from(Some(OsStr::new("/tmp/x")), Some(user)).unwrap();
    assert_eq!(p.home, Path::new("/tmp/x"));
    let p = Paths::resolve_from(Some(OsStr::new("")), Some(user)).unwrap();
    assert_eq!(p.home, Path::new("/Users/me/.rocket"));
    let p = Paths::resolve_from(None, Some(user)).unwrap();
    assert_eq!(p.home, Path::new("/Users/me/.rocket"));
    assert!(matches!(
        Paths::resolve_from(None, None),
        Err(PathsError::NoHome)
    ));
}

#[test]
fn relative_home_becomes_absolute() {
    let p = Paths::from_home("rel/dir").unwrap();
    assert!(p.home.is_absolute());
    assert!(p.home.ends_with("rel/dir"));
}

#[test]
fn socket_path_longer_than_103_bytes_is_rejected() {
    // "/rocketd.sock" is 13 bytes: 90 bytes of home is the last accepted size.
    let ok = format!("/{}", "a".repeat(89));
    assert_eq!(ok.len(), 90);
    assert!(Paths::from_home(&ok).is_ok());
    let long = format!("/{}", "a".repeat(90));
    match Paths::from_home(&long) {
        Err(PathsError::SocketTooLong(p)) => assert!(p.ends_with("rocketd.sock")),
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn ensure_creates_private_dirs() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::Builder::new()
        .prefix("rkp")
        .tempdir_in("/tmp")
        .unwrap();
    let p = Paths::from_home(tmp.path().join("home")).unwrap();
    p.ensure().unwrap();
    for d in [&p.home, &p.logs] {
        let mode = std::fs::metadata(d).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{}", d.display());
    }
    p.ensure().unwrap(); // idempotent
}

const DAEMON_JSON: &str = r#"{"version":"0.1.0-dev","api":"v1","pid":4242,"socket":"/Users/me/.rocket/rocketd.sock","http":"http://127.0.0.1:52817","token":"s3cr3t","rocket_bin":"/usr/local/bin/rocket","started_at":"2026-10-05T00:14:10.938611Z"}"#;

#[test]
fn daemon_info_parses_swift_fixture() {
    let i = DaemonInfo::parse(DAEMON_JSON.as_bytes()).unwrap();
    assert_eq!(i.version, "0.1.0-dev");
    assert_eq!(i.api, "v1");
    assert_eq!(i.pid, 4242);
    assert_eq!(i.http, "http://127.0.0.1:52817");
    assert_eq!(i.token, "s3cr3t");
    assert_eq!(i.rocket_bin, "/usr/local/bin/rocket");
    assert_eq!(i.started_at.unix_timestamp(), 1_791_159_250);
    // Byte-compatible with Go's json.Marshal field order.
    let again: DaemonInfo = serde_json::from_str(&serde_json::to_string(&i).unwrap()).unwrap();
    assert_eq!(again, i);
    assert!(
        serde_json::to_string(&i)
            .unwrap()
            .starts_with(r#"{"version":"0.1.0-dev","api":"v1","pid":4242,"socket""#)
    );
}

#[test]
fn daemon_info_is_lenient_like_the_swift_loader() {
    let i =
        DaemonInfo::parse(br#"{"http":"http://127.0.0.1:1","token":"t","extra":true}"#).unwrap();
    assert_eq!(i.token, "t");
    assert_eq!(i.pid, 0);
    assert!(
        DaemonInfo::parse(br#"{"token":"t"}"#).is_ok(),
        "http may be absent: TCP just unavailable"
    );
    assert!(DaemonInfo::parse(b"not json").is_err());
}

#[test]
fn daemon_info_write_is_atomic_and_private() {
    let tmp = tempfile::Builder::new()
        .prefix("rki")
        .tempdir_in("/tmp")
        .unwrap();
    let path = tmp.path().join("daemon.json");
    let i = DaemonInfo::parse(DAEMON_JSON.as_bytes()).unwrap();
    i.write(&path).unwrap();
    assert_eq!(DaemonInfo::load(&path).unwrap(), i);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "temp file must be renamed away");
    assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));
    // Missing file is a DaemonInfo error naming the path.
    let err = DaemonInfo::load(&tmp.path().join("nope.json")).unwrap_err();
    assert!(err.to_string().contains("nope.json"), "{err}");
}
