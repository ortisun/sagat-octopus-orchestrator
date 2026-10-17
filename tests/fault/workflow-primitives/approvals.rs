//! Approval crash boundaries, duplicate delivery and settlement races against real PostgreSQL.

use std::{sync::Arc, time::Duration};

use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalExpiryAttempt, ApprovalOpening,
    ApprovalProgressError, ApprovalProgressStore, ApprovalRoutes,
};
use sagat_application::clock::SystemClock;
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine, instance_step_id};
use sagat_application::primitives::SchedulerAuthority;
use sagat_application::runs::RunService;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalError, ApprovalPolicy, ApprovalReasonCode,
    ApprovalReceiptId, ApprovalRequest, ApprovalRequestId, ApprovalStatus, DecisionKey,
    ExecutionInstanceId, RunId,
};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::workflow::{Digest, NodeId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgApprovalProgressStore, PgRunStore};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::support::engine::{chain, ctx, world};

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
    let start = Arc::new(Barrier::new(3));
    let decision_start = start.clone();
    let decided = tokio::spawn(async move {
        decision_start.wait().await;
        decision_store.decide(&decision).await
    });
    let expiry_start = start.clone();
    let expired = tokio::spawn(async move {
        expiry_start.wait().await;
        expiry_store.expire(&expiry).await
    });
    start.wait().await;
    let (decided, expired) = tokio::join!(decided, expired);
    let decided = decided.expect("decision contender completed");
    let expired = expired.expect("expiry contender completed");
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

/// A decision whose body (hash, decision and reason code) the test chooses.
#[allow(clippy::too_many_arguments)]
fn body_decision(
    fixture: &Fixture,
    actor: &Principal,
    key: &str,
    hash: u8,
    outcome: ApprovalDecision,
    reason: Option<&str>,
    at: OffsetDateTime,
) -> ApprovalDecisionAttempt {
    ApprovalDecisionAttempt::new(
        address(fixture),
        actor.subject().clone(),
        ApprovalReceiptId::generate(),
        ApprovalDecisionCommand::new(
            DecisionKey::parse(key).expect("decision key"),
            Digest::new([hash; 32]),
            outcome,
            reason.map(|code| ApprovalReasonCode::parse(code).expect("reason code")),
        ),
        actor_audit(fixture, actor, at),
        Uuid::now_v7(),
        at,
    )
}

/// Receipts, journal events, outbox rows and audit rows that belong to the approval's run.
async fn settlement_footprint(fixture: &Fixture) -> (i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3), \
          (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$3), \
          (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$4)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(fixture.request.id().to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("settlement footprint")
}

#[tokio::test]
async fn a_used_decision_key_with_another_body_is_refused_after_restart_and_changes_nothing() {
    let fixture = fixture("approval-key-body").await;
    let at = fixture.opened_at + Duration::from_secs(10);
    let committed = fixture
        .store
        .decide(&body_decision(
            &fixture,
            &fixture.actor,
            "one-key",
            71,
            ApprovalDecision::Approve,
            Some("POLICY_OK"),
            at,
        ))
        .await
        .expect("first decision commits");
    let before = settlement_footprint(&fixture).await;
    assert_eq!(before.0, 1);

    // A new process image: nothing of the first call is in memory.
    let restarted = PgApprovalProgressStore::new(fixture.world.db.pool.clone());
    let changed_bodies = [
        (72, ApprovalDecision::Approve, Some("POLICY_OK")),
        (71, ApprovalDecision::Reject, Some("POLICY_OK")),
        (71, ApprovalDecision::Approve, Some("CHANGED")),
        (71, ApprovalDecision::Approve, None),
    ];
    for (hash, outcome, reason) in changed_bodies {
        let refused = restarted
            .decide(&body_decision(
                &fixture,
                &fixture.actor,
                "one-key",
                hash,
                outcome,
                reason,
                at + Duration::from_secs(1),
            ))
            .await;
        assert!(
            matches!(
                refused,
                Err(ApprovalProgressError::Approval(
                    ApprovalError::IdempotencyKeyReused
                ))
            ),
            "hash {hash} {outcome:?} {reason:?}: {refused:?}"
        );
        assert_eq!(settlement_footprint(&fixture).await, before);
    }

    // A different key is a different decision: the approval already has its outcome.
    let other_key = restarted
        .decide(&body_decision(
            &fixture,
            &fixture.actor,
            "another-key",
            72,
            ApprovalDecision::Reject,
            None,
            at + Duration::from_secs(2),
        ))
        .await;
    assert!(matches!(
        other_key,
        Err(ApprovalProgressError::Approval(
            ApprovalError::AlreadyDecided
        ))
    ));
    assert_eq!(settlement_footprint(&fixture).await, before);

    // The refusals did not disturb the receipt the first call produced.
    let replay = restarted
        .decide(&body_decision(
            &fixture,
            &fixture.actor,
            "one-key",
            71,
            ApprovalDecision::Approve,
            Some("POLICY_OK"),
            at + Duration::from_secs(3),
        ))
        .await
        .expect("the identical body still replays");
    assert!(replay.replayed());
    assert_eq!(replay.receipt().id(), committed.receipt().id());
    assert_eq!(settlement_footprint(&fixture).await, before);
}

