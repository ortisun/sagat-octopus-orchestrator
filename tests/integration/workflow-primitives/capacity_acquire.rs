//! Native dormant Acquire: real ChildRoot lineage, runtime role and retained receipts.

use super::*;
use sagat_application::capacity_acquire::{
    CapacityAcquireCommand, CapacityAcquireError as Error, CurrentChildRootAcquireStore,
};
use sagat_application::capacity_vacate::{CapacityVacateCommand, CurrentChildRootVacateStore};
use sagat_infra_postgres::{PgCapacityAcquireStore, PgCapacityVacateStore};

async fn command(n: &NativeFixture) -> CapacityAcquireCommand {
    let c = native_command(n, RunId::new(n.f.chain)).await;
    CapacityAcquireCommand {
        operation_key: c.operation_key,
        identity: c.identity,
        expected: c.expected,
        scheduler: c.scheduler,
    }
}

async fn vacant_fixture() -> NativeFixture {
    let n = super::barriers::runtime_fixture().await;
    let c = command(&n).await;
    PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&CapacityVacateCommand {
            operation_key: c.operation_key,
            identity: c.identity,
            expected: c.expected,
            scheduler: c.scheduler,
        })
        .await
        .expect("real quiescent ChildRoot Vacate precedes Acquire");
    n
}

async fn counts(f: &Fixture) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2 AND kind='ACQUIRE'),(SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$3 AND action='capacity.member.acquired'),(SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='capacity.member.acquired')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(f.group.to_string())
        .fetch_one(&f.w.db.pool).await.expect("Acquire side effects")
}

async fn header(f: &Fixture) -> (i64, i64, i64, String, Option<Uuid>) {
    sqlx::query_as("SELECT revision,epoch,active_members,possession,holder_member_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group)
        .fetch_one(&f.w.db.pool).await.expect("authoritative capacity header")
}

#[tokio::test]
async fn vacant_child_acquires_atomically_without_rewriting_execution_or_membership() {
    crate::support::bounded::within(
        "native ChildRoot Acquire apply",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let before = execution_snapshot(&n.f).await;
            let versions = super::vacate::untouched(&n.f).await;
            let result = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                .acquire_current_child_root(&c)
                .await
                .expect("vacant current ChildRoot acquires");
            assert_eq!(
                (
                    result.revision,
                    result.epoch,
                    result.active_members,
                    result.possession,
                    result.replayed
                ),
                (
                    3,
                    3,
                    1,
                    CapacityPossession::Held(CapacityMemberId::new(n.f.member)),
                    false
                )
            );
            assert_eq!(
                state(&n.f.w.db.pool, &n.f).await,
                (3, 3, 1, "HELD".into(), "ACTIVE".into())
            );
            assert_eq!(counts(&n.f).await, (1, 1, 1));
            let hints =
                super::super::hint_contract::assert_outbox_matches_the_contract(&n.f.w.db.pool)
                    .await;
            super::super::hint_contract::assert_published(&hints, &["capacity.member.acquired"]);
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(super::vacate::untouched(&n.f).await, versions);
        },
    )
    .await;
}

#[tokio::test]
async fn exact_replay_returns_the_original_acquisition_once() {
    crate::support::bounded::within(
        "native ChildRoot Acquire replay",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let a = store.acquire_current_child_root(&c).await.expect("acquire");
            let mut b = store
                .acquire_current_child_root(&c)
                .await
                .expect("receipt precedes occupied gate");
            assert!(b.replayed);
            b.replayed = false;
            assert_eq!(a, b);
            assert_eq!(counts(&n.f).await, (1, 1, 1));
        },
    )
    .await;
}

#[tokio::test]
async fn occupied_group_refuses_a_fresh_acquire_without_side_effects() {
    crate::support::bounded::within(
        "native ChildRoot Acquire occupied refusal",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = super::barriers::runtime_fixture().await;
            let c = command(&n).await;
            let before = execution_snapshot(&n.f).await;
            assert_eq!(
                PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                    .acquire_current_child_root(&c)
                    .await,
                Err(Error::Occupied)
            );
            assert_eq!(
                state(&n.f.w.db.pool, &n.f).await,
                (1, 1, 1, "HELD".into(), "ACTIVE".into())
            );
            assert_eq!(counts(&n.f).await, (0, 0, 0));
            assert_eq!(execution_snapshot(&n.f).await, before);
        },
    )
    .await;
}

