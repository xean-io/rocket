//! rocket's core model and the port traits the application core depends on.
//!
//! This crate is a faithful port of Go's `internal/domain` and
//! `internal/ports`. It performs no I/O: no clocks (callers pass `now`), no
//! processes, no filesystem access. Module names mirror the Go files:
//! [`project`], [`run`], [`job`], [`graph`], [`port_refs`], and [`ports`].
//!
//! # JSON compatibility
//!
//! Every type the Go daemon serializes (API wire format, SQLite `data` blobs)
//! encodes byte-for-byte like Go's `encoding/json`:
//!
//! * field names and order follow the Go struct tags;
//! * `omitempty` maps to `skip_serializing_if` with Go's zero-value rules
//!   (empty string, `0`, `false`, empty/nil slice or map, `None` pointer);
//!   non-pointer `time.Time` and `Option`-less fields are never omitted;
//! * maps are `BTreeMap`, matching Go's sorted map keys;
//! * times use RFC3339 with trailing-zero-trimmed fractional seconds
//!   (Go's `RFC3339Nano`), and a missing time decodes to Go's zero time
//!   `0001-01-01T00:00:00Z`;
//! * `time.Duration` is a nanosecond integer;
//! * decoding is lenient like Go: unknown fields are ignored and `null` for a
//!   slice or map yields an empty one.
//!
//! One unavoidable difference: Go distinguishes a nil slice (`null`) from an
//! empty one (`[]`) when there is no `omitempty`; Rust always writes `[]`.
//!
//! # Async ports: `async-trait`
//!
//! The composition root injects adapters as `Arc<dyn Trait>`, and tests inject
//! in-memory fakes the same way. Native `async fn` in traits is not
//! dyn-compatible, so the few ports that perform slow work (process stop,
//! compose, health probes) use the `async-trait` crate: it boxes the returned
//! future, which costs one allocation per call and is irrelevant next to a
//! process or network round trip. The alternative (hand-written
//! `Pin<Box<dyn Future>>` signatures) is the same thing with more noise, and
//! an enum-dispatch scheme would stop fakes from living in test code. Ports
//! that are cheap or inherently blocking (store, event bus, log sink,
//! manifest) stay synchronous, as in Go. Go's `context.Context` becomes future
//! cancellation: dropping the returned future abandons the call.

mod serde_util;

pub mod error;
pub mod graph;
pub mod job;
pub mod port_refs;
pub mod ports;
pub mod project;
pub mod run;

pub use error::Error;
pub use graph::{ALL_SERVICES, merge_profiles};
pub use job::{Job, JobKind, JobStatus, is_agent_owner};
pub use port_refs::{PortReference, port_references};
pub use project::{
    DEFAULT_HEALTH_TIMEOUT, DEFAULT_OWNER, Deploy, Environment, HealthCheck, HealthSpec,
    PortEnvBinding, PortSpec, Project, ProjectRef, Service, ServiceKind, Step, health_check_for,
};
pub use run::{
    Event, Health, Lease, PortHolder, PortRemap, Run, RunState, build_env, compose_project_name,
    event_type, remap_candidates,
};
