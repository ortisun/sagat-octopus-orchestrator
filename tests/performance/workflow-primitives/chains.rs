//! Continuation chains with many segments: each segment runs one activity, reaches its boundary and
//! the production scheduler hands the chain to a successor run in one transaction.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use uuid::Uuid;

use super::common::{
    Samples, TABLE_HEAD, budget, emit, list_env, machine, platform, postgres_description, resources,
};

const ACTIVITY: &str = "segment.work";

/// `SAGAT_PERF_CHAIN_SHAPE=direct` uses an unconditional hand-off that repeats the same input, so the
/// number of segments is not bounded by the nesting depth of the first input.
pub fn chain_document(name: &str, segments: usize) -> Value {
    if std::env::var("SAGAT_PERF_CHAIN_SHAPE").as_deref() == Ok("direct") {
        return json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "work",
            "nodes": [
                {"id": "work", "type": "activity", "activity": ACTIVITY, "next": "rollover"},
                {"id": "rollover", "type": "continueAsNew", "input": "input"}
            ],
            "chainPolicy": {"maxSegments": segments, "timeoutMs": 86_400_000}
        });
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": ACTIVITY, "next": "again"},
            {"id": "again", "type": "condition", "expression": "input.continue",
             "ifTrue": "rollover", "ifFalse": "done"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": segments, "timeoutMs": 86_400_000}
    })
}

/// The input of the first segment: `segments - 1` hand-offs, the last segment stops. Each hand-off
/// passes the rest of this value on as the successor's input, because the accepted expression subset
/// cannot construct an object.
pub fn nested_input(segments: usize) -> Value {
    if std::env::var("SAGAT_PERF_CHAIN_SHAPE").as_deref() == Ok("direct") {
        return json!({"continue": true});
    }
    let mut value = json!({"continue": false});
    for _ in 1..segments {
        value = json!({"continue": true, "next": value});
    }
    value
}

pub async fn run_case(segments: usize) -> String {
    let p = platform().await;
    let tenant = p.world.admin.tenant_id();
    let name = format!("perf-chain-{}", Uuid::now_v7());
    let (workflow, _) = p.world.publish(chain_document(&name, segments)).await;
    let input = nested_input(segments);
    let input_bytes = serde_json::to_vec(&input).expect("input").len();
    let started_at = Instant::now();
    let chain = match p
        .world
        .start_with(workflow, &input, &format!("chain-{}", Uuid::now_v7()), None)
        .await
    {
        Ok(run) => run,
        Err(error) => {
            return format!(
                "### {segments} segments\n\n- Start REFUSED ({input_bytes} byte nested input): {error:?}. No chain was run.\n"
            );
        }
    };
    let start_latency = started_at.elapsed();
    let header = format!("{}\n{}\n", machine(), postgres_description(&p.world).await);
    let session = p.session(&[ACTIVITY]).await;
    let before = resources(&p.world).await;
    let (mut peak_threads, mut peak_connections) = (before.threads, before.pg_connections);
    let mut samples = Samples::default();
    let mut handed_off = 0_u32;
    let mut sequence = Vec::new();
    let budget = budget();
    let started = Instant::now();
    let mut complete = false;
    let mut final_outcome = String::new();
    let mut idle = 0;
    while started.elapsed() < budget {
        let tasks = p.poll(&session, 8).await;
        for task in &tasks {
            let at = Instant::now();
            p.complete(&session, task, &json!({"processed": true}))
                .await;
            samples.record("worker: complete a segment activity", at.elapsed());
        }
        let at = Instant::now();
        let report = p.scheduler.tick().await;
        let elapsed = at.elapsed();
        assert_eq!(report.failed, 0, "scheduler failure: {report:?}");
        if report.continuations_advanced > 0 {
            samples.record(
                "scheduler tick that handed a chain to its successor",
                elapsed,
            );
            sequence.push(elapsed);
            handed_off += report.continuations_advanced;
            idle = 0;
        } else {
            samples.record("scheduler tick with no hand-off", elapsed);
            if tasks.is_empty() {
                idle += 1;
            }
        }
        if handed_off.is_multiple_of(64) {
            let now = resources(&p.world).await;
            peak_threads = peak_threads.max(now.threads);
            peak_connections = peak_connections.max(now.pg_connections);
        }
        let outcome: Option<String> = sqlx::query_scalar(
            "SELECT final_outcome FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(chain.as_uuid())
        .fetch_one(&p.world.db.pool)
        .await
        .expect("chain outcome");
        if let Some(outcome) = outcome {
            if std::env::var("SAGAT_PERF_CHAIN_SHAPE").as_deref() != Ok("direct") {
                assert_eq!(outcome, "SUCCEEDED");
            }
            final_outcome = outcome;
            complete = true;
            break;
        }
        assert!(idle < 400, "the chain stalled after {handed_off} hand-offs");
    }
    let wall = started.elapsed();
    let after = resources(&p.world).await;
    let (count, runs): (i32, i64) = sqlx::query_as(
        "SELECT c.segment_count, (SELECT count(*) FROM chain_segments s \
            WHERE s.tenant_id=c.tenant_id AND s.chain_id=c.chain_id) \
         FROM execution_chains c WHERE c.tenant_id=$1 AND c.chain_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&p.world.db.pool)
    .await
    .expect("chain size");
    if complete {
        assert_eq!(usize::try_from(count).expect("count"), segments);
    }
    format!(
        "{header}### {segments} segments{partial}\n\n\
         - Start request with the {input_bytes}-byte nested input: {start:.1} ms. Outcome: {done}; wall {wall:.1} s, {handed_off} hand-offs, {sps:.1} segments/s; `segment_count` {count}, {runs} segment rows.\n\
         - Resources before: {before}.\n\
         - Resources after: {after}. Peak threads {threads}, peak PostgreSQL connections {conns}.\n\
         - p50 (ms) of hand-off ticks by tenth of the chain, first to last: {deciles}\n\n\
         {head}{rows}\n",
        partial = if complete {
            ""
        } else {
            " (BUDGET EXHAUSTED, NOT COMPLETED)"
        },
        start = start_latency.as_secs_f64() * 1_000.0,
        done = if complete {
            format!("chain reached final outcome {final_outcome}")
        } else {
            "NOT completed within the time budget".to_owned()
        },
        wall = wall.as_secs_f64(),
        sps = handed_off as f64 / wall.as_secs_f64().max(f64::EPSILON),
        before = before,
        after = after,
        threads = peak_threads.max(after.threads),
        conns = peak_connections.max(after.pg_connections),
        deciles = {
            let d = super::collections::deciles(&sequence);
            if d.is_empty() {
                "n/a".to_owned()
            } else {
                d.iter()
                    .map(|v| format!("{v:.1}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        },
        head = TABLE_HEAD,
        rows = samples.rows(),
    )
}

#[tokio::test]
#[ignore = "capacity benchmark: run explicitly with --ignored"]
async fn measure_chain_segments() {
    for segments in list_env("SAGAT_PERF_CHAIN_SEGMENTS", &[10, 100]) {
        let fragment = run_case(segments).await;
        emit(&format!("chain-{segments}"), &fragment);
    }
    let _ = Duration::ZERO;
}
