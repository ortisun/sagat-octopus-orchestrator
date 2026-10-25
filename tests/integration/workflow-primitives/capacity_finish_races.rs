//! Remaining exact-member authority races. No production nesting or admission is enabled.
use super::*;
use crate::support::engine::{Session, activity, child_node, ctx, document};
use crate::support::eventually;
use sagat_application::chain_progress::{
    ChainHandoffCommand, ChainProgressStore, ContinuationSelection, EvaluatedChainHandoffCommand,
};
use sagat_application::{identity::IdentityStore, runs::StartRun, worker::Capabilities};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    identity::Subject,
    workflow::NodeId,
};
use sagat_domain::{
    definition::WorkflowId,
    identity::{Environment, Namespace, NamespaceId, Workspace, WorkspaceId},
};
use sagat_infra_postgres::PgChainProgressStore;
use sagat_infra_postgres::PgIdentityStore;
use serde_json::{Value, json};
use sqlx::Row;

async fn runtime_family(continuing: bool) -> NativeFixture {
    let mut n = native_fixture(continuing).await;
    n.f.w.execution_pool = n.f.w.db.runtime_pool_with(16).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&n.f.w.execution_pool)
        .await
        .expect("explicit native writer role");
    assert_eq!(role, "sagat_app");
    n
}

async fn same_tenant_namespace(w: &World) -> Namespace {
    let identity = PgIdentityStore::new(w.db.pool.clone());
    let workspace = Workspace::create(
        &w.admin,
        WorkspaceId::generate(),
        crate::support::slug("ws-capacity-alt"),
        crate::support::now(),
    )
    .expect("workspace");
    identity
        .create_workspace(
            &workspace,
            &AuditRecord::by(
                &w.admin,
                AuditAction::WorkspaceCreated,
                ResourceType::Workspace,
                workspace.id().to_string(),
                None,
                crate::support::now(),
            ),
        )
        .await
        .expect("persist workspace");
    let ns = Namespace::create(
        &w.admin,
        &workspace,
        NamespaceId::generate(),
        crate::support::slug("capacity-alt"),
        Environment::Dev,
        crate::support::now(),
    )
    .expect("namespace");
    identity
        .create_namespace(
            &ns,
            &AuditRecord::by(
                &w.admin,
                AuditAction::NamespaceCreated,
                ResourceType::Namespace,
                ns.id().to_string(),
                None,
                crate::support::now(),
            ),
        )
        .await
        .expect("persist namespace");
    ns
}

async fn publish_in(w: &World, ns: &Namespace, doc: Value) -> WorkflowId {
    let name = doc["metadata"]["name"].as_str().expect("workflow name");
    let bytes = serde_json::to_vec(&doc).expect("document");
    let created = w
        .defs
        .create_workflow(
            &w.admin,
            ns.slug().as_str(),
            name,
            &bytes,
            &format!("create-{}", Uuid::new_v4()),
            &ctx(),
        )
        .await
        .expect("create in exact namespace");
    let id = created.workflow.id();
    w.defs
        .publish(
            &w.admin,
            id,
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::new_v4()),
            &ctx(),
        )
        .await
        .expect("publish in exact namespace");
    id
}

async fn child_in(w: &World, ns: &Namespace) -> RunId {
    let name = format!("capacity-alt-child-{}", Uuid::new_v4());
    publish_in(w, ns, json!({
        "apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow",
        "metadata":{"name":name}, "entrypoint":"work",
        "nodes":[
            {"id":"work","type":"activity","activity":"capacity.alt.work","next":"again"},
            {"id":"again","type":"condition","expression":"input.continue","ifTrue":"rollover","ifFalse":"done"},
            {"id":"rollover","type":"continueAsNew","input":"input.next"},
            {"id":"done","type":"end"}
        ], "chainPolicy":{"maxSegments":3,"timeoutMs":3600000}
    })).await;
    let workflow = publish_in(
        w,
        ns,
        document(
            &format!("capacity-alt-parent-{}", Uuid::new_v4()),
            "kid",
            vec![child_node("kid", &name, Some("after")), activity("after")],
        ),
    )
    .await;
    let input =
        serde_json::to_vec(&json!({"continue":false,"next":{"continue":false}})).expect("input");
    let started = w
        .runs
        .start(
            &w.admin,
            &StartRun {
                namespace: ns.slug().as_str(),
                workflow,
                version: None,
                input: &input,
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::new_v4()),
            },
            &ctx(),
        )
        .await
        .expect("start in exact namespace");
    sagat_application::execution::child_run_id(
        started.info.run_id,
        &NodeId::parse("kid").expect("node"),
    )
}

