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