#[tokio::test]
async fn mismatched_binding_stale_counters_and_lost_scheduler_are_inert() {
    crate::support::bounded::within(
        "mismatched_binding_stale_counters_and_lost_scheduler_are_inert",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let original = command(&n).await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let before = execution_snapshot(&n.f).await;
            for field in 0..7 {
                let mut c = original.clone();
                let expected = match field {
                    0 => {
                        c.expected.group_revision += 1;
                        Error::StaleGroupRevision {
                            current: original.expected.group_revision,
                        }
                    }
                    1 => {
                        c.expected.group_epoch += 1;
                        Error::StaleGroupEpoch {
                            current: original.expected.group_epoch,
                        }
                    }
                    2 => {
                        c.expected.chain_revision += 1;
                        Error::StaleChainRevision {
                            current: original.expected.chain_revision,
                        }
                    }
                    3 => {
                        c.expected.run_revision += 1;
                        Error::StaleRunRevision {
                            current: original.expected.run_revision,
                        }
                    }
                    4 => {
                        c.expected.run_revision = u64::MAX;
                        Error::CounterExhausted
                    }
                    5 => {
                        c.scheduler =
                            SchedulerAuthority::for_run("not-the-owner", c.identity.head_run_id, 1);
                        Error::SchedulerFenceLost
                    }
                    6 => {
                        c.identity.scope = ChainScope::new(
                            n.f.w.other_admin.tenant_id(),
                            n.f.w.other_namespace.id(),
                        );
                        Error::Missing
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    store.acquire_current_child_root(&c).await,
                    Err(expected),
                    "refusal {field}"
                );
            }
            assert_eq!(counts(&n.f).await, (0, 0, 0));
            assert_eq!(execution_snapshot(&n.f).await, before);
        },
    )
    .await;
}

#[tokio::test]
async fn paused_and_manual_review_are_not_movable_but_running_is() {
    crate::support::bounded::within(
        "Acquire movable status",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            // Owner-only projection fixtures isolate the status gate; they do not claim public transitions.
            for status in ["PAUSED", "MANUAL_REVIEW"] {
                sqlx::query("UPDATE runs SET status=$3 WHERE tenant_id=$1 AND id=$2")
                    .bind(n.f.w.admin.tenant_id().as_uuid())
                    .bind(n.f.chain)
                    .bind(status)
                    .execute(&n.f.w.db.pool)
                    .await
                    .expect("status gate fixture");
                let c = command(&n).await;
                let before = execution_snapshot(&n.f).await;
                assert_eq!(
                    store.acquire_current_child_root(&c).await,
                    Err(Error::NotMovable)
                );
                assert_eq!(execution_snapshot(&n.f).await, before);
                assert_eq!(counts(&n.f).await, (0, 0, 0));
            }
            sqlx::query("UPDATE runs SET status='RUNNING' WHERE tenant_id=$1 AND id=$2")
                .bind(n.f.w.admin.tenant_id().as_uuid())
                .bind(n.f.chain)
                .execute(&n.f.w.db.pool)
                .await
                .expect("restore movable fixture");
            store
                .acquire_current_child_root(&command(&n).await)
                .await
                .expect("running is movable without runnable-work probe");
        },
    )
    .await;
}