async fn complete_in(w: &World, ns: &Namespace, child: RunId) {
    w.bindings
        .bind(
            &w.admin,
            ns.id(),
            w.worker.subject().as_str(),
            &["*".to_owned()],
            &ctx(),
        )
        .await
        .expect("namespace binding");
    let caps = Capabilities {
        activities: vec!["capacity.alt.work".into()],
        languages: vec!["rust".into()],
        pools: vec![],
        max_inflight: 1,
        protocol_version: "1".into(),
    };
    let registered = w
        .workers
        .register(
            &w.worker,
            ns.slug().as_str(),
            &format!("capacity-alt-{}", Uuid::new_v4()),
            &caps,
        )
        .await
        .expect("namespace worker");
    let session = Session {
        principal: w.worker.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    let mut tasks = w.poll(&session, 1).await;
    assert_eq!(tasks.len(), 1);
    let task = tasks.remove(0);
    assert_eq!(task.run_id, child);
    assert_eq!(task.activity.as_str(), "capacity.alt.work");
    w.complete(&session, &task, &json!({}))
        .await
        .expect("actual FINAL in second namespace");
}

async fn command_in(
    f: &Fixture,
    ns: NamespaceId,
    member: Uuid,
    chain: RunId,
) -> CapacityFinishCommand {
    let (chain_revision,run_revision):(i64,i64)=sqlx::query_as("SELECT c.revision,r.run_version FROM execution_chains c JOIN runs r ON r.tenant_id=c.tenant_id AND r.namespace_id=c.namespace_id AND r.id=$3 WHERE c.tenant_id=$1 AND c.namespace_id=$2 AND c.chain_id=$3")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(ns.as_uuid()).bind(chain.as_uuid()).fetch_one(&f.w.db.pool).await.expect("exact namespace revisions");
    let (group_revision, epoch): (i64, i64) = sqlx::query_as(
        "SELECT revision,epoch FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("current group counters");
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=run_shard($1)")
            .bind(chain.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("own fence");
    CapacityFinishCommand::new(
        Uuid::new_v4(),
        CapacityFinishIdentity::new(
            ChainScope::new(f.w.admin.tenant_id(), ns),
            CapacityGroupId::new(f.group),
            CapacityMemberId::new(member),
            chain,
            chain,
        ),
        CapacityFinishExpectations::new(
            u64::try_from(group_revision).expect("revision"),
            u64::try_from(epoch).expect("epoch"),
            u64::try_from(chain_revision).expect("chain revision"),
            u64::try_from(run_revision).expect("run revision"),
        ),
        SchedulerAuthority::for_run(owner, chain, u64::try_from(generation).expect("generation")),
    )
}

#[tokio::test]
async fn same_tenant_members_in_two_namespaces_keep_exact_binding_and_receipt_identity() {
    for holder_in_second_namespace in [false, true] {
        let n = runtime_family(false).await;
        let ns = same_tenant_namespace(&n.f.w).await;
        let second = child_in(&n.f.w, &ns).await;
        let report = n.scheduler.tick().await;
        assert_eq!(report.children_started, 1, "{report:?}");
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(complete_native_activity(&n).await, RunId::new(n.f.chain));
        complete_in(&n.f.w, &ns, second).await;
        let other = Uuid::new_v4();
        let holder = if holder_in_second_namespace {
            other
        } else {
            n.f.member
        };
        let mut tx =
            n.f.w
                .db
                .pool
                .begin()
                .await
                .expect("owner-only two namespace group fixture");
        sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')").bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(other).bind(ns.id().as_uuid()).bind(second.as_uuid()).execute(&mut *tx).await.expect("second physical child binding");
        sqlx::query("UPDATE capacity_groups SET revision=2,active_members=2,holder_member_id=$3 WHERE tenant_id=$1 AND group_id=$2").bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(holder).execute(&mut *tx).await.expect("two member header");
        tx.commit().await.expect("holder references active member");
        let before = execution_snapshot(&n.f).await;
        let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
        let mut wrong = command_in(&n.f, ns.id(), other, second).await;
        wrong.identity.scope = ChainScope::new(n.f.w.admin.tenant_id(), n.f.w.namespace.id());
        assert_eq!(
            store.finish_current_child_root(&wrong).await,
            Err(CapacityFinishError::BindingMismatch)
        );
        assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
        let first_command = native_command(&n, RunId::new(n.f.chain)).await;
        let first = store
            .finish_current_child_root(&first_command)
            .await
            .expect("first namespace finishes");
        let first_epoch = if holder_in_second_namespace { 1 } else { 2 };
        assert_eq!(
            (first.revision, first.epoch, first.active_members),
            (3, first_epoch, 1)
        );
        assert_eq!(
            first.possession,
            if holder_in_second_namespace {
                CapacityPossession::Held(CapacityMemberId::new(other))
            } else {
                CapacityPossession::Vacant
            }
        );
        let active:String=sqlx::query_scalar("SELECT state FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3").bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).bind(other).fetch_one(&n.f.w.db.pool).await.expect("other namespace unchanged");
        assert_eq!(active, "ACTIVE");
        assert_eq!(execution_snapshot(&n.f).await, before);
        let last = store
            .finish_current_child_root(&command_in(&n.f, ns.id(), other, second).await)
            .await
            .expect("second namespace retires group");
        assert_eq!(
            (
                last.revision,
                last.epoch,
                last.active_members,
                last.possession
            ),
            (4, first_epoch + 1, 0, CapacityPossession::Retired)
        );
        let replay = store
            .finish_current_child_root(&first_command)
            .await
            .expect("historical result after other namespace finishes");
        assert!(replay.replayed);
        let mut original = first.clone();
        original.replayed = true;
        assert_eq!(replay, original);
        assert_eq!(finish_counts(&n.f).await, (2, 2, 2));
        assert_eq!(execution_snapshot(&n.f).await, before);
        let rows=sqlx::query("SELECT r.member_id,r.namespace_id,r.chain_id,a.action,a.resource_type,a.resource_id,a.actor,o.event_type,o.payload FROM capacity_group_finish_receipts r JOIN audit_records a ON a.tenant_id=r.tenant_id AND a.id=r.audit_id JOIN event_outbox o ON o.tenant_id=r.tenant_id AND o.id=r.outbox_id WHERE r.tenant_id=$1 AND r.group_id=$2").bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).fetch_all(&n.f.w.db.pool).await.expect("receipt-linked metadata");
        assert_eq!(rows.len(), 2);
        for row in rows {
            let member: Uuid = row.get("member_id");
            let (expected_ns, expected_chain, result) = if member == n.f.member {
                (n.f.w.namespace.id(), RunId::new(n.f.chain), &first)
            } else {
                assert_eq!(member, other);
                (ns.id(), second, &last)
            };
            assert_eq!(row.get::<Uuid, _>("namespace_id"), expected_ns.as_uuid());
            assert_eq!(row.get::<Uuid, _>("chain_id"), expected_chain.as_uuid());
            assert_eq!(row.get::<String, _>("action"), "capacity.member.finished");
            assert_eq!(row.get::<String, _>("resource_type"), "capacity_group");
            assert_eq!(row.get::<String, _>("resource_id"), n.f.group.to_string());
            assert_eq!(row.get::<String, _>("actor"), Subject::engine().as_str());
            assert_eq!(
                row.get::<String, _>("event_type"),
                "capacity.member.finished"
            );
            let payload: Value = row.get("payload");
            for (field, value) in [
                ("tenantId", n.f.w.admin.tenant_id().to_string()),
                ("namespaceId", expected_ns.to_string()),
                ("groupId", n.f.group.to_string()),
                ("memberId", member.to_string()),
                ("chainId", expected_chain.to_string()),
                ("headRunId", expected_chain.to_string()),
            ] {
                assert_eq!(payload[field], value);
            }
            assert_eq!(payload["resultingRevision"], result.revision);
            assert_eq!(payload["epoch"], result.epoch);
            assert_eq!(
                payload["retired"],
                result.possession == CapacityPossession::Retired
            );
        }
        crate::hint_contract::assert_outbox_matches_the_contract(&n.f.w.db.pool).await;
    }
}

async fn evaluated_handoff(
    n: &NativeFixture,
    old: &CapacityFinishCommand,
    successor: RunId,
) -> EvaluatedChainHandoffCommand {
    let selection = ContinuationSelection::new(
        NodeId::parse("rollover").expect("boundary"),
        serde_json::to_vec(&serde_json::json!({"continue":false})).expect("evaluated input.next"),
        None,
        None,
    )
    .expect("selection");
    let trace: Option<String> =
        sqlx::query_scalar("SELECT trace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(n.f.w.admin.tenant_id().as_uuid())
            .bind(n.f.chain)
            .fetch_one(&n.f.w.db.pool)
            .await
            .expect("own persisted trace");
    let now = crate::support::now();
    let command = ChainHandoffCommand::new(
        old.identity.scope,
        old.identity.chain_id,
        old.identity.head_run_id,
        old.expected.chain_revision,
        old.expected.run_revision,
        successor,
        selection.transfer(),
        old.scheduler.clone(),
        Subject::engine(),
        trace.clone(),
        now,
        AuditRecord::engine(
            old.identity.scope.tenant_id(),
            AuditAction::RunContinued,
            ResourceType::Run,
            old.identity.head_run_id.to_string(),
            "continuation selected",
            trace,
            now,
        ),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    selection
        .into_command(command)
        .expect("evaluated handoff command")
}

#[tokio::test]
async fn committed_handoff_overtakes_discovered_old_owner_without_forwarding_finish() {
    let n = runtime_family(true).await;
    let initial = complete_native_activity(&n).await;
    let finish = native_command(&n, initial).await;
    let successor = RunId::generate();
    let handoff = evaluated_handoff(&n, &finish, successor).await;
    let mut gate =
        n.f.w
            .db
            .pool
            .begin()
            .await
            .expect("unchanged chain tuple gate");
    sqlx::query("SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).execute(&mut *gate).await.expect("chain gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate pid");
    let pool = n.f.w.execution_pool.clone();
    let cache = n.f.w.cache.clone();
    let advancing = tokio::spawn(async move {
        PgChainProgressStore::new(pool, cache)
            .continue_evaluated(&handoff)
            .await
    });
    let handoff_pid=eventually("actual handoff writer queues first on the gated chain",||async {
        sqlx::query_scalar::<_,i32>("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND wait_event_type='Lock' AND position('SELECT namespace_id, current_run_id, definition_version_id' in query)>0 AND $1=ANY(pg_blocking_pids(pid))")
            .bind(gate_pid).fetch_optional(&n.f.w.db.pool).await.expect("handoff blocker")
    }).await;
    let pool = n.f.w.execution_pool.clone();
    let old = finish.clone();
    let finishing = tokio::spawn(async move {
        PgCapacityFinishStore::new(pool)
            .finish_current_child_root(&old)
            .await
    });
    eventually("Finish has read the member binding and queues behind the actual handoff writer",||async {
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND wait_event_type='Lock' AND position('SELECT current_run_id,revision,final_outcome' in query)>0 AND $1=ANY(pg_blocking_pids(pid)))")
            .bind(handoff_pid).fetch_one(&n.f.w.db.pool).await.expect("observed queue dependency").then_some(())
    }).await;
    // Both real mutations are still blocked before own Run; this is not timing-based ordering.
    sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE NOWAIT",
    )
    .bind(n.f.w.admin.tenant_id().as_uuid())
    .bind(n.f.w.namespace.id().as_uuid())
    .bind(initial.as_uuid())
    .execute(&n.f.w.db.pool)
    .await
    .expect("own run free before chain guard");
    gate.rollback().await.expect("release queue");
    let applied = advancing
        .await
        .expect("handoff task")
        .expect("real evaluated child handoff commits");
    assert!(!applied.replayed());
    assert_eq!(
        finishing.await.expect("Finish task"),
        Err(CapacityFinishError::StaleHead { current: successor })
    );
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
    assert_eq!(
        state(&n.f.w.db.pool, &n.f).await,
        (1, 1, 1, "HELD".into(), "ACTIVE".into())
    );
    let predecessor:(String,Uuid)=sqlx::query_as("SELECT completion_kind,successor_run_id FROM chain_segments WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND run_id=$3")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).fetch_one(&n.f.w.db.pool).await.expect("actual continued predecessor");
    assert_eq!(predecessor, ("CONTINUED".into(), successor.as_uuid()));
    assert_eq!(complete_native_activity(&n).await, successor);
    let before = execution_snapshot(&n.f).await;
    let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
    assert_eq!(
        store.finish_current_child_root(&finish).await,
        Err(CapacityFinishError::StaleHead { current: successor })
    );
    store
        .finish_current_child_root(&native_command(&n, successor).await)
        .await
        .expect("only new current FINAL settles the logical member");
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    assert_eq!(execution_snapshot(&n.f).await, before);
}
