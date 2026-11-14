//! The expiry of a reset that waits for its source under crashes and races (`T018-057`, D2059 to D2068, AGENTS rule 9):
//! the expiry is one transaction (the fence, the lock of the entry, the look at the window and at the source, the entry
//! `FAILED` and its audit record), so a process killed in it leaves the entry pending and the step sent again ends it
//! once; two engines that meet at the entry end it once; an expiry and a completion that meet at the entry make one
//! outcome, and an ended source wins; an engine that lost its lease ends nothing; an entry that errors is left pending,
//! backed off from, and does not stop the others. The connection is killed inside PostgreSQL after each writing statement
//! in turn (`tripwire.rs`); the races are made with held row locks and the queue of the lock, never with sleeps. The
//! window is read on a clock the test sets. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use sagat_application::reset_completion::{Completed, ResetCompleter, SOURCE_DID_NOT_END};
use sagat_application::schedules::{SHARD_LEASE_LOST, ShardFence};
use sagat_domain::execution::{ResetId, RunId};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore};
use sagat_scheduler::{Backoff, Keeper, KeeperSettings, Placement, ResetPass, ResetSettings};
use sqlx::AssertSqlSafe;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::cancel_source::{
    Completions, Open, due, finish, made_from, open_pair, request, wait_for_backends_on_locks,
    worker,
};
use crate::support::bounded::within;
use crate::support::schedules::{Fixture, TestClock, fixture};
use crate::tripwire::{Tripwire, table_digests};

type Expiring = ResetCompleter<PgRunStore, Completions, PgIdentityStore, TestClock>;

fn completer_at(f: &Fixture, clock: &TestClock) -> Expiring {
    ResetCompleter::new(
        f.w.store.clone(),
        Completions(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        clock.clone(),
    )
}

fn window() -> Duration {
    Duration::seconds(
        i64::try_from(ResetSettings::default().expiry.as_secs()).expect("a day in seconds"),
    )
}

async fn requested_at(f: &Fixture, id: ResetId) -> OffsetDateTime {
    sqlx::query_scalar("SELECT created_at FROM run_resets WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the entry")
}

/// `(state, failure reason)` of an entry.
async fn state_of(f: &Fixture, id: ResetId) -> (String, Option<String>) {
    sqlx::query_as("SELECT state, failure_reason FROM run_resets WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the entry")
}

/// How many records say that the entry expired.
async fn expiries(f: &Fixture, id: ResetId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.reset' AND reason = $1",
    )
    .bind(format!("completes reset {id}: failed {SOURCE_DID_NOT_END}"))
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the audit trail")
}

/// A pending entry whose source is open, with the clock at the end of its window and the cutoff the pass computes.
struct Lapsed {
    /// The source, with the attempt of `b` that is still leased to the worker: `finish` ends the source.
    open: Open,
    source: RunId,
    id: ResetId,
    clock: TestClock,
    cutoff: OffsetDateTime,
}

async fn lapsed(f: &Fixture, session: &crate::support::engine::Session, key: &str) -> Lapsed {
    let open = open_pair(f, session).await;
    let made = request(f, open.run, open.after_a, key)
        .await
        .expect("the request");
    let asked = requested_at(f, made.reset.id).await;
    Lapsed {
        source: open.run,
        open,
        id: made.reset.id,
        clock: TestClock::at(asked + window()),
        cutoff: asked,
    }
}

/// A transaction that holds the row of the run the way the one that **ends** it does: an `UPDATE` that changes no key
/// column takes `FOR NO KEY UPDATE`, which `FOR SHARE` conflicts with and `FOR KEY SHARE` does not. Uncommitted: the caller
/// lets go with `rollback`.
async fn hold_as_the_end_of_a_run_does(
    f: &Fixture,
    run: RunId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = f.w.db.pool.begin().await.expect("hold the source");
    let updated = sqlx::query("UPDATE runs SET updated_at = updated_at WHERE id = $1")
        .bind(run.as_uuid())
        .execute(&mut *held)
        .await
        .expect("update the source")
        .rows_affected();
    assert_eq!(updated, 1, "the source is held by an uncommitted update");
    held
}

