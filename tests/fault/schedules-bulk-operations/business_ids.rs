//! A start under a business identifier dies at any write (`T018-036`, `SC-018-006`,
//! `FR-018-018`, `FR-018-021`): the claim of the identifier, the run, its events and its audit are one
//! transaction, so a process killed between two of its statements leaves nothing behind, not a
//! binding without its run and not a run without its binding, and the same start sent again, which
//! is what a client that lost the answer does, makes exactly one run that holds the identifier. The
//! connection of the transaction is killed inside PostgreSQL after each of its writing statements in
//! turn (`tripwire.rs`). Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::runs::{BusinessStart, StartOptions, StartRun};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use uuid::Uuid;

use crate::support::engine::ctx;
use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::{Tripwire, table_digests};

fn under(id: &str) -> StartOptions {
    StartOptions {
        business: Some(BusinessStart {
            id: BusinessId::new(id).expect("an identifier"),
            conflict: ConflictPolicy::Fail,
            reuse: ReusePolicy::AllowDuplicate,
            window: ReuseWindow::default(),
        }),
        ..StartOptions::default()
    }
}

async fn start_under(
    f: &Fixture,
    id: &str,
    key: &str,
) -> Result<sagat_application::runs::Started, sagat_application::error::AppError> {
    f.w.runs
        .start_with_options(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow: f.workflow,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key,
            },
            under(id),
            &ctx(),
        )
        .await
}

async fn held_by(f: &Fixture, id: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT execution_id FROM business_ids WHERE business_id = $1")
        .bind(id)
        .fetch_optional(&f.w.db.pool)
        .await
        .expect("binding")
}

async fn runs_under(f: &Fixture, id: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM runs WHERE business_id = $1 ORDER BY id")
        .bind(id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("runs")
}

#[tokio::test]
async fn a_start_that_dies_at_any_write_leaves_nothing_and_sent_again_makes_one_run() {
    let f = fixture().await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // A first start of a tenant makes writes that later ones do not (its first counters); one is made
    // first so that the probe shows the writes of an ordinary start.
    start_under(&f, "die-warm-up", "die-warm-up-key")
        .await
        .expect("the warm-up start");

    // Which writes does the start make? Watch a normal one.
    wire.watch().await;
    start_under(&f, "die-probe", "die-probe-key")
        .await
        .expect("the probe start");
    wire.disarm().await;
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "a start is one transaction: {writes:?}"
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in ["business_ids", "runs", "run_events", "audit_records"] {
        assert!(
            tables.contains(table),
            "the start is expected to write {table}: {writes:?}"
        );
    }
    println!(
        "a start under an identifier makes {} writing statements in one transaction, on {} tables: {tables:?}",
        writes.len(),
        tables.len()
    );

    // Die after each of those writes, then send the same start again.
    for (_, position, table) in &writes {
        let id = format!("dies-at-{position}");
        let key = format!("dies-at-{position}-key");
        let digests = table_digests(&f.w.db.pool).await;

        wire.arm(*position).await;
        let result = start_under(&f, &id, &key).await;
        wire.disarm().await;

        assert!(
            result.is_err(),
            "cut off at write {position} ({table}), the start answered {result:?}"
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

        // sent again: one run, and it holds the identifier
        let again = start_under(&f, &id, &key)
            .await
            .unwrap_or_else(|error| panic!("write {position} ({table}), sent again: {error:?}"));
        let run = again.run.info.run_id.as_uuid();
        assert_eq!(runs_under(&f, &id).await, [run], "write {position}");
        assert_eq!(held_by(&f, &id).await, Some(run), "write {position}");
        // and a third time is a replay of it, not another run
        let third = start_under(&f, &id, &key)
            .await
            .expect("a replay of the start");
        assert_eq!(third.run.info.run_id.as_uuid(), run, "write {position}");
        assert_eq!(runs_under(&f, &id).await.len(), 1, "write {position}");
    }
}
