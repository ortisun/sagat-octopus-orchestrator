//! Update-with-Start (`T018-041`, `FR-018-025`, `FR-018-026`, `SC-018-008`): a declared Update is applied
//! to the open run of a business identifier, or a run is created and the Update applied to it in the same
//! transaction. A business rejection leaves the run started with a stable `REJECTED` receipt; a failure
//! before admission creates nothing. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::declared_operations::DeclaredOperationStoreError;
use sagat_application::error::AppError;
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::runs::{
    BusinessStart, StartOptions, StartRun, UpdateAtStart, UpdateStarted,
};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{OperationError, RunId, RunStatus, UpdateKey, UpdateReceiptStatus};
use sagat_domain::identity::Role;
use sagat_domain::workflow::ActivityName;
use serde_json::json;

use crate::business_ids::{holder_of, key, runs_of};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{DECLARED_OPERATION as OPERATION, ctx, declared_waiter};

const INPUT: &[u8] = br#"{"account":{"balance":10},"k":"s"}"#;

async fn declared(f: &Fixture) -> WorkflowId {
    f.w.publish(declared_waiter(&format!(
        "update-with-start-{}",
        uuid::Uuid::now_v7()
    )))
    .await
    .0
}

fn business(id: &str, conflict: ConflictPolicy) -> BusinessStart {
    BusinessStart {
        id: BusinessId::new(id).expect("an identifier"),
        conflict,
        reuse: ReusePolicy::AllowDuplicate,
        window: ReuseWindow::default(),
    }
}

fn arguments(allowed: bool, balance: i64) -> DeclaredOperationArguments {
    DeclaredOperationArguments::parse(
        json!({"allowed": allowed, "nextState": {"balance": balance}})
            .to_string()
            .as_bytes(),
    )
    .expect("arguments")
}

/// One Update-with-Start request, the pieces the cases vary.
struct Call<'a> {
    workflow: WorkflowId,
    id: &'a str,
    conflict: ConflictPolicy,
    operation: &'a str,
    update_key: &'a str,
    arguments: DeclaredOperationArguments,
    expected_revision: Option<u64>,
    delay: Option<std::time::Duration>,
    key: String,
}

impl<'a> Call<'a> {
    fn new(workflow: WorkflowId, id: &'a str, balance: i64) -> Self {
        Self {
            workflow,
            id,
            conflict: ConflictPolicy::UseExisting,
            operation: OPERATION,
            update_key: "update-key-0001",
            arguments: arguments(true, balance),
            expected_revision: None,
            delay: None,
            key: key(),
        }
    }

    async fn send(&self, f: &Fixture) -> Result<UpdateStarted, AppError> {
        let operation = ActivityName::parse(self.operation).expect("an operation name");
        let update_key = UpdateKey::parse(self.update_key).expect("an update key");
        f.w.runs
            .update_with_start(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: self.workflow,
                    version: None,
                    input: INPUT,
                    correlation_id: None,
                    deadline: None,
                    key: &self.key,
                },
                StartOptions {
                    business: Some(business(self.id, self.conflict)),
                    start_delay: self.delay,
                },
                UpdateAtStart {
                    operation: &operation,
                    update_key: &update_key,
                    arguments: &self.arguments,
                    expected_state_revision: self.expected_revision,
                },
                &ctx(),
            )
            .await
    }
}