#[tokio::test]
async fn a_revoked_approver_cannot_replay_a_receipt_it_committed() {
    let fixture = fixture("approval-revoked-replay").await;
    let at = fixture.opened_at + Duration::from_secs(10);
    fixture
        .store
        .decide(&body_decision(
            &fixture,
            &fixture.actor,
            "kept-key",
            71,
            ApprovalDecision::Approve,
            None,
            at,
        ))
        .await
        .expect("decision commits while the approver is a member");
    let before = settlement_footprint(&fixture).await;
    sqlx::query("DELETE FROM memberships WHERE tenant_id=$1 AND subject=$2")
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.actor.subject().as_str())
        .execute(&fixture.world.db.pool)
        .await
        .expect("revoke the approver after the decision");

    for hash in [71, 72] {
        let denied = PgApprovalProgressStore::new(fixture.world.db.pool.clone())
            .decide(&body_decision(
                &fixture,
                &fixture.actor,
                "kept-key",
                hash,
                ApprovalDecision::Approve,
                None,
                at + Duration::from_secs(5),
            ))
            .await;
        assert!(
            matches!(
                denied,
                Err(ApprovalProgressError::Approval(ApprovalError::Forbidden))
            ),
            "identical and changed bodies are both refused before the receipt is read: {denied:?}"
        );
    }
    // The refusals are audited as one denial (repeats inside the audit window collapse); nothing else
    // moved: no receipt, journal event or outbox row.
    let after = settlement_footprint(&fixture).await;
    assert_eq!((after.0, after.1, after.2), (before.0, before.1, before.2));
    assert_eq!(after.3, before.3 + 1);
    let denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='approval.denied'",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("denial audit rows");
    assert_eq!(denials, 1);
    assert_eq!(
        fixture
            .store
            .load(address(&fixture))
            .await
            .expect("the outcome stands")
            .status(),
        ApprovalStatus::Approved
    );
}

/// SQL that makes the audit row of one approval action fail, written once for every action.
macro_rules! reject_audit_of {
    ($action:literal) => {
        concat!(
            "CREATE FUNCTION reject_approval_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN IF NEW.action='",
            $action,
            "' THEN RAISE EXCEPTION 'injected audit failure'; END IF; RETURN NEW; END $$; \
             CREATE TRIGGER reject_approval_audit BEFORE INSERT ON audit_records \
             FOR EACH ROW EXECUTE FUNCTION reject_approval_audit()"
        )
    };
}

/// Installs the audit failure, runs `attempt`, removes the failure and returns what the attempt answered.
async fn with_audit_failure<T>(
    fixture: &Fixture,
    install: &'static str,
    attempt: impl std::future::Future<Output = T>,
) -> T {
    sqlx::raw_sql(install)
        .execute(&fixture.world.db.pool)
        .await
        .expect("install the audit failure");
    let answered = attempt.await;
    sqlx::raw_sql(
        "DROP TRIGGER reject_approval_audit ON audit_records; \
         DROP FUNCTION reject_approval_audit()",
    )
    .execute(&fixture.world.db.pool)
    .await
    .expect("remove the audit failure");
    answered
}

