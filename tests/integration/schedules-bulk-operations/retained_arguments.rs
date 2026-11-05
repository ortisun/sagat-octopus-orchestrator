//! The arguments of a declared Update are kept only when its declaration opts in with `reapplyOnReset`
//! (`T018-043`, `FR-018-039`): as a content-addressed payload, referenced by the receipt of the Update, charged
//! to `tenant_content_bytes`, and announced by the journal row. An Update that does not opt in writes no
//! payload for its arguments. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::runs::{StartOptions, StartRun, UpdateAtStart};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{RunId, UpdateKey};
use sagat_domain::workflow::ActivityName;
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::business_ids::key;
use crate::firing::{Fixture, fixture};
use crate::support::engine::{DECLARED_OPERATION as OPERATION, ctx, declared_waiter};

const INPUT: &[u8] = br#"{"account":{"balance":10},"k":"s"}"#;

/// The workflow of [`declared_waiter`], with the Update opting in to the retention of its arguments.
fn opting_in(name: &str) -> Value {
    let mut definition = declared_waiter(name);
    definition["state"]["updates"][OPERATION]["reapplyOnReset"] = json!(true);
    definition
}

/// Publishes a workflow whose declaration does (or does not) opt in. The capability is the deployment's;
/// these tests are the deployment that turned it on.
async fn publish(f: &Fixture, opt_in: bool) -> WorkflowId {
    sagat_infra_ir::set_reset_reapply(true);
    let name = format!("retained-arguments-{}", uuid::Uuid::now_v7());
    let definition = if opt_in {
        opting_in(&name)
    } else {
        declared_waiter(&name)
    };
    f.w.publish(definition).await.0
}

fn arguments(allowed: bool, balance: i64) -> DeclaredOperationArguments {
    DeclaredOperationArguments::parse(
        json!({"allowed": allowed, "nextState": {"balance": balance}})
            .to_string()
            .as_bytes(),
    )
    .expect("arguments")
}