async fn journal(f: &Fixture, run: RunId) -> Vec<String> {
    sqlx::query_scalar("SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY seq")
        .bind(run.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the journal")
}

/// `(state_revision, last_changed_run_version)` of the run's declared state.
async fn state_of(f: &Fixture, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT state_revision, last_changed_run_version FROM workflow_operation_states \
         WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the declared state")
}

/// The statuses of the receipts of the run's Updates.
async fn receipts_of(f: &Fixture, run: RunId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT status FROM operation_update_receipts WHERE resolved_run_id = $1 ORDER BY committed_at",
    )
    .bind(run.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("receipts")
}

/// The audit records written about the run, oldest first.
async fn audits_of(f: &Fixture, run: RunId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT action FROM audit_records WHERE resource_id = $1 ORDER BY created_at",
    )
    .bind(run.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit")
}

/// How many outbox notifications of a state update the run has.
async fn state_notifications(f: &Fixture, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE aggregate_id = $1 AND event_type = 'run.state_updated'",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("outbox")
}

/// Whether the run is still held by its start delay.
async fn is_held(f: &Fixture, run: RunId) -> bool {
    sqlx::query_scalar("SELECT start_delayed_until IS NOT NULL FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the column")
}

fn balance_of(done: &UpdateStarted) -> Option<String> {
    done.update
        .result
        .as_ref()
        .map(|result| String::from_utf8_lossy(result.content()).into_owned())
}

#[tokio::test]
async fn an_absent_identifier_starts_the_run_and_applies_the_update_in_the_same_transaction() {
    let f = fixture().await;
    let workflow = declared(&f).await;

    let done = Call::new(workflow, "acct-1", 25)
        .send(&f)
        .await
        .expect("started and updated");

    assert!(done.started);
    assert_eq!(
        done.update.resolution.receipt().status(),
        UpdateReceiptStatus::Applied
    );
    assert!(!done.update.resolution.replayed());
    assert_eq!(balance_of(&done).as_deref(), Some("25"));
    let run = done.run.info.run_id;
    assert_eq!(holder_of(&f, "acct-1").await, Some(run.as_uuid()));
    assert_eq!(state_of(&f, run).await.0, 1, "the state moved one revision");
    let events = journal(&f, run).await;
    assert_eq!(events.first().map(String::as_str), Some("run.started"));
    assert_eq!(
        events.last().map(String::as_str),
        Some("state.updated"),
        "the Update follows the start in the journal: {events:?}"
    );
    assert_eq!(receipts_of(&f, run).await, ["APPLIED"]);
    let audits = audits_of(&f, run).await;
    assert!(
        audits.iter().any(|a| a == "run.started") && audits.iter().any(|a| a == "run.updated"),
        "the start and the Update are both audited: {audits:?}"
    );
    assert_eq!(
        state_notifications(&f, run).await,
        1,
        "and the Update is notified once, with the start"
    );
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn an_open_holder_gets_the_update_and_nothing_is_started() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let holder = Call::new(workflow, "acct-2", 11)
        .send(&f)
        .await
        .expect("the holder");

    let mut again = Call::new(workflow, "acct-2", 40);
    again.update_key = "update-key-0002";
    let done = again.send(&f).await.expect("updated");

    assert!(!done.started, "it found the holder open");
    assert_eq!(done.run.info.run_id, holder.run.info.run_id);
    assert_eq!(balance_of(&done).as_deref(), Some("40"));
    assert_eq!(runs_of(&f, "acct-2").await.len(), 1, "no second run");
    assert_eq!(state_of(&f, done.run.info.run_id).await.0, 2);
    assert_eq!(
        receipts_of(&f, done.run.info.run_id).await,
        ["APPLIED", "APPLIED"]
    );
}

#[tokio::test]
async fn fail_makes_an_open_holder_a_conflict_and_changes_nothing() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let holder = Call::new(workflow, "acct-3", 11)
        .send(&f)
        .await
        .expect("the holder");
    let before = journal(&f, holder.run.info.run_id).await;

    let mut conflicting = Call::new(workflow, "acct-3", 99);
    conflicting.conflict = ConflictPolicy::Fail;
    conflicting.update_key = "update-key-0003";
    let refused = conflicting.send(&f).await;

    assert!(
        matches!(
            refused,
            Err(AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_IN_USE",
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(journal(&f, holder.run.info.run_id).await, before);
    assert_eq!(state_of(&f, holder.run.info.run_id).await.0, 1);
    assert_eq!(runs_of(&f, "acct-3").await.len(), 1);
}

#[tokio::test]
async fn a_validator_that_says_no_leaves_the_run_started_with_a_stable_rejected_receipt() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let mut call = Call::new(workflow, "acct-4", 25);
    call.arguments = arguments(false, 25);

    let done = call.send(&f).await.expect("started, the update rejected");

    assert!(done.started, "the run was created and stays");
    assert_eq!(
        done.update.resolution.receipt().status(),
        UpdateReceiptStatus::Rejected
    );
    assert!(done.update.result.is_none());
    let run = done.run.info.run_id;
    assert_eq!(holder_of(&f, "acct-4").await, Some(run.as_uuid()));
    assert_eq!(receipts_of(&f, run).await, ["REJECTED"]);
    assert_eq!(state_of(&f, run).await.0, 0, "the state did not move");
    assert!(
        !journal(&f, run).await.iter().any(|e| e == "state.updated"),
        "a rejection writes no state"
    );

    // a retry of the same request answers the same rejection and writes nothing more
    let again = call.send(&f).await.expect("the same request again");
    assert_eq!(again.run.info.run_id, run);
    assert_eq!(
        again.update.resolution.receipt().status(),
        UpdateReceiptStatus::Rejected
    );
    assert!(again.update.resolution.replayed());
    assert_eq!(receipts_of(&f, run).await, ["REJECTED"]);
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn a_stale_expected_revision_is_a_rejection_of_the_same_kind() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let mut call = Call::new(workflow, "acct-5", 25);
    call.expected_revision = Some(7);

    let done = call.send(&f).await.expect("started, the update rejected");

    assert!(done.started);
    let receipt = done.update.resolution.receipt();
    assert_eq!(receipt.status(), UpdateReceiptStatus::Rejected);
    assert_eq!(
        receipt.rejection().map(|rejection| rejection.reason()),
        Some("REVISION_CONFLICT")
    );
    assert_eq!(state_of(&f, done.run.info.run_id).await.0, 0);
}

#[tokio::test]
async fn an_undeclared_operation_creates_nothing() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let runs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    let mut call = Call::new(workflow, "acct-6", 25);
    call.operation = "balance.unknown";

    let refused = call.send(&f).await;

    assert!(
        matches!(refused, Err(AppError::DeclaredUpdate(_))),
        "{refused:?}"
    );
    let runs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs_after, runs_before, "no run was created");
    assert_eq!(holder_of(&f, "acct-6").await, None, "and no binding");
    // the identifier is free: the same request with a declared operation may take it
    let free = Call::new(workflow, "acct-6", 25).send(&f).await;
    assert!(free.is_ok(), "{free:?}");
}

