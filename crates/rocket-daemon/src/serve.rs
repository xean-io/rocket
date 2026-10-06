//! [`run`]: the daemon's life cycle (Go: `daemon.Run`).

use crate::log::logf;
use crate::{RunOptions, SHUTDOWN_GRACE, lock, token};
use anyhow::{Context, Result, anyhow, bail};
use rocket_adapters::{compose, events, logs, probe, process, sqlite, task};
use rocket_api::{Server, VERSION, auth};
use rocket_app::{App, Deps, Options};
use rocket_client::{DaemonInfo, MAX_SOCKET_PATH, Paths};
use rocket_domain::api::HealthInfo;
use rocket_domain::ports::EventBus;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval_at, timeout};
use tokio_util::sync::CancellationToken;

/// Removes the files a running daemon owns, on every exit path (Go: the
/// deferred `os.Remove`s).
struct Files<'a> {
    paths: &'a Paths,
    daemon_json: bool,
    socket_pid: bool,
}

impl Files<'_> {
    fn remove_daemon_json(&mut self) {
        if std::mem::take(&mut self.daemon_json) {
            let _ = std::fs::remove_file(&self.paths.daemon_json);
        }
    }

    fn remove_socket_and_pid(&mut self) {
        if std::mem::take(&mut self.socket_pid) {
            let _ = std::fs::remove_file(&self.paths.pid_file);
            let _ = std::fs::remove_file(&self.paths.socket);
        }
    }
}

impl Drop for Files<'_> {
    fn drop(&mut self) {
        self.remove_daemon_json();
        self.remove_socket_and_pid();
    }
}

/// Serves until SIGINT/SIGTERM (with [`RunOptions::handle_signals`]),
/// [`RunOptions::stop`] or `POST /v1/shutdown`, then stops accepting,
/// removes `daemon.json`, the socket and the pid file, and closes the
/// application. Supervised services keep running and are adopted by the next
/// daemon.
pub async fn run(opts: RunOptions) -> Result<()> {
    let RunOptions {
        paths,
        version,
        stop,
        handle_signals,
        ttl_interval,
    } = opts;
    // POST /v1/shutdown cancels only this child token. Signal handlers are
    // installed before anything is published (socket, daemon.json), so a
    // client that sees the daemon answer can already stop it gracefully.
    let stop = stop.child_token();
    let _stop_on_exit = stop.clone().drop_guard();
    if handle_signals {
        spawn_signal_handlers(&stop)?;
    }
    if paths.socket.as_os_str().len() > MAX_SOCKET_PATH {
        bail!(
            "socket path {} is too long for a unix socket; set ROCKET_HOME to a shorter directory",
            paths.socket.display()
        );
    }
    paths.ensure()?;
    let _lock = lock::acquire(&paths.lock_file)
        .map_err(|e| anyhow!("another rocketd holds {}: {e}", paths.lock_file.display()))?;

    if let Ok(Ok(_)) = timeout(
        Duration::from_millis(500),
        UnixStream::connect(&paths.socket),
    )
    .await
    {
        bail!("rocketd is already running");
    }
    let _ = std::fs::remove_file(&paths.socket); // stale socket from a crashed daemon

    let store = Arc::new(sqlite::Store::open(&paths.db)?);
    let bus = Arc::new(events::Bus::new());
    let sink = Arc::new(logs::Sink::new(
        paths.logs.clone(),
        Some(bus.clone() as Arc<dyn EventBus>),
    ));
    let app = App::new(Deps {
        store,
        manifests: Arc::new(rocket_manifest::Loader),
        env: Arc::new(rocket_manifest::EnvFiles),
        runner: Arc::new(process::Runner),
        compose: Arc::new(compose::Driver::new()),
        tasks: Arc::new(task::Driver::new()),
        probe: Arc::new(probe::Ports::new()),
        health: Arc::new(probe::Health::new()),
        bus: Some(bus.clone() as Arc<dyn EventBus>),
        logs: sink.clone(),
        options: Options::default(),
    });

    match app.reconcile().await {
        Ok(rec) => logf!(
            "reconcile: adopted={} dead={} released_leases={} lost_jobs={}",
            rec.adopted.len(),
            rec.dead.len(),
            rec.released_leases.len(),
            rec.lost_jobs.len()
        ),
        Err(e) => logf!("reconcile: {e}"),
    }

    let mut files = Files {
        paths: &paths,
        daemon_json: false,
        socket_pid: false,
    };
    let unix_listener = UnixListener::bind(&paths.socket)
        .with_context(|| format!("listen on {}", paths.socket.display()))?;
    files.socket_pid = true;
    let _ = std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o600));
    write_pid_file(&paths.pid_file);

    // The TCP listener (for GUI clients) serves the same API behind a bearer
    // token; the unix socket (mode 0600) stays token-free for the CLI.
    let started_at = OffsetDateTime::now_utc();
    let pid = std::process::id();
    let tcp = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => Some(l),
        Err(e) => {
            logf!("tcp listener disabled: {e}");
            None
        }
    };
    let http_url = tcp
        .as_ref()
        .and_then(|l| l.local_addr().ok())
        .map(|a| format!("http://{a}"))
        .unwrap_or_default();

    let server = {
        let stop = stop.clone();
        Server::new(
            app.clone(),
            bus.clone(),
            HealthInfo {
                ok: true,
                api: VERSION.to_string(),
                version: version.clone(),
                pid: i32::try_from(pid).unwrap_or(0),
                started_at,
                home: paths.home.to_string_lossy().into_owned(),
                socket: paths.socket.to_string_lossy().into_owned(),
                http: http_url.clone(),
            },
        )
        .on_shutdown(move || stop.cancel())
    };
    let router = server.router();
    let tcp_router = if tcp.is_some() {
        let token = token::new_token().context("generate the API token")?;
        let info = DaemonInfo {
            version: version.clone(),
            api: VERSION.to_string(),
            pid: i32::try_from(pid).unwrap_or(0),
            socket: paths.socket.to_string_lossy().into_owned(),
            http: http_url.clone(),
            token: token.clone(),
            rocket_bin: executable(),
            started_at,
        };
        info.write(&paths.daemon_json)
            .with_context(|| format!("write {}", paths.daemon_json.display()))?;
        files.daemon_json = true;
        Some(auth::require_token(router.clone(), &token))
    } else {
        None
    };

    let ticker = spawn_ttl_ticker(app.clone(), stop.clone(), ttl_interval);

    // Both listeners drain gracefully once `serving` is cancelled.
    let serving = CancellationToken::new();
    let (fail_tx, mut fail_rx) = tokio::sync::mpsc::channel::<std::io::Error>(2);
    let mut servers: Vec<JoinHandle<()>> = Vec::new();
    {
        let (serving, fail) = (serving.clone(), fail_tx.clone());
        servers.push(tokio::spawn(async move {
            let r = axum::serve(unix_listener, router)
                .with_graceful_shutdown(serving.cancelled_owned())
                .await;
            if let Err(e) = r {
                let _ = fail.send(e).await;
            }
        }));
    }
    if let (Some(listener), Some(router)) = (tcp, tcp_router) {
        let (serving, fail) = (serving.clone(), fail_tx.clone());
        servers.push(tokio::spawn(async move {
            let r = axum::serve(listener, router)
                .with_graceful_shutdown(serving.cancelled_owned())
                .await;
            if let Err(e) = r {
                let _ = fail.send(e).await;
            }
        }));
    }
    drop(fail_tx);
    logf!(
        "rocketd {version} listening on {} and {} (pid {pid})",
        paths.socket.display(),
        if http_url.is_empty() {
            "(no tcp)"
        } else {
            &http_url
        }
    );

    let outcome: Result<()> = tokio::select! {
        () = stop.cancelled() => Ok(()),
        Some(e) = fail_rx.recv() => Err(e.into()),
    };

    // Remove the discovery file before the listeners close, so a client that
    // sees the socket go away never finds a stale daemon.json.
    files.remove_daemon_json();
    stop.cancel();
    server.closing().cancel(); // end open SSE streams so draining can finish
    serving.cancel();
    let drain = async {
        for s in &mut servers {
            let _ = s.await;
        }
    };
    if timeout(SHUTDOWN_GRACE, drain).await.is_err() {
        for s in &servers {
            s.abort();
        }
    }
    let _ = ticker.await;
    files.remove_socket_and_pid();
    app.close().await;
    let sink_for_close = sink.clone();
    let _ = tokio::task::spawn_blocking(move || sink_for_close.close()).await;
    drop(files);
    if outcome.is_ok() {
        logf!(
            "rocketd stopped; supervised services keep running and will be adopted on next start"
        );
    }
    outcome
}

