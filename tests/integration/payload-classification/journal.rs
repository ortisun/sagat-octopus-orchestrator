//! Real PostgreSQL journal carrier and sequence tests.
#![allow(clippy::expect_used)]

use sagat_application::error::AppError;
use sagat_application::payload_disclosure::{
    DisclosureSelector, PayloadDisclosureError, PayloadDisclosureResult,
};
use sagat_domain::classification::Level;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::disclosure::{disclose, disclosed_string};
use super::support::engine::{with, world};

#[tokio::test]
async fn journal_sequence_uses_the_exact_persisted_scope_when_node_ids_repeat() {
    let world = world().await;
    let source = json!({
        "apiVersion":"sagat-octopus-orchestrator.io/v1",
        "kind":"Workflow",
        "metadata":{"name":"classified-scoped-journal"},
        "entrypoint":"first_owner",
        "bodies":{
            "first":{
                "entrypoint":"same",
                "nodes":[
                    {"id":"same","type":"activity","activity":"act.first","next":"returned",
                     "outputSchema":{"type":"object","additionalProperties":false,
                       "x-sagat-classification":"public","properties":{"root":{"type":"string"}}}},
                    {"id":"returned","type":"return","result":"steps.same.output"}
                ]
            },
            "second":{
                "entrypoint":"same",
                "nodes":[
                    {"id":"same","type":"activity","activity":"act.second","next":"returned",
                     "outputSchema":{"type":"object","additionalProperties":false,
                       "x-sagat-classification":"pii","properties":{"body":{"type":"string"}}}},
                    {"id":"returned","type":"return","result":"steps.same.output"}
                ]
            }
        },
        "nodes":[
            {"id":"first_owner","type":"forEach","items":"input.items","body":"first",
             "result":"bodyResult","maxItems":1,"maxConcurrency":1,"next":"second_owner"},
            {"id":"second_owner","type":"forEach","items":"input.items","body":"second",
             "result":"bodyResult","maxItems":1,"maxConcurrency":1,"next":"done"},
            {"id":"done","type":"end"}
        ]
    });
    let (workflow, _) = world.publish(source).await;
    let run = world.start(workflow, &json!({"items":[1]})).await;
    let root_payload = br#"{"root":"root-value"}"#;
    let body_payload = br#"{"body":"body-value"}"#;
    let root_digest = Sha256::digest(root_payload).to_vec();
    let body_digest = Sha256::digest(body_payload).to_vec();
    for (digest, payload) in [
        (&root_digest, root_payload.as_slice()),
        (&body_digest, body_payload.as_slice()),
    ] {
        sqlx::query(
            "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(world.admin.tenant_id().as_uuid())
        .bind(digest)
        .bind(payload)
        .execute(&world.db.pool)
        .await
        .expect("test payload");
    }
    let first: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(seq),0)+1 FROM run_events WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("next sequence");
    for (sequence, digest, path_nodes, path_indices) in [
        (first, &root_digest, vec!["first_owner".to_owned()], vec![0]),
        (
            first + 1,
            &body_digest,
            vec!["second_owner".to_owned()],
            vec![0],
        ),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id,run_id,seq,event_type,node,attempt,detail,payload_digest,actor,occurred_at, \
              instance_id,instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,$3,'step.completed','same',1,'{}',$4,'test',clock_timestamp(),$5,$6,$7)",
        )
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .bind(sequence)
        .bind(digest)
        .bind(Uuid::now_v7())
        .bind(path_nodes)
        .bind(path_indices)
        .execute(&world.db.pool)
        .await
        .expect("scoped completion event");
    }

    let rows: Vec<(i64, Vec<String>)> = sqlx::query_as(
        "SELECT seq,instance_path_nodes FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
         AND event_type='step.completed' AND node='same' ORDER BY seq",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&world.db.pool)
    .await
    .expect("completion events");
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|(sequence, _)| *sequence >= first)
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, ["first_owner"]);
    assert_eq!(rows[1].1, ["second_owner"]);

    let root_answer = disclose(
        &world,
        &world.admin,
        run,
        DisclosureSelector::Journal {
            sequence: u64::try_from(rows[0].0).expect("positive sequence"),
        },
    )
    .await
    .expect("root disclosure");
    let body_answer = disclose(
        &world,
        &world.admin,
        run,
        DisclosureSelector::Journal {
            sequence: u64::try_from(rows[1].0).expect("positive sequence"),
        },
    )
    .await
    .expect("body disclosure");
    let PayloadDisclosureResult::Disclosed { projection, .. } = root_answer.result else {
        panic!("first body carrier")
    };
    assert_eq!(
        disclosed_string(&projection.tree, "root"),
        Some("root-value")
    );
    let PayloadDisclosureResult::Disclosed { projection, .. } = body_answer.result else {
        panic!("second body carrier")
    };
    assert!(matches!(
        projection.tree,
        sagat_application::classification::ClassifiedNode::Withheld {
            level: Level::Pii,
            ..
        }
    ));
}

#[tokio::test]
async fn journal_never_substitutes_another_sequence_or_payload() {
    let world = world().await;
    let source = json!({
        "apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
        "metadata":{"name":"classified-journal-sequence"},"entrypoint":"work",
        "nodes":[with(super::support::engine::activity("work"), "outputSchema", json!({
            "type":"object","x-sagat-classification":"public"
        }))]
    });
    let (workflow, _) = world.publish(source).await;
    let run = world.start(workflow, &json!({})).await;
    let started: i64 = sqlx::query_scalar(
        "SELECT seq FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND payload_digest IS NULL ORDER BY seq LIMIT 1",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("payload-free event");
    let no_payload = disclose(
        &world,
        &world.admin,
        run,
        DisclosureSelector::Journal {
            sequence: u64::try_from(started).expect("positive sequence"),
        },
    )
    .await
    .expect("closed no-payload answer");
    assert!(matches!(
        no_payload.result,
        PayloadDisclosureResult::Unavailable { .. }
    ));
    assert_eq!(
        disclose(
            &world,
            &world.admin,
            run,
            DisclosureSelector::Journal {
                sequence: i64::MAX as u64
            },
        )
        .await,
        Err(PayloadDisclosureError::Application(AppError::NotFound))
    );
}
