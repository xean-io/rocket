//! Ports of `internal/adapters/probe/{probe_test,probe_integration_test}.go`.

use rocket_adapters::probe::{Health, Ports, parse_cwd, parse_lsof};
use rocket_domain::HealthCheck;
use rocket_domain::ports::{HealthProbe, PortProbe};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Tests that bind, dial or probe ephemeral ports run one at a time: a
/// concurrent test's outbound connection can briefly occupy the very port
/// another test just released and asserts to be free.
static PORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn parse_lsof_first_process() {
    let out = "p4242\ncnode\nn*:3000\np4243\ncother\n";
    let h = parse_lsof(out).unwrap();
    assert_eq!((h.pid, h.command.as_str()), (4242, "node"));
    assert!(parse_lsof("").is_none(), "empty output should yield none");
    assert_eq!(parse_cwd("p1\nfcwd\nn/Users/me/code\n"), "/Users/me/code");
    assert_eq!(parse_cwd("p1\nfcwd\n"), "");
}

#[test]
fn parse_lsof_skips_malformed_pid_lines() {
    assert!(parse_lsof("pnotanumber\ncx\n").is_none());
    let h = parse_lsof("pbad\np7\ncgo\n").unwrap();
    assert_eq!((h.pid, h.command.as_str()), (7, "go"));
}

fn lsof_available() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("lsof").is_file()))
}

#[test]
fn holder_reports_listening_process() {
    let _serial = PORTS.blocking_lock();
    if !lsof_available() {
        eprintln!("skipped: lsof not installed");
        return;
    }
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let h = Ports::new().holder(port).unwrap().expect("holder");
    assert_eq!(h.pid as u32, std::process::id());
    assert!(!h.cwd.is_empty() && !h.command.is_empty(), "{h:?}");
}

#[test]
fn holder_is_none_without_lsof_or_listener() {
    let _serial = PORTS.blocking_lock();
    let p = Ports {
        lsof_bin: Some("/nonexistent/lsof".into()),
    };
    assert!(p.holder(1).unwrap().is_none());
    if lsof_available() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(Ports::new().holder(port).unwrap().is_none());
    }
}

#[test]
fn free_detects_listener() {
    let _serial = PORTS.blocking_lock();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    assert!(
        !Ports::new().free(port),
        "port {port} reported free while listening"
    );
    drop(l);
    assert!(
        Ports::new().free(port),
        "port {port} reported busy after close"
    );
}

/// Minimal HTTP/1.1 server: `/broken` -> 503, `/redir` -> 302 /broken,
/// `/redir-ok` -> 302 /live, anything else -> 200.
async fn serve(listener: TcpListener) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (status, extra) = match path.as_str() {
                "/broken" => ("503 Service Unavailable", ""),
                "/redir" => ("302 Found", "Location: /broken\r\n"),
                "/redir-ok" => ("302 Found", "Location: /live\r\n"),
                _ => ("200 OK", ""),
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
    }
}

fn check(kind: &str, port: u16, path: &str) -> HealthCheck {
    HealthCheck {
        kind: kind.into(),
        port,
        path: path.into(),
    }
}

#[tokio::test]
async fn health_checks() {
    let _serial = PORTS.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(serve(listener));
    let h = Health::new();
    h.check(&check("http", port, "/live"))
        .await
        .expect("http ok");
    assert!(
        h.check(&check("http", port, "/broken")).await.is_err(),
        "503 must fail"
    );
    h.check(&check("tcp", port, "")).await.expect("tcp");
    // Redirects are followed like Go's http.Client: the final status decides.
    h.check(&check("http", port, "/redir-ok"))
        .await
        .expect("redirect to 200");
    assert!(
        h.check(&check("http", port, "/redir")).await.is_err(),
        "redirect to 503 must fail"
    );
    server.abort();
    let _ = server.await;
    assert!(
        h.check(&check("tcp", port, "")).await.is_err(),
        "closed port must fail"
    );
    assert!(h.check(&check("http", port, "/live")).await.is_err());
}

#[tokio::test]
async fn http_check_times_out_on_a_silent_server() {
    let _serial = PORTS.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            held.push(s); // accept, never answer
        }
    });
    let h = Health {
        timeout: std::time::Duration::from_millis(200),
    };
    let began = std::time::Instant::now();
    assert!(h.check(&check("http", port, "/")).await.is_err());
    assert!(began.elapsed() < std::time::Duration::from_secs(5));
    server.abort();
}
