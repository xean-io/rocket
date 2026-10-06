//! Supervision of local processes: exit watchers for processes this daemon
//! started and liveness pollers for adopted ones (Go: `watchProcess`,
//! `adopt`, `onExit` in `up.go`).

use crate::app::{App, State, key};
use rocket_domain::ports::ProcessHandle;
use rocket_domain::{Health, Run, RunState};
use tokio::sync::watch;

/// Exit notification of a supervised process (Go: `watch`). `exited` holds
/// the exit code once the process is gone; adopted processes report `-1`.
#[derive(Clone)]
pub(crate) struct Watch {
    pub pid: i32,
    exited: watch::Receiver<Option<i32>>,
}

impl Watch {
    /// The exit code, once the process has exited.
    pub fn code(&self) -> Option<i32> {
        *self.exited.borrow()
    }

    pub fn is_exited(&self) -> bool {
        self.code().is_some()
    }

    /// Completes when the process has exited; never completes if the watcher
    /// was shut down before observing an exit.
    pub async fn wait_exit(&self) {
        let mut rx = self.exited.clone();
        if rx.wait_for(Option::is_some).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl App {
    /// Starts watching a process this daemon spawned. Needs the state lock,
    /// like Go (`a.mu` held), so the watch is registered before an exit can
    /// be observed.
    pub(crate) fn watch_process(
        &self,
        st: &mut State,
        project: &str,
        service: &str,
        h: ProcessHandle,
    ) -> Watch {
        let (tx, rx) = watch::channel(None);
        let w = Watch {
            pid: h.pid,
            exited: rx,
        };
        st.watches.insert(key(project, service), w.clone());
        let app = self.clone();
        let (project, service) = (project.to_string(), service.to_string());
        let pid = h.pid;
        let mut done = h.done;
        self.inner.tasks.spawn(async move {
            tokio::select! {
                // A dropped sender means the waiter thread died: report -1.
                code = &mut done => {
                    let code = code.unwrap_or(-1);
                    let _ = tx.send(Some(code));
                    app.on_exit(&project, &service, pid, code);
                }
                () = app.inner.shutdown.cancelled() => {}
            }
        });
        w
    }

    /// Polls a process started by a previous daemon until it disappears.
    /// Needs the state lock.
    pub(crate) fn adopt(&self, st: &mut State, r: &Run) {
        let (tx, rx) = watch::channel(None);
        st.watches.insert(
            key(&r.project, &r.service),
            Watch {
                pid: r.pid,
                exited: rx,
            },
        );
        let app = self.clone();
        let run = r.clone();
        self.inner.tasks.spawn(async move {
            let period = app.cfg().poll_interval;
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            loop {
                tokio::select! {
                    () = app.inner.shutdown.cancelled() => return,
                    _ = ticker.tick() => {
                        if !app.d().runner.alive(run.pid, run.pgid) {
                            let _ = tx.send(Some(-1));
                            app.on_exit(&run.project, &run.service, run.pid, -1);
                            return;
                        }
                    }
                }
            }
        });
    }

    /// Records the end of a supervised process.
    pub(crate) fn on_exit(&self, project: &str, service: &str, pid: i32, code: i32) {
        let mut st = self.lock();
        let k = key(project, service);
        if st.watches.get(&k).is_some_and(|w| w.pid == pid) {
            st.watches.remove(&k);
        }
        let Ok(Some(mut run)) = self.d().store.get_run(project, service) else {
            return;
        };
        if run.pid != pid {
            return;
        }
        match run.state {
            // `up` owns the outcome and observes the exit itself.
            RunState::Starting => {}
            RunState::Running => {
                run.state = RunState::Exited;
                run.health = Health::Unknown;
                run.stopped_at = Some(self.now());
                run.exit_code = Some(code);
                let _ = self.save_run(&run);
                self.release_leases(project, service);
            }
            _ => {
                if run.exit_code.is_none() {
                    run.exit_code = Some(code);
                    let _ = self.d().store.save_run(&run);
                }
            }
        }
    }
}