#[tokio::test]
async fn final_child_cannot_acquire_and_cross_operation_keys_conflict_before_cas() {
    crate::support::bounded::within("Acquire finality and receipt domain", crate::support::bounded::LIFECYCLE_LIMIT, async {
            let n = vacant_fixture().await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let mut c = command(&n).await;
            let vacate_key: Uuid = sqlx::query_scalar("SELECT operation_key FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2 AND kind='VACATE'")
                    .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).fetch_one(&n.f.w.db.pool).await.expect("original Vacate receipt");
            c.operation_key = vacate_key;
            assert_eq!(
                store.acquire_current_child_root(&c).await,
                Err(Error::FingerprintConflict)
            );
            complete_native_activity(&n).await;
            let c = command(&n).await;
            let before = execution_snapshot(&n.f).await;
            assert_eq!(
                store.acquire_current_child_root(&c).await,
                Err(Error::ChainFinal)
            );
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(counts(&n.f).await, (0, 0, 0));
        }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn acquire_exports_closed_outcomes_and_counts_only_acknowledged_first_transitions() {
    use crate::support::collector::Collector;
    use sagat_telemetry::{Metrics, TraceId};
    use std::time::Duration;
    use tracing::Instrument;
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;
    let n = vacant_fixture().await;
    let c = command(&n).await;
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let subscriber =
        tracing_subscriber::registry().with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0));
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    async {
        let parent = tracing::info_span!(target:"sagat_test","acquire.caller",trace_id=%trace);
        async {
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            store.acquire_current_child_root(&c).await.expect("applied");
            assert!(
                store
                    .acquire_current_child_root(&c)
                    .await
                    .expect("replayed")
                    .replayed
            );
            let next = command(&n).await;
            assert_eq!(
                store.acquire_current_child_root(&next).await,
                Err(Error::Occupied)
            );
        }
        .instrument(parent)
        .await;
    }
    .with_subscriber(subscriber)
    .await;
    sink.shutdown(Duration::from_secs(5)).await;
    let spans = collector.named("workflow.capacity.acquire");
    assert_eq!(spans.len(), 3);
    let outcomes: std::collections::BTreeSet<_> = spans
        .iter()
        .map(|s| {
            assert_eq!(s.trace_id, trace.to_string());
            s.attr("sagat.outcome").expect("closed outcome").to_owned()
        })
        .collect();
    assert_eq!(
        outcomes,
        std::collections::BTreeSet::from([
            "applied".to_owned(),
            "replayed".to_owned(),
            "occupied".to_owned()
        ])
    );
    let page = metrics.render();
    for outcome in ["applied", "replayed", "occupied"] {
        let total: u64 = page
            .lines()
            .filter(|l| {
                l.starts_with("workflow_capacity_transfer_attempts_total{")
                    && l.contains(&format!("outcome=\"{outcome}\""))
                    && l.contains("operation=\"acquire\"")
            })
            .map(|l| {
                l.rsplit(' ')
                    .next()
                    .expect("sample")
                    .parse::<u64>()
                    .expect("counter")
            })
            .sum();
        assert_eq!(total, 1);
    }
    let transition: u64 = page
        .lines()
        .filter(|l| l.starts_with("workflow_capacity_member_acquired_total "))
        .map(|l| {
            l.rsplit(' ')
                .next()
                .expect("sample")
                .parse::<u64>()
                .expect("counter")
        })
        .sum();
    assert_eq!(transition, 1);
    for line in page.lines().filter(|l| l.starts_with("workflow_capacity_")) {
        assert!(!line.contains(&n.f.group.to_string()));
        assert!(!line.contains(&n.f.w.admin.tenant_id().to_string()));
    }
}

