//! O706: real worker polling with an opt-in ChildRoot possession prerequisite.
use super::*;
use sagat_application::{
    capacity_vacate::{CapacityVacateCommand, CurrentChildRootVacateStore},
    clock::SystemClock,
    entropy::SystemEntropy,
    execution::{Engine, RunTx},
    runs::{Lock, RunStore},
    worker::{CapacityWindow, WorkerService, WorkerSettings},
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::{PgCapacityAcquireStore, PgCapacityVacateStore, PgRunStore};
use std::time::Duration;

#[path = "capacity_worker_preflight.rs"]
mod preflight;

async fn guarded_fixture() -> NativeFixture {
    guarded_fixture_with_continuation(false).await
}

pub(super) async fn guarded_fixture_with_continuation(continue_first: bool) -> NativeFixture {
    use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
    let mut w = world().await;
    w.execution_pool = w.db.runtime_pool_with(16).await;
    let store = PgRunStore::new(w.execution_pool.clone(), w.cache.clone());
    let placement = Placement::new(format!("capacity-worker-{}", Uuid::new_v4()));
    Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(600),
        },
    )
    .round()
    .await
    .expect("scheduler authority outlives 300s scenario bound");
    let scheduler = Scheduler::new(
        store,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(w.cache.clone())
    .with_placement(placement);
    let (parent, child) = spawn_native_child(&w, continue_first).await;
    let report = scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    assert_eq!(report.failed, 0);
    let f = seed_capacity(w, child).await;
    let mut n = NativeFixture {
        f,
        parent,
        scheduler,
    };
    configure_guarded_worker(&mut n);
    n
}

fn configure_guarded_worker(n: &mut NativeFixture) {
    n.f.w.workers = default_workers(n).with_child_root_capacity_guard();
}

fn default_workers(n: &NativeFixture) -> crate::support::engine::Workers {
    WorkerService::new(
        PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone()),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings {
            lease_ttl: Duration::from_secs(600),
            session_ttl: Duration::from_secs(600),
            ..WorkerSettings::default()
        },
    )
    .with_vault(n.f.w.keys.cipher())
}

async fn vacate(n: &NativeFixture) {
    let c = native_command(n, RunId::new(n.f.chain)).await;
    PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
        .vacate_current_child_root(&CapacityVacateCommand {
            operation_key: c.operation_key,
            identity: c.identity,
            expected: c.expected,
            scheduler: c.scheduler,
        })
        .await
        .expect("real native Vacate commits before poll");
}

async fn try_acquire(
    n: &NativeFixture,
) -> Result<(), sagat_application::capacity_acquire::CapacityAcquireError> {
    use sagat_application::capacity_acquire::{
        CapacityAcquireCommand, CurrentChildRootAcquireStore,
    };
    let c = native_command(n, RunId::new(n.f.chain)).await;
    PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
        .acquire_current_child_root(&CapacityAcquireCommand {
            operation_key: c.operation_key,
            identity: c.identity,
            expected: c.expected,
            scheduler: c.scheduler,
        })
        .await
        .map(|_| ())
}

async fn acquire(n: &NativeFixture) {
    try_acquire(n)
        .await
        .expect("real native Acquire commits before poll");
}

/// A second ACTIVE ChildRoot member of the same group on its own chain (owner join fixture;
/// possession itself moves only through the real native commands). Returns that chain and member.
async fn join_peer(n: &NativeFixture) -> (RunId, Uuid) {
    let (_, peer) = spawn_native_child(&n.f.w, false).await;
    let report = n.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    assert_eq!(report.failed, 0);
    let member = Uuid::new_v4();
    let mut owner = n.f.w.db.pool.begin().await.expect("second member fixture");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(member)
        .bind(n.f.w.namespace.id().as_uuid()).bind(peer.as_uuid())
        .execute(&mut *owner).await.expect("peer membership fixture");
    sqlx::query("UPDATE capacity_groups SET revision=revision+1,active_members=active_members+1 WHERE tenant_id=$1 AND group_id=$2")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group)
        .execute(&mut *owner).await.expect("fixture join preserves old holder");
    owner.commit().await.expect("join fixture commits");
    (peer, member)
}

