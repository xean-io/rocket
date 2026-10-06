use rocket_domain::ports::{Error, ProcessHandle, ProcessRunner, ProcessSpec, Result, async_trait};
use std::time::Duration;

/// Stub until Job Object support lands on Windows.
#[derive(Debug, Clone, Copy, Default)]
pub struct Runner;

#[async_trait]
impl ProcessRunner for Runner {
    fn start(&self, _spec: ProcessSpec) -> Result<ProcessHandle> {
        Err(Error::Unsupported)
    }

    async fn stop(&self, _pgid: i32, _grace: Duration) -> Result<()> {
        Err(Error::Unsupported)
    }

    fn alive(&self, _pid: i32, _pgid: i32) -> bool {
        false
    }
}