#[tokio::test]
async fn original_acquisition_replays_after_later_vacate_continuation_and_successor_lease() {
    crate::support::bounded::within(
        "Acquire historical reconciliation",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let mut n = native_fixture(true).await;
            n.f.w.execution_pool = n.f.w.db.runtime_pool_with(16).await;
            let initial = command(&n).await;
            let vacate = PgCapacityVacateStore::new(n.f.w.execution_pool.clone());
            vacate
                .vacate_current_child_root(&CapacityVacateCommand {
                    operation_key: initial.operation_key,
                    identity: initial.identity,
                    expected: initial.expected,
                    scheduler: initial.scheduler,
                })
                .await
                .expect("initial vacancy");
            let c = command(&n).await;
            let acquire = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let first = acquire
                .acquire_current_child_root(&c)
                .await
                .expect("first acquisition");
            let later = command(&n).await;
            vacate
                .vacate_current_child_root(&CapacityVacateCommand {
                    operation_key: later.operation_key,
                    identity: later.identity,
                    expected: later.expected,
                    scheduler: later.scheduler,
                })
                .await
                .expect("later real Vacate");
            let mut replay = acquire
                .acquire_current_child_root(&c)
                .await
                .expect("original Held receipt after later vacancy");
            assert!(replay.replayed);
            replay.replayed = false;
            assert_eq!(replay, first);
            assert_eq!(
                state(&n.f.w.db.pool, &n.f).await,
                (4, 4, 1, "VACANT".into(), "ACTIVE".into())
            );
            complete_native_activity(&n).await;
            for _ in 0..8 {
                let round = n.scheduler.tick().await;
                assert_eq!(round.failed, 0);
                if !round.did_work() {
                    break;
                }
            }
            let head: Uuid = sqlx::query_scalar(
                "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
            )
            .bind(n.f.w.admin.tenant_id().as_uuid())
            .bind(n.f.chain)
            .fetch_one(&n.f.w.db.pool)
            .await
            .expect("continued head");
            assert_ne!(head, n.f.chain);
            let session = n.f.w.session(&["capacity.work"]).await;
            let tasks = n.f.w.poll(&session, 1).await;
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].run_id.as_uuid(), head);
            let before = execution_snapshot(&n.f).await;
            let mut replay = acquire
                .acquire_current_child_root(&c)
                .await
                .expect("historical receipt before current-head and lease gates");
            assert!(replay.replayed);
            replay.replayed = false;
            assert_eq!(replay, first);
            let mut stale = c.clone();
            stale.operation_key = Uuid::new_v4();
            assert_eq!(
                acquire.acquire_current_child_root(&stale).await,
                Err(Error::StaleHead {
                    current: RunId::new(head)
                })
            );
            let mut conflict = c.clone();
            conflict.expected.run_revision += 1;
            assert_eq!(
                acquire.acquire_current_child_root(&conflict).await,
                Err(Error::FingerprintConflict)
            );
            assert_eq!(counts(&n.f).await, (1, 1, 1));
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(header(&n.f).await, (4, 4, 1, "VACANT".into(), None));
            assert_eq!(state(&n.f.w.db.pool, &n.f).await.4, "ACTIVE");
        },
    )
    .await;
}

#[tokio::test]
async fn original_receipt_survives_actual_finish_but_fresh_acquire_refuses_inactive_member() {
    crate::support::bounded::within(
        "Acquire receipt after Finish",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let c = command(&n).await;
            let acquire = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let original = acquire
                .acquire_current_child_root(&c)
                .await
                .expect("acquisition");
            let head = complete_native_activity(&n).await;
            PgCapacityFinishStore::new(n.f.w.execution_pool.clone())
                .finish_current_child_root(&native_command(&n, head).await)
                .await
                .expect("actual Finish retires last active member");
            let before = state(&n.f.w.db.pool, &n.f).await;
            assert_eq!(
                (before.2, before.3.as_str(), before.4.as_str()),
                (0, "RETIRED", "FINISHED")
            );
            let snapshot = execution_snapshot(&n.f).await;
            let mut replay = acquire
                .acquire_current_child_root(&c)
                .await
                .expect("original receipt before inactive and final gates");
            assert!(replay.replayed);
            replay.replayed = false;
            assert_eq!(replay, original);
            assert_eq!(
                acquire.acquire_current_child_root(&command(&n).await).await,
                Err(Error::InactiveMember)
            );
            assert_eq!(state(&n.f.w.db.pool, &n.f).await, before);
            assert_eq!(execution_snapshot(&n.f).await, snapshot);
            assert_eq!(counts(&n.f).await, (1, 1, 1));
        },
    )
    .await;
}