/// The claim transaction of `run`, left open after its capacity prerequisite answered.
async fn open_claim(n: &NativeFixture, run: RunId) -> (bool, <PgRunStore as RunStore>::Tx) {
    let store = PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone());
    let mut tx = store
        .begin(n.f.w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("claim transaction begins")
        .expect("exact member run");
    let available = tx
        .worker_capacity_available()
        .await
        .expect("capacity prerequisite evaluates");
    (available, tx)
}

async fn group_lock(n: &NativeFixture) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut gate =
        n.f.w
            .db
            .pool
            .begin()
            .await
            .expect("external unchanged G barrier");
    sqlx::query(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.group)
    .execute(&mut *gate)
    .await
    .expect("G held independently");
    gate
}

async fn assert_no_claim(n: &NativeFixture) {
    let session = n.f.w.session(&["capacity.work"]).await;
    let before = execution_snapshot(&n.f).await;
    let tuples = super::vacate::untouched(&n.f).await;
    assert!(n.f.w.poll(&session, 1).await.is_empty());
    assert_eq!(execution_snapshot(&n.f).await, before);
    assert_eq!(super::vacate::untouched(&n.f).await, tuples);
}

#[tokio::test]
async fn vacant_child_stays_ready_without_consuming_an_attempt_or_quota() {
    crate::support::bounded::within(
        "worker poll after ChildRoot Vacate",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let session = n.f.w.session(&["capacity.work"]).await;
            vacate(&n).await;
            let before = execution_snapshot(&n.f).await;
            let tuples = super::vacate::untouched(&n.f).await;
            assert!(
                n.f.w.poll(&session, 1).await.is_empty(),
                "vacancy grants no external lease"
            );
            assert_eq!(execution_snapshot(&n.f).await, before);
            assert_eq!(super::vacate::untouched(&n.f).await, tuples);
            // Default-off is executable: the same still-READY work uses the legacy path.
            let tasks = default_workers(&n)
                .poll(
                    &session.principal,
                    session.worker_id,
                    &session.token,
                    1,
                    None,
                )
                .await
                .expect("default worker retains dormant compatibility");
            assert_eq!(
                tasks.len(),
                1,
                "constructor must not activate the incomplete protocol"
            );
        },
    )
    .await;
}

#[tokio::test]
async fn acquire_allows_a_lease_and_existing_report_ignores_a_busy_group() {
    crate::support::bounded::within(
        "held ChildRoot grant and report while G busy",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            vacate(&n).await;
            acquire(&n).await;
            let session = n.f.w.session(&["capacity.work"]).await;
            let tasks = n.f.w.poll(&session, 1).await;
            assert_eq!(tasks.len(), 1);
            let gate = group_lock(&n).await;
            let heartbeat = tokio::time::timeout(
                Duration::from_secs(5),
                n.f.w.workers.heartbeat(
                    &session.principal,
                    &crate::support::engine::task_ref(&tasks[0]),
                ),
            )
            .await
            .expect("heartbeat never waits on G")
            .expect("existing lease renews");
            assert!(heartbeat.lease_expires_at.is_some());
            let generation: i64 = sqlx::query_scalar(
                "SELECT generation FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='work'",
            )
            .bind(n.f.w.admin.tenant_id().as_uuid())
            .bind(tasks[0].run_id.as_uuid())
            .fetch_one(&n.f.w.db.pool)
            .await
            .expect("renewed lease fence");
            assert_eq!(
                u64::try_from(generation).expect("nonnegative generation"),
                tasks[0].fence.generation
            );
            tokio::time::timeout(
                Duration::from_secs(5),
                n.f.w.complete(&session, &tasks[0], &serde_json::json!({})),
            )
            .await
            .expect("report never waits on G")
            .expect("current lease reports normally");
            gate.rollback().await.expect("end report barrier");
            n.f.w.assert_consistent(tasks[0].run_id).await;
        },
    )
    .await;
}

