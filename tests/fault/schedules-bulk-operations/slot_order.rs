//! The slot order of the admission paths that exist (`T018-012`, `NFR-018-001`, ADR-0050 decision 7): each path takes
//! the locks of its reference in the order of the table of `plan.md`, so none of them can be one half of a lock cycle
//! with another path that does.
//!
//! A real path of the platform runs against a reference transaction (`support/lock_order.rs`, [`against_at`]): the
//! reference holds one lock of the case's list, waits until the path is blocked by it (`pg_blocking_pids`, not a time),
//! takes the locks after it in the order of the slots, and commits. A path that held, while it waited, a lock the
//! reference takes after the one it holds would make the reference ask for something the path holds, and PostgreSQL
//! would break the cycle with `40P01` for one of the two. Every case runs the path once **for every start** (the
//! reference holds the lock `k` for `k` from the first to the last but one), on state of its own, so every pair of locks
//! of the list is ordered, not only the first against the rest. Every run asserts that the reference committed, that
//! the path did what it was asked, and that the path was seen waiting for the lock `k` (otherwise it never met the
//! reference and the run fails by name).
//!
//! What it proves: for the paths of its cases (the schedule pass, trigger-now, a backfill step, the release of a buffered
//! time, the payment of a `CANCEL_OTHER` debt, the release of a delayed start, the claim of a business identifier, the
//! request and the completion of a pending reset), the locks the references name are taken in the order they are listed
//! in. The authority, in the references of the paths that look at it again under the lock (the schedule paths and the completion of
//! a reset), is the memberships of the tenant locked `FOR UPDATE`, which conflicts with the `FOR SHARE` of that check (slot 4,
//! `FR-018-015`): a path that checks it after its quota locks, or before its aggregate, is red; a person's start is judged by the
//! role of the caller and has no such lock. What it does **not** prove: the order of a lock the references do not name, any path it
//! does not run (feature 019, not approved; 022, whose slot does not exist; the batch item, reset `REQUIRE_TERMINAL`,
//! with-start, update-with-start: `T018-012` stays open for them, `G2029`), or an interleaving of more than two
//! transactions. The lease row of a shard is in no reference: it is a leaf lock outside the slots (ADR-0050 decision 7,
//! `D2083`), kept so by `scripts/check-architecture.py`.
//!
//! It is a release gate: a pull request runs `fault_schedules_bulk_operations` only when a file of it changes.
//! Real PostgreSQL, the real run service, scheduler and passes.
#![allow(clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::pin::Pin;

use sagat_application::identity::RequestContext;
use sagat_application::runs::{BusinessStart, StartOptions, StartRun};
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{BackfillRequest, DueBackfill, ScheduleRecord, ShardFence};
use sagat_application::shards::Lease;
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::quota::QuotaResource;
use sagat_domain::sharding::Shard;
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::cancel_other::{cancel_other, waiting_behind_an_open_run};
use crate::cancel_source::{asking, completer, due, finish, open_pair, room_for_pending, worker};
use crate::delay::due_run;
use crate::run_driven_locks::shard_owner;
use crate::support::engine::ctx;
use crate::support::lock_order::{Lock, Outcome, Slot, against, against_at, deadlocks, run_plan};
use crate::support::schedules::{Fixture, fixture, hourly, t0};

// --- the harness proves itself against real locks ----------------------------------------------------

async fn scratch(f: &Fixture) {
    for statement in [
        "CREATE TABLE slot_order_rows (id bigint PRIMARY KEY)",
        "INSERT INTO slot_order_rows VALUES (1), (2), (3)",
    ] {
        sqlx::query(statement)
            .execute(&f.w.db.pool)
            .await
            .expect("scratch table");
    }
}

