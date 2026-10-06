//! Reconnect schedule of the event-stream supervisor.

use std::time::Duration;

const BASE: Duration = Duration::from_millis(250);
const CAP: Duration = Duration::from_secs(10);

/// Exponential backoff: 250ms, 500ms, 1s, 2s, 4s, 8s, then 10s forever.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// The delay before the next attempt; advances the schedule.
    pub fn next_delay(&mut self) -> Duration {
        let factor = 1u32.checked_shl(self.attempt).unwrap_or(u32::MAX);
        self.attempt = self.attempt.saturating_add(1);
        BASE.saturating_mul(factor).min(CAP)
    }

    /// Restarts the schedule after a successful connection.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(b: &mut Backoff, n: usize) -> Vec<u128> {
        (0..n).map(|_| b.next_delay().as_millis()).collect()
    }

    #[test]
    fn doubles_until_the_cap() {
        let mut b = Backoff::new();
        assert_eq!(
            ms(&mut b, 9),
            [250, 500, 1000, 2000, 4000, 8000, 10000, 10000, 10000]
        );
    }

    #[test]
    fn reset_restarts_the_schedule() {
        let mut b = Backoff::new();
        ms(&mut b, 5);
        b.reset();
        assert_eq!(ms(&mut b, 2), [250, 500]);
    }

    #[test]
    fn never_overflows_or_exceeds_the_cap() {
        let mut b = Backoff {
            attempt: u32::MAX - 1,
        };
        for _ in 0..4 {
            assert_eq!(b.next_delay(), CAP);
        }
    }
}