/// One Update-with-Start request that creates a run and applies the Update to it.
async fn apply(
    f: &Fixture,
    workflow: WorkflowId,
    business: &str,
    update_key: &str,
    arguments: &DeclaredOperationArguments,
) -> RunId {
    let operation = ActivityName::parse(OPERATION).expect("an operation name");
    let update_key = UpdateKey::parse(update_key).expect("an update key");
    f.w.runs
        .update_with_start(
            &f.w.admin,
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
                business: Some(sagat_application::runs::BusinessStart {
                    id: BusinessId::new(business).expect("an identifier"),
                    conflict: ConflictPolicy::UseExisting,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
                start_delay: None,
            },
            UpdateAtStart {
                operation: &operation,
                update_key: &update_key,
                arguments,
                expected_state_revision: None,
                batch_job: None,
            },
            &ctx(),
        )
        .await
        .expect("started and updated")
        .run
        .info
        .run_id
}

/// What the receipt of the run's one Update points at: the digest of the retained arguments, if any.
async fn retained_digest(f: &Fixture, run: RunId) -> Option<Vec<u8>> {
    sqlx::query_scalar(
        "SELECT arguments_digest FROM operation_update_receipts WHERE resolved_run_id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the receipt of the Update")
}

/// The content of the payload with this digest, if the tenant has one.
async fn payload(f: &Fixture, digest: &[u8]) -> Option<Vec<u8>> {
    sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
        .bind(f.w.admin.tenant_id().as_uuid())
        .bind(digest)
        .fetch_optional(&f.w.db.pool)
        .await
        .expect("the payload")
}

/// What the journal row `state.updated` says about the arguments: `retained`, absent in an older row.
async fn announced(f: &Fixture, run: RunId) -> Option<String> {
    sqlx::query_scalar(
        "SELECT detail->>'retained' FROM run_events WHERE run_id = $1 AND event_type = 'state.updated'",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the journal row of the Update")
}

/// Bytes of content the tenant is charged for.
async fn charged(f: &Fixture) -> i64 {
    sqlx::query_scalar("SELECT content_bytes FROM tenant_retained_usage WHERE tenant_id = $1")
        .bind(f.w.admin.tenant_id().as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the usage")
}

fn digest_of(arguments: &DeclaredOperationArguments) -> Vec<u8> {
    sha2::Sha256::digest(arguments.canonical()).to_vec()
}

#[tokio::test]
async fn an_update_that_opts_in_keeps_its_arguments_and_one_that_does_not_keeps_none() {
    let f = fixture().await;
    let kept = publish(&f, true).await;
    let plain = publish(&f, false).await;

    let args = arguments(true, 7);
    let run = apply(&f, kept, "cart-kept", "update-key-0001", &args).await;
    assert_eq!(
        retained_digest(&f, run).await,
        Some(digest_of(&args)),
        "the receipt points at the arguments"
    );
    assert_eq!(
        payload(&f, &digest_of(&args)).await.as_deref(),
        Some(args.canonical()),
        "the canonical arguments are content of the tenant"
    );
    assert_eq!(announced(&f, run).await.as_deref(), Some("true"));

    let other = arguments(true, 8);
    let unkept = apply(&f, plain, "cart-plain", "update-key-0002", &other).await;
    assert_eq!(retained_digest(&f, unkept).await, None);
    assert_eq!(
        payload(&f, &digest_of(&other)).await,
        None,
        "an Update that does not opt in writes no payload for its arguments"
    );
    assert_eq!(announced(&f, unkept).await.as_deref(), Some("false"));

    f.w.store
        .verify_run(f.w.admin.tenant_id(), run)
        .await
        .expect("the projection equals a replay of the journal");
}

#[tokio::test]
async fn the_retained_arguments_are_charged_to_the_content_of_the_tenant() {
    let f = fixture().await;
    let kept = publish(&f, true).await;
    let plain = publish(&f, false).await;
    // a run of each, so the input they share is already charged when the two are compared
    apply(
        &f,
        plain,
        "warm-plain",
        "update-key-0001",
        &arguments(true, 1),
    )
    .await;
    apply(
        &f,
        kept,
        "warm-kept",
        "update-key-0002",
        &arguments(true, 2),
    )
    .await;

    // two Updates whose state, result and arguments are the same size; only one keeps its arguments
    let before = charged(&f).await;
    apply(&f, plain, "cart-p", "update-key-0003", &arguments(true, 3)).await;
    let without = charged(&f).await - before;
    let before = charged(&f).await;
    let args = arguments(true, 4);
    apply(&f, kept, "cart-k", "update-key-0004", &args).await;
    let with = charged(&f).await - before;

    assert_eq!(
        with - without,
        i64::try_from(args.canonical().len()).expect("small"),
        "the difference is the arguments, once"
    );
}

#[tokio::test]
async fn the_same_arguments_are_one_copy_and_a_rejected_update_keeps_none() {
    let f = fixture().await;
    let kept = publish(&f, true).await;

    // the same arguments applied to two runs are one payload (content addressing)
    let args = arguments(true, 5);
    let first = apply(&f, kept, "same-1", "update-key-0001", &args).await;
    let second = apply(&f, kept, "same-2", "update-key-0002", &args).await;
    assert_eq!(
        retained_digest(&f, first).await,
        retained_digest(&f, second).await
    );
    let copies: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(digest_of(&args))
            .fetch_one(&f.w.db.pool)
            .await
            .expect("count");
    assert_eq!(copies, 1);

    // a validator that says no leaves the run started with a REJECTED receipt and keeps nothing
    let refused = arguments(false, 6);
    let run = apply(&f, kept, "refused", "update-key-0003", &refused).await;
    assert_eq!(retained_digest(&f, run).await, None);
    assert_eq!(payload(&f, &digest_of(&refused)).await, None);
    let updated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'state.updated'",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count");
    assert_eq!(updated, 0, "a rejection is no state change");
}