fn write_pid_file(path: &Path) {
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(std::process::id().to_string().as_bytes())
    };
    let _ = write();
}

/// The absolute, symlink-resolved path of this binary (Go: `executable`).
fn executable() -> String {
    std::env::current_exe()
        .map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// SIGINT/SIGTERM stop the daemon; SIGHUP is ignored (Go: `signal.Ignore`).
fn spawn_signal_handlers(stop: &CancellationToken) -> Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    let mut hup = signal(SignalKind::hangup())?;
    let stop = stop.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = term.recv() => logf!("received terminated, shutting down"),
            _ = int.recv() => logf!("received interrupt, shutting down"),
            () = stop.cancelled() => return,
        }
        stop.cancel();
    });
    tokio::spawn(async move { while hup.recv().await.is_some() {} });
    Ok(())
}

/// Checks persisted job and service deadlines every `period`.
fn spawn_ttl_ticker(app: App, stop: CancellationToken, period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = interval_at(tokio::time::Instant::now() + period, period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = tick.tick() => {}
            }
            match app.expire_jobs_ttl().await {
                Ok(expired) => {
                    for job in expired {
                        logf!("ttl expired: job {} ({}/{})", job.id, job.project, job.name);
                    }
                }
                Err(e) => logf!("job ttl: {e}"),
            }
            match app.expire_ttl().await {
                Ok(expired) => {
                    for r in expired {
                        logf!(
                            "ttl expired: {}/{} (owner {})",
                            r.project,
                            r.service,
                            r.owner
                        );
                    }
                }
                Err(e) => logf!("ttl: {e}"),
            }
        }
    })
}