#[tokio::test]
async fn busy_group_skips_without_a_lease_and_releases_own_chain_and_run() {
    crate::support::bounded::within(
        "busy G worker lease refusal",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let gate = group_lock(&n).await;
            tokio::time::timeout(Duration::from_secs(5), assert_no_claim(&n))
                .await.expect("worker does not wait on G");
            let mut probe = n.f.w.db.pool.begin().await.expect("independent lock probe");
            sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE NOWAIT")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain)
                .execute(&mut *probe).await.expect("claim committed and released own chain");
            sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain)
                .execute(&mut *probe).await.expect("claim committed and released own run");
            probe.rollback().await.expect("probe ends");
            gate.rollback().await.expect("release G");
            let session = n.f.w.session(&["capacity.work"]).await;
            assert_eq!(n.f.w.poll(&session, 1).await.len(), 1);
        },
    ).await;
}

#[tokio::test]
async fn duplicate_active_groups_do_not_choose_an_arbitrary_holder() {
    crate::support::bounded::within(
        "ambiguous ChildRoot membership",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            // Owner fixture only: existing schema allows the same chain in distinct groups.
            // This is not a production creator or proof of nested admission.
            let group = Uuid::new_v4();
            let member = Uuid::new_v4();
            let mut tx = n.f.w.db.pool.begin().await.expect("ambiguous owner fixture");
            sqlx::query("INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id) VALUES($1,$2,1,1,1,'HELD',$3)")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(group).bind(member)
                .execute(&mut *tx).await.expect("second group");
            sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(group).bind(member)
                .bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain)
                .execute(&mut *tx).await.expect("second active membership");
            tx.commit().await.expect("deferred FK valid");
            assert_no_claim(&n).await;
        },
    ).await;
}

#[tokio::test]
async fn an_active_nonholder_stays_ready_while_the_other_member_is_leased() {
    use sagat_application::capacity_acquire::{
        CapacityAcquireCommand, CurrentChildRootAcquireStore,
    };
    crate::support::bounded::within(
        "unambiguous nonholder worker refusal",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let (peer, member) = join_peer(&n).await;
            vacate(&n).await;
            let c = native_member_command(&n.f, member, peer, peer).await;
            PgCapacityAcquireStore::new(n.f.w.execution_pool.clone())
                .acquire_current_child_root(&CapacityAcquireCommand {
                    operation_key:c.operation_key, identity:c.identity, expected:c.expected, scheduler:c.scheduler,
                }).await.expect("peer genuinely acquires G");
            let session = n.f.w.session(&["capacity.work"]).await;
            let tasks = n.f.w.poll(&session, 2).await;
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].run_id, peer, "SQL must bind the selected member, not substitute G's holder");
            let old: (String, i32, Option<Vec<u8>>) = sqlx::query_as("SELECT status,attempt,lease_token_hash FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='work'")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.chain)
                .fetch_one(&n.f.w.db.pool).await.expect("nonholder step remains untouched");
            assert_eq!(old, ("READY".into(), 0, None));
            let used: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND status IN ('LEASED','RUNNING')")
                .bind(n.f.w.admin.tenant_id().as_uuid()).fetch_one(&n.f.w.db.pool).await.expect("actual activity occupancy");
            assert_eq!(used, 1, "only the holder spends activity concurrency");
        },
    ).await;
}