#[tokio::test]
async fn arguments_the_declaration_does_not_accept_create_nothing() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let mut call = Call::new(workflow, "acct-7", 25);
    call.arguments =
        DeclaredOperationArguments::parse(br#"{"allowed": "yes"}"#).expect("well-formed JSON");

    let refused = call.send(&f).await;

    assert_eq!(
        refused.err(),
        Some(AppError::DeclaredUpdate(
            DeclaredOperationStoreError::Operation(OperationError::InvalidArguments)
        ))
    );
    assert_eq!(holder_of(&f, "acct-7").await, None);
    assert!(runs_of(&f, "acct-7").await.is_empty(), "no run was created");
}

#[tokio::test]
async fn the_same_key_replays_the_outcome_and_another_update_under_it_is_another_request() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let call = Call::new(workflow, "acct-8", 25);

    let first = call.send(&f).await.expect("started and updated");
    let replay = call.send(&f).await.expect("a retry of the same request");

    assert_eq!(replay.run.info.run_id, first.run.info.run_id);
    assert!(replay.started, "it says what the first request did");
    assert!(replay.update.resolution.replayed());
    assert_eq!(balance_of(&replay), balance_of(&first));
    assert_eq!(runs_of(&f, "acct-8").await.len(), 1);
    assert_eq!(
        state_of(&f, first.run.info.run_id).await.0,
        1,
        "applied once"
    );

    // every part of the Update is part of the request: its arguments, its key and the revision it expects
    let mut other = Call::new(workflow, "acct-8", 26);
    other.key = call.key.clone();
    let mut other_key = Call::new(workflow, "acct-8", 25);
    other_key.key = call.key.clone();
    other_key.update_key = "update-key-0099";
    let mut other_revision = Call::new(workflow, "acct-8", 25);
    other_revision.key = call.key.clone();
    other_revision.expected_revision = Some(1);
    for (what, another) in [
        ("arguments", other),
        ("update key", other_key),
        ("expected revision", other_revision),
    ] {
        assert_eq!(
            another.send(&f).await.err(),
            Some(AppError::IdempotencyKeyReused),
            "another {what} under the same key"
        );
    }
}

