//! Wraps `docker compose` with a forced project name so rocket-managed stacks
//! never collide with each other or with manual runs (Go: `adapters/compose`).
//!
//! The argv, working directory, environment and error wording match the Go
//! adapter. Dropping a returned future kills the child (Go's context
//! cancellation).
//!
//! Volume-creation hints and failed-attempt tracking are application logic
//! (`app/volume_hints.go` in Go) and are ported with the use cases,
//! not here; this module only supplies the evidence they need
//! ([`named_volumes`](ComposeDriver::named_volumes) and
//! [`volume_exists`](ComposeDriver::volume_exists)).

use crate::lookup::{look_path, not_found};
use rocket_domain::ports::{ComposeDriver, ComposeTarget, Error, Result, async_trait};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// How much of a failed command's stderr ends up in the error message.
const ERROR_TAIL_BYTES: usize = 400;
/// Stderr bytes remembered for the error message (a generous bound).
const TAIL_KEEP: usize = 64 * 1024;

/// [`ComposeDriver`] that shells out to the docker CLI.
#[derive(Debug, Clone, Default)]
pub struct Driver {
    /// The docker binary; defaults to `"docker"` when empty.
    pub bin: String,
}

impl Driver {
    /// Uses `docker` from `PATH`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses an explicit binary (tests point this at a fixture script).
    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    fn bin(&self) -> &str {
        if self.bin.is_empty() {
            "docker"
        } else {
            &self.bin
        }
    }
}

/// Builds `compose -p <name> -f … --profile … <op…>`.
pub fn args(t: &ComposeTarget, op: &[&str]) -> Vec<String> {
    let mut a = vec!["compose".to_string(), "-p".into(), t.project_name.clone()];
    for f in &t.files {
        a.push("-f".into());
        a.push(f.clone());
    }
    for p in &t.profiles {
        a.push("--profile".into());
        a.push(p.clone());
    }
    a.extend(op.iter().map(ToString::to_string));
    a
}

/// Where one of the child's output streams goes.
#[derive(Clone, Copy)]
enum Route {
    /// The caller's writer.
    Out,
    /// The captured stdout buffer.
    Buf,
    Discard,
}

/// A command's working directory and environment (Go: `cmd.Dir`, `cmd.Env`).
struct Context<'a> {
    dir: &'a Path,
    env: &'a [String],
}

struct Streams<'a> {
    stdout: Route,
    stderr: Route,
    out: Option<&'a mut (dyn Write + Send)>,
    buf: Option<&'a mut Vec<u8>>,
}

impl Streams<'_> {
    fn route(&mut self, route: Route, data: &[u8], write_err: &mut Option<std::io::Error>) {
        match route {
            Route::Discard => {}
            Route::Buf => {
                if let Some(buf) = self.buf.as_deref_mut() {
                    buf.extend_from_slice(data);
                }
            }
            Route::Out => {
                if let Some(out) = self.out.as_deref_mut()
                    && write_err.is_none()
                    && let Err(e) = out.write_all(data)
                {
                    *write_err = Some(e);
                }
            }
        }
    }
}