#[tokio::test]
async fn a_path_in_the_order_of_the_reference_completes_and_one_against_it_is_a_deadlock() {
    let f = fixture().await;
    scratch(&f).await;
    let low = Lock::row("row 1", "slot_order_rows", 1);
    let high = Lock::row("row 2", "slot_order_rows", 2);
    let reference = vec![
        (Slot::Aggregate, low.clone()),
        (Slot::ExistingRuns, high.clone()),
    ];

    // in the order: the path waits for the first lock, the reference takes the second and commits, the path follows
    let ordered = against(
        &f.w.db.pool,
        &reference,
        run_plan(&f.w.db.pool, &[low.clone(), high.clone()]),
    )
    .await;
    assert_eq!(ordered.reference, Outcome::Committed, "{ordered:?}");
    assert_eq!(ordered.path, Outcome::Committed, "{ordered:?}");

    // against it: the path holds the second while it waits for the first, and the reference asks for the second
    let wrong = against(
        &f.w.db.pool,
        &reference,
        run_plan(&f.w.db.pool, &[high, low]),
    )
    .await;
    assert_eq!(
        deadlocks(&[wrong.reference.clone(), wrong.path.clone()]),
        1,
        "exactly one of the two is the victim: {wrong:?}"
    );
    assert!(
        wrong.reference == Outcome::Committed || wrong.path == Outcome::Committed,
        "and the other one goes on: {wrong:?}"
    );
}

/// A path that takes its first lock first and then the third before the second: the start 0 does not see it (the path holds
/// nothing while it waits for the first lock), the start 1 does (it holds the third while it waits for the second).
#[tokio::test]
async fn a_start_at_every_lock_finds_the_pair_the_first_start_cannot_see() {
    let f = fixture().await;
    scratch(&f).await;
    let one = Lock::row("row 1", "slot_order_rows", 1);
    let two = Lock::row("row 2", "slot_order_rows", 2);
    let three = Lock::row("row 3", "slot_order_rows", 3);
    let reference = vec![
        (Slot::Aggregate, one.clone()),
        (Slot::Authority, two.clone()),
        (Slot::NewRuns, three.clone()),
    ];
    let wrong = [one, three, two];

    let first = against_at(&f.w.db.pool, &reference, 0, run_plan(&f.w.db.pool, &wrong)).await;
    assert_eq!(first.reference, Outcome::Committed, "{first:?}");
    assert_eq!(first.path, Outcome::Committed, "{first:?}");

    let second = against_at(&f.w.db.pool, &reference, 1, run_plan(&f.w.db.pool, &wrong)).await;
    assert_eq!(
        deadlocks(&[second.reference.clone(), second.path.clone()]),
        1,
        "the third lock was taken before the second: {second:?}"
    );
}

#[tokio::test]
#[should_panic(expected = "ended without waiting")]
async fn a_path_that_never_meets_the_reference_fails_by_name_and_does_not_pass() {
    let f = fixture().await;
    scratch(&f).await;
    let reference = vec![
        (Slot::Aggregate, Lock::row("row 1", "slot_order_rows", 1)),
        (Slot::ExistingRuns, Lock::row("row 2", "slot_order_rows", 2)),
    ];
    // a path that takes neither
    against(&f.w.db.pool, &reference, async {}).await;
}

// --- the locks of the references ------------------------------------------------------------------------

fn tenant(f: &Fixture) -> Uuid {
    f.w.admin.tenant_id().as_uuid()
}

fn advisory(name: &'static str, resource: QuotaResource, scope: String) -> Lock {
    Lock::statement(
        name,
        format!("SELECT pg_advisory_xact_lock(hashtextextended('quota:{resource}:{scope}', 0))"),
    )
}

/// The usage of the tenant, always the last lock (slot 8b).
fn usage(f: &Fixture) -> (Slot, Lock) {
    (
        Slot::Usage,
        Lock::statement(
            "the retained usage of the tenant",
            format!(
                "SELECT 1 FROM tenant_retained_usage WHERE tenant_id = '{}' FOR UPDATE",
                tenant(f)
            ),
        ),
    )
}

/// What every admission of a run ends in: the authority it is admitted under (slot 4), the quotas of the active runs of
/// the namespace and of the tenant (slot 6), the usage (slot 8b).
fn admission(f: &Fixture) -> Vec<(Slot, Lock)> {
    let tenant = tenant(f);
    let namespace = f.w.namespace.id().as_uuid();
    vec![
        (
            Slot::Authority,
            Lock::statement(
                "the memberships of the tenant",
                format!("SELECT 1 FROM memberships WHERE tenant_id = '{tenant}' FOR UPDATE"),
            ),
        ),
        (
            Slot::NewRuns,
            advisory(
                "the active runs of the namespace",
                QuotaResource::ActiveRuns,
                format!("{tenant}:{namespace}"),
            ),
        ),
        (
            Slot::NewRuns,
            advisory(
                "the active runs of the tenant",
                QuotaResource::TenantActiveRuns,
                tenant.to_string(),
            ),
        ),
        usage(f),
    ]
}

