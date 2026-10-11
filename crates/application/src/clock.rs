//! Time source port, injected so use cases are deterministic under test.

use time::OffsetDateTime;

/// Source of the current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

/// Wall-clock UTC time, to the microsecond.
///
/// PostgreSQL keeps a microsecond, and some clocks give a nanosecond (Linux's does, macOS's does
/// not). A time the platform answers with when it creates something must be the time it answers with
/// when it is read back, so it is cut to what the store can keep before anything is done with it.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        truncate_to_micros(OffsetDateTime::now_utc())
    }
}

/// `at` without the nanoseconds below the microsecond.
#[must_use]
pub fn truncate_to_micros(at: OffsetDateTime) -> OffsetDateTime {
    let nanos = at.nanosecond();
    at.replace_nanosecond(nanos - nanos % 1_000).unwrap_or(at)
}