/// Runs `bin args…`, routing its streams, and reports failure like Go's
/// `run`: `<exit status>: <last 400 bytes of stderr>`. Stderr is always
/// remembered for the message regardless of where it is routed.
async fn exec(
    bin: &str,
    args: &[String],
    ctx: &Context<'_>,
    mut streams: Streams<'_>,
) -> Result<()> {
    let path = look_path(bin).ok_or_else(|| Error::other(not_found(bin)))?;
    let mut cmd = Command::new(path);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !ctx.dir.as_os_str().is_empty() {
        cmd.current_dir(ctx.dir);
    }
    if !ctx.env.is_empty() {
        cmd.env_clear();
        for kv in ctx.env {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }
    }
    let mut child = cmd.spawn()?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(Error::msg("child pipes unavailable"));
    };
    let (mut stdout_open, mut stderr_open) = (true, true);
    let mut tail: Vec<u8> = Vec::new();
    let mut write_err = None;
    let (mut b1, mut b2) = (vec![0u8; 8192], vec![0u8; 8192]);
    while stdout_open || stderr_open {
        tokio::select! {
            n = stdout.read(&mut b1), if stdout_open => match n {
                Ok(n) if n > 0 => streams.route(streams.stdout, &b1[..n], &mut write_err),
                _ => stdout_open = false,
            },
            n = stderr.read(&mut b2), if stderr_open => match n {
                Ok(n) if n > 0 => {
                    tail.extend_from_slice(&b2[..n]);
                    if tail.len() > TAIL_KEEP {
                        tail.drain(..tail.len() - TAIL_KEEP);
                    }
                    streams.route(streams.stderr, &b2[..n], &mut write_err);
                }
                _ => stderr_open = false,
            },
        }
    }
    let status = child.wait().await?;
    if !status.success() {
        return Err(failure(status, &tail));
    }
    match write_err {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

fn failure(status: ExitStatus, tail: &[u8]) -> Error {
    let base = describe(status);
    let text = String::from_utf8_lossy(tail);
    let msg = text.trim();
    let msg = if msg.len() > ERROR_TAIL_BYTES {
        let bytes = &msg.as_bytes()[msg.len() - ERROR_TAIL_BYTES..];
        format!("…{}", String::from_utf8_lossy(bytes))
    } else {
        msg.to_string()
    };
    if msg.is_empty() {
        Error::msg(base)
    } else {
        Error::msg(format!("{base}: {msg}"))
    }
}

/// Go's `ExitError` wording.
fn describe(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return format!("signal: {sig}");
        }
    }
    match status.code() {
        Some(c) => format!("exit status {c}"),
        None => "exit status -1".to_string(),
    }
}

impl Driver {
    /// Runs a `docker compose …` operation for `t`.
    async fn compose<'a>(
        &self,
        t: &ComposeTarget,
        op: &[&str],
        stdout: Route,
        stderr: Route,
        out: Option<&'a mut (dyn Write + Send)>,
        buf: Option<&'a mut Vec<u8>>,
    ) -> Result<()> {
        let ctx = Context {
            dir: &t.dir,
            env: &t.env,
        };
        exec(
            self.bin(),
            &args(t, op),
            &ctx,
            Streams {
                stdout,
                stderr,
                out,
                buf,
            },
        )
        .await
    }
}

#[async_trait]
impl ComposeDriver for Driver {
    /// Nonexternal named volumes mounted by `service`, from the normalized
    /// `compose config` model.
    async fn named_volumes(&self, t: &ComposeTarget, service: &str) -> Result<Vec<String>> {
        let mut buf = Vec::new();
        self.compose(
            t,
            &["config", "--format", "json"],
            Route::Buf,
            Route::Discard,
            None,
            Some(&mut buf),
        )
        .await?;
        named_volumes_from_config(&buf, service)
    }

    /// Only a successful `docker volume ls` counts as evidence of absence;
    /// `docker volume inspect` also fails on missing volumes, which would
    /// obscure operational errors.
    async fn volume_exists(&self, t: &ComposeTarget, name: &str) -> Result<bool> {
        let mut buf = Vec::new();
        let ctx = Context {
            dir: &t.dir,
            env: &t.env,
        };
        exec(
            self.bin(),
            &[
                "volume".into(),
                "ls".into(),
                "--format".into(),
                "{{.Name}}".into(),
            ],
            &ctx,
            Streams {
                stdout: Route::Buf,
                stderr: Route::Discard,
                out: None,
                buf: Some(&mut buf),
            },
        )
        .await?;
        let text = String::from_utf8_lossy(&buf);
        Ok(text.trim().split('\n').any(|l| l == name))
    }