/// A fence that the database accepts for the shard of `owner_of`: the lease row is given to a replica of this test. The
/// lease row is no lock of any reference (it is a leaf lock): the path takes it `FOR SHARE` and nothing waits for that.
async fn fence(f: &Fixture, owner_of: Uuid) -> ShardFence {
    let shard = Shard::of(owner_of);
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'slot-order-engine', generation = 3, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("the shard has an owner");
    ShardFence {
        owner: "slot-order-engine".to_owned(),
        lease: Lease {
            shard,
            generation: 3,
        },
    }
}

fn schedule_row(f: &Fixture, schedule: &ScheduleRecord) -> (Slot, Lock) {
    (
        Slot::Aggregate,
        Lock::statement(
            "the row of the schedule",
            format!(
                "SELECT 1 FROM schedules WHERE tenant_id = '{}' AND id = '{}' FOR UPDATE",
                tenant(f),
                schedule.id.as_uuid()
            ),
        ),
    )
}

fn run_row(f: &Fixture, run: Uuid) -> (Slot, Lock) {
    (
        Slot::ExistingRuns,
        Lock::statement(
            "the row of the run",
            format!(
                "SELECT 1 FROM runs WHERE tenant_id = '{}' AND id = '{run}' FOR UPDATE",
                tenant(f)
            ),
        ),
    )
}

/// What one run of a case does: the locks of the reference, and the path, which answers whether it did what it was asked.
struct Prepared<'a> {
    reference: Vec<(Slot, Lock)>,
    path: Pin<Box<dyn Future<Output = Result<(), String>> + 'a>>,
}

type Preparing<'a> = Pin<Box<dyn Future<Output = Prepared<'a>> + 'a>>;

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(what.to_string())
    }
}

/// Runs the path of `case` once for every start of its reference: `prepare(k)` makes the state of the run `k` (a schedule of its
/// own, a run of its own) and its reference, and the reference holds its lock `k` while the path is blocked by it.
async fn at_every_start<'a, P>(f: &'a Fixture, case: &str, prepare: P)
where
    P: Fn(usize) -> Preparing<'a>,
{
    let first = prepare(0).await;
    let locks = first.reference.len();
    assert!(locks >= 2, "{case}: a reference of at least two locks");
    let mut prepared = Some(first);
    for start in 0..locks - 1 {
        let run = match prepared.take() {
            Some(run) => run,
            None => prepare(start).await,
        };
        let held = run.reference[start].1.name;
        let seen = against_at(&f.w.db.pool, &run.reference, start, run.path).await;
        assert_eq!(
            seen.reference,
            Outcome::Committed,
            "{case}, the reference holding `{held}` (lock {start} of {locks}): the path holds a lock the reference takes after it \
             while it waits, or was a deadlock victim itself: {seen:?}"
        );
        assert_eq!(
            seen.path,
            Ok(()),
            "{case}, the reference holding `{held}` (lock {start} of {locks})"
        );
    }
}

// --- the schedule pass and the commands of a person -----------------------------------------------------------------

#[tokio::test]
async fn a_scheduled_action_takes_the_schedule_then_the_authority_then_the_quotas_then_the_usage() {
    let f = fixture().await;
    let fx = &f;
    at_every_start(fx, "a scheduled action", |k| {
        Box::pin(async move {
            let schedule = fx
                .create(
                    &format!("slot-order-action-{k}"),
                    &hourly(),
                    &json!({}),
                    None,
                )
                .await;
            fx.clock.advance(Duration::hours(1) + Duration::seconds(1));
            let fence = fence(fx, schedule.id.as_uuid()).await;
            let mut reference = vec![schedule_row(fx, &schedule)];
            reference.extend(admission(fx));
            Prepared {
                reference,
                path: Box::pin(async move {
                    let fired = fx
                        .firing
                        .fire(&fx.due(&schedule), Some(fence))
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    ensure(fired == Fired::Acted, format!("{fired:?}"))?;
                    ensure(fx.runs(&schedule).await.len() == 1, "it started its run")
                }),
            }
        })
    })
    .await;
}

