//! The cursor of the listing of a schedule's decisions: what it hands out comes back, and nothing
//! else is taken for one.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::schedules::ActionCursor;
use sagat_domain::schedules::Origin;
use time::OffsetDateTime;

fn at(micros: i128) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(micros * 1_000).expect("an instant")
}

#[test]
fn a_cursor_comes_back_as_it_was_handed_out() {
    for (decided, origin, nominal) in [
        (1_793_791_012_982_123, Origin::Spec, 1_793_790_000_000_000),
        (1_793_791_012_982_123, Origin::Manual, 1_793_791_012_982_122),
        (0, Origin::Spec, 0),
        (-86_400_000_000, Origin::Manual, -1),
    ] {
        let cursor = ActionCursor {
            decided_at: at(decided),
            origin,
            nominal_at: at(nominal),
        };
        assert_eq!(ActionCursor::decode(&cursor.encode()), Some(cursor));
    }
}

#[test]
fn a_string_that_was_not_handed_out_is_not_a_cursor() {
    for text in [
        "",
        "nonsense",
        "1",
        "1.S",
        "1.S.2.3",
        "a.S.2",
        "1.S.b",
        "1.X.2",
        "1.s.2",
        " 1.S.2",
        "1.S.2 ",
        "1..2",
        // too far from now to be an instant
        "999999999999999999999999999999.S.2",
        "1.S.99999999999999999999999999999999999999",
    ] {
        assert_eq!(ActionCursor::decode(text), None, "{text:?}");
    }
}
