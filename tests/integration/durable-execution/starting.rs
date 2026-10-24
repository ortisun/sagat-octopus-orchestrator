//! Starting runs: what is accepted, what is refused, idempotency, listing (FR-002-008, FR-001-002).

use sagat_application::error::AppError;
use sagat_application::identity::PageRequest;
use sagat_application::runs::{RunFilter, StartRun};
use sagat_domain::execution::RunStatus;
use serde_json::json;
use time::{Duration, OffsetDateTime};

use crate::support::engine::{activity, chain, ctx, document, world};

async fn start(
    w: &crate::support::engine::World,
    request: StartRun<'_>,
) -> Result<sagat_application::execution::RunRecord, AppError> {
    w.runs.start(&w.admin, &request, &ctx()).await
}

fn request<'a>(
    wf: sagat_domain::definition::WorkflowId,
    input: &'a [u8],
    key: &'a str,
) -> StartRun<'a> {
    StartRun {
        namespace: "payments",
        workflow: wf,
        version: None,
        input,
        correlation_id: None,
        deadline: None,
        key,
    }
}

#[tokio::test]
async fn a_started_run_records_who_started_it_and_how_to_correlate_it() {
    let w = world().await;
    let (wf, version) = w.publish(chain("recorded")).await;
    let record = start(
        &w,
        StartRun {
            correlation_id: Some("order-77"),
            ..request(wf, br#"{"x":1}"#, "start-key-0001")
        },
    )
    .await
    .expect("start");
    assert_eq!(record.status, RunStatus::Running);
    assert_eq!(record.info.workflow_id, wf);
    assert_eq!(record.info.version_id, version);
    assert_eq!(record.info.started_by, "alice");
    assert_eq!(record.info.correlation_id.as_deref(), Some("order-77"));
    assert_eq!(
        record.info.trace_id.as_deref(),
        Some("0af7651916cd43dd8448eb211c80319c")
    );
    assert!(record.version > 0, "the first events were journaled");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.started' AND resource_id = $1",
    )
    .bind(record.info.run_id.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn a_run_stays_bound_to_the_version_it_started_on() {
    let w = world().await;
    let (wf, v1) = w.publish(chain("pinned")).await;
    let run = w.start(wf, &json!({})).await;

    // The workflow is edited and published again while the run is in flight.
    let record = w
        .defs
        .get_workflow(&w.admin, wf, &ctx())
        .await
        .expect("get");
    let new_doc = serde_json::to_vec(&document("pinned", "x", vec![activity("x")])).expect("json");
    let edited = w
        .defs
        .update_draft(
            &w.admin,
            wf,
            record.workflow.draft_revision(),
            &new_doc,
            &ctx(),
        )
        .await
        .expect("edit");
    let v2 = w
        .defs
        .publish(
            &w.admin,
            wf,
            edited.workflow.draft_revision(),
            None,
            "publish-key-2",
            &ctx(),
        )
        .await
        .expect("v2");
    assert_ne!(v2.version.id(), v1);

    let running = w.runs.get(&w.admin, run, &ctx()).await.expect("get run");
    assert_eq!(
        running.info.version_id, v1,
        "the run did not move to the new version"
    );
    let session = w.session(&["act.a", "act.b", "act.x"]).await;
    assert_eq!(
        w.one_task(&session).await.instance.node().as_str(),
        "a",
        "it keeps executing the graph it started with"
    );
    // New runs use the new active version, unless they pin the old one.
    let fresh = start(&w, request(wf, b"{}", "start-key-0002"))
        .await
        .expect("fresh");
    assert_eq!(fresh.info.version_id, v2.version.id());
    let pinned = start(
        &w,
        StartRun {
            version: Some(v1),
            ..request(wf, b"{}", "start-key-0003")
        },
    )
    .await
    .expect("pinned");
    assert_eq!(pinned.info.version_id, v1);
}

#[tokio::test]
async fn a_workflow_with_no_published_version_cannot_be_started() {
    let w = world().await;
    let bytes = serde_json::to_vec(&chain("draft-only")).expect("json");
    let draft = w
        .defs
        .create_workflow(
            &w.admin,
            "payments",
            "draft-only",
            &bytes,
            "create-key-0001",
            &ctx(),
        )
        .await
        .expect("draft");
    let result = start(&w, request(draft.workflow.id(), b"{}", "start-key-0001")).await;
    assert_eq!(
        result.map(|_| ()),
        Err(AppError::StateConflict(
            "the workflow has no published version"
        ))
    );
}

#[tokio::test]
async fn bad_requests_are_refused_before_anything_is_created() {
    let w = world().await;
    let (wf, _) = w.publish(chain("validated")).await;
    let huge = format!("{{\"blob\":\"{}\"}}", "x".repeat(300 * 1024));
    let long_id = "c".repeat(161);
    let past = OffsetDateTime::now_utc() - Duration::hours(1);
    let far = OffsetDateTime::now_utc() + Duration::days(400);
    for (what, result) in [
        (
            "array input",
            start(&w, request(wf, b"[1]", "start-key-0001")).await,
        ),
        (
            "garbage input",
            start(&w, request(wf, b"not json", "start-key-0002")).await,
        ),
        (
            "oversized input",
            start(&w, request(wf, huge.as_bytes(), "start-key-0003")).await,
        ),
        (
            "long correlation id",
            start(
                &w,
                StartRun {
                    correlation_id: Some(&long_id),
                    ..request(wf, b"{}", "start-key-0004")
                },
            )
            .await,
        ),
        (
            "deadline in the past",
            start(
                &w,
                StartRun {
                    deadline: Some(past),
                    ..request(wf, b"{}", "start-key-0005")
                },
            )
            .await,
        ),
        (
            "deadline beyond a year",
            start(
                &w,
                StartRun {
                    deadline: Some(far),
                    ..request(wf, b"{}", "start-key-0006")
                },
            )
            .await,
        ),
    ] {
        assert!(
            matches!(result, Err(AppError::NotAllowed(_))),
            "{what}: {result:?}"
        );
    }
    assert_eq!(w.runs_of(&RunFilter::default()).await.len(), 0);
    let unknown_namespace = w
        .runs
        .start(
            &w.admin,
            &StartRun {
                namespace: "nowhere",
                ..request(wf, b"{}", "start-key-0007")
            },
            &ctx(),
        )
        .await;
    assert_eq!(unknown_namespace.map(|_| ()), Err(AppError::NotFound));
    let unknown_workflow = start(
        &w,
        request(
            sagat_domain::definition::WorkflowId::generate(),
            b"{}",
            "start-key-0008",
        ),
    )
    .await;
    assert_eq!(unknown_workflow.map(|_| ()), Err(AppError::NotFound));
}

#[tokio::test]
async fn starting_twice_with_one_key_creates_one_run() {
    let w = world().await;
    let (wf, _) = w.publish(chain("idempotent-start")).await;
    let first = start(&w, request(wf, br#"{"n":1}"#, "start-key-0001"))
        .await
        .expect("first");
    let retry = start(&w, request(wf, br#"{"n":1}"#, "start-key-0001"))
        .await
        .expect("retry");
    assert_eq!(first.info.run_id, retry.info.run_id);
    assert_eq!(w.runs_of(&RunFilter::default()).await.len(), 1);
    let other = start(&w, request(wf, br#"{"n":2}"#, "start-key-0001")).await;
    assert_eq!(other.map(|_| ()), Err(AppError::IdempotencyKeyReused));
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'run.started'")
            .fetch_one(&w.db.pool)
            .await
            .expect("audit");
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn sixteen_simultaneous_starts_with_one_key_create_one_run() {
    let w = world().await;
    let (wf, _) = w.publish(chain("start-storm")).await;
    let results = futures_util::future::join_all(
        (0..16).map(|_| start(&w, request(wf, br#"{"n":1}"#, "start-key-0001"))),
    )
    .await;
    let ids: std::collections::BTreeSet<_> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .map(|r| r.info.run_id)
        .collect();
    assert_eq!(ids.len(), 1, "{results:?}");
    assert_eq!(w.runs_of(&RunFilter::default()).await.len(), 1);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE node = 'a'")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs, 1);
}

#[tokio::test]
async fn runs_are_listed_newest_first_with_filters_and_stable_pages() {
    let w = world().await;
    let (wf, _) = w.publish(chain("listing")).await;
    let (other, _) = w.publish(chain("listing-other")).await;
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(w.start(wf, &json!({})).await);
    }
    let foreign = w.start(other, &json!({})).await;
    w.operate(ids[0], sagat_application::runs::RunOperation::Cancel)
        .await
        .expect("cancel");

    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = w
            .runs
            .list(
                &w.admin,
                "payments",
                &RunFilter::default(),
                &PageRequest {
                    limit: Some(2),
                    cursor: cursor.clone(),
                },
            )
            .await
            .expect("page");
        seen.extend(page.items.iter().map(|r| r.info.run_id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut expected = ids.clone();
    expected.push(foreign);
    expected.sort();
    expected.reverse();
    assert_eq!(seen, expected, "newest first, no gaps, no repeats");

    assert_eq!(
        w.runs_of(&RunFilter {
            origin: None,
            workflow: Some(other),
            status: None
        })
        .await,
        [foreign]
    );
    let cancelled = w
        .runs_of(&RunFilter {
            origin: None,
            workflow: None,
            status: Some(RunStatus::Cancelled),
        })
        .await;
    assert_eq!(cancelled, [ids[0]]);
}

#[tokio::test]
async fn the_journal_and_the_runtime_graph_show_what_happened() {
    let w = world().await;
    let (wf, _) = w.publish(chain("observable")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"done": true}))
        .await
        .expect("a");

    let first_page = w
        .runs
        .journal(
            &w.admin,
            run,
            &PageRequest {
                limit: Some(3),
                cursor: None,
            },
            &ctx(),
        )
        .await
        .expect("journal");
    assert_eq!(
        first_page.items.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let next = w
        .runs
        .journal(
            &w.admin,
            run,
            &PageRequest {
                limit: Some(100),
                cursor: first_page.next_cursor,
            },
            &ctx(),
        )
        .await
        .expect("rest");
    assert_eq!(next.items.first().map(|e| e.seq), Some(4));
    assert!(next.next_cursor.is_none());
    let completed = next
        .items
        .iter()
        .find(|e| e.event_type == "step.completed")
        .expect("completion");
    assert_eq!(completed.node.as_deref(), Some("a"));
    assert_eq!(completed.attempt, Some(1));
    assert_eq!(completed.actor, "worker:worker-1");
    assert!(
        completed
            .payload_ref
            .as_deref()
            .is_some_and(|d| d.len() == 64),
        "the output is referenced by digest"
    );

    let graph = w.runs.graph(&w.admin, run, &ctx()).await.expect("graph");
    assert_eq!(graph.definition.len(), 2);
    let statuses: Vec<_> = graph
        .steps
        .iter()
        .map(|s| (s.node.as_str().to_owned(), s.status))
        .collect();
    assert_eq!(
        statuses,
        [
            (
                "a".to_owned(),
                sagat_domain::execution::StepStatus::Succeeded
            ),
            ("b".to_owned(), sagat_domain::execution::StepStatus::Ready)
        ]
    );
}

#[tokio::test]
async fn payloads_are_stored_once_by_content() {
    let w = world().await;
    let (wf, _) = w.publish(chain("deduplicated")).await;
    for _ in 0..3 {
        w.start(wf, &json!({"same": "input"})).await;
    }
    let distinct: i64 = sqlx::query_scalar("SELECT count(*) FROM payloads")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(
        distinct, 1,
        "three runs with the same input share one stored payload"
    );
}

#[tokio::test]
async fn a_step_output_larger_than_the_limit_is_refused() {
    let w = world().await;
    let (wf, _) = w.publish(chain("big-output")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    let huge = format!("{{\"blob\":\"{}\"}}", "x".repeat(300 * 1024));
    let result = w
        .workers
        .complete(
            &session.principal,
            &crate::support::engine::task_ref(&task),
            huge.as_bytes(),
            "big-output-key",
        )
        .await;
    assert!(matches!(result, Err(AppError::NotAllowed(_))), "{result:?}");
    let not_json = w
        .workers
        .complete(
            &session.principal,
            &crate::support::engine::task_ref(&task),
            b"not json",
            "bad-output-key",
        )
        .await;
    assert!(matches!(not_json, Err(AppError::NotAllowed(_))));
    assert_eq!(
        w.step_status(run, "a").await,
        Some(sagat_domain::execution::StepStatus::Leased),
        "the attempt is still open"
    );
}