#[tokio::test]
async fn an_audit_failure_rolls_back_a_decision_and_the_retry_commits_it_once() {
    let fixture = fixture("approval-audit-decide").await;
    let at = fixture.opened_at + Duration::from_secs(5);
    let attempt = body_decision(
        &fixture,
        &fixture.actor,
        "audited",
        71,
        ApprovalDecision::Approve,
        None,
        at,
    );
    let failed = with_audit_failure(
        &fixture,
        reject_audit_of!("approval.decided"),
        fixture.store.decide(&attempt),
    )
    .await;
    assert!(
        matches!(failed, Err(ApprovalProgressError::Storage(_))),
        "{failed:?}"
    );
    assert_pending_after_failed_settlement(&fixture).await;
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='approval.decided'",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("audit rows");
    assert_eq!(audits, 0);

    // Nothing of the failed attempt is left behind, so the same body commits as a first decision.
    let retried = fixture
        .store
        .decide(&attempt)
        .await
        .expect("the retry commits after the failure is gone");
    assert!(!retried.replayed());
    assert_eq!(
        fixture
            .store
            .load(address(&fixture))
            .await
            .expect("decided")
            .status(),
        ApprovalStatus::Approved
    );
    assert_eq!(settlement_footprint(&fixture).await.0, 1);
}

#[tokio::test]
async fn an_audit_failure_rolls_back_an_expiry_and_the_retry_expires_it_once() {
    let fixture = fixture("approval-audit-expire").await;
    let due = fixture.request.deadline() + Duration::from_secs(1);
    let attempt = expiry(&fixture, due, Uuid::now_v7());
    let failed = with_audit_failure(
        &fixture,
        reject_audit_of!("approval.expired"),
        fixture.store.expire(&attempt),
    )
    .await;
    assert!(
        matches!(failed, Err(ApprovalProgressError::Storage(_))),
        "{failed:?}"
    );
    assert_pending_after_failed_settlement(&fixture).await;

    let retried = fixture
        .store
        .expire(&expiry(&fixture, due, Uuid::now_v7()))
        .await
        .expect("the retry expires after the failure is gone");
    assert!(retried.changed());
    assert_eq!(retried.status(), ApprovalStatus::Expired);
    let outcomes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
         AND event_type IN ('approval.decided','approval.expired')",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("outcome journal");
    assert_eq!(outcomes, 1);
}

#[tokio::test]
async fn an_audit_failure_rolls_back_opening_and_the_retry_opens_one_request() {
    let fixture = pending_fixture("approval-audit-open").await;
    let opening = ApprovalOpening::new(
        fixture.request.clone(),
        fixture.routes.clone(),
        fixture.scheduler.clone(),
        engine_audit(&fixture, AuditAction::ApprovalOpened, fixture.opened_at),
        Uuid::now_v7(),
        fixture.opened_at,
    );
    let failed = with_audit_failure(
        &fixture,
        reject_audit_of!("approval.opened"),
        fixture.store.open(&opening),
    )
    .await;
    assert!(
        matches!(failed, Err(ApprovalProgressError::Storage(_))),
        "{failed:?}"
    );
    let evidence: (i64, i64, i64, i64, String, bool, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM approval_requests WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM approval_timers WHERE tenant_id=$1 AND request_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$3 \
             AND event_type='approval.opened'), \
          (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$3 \
             AND event_type LIKE 'approval.%'), \
          (SELECT status FROM steps WHERE tenant_id=$1 AND run_id=$3 AND node='approval'), \
          (SELECT approval_request_id IS NOT NULL FROM steps \
             WHERE tenant_id=$1 AND run_id=$3 AND node='approval'), \
          (SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$3)",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.request.id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("opening rollback evidence");
    assert_eq!(evidence, (0, 0, 0, 0, "PENDING".into(), false, 1));

    fixture
        .store
        .open(&opening)
        .await
        .expect("the same opening commits once the failure is gone");
    assert_eq!(
        fixture
            .store
            .load(address(&fixture))
            .await
            .expect("opened")
            .status(),
        ApprovalStatus::Pending
    );
}

