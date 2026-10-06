//! Host port inspection and TCP/HTTP readiness checks (Go: `adapters/probe`).

use crate::lookup::look_path;
use bytes::Bytes;
use http_body_util::Empty;
use hyper::header::{HOST, LOCATION};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use rocket_domain::ports::{Error, HealthProbe, PortProbe, Result, async_trait};
use rocket_domain::{HealthCheck, PortHolder};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a loopback dial may take before the port counts as closed.
const DIAL_TIMEOUT: Duration = Duration::from_millis(150);
/// Budget shared by the two `lsof` invocations of one lookup.
const LSOF_TIMEOUT: Duration = Duration::from_secs(3);
/// Per-attempt budget of an HTTP probe (Go's default `http.Client` timeout).
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
/// Go's `http.Client` gives up after this many redirects.
const MAX_REDIRECTS: usize = 10;

/// [`PortProbe`] backed by loopback dials/binds and `lsof`.
#[derive(Debug, Clone, Default)]
pub struct Ports {
    /// The `lsof` binary; defaults to `"lsof"`.
    pub lsof_bin: Option<String>,
}

impl Ports {
    /// Uses `lsof` from `PATH`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl PortProbe for Ports {
    /// Whether nothing accepts connections on the port and it can be bound on
    /// both loopback and the wildcard address.
    fn free(&self, port: u16) -> bool {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            if TcpStream::connect_timeout(&SocketAddr::new(ip, port), DIAL_TIMEOUT).is_ok() {
                return false;
            }
        }
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        ] {
            if TcpListener::bind(SocketAddr::new(ip, port)).is_err() {
                return false;
            }
        }
        true
    }

    /// The process listening on `port`, found with `lsof`. `None` when there
    /// is no holder or `lsof` is unavailable. Blocks for up to three seconds,
    /// so call it from a blocking context.
    fn holder(&self, port: u16) -> Result<Option<PortHolder>> {
        let bin = self
            .lsof_bin
            .as_deref()
            .filter(|b| !b.is_empty())
            .unwrap_or("lsof");
        let Some(bin) = look_path(bin) else {
            return Ok(None);
        };
        let deadline = Instant::now() + LSOF_TIMEOUT;
        let out = run_capture(
            &bin,
            &[
                "-nP".into(),
                format!("-iTCP:{port}"),
                "-sTCP:LISTEN".into(),
                "-Fpcn".into(),
            ],
            deadline,
        );
        let Some(mut h) = parse_lsof(&String::from_utf8_lossy(&out)) else {
            return Ok(None);
        };
        let cwd = run_capture(
            &bin,
            &[
                "-a".into(),
                "-p".into(),
                h.pid.to_string(),
                "-d".into(),
                "cwd".into(),
                "-Fn".into(),
            ],
            deadline,
        );
        h.cwd = parse_cwd(&String::from_utf8_lossy(&cwd));
        Ok(Some(h))
    }
}

/// Runs `bin` and returns whatever it wrote to stdout, killing it at
/// `deadline`. Failures yield partial or empty output, like Go's `.Output()`
/// whose error is ignored.
fn run_capture(bin: &Path, args: &[String], deadline: Instant) -> Vec<u8> {
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let Some(mut stdout) = child.stdout.take() else {
        return Vec::new();
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    loop {
        match child.try_wait() {
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(Some(_)) | Err(_) => break,
        }
    }
    reader.join().unwrap_or_default()
}

/// Parses `lsof -F pcn` output and returns the first process.
pub fn parse_lsof(out: &str) -> Option<PortHolder> {
    let mut h: Option<PortHolder> = None;
    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('p') {
            if h.is_some() {
                return h;
            }
            let Ok(pid) = rest.parse::<i32>() else {
                continue;
            };
            h = Some(PortHolder {
                pid,
                command: String::new(),
                cwd: String::new(),
            });
        } else if let Some(rest) = line.strip_prefix('c')
            && let Some(h) = h.as_mut()
        {
            h.command = rest.to_string();
        }
    }
    h
}

/// The first `n<path>` line of `lsof -Fn` output.
pub fn parse_cwd(out: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix('n'))
        .unwrap_or_default()
        .to_string()
}

