//! The clock of the window of the expiry, and a source another transaction holds (`T018-057`, D2079, D2081; the review of
//! 2026-11-09). The window runs on the database's time from the request to the pass: the register keeps `created_at` from
//! the transaction of the source (`clock_timestamp()` after its row lock is held), the control plane's clock only stamps the
//! audit records of the request, and the engine's pass reads a `DatabaseClock` that `loops::reset_round` moves to the
//! database's time before it looks. Real PostgreSQL; nothing sleeps (the time of a test is a clock it sets, and an entry is
//! made older with one statement).
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::execution::Engine;
use sagat_application::reset_completion::{ResetCompleter, SOURCE_DID_NOT_END};
use sagat_application::runs::RunService;
use sagat_domain::execution::ResetId;
use sagat_infra_postgres::{DatabaseClock, PgIdentityStore, PgRunStore};
use sagat_scheduler::{Backoff, Keeper, KeeperSettings, Placement, ResetPass, ResetSettings};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::business_ids::key;
use crate::cancel_source::{
    Completions, asking, made_from, open_chain, open_chain_tagged, outcome,
};
use crate::cancel_source_expiry::{
    asked, counts, engine_with, hold_as_the_end_of_a_run_does, latest_request, requested_at, window,
};
use crate::firing::{Fixture, TestClock, fixture};

/// The database's own time, now.
async fn database_now(f: &Fixture) -> OffsetDateTime {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the database clock")
}

/// Makes an entry older by `by`, as if its request had been made that much earlier. The register forbids changing what a
/// reset is (`created_at` among it: the guard trigger is `ENABLE ALWAYS`), so this one statement is made with the guard
/// switched off by the owner of the table and switched on again at once. Three statements, each committed: the table cannot
/// be altered in a transaction that has deferred trigger events pending (the accounting of the register), and every test has
/// a database of its own, so nothing else is writing to it between them.
async fn age(f: &Fixture, id: ResetId, by: Duration) {
    sqlx::query("ALTER TABLE run_resets DISABLE TRIGGER run_resets_guard")
        .execute(&f.w.db.pool)
        .await
        .expect("switch the guard off to age one entry");
    let aged = sqlx::query(
        "UPDATE run_resets SET created_at = created_at - make_interval(secs => $2) WHERE id = $1",
    )
    .bind(id.as_uuid())
    .bind(by.as_seconds_f64())
    .execute(&f.w.db.pool)
    .await
    .expect("age the entry")
    .rows_affected();
    sqlx::query("ALTER TABLE run_resets ENABLE ALWAYS TRIGGER run_resets_guard")
        .execute(&f.w.db.pool)
        .await
        .expect("switch the guard on again");
    assert_eq!(aged, 1);
}

type OnDatabaseClock =
    ResetPass<PgRunStore, Completions, PgIdentityStore, DatabaseClock<TestClock>>;

/// An engine as `apps/execution-engine` builds the reset pass: the pass reads a [`DatabaseClock`] over a process clock that
/// the test sets `skew` away from the database's, and a round goes through `loops::reset_round`, which moves the clock to
/// the database's time before the pass looks. It holds every shard.
async fn engine_on_the_database_clock(
    f: &Fixture,
    skew: Duration,
) -> (DatabaseClock<TestClock>, OnDatabaseClock) {
    let placement = Placement::new(format!("engine-skewed-{}", Uuid::now_v7()));
    Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("keeper");
    let clock = DatabaseClock::new(
        f.w.db.pool.clone(),
        TestClock::at(database_now(f).await + skew),
    );
    let pass = ResetPass::new(
        ResetCompleter::new(
            f.w.store.clone(),
            Completions(Arc::clone(&f.w)),
            PgIdentityStore::new(f.w.db.pool.clone()),
            clock.clone(),
        ),
        placement,
        ResetSettings::default(),
    );
    (clock, pass)
}

