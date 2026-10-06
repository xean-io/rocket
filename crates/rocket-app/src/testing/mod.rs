//! Test support: port fakes and the shared harness (Go: `fakes_test.go` and
//! the harness half of `app_test.go`). Compiled only for unit tests, so the
//! job tests of R6 can reuse it from inside this crate.

pub mod fakes;
pub mod harness;
pub mod projects;

pub use fakes::*;
pub use harness::*;
pub use projects::*;