#[tokio::test]
async fn trigger_now_takes_the_schedule_then_the_authority_then_the_quotas_then_the_usage() {
    let f = fixture().await;
    let fx = &f;
    at_every_start(fx, "trigger-now", |k| {
        Box::pin(async move {
            let schedule = fx
                .create(
                    &format!("slot-order-trigger-{k}"),
                    &hourly(),
                    &json!({}),
                    None,
                )
                .await;
            let mut reference = vec![schedule_row(fx, &schedule)];
            reference.extend(admission(fx));
            Prepared {
                reference,
                path: Box::pin(async move {
                    let key = format!("slot-order-trigger-now-{k}");
                    let action = fx
                        .firing
                        .trigger(
                            &fx.w.admin,
                            schedule.id,
                            &TriggerRequest {
                                overlap: None,
                                key: &key,
                            },
                            &RequestContext::default(),
                        )
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    ensure(action.outcome == "STARTED", &action.outcome)?;
                    ensure(fx.runs(&schedule).await.len() == 1, "it started its run")
                }),
            }
        })
    })
    .await;
}

#[tokio::test]
async fn a_backfill_step_takes_the_schedule_the_backfill_then_the_authority_the_quotas_and_the_usage()
 {
    let f = fixture().await;
    let fx = &f;
    // every start has a backfill of its own that stays in progress: room for them, the deployment ceiling first, then the limit
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'schedule_backfills', 64, 'operator')",
    )
    .bind(tenant(fx))
    .execute(&fx.w.db.pool)
    .await
    .expect("a higher ceiling");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'schedule_backfills', 64, 'admin')",
    )
    .bind(tenant(fx))
    .bind(fx.w.namespace.id().as_uuid())
    .execute(&fx.w.db.pool)
    .await
    .expect("room for the backfills");
    at_every_start(fx, "a backfill step", |k| {
        Box::pin(async move {
            let schedule = fx
                .create(&format!("slot-order-backfill-{k}"), &hourly(), &json!({}), None)
                .await;
            let backfill = fx
                .service
                .create_backfill(
                    &fx.w.admin,
                    schedule.id,
                    &BackfillRequest {
                        start: t0() - Duration::hours(5),
                        end: t0(),
                        overlap: None,
                    },
                    &RequestContext::default(),
                )
                .await
                .expect("a backfill");
            let fence = fence(fx, schedule.id.as_uuid()).await;
            let mut reference = vec![
                schedule_row(fx, &schedule),
                (
                    Slot::Aggregate,
                    Lock::statement(
                        "the row of the backfill",
                        format!(
                            "SELECT 1 FROM schedule_backfills WHERE tenant_id = '{}' AND schedule_id = '{}' FOR UPDATE",
                            tenant(fx),
                            schedule.id.as_uuid()
                        ),
                    ),
                ),
            ];
            reference.extend(admission(fx));
            Prepared {
                reference,
                path: Box::pin(async move {
                    let fired = fx
                        .firing
                        .backfill_step(
                            &DueBackfill {
                                tenant_id: backfill.tenant_id,
                                schedule: backfill.schedule,
                                id: backfill.id,
                            },
                            Some(fence),
                        )
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    ensure(fired == Fired::Acted, format!("{fired:?}"))?;
                    ensure(fx.runs(&schedule).await.len() == 1, "one time, one run")
                }),
            }
        })
    })
    .await;
}

#[tokio::test]
async fn a_buffered_time_that_is_released_takes_the_schedule_then_the_authority_the_quotas_and_the_usage()
 {
    let f = fixture().await;
    let fx = &f;
    at_every_start(fx, "the release of a buffered time", |k| {
        Box::pin(async move {
            let schedule = fx
                .create(
                    &format!("slot-order-buffer-{k}"),
                    &hourly(),
                    &json!({"overlap": "BUFFER_ONE"}),
                    None,
                )
                .await;
            fx.clock.advance(Duration::hours(1) + Duration::seconds(1));
            assert_eq!(fx.fire(&schedule).await, Fired::Acted, "13:00 starts");
            let first = fx.runs(&schedule).await[0];
            fx.clock.advance(Duration::hours(1));
            assert_eq!(fx.fire(&schedule).await, Fired::Acted, "14:00 is buffered");
            fx.end(first, "SUCCEEDED").await;
            // a time that waits is looked at again every five seconds
            fx.clock.advance(Duration::seconds(6));
            // the first step forgets the run that ended; the next one releases the time that waited
            assert_eq!(
                fx.fire(&schedule).await,
                Fired::Acted,
                "the run is forgotten"
            );
            let fence = fence(fx, schedule.id.as_uuid()).await;
            let mut reference = vec![schedule_row(fx, &schedule)];
            reference.extend(admission(fx));
            Prepared {
                reference,
                path: Box::pin(async move {
                    let fired = fx
                        .firing
                        .fire(&fx.due(&schedule), Some(fence))
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    ensure(fired == Fired::Acted, format!("{fired:?}"))?;
                    ensure(
                        fx.runs(&schedule).await.len() == 2,
                        "the buffered time started",
                    )
                }),
            }
        })
    })
    .await;
}

