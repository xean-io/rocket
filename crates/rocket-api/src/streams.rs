//! The three SSE endpoints: `/v1/events`, `/v1/logs?follow=true` and
//! `/v1/jobs/{id}/logs?follow=true` (Go: `events`, `logs`, `jobLogs`).

use crate::error::ApiError;
use crate::query::Query;
use crate::server::{Server, blocking};
use crate::sse::{self, Writer};
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use rocket_adapters::logs::follow_file;
use rocket_app::LogsRequest;
use rocket_domain::ports::Subscription;
use rocket_domain::{Event, event_type};
use std::collections::HashSet;
use std::io;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

/// Buffer of every bus subscription made by the API (Go: `Subscribe(1024)`).
const SUBSCRIBER_BUFFER: usize = 1024;
/// How often the job-log follower looks at the file (Go: 100ms).
const FOLLOW_POLL: Duration = Duration::from_millis(100);

/// AND-combined event filter; empty fields match everything.
#[derive(Debug, Default)]
pub(crate) struct Filter {
    project: String,
    service: String,
    job: String,
    types: HashSet<String>,
}

impl Filter {
    fn from_query(q: &Query) -> Self {
        Self {
            project: q.get("project").to_string(),
            service: q.get("service").to_string(),
            job: q.get("job").to_string(),
            types: q
                .get("types")
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }

    pub(crate) fn matches(&self, e: &Event) -> bool {
        (self.project.is_empty() || e.project == self.project)
            && (self.job.is_empty() || e.job_id == self.job)
            && (self.service.is_empty() || e.service == self.service)
            && (self.types.is_empty() || self.types.contains(&e.r#type))
    }
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

fn base_event(kind: &str) -> Event {
    Event {
        r#type: kind.to_string(),
        time: now(),
        project: String::new(),
        service: String::new(),
        state: None,
        line: String::new(),
        run: None,
        lease: None,
        job_id: String::new(),
        status: None,
        job: None,
    }
}

/// A heartbeat ticker whose first tick comes after one full period.
fn heartbeat(period: Duration) -> tokio::time::Interval {
    let mut t = interval_at(Instant::now() + period, period);
    t.set_missed_tick_behavior(MissedTickBehavior::Skip);
    t
}

/// Forwards matching bus events and `: ping` heartbeats until the client or
/// the bus goes away.
async fn pump(out: Writer, mut sub: Subscription, filter: Filter, period: Duration) {
    let mut ping = heartbeat(period);
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if !out.comment("ping").await {
                    return;
                }
            }
            e = sub.recv() => match e {
                None => return,
                Some(e) => {
                    if filter.matches(&e) && !out.event(&e).await {
                        return;
                    }
                }
            },
        }
    }
}

/// `GET /v1/events`.
pub(crate) async fn events(State(s): State<Server>, RawQuery(raw): RawQuery) -> Response {
    let filter = Filter::from_query(&Query::parse(raw.as_deref()));
    // Subscribe before answering, so nothing published after the client sees
    // the response is missed.
    let sub = s.bus.subscribe(SUBSCRIBER_BUFFER);
    let period = s.heartbeat;
    sse::response(s.closing(), move |out| pump(out, sub, filter, period))
}

/// `GET /v1/logs?follow=true`: the tail first, then live `log.line` events.
pub(crate) async fn follow_logs(s: Server, req: LogsRequest) -> Result<Response, ApiError> {
    // Subscribe before reading the tail so no line falls in between.
    let sub = s.bus.subscribe(SUBSCRIBER_BUFFER);
    let res = s.app.logs(req).await?;
    let filter = Filter {
        project: res.project.clone(),
        service: res.service.clone(),
        job: String::new(),
        types: HashSet::from([event_type::LOG_LINE.to_string()]),
    };
    let period = s.heartbeat;
    Ok(sse::response(s.closing(), move |out| async move {
        for line in res.lines {
            let mut e = base_event(event_type::LOG_LINE);
            e.project.clone_from(&res.project);
            e.service.clone_from(&res.service);
            e.line = line;
            if !out.event(&e).await {
                return;
            }
        }
        pump(out, sub, filter, period).await;
    }))
}

/// `GET /v1/jobs/{id}/logs?follow=true`: the job log read straight from its
/// file (no gaps, no duplicates), then one terminal `job.state` and EOF.
pub(crate) async fn follow_job_logs(
    s: Server,
    id: String,
    tail: i64,
) -> Result<Response, ApiError> {
    let app = s.app.clone();
    let job = {
        let (app, id) = (app.clone(), id.clone());
        blocking(move || app.get_job(&id)).await?
    };
    let period = s.heartbeat;
    Ok(sse::response(s.closing(), move |out| async move {
        let finished = {
            let (app, id) = (app.clone(), id.clone());
            move || app.get_job(&id).map_or(true, |j| j.status.terminal())
        };
        let emit = |line: String| {
            let mut e = base_event(event_type::JOB_LOG);
            e.project.clone_from(&job.project);
            e.job_id.clone_from(&id);
            e.line = line;
            let out = &out;
            async move {
                if out.event(&e).await {
                    Ok(())
                } else {
                    Err(io::Error::other("client gone").into())
                }
            }
        };
        let follow = follow_file(
            std::path::Path::new(&job.log_path),
            tail,
            FOLLOW_POLL,
            finished,
            emit,
        );
        tokio::pin!(follow);
        let mut ping = heartbeat(period);
        let result = loop {
            tokio::select! {
                r = &mut follow => break r,
                _ = ping.tick() => {
                    if !out.comment("ping").await {
                        return;
                    }
                }
            }
        };
        // The follower drained everything before returning, and this task is
        // the only writer: the terminal state is strictly the last frame.
        if result.is_err() {
            return;
        }
        if let Ok(fin) = app.get_job(&id)
            && fin.status.terminal()
        {
            let mut e = base_event(event_type::JOB_STATE);
            e.project.clone_from(&fin.project);
            e.job_id.clone_from(&id);
            e.status = Some(fin.status);
            e.job = Some(Box::new(fin));
            out.event(&e).await;
        }
    }))
}

/// Handler for `GET /v1/jobs/{id}/logs`.
pub(crate) async fn job_logs(
    State(s): State<Server>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let q = Query::parse(raw.as_deref());
    let tail = q.int("tail");
    if !q.flag("follow") {
        let n = usize::try_from(tail.unwrap_or(0)).unwrap_or(0);
        let app = s.app.clone();
        let res = blocking(move || app.job_logs(&id, n)).await?;
        return Ok(crate::server::ok(&res));
    }
    // follow without a (valid) tail: the whole log.
    follow_job_logs(s, id, tail.unwrap_or(-1)).await
}