/// ADR-0075 activation condition (j): a claim that is not the holder is decided from an unlocked
/// header read, so it never takes a row lock on G. Otherwise, while the group is vacant, every
/// member poller would hold G until its commit and starve the NOWAIT native Acquire.
#[tokio::test]
async fn a_peer_poll_on_a_vacant_group_does_not_starve_native_acquire() {
    crate::support::bounded::within(
        "vacant group: member pollers versus Acquire",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let (peer, _) = join_peer(&n).await;
            vacate(&n).await;
            // The peer's claim transaction stays open, exactly as it does while a worker
            // finishes the other candidates of the same poll.
            let (available, claim) = open_claim(&n, peer).await;
            assert!(!available, "a vacant group grants no member a lease");
            let acquired = try_acquire(&n).await;
            claim.commit().await.expect("peer claim ends");
            assert!(
                acquired.is_ok(),
                "a peer's refused claim must not hold G against Acquire: {acquired:?}"
            );
        },
    )
    .await;
}

#[tokio::test]
async fn a_nonholder_refusal_takes_no_row_lock_on_g() {
    crate::support::bounded::within(
        "held group: nonholder claims never lock G",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            // The fixture member holds G; the peer is an ACTIVE non-holder.
            let (peer, _) = join_peer(&n).await;
            let (available, claim) = open_claim(&n, peer).await;
            assert!(!available, "a non-holder is refused");
            let probe = sqlx::query(
                "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 \
                 FOR UPDATE NOWAIT",
            )
            .bind(n.f.w.admin.tenant_id().as_uuid())
            .bind(n.f.group)
            .execute(&n.f.w.db.pool)
            .await;
            claim.commit().await.expect("peer claim ends");
            assert!(
                probe.is_ok(),
                "the refused non-holder claim still holds a row lock on G: {probe:?}"
            );
        },
    )
    .await;
}

/// The runs of one part of the bounded candidate window a poll would receive (root paths only).
async fn window(n: &NativeFixture, part: CapacityWindow, limit: u32) -> Vec<RunId> {
    use sagat_application::worker::WorkerStore;
    let store = PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone());
    store
        .claimable_paths(
            n.f.w.admin.tenant_id(),
            n.f.w.namespace.id(),
            &["capacity.work".to_owned(), "act.a".to_owned()],
            limit,
            100,
            "capacity-window",
            part,
        )
        .await
        .expect("candidate window")
        .into_iter()
        .map(|(run, _)| run)
        .collect()
}

/// Owner fixture: the discovery order is `ready_at`, so ageing a step is how a test puts a
/// non-holder ahead of the holder without sleeping.
async fn make_older(n: &NativeFixture, run: RunId) {
    sqlx::query(
        "UPDATE steps SET ready_at = ready_at - interval '1 hour' \
         WHERE tenant_id=$1 AND run_id=$2 AND status='READY'",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .execute(&n.f.w.db.pool)
    .await
    .expect("owner fixture ages the ready step");
}

/// ADR-0075 activation condition (d) / G715: the candidate window is only a hint. `Any` is the
/// legacy window (the dormant compatibility promise); `Eligible` and `Ineligible` partition it by
/// whether the possession prerequisite certainly refuses the run (an ACTIVE ChildRoot member that
/// is not the holder), and the prerequisite itself stays the authority.
#[tokio::test]
async fn the_capacity_window_partitions_the_legacy_window_and_never_hides_an_available_run() {
    crate::support::bounded::within(
        "capacity-aware candidate window",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let holder = RunId::new(n.f.chain);
            let (peer, _) = join_peer(&n).await;
            let (workflow, _) = n.f.w.publish(chain("capacity-window-nonmember")).await;
            let nonmember = n.f.w.start(workflow, &serde_json::json!({})).await;
            make_older(&n, peer).await;
            // Discovery order is ready_at: the non-holder peer, the holder, the non-member.
            assert_eq!(
                window(&n, CapacityWindow::Any, 4).await,
                vec![peer, holder, nonmember]
            );
            assert_eq!(
                window(&n, CapacityWindow::Eligible, 4).await,
                vec![holder, nonmember]
            );
            assert_eq!(window(&n, CapacityWindow::Ineligible, 4).await, vec![peer]);
            assert_eq!(
                window(&n, CapacityWindow::Eligible, 1).await,
                vec![holder],
                "an older non-holder must not take the only place of the eligible window"
            );
            // Never narrower than the prerequisite, in a held and in a vacant group.
            for vacant in [false, true] {
                if vacant {
                    vacate(&n).await;
                }
                let eligible = window(&n, CapacityWindow::Eligible, 4).await;
                let ineligible = window(&n, CapacityWindow::Ineligible, 4).await;
                let mut both = [eligible.clone(), ineligible.clone()].concat();
                both.sort();
                let mut any = window(&n, CapacityWindow::Any, 4).await;
                any.sort();
                assert_eq!(both, any, "the two parts partition the window (vacant={vacant})");
                for run in [holder, peer, nonmember] {
                    let (available, claim) = open_claim(&n, run).await;
                    claim.commit().await.expect("read-only claim ends");
                    assert!(
                        !available || eligible.contains(&run),
                        "run {run} passes the prerequisite but left the eligible window (vacant={vacant})"
                    );
                }
                if vacant {
                    assert_eq!(eligible, vec![nonmember], "a vacant group offers no member");
                    assert_eq!(ineligible.len(), 2, "both members are certainly refused");
                }
            }
        },
    )
    .await;
}