#[tokio::test]
async fn paying_a_cancel_other_debt_takes_the_run_then_the_debts_then_the_usage() {
    let f = fixture().await;
    let fx = &f;
    at_every_start(fx, "paying a debt", |k| {
        Box::pin(async move {
            let (schedule, open) =
                waiting_behind_an_open_run(fx, &format!("slot-order-cancel-other-{k}"), cancel_other())
                    .await;
            let fence = fence(fx, schedule.id.as_uuid()).await;
            // order (b): the run first, then the row of the bookkeeping of the aggregate, the usage last
            let reference = vec![
                run_row(fx, open),
                (
                    Slot::Bookkeeping,
                    Lock::statement(
                        "the debts of the schedule",
                        format!(
                            "SELECT 1 FROM schedule_obligations WHERE tenant_id = '{}' AND schedule_id = '{}' FOR UPDATE",
                            tenant(fx),
                            schedule.id.as_uuid()
                        ),
                    ),
                ),
                usage(fx),
            ];
            Prepared {
                reference,
                path: Box::pin(async move {
                    let fired = fx
                        .firing
                        .fire(&fx.due(&schedule), Some(fence))
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    ensure(fired == Fired::Acted, format!("{fired:?}"))?;
                    let paid: Vec<String> = sqlx::query_scalar(
                        "SELECT state FROM schedule_obligations WHERE schedule_id = $1",
                    )
                    .bind(schedule.id.as_uuid())
                    .fetch_all(&fx.w.db.pool)
                    .await
                    .map_err(|e| e.to_string())?;
                    ensure(paid == ["DONE"], format!("the debt was paid: {paid:?}"))
                }),
            }
        })
    })
    .await;
}

// --- the paths of a run --------------------------------------------------------------------------------------------

/// The release of a delayed start rides on the passes of the runs: the run's row, and the lease of its shard (`FOR SHARE`, a
/// leaf lock that no reference names), then the usage last.
#[tokio::test]
async fn the_release_of_a_delayed_start_takes_the_run_then_the_usage() {
    let f = fixture().await;
    let fx = &f;
    at_every_start(fx, "the release of a delayed start", |k| {
        Box::pin(async move {
            let held = due_run(fx, &format!("slot-order-delay-key-{k}")).await;
            let passes = shard_owner(fx).await;
            let reference = vec![run_row(fx, held.as_uuid()), usage(fx)];
            Prepared {
                reference,
                path: Box::pin(async move {
                    let report = passes.tick().await;
                    ensure(report.starts_released == 1, format!("{report:?}"))
                }),
            }
        })
    })
    .await;
}

#[tokio::test]
async fn the_claim_of_a_business_identifier_takes_the_binding_then_the_quotas_and_the_usage() {
    let f = fixture().await;
    let fx = &f;
    at_every_start(fx, "the claim of an identifier", |k| {
        Box::pin(async move {
            let id = format!("slot-order-id-{k}");
            // the identifier is bound to a run that has ended: the next start finds the binding, locks it and reuses it
            let holder = start_under(fx, &id, &format!("slot-order-first-key-{k}"))
                .await
                .expect("the first start");
            fx.end(holder.as_uuid(), "SUCCEEDED").await;
            let mut reference = vec![(
                Slot::Aggregate,
                Lock::statement(
                    "the binding of the identifier",
                    format!(
                        "SELECT 1 FROM business_ids WHERE tenant_id = '{}' AND namespace_id = '{}' AND business_id = '{id}' FOR UPDATE",
                        tenant(fx),
                        fx.w.namespace.id().as_uuid()
                    ),
                ),
            )];
            // a person's start is judged by the role of the caller, not by a lock on the memberships: the authority is not in this
            // reference (the paths of a schedule and the completion of a reset look at it again under the lock)
            reference.extend(
                admission(fx)
                    .into_iter()
                    .filter(|(slot, _)| *slot != Slot::Authority),
            );
            Prepared {
                reference,
                path: Box::pin(async move {
                    let second = start_under(fx, &id, &format!("slot-order-second-key-{k}")).await?;
                    ensure(second != holder, "the identifier was reused by a new run")?;
                    let bound: String =
                        sqlx::query_scalar("SELECT execution_id::text FROM business_ids WHERE business_id = $1")
                            .bind(&id)
                            .fetch_one(&fx.w.db.pool)
                            .await
                            .map_err(|e| e.to_string())?;
                    ensure(bound == second.as_uuid().to_string(), "and holds it")
                }),
            }
        })
    })
    .await;
}