#[derive(Debug)]
struct StructuredJsonIrCodec;

impl IrCodec for StructuredJsonIrCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        IrCodec::check_draft(&JsonIrCodec, document)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        JsonIrCodec.parse_structured(document)
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        IrCodec::diff(&JsonIrCodec, from, to)
    }
}

fn run_service(fixture: &Fixture) -> RunService<PgRunStore, SystemClock, JsonEvaluator> {
    let cache = DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8);
    RunService::new(
        PgRunStore::new(fixture.world.db.pool.clone(), cache.clone()),
        SystemClock,
        Engine::new(JsonEvaluator),
        cache,
    )
}

async fn raw_signals_are_refused(fixture: &Fixture, round: &str) {
    let runs = run_service(fixture);
    for name in ["approval.approve", "approval.decision", "approval.expire"] {
        let result = runs
            .signal(
                &fixture.world.admin,
                fixture.run,
                name,
                br#"{"approved":true}"#,
                &format!("raw-{round}-{name}"),
                &ctx(),
            )
            .await;
        assert!(
            matches!(result, Err(AppError::StateConflict(_))),
            "{round} {name}: {result:?}"
        );
    }
}

#[tokio::test]
async fn a_raw_signal_never_settles_an_approval_before_during_or_after_its_outcome() {
    // Before: the request stays pending and nothing is routed or journalled.
    let pending = fixture("approval-signal-before").await;
    let before = settlement_footprint(&pending).await;
    raw_signals_are_refused(&pending, "pending").await;
    assert_eq!(settlement_footprint(&pending).await, before);
    assert_eq!(
        pending
            .store
            .load(address(&pending))
            .await
            .expect("still pending")
            .status(),
        ApprovalStatus::Pending
    );
    let routed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 \
         AND node IN ('approved','rejected','expired')",
    )
    .bind(pending.world.admin.tenant_id().as_uuid())
    .bind(pending.run.as_uuid())
    .fetch_one(&pending.world.db.pool)
    .await
    .expect("routes");
    assert_eq!(routed, 0);

    // During: signals racing a real decision cannot change who wins or add an outcome.
    let racing = fixture("approval-signal-during").await;
    let decision = body_decision(
        &racing,
        &racing.actor,
        "racing-key",
        71,
        ApprovalDecision::Reject,
        None,
        racing.opened_at + Duration::from_secs(5),
    );
    let store = racing.store.clone();
    let start = Arc::new(Barrier::new(2));
    let decide_start = start.clone();
    let decided = tokio::spawn(async move {
        decide_start.wait().await;
        store.decide(&decision).await
    });
    start.wait().await;
    raw_signals_are_refused(&racing, "racing").await;
    decided
        .await
        .expect("decision task")
        .expect("the decision commits");
    assert_eq!(
        racing
            .store
            .load(address(&racing))
            .await
            .expect("decided")
            .status(),
        ApprovalStatus::Rejected
    );

    // After: neither a decision nor an expiry gains a second outcome from a signal.
    let after = settlement_footprint(&racing).await;
    raw_signals_are_refused(&racing, "decided").await;
    assert_eq!(settlement_footprint(&racing).await, after);

    let expired = fixture("approval-signal-after-expiry").await;
    expired
        .store
        .expire(&expiry(
            &expired,
            expired.request.deadline() + Duration::from_secs(1),
            Uuid::now_v7(),
        ))
        .await
        .expect("expiry commits");
    let after = settlement_footprint(&expired).await;
    raw_signals_are_refused(&expired, "expired").await;
    assert_eq!(settlement_footprint(&expired).await, after);
    for fixture in [&racing, &expired] {
        let outcomes: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
             AND event_type IN ('approval.decided','approval.expired')",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("outcomes");
        assert_eq!(outcomes, 1);
    }
}
