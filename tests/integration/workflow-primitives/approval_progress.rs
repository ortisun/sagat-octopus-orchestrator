//! Real PostgreSQL evidence for canonical workflow approval progress.

use std::time::Duration;

use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalExpiryAttempt, ApprovalOpening,
    ApprovalProgressError, ApprovalProgressStore, ApprovalRoutes,
};
use sagat_application::execution::instance_step_id;
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalError, ApprovalPolicy, ApprovalReceiptId,
    ApprovalRequest, ApprovalRequestId, ApprovalStatus, DecisionKey, ExecutionInstanceId, RunId,
};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::workflow::{Digest, NodeId};
use sagat_infra_postgres::PgApprovalProgressStore;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::support::engine::{chain, world};

struct Fixture {
    world: crate::support::engine::World,
    store: PgApprovalProgressStore,
    actor: Principal,
    run: RunId,
    request: ApprovalRequest,
    routes: ApprovalRoutes,
    scheduler: SchedulerAuthority,
    opened_at: OffsetDateTime,
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

fn approval_document(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "approval",
        "nodes": [
            {
                "id": "approval",
                "type": "approval",
                "timeoutMs": 60000,
                "policy": { "roles": ["operator"], "excludeInitiator": true },
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            { "id": "approved", "type": "activity", "activity": "approval.approved" },
            { "id": "rejected", "type": "activity", "activity": "approval.rejected" },
            { "id": "expired", "type": "activity", "activity": "approval.expired" }
        ]
    })
}

async fn approval_run(
    world: &crate::support::engine::World,
    name: &str,
) -> (RunId, WorkflowVersionId) {
    let (workflow, _) = world.publish(chain(name)).await;
    let document = serde_json::to_vec(&approval_document(name)).expect("approval document");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    let now = crate::support::now();
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,$7)",
    )
    .bind(version.as_uuid())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(world.admin.subject().as_str())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("approval workflow version");
    let input = b"{}";
    let input_digest = digest(input);
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(input.as_slice())
    .execute(&world.db.pool)
    .await
    .expect("approval run input");
    let run = RunId::generate();
    sqlx::query(
        "INSERT INTO runs \
         (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
          input_digest, started_by, created_at, updated_at, root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',1,$6,$7,$8,$8,$1)",
    )
    .bind(run.as_uuid())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(world.admin.subject().as_str())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("approval run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1,$2,1,'run.started',$3,'system:scheduler',$4)",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(json!({"entrypoint": "approval"}))
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("run started journal");
    let owner = ExecutionInstanceId::root(NodeId::parse("approval").expect("approval node"));
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, kind, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1,$2,$3,$4,$5,'PENDING',$6,'FORWARD','{}','{}')",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(owner.node().as_str())
    .bind(instance_step_id(version, run, &owner))
    .bind(world.namespace.id().as_uuid())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("pending approval owner");
    (run, version)
}

async fn scheduler(pool: &sqlx::PgPool, run: RunId) -> SchedulerAuthority {
    let owner = "approval-progress-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET generation = generation + 1, owner=$2, \
                expires_at=clock_timestamp() + interval '1 hour', acquired_at=clock_timestamp() \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("scheduler fence");
    SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("nonnegative generation"),
    )
}

