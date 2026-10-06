//! One-connection-per-request HTTP/1.1 transport over a unix socket or the
//! token-protected TCP listener. Local connections are cheap, and a fresh
//! connection per call keeps SSE lifetimes independent of each other.

use crate::daemon_info::DaemonInfo;
use crate::error::ClientError;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HOST, USER_AGENT};
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::JoinHandle;

/// Aborts the connection driver task when dropped, which closes the socket.
pub(crate) struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
struct TcpTarget {
    /// `host:port`, used both to connect and as the Host header.
    authority: String,
    token: String,
}

enum Target {
    Unix(PathBuf),
    Tcp(Mutex<TcpTarget>),
}

pub(crate) struct Transport {
    target: Target,
    /// daemon.json to re-read once after a 401 (TCP only).
    refresh_from: Option<PathBuf>,
}

pub(crate) fn parse_authority(base_url: &str) -> Result<String, ClientError> {
    let uri: hyper::Uri = base_url
        .parse()
        .map_err(|e| ClientError::Endpoint(format!("{base_url}: {e}")))?;
    if uri.scheme_str() != Some("http") {
        return Err(ClientError::Endpoint(format!(
            "{base_url}: only http:// endpoints are supported"
        )));
    }
    let host = uri
        .host()
        .ok_or_else(|| ClientError::Endpoint(format!("{base_url}: missing host")))?;
    let port = uri.port_u16().unwrap_or(80);
    Ok(if host.contains(':') {
        format!("[{host}]:{port}") // bare IPv6 literal
    } else {
        format!("{host}:{port}")
    })
}

impl Transport {
    pub(crate) fn unix(socket: PathBuf) -> Self {
        Self {
            target: Target::Unix(socket),
            refresh_from: None,
        }
    }

    pub(crate) fn tcp(
        base_url: &str,
        token: &str,
        refresh_from: Option<PathBuf>,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            target: Target::Tcp(Mutex::new(TcpTarget {
                authority: parse_authority(base_url)?,
                token: token.to_owned(),
            })),
            refresh_from,
        })
    }

    /// Sends one request. On a 401 over TCP, daemon.json is re-read once
    /// (the token rotates on every daemon start) and the request retried.
    pub(crate) async fn send(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<&[u8]>,
    ) -> Result<(Response<Incoming>, AbortOnDrop), ClientError> {
        let first = self.send_once(&method, path_and_query, body).await?;
        let (Target::Tcp(target), Some(file)) = (&self.target, &self.refresh_from) else {
            return Ok(first);
        };
        if first.0.status() != 401 {
            return Ok(first);
        }
        let Ok(info) = DaemonInfo::load(file) else {
            return Ok(first);
        };
        let Ok(authority) = parse_authority(&info.http) else {
            return Ok(first);
        };
        *target.lock().unwrap_or_else(|p| p.into_inner()) = TcpTarget {
            authority,
            token: info.token,
        };
        drop(first);
        self.send_once(&method, path_and_query, body).await
    }

    async fn send_once(
        &self,
        method: &Method,
        path_and_query: &str,
        body: Option<&[u8]>,
    ) -> Result<(Response<Incoming>, AbortOnDrop), ClientError> {
        let mut req = Request::builder()
            .method(method.clone())
            .uri(path_and_query)
            .header(
                USER_AGENT,
                concat!("rocket-client/", env!("CARGO_PKG_VERSION")),
            );
        if body.is_some() {
            req = req.header(CONTENT_TYPE, "application/json");
        }
        let payload = Bytes::copy_from_slice(body.unwrap_or_default());
        match &self.target {
            Target::Unix(socket) => {
                let req = req
                    .header(HOST, "rocketd")
                    .body(Full::new(payload))
                    .map_err(|e| ClientError::Http(e.to_string()))?;
                let io = connect_unix(socket).await?;
                round_trip(io, req).await
            }
            Target::Tcp(target) => {
                let t = target.lock().unwrap_or_else(|p| p.into_inner()).clone();
                let req = req
                    .header(HOST, &t.authority)
                    .header(AUTHORIZATION, format!("Bearer {}", t.token))
                    .body(Full::new(payload))
                    .map_err(|e| ClientError::Http(e.to_string()))?;
                let io = tokio::net::TcpStream::connect(&t.authority)
                    .await
                    .map_err(|source| ClientError::Connect {
                        target: format!("http://{}", t.authority),
                        source,
                    })?;
                round_trip(io, req).await
            }
        }
    }
}

#[cfg(unix)]
async fn connect_unix(socket: &Path) -> Result<tokio::net::UnixStream, ClientError> {
    tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|source| ClientError::Connect {
            target: socket.display().to_string(),
            source,
        })
}

#[cfg(not(unix))]
async fn connect_unix(_socket: &Path) -> Result<tokio::net::TcpStream, ClientError> {
    Err(ClientError::Endpoint(
        "unix sockets are not supported on this platform; use the TCP transport".into(),
    ))
}

async fn round_trip<T>(
    io: T,
    req: Request<Full<Bytes>>,
) -> Result<(Response<Incoming>, AbortOnDrop), ClientError>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|e| ClientError::Http(e.to_string()))?;
    let guard = AbortOnDrop(tokio::spawn(async move {
        let _ = conn.await;
    }));
    let resp = sender
        .send_request(req)
        .await
        .map_err(|e| ClientError::Http(e.to_string()))?;
    Ok((resp, guard))
}
