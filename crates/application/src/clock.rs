//! Time source port, injected so use cases are deterministic under test.

use time::OffsetDateTime;

/// Source of the current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

/// Wall-clock UTC time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}