fn engine_audit(fixture: &Fixture, action: AuditAction, at: OffsetDateTime) -> AuditRecord {
    AuditRecord::engine(
        fixture.world.admin.tenant_id(),
        action,
        ResourceType::ApprovalRequest,
        fixture.request.id().to_string(),
        action.as_str(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        at,
    )
}

fn actor_audit(fixture: &Fixture, actor: &Principal, at: OffsetDateTime) -> AuditRecord {
    AuditRecord::by(
        actor,
        AuditAction::ApprovalDecided,
        ResourceType::ApprovalRequest,
        fixture.request.id().to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        at,
    )
}

fn address(fixture: &Fixture) -> ApprovalAddress {
    ApprovalAddress::new(
        fixture.world.admin.tenant_id(),
        fixture.world.namespace.id(),
        fixture.request.id(),
    )
}

fn decision(
    fixture: &Fixture,
    actor: &Principal,
    key: &str,
    at: OffsetDateTime,
    outbox: Uuid,
) -> ApprovalDecisionAttempt {
    ApprovalDecisionAttempt::new(
        address(fixture),
        actor.subject().clone(),
        ApprovalReceiptId::generate(),
        ApprovalDecisionCommand::new(
            DecisionKey::parse(key).expect("decision key"),
            Digest::new([71; 32]),
            ApprovalDecision::Approve,
            None,
        ),
        actor_audit(fixture, actor, at),
        outbox,
        at,
    )
}

fn expiry(fixture: &Fixture, at: OffsetDateTime, outbox: Uuid) -> ApprovalExpiryAttempt {
    ApprovalExpiryAttempt::new(
        address(fixture),
        fixture.scheduler.clone(),
        engine_audit(fixture, AuditAction::ApprovalExpired, at),
        outbox,
        at,
    )
}

async fn pending_fixture(name: &str) -> Fixture {
    let world = world().await;
    let actor = world.member("approval-operator", Role::Operator).await;
    let (run, _) = approval_run(&world, name).await;
    let scheduler = scheduler(&world.db.pool, run).await;
    let opened_at = crate::support::now();
    let request = ApprovalRequest::open(
        ApprovalRequestId::generate(),
        world.admin.tenant_id(),
        world.namespace.id(),
        run,
        ExecutionInstanceId::root(NodeId::parse("approval").expect("approval node")),
        ApprovalPolicy::new(Vec::new(), vec![Role::Operator]).expect("approval policy"),
        world.admin.subject().clone(),
        2,
        opened_at + Duration::from_secs(60),
    );
    let routes = ApprovalRoutes::new(
        NodeId::parse("approved").expect("approved route"),
        NodeId::parse("rejected").expect("rejected route"),
        NodeId::parse("expired").expect("expired route"),
    );
    let store = PgApprovalProgressStore::new(world.db.pool.clone());
    Fixture {
        world,
        store,
        actor,
        run,
        request,
        routes,
        scheduler,
        opened_at,
    }
}

async fn fixture(name: &str) -> Fixture {
    let result = pending_fixture(name).await;
    result
        .store
        .open(&ApprovalOpening::new(
            result.request.clone(),
            result.routes.clone(),
            result.scheduler.clone(),
            engine_audit(&result, AuditAction::ApprovalOpened, result.opened_at),
            Uuid::now_v7(),
            result.opened_at,
        ))
        .await
        .expect("open approval");
    result
}

async fn reserve_outbox(fixture: &Fixture, id: Uuid) {
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES ($1,$2,$3,'fixture.reserved','{}')",
    )
    .bind(id)
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("reserve outbox id");
}

async fn assert_pending_after_failed_settlement(fixture: &Fixture) {
    assert_eq!(
        fixture
            .store
            .load(address(fixture))
            .await
            .expect("pending approval remains canonical")
            .status(),
        ApprovalStatus::Pending
    );
    let evidence: (i64, i64, String, String, bool, i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3 \
             AND event_type IN ('approval.decided','approval.expired')), \
          (SELECT state FROM approval_timers WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$3 AND node='approval'), \
          (SELECT routed FROM steps WHERE tenant_id=$1 AND run_id=$3 AND node='approval'), \
          (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$3 \
             AND node IN ('approved','expired')), \
          (SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$3)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("settlement rollback evidence");
    assert_eq!(
        evidence,
        (0, 0, "PENDING".into(), "WAITING".into(), false, 0, 2)
    );
}

#[tokio::test]
async fn stale_scheduler_fence_cannot_open_or_expire_an_approval() {
    let fixture = pending_fixture("approval-stale-fence").await;
    let stale = SchedulerAuthority::for_run(
        fixture.scheduler.owner(),
        fixture.run,
        fixture.scheduler.lease().generation + 1,
    );
    let opening = ApprovalOpening::new(
        fixture.request.clone(),
        fixture.routes.clone(),
        stale.clone(),
        engine_audit(&fixture, AuditAction::ApprovalOpened, fixture.opened_at),
        Uuid::now_v7(),
        fixture.opened_at,
    );
    assert!(matches!(
        fixture.store.open(&opening).await,
        Err(ApprovalProgressError::SchedulerFenceLost)
    ));
    let opened: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests WHERE tenant_id=$1 AND request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("no stale opening");
    assert_eq!(opened, 0);

    fixture
        .store
        .open(&ApprovalOpening::new(
            fixture.request.clone(),
            fixture.routes.clone(),
            fixture.scheduler.clone(),
            engine_audit(&fixture, AuditAction::ApprovalOpened, fixture.opened_at),
            Uuid::now_v7(),
            fixture.opened_at,
        ))
        .await
        .expect("open with current fence");
    let expiration = ApprovalExpiryAttempt::new(
        address(&fixture),
        stale,
        engine_audit(
            &fixture,
            AuditAction::ApprovalExpired,
            fixture.request.deadline() + Duration::from_secs(1),
        ),
        Uuid::now_v7(),
        fixture.request.deadline() + Duration::from_secs(1),
    );
    assert!(matches!(
        fixture.store.expire(&expiration).await,
        Err(ApprovalProgressError::SchedulerFenceLost)
    ));
    assert_eq!(
        fixture
            .store
            .load(address(&fixture))
            .await
            .expect("approval remains pending")
            .status(),
        ApprovalStatus::Pending
    );
}