/// An ordinary start of `workflow` under `id` with the body an Update-with-Start would carry.
async fn plain_start(
    f: &Fixture,
    workflow: WorkflowId,
    id: &str,
    key: &str,
) -> Result<sagat_application::runs::Started, AppError> {
    f.w.runs
        .start_with_options(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: INPUT,
                correlation_id: None,
                deadline: None,
                key,
            },
            StartOptions {
                business: Some(business(id, ConflictPolicy::UseExisting)),
                start_delay: None,
            },
            &ctx(),
        )
        .await
}

#[tokio::test]
async fn a_key_used_by_a_plain_start_is_not_an_update_with_start_and_the_other_way_round() {
    let f = fixture().await;
    let workflow = declared(&f).await;

    // an ordinary start came first: an Update-with-Start under its key is another operation, not a
    // retry that would be answered as if it had applied an Update
    let mut call = Call::new(workflow, "acct-11", 25);
    plain_start(&f, workflow, "acct-11", &call.key)
        .await
        .expect("started");
    assert_eq!(
        call.send(&f).await.err(),
        Some(AppError::IdempotencyKeyReused)
    );
    assert_eq!(runs_of(&f, "acct-11").await.len(), 1);

    // and an Update-with-Start came first: a plain start under its key is not a retry of it either
    call = Call::new(workflow, "acct-12", 25);
    call.send(&f).await.expect("started and updated");
    assert_eq!(
        plain_start(&f, workflow, "acct-12", &call.key).await.err(),
        Some(AppError::IdempotencyKeyReused)
    );
    assert_eq!(runs_of(&f, "acct-12").await.len(), 1);
}

#[tokio::test]
async fn a_delayed_run_admits_the_update() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let mut call = Call::new(workflow, "acct-9", 25);
    call.delay = Some(std::time::Duration::from_secs(3_600));

    let done = call.send(&f).await.expect("started held, updated");

    let run = done.run.info.run_id;
    assert_eq!(
        done.update.resolution.receipt().status(),
        UpdateReceiptStatus::Applied
    );
    assert_eq!(f.w.run_status(run).await, RunStatus::Waiting);
    assert!(
        is_held(&f, run).await,
        "the Update did not release the hold"
    );
    assert_eq!(state_of(&f, run).await.0, 1);
    let events = journal(&f, run).await;
    assert!(
        events.iter().any(|e| e == "run.start_delayed"),
        "still held: {events:?}"
    );
    assert!(
        !events.iter().any(|e| e == "run.start_released"),
        "the Update did not release it: {events:?}"
    );
    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn it_needs_the_right_to_start_and_the_right_to_update() {
    let f = fixture().await;
    let workflow = declared(&f).await;
    let viewer = f.w.member("reader", Role::Viewer).await;
    let operation = ActivityName::parse(OPERATION).expect("an operation name");
    let update_key = UpdateKey::parse("update-key-0009").expect("an update key");
    let arguments = arguments(true, 25);

    let refused =
        f.w.runs
            .update_with_start(
                &viewer,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: INPUT,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                StartOptions {
                    business: Some(business("acct-10", ConflictPolicy::UseExisting)),
                    start_delay: None,
                },
                UpdateAtStart {
                    operation: &operation,
                    update_key: &update_key,
                    arguments: &arguments,
                    expected_state_revision: None,
                },
                &ctx(),
            )
            .await;

    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "{refused:?}"
    );
    assert!(runs_of(&f, "acct-10").await.is_empty());
}
