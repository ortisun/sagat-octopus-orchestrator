//! A collection of 10,000 items read from a blob (T014-004: "test 10,000-item reference
//! materialization separately"), against real PostgreSQL.
//!
//! The source is an array of 10,000 blob references, about 1.9 MiB: well over the 256 KiB a
//! payload may hold and under the 4 MiB the run may retain. The entry, the membership, the
//! admission at the maximum concurrency of 256, the ordered aggregation and the byte ceilings all
//! run for real; only the worker is in process (no gRPC) so that 10,000 completions are bearable.
//!
//! `entry_of_ten_thousand_items_is_one_exact_membership` runs with the suite. The end-to-end run
//! takes minutes and is `#[ignore]`d: `cargo test -p sagat-tests --test integration_workflow_primitives
//! --locked collection_10k -- --ignored --nocapture`. Time and memory go to stderr.

use std::time::Instant;

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::worker::{Capabilities, WorkerService, WorkerSettings};
use sagat_domain::execution::{ExecutionInstanceId, RunId};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::blob_collections::*;
use crate::support::engine::task_ref;
use crate::support::subject;
use crate::support::worker::Platform;

const ITEMS: usize = 10_000;

fn ten_thousand_ir(name: &str) -> Value {
    let mut document = collection_ir(name, "input", 10_000);
    document["nodes"][0]["maxConcurrency"] = json!(256);
    document
}