#[tokio::test]
async fn settlement_outbox_failures_roll_back_decision_and_expiry() {
    let decision_fixture = fixture("approval-decision-rollback").await;
    let decision_outbox = Uuid::now_v7();
    reserve_outbox(&decision_fixture, decision_outbox).await;
    let failed_decision = decision(
        &decision_fixture,
        &decision_fixture.actor,
        "rollback-decision",
        decision_fixture.opened_at + Duration::from_secs(5),
        decision_outbox,
    );
    assert!(matches!(
        decision_fixture.store.decide(&failed_decision).await,
        Err(ApprovalProgressError::Storage(_))
    ));
    assert_pending_after_failed_settlement(&decision_fixture).await;

    let expiry_fixture = fixture("approval-expiry-rollback").await;
    let expiry_outbox = Uuid::now_v7();
    reserve_outbox(&expiry_fixture, expiry_outbox).await;
    let failed_expiry = expiry(
        &expiry_fixture,
        expiry_fixture.request.deadline() + Duration::from_secs(1),
        expiry_outbox,
    );
    assert!(matches!(
        expiry_fixture.store.expire(&failed_expiry).await,
        Err(ApprovalProgressError::Storage(_))
    ));
    assert_pending_after_failed_settlement(&expiry_fixture).await;
}

#[tokio::test]
async fn restart_and_identical_replay_return_the_original_authorized_receipt() {
    let fixture = fixture("approval-replay").await;
    let restarted = PgApprovalProgressStore::new(fixture.world.db.pool.clone());
    let loaded = restarted
        .load(address(&fixture))
        .await
        .expect("load request");
    assert_eq!(loaded.status(), ApprovalStatus::Pending);
    assert_eq!(loaded.policy(), fixture.request.policy());

    let first_at = fixture.opened_at + Duration::from_secs(10);
    let first = decision(
        &fixture,
        &fixture.actor,
        "same-decision",
        first_at,
        Uuid::now_v7(),
    );
    let committed = restarted.decide(&first).await.expect("commit decision");
    assert!(!committed.replayed());

    let before: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3), \
          (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$3)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("counts before replay");
    let replay = decision(
        &fixture,
        &fixture.actor,
        "same-decision",
        first_at + Duration::from_secs(5),
        Uuid::now_v7(),
    );
    let replayed = PgApprovalProgressStore::new(fixture.world.db.pool.clone())
        .decide(&replay)
        .await
        .expect("replay decision");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt().id(), committed.receipt().id());
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3), \
          (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$3)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("counts after replay");
    assert_eq!(after, before);
    let restored = restarted
        .load(address(&fixture))
        .await
        .expect("load decided");
    assert_eq!(restored.status(), ApprovalStatus::Approved);
    assert_eq!(
        restored.winning_receipt_id(),
        Some(committed.receipt().id())
    );
}

#[tokio::test]
async fn decision_and_expiry_race_has_one_first_commit_winner() {
    let fixture = fixture("approval-race").await;
    let due = fixture.request.deadline() + Duration::from_secs(1);
    let decision = decision(
        &fixture,
        &fixture.actor,
        "racing-decision",
        due,
        Uuid::now_v7(),
    );
    let expiry = expiry(&fixture, due, Uuid::now_v7());
    let decision_store = fixture.store.clone();
    let expiry_store = fixture.store.clone();
    let (decided, expired) = tokio::join!(
        decision_store.decide(&decision),
        expiry_store.expire(&expiry)
    );
    let restored = fixture.store.load(address(&fixture)).await.expect("winner");
    match restored.status() {
        ApprovalStatus::Approved => {
            assert!(decided.is_ok());
            assert!(!expired.expect("losing expiry is a no-op").changed());
        }
        ApprovalStatus::Expired => {
            assert!(matches!(
                decided,
                Err(ApprovalProgressError::Approval(
                    ApprovalError::AlreadyDecided
                ))
            ));
            assert!(expired.expect("winning expiry").changed());
        }
        status => panic!("unexpected race outcome {status:?}"),
    }
    let outcomes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
         AND event_type IN ('approval.decided','approval.expired')",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("one outcome journal");
    assert_eq!(outcomes, 1);
}

