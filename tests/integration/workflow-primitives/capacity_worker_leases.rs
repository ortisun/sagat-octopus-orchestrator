//! O706: real worker polling with an opt-in ChildRoot possession prerequisite.
use super::*;
use sagat_application::{
    capacity_vacate::{CapacityVacateCommand, CurrentChildRootVacateStore},
    clock::SystemClock,
    entropy::SystemEntropy,
    execution::{Engine, RunTx},
    runs::{Lock, RunStore},
    worker::{WorkerService, WorkerSettings},
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::{PgCapacityAcquireStore, PgCapacityVacateStore, PgRunStore};
use std::time::Duration;

#[path = "capacity_worker_preflight.rs"]
mod preflight;

async fn guarded_fixture() -> NativeFixture {
    guarded_fixture_with_continuation(false).await
}

async fn guarded_fixture_with_continuation(continue_first: bool) -> NativeFixture {
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

async fn acquire(n: &NativeFixture) {
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
        .expect("real native Acquire commits before poll");
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
            let (_, peer) = spawn_native_child(&n.f.w, false).await;
            let report = n.scheduler.tick().await;
            assert_eq!(report.children_started, 1, "{report:?}");
            assert_eq!(report.failed, 0);
            let member = Uuid::new_v4();
            // Owner join fixture only; possession itself moves via real native commands.
            let mut owner = n.f.w.db.pool.begin().await.expect("second member fixture");
            sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(member)
                .bind(n.f.w.namespace.id().as_uuid()).bind(peer.as_uuid())
                .execute(&mut *owner).await.expect("peer membership fixture");
            sqlx::query("UPDATE capacity_groups SET revision=revision+1,active_members=active_members+1 WHERE tenant_id=$1 AND group_id=$2")
                .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group)
                .execute(&mut *owner).await.expect("fixture join preserves old holder");
            owner.commit().await.expect("join fixture commits");
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
