//! SSE behaviour: ports of `events_integration_test.go` and
//! `job_logs_integration_test.go`, with the 15 s heartbeat shortened through
//! `Server::with_heartbeat`.

mod common;

use common::{Harness, data_events, event, open, serve};
use rocket_domain::ports::{EventBus, Store};
use rocket_domain::{Job, JobKind, JobStatus, RunState, event_type};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

fn job(id: &str, status: JobStatus, log_path: &Path) -> Job {
    Job {
        id: id.into(),
        project: "fixture".into(),
        name: "check".into(),
        kind: JobKind::Pipeline,
        env: String::new(),
        profiles: vec![],
        owner: "user".into(),
        steps: vec![],
        args: vec![],
        status,
        step: 0,
        pid: 0,
        pgid: 0,
        exit_code: None,
        started_at: time::OffsetDateTime::now_utc(),
        expires_at: None,
        finished_at: None,
        duration_ms: 0,
        log_path: log_path.to_str().unwrap().into(),
        error: String::new(),
    }
}

#[test]
fn default_heartbeat_is_fifteen_seconds() {
    let h = Harness::new();
    assert_eq!(h.server.heartbeat, Duration::from_secs(15));
}

#[tokio::test]
async fn events_stream_opens_with_ok_and_formats_frames() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events?project=p&types=service.state").await;
    assert_eq!(s.status, 200);
    assert_eq!(s.headers["content-type"], "text/event-stream");
    assert_eq!(s.headers["cache-control"], "no-cache");
    // Clients like URLSession only surface the response once body bytes arrive.
    s.read_until(": ok\n\n").await;

    h.bus.publish(event(event_type::LOG_LINE, "p", "api")); // filtered by type
    let mut other = event(event_type::SERVICE_STATE, "other", "api");
    other.state = Some(RunState::Running);
    h.bus.publish(other); // filtered by project
    let mut wanted = event(event_type::SERVICE_STATE, "p", "api");
    wanted.state = Some(RunState::Running);
    h.bus.publish(wanted);

    let text = s.read_until("\"state\":\"running\"}\n\n").await.to_string();
    assert_eq!(
        text,
        concat!(
            ": ok\n\n",
            "event: service.state\n",
            "data: {\"type\":\"service.state\",\"time\":\"2026-10-05T00:14:10Z\",",
            "\"project\":\"p\",\"service\":\"api\",\"state\":\"running\"}\n\n"
        )
    );
}

#[tokio::test]
async fn events_filter_by_job_service_and_types_list() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut by_job = open(addr, "/v1/events?job=j2").await;
    let mut by_service = open(addr, "/v1/events?service=web&types=%20log.line%20,,job.log").await;

    let mut other_job = event(event_type::JOB_LOG, "p", "");
    other_job.job_id = "j1".into();
    other_job.line = "other job".into();
    let mut mine = event(event_type::JOB_LOG, "p", "");
    mine.job_id = "j2".into();
    mine.line = "mine".into();
    let mut web_log = event(event_type::LOG_LINE, "p", "web");
    web_log.line = "web says hi".into();
    let api_log = event(event_type::LOG_LINE, "p", "api");
    let mut web_state = event(event_type::SERVICE_STATE, "p", "web");
    web_state.state = Some(RunState::Running);
    for e in [other_job, mine, web_log, api_log, web_state] {
        h.bus.publish(e);
    }

    by_job.read_until("\"line\":\"mine\"").await;
    let text = by_job
        .read_quiet(Duration::from_millis(200))
        .await
        .to_string();
    assert_eq!(data_events(&text).len(), 1, "{text}");
    assert!(text.contains("event: job.log\n"), "{text}");

    by_service.read_until("web says hi").await;
    let text = by_service
        .read_quiet(Duration::from_millis(200))
        .await
        .to_string();
    let evs = data_events(&text);
    assert_eq!(evs.len(), 1, "{text}");
    assert_eq!(evs[0]["service"], "web");
}