#[tokio::test]
async fn expiry_then_decision_and_decision_then_expiry_keep_the_first_outcome() {
    let expiry_first = fixture("approval-expiry-first").await;
    let due = expiry_first.request.deadline() + Duration::from_secs(1);
    let transition = expiry_first
        .store
        .expire(&expiry(&expiry_first, due, Uuid::now_v7()))
        .await
        .expect("expire first");
    assert!(transition.changed());
    let late = expiry_first
        .store
        .decide(&decision(
            &expiry_first,
            &expiry_first.actor,
            "late-decision",
            due,
            Uuid::now_v7(),
        ))
        .await;
    assert!(matches!(
        late,
        Err(ApprovalProgressError::Approval(
            ApprovalError::AlreadyDecided
        ))
    ));

    let decision_first = fixture("approval-decision-first").await;
    decision_first
        .store
        .decide(&decision(
            &decision_first,
            &decision_first.actor,
            "early-decision",
            decision_first.opened_at + Duration::from_secs(5),
            Uuid::now_v7(),
        ))
        .await
        .expect("decide first");
    let losing = decision_first
        .store
        .expire(&expiry(
            &decision_first,
            decision_first.request.deadline() + Duration::from_secs(1),
            Uuid::now_v7(),
        ))
        .await
        .expect("losing expiry");
    assert!(!losing.changed());
    assert_eq!(losing.status(), ApprovalStatus::Approved);
}

#[tokio::test]
async fn current_membership_and_tenant_boundary_are_checked_before_receipt_replay() {
    let fixture = fixture("approval-authz").await;
    sqlx::query("DELETE FROM memberships WHERE tenant_id=$1 AND subject=$2")
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.actor.subject().as_str())
        .execute(&fixture.world.db.pool)
        .await
        .expect("revoke approver");
    let denied = fixture
        .store
        .decide(&decision(
            &fixture,
            &fixture.actor,
            "revoked",
            fixture.opened_at + Duration::from_secs(5),
            Uuid::now_v7(),
        ))
        .await;
    assert!(matches!(
        denied,
        Err(ApprovalProgressError::Approval(ApprovalError::Forbidden))
    ));
    assert_eq!(
        fixture
            .store
            .load(address(&fixture))
            .await
            .expect("still pending")
            .status(),
        ApprovalStatus::Pending
    );

    let foreign_address = ApprovalAddress::new(
        fixture.world.other_admin.tenant_id(),
        fixture.world.other_namespace.id(),
        fixture.request.id(),
    );
    let foreign_at = fixture.opened_at + Duration::from_secs(6);
    let foreign = ApprovalDecisionAttempt::new(
        foreign_address,
        fixture.world.other_admin.subject().clone(),
        ApprovalReceiptId::generate(),
        ApprovalDecisionCommand::new(
            DecisionKey::parse("foreign").expect("foreign key"),
            Digest::new([72; 32]),
            ApprovalDecision::Approve,
            None,
        ),
        AuditRecord::by(
            &fixture.world.other_admin,
            AuditAction::ApprovalDecided,
            ResourceType::ApprovalRequest,
            fixture.request.id().to_string(),
            None,
            foreign_at,
        ),
        Uuid::now_v7(),
        foreign_at,
    );
    assert!(matches!(
        fixture.store.decide(&foreign).await,
        Err(ApprovalProgressError::NotFound)
    ));
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("no receipt");
    assert_eq!(receipts, 0);
}