async fn start_under(
    f: &Fixture,
    id: &str,
    key: &str,
) -> Result<sagat_domain::execution::RunId, String> {
    f.w.runs
        .start_with_options(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow: f.workflow,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key,
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: BusinessId::new(id).expect("an identifier"),
                    conflict: ConflictPolicy::Fail,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
                ..StartOptions::default()
            },
            &ctx(),
        )
        .await
        .map(|started| started.run.info.run_id)
        .map_err(|e| format!("{e:?}"))
}

// --- the register of pending resets --------------------------------------------------------------------------------

#[tokio::test]
async fn the_request_of_a_reset_that_waits_takes_the_source_then_its_quota_then_the_usage() {
    let f = fixture().await;
    let fx = &f;
    let session = worker(fx).await;
    room_for_pending(fx, 64).await;
    let session = &session;
    at_every_start(fx, "the request of a reset that waits", |k| {
        Box::pin(async move {
            let open = open_pair(fx, session).await;
            // the deviation of ADR-0081 decision 5: the source (3R), then the quota of the pending resets (8b), then the entry
            let reference = vec![
                run_row(fx, open.run.as_uuid()),
                (
                    Slot::Usage,
                    advisory(
                        "the quota of the pending resets",
                        QuotaResource::PendingResets,
                        format!("{}:{}", tenant(fx), fx.w.namespace.id().as_uuid()),
                    ),
                ),
                usage(fx),
            ];
            Prepared {
                reference,
                path: Box::pin(async move {
                    let made =
                        fx.w.runs
                            .reset(
                                &fx.w.admin,
                                open.run,
                                asking(open.after_a),
                                &format!("slot-order-request-key-{k}"),
                                &ctx(),
                            )
                            .await
                            .map_err(|e| format!("{e:?}"))?;
                    ensure(
                        made.reset.state == sagat_application::runs::ResetState::PendingSource,
                        "the entry waits",
                    )
                }),
            }
        })
    })
    .await;
}

#[tokio::test]
async fn the_completion_of_a_reset_that_waited_takes_the_entry_then_the_source_the_authority_the_quotas_and_the_usage()
 {
    let f = fixture().await;
    let fx = &f;
    let session = worker(fx).await;
    room_for_pending(fx, 64).await;
    let session = &session;
    at_every_start(fx, "the completion of a reset", |k| {
        Box::pin(async move {
            let open = open_pair(fx, session).await;
            let entry = fx
                .w
                .runs
                .reset(
                    &fx.w.admin,
                    open.run,
                    asking(open.after_a),
                    &format!("slot-order-completion-key-{k}"),
                    &ctx(),
                )
                .await
                .expect("the request")
                .reset
                .id;
            finish(fx, session, &open).await;
            let mut reference = vec![
                (
                    Slot::Aggregate,
                    Lock::statement(
                        "the entry of the register",
                        format!(
                            "SELECT 1 FROM run_resets WHERE tenant_id = '{}' AND id = '{}' FOR UPDATE",
                            tenant(fx),
                            entry.as_uuid()
                        ),
                    ),
                ),
                run_row(fx, open.run.as_uuid()),
            ];
            reference.extend(admission(fx));
            let completer = completer(fx);
            let due = due(fx, entry);
            Prepared {
                reference,
                path: Box::pin(async move {
                    let done = completer
                        .step(&due, None)
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    ensure(
                        done == sagat_application::reset_completion::Completed::Created,
                        format!("{done:?}"),
                    )
                }),
            }
        })
    })
    .await;
}
