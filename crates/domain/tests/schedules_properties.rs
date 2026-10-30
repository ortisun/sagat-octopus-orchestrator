//! Invariants of the schedule decision for any state, not only the examples of `schedules.rs`
//! (`T018-003`).
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_domain::schedules::{
    CatchupMode, Ceilings, Decision, Outcome, Overlap, Policies, Release, Standing, decide,
    jitter_offset_ms, plan_catch_up, release,
};
use time::OffsetDateTime;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
}

fn overlaps() -> impl Strategy<Value = Overlap> {
    prop_oneof![
        Just(Overlap::Skip),
        Just(Overlap::BufferOne),
        Just(Overlap::BufferAll),
        Just(Overlap::CancelOther),
        Just(Overlap::AllowAll),
    ]
}

fn standings() -> impl Strategy<Value = Standing> {
    (0..1200u32, 0..1200u32, proptest::option::of(0..1300u32)).prop_map(
        |(open_runs, buffered, remaining)| Standing {
            open_runs,
            buffered,
            remaining,
        },
    )
}

fn policies_with(overlap: Overlap, buffer_limit: u32) -> Policies {
    Policies::new(
        overlap,
        31_622_400_000,
        CatchupMode::One,
        false,
        buffer_limit,
        &Ceilings::default(),
    )
    .expect("policies")
}

proptest! {
    /// Only `ALLOW_ALL` starts a run next to an open one; every other policy waits for it.
    #[test]
    fn nothing_but_allow_all_starts_beside_an_open_run(
        overlap in overlaps(), standing in standings(), limit in 1..=1000u32,
    ) {
        let decision = decide(&policies_with(overlap, limit), &standing, &Ceilings::default());
        if decision == Decision::Start && standing.open_runs > 0 {
            prop_assert_eq!(overlap, Overlap::AllowAll);
        }
    }

    /// A new time never starts ahead of an action that is already waiting.
    #[test]
    fn a_new_time_never_jumps_the_queue(
        overlap in overlaps(), mut standing in standings(), limit in 1..=1000u32,
    ) {
        standing.buffered = standing.buffered.max(1);
        let decision = decide(&policies_with(overlap, limit), &standing, &Ceilings::default());
        if overlap != Overlap::AllowAll {
            prop_assert_ne!(decision, Decision::Start);
        }
    }

    /// Accepting a buffered action never takes the buffer over its limit (one for BUFFER_ONE and
    /// CANCEL_OTHER), and never over the namespace ceiling.
    #[test]
    fn the_buffer_never_overflows(
        overlap in overlaps(), standing in standings(), limit in 1..=1000u32, ceiling in 1..=1000u32,
    ) {
        let ceilings = Ceilings { buffer_limit: ceiling, ..Ceilings::default() };
        let policies = Policies::new(overlap, 31_622_400_000, CatchupMode::One, false, limit, &ceilings)
            .expect("policies");
        if let Decision::Buffer { .. } = decide(&policies, &standing, &ceilings) {
            let capacity = match overlap {
                Overlap::BufferOne | Overlap::CancelOther => 1,
                Overlap::BufferAll => limit.min(ceiling),
                Overlap::Skip | Overlap::AllowAll => 0,
            };
            prop_assert!(standing.buffered < capacity, "{standing:?} {overlap:?} capacity {capacity}");
        }
    }

    /// Buffered actions never outnumber the remaining ones: each waiting action holds one.
    #[test]
    fn a_waiting_action_always_has_a_remaining_action_to_run_on(
        overlap in overlaps(), standing in standings(), limit in 1..=1000u32,
    ) {
        let decision = decide(&policies_with(overlap, limit), &standing, &Ceilings::default());
        if let (Some(remaining), Decision::Buffer { .. } | Decision::Start) = (standing.remaining, decision) {
            prop_assert!(remaining > standing.buffered, "{standing:?} {decision:?}");
        }
    }

    /// Every decision that records something names a terminal-or-buffered outcome of the contract,
    /// and `Exhausted` is the only one that records nothing.
    #[test]
    fn exactly_exhausted_records_nothing(
        overlap in overlaps(), standing in standings(), limit in 1..=1000u32,
    ) {
        let decision = decide(&policies_with(overlap, limit), &standing, &Ceilings::default());
        prop_assert_eq!(decision.outcome().is_none(), decision == Decision::Exhausted);
        if let Some(outcome) = decision.outcome() {
            prop_assert_eq!(outcome.as_str().parse::<Outcome>(), Ok(outcome));
        }
    }

    /// A waiting action is released exactly when something waits and nothing is open.
    #[test]
    fn release_follows_what_waits_and_what_is_open(standing in standings()) {
        let expected = match (standing.buffered, standing.open_runs) {
            (0, _) => Release::Nothing,
            (_, 0) => Release::StartOldest,
            _ => Release::Wait,
        };
        prop_assert_eq!(release(&standing), expected);
    }

    /// A catch-up plan accounts for every due time exactly once, in order.
    #[test]
    fn a_catch_up_plan_conserves_the_due_times(
        mut seconds in proptest::collection::vec(0..2_000_000i64, 0..300),
        window in 10..=1_000_000i64,
        all in any::<bool>(),
        ceiling in 1..=1000u32,
    ) {
        seconds.sort_unstable();
        seconds.dedup();
        let due: Vec<_> = seconds.iter().map(|s| at(*s)).collect();
        let now = at(2_000_000);
        let mode = if all { CatchupMode::All } else { CatchupMode::One };
        let policies = Policies::new(
            Overlap::Skip,
            u64::try_from(window).expect("window") * 1000,
            mode,
            false,
            100,
            &Ceilings::default(),
        ).expect("policies");
        let ceilings = Ceilings { catchup_actions: ceiling, ..Ceilings::default() };
        let plan = plan_catch_up(&due, now, &policies, &ceilings);

        let missed = plan.missed.map_or(0, |s| s.count as usize);
        let coalesced = plan.coalesced.map_or(0, |s| s.count as usize);
        prop_assert_eq!(missed + plan.act.len() + coalesced, due.len());
        // the acted-on times are the earliest of those inside the window
        let inside: Vec<_> = due.iter().copied().filter(|t| (now - *t).whole_milliseconds() <= i128::from(window) * 1000).collect();
        prop_assert_eq!(&plan.act[..], &inside[..plan.act.len()]);
        prop_assert!(plan.act.windows(2).all(|w| w[0] < w[1]));
        match mode {
            CatchupMode::One => prop_assert!(plan.act.len() <= 1),
            CatchupMode::All => prop_assert!(plan.act.len() <= ceiling as usize),
        }
        // a summary covers an unbroken run of the due times and never an acted-on one
        if let Some(s) = plan.missed {
            prop_assert!(plan.act.iter().all(|t| *t > s.last));
        }
        if let Some(s) = plan.coalesced {
            prop_assert!(plan.act.iter().all(|t| *t < s.first));
        }
    }

    /// Jitter stays inside both the setting and the gap, whatever the digest.
    #[test]
    fn the_jitter_stays_inside_the_setting_and_the_gap(
        head in any::<u64>(), jitter in 0..=86_400_000u32, gap in proptest::option::of(0..=100_000_000u64),
    ) {
        let mut digest = [0u8; 32];
        digest[..8].copy_from_slice(&head.to_be_bytes());
        let offset = jitter_offset_ms(&digest, jitter, gap);
        let bound = gap.map_or(u64::from(jitter), |g| g.min(u64::from(jitter)));
        if bound == 0 {
            prop_assert_eq!(offset, 0);
        } else {
            prop_assert!(offset < bound);
        }
    }
}