#[tokio::test]
async fn outbox_failure_rolls_back_request_timer_journal_projection_and_audit() {
    let world = world().await;
    let (run, _) = approval_run(&world, "approval-rollback").await;
    let scheduler = scheduler(&world.db.pool, run).await;
    let opened_at = crate::support::now();
    let request = ApprovalRequest::open(
        ApprovalRequestId::generate(),
        world.admin.tenant_id(),
        world.namespace.id(),
        run,
        ExecutionInstanceId::root(NodeId::parse("approval").expect("approval node")),
        ApprovalPolicy::new(Vec::new(), vec![Role::Operator]).expect("approval policy"),
        world.admin.subject().clone(),
        2,
        opened_at + Duration::from_secs(60),
    );
    let duplicate_outbox = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES ($1,$2,$3,'fixture.reserved','{}')",
    )
    .bind(duplicate_outbox)
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("reserve outbox id");
    let opening = ApprovalOpening::new(
        request.clone(),
        ApprovalRoutes::new(
            NodeId::parse("approved").expect("approved route"),
            NodeId::parse("rejected").expect("rejected route"),
            NodeId::parse("expired").expect("expired route"),
        ),
        scheduler,
        AuditRecord::engine(
            world.admin.tenant_id(),
            AuditAction::ApprovalOpened,
            ResourceType::ApprovalRequest,
            request.id().to_string(),
            "request_opened",
            None,
            opened_at,
        ),
        duplicate_outbox,
        opened_at,
    );
    let store = PgApprovalProgressStore::new(world.db.pool.clone());
    assert!(matches!(
        store.open(&opening).await,
        Err(ApprovalProgressError::Storage(_))
    ));
    let evidence: (i64, i64, i64, i64, String, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM approval_requests WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM approval_timers WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3 AND event_type='approval.opened'), \
          (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$4 AND action='approval.opened'), \
          (SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$3 AND node='approval'), \
          (SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$3)",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(request.id().as_uuid())
    .bind(run.as_uuid())
    .bind(request.id().to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("rolled back evidence");
    assert_eq!(evidence, (0, 0, 0, 0, "PENDING".into(), 1));

    let inconsistent = sqlx::query(
        "UPDATE steps SET approval_request_id=$3 \
         WHERE tenant_id=$1 AND run_id=$2 AND node='approval'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(Uuid::now_v7())
    .execute(&world.db.pool)
    .await;
    assert!(inconsistent.is_err());
}

/// A primitive transaction cut off at any await point (T014-007) commits whole or not at all, and
/// never leaves its connection inside a transaction: decisions are cut off after a few microseconds,
/// round after round, on a pool of one connection, so every round meets what the one before left.
#[tokio::test]
async fn a_decision_cut_off_at_any_await_commits_whole_or_not_at_all() {
    let fixture = fixture("approval-cut-off").await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&fixture.world.db.url)
        .await
        .expect("one-connection pool");
    let cut = PgApprovalProgressStore::new(pool.clone());
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let request = fixture.request.id().as_uuid();
    let mut committed_rounds = 0;
    let mut cut_rounds = 0;
    for round in 0..400_u64 {
        let at = fixture.opened_at + Duration::from_secs(1);
        let attempt = decision(&fixture, &fixture.actor, "cut-off", at, Uuid::now_v7());
        let micros = 20 + (round * 41) % 4_000;
        if tokio::time::timeout(Duration::from_micros(micros), cut.decide(&attempt))
            .await
            .is_err()
        {
            cut_rounds += 1;
        }

        // Nothing is left open on the connection: on its own a statement is its own transaction,
        // so `now()` moves; inside a forgotten transaction it would not.
        let first: OffsetDateTime = sqlx::query_scalar("SELECT now()")
            .fetch_one(&pool)
            .await
            .expect("first statement");
        let second: OffsetDateTime = sqlx::query_scalar("SELECT now()")
            .fetch_one(&pool)
            .await
            .expect("second statement");
        assert_ne!(
            first, second,
            "round {round} left its connection in a transaction"
        );

        // All of the decision or none of it.
        let (status, receipts, settled, audits): (String, i64, i64, i64) = sqlx::query_as(
            "SELECT r.status, \
                    (SELECT count(*) FROM approval_decision_receipts x \
                      WHERE x.tenant_id=r.tenant_id AND x.request_id=r.request_id), \
                    (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id \
                      AND o.aggregate_id=r.run_id AND o.event_type='approval.settled'), \
                    (SELECT count(*) FROM audit_records a WHERE a.tenant_id=r.tenant_id \
                      AND a.resource_id=r.request_id::text AND a.action='approval.decided') \
             FROM approval_requests r WHERE r.tenant_id=$1 AND r.request_id=$2",
        )
        .bind(tenant)
        .bind(request)
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("decision evidence");
        match status.as_str() {
            "PENDING" => assert_eq!((receipts, settled, audits), (0, 0, 0), "round {round}"),
            "APPROVED" => {
                assert_eq!((receipts, settled, audits), (1, 1, 1), "round {round}");
                committed_rounds += 1;
            }
            other => panic!("round {round}: unexpected status {other}"),
        }
    }

    assert!(
        cut_rounds > 0,
        "no round was cut off: the test proves nothing"
    );
    // Uncut, the same decision commits or replays exactly once.
    let at = fixture.opened_at + Duration::from_secs(1);
    let outcome = cut
        .decide(&decision(
            &fixture,
            &fixture.actor,
            "cut-off",
            at,
            Uuid::now_v7(),
        ))
        .await
        .expect("uncut decision");
    assert_eq!(outcome.replayed(), committed_rounds > 0);
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2",
    )
    .bind(tenant)
    .bind(request)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("receipts");
    assert_eq!(receipts, 1);
}