/// The window is `4 * wanted` candidates: with `wanted = 1`, four older non-holders hide the
/// holder for ever unless the worker passes its guard flag to candidate selection.
#[tokio::test]
async fn a_guarded_poll_is_not_crowded_out_by_older_nonholders() {
    crate::support::bounded::within(
        "poll window versus non-holders",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            for _ in 0..4 {
                let (peer, _) = join_peer(&n).await;
                make_older(&n, peer).await;
            }
            let session = n.f.w.session(&["capacity.work"]).await;
            let tasks = n.f.w.poll(&session, 1).await;
            assert_eq!(
                tasks.len(),
                1,
                "the holder's ready step is offered to the poll"
            );
            assert_eq!(tasks[0].run_id, RunId::new(n.f.chain));
        },
    )
    .await;
}

#[tokio::test]
async fn finished_history_is_not_legacy_and_one_active_can_follow_retained_history() {
    crate::support::bounded::within(
        "membership history classification",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let head = complete_native_activity(&n).await;
            let c = native_command(&n, head).await;
            PgCapacityFinishStore::new(n.f.w.execution_pool.clone())
                .finish_current_child_root(&c).await.expect("real final member Finish");
            let store = PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone());
            let mut tx = store.begin(n.f.w.admin.tenant_id(), head, Lock::Wait)
                .await.expect("root begin").expect("exact finished run");
            assert!(!tx.worker_capacity_available().await.expect("history is not legacy"));
            tx.commit().await.expect("read guard ends");
            // The storage classifier alone is under test: this owner-seeded binding is not
            // executable admission of a terminal run. Normal worker state checks still refuse it.
            let group = Uuid::new_v4();
            let member = Uuid::new_v4();
            let mut owner = n.f.w.db.pool.begin().await.expect("new metadata fixture");
            sqlx::query("INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id) VALUES($1,$2,1,1,1,'HELD',$3)")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(group).bind(member)
                .execute(&mut *owner).await.expect("new group metadata");
            sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(group).bind(member)
                .bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain)
                .execute(&mut *owner).await.expect("one active plus retained history");
            owner.commit().await.expect("metadata fixture commits");
            let mut tx = store.begin(n.f.w.admin.tenant_id(), head, Lock::Wait)
                .await.expect("root begin").expect("existing run");
            assert!(tx.worker_capacity_available().await.expect("evaluate sole active membership"));
            tx.commit().await.expect("guard commit");
        },
    ).await;
}

