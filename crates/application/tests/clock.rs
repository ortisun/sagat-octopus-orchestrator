//! The clock the platform stamps what it creates with (found by the first run of the suite on Linux).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::clock::{Clock, SystemClock, truncate_to_micros};
use time::OffsetDateTime;

#[test]
fn a_time_is_cut_to_the_microsecond_the_store_keeps() {
    let at = OffsetDateTime::from_unix_timestamp_nanos(1_760_145_612_298_403_015).expect("a time");
    assert_eq!(at.nanosecond(), 298_403_015);
    let cut = truncate_to_micros(at);
    assert_eq!(cut.nanosecond(), 298_403_000);
    assert_eq!(cut.unix_timestamp(), at.unix_timestamp());
    // What is already a whole number of microseconds is left as it is.
    assert_eq!(truncate_to_micros(cut), cut);
}

#[test]
fn the_system_clock_never_gives_a_nanosecond() {
    for _ in 0..1_000 {
        assert_eq!(SystemClock.now().nanosecond() % 1_000, 0);
    }
}
