//! Port traits must be usable as `Arc<dyn Trait>` from a tokio composition
//! root and easy to fake in tests.

use rocket_domain::ports::{
    ComposeDriver, ComposeTarget, Error, EventBus, HealthProbe, ManifestLoader, PortProbe,
    ProcessHandle, ProcessRunner, ProcessSpec, Store, Subscription, TaskDriver,
};
use rocket_domain::{Event, HealthCheck, event_type};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[test]
fn sentinel_errors_are_typed_and_worded_like_go() {
    assert_eq!(
        Error::Unsupported.to_string(),
        "not supported on this platform"
    );
    assert_eq!(Error::LeaseTaken.to_string(), "port already leased");
    assert!(matches!(Error::msg("boom"), Error::Other(_)));
    assert_eq!(Error::msg("boom").to_string(), "boom");
    let io = std::io::Error::other("disk");
    assert_eq!(Error::from(io).to_string(), "disk");
}

#[derive(Default)]
struct FakeRunner {
    stopped: Mutex<Vec<(i32, Duration)>>,
}

#[async_trait::async_trait]
impl ProcessRunner for FakeRunner {
    fn start(&self, _spec: ProcessSpec) -> Result<ProcessHandle, Error> {
        let (tx, rx) = oneshot::channel();
        tx.send(7).unwrap();
        Ok(ProcessHandle {
            pid: 10,
            pgid: 10,
            done: rx,
        })
    }
    async fn stop(&self, pgid: i32, grace: Duration) -> Result<(), Error> {
        self.stopped.lock().unwrap().push((pgid, grace));
        Ok(())
    }
    fn alive(&self, pid: i32, pgid: i32) -> bool {
        pid == pgid
    }
}

struct FakeProbe;

#[async_trait::async_trait]
impl HealthProbe for FakeProbe {
    async fn check(&self, c: &HealthCheck) -> Result<(), Error> {
        if c.port == 1 {
            Err(Error::msg("refused"))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct FakeBus {
    subs: Mutex<Vec<mpsc::Sender<Event>>>,
}

impl EventBus for FakeBus {
    fn publish(&self, event: Event) {
        for s in self.subs.lock().unwrap().iter() {
            let _ = s.try_send(event.clone());
        }
    }
    fn subscribe(&self, buffer: usize) -> Subscription {
        let (tx, rx) = mpsc::channel(buffer);
        self.subs.lock().unwrap().push(tx);
        Subscription::new(rx, || {})
    }
}

#[tokio::test]
async fn async_ports_work_behind_arc_dyn() {
    let runner: Arc<dyn ProcessRunner> = Arc::new(FakeRunner::default());
    let handle = runner
        .start(ProcessSpec {
            argv: vec!["true".into()],
            dir: Path::new("/").to_path_buf(),
            env: vec![],
            output: None,
        })
        .unwrap();
    assert_eq!(handle.done.await.unwrap(), 7);
    // Shareable across spawned tasks (Send + Sync + 'static).
    let r2 = runner.clone();
    tokio::spawn(async move { r2.stop(10, Duration::from_secs(1)).await })
        .await
        .unwrap()
        .unwrap();
    assert!(runner.alive(3, 3));

    let probe: Arc<dyn HealthProbe> = Arc::new(FakeProbe);
    let check = |port| HealthCheck {
        kind: "tcp".into(),
        port,
        path: String::new(),
    };
    assert!(probe.check(&check(80)).await.is_ok());
    assert_eq!(
        probe.check(&check(1)).await.unwrap_err().to_string(),
        "refused"
    );
}

#[tokio::test]
async fn event_bus_subscription_receives_and_runs_cancel_on_drop() {
    let bus: Arc<dyn EventBus> = Arc::new(FakeBus::default());
    let mut sub = bus.subscribe(4);
    let ev: Event = serde_json::from_str(r#"{"type":"log.line","line":"hi"}"#).unwrap();
    bus.publish(ev);
    let got = sub.recv().await.unwrap();
    assert_eq!(got.r#type, event_type::LOG_LINE);

    let cancelled = Arc::new(Mutex::new(false));
    let flag = cancelled.clone();
    let (_tx, rx) = mpsc::channel::<Event>(1);
    let sub = Subscription::new(rx, move || *flag.lock().unwrap() = true);
    drop(sub);
    assert!(*cancelled.lock().unwrap());
}

// Compile-time proof that the remaining ports are dyn-compatible.
#[allow(dead_code)]
fn dyn_compatible(
    _: Option<Arc<dyn ManifestLoader>>,
    _: Option<Arc<dyn Store>>,
    _: Option<Arc<dyn ComposeDriver>>,
    _: Option<Arc<dyn TaskDriver>>,
    _: Option<Arc<dyn PortProbe>>,
    _: Option<ComposeTarget>,
) {
}
