//! End-to-end suite: drives the built `rocket` binary against a real daemon and
//! real subprocesses. Ported from the original Go integration suite (removed in R12; it lives in git history
//! before commit cd9cabe).
//!
//! Needs python3 and free local ports 18431-18433 and 18531. Run with
//! `cargo test -p rocket-cli --features e2e`.
#![cfg(unix)]

mod common;
mod end_to_end;
mod job_selection;
mod job_ttl;
mod jobs_and_ai;
mod pipeline_needs;
mod port_refs;
mod profiles;