/// The engine serves requests as the least-privilege runtime role, which can read `memberships`
/// but not lock a row of it: re-checking the decider's role must work for that role. The other
/// approval tests connect as the database owner, which hid a `permission denied for table
/// memberships` on every decision (G294).
#[tokio::test]
async fn a_decision_is_committed_by_the_least_privilege_runtime_role() {
    let fixture = fixture("approval-runtime-role").await;
    let runtime = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&fixture.world.db.url_for(
            crate::support::RUNTIME_ROLE,
            crate::support::RUNTIME_ROLE_PASSWORD,
        ))
        .await
        .expect("connect as the runtime role");
    let store = PgApprovalProgressStore::new(runtime);
    let at = fixture.opened_at + Duration::from_secs(1);
    let committed = store
        .decide(&decision(
            &fixture,
            &fixture.actor,
            "runtime-role",
            at,
            Uuid::now_v7(),
        ))
        .await
        .expect("the runtime role commits a decision");
    assert!(!committed.replayed());
    let replayed = store
        .decide(&decision(
            &fixture,
            &fixture.actor,
            "runtime-role",
            at,
            Uuid::now_v7(),
        ))
        .await
        .expect("and replays it");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt().id(), committed.receipt().id());
}

#[tokio::test]
async fn approval_post_wait_fence_rejects_opening_after_expiry_without_effects() {
    assert_opening_post_wait(false).await;
}

#[tokio::test]
async fn approval_post_wait_fence_preserves_generation_storage_error() {
    assert_opening_post_wait(true).await;
}

async fn assert_opening_post_wait(overflow: bool) {
    use crate::bounded_fences::{SchedulerExpiryBarrier, scope_checkpoint};
    let fixture = pending_fixture("approval-postwait").await;
    let store = PgApprovalProgressStore::new(fixture.world.db.runtime_pool().await);
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let command = |authority| {
        ApprovalOpening::new(
            fixture.request.clone(),
            fixture.routes.clone(),
            authority,
            engine_audit(&fixture, AuditAction::ApprovalOpened, fixture.opened_at),
            Uuid::now_v7(),
            fixture.opened_at,
        )
    };
    let claimed = if overflow {
        SchedulerAuthority::for_run(fixture.scheduler.owner(), fixture.run, u64::MAX)
    } else {
        fixture.scheduler.clone()
    };
    let prepared = command(claimed);
    let before = scope_checkpoint(&fixture.world.db.pool, tenant).await;
    let result = if overflow {
        store.open(&prepared).await
    } else {
        let barrier = SchedulerExpiryBarrier::hold(&fixture.world.db.pool, fixture.run).await;
        let (result, ()) = tokio::join!(store.open(&prepared), barrier.expire_after_wait());
        result
    };
    let error = result.expect_err("invalid authority must refuse approval opening");
    if overflow {
        assert!(matches!(
            error,
            ApprovalProgressError::Storage(sagat_application::error::StoreError::Internal { .. })
        ));
    } else {
        assert_eq!(error, ApprovalProgressError::SchedulerFenceLost);
    }
    assert_eq!(
        scope_checkpoint(&fixture.world.db.pool, tenant).await,
        before
    );
    if !overflow {
        let fresh = command(scheduler(&fixture.world.db.pool, fixture.run).await);
        assert_eq!(
            store.open(&fresh).await.expect("fresh opening").status(),
            ApprovalStatus::Pending
        );
        let audits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
             AND action='approval.opened' AND trace_id ~ '^[0-9a-f]{32}$'",
        )
        .bind(tenant)
        .bind(fixture.request.id().to_string())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("one traced opening audit");
        assert_eq!(audits, 1);
        let committed = scope_checkpoint(&fixture.world.db.pool, tenant).await;
        assert_eq!(
            store
                .open(&fresh)
                .await
                .expect_err("opening is not a replay port"),
            ApprovalProgressError::RuntimeUnavailable
        );
        assert_eq!(
            scope_checkpoint(&fixture.world.db.pool, tenant).await,
            committed
        );
    }
}