#[tokio::test]
async fn a_lease_retains_group_possession_while_waiting_for_activity_admission() {
    crate::support::bounded::within(
        "worker group lock survives until lease commit",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let session = n.f.w.session(&["capacity.work"]).await;
            let mut gate = n.f.w.db.pool.begin().await.expect("activity quota barrier");
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(format!("activity-quota:{}:{}:capacity.work", n.f.w.admin.tenant_id(), n.f.w.namespace.id()))
                .execute(&mut *gate).await.expect("hold exact activity admission lock");
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *gate).await.expect("barrier backend");
            let workers = n.f.w.workers.clone();
            let poll = tokio::spawn(async move {
                workers.poll(&session.principal, session.worker_id, &session.token, 1, None).await
            });
            crate::support::eventually("actual worker waits at activity admission after G", || async {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND $1=ANY(pg_blocking_pids(pid)) AND position('pg_advisory_xact_lock' in query)>0)")
                    .bind(pid).fetch_one(&n.f.w.db.pool).await.expect("observe held-lock barrier");
                waiting.then_some(())
            }).await;
            let error = sqlx::query("SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group)
                .execute(&n.f.w.db.pool).await.expect_err("worker retains G before lease commit");
            super::error_is(error, "55P03", None);
            gate.rollback().await.expect("allow activity admission");
            let tasks = poll.await.expect("poll task joins").expect("lease transaction commits");
            assert_eq!(tasks.len(), 1);
            sqlx::query("SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group)
                .execute(&n.f.w.db.pool).await.expect("G released after actual lease commit");
            let c = native_command(&n, tasks[0].run_id).await;
            let refused = PgCapacityVacateStore::new(n.f.w.execution_pool.clone())
                .vacate_current_child_root(&CapacityVacateCommand {
                    operation_key:c.operation_key, identity:c.identity, expected:c.expected, scheduler:c.scheduler,
                }).await;
            assert!(matches!(refused, Err(sagat_application::capacity_vacate::CapacityVacateError::NotQuiescent)), "{refused:?}");
        },
    ).await;
}

#[tokio::test]
async fn nonmember_work_is_compatible_even_while_an_unrelated_group_is_busy() {
    crate::support::bounded::within(
        "legacy worker compatibility",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture().await;
            let (workflow, _) = n.f.w.publish(chain("capacity-legacy-compatible")).await;
            let run = n.f.w.start(workflow, &serde_json::json!({})).await;
            let session = n.f.w.session(&["act.a"]).await;
            let gate = group_lock(&n).await;
            let tasks = n.f.w.poll(&session, 1).await;
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].run_id, run);
            gate.rollback().await.expect("release unrelated G");
        },
    )
    .await;
}

#[tokio::test]
async fn continued_member_uses_the_current_head_and_refuses_its_old_run() {
    crate::support::bounded::within(
        "worker membership follows actual continuation",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let n = guarded_fixture_with_continuation(true).await;
            let initial = complete_native_activity(&n).await;
            let report = n.scheduler.tick().await;
            assert_eq!(report.continuations_advanced, 1, "{report:?}");
            assert_eq!(report.failed, 0);
            let store = PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone());
            let mut tx = store
                .begin(n.f.w.admin.tenant_id(), initial, Lock::Wait)
                .await
                .expect("historical begin")
                .expect("retained old run");
            assert!(
                !tx.worker_capacity_available()
                    .await
                    .expect("old run is not current authority")
            );
            tx.commit().await.expect("historical probe ends");
            let session = n.f.w.session(&["capacity.work"]).await;
            let tasks = n.f.w.poll(&session, 1).await;
            assert_eq!(tasks.len(), 1);
            assert_ne!(tasks[0].run_id, initial);
        },
    )
    .await;
}

