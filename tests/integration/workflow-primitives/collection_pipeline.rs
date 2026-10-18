//! A public activity produces the immutable inline/blob membership of a later collection.
use sagat_domain::execution::RunId;
use serde_json::{Value, json};

use crate::support::blob_collections::*;

fn pipeline(name: &str) -> Value {
    let mut document = collection_ir(name, "steps.fetch.output", 3);
    document["entrypoint"] = json!("fetch");
    // The root producer and the body activity deliberately share their definition-node id.
    document["bodies"]["line"]["entrypoint"] = json!("fetch");
    document["bodies"]["line"]["nodes"][0]["id"] = json!("fetch");
    document["bodies"]["line"]["nodes"][1]["result"] = json!("steps.fetch.output");
    document["limits"] = json!({"maxInstances": 16, "maxDepth": 1});
    document["nodes"]
        .as_array_mut()
        .expect("root nodes")
        .insert(
            0,
            json!({"id": "fetch", "type": "activity", "activity": "src.fetch", "next": "items"}),
        );
    document
}

#[tokio::test]
async fn a_public_activity_can_produce_inline_collection_membership() {
    assert_pipeline_source(false, false).await;
}

#[tokio::test]
async fn a_scoped_grpc_blob_upload_needs_no_unrelated_iteration_history() {
    let p = platform().await;
    let workflow = p
        .publish(
            "bounded-scoped-blob-upload",
            &collection_ir("bounded-scoped-blob-upload", "input.items", 1),
        )
        .await;
    let run = start_inline(&p, &workflow, &json!({"items":[{"v":1}]})).await;
    let scheduler = scheduler(&p, None).await;
    let mut worker = scoped_worker(&p, "bounded-scoped-blob-worker", &["src.line"]).await;
    let mut leased = None;
    for _ in 0..8 {
        scheduler.tick().await;
        if let Some(task) = worker.poll(1).await.pop() {
            leased = Some(task);
            break;
        }
    }
    let task = leased.expect("one admitted scoped activity");
    assert!(
        task.instance
            .as_ref()
            .is_some_and(|id| !id.instance_path.is_empty())
    );
    sqlx::query("REVOKE SELECT ON iteration_checkpoints FROM sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("deny full-history restoration");
    let bytes = br#"{"v":1}"#;
    let header = sagat_api_grpc::pb::WriteBlobHeader {
        content_type: "application/json".into(),
        expected_size: bytes.len() as i64,
        expected_sha256: sha(bytes),
        ..Default::default()
    };
    let uploaded = worker
        .write_blob(&task, header.clone(), bytes, 4)
        .await
        .expect("public scoped upload must authorize only the current exact path");
    assert_eq!(uploaded.size, bytes.len() as i64);
    assert_eq!(uploaded.sha256, sha(bytes));
    let mut forged = task.clone();
    forged.generation += 1;
    forged.task_id = format!(
        "{}/{}",
        task.task_id.rsplit_once('/').expect("task generation").0,
        forged.generation
    );
    let refused = worker
        .write_blob(&forged, header, bytes, 4)
        .await
        .expect_err("skipping the managed-root probe must not grant a forged capability");
    assert_eq!(refused.code(), tonic::Code::Aborted);
    assert_eq!(
        crate::support::worker::machine_code(&refused),
        "STALE_ATTEMPT"
    );
    let objects: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs WHERE tenant_id=$1")
        .bind(p.tenant.as_uuid())
        .fetch_one(&p.api.db.pool)
        .await
        .expect("object reservations");
    assert_eq!(objects, 1, "forged upload reserves no second object");
    sqlx::query("GRANT SELECT ON iteration_checkpoints TO sagat_app")
        .execute(&p.api.db.pool)
        .await
        .expect("restore replay oracle");
    // This test isolates upload authorization; scoped_blob_completion covers its report transaction.
    worker
        .complete(&task, &json!({"v":1}))
        .await
        .expect("inline completion");
    tick_until_settled(&p, &scheduler, run, 20).await;
    assert_eq!(status_and_code(&p, run).await.0, "SUCCEEDED");
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("full replay");
}

#[tokio::test]
async fn a_public_activity_can_commit_a_blob_as_its_collection_source() {
    assert_pipeline_source(true, false).await;
}

#[tokio::test]
async fn a_public_pipeline_refuses_an_uncommitted_source_blob() {
    assert_pipeline_source(true, true).await;
}

async fn assert_pipeline_source(blob: bool, uncommitted: bool) {
    let p = platform().await;
    let workflow = p
        .publish(
            "public-source-pipeline",
            &pipeline("public-source-pipeline"),
        )
        .await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    scheduler.tick().await;
    assert_eq!(
        scopes(&p, run).await,
        0,
        "a collection cannot enter before its source completes"
    );
    let mut worker = p.worker("public-source-producer").await;
    worker
        .register_version(&["src.fetch", "src.line"], if blob { "1.5" } else { "1.4" })
        .await;
    let task = worker.poll(1).await.pop().expect("root source task");
    let array = json!([{"v": 3}, {"v": 1}, {"v": 3}]);
    if blob {
        let bytes = serde_json::to_vec(&array).expect("source JSON");
        let uploaded = worker
            .write_blob(
                &task,
                sagat_api_grpc::pb::WriteBlobHeader {
                    content_type: "application/json".into(),
                    expected_size: bytes.len() as i64,
                    expected_sha256: sha(&bytes),
                    ..Default::default()
                },
                &bytes,
                16,
            )
            .await
            .expect("stream the source blob under the actual root lease");
        if uncommitted {
            worker
                .complete(
                    &task,
                    &json!({"$blob": {
                        "id": uploaded.blob_id, "size": uploaded.size,
                        "sha256": uploaded.sha256, "contentType": uploaded.content_type
                    }}),
                )
                .await
                .expect("an inline descriptor does not commit its uploaded object");
        } else {
            let first = worker
                .complete_with_blob(&task, &uploaded.blob_id)
                .await
                .expect("commit source output");
            assert!(!first.already_applied);
            let replay = worker
                .complete_with_blob(&task, &uploaded.blob_id)
                .await
                .expect("replay committed source");
            assert!(replay.already_applied);
        }
    } else {
        worker
            .complete(&task, &array)
            .await
            .expect("commit inline source");
    }
    if uncommitted {
        tick_until_settled(&p, &scheduler, run, 20).await;
        let (status, code) = status_and_code(&p, run).await;
        assert_eq!(
            (status.as_str(), code.as_deref()),
            ("FAILED", Some("BLOB_UNAVAILABLE"))
        );
        assert_eq!(
            scopes(&p, run).await,
            0,
            "no membership is captured from an unauthorized object"
        );
    } else {
        scheduler.tick().await;
        scheduler.tick().await;
        if !blob {
            assert!(
                worker.poll(1).await.is_empty(),
                "the 1.4 producer can handle root work but not the admitted scoped activity"
            );
        }
        let mut body_worker = scoped_worker(&p, "public-source-body", &["src.line"]).await;
        let observed = run_to_the_end(&p, &scheduler, &mut body_worker, run).await;
        assert_eq!(
            observed,
            vec![
                (0, json!({"v":3})),
                (1, json!({"v":1})),
                (2, json!({"v":3}))
            ]
        );
        assert_eq!(aggregate(&p, run).await, json!([3, 1, 3]));
        assert_eq!(status_and_code(&p, run).await.0, "SUCCEEDED");
    }
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("full replay verifies root and scoped journal/projections");
}

#[tokio::test]
async fn a_failed_source_activity_starts_no_collection() {
    let p = platform().await;
    let workflow = p
        .publish(
            "failed-source-pipeline",
            &pipeline("failed-source-pipeline"),
        )
        .await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let mut worker = scoped_worker(&p, "failed-source-worker", &["src.fetch", "src.line"]).await;
    let task = worker.poll(1).await.pop().expect("source task");
    worker
        .fail(&task, false)
        .await
        .expect("source fails durably");
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    tick_until_settled(&p, &scheduler, run, 20).await;
    assert_eq!(status_and_code(&p, run).await.0, "FAILED");
    assert_eq!(scopes(&p, run).await, 0);
    assert!(worker.poll(1).await.is_empty());
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("failed source replay");
}