#[tokio::test]
async fn exact_binding_and_persistence_counter_ceiling_refuse_without_mutation() {
    crate::support::bounded::within(
        "Acquire binding and counter ceiling",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = vacant_fixture().await;
            let original = command(&n).await;
            let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
            let before = execution_snapshot(&n.f).await;
            for field in 0..4 {
                let mut c = original.clone();
                let expected = match field {
                    0 => {
                        c.identity.scope =
                            ChainScope::new(n.f.w.admin.tenant_id(), n.f.w.other_namespace.id());
                        Error::BindingMismatch
                    }
                    1 => {
                        c.identity.member_id = CapacityMemberId::new(Uuid::new_v4());
                        Error::Missing
                    }
                    2 => {
                        c.identity.chain_id = n.parent;
                        Error::BindingMismatch
                    }
                    3 => {
                        c.identity.head_run_id = n.parent;
                        Error::BindingMismatch
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    store.acquire_current_child_root(&c).await,
                    Err(expected),
                    "binding {field}"
                );
            }
            assert_eq!(execution_snapshot(&n.f).await, before);
            // A legal stored header at PostgreSQL's upper counter bound cannot advance.
            sqlx::query("UPDATE capacity_groups SET revision=9223372036854775807,epoch=9223372036854775807 WHERE tenant_id=$1 AND group_id=$2")
                        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).execute(&n.f.w.db.pool).await.expect("legal counter ceiling fixture");
            let ceiling = state(&n.f.w.db.pool, &n.f).await;
            assert_eq!(
                store.acquire_current_child_root(&command(&n).await).await,
                Err(Error::CounterExhausted)
            );
            assert_eq!(state(&n.f.w.db.pool, &n.f).await, ceiling);
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(counts(&n.f).await, (0, 0, 0));
        },
    ).await;
}

#[path = "capacity_acquire_recovery.rs"]
mod recovery;

#[tokio::test]
async fn a_live_or_expired_worker_lease_is_not_an_acquire_readiness_predicate() {
    crate::support::bounded::within(
        "Acquire possession without worker readiness",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            for expired in [false, true] {
                let w = crate::support::engine::world_with(sagat_application::worker::WorkerSettings {
                    lease_ttl: std::time::Duration::from_secs(600),
                    session_ttl: std::time::Duration::from_secs(600),
                    ..sagat_application::worker::WorkerSettings::default()
                }).await;
                let (parent, child) = spawn_native_child(&w, false).await;
                let scheduler = super::super::chains::scheduler(&w, "capacity-acquire-long-lease").await;
                let round = scheduler.tick().await;
                assert_eq!(round.children_started, 1, "{round:?}");
                assert_eq!(round.failed, 0);
                let mut f = seed_capacity(w, child).await;
                f.w.execution_pool = f.w.db.runtime_pool_with(16).await;
                let n = NativeFixture { f, parent, scheduler };
                let initial = command(&n).await;
                PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
                    .vacate_current_child_root(&CapacityVacateCommand {
                        operation_key: initial.operation_key, identity: initial.identity,
                        expected: initial.expected, scheduler: initial.scheduler,
                    }).await.expect("vacancy before real lease");
                let session = n.f.w.session(&["capacity.work"]).await;
                let tasks = n.f.w.poll(&session, 1).await;
                assert_eq!(tasks.len(), 1);
                assert_eq!(tasks[0].run_id.as_uuid(), n.f.chain);
                if expired {
                    n.f.w.age_lease(RunId::new(n.f.chain), "work").await;
                }
                let c = command(&n).await;
                let before = execution_snapshot(&n.f).await;
                let versions = super::vacate::untouched(&n.f).await;
                let lease: (String, bool) = sqlx::query_as("SELECT status,lease_expires_at>clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='work'")
                    .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain)
                    .fetch_one(&n.f.w.db.pool).await.expect("actual lease liveness immediately before Acquire");
                assert_eq!(lease, ("LEASED".into(), !expired));
                PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                    .acquire_current_child_root(&c)
                    .await
                    .expect("Acquire does not infer runnable work or inspect lease eligibility");
                assert_eq!(
                    header(&n.f).await,
                    (3, 3, 1, "HELD".into(), Some(n.f.member))
                );
                assert_eq!(execution_snapshot(&n.f).await, before);
                assert_eq!(super::vacate::untouched(&n.f).await, versions);
                assert_eq!(counts(&n.f).await, (1, 1, 1));
            }
        },
    )
    .await;
}