#[tokio::test]
async fn no_filters_stream_everything_in_order() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events").await;
    for i in 0..5 {
        let mut e = event(event_type::LOG_LINE, "p", "api");
        e.line = format!("line {i}");
        h.bus.publish(e);
    }
    s.read_until("line 4").await;
    let lines: Vec<String> = data_events(&s.buf)
        .iter()
        .map(|e| e["line"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(lines, ["line 0", "line 1", "line 2", "line 3", "line 4"]);
}

#[tokio::test]
async fn sse_data_is_html_escaped_like_json_marshal() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events").await;
    let mut e = event(event_type::LOG_LINE, "p", "api");
    e.line = "<b>&\u{2028}\u{2029}</b>".into();
    h.bus.publish(e);
    let text = s.read_until("\"}\n\n").await.to_string();
    assert!(
        text.contains(r#""line":"\u003cb\u003e\u0026\u2028\u2029\u003c/b\u003e""#),
        "{text}"
    );
}

#[tokio::test]
async fn heartbeat_pings_follow_the_ok_comment() {
    let h = Harness::with_heartbeat(Duration::from_millis(100));
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events").await;
    let started = Instant::now();
    s.read_until(": ok\n\n: ping\n\n: ping\n\n").await;
    // The first ping comes after one full interval, not immediately.
    assert!(
        started.elapsed() >= Duration::from_millis(180),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn slow_consumers_drop_events_without_blocking_the_publisher() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events").await;
    // The client does not read; the bus must never block the publisher.
    let started = Instant::now();
    for i in 0..5000 {
        let mut e = event(event_type::LOG_LINE, "p", "api");
        e.line = format!("n{i}");
        h.bus.publish(e);
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "publisher blocked"
    );
    let text = s.read_quiet(Duration::from_millis(500)).await.to_string();
    let delivered = data_events(&text).len();
    assert!(delivered > 0 && delivered < 5000, "delivered {delivered}");
    // Whatever arrives is in publish order, starting with the oldest event.
    assert!(text.contains("\"line\":\"n0\""));
}

#[tokio::test]
async fn dropping_the_client_releases_the_subscription() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events").await;
    s.read_until(": ok\n\n").await;
    assert_eq!(h.bus.subscriber_count(), 1);
    drop(s);
    for _ in 0..200 {
        if h.bus.subscriber_count() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("subscription leaked after the client went away");
}

#[tokio::test]
async fn closing_the_server_ends_open_streams() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let mut s = open(addr, "/v1/events").await;
    s.read_until(": ok\n\n").await;
    h.server.closing().cancel();
    let text = s.read_to_end().await;
    assert_eq!(text, ": ok\n\n");
}

#[tokio::test]
async fn log_follow_sends_tail_then_live_lines_of_that_service() {
    let h = Harness::new();
    let dir = h.fixture();
    let (addr, _srv) = serve(h.router()).await;
    let add = common::call(
        &h.router(),
        "POST",
        "/v1/projects",
        Some(&format!(r#"{{"path":"{}"}}"#, dir.display())),
    )
    .await;
    assert_eq!(add.status, 200);
    let log_dir = h.tmp.path().join("logs/rocket-fixture");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("sleeper.log"), "one\ntwo\nthree\n").unwrap();

    let mut s = open(
        addr,
        "/v1/logs?project=rocket-fixture&service=sleeper&tail=2&follow=true",
    )
    .await;
    assert_eq!(s.status, 200);
    s.read_until("three").await;
    let evs = data_events(&s.buf);
    let lines: Vec<&str> = evs.iter().map(|e| e["line"].as_str().unwrap()).collect();
    assert_eq!(lines, ["two", "three"], "{}", s.buf);
    assert!(s.buf.starts_with(": ok\n\n"));
    assert!(evs.iter().all(|e| e["type"] == "log.line"
        && e["project"] == "rocket-fixture"
        && e["service"] == "sleeper"));

    // Live: only log.line of this service passes.
    let mut live = event(event_type::LOG_LINE, "rocket-fixture", "sleeper");
    live.line = "live".into();
    let mut other = event(event_type::LOG_LINE, "rocket-fixture", "static");
    other.line = "other service".into();
    let mut state = event(event_type::SERVICE_STATE, "rocket-fixture", "sleeper");
    state.state = Some(RunState::Running);
    for e in [other, state, live] {
        h.bus.publish(e);
    }
    s.read_until("\"line\":\"live\"").await;
    assert!(
        !s.buf.contains("other service") && !s.buf.contains("service.state"),
        "{}",
        s.buf
    );
}

#[tokio::test]
async fn log_follow_errors_are_json_not_streams() {
    let h = Harness::new();
    let resp = common::call(
        &h.router(),
        "GET",
        "/v1/logs?follow=true&project=nope&service=x",
        None,
    )
    .await;
    assert_eq!(resp.status, 404);
    assert_eq!(resp.header("content-type"), "application/json");
    assert_eq!(
        h.bus.subscriber_count(),
        0,
        "the early subscription must be released"
    );
}

#[tokio::test]
async fn job_follow_heartbeat_and_trailing_terminal_ordering() {
    // Go: TestJobFollowHeartbeatAndTrailingTerminalOrdering.
    let h = Harness::with_heartbeat(Duration::from_millis(150));
    let (addr, _srv) = serve(h.router()).await;
    let path = h.tmp.path().join("job.log");
    std::fs::write(&path, "initial\npartial").unwrap();
    h.store
        .save_job(&job("jstream", JobStatus::Running, &path))
        .unwrap();

    let started = Instant::now();
    let mut s = open(addr, "/v1/jobs/jstream/logs?follow=true&tail=1").await;
    assert_eq!(s.headers["content-type"], "text/event-stream");
    s.read_until(": ping\n\n").await;
    assert!(
        started.elapsed() >= Duration::from_millis(120),
        "{:?}",
        started.elapsed()
    );

    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    f.write_all(b"-end").unwrap();
    drop(f);
    h.store
        .save_job(&job("jstream", JobStatus::Succeeded, &path))
        .unwrap();

    let text = s.read_to_end().await.to_string();
    let events = data_events(&text);
    let terminals = events
        .iter()
        .filter(|e| e["type"] == "job.state" && e["status"] != "running")
        .count();
    assert_eq!(terminals, 1, "{text}");
    assert_eq!(events.len(), 3, "{text}");
    assert_eq!(events[0]["line"], "initial");
    assert_eq!(events[0]["type"], "job.log");
    assert_eq!(events[0]["job_id"], "jstream");
    assert_eq!(events[0]["project"], "fixture");
    assert_eq!(events[1]["line"], "partial-end");
    assert_eq!(events[2]["status"], "succeeded");
    assert_eq!(events[2]["job"]["id"], "jstream");
    // Nothing (no heartbeat, no log) follows the terminal event.
    let last_data = text.rfind("data: ").unwrap();
    let tail = &text[last_data..];
    assert_eq!(
        tail.matches("\n\n").count(),
        1,
        "frame after terminal: {tail:?}"
    );
    assert!(tail.ends_with("\n\n"), "{tail:?}");
}

#[tokio::test]
async fn job_follow_of_a_finished_job_drains_and_closes() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;
    let path = h.tmp.path().join("done.log");
    std::fs::write(&path, "a\nb\nlast without newline").unwrap();
    h.store
        .save_job(&job("jdone", JobStatus::Failed, &path))
        .unwrap();

    let mut s = open(addr, "/v1/jobs/jdone/logs?follow=true").await;
    let text = s.read_to_end().await.to_string();
    let events = data_events(&text);
    let lines: Vec<&str> = events
        .iter()
        .filter(|e| e["type"] == "job.log")
        .map(|e| e["line"].as_str().unwrap())
        .collect();
    assert_eq!(lines, ["a", "b", "last without newline"], "{text}");
    assert_eq!(events.last().unwrap()["type"], "job.state");
    assert_eq!(events.last().unwrap()["status"], "failed");
    assert!(text.starts_with(": ok\n\nevent: job.log\n"), "{text}");
    assert!(text.contains("\nevent: job.state\ndata: "), "{text}");

    // tail=1: only the last complete line, then the partial remainder.
    let mut s = open(addr, "/v1/jobs/jdone/logs?follow=true&tail=1").await;
    let events = data_events(s.read_to_end().await);
    let lines: Vec<&str> = events
        .iter()
        .filter(|e| e["type"] == "job.log")
        .map(|e| e["line"].as_str().unwrap())
        .collect();
    assert_eq!(lines, ["b", "last without newline"]);
}

#[tokio::test]
async fn job_follow_errors() {
    let h = Harness::new();
    let (addr, _srv) = serve(h.router()).await;

    let mut s = open(addr, "/v1/jobs/jnope/logs?follow=true").await;
    assert_eq!(s.status, 404);
    assert_eq!(s.headers["content-type"], "application/json");
    let body = s.read_to_end().await.to_string();
    assert!(
        body.contains(r#""error": "not found: job \"jnope\"""#),
        "{body}"
    );

    // A missing log file ends the stream right after the opening comment.
    h.store
        .save_job(&job(
            "jnolog",
            JobStatus::Succeeded,
            &h.tmp.path().join("missing.log"),
        ))
        .unwrap();
    let mut s = open(addr, "/v1/jobs/jnolog/logs?follow=true").await;
    assert_eq!(s.status, 200);
    assert_eq!(s.read_to_end().await, ": ok\n\n");
}

#[tokio::test]
async fn job_logs_tail_defaults_and_unknown_job() {
    let h = Harness::new();
    // The log sink resolves job logs by project and id.
    let path = h.tmp.path().join("logs/fixture/jobs/jtail.log");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content: String = (0..150).map(|i| format!("l{i}\n")).collect();
    std::fs::write(&path, content).unwrap();
    h.store
        .save_job(&job("jtail", JobStatus::Succeeded, &path))
        .unwrap();
    // Default tail is 100 lines.
    let resp = common::call(&h.router(), "GET", "/v1/jobs/jtail/logs", None).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["lines"].as_array().unwrap().len(), 100);
    let resp = common::call(&h.router(), "GET", "/v1/jobs/jtail/logs?tail=3", None).await;
    assert_eq!(
        resp.json()["lines"],
        serde_json::json!(["l147", "l148", "l149"])
    );
    let resp = common::call(&h.router(), "GET", "/v1/jobs/jtail/logs?tail=-5", None).await;
    assert_eq!(resp.json()["lines"].as_array().unwrap().len(), 100);
}