#[tokio::test]
async fn approval_post_wait_fence_rejects_due_expiry_without_effects() {
    use crate::bounded_fences::{SchedulerExpiryBarrier, scope_checkpoint};
    let fixture = pending_fixture("approval-expiry-postwait").await;
    let store = PgApprovalProgressStore::new(fixture.world.db.runtime_pool().await);
    store
        .open(&ApprovalOpening::new(
            fixture.request.clone(),
            fixture.routes.clone(),
            fixture.scheduler.clone(),
            engine_audit(&fixture, AuditAction::ApprovalOpened, fixture.opened_at),
            Uuid::now_v7(),
            fixture.opened_at,
        ))
        .await
        .expect("open through runtime role");
    // Wait for the actual persisted business deadline; do not substitute a future occurred_at.
    // The usual eventually helper is deliberately limited to five seconds, shorter than this IR.
    let due_at = tokio::time::timeout(Duration::from_secs(75), async {
        loop {
            let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&fixture.world.db.pool)
                .await
                .expect("database due clock");
            if now >= fixture.request.deadline() {
                break now;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("approval becomes due within its 60-second fixture budget");
    let command = |authority| {
        ApprovalExpiryAttempt::new(
            address(&fixture),
            authority,
            engine_audit(&fixture, AuditAction::ApprovalExpired, due_at),
            Uuid::now_v7(),
            due_at,
        )
    };
    let prepared = command(fixture.scheduler.clone());
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let before = scope_checkpoint(&fixture.world.db.pool, tenant).await;
    let barrier = SchedulerExpiryBarrier::hold(&fixture.world.db.pool, fixture.run).await;
    let (result, ()) = tokio::join!(store.expire(&prepared), barrier.expire_after_wait());
    assert_eq!(
        result.expect_err("expired scheduler cannot settle due approval"),
        ApprovalProgressError::SchedulerFenceLost
    );
    assert_eq!(
        scope_checkpoint(&fixture.world.db.pool, tenant).await,
        before
    );
    let fresh = command(scheduler(&fixture.world.db.pool, fixture.run).await);
    let settled = store
        .expire(&fresh)
        .await
        .expect("fresh own authority expires due request");
    assert!(settled.changed());
    assert_eq!(settled.status(), ApprovalStatus::Expired);
    let effects: (i64,i64,i64,String) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
          AND action='approval.expired' AND trace_id ~ '^[0-9a-f]{32}$'), \
         (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3 AND event_type='approval.expired'), \
         (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id=$4), \
         (SELECT state FROM approval_timers WHERE tenant_id=$1 AND request_id=$5)",
    ).bind(tenant).bind(fixture.request.id().to_string()).bind(fixture.run.as_uuid())
    .bind(fresh.outbox_id()).bind(fixture.request.id().as_uuid()).fetch_one(&fixture.world.db.pool)
    .await.expect("atomic expiry effects");
    assert_eq!(effects, (1, 1, 1, "FIRED".into()));
}

/// Trusted storage protocol fixture: this does not open new public approval/chain shapes.
#[tokio::test]
async fn approval_chain_prefix_precedes_run_lock_and_lasts_until_decision_commit() {
    let fixture = fixture("approval-chain-prefix").await;
    let pool = &fixture.world.db.pool;
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let run = fixture.run.as_uuid();
    let mut setup = pool.begin().await.expect("chain setup");
    sqlx::query("UPDATE runs SET deadline_at=clock_timestamp()+interval '1 hour' WHERE tenant_id=$1 AND id=$2")
        .bind(tenant).bind(run).execute(&mut *setup).await.expect("original chain deadline");
    sagat_infra_postgres::initialize_chain_in_transaction(
        &mut setup,
        &sagat_application::chain_progress::ChainInitialization::new(
            sagat_domain::execution::ChainScope::new(
                fixture.world.admin.tenant_id(),
                fixture.world.namespace.id(),
            ),
            fixture.run,
            sagat_domain::execution::ChainPolicy::new(2).expect("chain policy"),
        ),
    )
    .await
    .expect("real chain initialization after approval opened");
    setup
        .commit()
        .await
        .expect("persist consistent chain metadata");
    sqlx::raw_sql(
        "CREATE FUNCTION test_hold_approval_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN PERFORM pg_advisory_xact_lock(37140038::bigint); RETURN NEW; END $$; \
         CREATE TRIGGER test_hold_approval_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW WHEN (NEW.action='approval.decided') EXECUTE FUNCTION test_hold_approval_audit()",
    )
    .execute(pool)
    .await
    .expect("decision lifetime gate");
    let mut audit_gate = pool.begin().await.expect("audit gate");
    let audit_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *audit_gate)
        .await
        .expect("audit backend");
    sqlx::query("SELECT pg_advisory_xact_lock(37140038::bigint)")
        .execute(&mut *audit_gate)
        .await
        .expect("hold audit gate");
    let mut chain_gate = pool.begin().await.expect("chain gate");
    let chain_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *chain_gate)
        .await
        .expect("chain backend");
    sqlx::query(
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(run)
    .execute(&mut *chain_gate)
    .await
    .expect("hold original chain");
    let store = PgApprovalProgressStore::new(fixture.world.db.runtime_pool().await);
    let command = decision(
        &fixture,
        &fixture.actor,
        "chain-prefix",
        fixture.opened_at,
        Uuid::now_v7(),
    );
    let mut request = Box::pin(store.decide(&command));
    tokio::select! {
        result = &mut request => panic!("decision bypassed chain gate: {result:?}"),
        () = crate::support::eventually("approval decision waits on chain before run", || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%execution_chains%' \
                 AND (NOT $2 OR usename='sagat_app'))",
            ).bind(chain_pid).bind(crate::support::as_runtime_role()).fetch_one(pool).await
            .expect("actual chain wait").then_some(())
        }) => {}
    }
    let mut probe = pool.acquire().await.expect("early own-run probe");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
        .bind(tenant)
        .bind(run)
        .execute(&mut *probe)
        .await
        .expect("chain wait holds no own run");
    drop(probe);
    chain_gate
        .rollback()
        .await
        .expect("release initial chain guard");
    tokio::select! {
        result = &mut request => panic!("audit gate must pause decision: {result:?}"),
        () = crate::support::eventually("decision reaches final audit in same transaction", || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND $1=ANY(pg_blocking_pids(pid)) AND (NOT $2 OR usename='sagat_app'))",
            ).bind(audit_pid).bind(crate::support::as_runtime_role()).fetch_one(pool).await
            .expect("actual final audit wait").then_some(())
        }) => {}
    }
    let mut probe = pool.acquire().await.expect("retained guards probe");
    for statement in [
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR SHARE NOWAIT",
        "SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT",
    ] {
        let error = sqlx::query(statement)
            .bind(tenant)
            .bind(run)
            .execute(&mut *probe)
            .await
            .expect_err("exclusive chain and own-run guards retained until commit");
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("55P03")
        );
    }
    drop(probe);
    audit_gate.rollback().await.expect("release audit guard");
    let committed = request.await.expect("approved after ordered locks");
    assert!(!committed.replayed());
    let effects: (String, i64, Vec<String>, i64, i64, i64) = sqlx::query_as(
        "SELECT status, \
         (SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2), \
         (SELECT array_agg(event_type ORDER BY seq) FROM run_events WHERE tenant_id=$1 AND run_id=$3 AND seq>3), \
         (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id=$4), \
         (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$5 \
          AND action='approval.decided' AND trace_id ~ '^[0-9a-f]{32}$'), \
         (SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$3) \
         FROM approval_requests WHERE tenant_id=$1 AND request_id=$2",
    ).bind(tenant).bind(fixture.request.id().as_uuid()).bind(run).bind(command.outbox_id())
    .bind(fixture.request.id().to_string()).fetch_one(pool).await.expect("one atomic decision");
    assert_eq!(
        effects,
        (
            "APPROVED".into(),
            1,
            vec!["approval.decided".into(), "step.routed".into()],
            1,
            1,
            5
        )
    );
    let before_replay = crate::bounded_fences::scope_checkpoint(pool, tenant).await;
    assert!(
        store
            .decide(&command)
            .await
            .expect("current-authority receipt replay")
            .replayed()
    );
    assert_eq!(
        crate::bounded_fences::scope_checkpoint(pool, tenant).await,
        before_replay
    );
    // The original chain is still the member's chain; deciding never forwards or changes its head.
    let head: Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(tenant)
    .bind(run)
    .fetch_one(pool)
    .await
    .expect("pinned chain head");
    assert_eq!(head, run);
}
