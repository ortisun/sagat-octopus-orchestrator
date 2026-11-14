//! The instance-capability registry under races and faults (`T018-080`, `SC-018-017`, `NFR-018-001`): an activation that
//! races a registration never leaves an instance serving that lacks the capability, in either order; a table counted by
//! many registrations at once never holds more than its limit; a registry that cannot be asked at the start means no
//! serving, and one that fails while the process serves changes nothing until the next beat that succeeds.
//!
//! The races are held by locks and watched in `pg_stat_activity`, never timed. The age of a heartbeat is written.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sagat_application::error::StoreError;
use sagat_application::instances::{
    Activated, Activation, Beat, Capability, ChangeRefused, Heartbeat, InstanceKind, InstanceStore,
    InstanceToken, Member, Refusal, Registered, Registration, Requirements,
};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::PgInstanceStore;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::support::instances::{Registry, registration, registry};

const ENGINE: InstanceKind = InstanceKind::ExecutionEngine;
const SCHEDULES: Capability = Capability::SchedulesV1;

/// An activation that stops after it has locked the epoch, verified, recorded and run its own step, and holds the
/// transaction open until released. Returns when it holds it.
struct HeldActivation {
    release: oneshot::Sender<()>,
    finished: tokio::task::JoinHandle<Result<Activated, ChangeRefused>>,
}

