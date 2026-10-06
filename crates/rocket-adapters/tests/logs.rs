//! Port of `internal/adapters/logs/logs_test.go`.

use rocket_adapters::events::Bus;
use rocket_adapters::logs::{Sink, follow_file};
use rocket_domain::event_type;
use rocket_domain::ports::{EventBus, LogSink, Subscription};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn sink(dir: &std::path::Path, interval: Duration, ring: usize) -> (Sink, Subscription) {
    let bus = Arc::new(Bus::new());
    let sub = bus.subscribe(100);
    let s = Sink::new(dir, Some(bus as Arc<dyn EventBus>))
        .with_poll_interval(interval)
        .with_ring_size(ring);
    (s, sub)
}

#[tokio::test]
async fn open_follow_tail_and_events() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut sub) = sink(dir.path(), Duration::from_millis(10), 3);

    let (mut f, path) = s.open("proj", "api").unwrap();
    assert_eq!(path, s.path("proj", "api").to_str().unwrap());
    for i in 1..=4 {
        writeln!(f, "line {i}").unwrap();
    }
    write!(f, "partial").unwrap();
    drop(f);

    let mut got = Vec::new();
    while got.len() < 4 {
        let e = tokio::time::timeout(Duration::from_secs(2), sub.recv())
            .await
            .unwrap_or_else(|_| panic!("got only {got:?}"))
            .unwrap();
        assert_eq!(e.r#type, event_type::LOG_LINE);
        assert_eq!((e.project.as_str(), e.service.as_str()), ("proj", "api"));
        got.push(e.line);
    }
    assert_eq!(got, ["line 1", "line 2", "line 3", "line 4"]);
    assert_eq!(s.tail("proj", "api", 2).unwrap(), ["line 3", "line 4"]);
    // more than the ring holds -> falls back to the file
    assert_eq!(
        s.tail("proj", "api", 10).unwrap(),
        ["line 1", "line 2", "line 3", "line 4", "partial"]
    );
    s.close();
}

#[tokio::test]
async fn job_log_publishes_job_events_and_flushes_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut sub) = sink(dir.path(), Duration::from_secs(3600), 2000); // only close_job's final poll publishes

    let (mut f, path) = s.open_job("proj", "j1").unwrap();
    assert_eq!(path, s.job_path("proj", "j1").to_str().unwrap());
    write!(f, "one\ntwo\n").unwrap();
    drop(f);
    s.close_job("proj", "j1");

    let mut got = Vec::new();
    while got.len() < 2 {
        let e = tokio::time::timeout(Duration::from_secs(2), sub.recv())
            .await
            .unwrap_or_else(|_| panic!("got only {got:?}"))
            .unwrap();
        assert_eq!(e.r#type, event_type::JOB_LOG);
        assert_eq!(e.job_id, "j1");
        assert_eq!(e.project, "proj");
        assert_eq!(e.service, "");
        got.push(e.line);
    }
    assert_eq!(got, ["one", "two"]);
    assert_eq!(s.tail_job("proj", "j1", 1).unwrap(), ["two"]);
    s.remove_job("proj", "j1").unwrap();
    assert!(s.tail_job("proj", "j1", 1).is_err(), "log not removed");
    s.remove_job("proj", "j1").unwrap(); // already gone: not an error
    s.close();
}

#[tokio::test]
async fn close_job_flushes_trailing_partial() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut sub) = sink(dir.path(), Duration::from_secs(3600), 2000);
    let (mut f, _) = s.open_job("p", "jpartial").unwrap();
    f.write_all(b"partial-without-newline").unwrap();
    drop(f);
    s.close_job("p", "jpartial");
    let e = sub
        .try_recv()
        .expect("close_job dropped the trailing partial line");
    assert_eq!(e.r#type, event_type::JOB_LOG);
    assert_eq!(e.line, "partial-without-newline");
    s.close();
}

#[tokio::test]
async fn follower_resets_on_truncation_and_trims_carriage_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut sub) = sink(dir.path(), Duration::from_millis(10), 50);
    let (mut f, path) = s.open("p", "svc").unwrap();
    f.write_all(b"first\r\nsecond\n").unwrap();
    f.flush().unwrap();
    let mut got = Vec::new();
    for _ in 0..2 {
        got.push(recv_line(&mut sub).await);
    }
    assert_eq!(got, ["first", "second"]);
    // Truncate (rotation): the follower must restart from offset 0.
    std::fs::write(&path, b"").unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    let mut again = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    again.write_all(b"after\n").unwrap();
    assert_eq!(recv_line(&mut sub).await, "after");
    s.close();
}

async fn recv_line(sub: &mut Subscription) -> String {
    tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("timeout")
        .unwrap()
        .line
}

#[tokio::test]
async fn open_only_follows_content_appended_after_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p").join("svc.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "old line\n").unwrap();
    let (s, mut sub) = sink(dir.path(), Duration::from_millis(10), 50);
    let (mut f, _) = s.open("p", "svc").unwrap();
    f.write_all(b"new line\n").unwrap();
    assert_eq!(recv_line(&mut sub).await, "new line");
    s.close();
}

#[test]
fn tail_missing_file_is_an_error_and_empty_file_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _sub) = sink(dir.path(), Duration::from_secs(3600), 10);
    assert!(s.tail("p", "none", 5).is_err());
    let (_f, _) = s.open("p", "empty").unwrap();
    assert_eq!(s.tail("p", "empty", 5).unwrap(), Vec::<String>::new());
    s.close();
}

#[cfg(unix)]
#[test]
fn log_files_and_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (s, _sub) = sink(dir.path(), Duration::from_secs(3600), 10);
    let (_f, path) = s.open("p", "svc").unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(std::path::Path::new(&path)), 0o600);
    assert_eq!(mode(&dir.path().join("p")), 0o700);
    s.close();
}

#[tokio::test]
async fn follow_file_streams_until_finished_and_drained() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.log");
    std::fs::write(&path, "a\nb\nc\n").unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    {
        let (finished, path) = (finished.clone(), path.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            write!(f, "d\ne-no-newline").unwrap();
            drop(f);
            finished.store(true, Ordering::SeqCst);
        });
    }
    let mut got = Vec::new();
    follow_file(
        &path,
        2,
        Duration::from_millis(5),
        || finished.load(Ordering::SeqCst),
        |l| {
            got.push(l);
            async { Ok(()) }
        },
    )
    .await
    .unwrap();
    assert_eq!(got, ["b", "c", "d", "e-no-newline"]);

    let mut got = Vec::new();
    follow_file(
        &path,
        -1,
        Duration::from_millis(1),
        || true,
        |l| {
            got.push(l);
            async { Ok(()) }
        },
    )
    .await
    .unwrap();
    assert_eq!(got.len(), 5);
    assert_eq!(got[0], "a");
}

#[tokio::test]
async fn follow_file_stops_when_emit_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.log");
    std::fs::write(&path, "a\nb\n").unwrap();
    let err = follow_file(
        &path,
        -1,
        Duration::from_millis(1),
        || true,
        |_| async { Err(rocket_domain::ports::Error::msg("client gone")) },
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "client gone");
    // A missing file is reported to the caller.
    assert!(
        follow_file(
            &dir.path().join("none"),
            0,
            Duration::from_millis(1),
            || true,
            |_| async { Ok(()) }
        )
        .await
        .is_err()
    );
}