/// A run service as a control plane builds it, on the process clock given.
fn service_on(
    f: &Fixture,
    clock: TestClock,
) -> RunService<PgRunStore, TestClock, sagat_infra_ir::JsonEvaluator> {
    RunService::new(
        f.w.store.clone(),
        clock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_reset_reapply(true)
    .with_reset_cancel_source(true)
}

/// The window starts at the database's time of the request and not at the clock of the process that took it: the register
/// keeps `created_at` from the transaction of the source, and the control plane's clock only stamps the audit records of the
/// request. A control plane whose clock is three hours off, either way, neither cuts the window short nor stretches it.
/// (Review of 2026-11-09, A2: G2014 is closed by this test.)
#[tokio::test]
async fn a_control_plane_whose_clock_is_three_hours_off_neither_cuts_the_window_short_nor_stretches_it()
 {
    for skew in [Duration::hours(3), -Duration::hours(3)] {
        let f = fixture().await;
        let before = database_now(&f).await;
        let service = service_on(&f, TestClock::at(before + skew));
        let open = open_chain(&f).await;
        let made = service
            .reset(
                &f.w.admin,
                open.run,
                asking(open.after_a, true),
                &key(),
                &crate::support::engine::ctx(),
            )
            .await
            .expect("the request");
        let after = database_now(&f).await;
        let id = made.reset.id;

        // what the register keeps is the database's time of the request, between two readings of the database's clock; the
        // clock of the process only stamped the record of the audit trail
        let asked_at = requested_at(&f, id).await;
        assert!(
            before <= asked_at && asked_at <= after,
            "skew {skew}: asked at {asked_at}, the database read {before} and {after}"
        );
        let stamped: OffsetDateTime = sqlx::query_scalar(
            "SELECT created_at FROM audit_records WHERE action = 'run.reset' AND resource_id = $1 \
             ORDER BY created_at LIMIT 1",
        )
        .bind(open.run.to_string())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the record of the request");
        assert!(
            (stamped - before - skew).abs() < Duration::minutes(1),
            "the audit record carries the clock of the process: {stamped} for {before} and a skew of {skew}"
        );

        // and the pass, on the database's clock, ends it when a window of the database's time has passed
        let (clock, pass) = engine_on_the_database_clock(&f, Duration::ZERO).await;
        age(&f, id, window() - Duration::seconds(60)).await;
        let inside = sagat_execution_engine::loops::reset_round(&clock, &pass).await;
        assert_eq!(
            counts(&inside),
            (0, 0, 0, 0, 0, 0),
            "skew {skew}: {inside:?}"
        );
        assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE", "skew {skew}");
        age(&f, id, Duration::seconds(120)).await;
        let over = sagat_execution_engine::loops::reset_round(&clock, &pass).await;
        assert_eq!(counts(&over), (0, 1, 0, 1, 0, 0), "skew {skew}: {over:?}");
        assert_eq!(
            outcome(&f, id).await.1.as_deref(),
            Some(SOURCE_DID_NOT_END),
            "skew {skew}"
        );
    }
}

/// The reset pass of an engine reads the window on the database's clock, whatever its own is: a round goes through
/// `loops::reset_round`, which moves a [`DatabaseClock`] to the database's time before the pass looks. With a process clock
/// three hours ahead an entry 22 hours old must not expire under a window of 24 (it would look 25 hours old), and with one
/// three hours behind an entry 24 hours and a minute old must (it would look 21 hours old). (Review of 2026-11-09, A3.)
#[tokio::test]
async fn an_engine_whose_clock_is_three_hours_off_reads_the_window_on_the_database_clock() {
    for skew in [Duration::hours(3), -Duration::hours(3)] {
        let f = fixture().await;
        let open = open_chain(&f).await;
        let id = asked(&f, &open).await;
        let (clock, pass) = engine_on_the_database_clock(&f, skew).await;

        age(&f, id, Duration::hours(22)).await;
        let inside = sagat_execution_engine::loops::reset_round(&clock, &pass).await;
        assert_eq!(
            counts(&inside),
            (0, 0, 0, 0, 0, 0),
            "skew {skew}: {inside:?}"
        );
        assert_eq!(outcome(&f, id).await.0, "PENDING_SOURCE", "skew {skew}");

        age(&f, id, Duration::hours(2) + Duration::minutes(1)).await;
        let over = sagat_execution_engine::loops::reset_round(&clock, &pass).await;
        assert_eq!(counts(&over), (0, 1, 0, 1, 0, 0), "skew {skew}: {over:?}");
        assert_eq!(
            outcome(&f, id).await.1.as_deref(),
            Some(SOURCE_DID_NOT_END),
            "skew {skew}"
        );
    }
}

// ----------------------------------------------------------------------- a source another transaction holds (D2081)

/// An entry past its window whose source another transaction holds is left out for a short constant time, with a line that
/// names it, and does not keep its slot in the share of its tenant: with a share of one, the older entry over a source that
/// stays locked would otherwise be the one the scan takes every round and the newer entries of the tenant would never be
/// looked at (G2017). Held with an uncommitted `UPDATE`, as the transaction that ends a run holds it.
#[tokio::test]
async fn an_entry_whose_source_another_transaction_holds_is_left_out_for_a_second_and_does_not_hide_the_newer_ones()
 {
    let f = fixture().await;
    let stuck = open_chain_tagged(&f, "1").await;
    let free = open_chain_tagged(&f, "2").await;
    let stuck_id = asked(&f, &stuck).await;
    let free_id = asked(&f, &free).await;
    let engine = engine_with(
        &f,
        "engine-one",
        ResetSettings {
            per_tenant: 1,
            ..ResetSettings::default()
        },
    )
    .await;
    engine
        .clock
        .set(latest_request(&f, &[stuck_id, free_id]).await + window());
    let held = hold_as_the_end_of_a_run_does(&f, stuck.run).await;

    let (_guard, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);
    let t0 = std::time::Instant::now();
    let first = engine.pass.round_at(t0).await;
    assert_eq!(
        (first.lapsed, first.busy, first.expired, first.errors),
        (1, 1, 0, 0),
        "the older entry takes the share of the tenant and its source is held: {first:?}"
    );
    assert!(
        !first.did_work(),
        "a busy source is not work: the loop rests"
    );
    // the line names the tenant and the entry
    let tenant = f.w.admin.tenant_id().to_string();
    let text = logs.text();
    let line = text
        .lines()
        .find(|line| line.contains(&stuck_id.to_string()))
        .unwrap_or_else(|| panic!("no line names the entry whose source is held\n{text}"));
    assert!(
        line.contains(&tenant) && line.contains("tenant_id") && line.contains("reset_id"),
        "{line}"
    );

    // the next round does not take the same entry again: it is left out, and the newer one is looked at
    let second = engine.pass.round_at(t0).await;
    assert_eq!(second.backing_off, 1, "{second:?}");
    assert_eq!(
        (second.lapsed, second.busy, second.expired),
        (1, 0, 1),
        "{second:?}"
    );
    assert_eq!(outcome(&f, free_id).await.0, "FAILED");
    assert_eq!(outcome(&f, stuck_id).await.0, "PENDING_SOURCE");

    // for a second only: then it is tried again, and found busy again while the source is held
    let third = engine
        .pass
        .round_at(t0 + Backoff::BUSY - StdDuration::from_millis(1))
        .await;
    assert_eq!((third.lapsed, third.backing_off), (0, 1), "{third:?}");
    let fourth = engine.pass.round_at(t0 + Backoff::BUSY).await;
    assert_eq!((fourth.lapsed, fourth.busy), (1, 1), "{fourth:?}");

    // let go of it: after the pause the entry is ended, and a busy source never counted as a failure (the delay is a second,
    // not the doubled one of an error)
    held.rollback().await.expect("let go of the source");
    let fifth = engine.pass.round_at(t0 + Backoff::BUSY * 2).await;
    assert_eq!(
        (fifth.lapsed, fifth.expired, fifth.errors),
        (1, 1, 0),
        "{fifth:?}"
    );
    assert_eq!(
        outcome(&f, stuck_id).await.1.as_deref(),
        Some(SOURCE_DID_NOT_END)
    );
    assert!(made_from(&f, stuck.run).await.is_empty());
}