/// 10,000 references of about 190 bytes each; `v` is the item's own index.
fn references() -> (Vec<Value>, Vec<u8>) {
    let items: Vec<Value> = (0..ITEMS)
        .map(|n| {
            json!({"v": n, "$blob": {
                "id": Uuid::now_v7().to_string(), "size": 1_000_000 + n,
                "sha256": hex(&Sha256::digest(format!("item-{n}").as_bytes())),
                "contentType": "application/pdf"}})
        })
        .collect();
    let bytes = serde_json::to_vec(&items).expect("json");
    (items, bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn raise_iteration_capacity(p: &Platform, limit: u64) {
    let (status, body) = p
        .rest(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": limit})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

/// Resident set of this process in KiB (`ps`), 0 when it cannot be read.
fn rss_kib() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn entry_of_ten_thousand_items_is_one_exact_membership() {
    let p = platform().await;
    raise_iteration_capacity(&p, 256).await;
    let workflow = p
        .publish("coll-10k-entry", &ten_thousand_ir("coll-10k-entry"))
        .await;
    let (items, bytes) = references();
    assert!(bytes.len() > 256 * 1024 && bytes.len() < 4 * 1024 * 1024);
    let blob = upload(&p, &bytes).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;

    let started = Instant::now();
    let report = scheduler.tick().await;
    let entered = started.elapsed();
    assert_eq!(
        (report.collections_advanced, report.failed),
        (1, 0),
        "{report:?}"
    );
    eprintln!(
        "10,000 references ({} KiB): read, materialized and entered in {entered:?}; rss {} MiB",
        bytes.len() / 1024,
        rss_kib() / 1024
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_scope_items item JOIN iteration_scopes scope \
           ON scope.scope_id = item.scope_id WHERE scope.run_id = $1",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count");
    assert_eq!(count, ITEMS as i64);
    let stored: Vec<(i32, String)> = sqlx::query_as(
        "SELECT item.iteration_index, convert_from(payload.content, 'UTF8') \
         FROM collection_scope_items item JOIN payloads payload \
           ON payload.tenant_id = item.tenant_id AND payload.digest = item.item_digest \
         JOIN iteration_scopes scope ON scope.scope_id = item.scope_id \
         WHERE scope.run_id = $1 ORDER BY item.iteration_index",
    )
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("items");
    for (index, content) in stored {
        assert_eq!(
            serde_json::from_str::<Value>(&content).expect("json"),
            items[usize::try_from(index).expect("index")],
            "index {index} holds the element at that position"
        );
    }
    // The source is read once: a second read of the same blob is a different request, not a
    // retry, so a second scope would show up here.
    scheduler.tick().await;
    assert_eq!(scopes(&p, run).await, 1);

    // One more element than maxItems is refused before anything is retained.
    let mut over = items.clone();
    over.push(json!({"v": ITEMS}));
    let over_blob = upload(&p, &serde_json::to_vec(&over).expect("json")).await;
    let over_run = start_with_blob(&p, &workflow, over_blob["id"].as_str().expect("id")).await;
    tick_until_settled(&p, &scheduler, over_run, 6).await;
    assert_eq!(
        status_and_code(&p, over_run).await,
        ("FAILED".to_owned(), Some("ITERATION_LIMIT".to_owned()))
    );
    assert_eq!(scopes(&p, over_run).await, 0);
}

#[tokio::test]
#[ignore = "10,000 iterations end to end take minutes; run explicitly and read stderr"]
async fn ten_thousand_items_are_admitted_concurrently_aggregated_in_order_and_bounded() {
    let p = platform().await;
    raise_iteration_capacity(&p, 256).await;
    let workflow = p
        .publish("coll-10k-run", &ten_thousand_ir("coll-10k-run"))
        .await;
    let (_, bytes) = references();
    let blob = upload(&p, &bytes).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let (scheduler, keeper) = scheduler_with_keeper(&p, Some(blob_source(&p))).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("node"));

    // The worker side: in process, over the platform's own store, protocol 1.5.
    let store: PgRunStore = p.api.executions.clone().with_scoped_worker_protocol();
    let workers = WorkerService::new(
        store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let principal = Principal::new(p.tenant, subject("coll-10k-worker"), Role::Worker);
    p.bind_everything("coll-10k-worker").await;
    let registered = workers
        .register(
            &principal,
            "payments",
            "coll-10k-worker",
            &Capabilities {
                activities: vec!["src.line".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 256,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register");
    let token = registered.session_token.expose().to_owned();

    let started = Instant::now();
    let mut peak_rss = rss_kib();
    let mut peak_active = 0_i64;
    let mut completed = 0_usize;
    let mut advances = 0_usize;
    let mut idle = 0;
    loop {
        // The shard lease is renewed as the engine's keeper would.
        keeper.round().await.expect("renew the shard leases");
        // Drive the collection until it has nothing more to do right now.
        loop {
            match scheduler
                .advance_collection(p.tenant, RunId::new(run), &owner)
                .await
            {
                Ok(sagat_scheduler::CollectionAdvance::Waiting) => break,
                Ok(sagat_scheduler::CollectionAdvance::Finished) => {
                    advances += 1;
                    break;
                }
                Ok(_) => {
                    advances += 1;
                    if advances.is_multiple_of(250) {
                        eprintln!(
                            "  {advances} transitions in {:?}, rss {} MiB",
                            started.elapsed(),
                            rss_kib() / 1024
                        );
                    }
                }
                Err(error) => panic!("advance {advances}: {error}"),
            }
        }
        peak_rss = peak_rss.max(rss_kib());
        let active: i64 = sqlx::query_scalar(
            "SELECT active_count::bigint FROM iteration_scopes WHERE run_id = $1",
        )
        .bind(run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("active");
        peak_active = peak_active.max(active);
        assert!(
            active <= 256,
            "never more than 256 iterations at once: {active}"
        );
        let (status, _) = status_and_code(&p, run).await;
        if status == "SUCCEEDED" {
            break;
        }
        let mut tasks = workers
            .poll(&principal, registered.worker_id, &token, 256, None)
            .await
            .expect("poll");
        if tasks.is_empty() {
            idle += 1;
            assert!(
                idle < 20,
                "no progress: status {status}, completed {completed}"
            );
            continue;
        }
        idle = 0;
        // Reverse order: the aggregate must come out in input order whatever the completion order.
        tasks.reverse();
        for task in tasks {
            let input: Value = serde_json::from_slice(&task.input).expect("task input");
            workers
                .complete(
                    &principal,
                    &task_ref(&task),
                    input["item"]["v"].to_string().as_bytes(),
                    &format!("done-{}", task.step_id),
                )
                .await
                .expect("complete");
            completed += 1;
        }
        if completed % 1000 < 256 {
            eprintln!(
                "{completed} completed, {advances} scheduler transitions, {:?}, rss {} MiB",
                started.elapsed(),
                rss_kib() / 1024
            );
        }
    }
    let elapsed = started.elapsed();
    eprintln!(
        "10,000 iterations: {completed} completions, {advances} collection transitions in \
         {elapsed:?} ({:.1} iterations/s); peak active {peak_active}; peak rss {} MiB",
        completed as f64 / elapsed.as_secs_f64(),
        peak_rss / 1024
    );

    assert_eq!(completed, ITEMS);
    let aggregate = aggregate(&p, run).await;
    let aggregate = aggregate.as_array().expect("an array");
    assert_eq!(aggregate.len(), ITEMS);
    for (index, value) in aggregate.iter().enumerate() {
        assert_eq!(value, &json!(index), "position {index}");
    }
    let admitted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_iterations WHERE run_id = $1 AND lifecycle = 'CHECKPOINTED'",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("iterations");
    assert_eq!(
        admitted, ITEMS as i64,
        "each index was admitted and checkpointed once"
    );
    assert!(peak_active > 1, "the window really ran concurrently");
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("journal and projection agree after 10,000 iterations");
}
