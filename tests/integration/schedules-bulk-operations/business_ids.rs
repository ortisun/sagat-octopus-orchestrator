//! Business identifiers (`T018-034`, `T018-036`, `FR-018-018..021`, `SC-018-006`): at most one open
//! execution holds an identifier of a namespace at any commit, the policies decide what a start does
//! when it is held or remembered, and the binding is claimed in the transaction that creates the
//! run. Real PostgreSQL, the real run service.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::error::AppError;
use sagat_application::runs::{BusinessStart, RunOperation, StartOptions, StartRun, Started};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::execution::RunId;
use serde_json::json;
use uuid::Uuid;

use crate::firing::{Fixture, fixture};
use crate::support::engine::ctx;

pub(crate) fn business(id: &str, conflict: ConflictPolicy, reuse: ReusePolicy) -> StartOptions {
    StartOptions {
        business: Some(BusinessStart {
            id: BusinessId::new(id).expect("an identifier"),
            conflict,
            reuse,
            window: ReuseWindow::default(),
        }),
        ..StartOptions::default()
    }
}

pub(crate) async fn start(
    f: &Fixture,
    options: StartOptions,
    key: &str,
) -> Result<Started, AppError> {
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
            options,
            &ctx(),
        )
        .await
}

pub(crate) fn key() -> String {
    format!("business-{}", Uuid::now_v7())
}

pub(crate) async fn runs_of(f: &Fixture, id: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM runs WHERE business_id = $1 ORDER BY id")
        .bind(id)
        .fetch_all(&f.w.db.pool)
        .await
        .expect("runs")
}

pub(crate) async fn holder_of(f: &Fixture, id: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT execution_id FROM business_ids WHERE business_id = $1")
        .bind(id)
        .fetch_optional(&f.w.db.pool)
        .await
        .expect("binding")
}

/// Ends the run: `a` and `b` succeed (the run `SUCCEEDED`).
async fn succeed(f: &Fixture, run: RunId) {
    let session = f.w.session(&["act.a", "act.b"]).await;
    for activity in ["act.a", "act.b"] {
        let task = f.w.one_task_of(&session, activity).await;
        f.w.complete(&session, &task, &json!({}))
            .await
            .expect("complete");
    }
    assert_eq!(
        f.w.run_status(run).await,
        sagat_domain::execution::RunStatus::Succeeded
    );
}

/// Ends the run without success.
pub(crate) async fn cancel(f: &Fixture, run: RunId) {
    f.w.operate_run(run, RunOperation::Cancel).await;
    assert!(f.w.run_status(run).await.is_terminal());
}