#[tokio::test]
async fn scoped_claims_use_the_same_group_guard_before_attempt_or_quota() {
    crate::support::bounded::within(
        "exact-path worker capacity prerequisite",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            let f = super::super::scoped_worker::fixture("capacity-scoped-guard").await;
            let (group, member) = seed_scoped_binding(&f).await;
            let tenant = f.world.admin.tenant_id().as_uuid();
            let run = f.run.as_uuid();
            let before: Vec<(String, i32, Option<Vec<u8>>)> = sqlx::query_as("SELECT status,attempt,lease_token_hash FROM steps WHERE tenant_id=$1 AND run_id=$2 ORDER BY instance_path_nodes,instance_path_indices,node")
                .bind(tenant).bind(run).fetch_all(&f.world.db.pool).await.expect("exact step baseline");
            assert!(before.iter().any(|(status, attempt, token)| status == "READY" && *attempt == 0 && token.is_none()));
            let tasks = f.workers.clone().with_child_root_capacity_guard().poll(
                &f.session.principal, f.session.worker_id, &f.session.token, 2, None,
            ).await.expect("scoped poll");
            assert!(tasks.is_empty(), "vacant group blocks exact-path external leases");
            let after: Vec<(String, i32, Option<Vec<u8>>)> = sqlx::query_as("SELECT status,attempt,lease_token_hash FROM steps WHERE tenant_id=$1 AND run_id=$2 ORDER BY instance_path_nodes,instance_path_indices,node")
                .bind(tenant).bind(run).fetch_all(&f.world.db.pool).await.expect("steps after scoped refusal");
            assert_eq!(after, before);
            // Owner metadata fixture only: do not claim native Acquire/public nested support.
            sqlx::query("UPDATE capacity_groups SET possession='HELD',holder_member_id=$3,revision=2,epoch=2 WHERE tenant_id=$1 AND group_id=$2")
                .bind(tenant).bind(group).bind(member).execute(&f.world.db.pool).await.expect("owner fixture selects active holder");
            let tasks = f.workers.clone().with_child_root_capacity_guard().poll(
                &f.session.principal, f.session.worker_id, &f.session.token, 1, None,
            ).await.expect("held scoped poll");
            assert_eq!(tasks.len(), 1, "guarded exact-path adapter can grant an eligible holder");
            assert!(!tasks[0].instance.path().is_root());
        },
    ).await;
}

async fn seed_scoped_binding(f: &super::super::scoped_worker::Fixture) -> (Uuid, Uuid) {
    // Internal structured fixture and owner metadata only: tests adapter wiring,
    // not public ChildRoot+collection admission or a production creator.
    let group = Uuid::new_v4();
    let member = Uuid::new_v4();
    let tenant = f.world.admin.tenant_id().as_uuid();
    let namespace = f.world.namespace.id().as_uuid();
    let run = f.run.as_uuid();
    let mut owner = f
        .world
        .db
        .pool
        .begin()
        .await
        .expect("scoped metadata fixture");
    sqlx::query("INSERT INTO execution_chains(tenant_id,namespace_id,chain_id,current_run_id,definition_version_id,started_by,max_segments,original_deadline_at,created_at,updated_at) SELECT tenant_id,namespace_id,id,id,workflow_version_id,started_by,1,clock_timestamp()+interval '1 hour',clock_timestamp(),clock_timestamp() FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3")
    .bind(tenant).bind(namespace).bind(run).execute(&mut *owner).await.expect("owner chain identity fixture");
    sqlx::query("INSERT INTO chain_segments(tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) VALUES($1,$2,$3,0,$3,clock_timestamp())")
    .bind(tenant).bind(namespace).bind(run).execute(&mut *owner).await.expect("owner segment fixture");
    sqlx::query("INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession) VALUES($1,$2,1,1,1,'VACANT')")
    .bind(tenant).bind(group).execute(&mut *owner).await.expect("vacant owner group fixture");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
    .bind(tenant).bind(group).bind(member).bind(namespace).bind(run)
    .execute(&mut *owner).await.expect("chain capacity binding fixture");
    owner.commit().await.expect("owner fixture commits");
    (group, member)
}
