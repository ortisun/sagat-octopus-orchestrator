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
async fn busy_group_rolls_back_and_releases_own_chain_and_run_guards() {
    crate::support::bounded::within("busy_group_rolls_back_and_releases_own_chain_and_run_guards", crate::support::bounded::LIFECYCLE_LIMIT, async {

    use tokio::time::{Duration, timeout};
    let n = vacant_fixture().await;
    let c = command(&n).await;
    let mut gate = n.f.w.db.pool.begin().await.expect("group barrier");
    sqlx::query(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.group)
    .execute(&mut *gate)
    .await
    .expect("hold group");
    let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        timeout(Duration::from_secs(2), store.acquire_current_child_root(&c))
            .await
            .expect("NOWAIT"),
        Err(Error::Busy)
    );
    let mut probe = n.f.w.db.pool.begin().await.expect("lock release probe");
    sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE NOWAIT")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain).execute(&mut *probe).await.expect("chain released");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
        .bind(n.f.w.admin.tenant_id().as_uuid())
        .bind(n.f.chain)
        .execute(&mut *probe)
        .await
        .expect("run released");
    probe.rollback().await.expect("probe ends");
    gate.rollback().await.expect("release group");
    assert_eq!(counts(&n.f).await, (0, 0, 0));
    store
        .acquire_current_child_root(&c)
        .await
        .expect("same command retries from fresh transaction");

    }).await;
}

#[tokio::test]
async fn outbox_refusal_rolls_back_header_audit_and_receipt_before_same_key_retry() {
    crate::support::bounded::within("outbox_refusal_rolls_back_header_audit_and_receipt_before_same_key_retry", crate::support::bounded::LIFECYCLE_LIMIT, async {

    let n = vacant_fixture().await;
    let c = command(&n).await;
    let store = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone());
    let before = execution_snapshot(&n.f).await;
    sqlx::raw_sql("CREATE FUNCTION refuse_acquire_hint() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_type='capacity.member.acquired' THEN RAISE EXCEPTION 'controlled outbox refusal' USING ERRCODE='23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_acquire_hint BEFORE INSERT ON event_outbox FOR EACH ROW EXECUTE FUNCTION refuse_acquire_hint();")
        .execute(&n.f.w.db.pool).await.expect("controlled outbox fault");
    assert!(matches!(
        store.acquire_current_child_root(&c).await,
        Err(Error::Storage(_))
    ));
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (2, 2, 1, "VACANT".into(), "ACTIVE".into())
    );
    assert_eq!(counts(&n.f).await, (0, 0, 0));
    assert_eq!(execution_snapshot(&n.f).await, before);
    sqlx::query("DROP TRIGGER refuse_acquire_hint ON event_outbox")
        .execute(&n.f.w.db.pool)
        .await
        .expect("remove owned test fault");
    store
        .acquire_current_child_root(&c)
        .await
        .expect("original command retries atomically");
    assert_eq!(counts(&n.f).await, (1, 1, 1));

    }).await;
}

#[tokio::test]
async fn lost_commit_ack_reconciles_actual_commit_or_abort_with_the_original_key() {
    crate::support::bounded::within("lost_commit_ack_reconciles_actual_commit_or_abort_with_the_original_key", crate::support::bounded::LIFECYCLE_LIMIT, async {

    use super::commit_proxy::{CommitAckLossProxy, CommitResolution};
    use crate::support::eventually;
    use tokio::time::{Duration, timeout};
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        let n = vacant_fixture().await;
        let c = command(&n).await;
        let before = execution_snapshot(&n.f).await;
        sqlx::raw_sql("CREATE TABLE acquire_commit_outcome(abort boolean NOT NULL); GRANT SELECT ON acquire_commit_outcome TO sagat_app; CREATE FUNCTION acquire_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(9407001); IF (SELECT abort FROM acquire_commit_outcome) THEN RAISE EXCEPTION 'controlled Acquire commit abort' USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort'; END IF; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER acquire_commit_probe AFTER INSERT ON capacity_group_transfer_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION acquire_commit_probe();")
            .execute(&n.f.w.db.pool).await.expect("controlled deferred native commit probe");
        sqlx::query("INSERT INTO acquire_commit_outcome VALUES($1)")
            .bind(resolution == CommitResolution::Aborted23514)
            .execute(&n.f.w.db.pool)
            .await
            .expect("selected real server outcome");
        let mut gate = n.f.w.db.pool.begin().await.expect("commit barrier");
        sqlx::query("SELECT pg_advisory_xact_lock(9407001)")
            .execute(&mut *gate)
            .await
            .expect("hold commit gate");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("gate pid");
        let mut proxy = CommitAckLossProxy::start(&n.f.w.db, resolution).await;
        proxy.arm();
        let pool = proxy.pool.clone();
        let original = c.clone();
        let pending = tokio::spawn(async move {
            PgCapacityAcquireStore::new(pool)
                .acquire_current_child_root(&original)
                .await
        });
        proxy.wait_sent().await;
        eventually("actual COMMIT is still executing behind the owned gate",||async {
            let seen:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND application_name='sagat-test-capacity-commit-loss' AND query='COMMIT' AND $1=ANY(pg_blocking_pids(pid)))")
                .bind(pid).fetch_one(&n.f.w.db.pool).await.expect("actual commit wait");seen.then_some(())
        }).await;
        assert_eq!(counts(&n.f).await, (0, 0, 0));
        proxy.lose_reply().await;
        assert_eq!(
            timeout(Duration::from_secs(5), pending)
                .await
                .expect("unknown outcome deadline")
                .expect("native task"),
            Err(Error::CommitOutcomeUnknown {
                operation_key: c.operation_key
            })
        );
        gate.rollback()
            .await
            .expect("release unresolved server COMMIT");
        assert_eq!(proxy.resolution().await, resolution);
        proxy.close().await;
        let expected = i64::from(resolution == CommitResolution::Committed);
        assert_eq!(counts(&n.f).await, (expected, expected, expected));
        assert_eq!(execution_snapshot(&n.f).await, before);
        sqlx::query("UPDATE acquire_commit_outcome SET abort=false")
            .execute(&n.f.w.db.pool)
            .await
            .expect("clear controlled abort for original-key retry");
        let reconciled = PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
            .acquire_current_child_root(&c)
            .await
            .expect("same key reconciles server outcome");
        assert_eq!(
            reconciled.replayed,
            resolution == CommitResolution::Committed
        );
        assert_eq!(counts(&n.f).await, (1, 1, 1));
    }

    }).await;
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
        c.operation_key=vacate_key;
        assert_eq!(store.acquire_current_child_root(&c).await, Err(Error::FingerprintConflict));
        complete_native_activity(&n).await;
        let c=command(&n).await;
        let before=execution_snapshot(&n.f).await;
        assert_eq!(store.acquire_current_child_root(&c).await, Err(Error::ChainFinal));
        assert_eq!(execution_snapshot(&n.f).await,before);
        assert_eq!(counts(&n.f).await,(0,0,0));
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