// ----------------------------------------------------------------------------------------------- two engines

#[tokio::test]
async fn two_engines_expiring_the_same_entry_record_one_expiry_and_the_second_is_refused() {
    let f = fixture().await;
    let session = worker(&f).await;
    let entry = lapsed(&f, &session, "expiry-two-engines-key").await;

    // both engines mean to expire it and meet at the row of the entry, which a third transaction holds
    let mut held = f.w.db.pool.begin().await.expect("hold the entry");
    sqlx::query("SELECT id FROM run_resets WHERE id = $1 FOR UPDATE")
        .bind(entry.id.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the entry");
    let (one, two) = (
        completer_at(&f, &entry.clock),
        completer_at(&f, &entry.clock),
    );
    let (a, b, ()) = tokio::join!(
        async { one.expire(&due(&f, entry.id), entry.cutoff, None).await },
        async { two.expire(&due(&f, entry.id), entry.cutoff, None).await },
        async {
            wait_for_backends_on_locks(&f, 2, "both engines wait for the entry").await;
            held.commit().await.expect("release the entry");
        }
    );
    let outcomes = [a.expect("one step"), b.expect("the other step")];
    let expired = outcomes
        .iter()
        .filter(|o| matches!(o, Completed::Expired))
        .count();
    let refused = outcomes
        .iter()
        .filter(|o| matches!(o, Completed::Refused(reason) if reason.contains("completed")))
        .count();
    assert_eq!((expired, refused), (1, 1), "{outcomes:?}");
    assert_eq!(
        state_of(&f, entry.id).await,
        ("FAILED".to_owned(), Some(SOURCE_DID_NOT_END.to_owned()))
    );
    assert_eq!(
        expiries(&f, entry.id).await,
        1,
        "the expiry is audited once"
    );
    assert!(made_from(&f, entry.source).await.is_empty());
}

// --------------------------------------------------------------- an expiry and a completion meet at the entry

/// The race test: an expiry and a completion that meet at the row of one entry make **one** outcome, whichever takes the
/// lock first. It is not the test of the guard "an ended source wins": `join!` polls the completion first, so the expiry
/// nearly always gets the lock second and is refused because the entry is done, and the look at the source is not what
/// keeps it green. That guard is covered deterministically by `the_store_ends_an_entry_only_when_it_is_past_the_cutoff_and_its_source_is_open`
/// (the store answers `SourceEnded`), `a_source_that_ends_after_the_entry_has_lapsed_but_before_the_pass_looks_is_completed_and_not_expired`
/// (the completer leaves the entry to the completion) and part (2) of `an_entry_is_never_both_expired_and_completed_whichever_comes_first`.
#[tokio::test]
async fn an_expiry_and_a_completion_that_meet_at_the_entry_make_one_outcome() {
    let f = fixture().await;
    let session = worker(&f).await;
    let entry = lapsed(&f, &session, "expiry-meets-completion-key").await;
    // the source has ended: the entry is the completion's, however late
    finish(&f, &session, &entry.open).await;

    let mut held = f.w.db.pool.begin().await.expect("hold the entry");
    sqlx::query("SELECT id FROM run_resets WHERE id = $1 FOR UPDATE")
        .bind(entry.id.as_uuid())
        .fetch_one(&mut *held)
        .await
        .expect("lock the entry");
    let (completing, expiring) = (
        completer_at(&f, &entry.clock),
        completer_at(&f, &entry.clock),
    );
    let (created, expired, ()) = tokio::join!(
        async { completing.step(&due(&f, entry.id), None).await },
        async {
            expiring
                .expire(&due(&f, entry.id), entry.cutoff + window(), None)
                .await
        },
        async {
            wait_for_backends_on_locks(&f, 2, "both wait for the entry").await;
            held.commit().await.expect("release the entry");
        }
    );
    assert_eq!(created.expect("the completion"), Completed::Created);
    // whichever of them took the lock first, the expiry did not end the entry: it was refused (the run was made) or it
    // found the source ended and left the entry to the completion
    let expired = expired.expect("the expiry");
    assert!(
        matches!(expired, Completed::Refused(_) | Completed::Idle),
        "{expired:?}"
    );
    assert_eq!(state_of(&f, entry.id).await.0, "CREATED");
    assert_eq!(made_from(&f, entry.source).await.len(), 1, "one run");
    assert_eq!(expiries(&f, entry.id).await, 0, "never also expired");
}

#[tokio::test]
async fn an_entry_is_never_both_expired_and_completed_whichever_comes_first() {
    let f = fixture().await;
    let session = worker(&f).await;

    // (1) the expiry commits first: the source ends later and the entry stays as it ended
    let first = lapsed(&f, &session, "expiry-order-one-key").await;
    let at = completer_at(&f, &first.clock);
    assert_eq!(
        at.expire(&due(&f, first.id), first.cutoff, None)
            .await
            .expect("the expiry"),
        Completed::Expired
    );
    finish(&f, &session, &first.open).await;
    assert_eq!(
        at.step(&due(&f, first.id), None)
            .await
            .expect("the completion"),
        Completed::Idle,
        "an entry that ended is not completed"
    );
    assert!(made_from(&f, first.source).await.is_empty());
    assert_eq!(
        state_of(&f, first.id).await,
        ("FAILED".to_owned(), Some(SOURCE_DID_NOT_END.to_owned()))
    );

    // (2) the source is in the hands of the transaction that ends it when the expiry looks: nothing is ended, and when
    // the source has ended the completion makes the run. The transaction holds the row the way an `UPDATE` that ends a run
    // does (`FOR NO KEY UPDATE`: a `FOR UPDATE` conflicts with every lock and would let `FOR KEY SHARE` pass for the guard),
    // and the call is bounded: with the look at the source taken without `SKIP LOCKED` the expiry would wait for this very
    // task to let go, and the test would hang instead of failing.
    let second = lapsed(&f, &session, "expiry-order-two-key").await;
    let at = completer_at(&f, &second.clock);
    let ending = hold_as_the_end_of_a_run_does(&f, second.source).await;
    assert_eq!(
        within(
            "the expiry does not wait for the row of the source",
            StdDuration::from_secs(60),
            at.expire(&due(&f, second.id), second.cutoff, None)
        )
        .await
        .expect("the expiry"),
        Completed::Busy,
        "the source is being changed: the entry is left for the next round"
    );
    assert_eq!(state_of(&f, second.id).await.0, "PENDING_SOURCE");
    ending.rollback().await.expect("let go");
    finish(&f, &session, &second.open).await;
    assert_eq!(
        at.expire(&due(&f, second.id), second.cutoff, None)
            .await
            .expect("the expiry again"),
        Completed::Idle,
        "the source has ended: an ended source wins"
    );
    assert_eq!(
        at.step(&due(&f, second.id), None)
            .await
            .expect("the completion"),
        Completed::Created
    );
    assert_eq!(made_from(&f, second.source).await.len(), 1);
    assert_eq!(expiries(&f, second.id).await, 0);
}

// ---------------------------------------------------------------------------------------------------- the fence

async fn holding_every_shard(f: &Fixture, name: &str) -> (Keeper<PgRunStore>, Placement) {
    let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("keeper");
    assert_eq!(placement.owned().iter().count(), Shard::all().count());
    (keeper, placement)
}

#[tokio::test]
async fn a_replica_that_lost_its_lease_expires_nothing_and_the_one_that_holds_it_does() {
    let f = fixture().await;
    let session = worker(&f).await;
    let entry = lapsed(&f, &session, "expiry-fence-key").await;
    let (_keeper, slow) = holding_every_shard(&f, "slow-engine").await;
    let shard = Shard::of(entry.id.as_uuid());
    let lease = slow
        .snapshot()
        .lease_of(shard, Instant::now())
        .expect("it holds the shard");
    let fence = ShardFence {
        owner: slow.owner().to_owned(),
        lease,
    };
    // another replica took the shard over while this one was working
    sqlx::query("UPDATE scheduler_shards SET owner = 'the-thief', generation = generation + 1 WHERE shard = $1")
        .bind(i16::from(shard.index()))
        .execute(&f.w.db.pool)
        .await
        .expect("take the shard");
    let digests = table_digests(&f.w.db.pool).await;

    let refused = completer_at(&f, &entry.clock)
        .expire(&due(&f, entry.id), entry.cutoff, Some(fence))
        .await
        .expect("a step");
    assert!(
        matches!(refused, Completed::Refused(reason) if reason == SHARD_LEASE_LOST),
        "{refused:?}"
    );
    assert_eq!(state_of(&f, entry.id).await.0, "PENDING_SOURCE");
    assert_eq!(expiries(&f, entry.id).await, 0);
    assert_eq!(
        table_digests(&f.w.db.pool).await,
        digests,
        "nothing changed"
    );

    // the pass of the replica that does not know yet (its snapshot still says it holds the shard) is refused the same
    // way, and the round says so
    let pass = ResetPass::new(
        completer_at(&f, &entry.clock),
        slow.clone(),
        ResetSettings::default(),
    );
    let round = pass.round().await;
    assert_eq!(
        (round.lapsed, round.expired, round.refused),
        (1, 0, 1),
        "{round:?}"
    );
    assert_eq!(state_of(&f, entry.id).await.0, "PENDING_SOURCE");
    assert_eq!(
        table_digests(&f.w.db.pool).await,
        digests,
        "nothing changed"
    );
}

// ----------------------------------------------------------------------------------------------------- crashes

#[tokio::test]
async fn an_expiry_that_dies_at_any_write_leaves_the_entry_pending_and_sent_again_ends_it_once() {
    let f = fixture().await;
    let session = worker(&f).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    let warm = lapsed(&f, &session, "expiry-warm-up-key").await;
    assert_eq!(
        completer_at(&f, &warm.clock)
            .expire(&due(&f, warm.id), warm.cutoff, None)
            .await
            .expect("warm-up"),
        Completed::Expired
    );

    let probe = lapsed(&f, &session, "expiry-probe-key").await;
    wire.watch().await;
    assert_eq!(
        completer_at(&f, &probe.clock)
            .expire(&due(&f, probe.id), probe.cutoff, None)
            .await
            .expect("the probe"),
        Completed::Expired
    );
    wire.disarm().await;
    let writes = wire.writes().await;
    assert_eq!(
        writes
            .iter()
            .map(|(xid, _, _)| *xid)
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "an expiry is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in ["run_resets", "audit_records"] {
        assert!(
            tables.contains(table),
            "an expiry is expected to write {table}: {writes:?}"
        );
    }
    println!(
        "an expiry makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let entry = lapsed(&f, &session, &format!("expiry-write-{position}-key")).await;
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = completer_at(&f, &entry.clock)
            .expire(&due(&f, entry.id), entry.cutoff, None)
            .await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the step answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        assert_eq!(
            state_of(&f, entry.id).await,
            ("PENDING_SOURCE".to_owned(), None),
            "write {position}"
        );
        assert_eq!(expiries(&f, entry.id).await, 0, "write {position}");

        // sent again: ended, once, with one record
        assert_eq!(
            completer_at(&f, &entry.clock)
                .expire(&due(&f, entry.id), entry.cutoff, None)
                .await
                .unwrap_or_else(|error| panic!(
                    "write {position} ({table}), sent again: {error:?}"
                )),
            Completed::Expired,
            "write {position}"
        );
        assert_eq!(
            state_of(&f, entry.id).await,
            ("FAILED".to_owned(), Some(SOURCE_DID_NOT_END.to_owned())),
            "write {position}"
        );
        assert_eq!(expiries(&f, entry.id).await, 1, "write {position}");
        assert!(
            made_from(&f, entry.source).await.is_empty(),
            "write {position}"
        );

        // and a third time finds nothing to do
        assert_eq!(
            completer_at(&f, &entry.clock)
                .expire(&due(&f, entry.id), entry.cutoff, None)
                .await
                .expect("a third step"),
            Completed::Idle,
            "write {position}"
        );
        assert_eq!(expiries(&f, entry.id).await, 1, "write {position}");
    }
}

// ------------------------------------------------------------------- an entry that errors is not an outcome

/// A trigger that refuses the audit record of the expiry of the entry whose source is `source`: an entry that errors
/// whenever the pass tries to end it (a defect, a constraint, an outage that touches one row).
async fn poison(f: &Fixture, source: RunId) {
    for statement in [
        "CREATE FUNCTION fault_poison() RETURNS trigger LANGUAGE plpgsql AS $f$ \
         BEGIN RAISE EXCEPTION 'poisoned' USING ERRCODE = 'XX000'; END $f$"
            .to_owned(),
        format!(
            "CREATE TRIGGER fault_poison BEFORE INSERT ON audit_records FOR EACH ROW \
             WHEN (NEW.resource_id = '{source}' AND NEW.reason LIKE '%failed {SOURCE_DID_NOT_END}') \
             EXECUTE FUNCTION fault_poison()"
        ),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&f.w.db.pool)
            .await
            .expect("poison the entry");
    }
}

