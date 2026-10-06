//! rocketd's HTTP/SSE API (Go: `adapters/api`), served with axum.
//! The contract is `crates/rocket-api/README.md`.

pub mod auth;
pub mod decode;
pub mod error;
pub mod json;
pub mod query;
mod server;
mod sse;
mod streams;

pub use error::ApiError;
pub use server::{HEARTBEAT, Server, VERSION};