    /// Starts one service detached, waits for its healthcheck and returns its
    /// container id (empty when `ps` finds none).
    async fn up(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> Result<String> {
        self.compose(
            t,
            &["up", "-d", "--wait", service],
            Route::Out,
            Route::Out,
            Some(out),
            None,
        )
        .await?;
        let mut buf = Vec::new();
        // Best effort, like Go: a failing `ps` just yields an empty id.
        let _ = self
            .compose(
                t,
                &["ps", "-q", service],
                Route::Buf,
                Route::Discard,
                None,
                Some(&mut buf),
            )
            .await;
        Ok(String::from_utf8_lossy(&buf).trim().to_string())
    }

    async fn stop(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> Result<()> {
        self.compose(
            t,
            &["stop", service],
            Route::Out,
            Route::Out,
            Some(out),
            None,
        )
        .await
    }

    /// Removes the whole compose project (containers and network).
    async fn down(&self, t: &ComposeTarget, out: &mut (dyn Write + Send)) -> Result<()> {
        self.compose(
            t,
            &["down", "--remove-orphans"],
            Route::Out,
            Route::Out,
            Some(out),
            None,
        )
        .await
    }

    /// Looks for a running container by compose labels (`docker ps`, not
    /// `docker compose ps`, as in Go).
    async fn running(&self, compose_project: &str, service: &str) -> Result<bool> {
        let mut buf = Vec::new();
        let argv: Vec<String> = [
            "ps".to_string(),
            "-q".into(),
            "--filter".into(),
            format!("label=com.docker.compose.project={compose_project}"),
            "--filter".into(),
            format!("label=com.docker.compose.service={service}"),
            "--filter".into(),
            "status=running".into(),
        ]
        .into();
        let ctx = Context {
            dir: Path::new(""),
            env: &[],
        };
        exec(
            self.bin(),
            &argv,
            &ctx,
            Streams {
                stdout: Route::Buf,
                stderr: Route::Discard,
                out: None,
                buf: Some(&mut buf),
            },
        )
        .await?;
        Ok(!String::from_utf8_lossy(&buf).trim().is_empty())
    }

    /// The last `tail` lines of a service's container logs (stdout and
    /// stderr merged).
    async fn logs(&self, t: &ComposeTarget, service: &str, tail: usize) -> Result<Vec<String>> {
        let mut buf = Vec::new();
        let tail = tail.to_string();
        self.compose(
            t,
            &["logs", "--no-color", "--tail", &tail, service],
            Route::Buf,
            Route::Buf,
            None,
            Some(&mut buf),
        )
        .await?;
        let text = String::from_utf8_lossy(&buf);
        let text = text.trim_end_matches('\n');
        if text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(text.split('\n').map(str::to_string).collect())
    }
}

#[derive(Deserialize)]
struct Config {
    services: Option<HashMap<String, ServiceConfig>>,
    volumes: Option<HashMap<String, VolumeConfig>>,
}

#[derive(Deserialize)]
struct ServiceConfig {
    volumes: Option<Vec<Mount>>,
}

#[derive(Deserialize)]
struct Mount {
    r#type: Option<String>,
    source: Option<String>,
}

#[derive(Deserialize)]
struct VolumeConfig {
    name: Option<String>,
    external: Option<bool>,
}

/// Selects `service`'s nonexternal named mounts from the normalized Compose
/// model, sorted and de-duplicated. Bind and anonymous mounts are not
/// first-run data.
pub fn named_volumes_from_config(data: &[u8], service: &str) -> Result<Vec<String>> {
    let config: Config = serde_json::from_slice(data)
        .map_err(|e| Error::msg(format!("decode compose config: {e}")))?;
    let services = config.services.unwrap_or_default();
    let volumes = config.volumes.unwrap_or_default();
    let svc = services
        .get(service)
        .ok_or_else(|| Error::msg(format!("compose config has no service {service:?}")))?;
    let mut set = BTreeSet::new();
    for mount in svc.volumes.iter().flatten() {
        let source = mount.source.as_deref().unwrap_or_default();
        if mount.r#type.as_deref() != Some("volume") || source.is_empty() {
            continue;
        }
        let volume = volumes
            .get(source)
            .ok_or_else(|| Error::msg(format!("compose config has no volume {source:?}")))?;
        if volume.external.unwrap_or(false) {
            continue;
        }
        let name = volume.name.as_deref().unwrap_or_default();
        if name.is_empty() {
            return Err(Error::msg(format!(
                "compose volume {source:?} has no normalized name"
            )));
        }
        set.insert(name.to_string());
    }
    Ok(set.into_iter().collect())
}
