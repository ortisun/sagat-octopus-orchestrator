//! An Update-with-Start dies at any write (`T018-041`, `SC-018-008`, `FR-018-025`): the claim of the
//! business identifier, the run, its events, the Update's state, receipt, journal event, audit and outbox
//! notification are one transaction, so a process killed between two of its statements leaves nothing
//! behind (not a run without its Update, not an Update without its run), and the same request sent
//! again, which is what a client that lost the answer does, makes exactly one run with exactly one
//! receipt. The connection of the transaction is killed inside PostgreSQL after each of its writing
//! statements in turn (`tripwire.rs`), for an Update that is applied and for one that is rejected.
//! Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::runs::{
    BusinessStart, StartOptions, StartRun, UpdateAtStart, UpdateStarted,
};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{UpdateKey, UpdateReceiptStatus};
use sagat_domain::workflow::ActivityName;
use serde_json::json;
use uuid::Uuid;

use crate::support::engine::{DECLARED_OPERATION, ctx, declared_waiter};
use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::{Tripwire, table_digests};

const INPUT: &[u8] = br#"{"account":{"balance":10},"k":"s"}"#;

async fn update_with_start(
    f: &Fixture,
    workflow: WorkflowId,
    id: &str,
    key: &str,
    allowed: bool,
) -> Result<UpdateStarted, sagat_application::error::AppError> {
    let operation = ActivityName::parse(DECLARED_OPERATION).expect("an operation");
    let update_key = UpdateKey::parse(&format!("{key}-update")).expect("an update key");
    let arguments = DeclaredOperationArguments::parse(
        json!({"allowed": allowed, "nextState": {"balance": 25}})
            .to_string()
            .as_bytes(),
    )
    .expect("arguments");
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
                key,
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: BusinessId::new(id).expect("an identifier"),
                    conflict: ConflictPolicy::UseExisting,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
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
        .await
}

async fn runs_under(f: &Fixture, id: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM runs WHERE business_id = $1 ORDER BY id")
        .bind(id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("runs")
}

async fn held_by(f: &Fixture, id: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT execution_id FROM business_ids WHERE business_id = $1")
        .bind(id)
        .fetch_optional(&f.w.db.pool)
        .await
        .expect("binding")
}

async fn receipts(f: &Fixture, run: Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT status FROM operation_update_receipts WHERE resolved_run_id = $1")
        .bind(run)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("receipts")
}

/// Kills the transaction after each of its writes in turn, for an Update that `allowed` says is
/// applied or rejected.
async fn dies_at_any_write(allowed: bool, tables_expected: &[&str], status: UpdateReceiptStatus) {
    let f = fixture().await;
    let workflow =
        f.w.publish(declared_waiter(&format!("die-{}", Uuid::now_v7())))
            .await
            .0;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // A first start of a tenant makes writes that later ones do not (its first counters); one is made
    // first so that the probe shows the writes of an ordinary request.
    update_with_start(&f, workflow, "die-warm-up", "die-warm-up-key", allowed)
        .await
        .expect("the warm-up request");

    wire.watch().await;
    update_with_start(&f, workflow, "die-probe", "die-probe-key", allowed)
        .await
        .expect("the probe request");
    wire.disarm().await;
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "an Update-with-Start is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in tables_expected {
        assert!(
            tables.contains(table),
            "the request is expected to write {table}: {writes:?}"
        );
    }
    println!(
        "an Update-with-Start ({status:?}) makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    for (_, position, table) in &writes {
        let id = format!("dies-at-{position}");
        let key = format!("dies-at-{position}-key");
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = update_with_start(&f, workflow, &id, &key, allowed).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the request answered {result:?}"
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "cut off at write {position} ({table}), a table changed"
        );
        assert_eq!(
            held_by(&f, &id).await,
            None,
            "no binding at write {position}"
        );
        assert!(
            runs_under(&f, &id).await.is_empty(),
            "no run at write {position}"
        );

        // sent again: one run, and one receipt of the kind the Update has
        let again = update_with_start(&f, workflow, &id, &key, allowed)
            .await
            .unwrap_or_else(|error| panic!("write {position} ({table}), sent again: {error:?}"));
        let run = again.run.info.run_id.as_uuid();
        assert_eq!(again.update.resolution.receipt().status(), status);
        assert_eq!(runs_under(&f, &id).await, [run], "write {position}");
        assert_eq!(held_by(&f, &id).await, Some(run), "write {position}");
        assert_eq!(receipts(&f, run).await.len(), 1, "write {position}");
        // and a third time is a replay of it, not another run and not another receipt
        let third = update_with_start(&f, workflow, &id, &key, allowed)
            .await
            .expect("a replay of the request");
        assert_eq!(third.run.info.run_id.as_uuid(), run, "write {position}");
        assert!(third.update.resolution.replayed(), "write {position}");
        assert_eq!(runs_under(&f, &id).await.len(), 1, "write {position}");
        assert_eq!(receipts(&f, run).await.len(), 1, "write {position}");
    }
}

#[tokio::test]
async fn an_applied_update_with_start_that_dies_at_any_write_leaves_nothing_and_sent_again_applies_once()
 {
    dies_at_any_write(
        true,
        &[
            "business_ids",
            "runs",
            "run_events",
            "audit_records",
            "operation_update_receipts",
            "workflow_operation_states",
            "event_outbox",
        ],
        UpdateReceiptStatus::Applied,
    )
    .await;
}

#[tokio::test]
async fn a_rejected_update_with_start_that_dies_at_any_write_leaves_nothing_and_sent_again_rejects_once()
 {
    dies_at_any_write(
        false,
        &[
            "business_ids",
            "runs",
            "run_events",
            "audit_records",
            "operation_update_receipts",
        ],
        UpdateReceiptStatus::Rejected,
    )
    .await;
}