async fn heal(f: &Fixture) {
    sqlx::query("DROP TRIGGER fault_poison ON audit_records")
        .execute(&f.w.db.pool)
        .await
        .expect("heal");
}

#[tokio::test]
async fn an_entry_that_errors_stays_pending_is_backed_off_from_and_does_not_stop_the_others() {
    let f = fixture().await;
    let session = worker(&f).await;
    let stuck = lapsed(&f, &session, "expiry-stuck-key").await;
    let others = [
        lapsed(&f, &session, "expiry-other-one-key").await,
        lapsed(&f, &session, "expiry-other-two-key").await,
    ];
    let (_keeper, placement) = holding_every_shard(&f, "engine-one").await;
    let latest = others
        .iter()
        .map(|entry| entry.cutoff)
        .chain([stuck.cutoff])
        .max()
        .expect("entries");
    let clock = TestClock::at(latest + window());
    let pass = ResetPass::new(
        completer_at(&f, &clock),
        placement,
        ResetSettings::default(),
    );
    poison(&f, stuck.source).await;

    let t0 = Instant::now();
    let first = pass.round_at(t0).await;
    assert_eq!(
        (first.lapsed, first.expired, first.errors),
        (3, 2, 1),
        "{first:?}"
    );
    // what the platform could not do is not an outcome: the entry is still pending, with no record of an end
    assert_eq!(
        state_of(&f, stuck.id).await,
        ("PENDING_SOURCE".to_owned(), None)
    );
    assert_eq!(expiries(&f, stuck.id).await, 0);
    for entry in &others {
        assert_eq!(state_of(&f, entry.id).await.0, "FAILED");
    }

    // the next round leaves it out until its time, and does not look at the others again
    let second = pass.round_at(t0).await;
    assert_eq!(
        (
            second.lapsed,
            second.expired,
            second.errors,
            second.backing_off
        ),
        (0, 0, 0, 1),
        "{second:?}"
    );
    // when its time has come it is tried again and, the cause being gone, ended once
    heal(&f).await;
    let third = pass.round_at(t0 + Backoff::BASE).await;
    assert_eq!(
        (third.lapsed, third.expired, third.errors),
        (1, 1, 0),
        "{third:?}"
    );
    assert_eq!(
        state_of(&f, stuck.id).await,
        ("FAILED".to_owned(), Some(SOURCE_DID_NOT_END.to_owned()))
    );
    assert_eq!(expiries(&f, stuck.id).await, 1);
}