#[tokio::test]
async fn a_free_identifier_is_bound_to_the_run_that_carries_it_and_a_second_start_is_refused_naming_the_holder()
 {
    let f = fixture().await;
    let first = start(
        &f,
        business(
            "order-1001",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the first start");
    assert!(!first.existing);
    let holder = first.run.info.run_id;
    assert_eq!(holder_of(&f, "order-1001").await, Some(holder.as_uuid()));
    assert_eq!(runs_of(&f, "order-1001").await, [holder.as_uuid()]);

    let refused = start(
        &f,
        business(
            "order-1001",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect_err("the identifier is held");
    assert_eq!(
        refused,
        AppError::BusinessIdConflict {
            reason: "BUSINESS_ID_IN_USE",
            holder,
            reusable_after: None
        }
    );
    assert_eq!(
        runs_of(&f, "order-1001").await.len(),
        1,
        "the refused start created nothing"
    );
}

#[tokio::test]
async fn a_binding_whose_holder_is_gone_holds_nothing() {
    // a holder purged by a retention policy leaves its binding behind: it reads as free
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO business_ids \
         (tenant_id, namespace_id, business_id, execution_id, workflow_id, bound_at, reuse_window_s) \
         VALUES ($1, $2, 'orphan-1', $3, $4, now(), 2592000)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(Uuid::now_v7())
    .bind(f.workflow.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a binding of a run that does not exist");
    let started = start(
        &f,
        business(
            "orphan-1",
            ConflictPolicy::Fail,
            ReusePolicy::RejectDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the identifier is free");
    assert_eq!(
        holder_of(&f, "orphan-1").await,
        Some(started.run.info.run_id.as_uuid())
    );
}

#[tokio::test]
async fn use_existing_returns_the_open_holder_and_creates_nothing() {
    let f = fixture().await;
    let held = start(
        &f,
        business(
            "invoice-7",
            ConflictPolicy::Fail,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the first start");
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");

    let again = start(
        &f,
        business(
            "invoice-7",
            ConflictPolicy::UseExisting,
            ReusePolicy::AllowDuplicate,
        ),
        &key(),
    )
    .await
    .expect("the holder is returned");
    assert!(again.existing, "it says the run already existed");
    assert_eq!(again.run.info.run_id, held.run.info.run_id);
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(before, after, "no run was created");
}

#[tokio::test]
async fn a_start_without_an_identifier_is_not_affected() {
    let f = fixture().await;
    let a = start(&f, StartOptions::default(), &key()).await.expect("a");
    let b = start(&f, StartOptions::default(), &key()).await.expect("b");
    assert_ne!(a.run.info.run_id, b.run.info.run_id);
    let bound: i64 = sqlx::query_scalar("SELECT count(*) FROM business_ids")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(bound, 0);
    assert!(a.run.info.run_id != b.run.info.run_id);
}

#[tokio::test]
async fn the_reuse_policy_decides_what_a_closed_holder_allows_inside_its_window() {
    // (the status the holder ends in, the policy, whether a new run is allowed)
    for (ends, policy, allowed) in [
        ("succeeds", ReusePolicy::AllowDuplicate, true),
        ("succeeds", ReusePolicy::AllowDuplicateFailedOnly, false),
        ("succeeds", ReusePolicy::RejectDuplicate, false),
        ("is cancelled", ReusePolicy::AllowDuplicate, true),
        ("is cancelled", ReusePolicy::AllowDuplicateFailedOnly, true),
        ("is cancelled", ReusePolicy::RejectDuplicate, false),
    ] {
        let f = fixture().await;
        let id = "customer-42";
        let first = start(
            &f,
            business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
            &key(),
        )
        .await
        .expect("the first start");
        let holder = first.run.info.run_id;
        if ends == "succeeds" {
            succeed(&f, holder).await;
        } else {
            cancel(&f, holder).await;
        }

        let second = start(&f, business(id, ConflictPolicy::Fail, policy), &key()).await;
        let what = format!("a holder that {ends}, policy {}", policy.as_str());
        if allowed {
            let second = second.unwrap_or_else(|error| panic!("{what}: {error:?}"));
            assert_ne!(second.run.info.run_id, holder, "{what}");
            assert_eq!(
                holder_of(&f, id).await,
                Some(second.run.info.run_id.as_uuid()),
                "{what}: the binding follows the new run"
            );
            assert_eq!(runs_of(&f, id).await.len(), 2, "{what}");
        } else {
            match second {
                Err(AppError::BusinessIdConflict {
                    reason,
                    holder: named,
                    reusable_after,
                }) => {
                    assert_eq!(reason, "BUSINESS_ID_REUSE_REJECTED", "{what}");
                    assert_eq!(named, holder, "{what}");
                    assert!(reusable_after.is_some(), "{what}: says until when");
                }
                other => panic!("{what}: {other:?}"),
            }
            assert_eq!(holder_of(&f, id).await, Some(holder.as_uuid()), "{what}");
        }
    }
}

#[tokio::test]
async fn after_the_window_the_identifier_is_free_whatever_the_policy() {
    let f = fixture().await;
    let id = "ticket-9";
    let first = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::RejectDuplicate),
        &key(),
    )
    .await
    .expect("the first start");
    succeed(&f, first.run.info.run_id).await;
    // inside the window: refused
    let refused = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::RejectDuplicate),
        &key(),
    )
    .await;
    assert!(matches!(refused, Err(AppError::BusinessIdConflict { .. })));
    // the run finished longer ago than the window it was bound with
    sqlx::query("UPDATE business_ids SET reuse_window_s = 60 WHERE business_id = $1")
        .bind(id)
        .execute(&f.w.db.pool)
        .await
        .expect("a short window");
    sqlx::query(
        "UPDATE runs SET finished_at = clock_timestamp() - interval '2 minutes' WHERE id = $1",
    )
    .bind(first.run.info.run_id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("an old finish");
    let again = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::RejectDuplicate),
        &key(),
    )
    .await
    .expect("free again");
    assert_ne!(again.run.info.run_id, first.run.info.run_id);
}

#[tokio::test]
async fn thirty_two_starts_of_one_identifier_give_one_open_run() {
    let f = fixture().await;
    let id = "race-1";
    let w = Arc::clone(&f.w);
    let workflow = f.workflow;
    let mut tasks = Vec::new();
    for n in 0..32 {
        let w = Arc::clone(&w);
        tasks.push(tokio::spawn(async move {
            w.runs
                .start_with_options(
                    &w.admin,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: &format!("race-start-{n}-{}", Uuid::now_v7()),
                    },
                    business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                    &ctx(),
                )
                .await
        }));
    }
    let (mut created, mut refused) = (0, 0);
    for task in tasks {
        match task.await.expect("task") {
            Ok(started) => {
                assert!(!started.existing);
                created += 1;
            }
            Err(AppError::BusinessIdConflict { reason, .. }) => {
                assert_eq!(reason, "BUSINESS_ID_IN_USE");
                refused += 1;
            }
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((created, refused), (1, 31));
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE business_id = $1 AND finished_at IS NULL",
    )
    .bind(id)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("open runs");
    assert_eq!(open, 1, "one open run holds the identifier");
}

#[tokio::test]
async fn thirty_two_starts_over_the_binding_of_a_closed_holder_give_one_new_run() {
    // the binding exists and its holder has closed: every start reads the same closed holder and
    // all of them want to take the binding over. Only the lock on the row makes the others read the
    // new holder, which is open.
    let f = fixture().await;
    let id = "race-3";
    let first = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("first");
    cancel(&f, first.run.info.run_id).await;
    let w = Arc::clone(&f.w);
    let workflow = f.workflow;
    let mut tasks = Vec::new();
    for n in 0..32 {
        let w = Arc::clone(&w);
        tasks.push(tokio::spawn(async move {
            w.runs
                .start_with_options(
                    &w.admin,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: &format!("race-over-{n}-{}", Uuid::now_v7()),
                    },
                    business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                    &ctx(),
                )
                .await
        }));
    }
    let (mut created, mut refused) = (0, 0);
    for task in tasks {
        match task.await.expect("task") {
            Ok(_) => created += 1,
            Err(AppError::BusinessIdConflict { .. }) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((created, refused), (1, 31));
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE business_id = $1 AND finished_at IS NULL",
    )
    .bind(id)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("open runs");
    assert_eq!(open, 1, "one open run holds the identifier");
}

#[tokio::test]
async fn thirty_two_starts_with_use_existing_all_get_the_one_run() {
    let f = fixture().await;
    let id = "race-2";
    let w = Arc::clone(&f.w);
    let workflow = f.workflow;
    let mut tasks = Vec::new();
    for n in 0..32 {
        let w = Arc::clone(&w);
        tasks.push(tokio::spawn(async move {
            w.runs
                .start_with_options(
                    &w.admin,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: &format!("race-use-{n}-{}", Uuid::now_v7()),
                    },
                    business(id, ConflictPolicy::UseExisting, ReusePolicy::AllowDuplicate),
                    &ctx(),
                )
                .await
                .expect("a start")
        }));
    }
    let mut answers = Vec::new();
    for task in tasks {
        answers.push(task.await.expect("task"));
    }
    let runs: std::collections::BTreeSet<_> = answers.iter().map(|a| a.run.info.run_id).collect();
    assert_eq!(runs.len(), 1, "everyone is given the same run");
    assert_eq!(
        answers.iter().filter(|a| !a.existing).count(),
        1,
        "and one of them created it"
    );
    assert_eq!(runs_of(&f, id).await.len(), 1);
}

#[tokio::test]
async fn a_retry_with_the_same_key_gets_the_same_run_and_another_identifier_under_that_key_is_a_different_request()
 {
    let f = fixture().await;
    let same = key();
    let first = start(
        &f,
        business("retry-1", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &same,
    )
    .await
    .expect("first");
    let retried = start(
        &f,
        business("retry-1", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &same,
    )
    .await
    .expect("a retry is answered with the first run");
    assert_eq!(retried.run.info.run_id, first.run.info.run_id);
    assert!(
        !retried.existing,
        "it is a replay of the creation, not a holder"
    );

    let other = start(
        &f,
        business("retry-2", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &same,
    )
    .await;
    assert_eq!(other.err(), Some(AppError::IdempotencyKeyReused));
    assert!(
        holder_of(&f, "retry-2").await.is_none(),
        "and it bound nothing"
    );
}

#[tokio::test]
async fn the_same_identifier_is_independent_across_namespaces_and_tenants() {
    let f = fixture().await;
    let id = "shared-name";
    let here = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("here");
    // the second tenant
    let workflow =
        f.w.publish_as_other_tenant(crate::support::engine::chain("shared-business"))
            .await;
    let there =
        f.w.runs
            .start_with_options(
                &f.w.other_admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: b"{}",
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &ctx(),
            )
            .await
            .expect("the other tenant binds the same text");
    assert_ne!(here.run.info.run_id, there.run.info.run_id);
    let bound: i64 = sqlx::query_scalar("SELECT count(*) FROM business_ids WHERE business_id = $1")
        .bind(id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(bound, 2, "one binding for each tenant");
}

#[tokio::test]
async fn the_identifier_is_charged_to_the_tenant_once_in_the_binding_and_once_on_the_run() {
    let f = fixture().await;
    let usage = |f: &Fixture| {
        let tenant = f.w.admin.tenant_id().as_uuid();
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT coalesce(content_bytes, 0)::bigint FROM tenant_retained_usage WHERE tenant_id = $1",
            )
            .bind(tenant)
            .fetch_optional(&pool)
            .await
            .expect("usage")
            .unwrap_or(0)
        }
    };
    let plain = start(&f, StartOptions::default(), &key())
        .await
        .expect("a start without");
    let _ = plain;
    let before = usage(&f).await;
    let id = "x".repeat(100);
    start(
        &f,
        business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a start with a hundred bytes of identifier");
    let after = usage(&f).await;
    let plain_start = {
        let again = usage(&f).await;
        start(&f, StartOptions::default(), &key())
            .await
            .expect("again");
        usage(&f).await - again
    };
    let with_identifier = after - before;
    assert!(
        with_identifier - plain_start >= 200,
        "the identifier is charged twice (binding and copy): a start with one added {with_identifier} bytes, one without {plain_start}"
    );
}

/// A scheduler that holds every shard: the handoff of a chain is its work. One placement for the whole
/// test, or the lease of the first tick keeps the shards from the next.
pub(crate) async fn scheduler_of(f: &Fixture) -> crate::support::engine::Sched {
    let placement = sagat_scheduler::Placement::new(format!("business-chain-{}", Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire the shards");
    crate::support::scale::scheduler(
        &f.w,
        sagat_scheduler::SchedulerSettings::default(),
        Some(placement),
    )
}

#[tokio::test]
async fn a_chain_holds_the_identifier_until_it_ends_and_every_segment_carries_it() {
    let f = fixture().await;
    let id = "chain-1";
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": format!("business-chain-{}", Uuid::now_v7())},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    let (workflow, _) = f.w.publish(document).await;
    let started =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: br#"{"next": {"next": {"segment": 3}}}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &ctx(),
            )
            .await
            .expect("the chain starts under the identifier");
    let first = started.run.info.run_id;
    let scheduler = scheduler_of(&f).await;
    let session = f.w.session(&["segment.work"]).await;
    let ended = || async {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT final_outcome FROM execution_chains WHERE chain_id = $1",
        )
        .bind(first.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the chain")
    };

    // each segment in turn: the first one is already SUCCEEDED when the second exists, and the
    // identifier is still held, by the chain's initial run
    for segment in 1..=3 {
        let tasks = f.w.poll(&session, 8).await;
        let steps: Vec<(Uuid, String, String)> =
            sqlx::query_as("SELECT run_id, node, status FROM steps ORDER BY updated_at")
                .fetch_all(&f.w.db.pool)
                .await
                .expect("steps");
        assert_eq!(tasks.len(), 1, "segment {segment}: {steps:?}");
        let task = tasks.into_iter().next().expect("a task");
        f.w.complete(&session, &task, &json!({}))
            .await
            .expect("the segment works");
        // the handoff may take more than one look: the boundary waits for open work to settle
        for _ in 0..6 {
            let report = scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            let moved: i32 = sqlx::query_scalar(
                "SELECT segment_count FROM execution_chains WHERE chain_id = $1",
            )
            .bind(first.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("segment count");
            if moved > segment || ended().await.is_some() {
                break;
            }
        }
        if segment < 3 {
            assert_eq!(ended().await, None, "segment {segment}: the chain is open");
            let refused = start(
                &f,
                business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &key(),
            )
            .await;
            assert_eq!(
                refused.err(),
                Some(AppError::BusinessIdConflict {
                    reason: "BUSINESS_ID_IN_USE",
                    holder: first,
                    reusable_after: None
                }),
                "segment {segment}: the first segment has ended, the chain has not"
            );
            assert_eq!(
                holder_of(&f, id).await,
                Some(first.as_uuid()),
                "the chain's initial run keeps the binding"
            );
            // the lookup says the same: held by the chain, whose current run is the segment now
            let binding = crate::business_id_lookup::lookup(&f, id)
                .await
                .expect("the binding of the chain");
            let head: Uuid = sqlx::query_scalar(
                "SELECT current_run_id FROM execution_chains WHERE chain_id = $1",
            )
            .bind(first.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the head");
            assert!(binding.open, "segment {segment}");
            assert_eq!(binding.execution_id, first);
            assert_eq!(binding.current_run_id.as_uuid(), head);
            assert_eq!(binding.last_status, None);
        }
    }
    assert!(
        ended().await.is_some(),
        "the chain reached its limit and ended"
    );
    let remembered = crate::business_id_lookup::lookup(&f, id)
        .await
        .expect("the chain that ended is remembered");
    assert!(!remembered.open);
    assert_eq!(remembered.execution_id, first);
    assert_eq!(
        remembered
            .last_status
            .map(|status| status.as_str().to_owned()),
        ended().await,
        "it says how the chain ended"
    );
    assert!(remembered.reusable_after.is_some());
    // every segment of the chain says what it was started under
    let segments: Vec<Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM chain_segments WHERE chain_id = $1 ORDER BY segment_index",
    )
    .bind(first.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("segments");
    assert_eq!(segments.len(), 3);
    assert_eq!(
        runs_of(&f, id).await.len(),
        3,
        "all three carry the identifier"
    );
    // and now it is free
    let again = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the chain ended, the identifier is free");
    assert_ne!(again.run.info.run_id, first);
}

/// Whether anybody holds the row of a binding: a probe that asks for it with a short wait.
async fn row_is_free(f: &Fixture, id: &str) -> bool {
    let mut probe = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SET LOCAL lock_timeout = '300ms'")
        .execute(&mut *probe)
        .await
        .expect("a short wait");
    let free = sqlx::query("SELECT 1 FROM business_ids WHERE business_id = $1 FOR UPDATE")
        .bind(id)
        .fetch_all(&mut *probe)
        .await
        .is_ok();
    drop(probe);
    free
}

#[tokio::test]
async fn a_refused_start_does_not_keep_the_row_of_the_binding() {
    let f = fixture().await;
    start(
        &f,
        business("held-1", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("first");
    for policy in [ConflictPolicy::Fail, ConflictPolicy::UseExisting] {
        start(
            &f,
            business("held-1", policy, ReusePolicy::AllowDuplicate),
            &key(),
        )
        .await
        .map(|_| ())
        .unwrap_or(());
        assert!(
            row_is_free(&f, "held-1").await,
            "after a start that {} the row is free",
            policy.as_str()
        );
    }
}

#[tokio::test]
async fn a_start_the_admission_refuses_after_the_claim_leaves_no_binding_behind() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("no room for runs");
    let refused = start(
        &f,
        business("quota-1", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect_err("no room");
    assert!(matches!(refused, AppError::QuotaExceeded(_)), "{refused:?}");
    assert!(
        holder_of(&f, "quota-1").await.is_none(),
        "the claim rolled back with the refused run"
    );
    assert!(runs_of(&f, "quota-1").await.is_empty());
}

/// The stage limit of the races below: a step that has to wait for a lock never finishes.
const STAGE: std::time::Duration = std::time::Duration::from_secs(60);

#[tokio::test]
async fn a_start_beside_the_end_of_its_holder_does_not_wait_for_it_and_sees_it_open_until_it_commits()
 {
    let f = fixture().await;
    let id = "terminal-race-1";
    let holder = start(&f, held_by(id), &key())
        .await
        .expect("the holder")
        .run
        .info
        .run_id;

    // the end of the holder is in flight: written, not yet committed
    let mut ending = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query(
        "UPDATE runs SET status = 'CANCELLED', finished_at = clock_timestamp() WHERE id = $1",
    )
    .bind(holder.as_uuid())
    .execute(&mut *ending)
    .await
    .expect("the end, uncommitted");

    // a start does not wait for it (it reads the holder, it does not lock it) and sees it open
    let refused = crate::support::bounded::within(
        "a start beside the end of its holder",
        STAGE,
        start(&f, held_by(id), &key()),
    )
    .await
    .expect_err("the holder has not ended as far as anybody can see");
    assert_eq!(
        refused,
        AppError::BusinessIdConflict {
            reason: "BUSINESS_ID_IN_USE",
            holder,
            reusable_after: None
        }
    );

    // once the end commits, the next start takes the identifier over
    ending.commit().await.expect("the end commits");
    let next = start(&f, held_by(id), &key())
        .await
        .expect("the holder ended")
        .run
        .info
        .run_id;
    assert_ne!(next, holder);
    assert_eq!(holder_of(&f, id).await, Some(next.as_uuid()));
    assert_eq!(runs_of(&f, id).await.len(), 2);
}

#[tokio::test]
async fn a_claim_in_flight_blocks_no_transition_of_the_run_that_holds_the_identifier() {
    let f = fixture().await;
    let id = "terminal-race-2";
    let holder = start(&f, held_by(id), &key())
        .await
        .expect("the holder")
        .run
        .info
        .run_id;

    // a claim in flight: the row of the binding is locked, as the claim of a start does
    let mut claiming = f.w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT 1 FROM business_ids WHERE business_id = $1 FOR UPDATE")
        .bind(id)
        .fetch_all(&mut *claiming)
        .await
        .expect("the row of the binding");

    // the holder goes through its end meanwhile: a transition of a run touches no binding
    crate::support::bounded::within("the end of the holder", STAGE, cancel(&f, holder)).await;

    claiming.rollback().await.expect("release the row");
    lookup_closed(&f, id).await;
}

fn held_by(id: &str) -> StartOptions {
    business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate)
}

async fn lookup_closed(f: &Fixture, id: &str) {
    let binding = crate::business_id_lookup::lookup(f, id)
        .await
        .expect("the binding");
    assert!(!binding.open, "the holder ended");
}