/// [`HealthProbe`] performing one TCP dial or HTTP GET against loopback
/// (IPv4 first, then IPv6).
#[derive(Debug, Clone)]
pub struct Health {
    /// Per-attempt budget of an HTTP probe.
    pub timeout: Duration,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            timeout: HTTP_TIMEOUT,
        }
    }
}

impl Health {
    /// Two-second HTTP timeout, like Go's default client.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl HealthProbe for Health {
    async fn check(&self, c: &HealthCheck) -> Result<()> {
        let mut last = Error::msg("no probe attempted");
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let addr = SocketAddr::new(ip, c.port);
            let attempt = if c.kind == "http" {
                self.http_check(addr, &c.path).await
            } else {
                tokio::net::TcpStream::connect(addr)
                    .await
                    .map(drop)
                    .map_err(Error::from)
            };
            match attempt {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

impl Health {
    async fn http_check(&self, addr: SocketAddr, path: &str) -> Result<()> {
        let url = format!("http://{addr}{path}");
        let target = match path {
            "" => "/".to_string(),
            p if p.starts_with('/') => p.to_string(),
            p if p.starts_with('?') => format!("/{p}"),
            _ => return Err(Error::msg(format!("parse {url:?}: invalid URL"))),
        };
        let attempt = get_following_redirects(addr.to_string(), target, &url);
        match tokio::time::timeout(self.timeout, attempt).await {
            Ok(r) => r,
            Err(_) => Err(Error::msg(format!(
                "GET {url}: context deadline exceeded (Client.Timeout exceeded)"
            ))),
        }
    }
}

/// GET with Go's redirect behaviour: follow up to ten `Location` hops, then
/// judge the final status (`>= 400` fails). Redirects the probe cannot follow
/// (non-`http` schemes) count as reachable.
async fn get_following_redirects(
    mut authority: String,
    mut target: String,
    url: &str,
) -> Result<()> {
    for _ in 0..=MAX_REDIRECTS {
        let (status, location) = get_once(&authority, &target).await?;
        if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            return finish(status, url);
        }
        let Some(location) = location else {
            return finish(status, url); // no Location: Go returns the response as is
        };
        match resolve_location(&authority, &target, &location) {
            Redirect::To(a, t) => (authority, target) = (a, t),
            Redirect::Unfollowable => return Ok(()),
        }
    }
    Err(Error::msg(format!(
        "GET {url}: stopped after {MAX_REDIRECTS} redirects"
    )))
}

fn finish(status: StatusCode, url: &str) -> Result<()> {
    if status.as_u16() >= 400 {
        return Err(Error::msg(format!("GET {url}: status {}", status.as_u16())));
    }
    Ok(())
}

async fn get_once(authority: &str, target: &str) -> Result<(StatusCode, Option<String>)> {
    let stream = tokio::net::TcpStream::connect(authority).await?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(Error::other)?;
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method(Method::GET)
        .uri(target)
        .header(HOST, authority)
        .body(Empty::<Bytes>::new())
        .map_err(Error::other)?;
    let resp = sender.send_request(req).await.map_err(Error::other);
    driver.abort();
    let resp = resp?;
    let location = resp
        .headers()
        .get(LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Ok((resp.status(), location))
}

enum Redirect {
    To(String, String),
    Unfollowable,
}

fn resolve_location(authority: &str, target: &str, location: &str) -> Redirect {
    if let Some(rest) = location.strip_prefix("http://") {
        let (host, path) = rest
            .find('/')
            .map_or((rest, "/"), |i| (&rest[..i], &rest[i..]));
        let has_port = if host.starts_with('[') {
            host.contains("]:")
        } else {
            host.contains(':')
        };
        let host = if has_port {
            host.to_string()
        } else {
            format!("{host}:80")
        };
        return Redirect::To(host, path.to_string());
    }
    if location.contains("://") {
        return Redirect::Unfollowable; // https and friends: no TLS here
    }
    if location.starts_with('/') {
        return Redirect::To(authority.to_string(), location.to_string());
    }
    // Relative reference: resolve against the directory of the current path.
    let base = target.split('?').next().unwrap_or("/");
    let dir = base.rfind('/').map_or("/", |i| &base[..=i]);
    Redirect::To(authority.to_string(), format!("{dir}{location}"))
}
