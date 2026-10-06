//! Concrete adapters for the rocket port traits (Go: `adapters/*`).
//!
//! One module per Go adapter package: [`sqlite`], [`events`], [`logs`],
//! [`process`], [`probe`], [`task`] and [`compose`]. Each one implements the
//! matching trait from [`rocket_domain::ports`] and keeps the Go behaviour
//! (argv, environment, timeouts, polling intervals, on-disk formats)
//! unchanged.
//!
//! # Composition root
//!
//! ```ignore
//! let bus = Arc::new(events::Bus::new());
//! let store = sqlite::Store::open(&home.join("state.db"))?;
//! let logs = logs::Sink::new(home.join("logs"), Some(bus.clone()));
//! let runner = process::Runner;
//! let ports = probe::Ports::new();
//! let health = probe::Health::new();
//! let task = task::Driver::new();
//! let compose = compose::Driver::new();
//! ```
//!
//! # Docker tests
//!
//! Tests that need a Docker daemon are `#[ignore]`d; run them with
//! `cargo test -p rocket-adapters -- --ignored`. They skip themselves when
//! `docker info` fails.

pub mod compose;
pub mod events;
pub mod logs;
pub mod probe;
pub mod process;
pub mod sqlite;
pub mod task;

mod lookup;
