//! Design 043, row 12 under a restart (T043-9, FR-043-009): a scheduler that stops between the check of a continuation's
//! input and the failure of its run leaves the run as it was; the scheduler that comes back checks again and fails the
//! run once. The point is held with a validator that tells the test it was asked and never answers; the stop is the end
//! of the round there (its future is dropped, as a process that stops drops everything it holds).

use std::sync::Arc;
use std::time::Duration;

use sagat_application::definitions::InputValidationMode;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::run_input::{
    PathSegment, Rule, RunInputGate, SchemaCheck, SchemaKey, SchemaRefusal, SchemaValidator,
};
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::workflow::InputSchema;
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::support::engine::{World, world};

/// Tells the test it was asked, then waits for an answer that never comes.
#[derive(Default)]
struct Held {
    asked: Notify,
    answer: Notify,
}

impl SchemaValidator for Held {
    fn check<'a>(&'a self, _: SchemaKey, _: &'a InputSchema, _: &'a [u8]) -> SchemaCheck<'a> {
        Box::pin(async move {
            self.asked.notify_one();
            self.answer.notified().await;
            Err(SchemaRefusal::Invalid {
                segments: vec![PathSegment::Property("segment".into())],
                rule: Rule::Type,
            })
        })
    }
}

type Sched = Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy>;

fn scheduler(w: &World, placement: Placement, gate: Arc<RunInputGate>) -> Sched {
    Scheduler::new(
        w.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_run_inputs(gate)
    .with_children(w.cache.clone())
    .with_placement(placement)
}

async fn placement(w: &World) -> Placement {
    let placement = Placement::new(format!("row-12-restart-{}", Uuid::now_v7()));
    Keeper::new(
        w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    placement
}

async fn count(w: &World, sql: &'static str, run: RunId) -> i64 {
    sqlx::query_scalar(sql)
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn a_restart_between_the_check_of_a_continuation_and_its_failure_fails_the_run_once() {
    let w = world().await;
    let name = format!("row-12-restart-{}", Uuid::now_v7());
    let (workflow, _) = w
        .publish(json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "work",
            "inputSchema": {
                "type": "object",
                "required": ["segment"],
                "properties": {"segment": {"type": "integer"}}
            },
            "nodes": [
                {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
                {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
            ],
            "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
        }))
        .await;
    let chain = w
        .start(workflow, &json!({"segment": 1, "next": {"segment": "two"}}))
        .await;
    let session = w.session(&["segment.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");

    let placement = placement(&w).await;
    let held = Arc::new(Held::default());
    let before = scheduler(
        &w,
        placement.clone(),
        Arc::new(RunInputGate::new(
            Arc::clone(&held) as Arc<dyn SchemaValidator>,
            InputValidationMode::Enforce,
        )),
    );
    // the input is evaluated and its check has begun: the process stops there, before the answer and the failure
    tokio::select! {
        biased;
        () = held.asked.notified() => {}
        _ = before.tick() => panic!("the round ended before the check of the continuation"),
    }
    drop(before);

    assert!(
        !w.run_status(chain).await.is_terminal(),
        "the stopped scheduler wrote nothing"
    );
    let stops =
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'run.stop_requested'";
    assert_eq!(count(&w, stops, chain).await, 0);

    let after = scheduler(
        &w,
        placement,
        sagat_infra_schema::run_input::RunInputValidator::gate(InputValidationMode::Enforce),
    );
    after.tick().await;
    after.tick().await;
    assert_eq!(w.run_status(chain).await, RunStatus::Failed);
    assert_eq!(count(&w, stops, chain).await, 1, "failed once");
    let code: serde_json::Value = sqlx::query_scalar(
        "SELECT detail->'code' FROM run_events WHERE run_id = $1 AND event_type = 'run.stop_requested'",
    )
    .bind(chain.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the stop");
    assert_eq!(code, json!("CONTINUATION_INPUT_INVALID"));
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM chain_segments WHERE chain_id = $1",
            chain
        )
        .await,
        1,
        "no successor"
    );
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM audit_records WHERE action = 'run.finished' AND resource_id = $1::text",
            chain
        )
        .await,
        1,
        "audited once"
    );
}
