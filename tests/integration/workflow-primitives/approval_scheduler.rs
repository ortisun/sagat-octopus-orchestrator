//! Real PostgreSQL evidence for scheduler-driven approval opening and expiry.

use std::{sync::Arc, time::Duration};

use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalProgressError, ApprovalProgressStore,
};
use sagat_application::clock::SystemClock;
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine, instance_step_id};
use sagat_application::runs::{Lock, RunOperation, RunService, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalError, ApprovalReceiptId, ApprovalRequestId,
    ApprovalStatus, DecisionKey, ExecutionInstanceId, RunId,
};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{Digest, NodeId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::engine::{World, chain, ctx, world};
use crate::support::now;

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

fn approval_workflow(name: &str, timeout_ms: u64) -> serde_json::Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "review",
        "nodes": [
            {
                "id": "review",
                "type": "approval",
                "timeoutMs": timeout_ms,
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

#[tokio::test]
async fn public_native_approval_can_be_published_and_started() {
    let world = world().await;
    let (workflow, _) = world
        .publish(approval_workflow("approval-public-start", 86_400_000))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let record = world.run_record(run).await.expect("started approval run");
    assert_eq!(record.status.as_str(), "RUNNING");
}

#[tokio::test]
async fn public_approval_can_be_cancelled_before_the_scheduler_opens_a_request() {
    let world = world().await;
    let (workflow, _) = world
        .publish(approval_workflow(
            "approval-public-early-cancel",
            86_400_000,
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let cancelled = world
        .runs
        .operate(
            &world.admin,
            run,
            RunOperation::Cancel,
            "cancel before approval opens",
            None,
            "cancel-before-approval-opens",
            &ctx(),
        )
        .await
        .expect("cancel unopened approval");
    assert_eq!(cancelled.status.as_str(), "CANCELLED");
    let requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("no approval request was opened");
    assert_eq!(requests, 0);
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

struct Fixture {
    world: World,
    store: PgRunStore,
    run: RunId,
    placement: Placement,
    actor: Principal,
}

async fn fixture(name: &str, timeout_ms: u64) -> Fixture {
    let world = world().await;
    let actor = world.member("approval-operator", Role::Operator).await;
    let (workflow, _) = world.publish(chain(name)).await;
    let document =
        serde_json::to_vec(&approval_workflow(name, timeout_ms)).expect("approval document");
    StructuredJsonIrCodec
        .parse(&document)
        .expect("typed approval definition");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    let tenant = world.admin.tenant_id();
    let created_at = now();
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id,tenant_id,workflow_id,version_number,digest,document,published_by,published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,$7)",
    )
    .bind(version.as_uuid())
    .bind(tenant.as_uuid())
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(world.admin.subject().as_str())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("trusted approval version");
    let input = b"{}";
    let input_digest = digest(input);
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
         ON CONFLICT (tenant_id,digest) DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(input.as_slice())
    .execute(&world.db.pool)
    .await
    .expect("run input");
    let run = RunId::generate();
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version, \
          input_digest,started_by,trace_id,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',1,$6,$7, \
                 '0af7651916cd43dd8448eb211c80319c',$8,$8,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant.as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(world.admin.subject().as_str())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("approval run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,trace_id,occurred_at) \
         VALUES ($1,$2,1,'run.started',jsonb_build_object('entrypoint','review'), \
                 'system:scheduler','0af7651916cd43dd8448eb211c80319c',$3)",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("run start journal");
    let owner = ExecutionInstanceId::root(NodeId::parse("review").expect("approval node"));
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,updated_at,kind, \
          instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,'review',$3,$4,'PENDING',$5,'FORWARD','{}','{}')",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
    .bind(world.namespace.id().as_uuid())
    .bind(created_at)
    .execute(&world.db.pool)
    .await
    .expect("pending approval step");

    let cache = DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8);
    let store = PgRunStore::new(world.db.pool.clone(), cache);
    let placement = Placement::new("approval-scheduler-test");
    Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    assert!(
        placement
            .snapshot()
            .lease_of(Shard::of(run.as_uuid()), std::time::Instant::now())
            .is_some()
    );
    Fixture {
        world,
        store,
        run,
        placement,
        actor,
    }
}

fn scheduler(fixture: &Fixture) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        fixture.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(fixture.placement.clone())
}

#[tokio::test]
async fn scheduler_opens_then_expires_and_routes_an_approval_after_restart() {
    let fixture = fixture("approval-scheduler-cycle", 1).await;
    let opened = scheduler(&fixture).tick().await;
    assert_eq!(opened.approvals_opened, 1, "{opened:?}");
    assert_eq!(opened.failed, 0, "{opened:?}");

    let (request, status, timer, run_status): (Uuid, String, String, String) = sqlx::query_as(
        "SELECT request.request_id,request.status,timer.state,run.status \
         FROM approval_requests request \
         JOIN approval_timers timer \
           ON timer.tenant_id=request.tenant_id AND timer.request_id=request.request_id \
         JOIN runs run ON run.tenant_id=request.tenant_id AND run.id=request.run_id \
         WHERE request.tenant_id=$1 AND request.run_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("opened approval");
    assert_eq!(
        (status.as_str(), timer.as_str(), run_status.as_str()),
        ("PENDING", "PENDING", "WAITING")
    );

    let expired = scheduler(&fixture).tick().await;
    assert_eq!(expired.approvals_expired, 1, "{expired:?}");
    assert_eq!(expired.failed, 0, "{expired:?}");
    let request_id = ApprovalRequestId::new(request);
    let evidence: (String, String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT request.status,timer.state,step.status, \
          (SELECT count(*) FROM run_events event \
           WHERE event.tenant_id=request.tenant_id AND event.run_id=request.run_id \
             AND event.event_type='approval.expired'), \
          (SELECT count(*) FROM steps route \
           WHERE route.tenant_id=request.tenant_id AND route.run_id=request.run_id \
             AND route.node='expired' AND route.status='PENDING'), \
          (SELECT count(*) FROM audit_records audit \
           WHERE audit.tenant_id=request.tenant_id \
             AND audit.resource_id=$3 AND audit.action IN ('approval.opened','approval.expired')) \
         FROM approval_requests request \
         JOIN approval_timers timer \
           ON timer.tenant_id=request.tenant_id AND timer.request_id=request.request_id \
         JOIN steps step ON step.tenant_id=request.tenant_id AND step.run_id=request.run_id \
           AND step.node='review' \
         WHERE request.tenant_id=$1 AND request.request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(request)
    .bind(request_id.to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("expiry evidence");
    assert_eq!(
        evidence,
        (
            "EXPIRED".into(),
            "FIRED".into(),
            "SUCCEEDED".into(),
            1,
            1,
            2
        )
    );
    let request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("request count");
    assert_eq!(request_count, 1);
}

async fn opened_request(fixture: &Fixture) -> ApprovalRequestId {
    let opened = scheduler(fixture).tick().await;
    assert_eq!(opened.approvals_opened, 1, "{opened:?}");
    assert_eq!(opened.failed, 0, "{opened:?}");
    ApprovalRequestId::new(
        sqlx::query_scalar(
            "SELECT request_id FROM approval_requests WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("opened request"),
    )
}

fn decision_attempt(
    fixture: &Fixture,
    request_id: ApprovalRequestId,
    decision: ApprovalDecision,
) -> ApprovalDecisionAttempt {
    let at = now();
    let audit = AuditRecord::by(
        &fixture.actor,
        AuditAction::ApprovalDecided,
        ResourceType::ApprovalRequest,
        request_id.to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        at,
    );
    ApprovalDecisionAttempt::new(
        ApprovalAddress::new(
            fixture.world.admin.tenant_id(),
            fixture.world.namespace.id(),
            request_id,
        ),
        fixture.actor.subject().clone(),
        ApprovalReceiptId::generate(),
        ApprovalDecisionCommand::new(
            DecisionKey::parse("approval-route-decision").expect("decision key"),
            Digest::new([19; 32]),
            decision,
            None,
        ),
        audit,
        Uuid::now_v7(),
        at,
    )
}

async fn decide(fixture: &Fixture, request_id: ApprovalRequestId, decision: ApprovalDecision) {
    PgRunStore::new(
        fixture.world.db.pool.clone(),
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
    .decide(&decision_attempt(fixture, request_id, decision))
    .await
    .expect("settle approval after restart");
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

#[tokio::test]
async fn scheduler_opened_approval_routes_approved_and_rejected_decisions_after_restart() {
    for (name, decision, status, route) in [
        (
            "approval-scheduler-approved",
            ApprovalDecision::Approve,
            "APPROVED",
            "approved",
        ),
        (
            "approval-scheduler-rejected",
            ApprovalDecision::Reject,
            "REJECTED",
            "rejected",
        ),
    ] {
        let fixture = fixture(name, 60_000).await;
        let request_id = opened_request(&fixture).await;
        decide(&fixture, request_id, decision).await;
        let evidence: (String, String, String, i64, i64) = sqlx::query_as(
            "SELECT request.status,timer.state,owner.status, \
               (SELECT count(*) FROM steps routed \
                WHERE routed.tenant_id=request.tenant_id AND routed.run_id=request.run_id \
                  AND routed.node=$3 AND routed.status='PENDING'), \
               (SELECT count(*) FROM run_events event \
                WHERE event.tenant_id=request.tenant_id AND event.run_id=request.run_id \
                  AND event.event_type='approval.decided') \
             FROM approval_requests request \
             JOIN approval_timers timer \
               ON timer.tenant_id=request.tenant_id AND timer.request_id=request.request_id \
             JOIN steps owner ON owner.tenant_id=request.tenant_id AND owner.run_id=request.run_id \
               AND owner.node='review' \
             WHERE request.tenant_id=$1 AND request.request_id=$2",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(request_id.as_uuid())
        .bind(route)
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("decision routing evidence");
        assert_eq!(
            evidence,
            (status.into(), "CANCELLED".into(), "SUCCEEDED".into(), 1, 1)
        );
        let published =
            crate::hint_contract::assert_outbox_matches_the_contract(&fixture.world.db.pool).await;
        assert!(
            published.iter().any(|kind| kind == "approval.settled"),
            "{published:?}"
        );
    }
}

#[tokio::test]
async fn raw_signal_cannot_settle_or_route_a_pending_approval() {
    let fixture = fixture("approval-scheduler-signal-denied", 60_000).await;
    let request_id = opened_request(&fixture).await;
    fixture
        .store
        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
        .await
        .expect("approval run reload")
        .expect("approval run exists");
    let runs = run_service(&fixture);
    let result = runs
        .signal(
            &fixture.world.admin,
            fixture.run,
            "approval.approve",
            br#"{"approved":true}"#,
            "approval-raw-signal",
            &ctx(),
        )
        .await;
    assert!(
        matches!(result, Err(AppError::StateConflict(_))),
        "{result:?}"
    );
    let evidence: (String, String, i64) = sqlx::query_as(
        "SELECT request.status,owner.status, \
           (SELECT count(*) FROM steps routed \
            WHERE routed.tenant_id=request.tenant_id AND routed.run_id=request.run_id \
              AND routed.node IN ('approved','rejected','expired')) \
         FROM approval_requests request \
         JOIN steps owner ON owner.tenant_id=request.tenant_id AND owner.run_id=request.run_id \
           AND owner.node='review' \
         WHERE request.tenant_id=$1 AND request.request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(request_id.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("approval remains pending");
    assert_eq!(evidence, ("PENDING".into(), "WAITING".into(), 0));
}

async fn cancel_run(fixture: &Fixture, key: &str) -> Result<(), AppError> {
    run_service(fixture)
        .operate(
            &fixture.world.admin,
            fixture.run,
            RunOperation::Cancel,
            "cancel pending approval",
            None,
            key,
            &ctx(),
        )
        .await
        .map(|_| ())
}

fn address(fixture: &Fixture, request_id: ApprovalRequestId) -> ApprovalAddress {
    ApprovalAddress::new(
        fixture.world.admin.tenant_id(),
        fixture.world.namespace.id(),
        request_id,
    )
}

#[tokio::test]
async fn cancelling_the_run_cancels_the_request_timer_and_audits_one_atomic_outcome() {
    let fixture = fixture("approval-scheduler-cancel", 60_000).await;
    let request_id = opened_request(&fixture).await;
    cancel_run(&fixture, "cancel-approval-run")
        .await
        .expect("cancel run");
    let raw: (
        String,
        String,
        bool,
        Option<String>,
        Option<Uuid>,
        i64,
        String,
    ) = sqlx::query_as(
        "SELECT request.status,owner.status,owner.routed,owner.waiting_kind, \
                owner.approval_request_id,request.outcome_sequence,event.event_type \
         FROM approval_requests request JOIN steps owner \
           ON owner.tenant_id=request.tenant_id AND owner.run_id=request.run_id \
          AND owner.node='review' \
         JOIN run_events event ON event.tenant_id=request.tenant_id \
          AND event.run_id=request.run_id AND event.seq=request.outcome_sequence \
         WHERE request.tenant_id=$1 AND request.request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(request_id.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("raw cancellation projection");
    assert_eq!(raw.0, "CANCELLED");
    assert_eq!(raw.1, "CANCELLED");
    assert!(raw.2);
    assert_eq!(raw.3, None);
    assert_eq!(raw.4, None);
    assert!(raw.5 > 2);
    assert_eq!(raw.6, "step.cancelled");
    let request = fixture
        .store
        .load(address(&fixture, request_id))
        .await
        .expect("reload cancelled approval");
    assert_eq!(request.status(), ApprovalStatus::Cancelled);
    let evidence: (String, String, String, String, i64, i64) = sqlx::query_as(
        "SELECT run.status,timer.state,owner.status,request.status, \
           (SELECT count(*) FROM event_outbox outbox \
            WHERE outbox.tenant_id=request.tenant_id AND outbox.aggregate_id=request.run_id \
              AND outbox.event_type='approval.cancelled'), \
           (SELECT count(*) FROM audit_records audit \
            WHERE audit.tenant_id=request.tenant_id AND audit.resource_id=$3 \
              AND audit.action='approval.cancelled') \
         FROM approval_requests request \
         JOIN approval_timers timer \
           ON timer.tenant_id=request.tenant_id AND timer.request_id=request.request_id \
         JOIN runs run ON run.tenant_id=request.tenant_id AND run.id=request.run_id \
         JOIN steps owner ON owner.tenant_id=request.tenant_id AND owner.run_id=request.run_id \
           AND owner.node='review' \
         WHERE request.tenant_id=$1 AND request.request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(request_id.as_uuid())
    .bind(request_id.to_string())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("cancellation evidence");
    assert_eq!(
        evidence,
        (
            "CANCELLED".into(),
            "CANCELLED".into(),
            "CANCELLED".into(),
            "CANCELLED".into(),
            1,
            1,
        )
    );
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.world.db.pool).await;
    crate::hint_contract::assert_published(&published, &["approval.opened", "approval.cancelled"]);
}

#[tokio::test]
async fn decision_expiry_and_cancel_races_keep_exactly_one_approval_outcome() {
    let decision_fixture = fixture("approval-scheduler-cancel-decision-race", 60_000).await;
    let decision_request = opened_request(&decision_fixture).await;
    let attempt = decision_attempt(
        &decision_fixture,
        decision_request,
        ApprovalDecision::Approve,
    );
    let decision_store = decision_fixture.store.clone();
    let (cancelled, decided) = tokio::join!(
        cancel_run(&decision_fixture, "cancel-decision-race"),
        decision_store.decide(&attempt),
    );
    cancelled.expect("run cancellation commits");
    let resolved = decision_fixture
        .store
        .load(address(&decision_fixture, decision_request))
        .await
        .expect("decision cancellation winner");
    match resolved.status() {
        ApprovalStatus::Approved => assert!(decided.is_ok()),
        ApprovalStatus::Cancelled => assert!(
            matches!(
                decided,
                Err(ApprovalProgressError::Approval(
                    ApprovalError::AlreadyDecided
                )) | Err(ApprovalProgressError::RuntimeUnavailable)
            ),
            "{decided:?}"
        ),
        status => panic!("unexpected decision/cancel outcome {status:?}"),
    }

    let expiry_fixture = fixture("approval-scheduler-cancel-expiry-race", 1).await;
    let expiry_request = opened_request(&expiry_fixture).await;
    let expirer = scheduler(&expiry_fixture);
    let (cancelled, expiry) = tokio::join!(
        cancel_run(&expiry_fixture, "cancel-expiry-race"),
        expirer.tick(),
    );
    cancelled.expect("run cancellation commits");
    assert_eq!(expiry.failed, 0, "{expiry:?}");
    let resolved = expiry_fixture
        .store
        .load(address(&expiry_fixture, expiry_request))
        .await
        .expect("expiry cancellation winner");
    assert!(matches!(
        resolved.status(),
        ApprovalStatus::Expired | ApprovalStatus::Cancelled
    ));

    for (fixture, request_id) in [
        (&decision_fixture, decision_request),
        (&expiry_fixture, expiry_request),
    ] {
        let outcomes: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_events \
             WHERE tenant_id=$1 AND run_id=$2 \
               AND (event_type IN ('approval.decided','approval.expired') \
                    OR (event_type='step.cancelled' AND node='review'))",
        )
        .bind(fixture.world.admin.tenant_id().as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("single canonical approval outcome");
        assert_eq!(outcomes, 1, "request {request_id}");
    }
}

async fn cancellation_failure_rolls_back(name: &str, install: &'static str, cleanup: &'static str) {
    let fixture = fixture(name, 60_000).await;
    let request_id = opened_request(&fixture).await;
    sqlx::raw_sql(install)
        .execute(&fixture.world.db.pool)
        .await
        .expect("install cancellation failure");
    assert!(matches!(
        cancel_run(&fixture, &format!("cancel-failure-{name}")).await,
        Err(AppError::Internal)
    ));
    sqlx::raw_sql(cleanup)
        .execute(&fixture.world.db.pool)
        .await
        .expect("remove cancellation failure");
    let evidence: (String, String, String, String, i64) = sqlx::query_as(
        "SELECT run.status,request.status,timer.state,owner.status, \
           (SELECT count(*) FROM run_events event \
            WHERE event.tenant_id=request.tenant_id AND event.run_id=request.run_id \
              AND event.event_type IN ('run.stop_requested','step.cancelled','run.finished')) \
         FROM approval_requests request \
         JOIN approval_timers timer \
           ON timer.tenant_id=request.tenant_id AND timer.request_id=request.request_id \
         JOIN runs run ON run.tenant_id=request.tenant_id AND run.id=request.run_id \
         JOIN steps owner ON owner.tenant_id=request.tenant_id AND owner.run_id=request.run_id \
           AND owner.node='review' \
         WHERE request.tenant_id=$1 AND request.request_id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(request_id.as_uuid())
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("rollback evidence");
    assert_eq!(
        evidence,
        (
            "WAITING".into(),
            "PENDING".into(),
            "PENDING".into(),
            "WAITING".into(),
            0,
        )
    );
}

#[tokio::test]
async fn cancellation_outbox_and_audit_failures_roll_back_the_whole_run_transition() {
    cancellation_failure_rolls_back(
        "approval-cancel-outbox-rollback",
        "CREATE FUNCTION reject_approval_cancel_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.event_type='approval.cancelled' THEN RAISE EXCEPTION 'injected outbox failure'; \
         END IF; RETURN NEW; END $$; \
         CREATE TRIGGER reject_approval_cancel_outbox BEFORE INSERT ON event_outbox \
         FOR EACH ROW EXECUTE FUNCTION reject_approval_cancel_outbox()",
        "DROP TRIGGER reject_approval_cancel_outbox ON event_outbox; \
         DROP FUNCTION reject_approval_cancel_outbox()",
    )
    .await;
    cancellation_failure_rolls_back(
        "approval-cancel-audit-rollback",
        "CREATE FUNCTION reject_approval_cancel_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action='approval.cancelled' THEN RAISE EXCEPTION 'injected audit failure'; \
         END IF; RETURN NEW; END $$; \
         CREATE TRIGGER reject_approval_cancel_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_approval_cancel_audit()",
        "DROP TRIGGER reject_approval_cancel_audit ON audit_records; \
         DROP FUNCTION reject_approval_cancel_audit()",
    )
    .await;
}