async fn hold_an_activation(store: &PgInstanceStore) -> HeldActivation {
    let (held_tx, held_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();
    let store = store.clone();
    let finished = tokio::spawn(async move {
        let actor = Subject::operator();
        store
            .activate(
                Activation {
                    capability: SCHEDULES,
                    kinds: &[ENGINE],
                    actor: &actor,
                },
                move |_| {
                    Box::pin(async move {
                        let _ = held_tx.send(());
                        let _ = release_rx.await;
                        Ok(())
                    })
                },
            )
            .await
    });
    crate::support::bounded::within(
        "the activation reaches its own step",
        std::time::Duration::from_secs(60),
        held_rx,
    )
    .await
    .expect("the activation reached its step");
    HeldActivation {
        release: release_tx,
        finished,
    }
}

#[tokio::test]
async fn a_registration_that_arrives_while_an_activation_holds_the_epoch_waits_and_is_then_refused()
{
    let r = registry().await;
    let held = hold_an_activation(&r.operator).await;

    // an old engine, and a new one, register while the activation is open
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    let process = r.process.clone();
    let registering_old = tokio::spawn({
        let process = process.clone();
        async move { process.register(&registration(old, ENGINE, &[])).await }
    });
    let registering_new = tokio::spawn(async move {
        process
            .register(&registration(new, ENGINE, &[SCHEDULES]))
            .await
    });
    r.wait_for_lock_waiters(2, "both registrations wait for the epoch")
        .await;
    assert!(
        !registering_old.is_finished() && !registering_new.is_finished(),
        "neither answers while the activation holds the epoch"
    );
    assert!(!r.row_exists(old).await && !r.row_exists(new).await);

    held.release.send(()).expect("release");
    assert_eq!(
        held.finished.await.expect("task").expect("activation"),
        Activated::Recorded { epoch: 1 }
    );
    assert_eq!(
        registering_old.await.expect("task").expect("register"),
        Registered::CapabilityMissing {
            epoch: 1,
            missing: vec!["schedules-v1".to_owned()]
        },
        "it waited, then met the requirement that committed"
    );
    assert_eq!(
        registering_new.await.expect("task").expect("register"),
        Registered::Serving { epoch: 1 }
    );
    assert!(
        !r.row_exists(old).await,
        "the old engine is not serving and not counted"
    );
    assert!(r.row_exists(new).await);
}

#[tokio::test]
async fn an_activation_that_arrives_while_a_registration_holds_the_epoch_waits_and_sees_the_instance()
 {
    let r = registry().await;
    // a registration that has taken the epoch share lock and written its row, and has not committed
    let old = Uuid::new_v4();
    let mut registering = r.db.pool.begin().await.expect("begin");
    let outcome: String = sqlx::query_scalar(
        "SELECT out_outcome FROM instance_register($1, $2, 'EXECUTION_ENGINE', NULL, '0.3.0', '{}')",
    )
    .bind(old)
    .bind(crate::support::instances::TEST_TOKEN.as_bytes().as_slice())
    .fetch_one(&mut *registering)
    .await
    .expect("register");
    assert_eq!(outcome, "REGISTERED");

    let operator = r.operator.clone();
    let activating = tokio::spawn(async move {
        let actor = Subject::operator();
        operator
            .activate(
                Activation {
                    capability: SCHEDULES,
                    kinds: &[ENGINE],
                    actor: &actor,
                },
                |_| Box::pin(async { Ok(()) }),
            )
            .await
    });
    r.wait_for_lock_waiters(1, "the activation waits for the epoch")
        .await;
    assert!(!activating.is_finished());
    assert_eq!(r.epoch_now().await, 0, "nothing is recorded yet");

    registering
        .commit()
        .await
        .expect("the registration commits");
    let refused = activating
        .await
        .expect("task")
        .expect_err("the instance it waited for lacks the capability and is live");
    assert_eq!(
        refused,
        ChangeRefused::InstancesLackCapability {
            missing: 1,
            examples: vec![old]
        }
    );
    assert_eq!((r.epoch_now().await, r.history().await), (0, 0));
}

#[tokio::test]
async fn a_removal_verifies_under_the_epoch_lock_so_a_registration_waits_for_it() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    let (held_tx, held_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();
    let operator = r.operator.clone();
    let removing = tokio::spawn(async move {
        let actor = Subject::operator();
        operator
            .remove(SCHEDULES, &actor, move |_| {
                Box::pin(async move {
                    let _ = held_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                })
            })
            .await
    });
    crate::support::bounded::within(
        "the removal reaches its verification",
        std::time::Duration::from_secs(60),
        held_rx,
    )
    .await
    .expect("the removal reached its verification");

    // an old engine registers while the feature verifies: it waits for the lock, and then meets what the removal decided
    let process = r.process.clone();
    let old = Uuid::new_v4();
    let registering =
        tokio::spawn(async move { process.register(&registration(old, ENGINE, &[])).await });
    r.wait_for_lock_waiters(1, "the registration waits for the epoch")
        .await;
    assert!(!registering.is_finished());
    assert!(!r.row_exists(old).await);

    release_tx.send(()).expect("release");
    assert_eq!(
        removing.await.expect("task").expect("removal"),
        sagat_application::instances::Removed::Removed { epoch: 2 }
    );
    assert_eq!(
        registering.await.expect("task").expect("register"),
        Registered::Serving { epoch: 2 },
        "the requirement was gone when it got the lock"
    );
}

#[tokio::test]
async fn however_registrations_and_an_activation_interleave_no_live_engine_lacks_a_recorded_capability()
 {
    // Not a barrier: many interleavings, one invariant. Whatever the order, a recorded requirement means every live
    // engine lists it, and a refused activation means one that lacked it was live.
    for round in 0..5 {
        let r = registry().await;
        let mut registrations = Vec::new();
        for i in 0..8 {
            let process = r.process.clone();
            let capabilities: Vec<Capability> = if i % 2 == 0 { vec![] } else { vec![SCHEDULES] };
            registrations.push(tokio::spawn(async move {
                let id = Uuid::new_v4();
                let got = process
                    .register(&registration(id, ENGINE, &capabilities))
                    .await
                    .expect("register");
                (id, capabilities.is_empty(), got)
            }));
        }
        let operator = r.operator.clone();
        let activation = tokio::spawn(async move {
            let actor = Subject::operator();
            operator
                .activate(
                    Activation {
                        capability: SCHEDULES,
                        kinds: &[ENGINE],
                        actor: &actor,
                    },
                    |_| Box::pin(async { Ok(()) }),
                )
                .await
        });
        let mut old_that_serve = Vec::new();
        for registered in registrations {
            let (id, lacks, got) = registered.await.expect("task");
            if lacks && matches!(got, Registered::Serving { .. }) {
                old_that_serve.push(id);
            }
        }
        let activation = activation.await.expect("task");
        let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirements")
            .fetch_one(&r.db.pool)
            .await
            .expect("count");
        let lacking_live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM serving_instances \
              WHERE kind = 'EXECUTION_ENGINE' AND NOT ('schedules-v1' = ANY (capabilities)) \
                AND last_seen > clock_timestamp() - interval '30 seconds'",
        )
        .fetch_one(&r.db.pool)
        .await
        .expect("count");
        match activation {
            Ok(Activated::Recorded { .. }) => {
                assert_eq!(recorded, 1, "round {round}");
                assert_eq!(
                    lacking_live, 0,
                    "round {round}: a requirement is recorded and an engine that lacks it is live and serving"
                );
            }
            Err(ChangeRefused::InstancesLackCapability { missing, examples }) => {
                assert_eq!(recorded, 0, "round {round}");
                assert!(missing >= 1, "round {round}");
                assert!(
                    examples.iter().all(|e| old_that_serve.contains(e)),
                    "round {round}: it named only engines that were accepted"
                );
            }
            other => panic!("round {round}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn registrations_counted_at_the_same_time_never_exceed_the_limit() {
    let r = registry().await;
    r.seed(990, ENGINE, 1).await;
    let mut registering = Vec::new();
    for _ in 0..24 {
        let process = r.process.clone();
        registering.push(tokio::spawn(async move {
            process
                .register(&registration(Uuid::new_v4(), ENGINE, &[]))
                .await
                .expect("register")
        }));
    }
    let (mut serving, mut full) = (0, 0);
    for registered in registering {
        match registered.await.expect("task") {
            Registered::Serving { .. } => serving += 1,
            Registered::Full { .. } => full += 1,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!((serving, full), (10, 14), "ten places were left");
    assert_eq!(r.rows().await, 1000);
}

// --- the registry fails ---------------------------------------------------------------------------------------------

/// The real registry that can be made unreachable.
#[derive(Clone)]
struct Flaky {
    inner: PgInstanceStore,
    down: Arc<AtomicBool>,
}

impl Flaky {
    fn check(&self) -> Result<(), StoreError> {
        if self.down.load(Ordering::SeqCst) {
            Err(StoreError::DatabaseUnavailable)
        } else {
            Ok(())
        }
    }
}

impl InstanceStore for Flaky {
    async fn register(&self, registration: &Registration<'_>) -> Result<Registered, StoreError> {
        self.check()?;
        self.inner.register(registration).await
    }
    async fn heartbeat(
        &self,
        instance: Uuid,
        token: &InstanceToken,
    ) -> Result<Heartbeat, StoreError> {
        self.check()?;
        self.inner.heartbeat(instance, token).await
    }
    async fn deregister(&self, instance: Uuid, token: &InstanceToken) -> Result<bool, StoreError> {
        self.check()?;
        self.inner.deregister(instance, token).await
    }
    async fn epoch(&self) -> Result<i64, StoreError> {
        self.check()?;
        self.inner.epoch().await
    }
    async fn requirements(&self) -> Result<Requirements, StoreError> {
        self.check()?;
        self.inner.requirements().await
    }
}

fn flaky(r: &Registry) -> Flaky {
    Flaky {
        inner: r.process.clone(),
        down: Arc::new(AtomicBool::new(false)),
    }
}

#[tokio::test]
async fn a_registry_that_cannot_be_asked_at_the_start_means_the_process_does_not_serve() {
    let r = registry().await;
    let store = flaky(&r);
    store.down.store(true, Ordering::SeqCst);
    let member = Member::new(
        store.clone(),
        Uuid::new_v4(),
        ENGINE,
        "0.3.0",
        &[SCHEDULES],
        None,
    );
    assert_eq!(
        member.register().await,
        Err(Refusal::Unavailable(StoreError::DatabaseUnavailable))
    );
    assert!(
        !member.gate().is_open(),
        "nothing serves before it registered"
    );
    assert_eq!(r.rows().await, 0);
    // when the registry is back it registers and serves
    store.down.store(false, Ordering::SeqCst);
    member.register().await.expect("registered");
    assert!(member.gate().is_open());
}

#[tokio::test]
async fn a_heartbeat_that_fails_while_serving_changes_nothing_and_the_next_one_that_succeeds_decides()
 {
    let r = registry().await;
    let store = flaky(&r);
    let member = Member::new(store.clone(), Uuid::new_v4(), ENGINE, "0.3.0", &[], None);
    member.register().await.expect("registered");
    assert!(member.gate().is_open());

    // the registry goes away for longer than the liveness window, and the operator records a requirement meanwhile
    store.down.store(true, Ordering::SeqCst);
    r.age(member.instance(), 60).await;
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE]).await.expect("activate"),
        Activated::Recorded { epoch: 1 },
        "the instance was silent for 60 s, so it is not live and does not block"
    );
    assert_eq!(member.beat().await, Err(StoreError::DatabaseUnavailable));
    assert_eq!(member.beat().await, Err(StoreError::DatabaseUnavailable));
    assert!(
        member.gate().is_open(),
        "the process keeps serving what it last read: that is the stated fail mode"
    );

    // the registry is back: the first beat that succeeds reads the epoch, and the gate closes
    store.down.store(false, Ordering::SeqCst);
    assert_eq!(member.beat().await, Ok(Beat::Refused));
    assert!(!member.gate().is_open());
    assert_eq!(member.gate().missing(), vec!["schedules-v1"]);
}

#[tokio::test]
async fn a_restarted_process_is_a_new_row_and_the_old_one_stops_counting_after_thirty_seconds() {
    let r = registry().await;
    let before = Member::new(
        r.process.clone(),
        Uuid::new_v4(),
        ENGINE,
        "0.3.0",
        &[],
        None,
    );
    before.register().await.expect("registered");
    // the process dies without leaving: its row stays, live for a while
    assert!(r.require(SCHEDULES, &[ENGINE]).await.is_err());
    let after = Member::new(
        r.process.clone(),
        Uuid::new_v4(),
        ENGINE,
        "0.3.1",
        &[SCHEDULES],
        None,
    );
    assert_ne!(
        before.instance(),
        after.instance(),
        "a restart is a new instance"
    );
    after.register().await.expect("registered");
    assert_eq!(r.rows().await, 2);
    assert!(
        r.require(SCHEDULES, &[ENGINE]).await.is_err(),
        "the dead process still counts until it has been silent for 30 s"
    );
    r.age(before.instance(), 31).await;
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE]).await.expect("activate"),
        Activated::Recorded { epoch: 1 }
    );
}

#[test]
fn the_registry_epoch_is_the_first_slot_of_the_lock_order_and_a_plan_that_returns_to_it_is_refused()
{
    use crate::support::lock_order::{Slot, assert_ascending};
    // the activation: the epoch, then whatever its step reads
    assert_ascending(&[
        (Slot::RegistryEpoch, "the requirement epoch, FOR UPDATE"),
        (Slot::Aggregate, "the handover row"),
    ]);
    let returned = std::panic::catch_unwind(|| {
        assert_ascending(&[
            (Slot::ExistingRuns, "a run"),
            (Slot::RegistryEpoch, "the requirement epoch, FOR SHARE"),
        ]);
    });
    assert!(
        returned.is_err(),
        "a transaction that locks a run and then takes the epoch could meet an activation that holds the epoch and waits"
    );
}
