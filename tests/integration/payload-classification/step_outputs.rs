//! Real execution-plane evidence for classified journal outputs and pinned descendants.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_api_grpc::pb::WriteBlobHeader;
use sagat_application::classification::{ClassifiedNode, ScalarValue};
use sagat_application::clock::SystemClock;
use sagat_application::payload_disclosure::{
    DisclosureSchemaCarrier, DisclosureSelector, PayloadDisclosure, PayloadDisclosureResult,
    PayloadDisclosureService, PayloadDisclosureUseCase, StructureOnlyReason,
    UnavailableDisclosureReason,
};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Principal, Role};
use sagat_infra_ir::JsonPayloadProjector;
use sagat_infra_postgres::PgRunStore;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use super::support;
use super::support::api::Options;
use super::support::engine::{
    World, activity, child_node, ctx, document, effect_activities, effect_node, end_node, flaky,
    wait_node, with, world,
};
use super::support::scripts;
use super::support::worker::{HUMAN, platform_with};

#[derive(Debug)]
struct JournalRecord {
    sequence: u64,
    attempt: Option<u32>,
    content: Option<Value>,
}

async fn journal_record(
    pool: &sqlx::PgPool,
    run: RunId,
    event_type: &str,
    node: &str,
) -> JournalRecord {
    let row = sqlx::query(
        "SELECT e.seq,e.attempt,p.content FROM run_events e LEFT JOIN payloads p \
         ON p.tenant_id=e.tenant_id AND p.digest=e.payload_digest \
         WHERE e.run_id=$1 AND e.event_type=$2 AND e.node=$3 ORDER BY e.seq DESC LIMIT 1",
    )
    .bind(run.as_uuid())
    .bind(event_type)
    .bind(node)
    .fetch_one(pool)
    .await
    .expect("journal event produced by the execution fixture");
    let sequence = u64::try_from(row.get::<i64, _>("seq")).expect("positive sequence");
    let attempt = row
        .get::<Option<i32>, _>("attempt")
        .map(u32::try_from)
        .transpose()
        .expect("nonnegative attempt");
    let content = row
        .get::<Option<Vec<u8>>, _>("content")
        .map(|bytes| serde_json::from_slice(&bytes).expect("retained event payload is JSON"));
    JournalRecord {
        sequence,
        attempt,
        content,
    }
}

async fn disclose_from(
    store: PgRunStore,
    principal: &Principal,
    run: RunId,
    selector: DisclosureSelector,
) -> PayloadDisclosure {
    let service = PayloadDisclosureService::new(
        store,
        SystemClock,
        Classifier::default(),
        JsonPayloadProjector,
    );
    let prepared = service
        .prepare(principal, run, selector, false, &ctx())
        .await
        .expect("prepare disclosure");
    service.disclose(prepared).await.expect("disclose payload")
}

async fn disclose_journal(
    world: &World,
    principal: &Principal,
    run: RunId,
    sequence: u64,
) -> PayloadDisclosure {
    disclose_from(
        world.store.clone(),
        principal,
        run,
        DisclosureSelector::Journal { sequence },
    )
    .await
}

fn disclosed(answer: &PayloadDisclosure) -> &ClassifiedNode {
    let PayloadDisclosureResult::Disclosed {
        schema, projection, ..
    } = &answer.result
    else {
        panic!("expected a classified value, got a closed non-value result")
    };
    assert_eq!(schema.carrier, DisclosureSchemaCarrier::NodeOutputSchema);
    &projection.tree
}

fn string_at<'a>(node: &'a ClassifiedNode, path: &[&str]) -> Option<&'a str> {
    let (name, rest) = path.split_first()?;
    let ClassifiedNode::Container { children, .. } = node else {
        return None;
    };
    let child = children
        .iter()
        .find(|child| child.name.as_deref() == Some(*name))?;
    if rest.is_empty() {
        return match &*child.node {
            ClassifiedNode::Value {
                value: ScalarValue::String(value),
                ..
            } => Some(value),
            _ => None,
        };
    }
    string_at(&child.node, rest)
}

fn public_schema(field: &str) -> Value {
    json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{(field):{"type":"string"}}
    })
}

#[tokio::test]
async fn activity_wait_and_reconciliation_disclose_their_actual_event_payloads() {
    let world = world().await;

    let activity_node = with(activity("charge"), "outputSchema", public_schema("receipt"));
    let (workflow, _) = world
        .publish(document(
            "classified-activity-output",
            "charge",
            vec![activity_node],
        ))
        .await;
    let activity_run = world.start(workflow, &json!({})).await;
    let activity_session = world.session(&["act.charge"]).await;
    let activity_task = world.one_task(&activity_session).await;
    world
        .complete(
            &activity_session,
            &activity_task,
            &json!({"receipt":"activity-value"}),
        )
        .await
        .expect("complete activity through the worker service");
    let activity_event =
        journal_record(&world.db.pool, activity_run, "step.completed", "charge").await;
    assert_eq!(activity_event.attempt, Some(1));
    assert_eq!(
        activity_event.content,
        Some(json!({"receipt":"activity-value"}))
    );
    let answer =
        disclose_journal(&world, &world.admin, activity_run, activity_event.sequence).await;
    assert_eq!(answer.node_id.as_deref(), Some("charge"));
    assert_eq!(answer.attempt, Some(1));
    assert_eq!(
        string_at(disclosed(&answer), &["receipt"]),
        Some("activity-value")
    );

    let wait = with(
        with(
            wait_node("paid", "payment.confirmed", "input.order", 300_000, None),
            "outputSchema",
            public_schema("confirmation"),
        ),
        "next",
        json!("done"),
    );
    let (workflow, _) = world
        .publish(document(
            "classified-wait-output",
            "paid",
            vec![wait, end_node("done")],
        ))
        .await;
    let wait_run = world.start(workflow, &json!({"order":"o-7"})).await;
    world
        .runs
        .signal(
            &world.admin,
            wait_run,
            "payment.confirmed",
            br#"{"confirmation":"wait-value"}"#,
            "classified-wait-signal",
            &ctx(),
        )
        .await
        .expect("signal the real waiting step");
    let wait_event = journal_record(&world.db.pool, wait_run, "wait.satisfied", "paid").await;
    assert_eq!(wait_event.attempt, None);
    assert_eq!(
        wait_event.content,
        Some(json!({"confirmation":"wait-value"}))
    );
    let answer = disclose_journal(&world, &world.admin, wait_run, wait_event.sequence).await;
    assert_eq!(answer.node_id.as_deref(), Some("paid"));
    assert_eq!(answer.attempt, None);
    assert_eq!(
        string_at(disclosed(&answer), &["confirmation"]),
        Some("wait-value")
    );

    let effect = with(
        effect_node("effect", None, true),
        "outputSchema",
        public_schema("provider"),
    );
    let (workflow, _) = world
        .publish(document(
            "classified-reconciliation-output",
            "effect",
            vec![effect],
        ))
        .await;
    let activities = effect_activities(&["effect"]);
    let reconciliation_session = world
        .session(&activities.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    let reconciliation_run = world.start(workflow, &json!({})).await;
    let _lost = world
        .one_task_of(&reconciliation_session, "act.effect")
        .await;
    world.age_lease(reconciliation_run, "effect").await;
    assert_eq!(world.scheduler.tick().await.leases_expired, 1);
    let lookup = world
        .one_task_of(&reconciliation_session, "lookup.effect")
        .await;
    world
        .answer_reconciliation(
            &reconciliation_session,
            &lookup,
            "applied",
            Some(json!({"provider":"reconciliation-value"})),
        )
        .await
        .expect("complete reconciliation through the worker service");
    let reconciliation_event = journal_record(
        &world.db.pool,
        reconciliation_run,
        "reconciliation.concluded",
        "effect",
    )
    .await;
    assert_eq!(reconciliation_event.attempt, None);
    assert_eq!(
        reconciliation_event.content,
        Some(json!({"provider":"reconciliation-value"}))
    );
    let answer = disclose_journal(
        &world,
        &world.admin,
        reconciliation_run,
        reconciliation_event.sequence,
    )
    .await;
    assert_eq!(answer.node_id.as_deref(), Some("effect"));
    assert_eq!(answer.attempt, None);
    assert_eq!(
        string_at(disclosed(&answer), &["provider"]),
        Some("reconciliation-value")
    );
}

#[tokio::test]
async fn a_real_built_script_output_uses_the_script_node_carrier() {
    let lab = scripts::lab().await;
    let script = lab
        .script(
            "classified-output-script",
            "typescript",
            "export default () => ({scriptValue: 'sandbox-value'})",
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(
            "classified-script-output",
            &digest,
            json!({"outputSchema":public_schema("scriptValue")}),
        )
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    let run_id: RunId = run.parse().expect("run id");
    let event = journal_record(&lab.p.api.db.pool, run_id, "step.completed", "s").await;
    assert_eq!(event.attempt, Some(1));
    assert_eq!(event.content, Some(json!({"scriptValue":"sandbox-value"})));
    let principal = Principal::new(lab.p.tenant, support::subject(HUMAN), Role::Admin);
    let answer = disclose_from(
        lab.p.api.executions.clone(),
        &principal,
        run_id,
        DisclosureSelector::Journal {
            sequence: event.sequence,
        },
    )
    .await;
    assert_eq!(answer.node_id.as_deref(), Some("s"));
    assert_eq!(answer.attempt, Some(1));
    assert_eq!(
        string_at(disclosed(&answer), &["scriptValue"]),
        Some("sandbox-value")
    );
}

fn versioned_child(name: &str, level: &str) -> Value {
    let mut child = document(
        name,
        "kid",
        vec![
            with(activity("kid"), "next", json!("done")),
            with(end_node("done"), "outputExpr", json!("steps.kid.output")),
        ],
    );
    child["inputSchema"] = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{"versioned":{"type":"string","x-sagat-classification":level}}
    });
    child["outputSchema"] = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{"versioned":{"type":"string","x-sagat-classification":level}}
    });
    child
}

#[tokio::test]
async fn child_uses_its_own_pinned_version_while_the_parent_completion_uses_the_parent_carrier() {
    let world = world().await;
    let (child_workflow, child_v1) = world
        .publish(versioned_child("classified-kid", "pii"))
        .await;
    let child = with(
        with(
            child_node("child", "classified-kid", None),
            "version",
            json!(1),
        ),
        "outputSchema",
        public_schema("versioned"),
    );
    let (parent_workflow, _) = world
        .publish(document("classified-parent", "child", vec![child]))
        .await;
    let child_v2 = world
        .publish_new_version(child_workflow, versioned_child("classified-kid", "public"))
        .await;
    assert_ne!(child_v1, child_v2);

    let parent_run = world
        .start(parent_workflow, &json!({"versioned":"pinned-child-input"}))
        .await;
    assert_eq!(world.scheduler.tick().await.children_started, 1);
    let child_run = sagat_application::execution::child_run_id(
        parent_run,
        &sagat_domain::workflow::NodeId::parse("child").expect("node id"),
    );
    let child_record = world.run_record(child_run).await.expect("child run");
    assert_eq!(child_record.info.version_id, child_v1);
    let session = world.session(&["act.kid"]).await;
    let task = world.one_task(&session).await;
    assert_eq!(task.run_id, child_run);
    world
        .complete(&session, &task, &json!({"versioned":"pinned-child-result"}))
        .await
        .expect("complete child activity");
    assert_eq!(world.scheduler.tick().await.children_finished, 1);

    let developer = world
        .member("pinned-child-developer", Role::Developer)
        .await;
    for selector in [DisclosureSelector::Input, DisclosureSelector::Result] {
        let answer = disclose_from(world.store.clone(), &developer, child_run, selector).await;
        let PayloadDisclosureResult::Disclosed {
            schema, projection, ..
        } = answer.result
        else {
            panic!("child input and result retain their pinned classified schema")
        };
        assert_eq!(schema.version_number, 1);
        assert_eq!(string_at(&projection.tree, &["versioned"]), None);
    }

    let parent_event = journal_record(&world.db.pool, parent_run, "child.finished", "child").await;
    assert_eq!(
        parent_event.content,
        Some(json!({"versioned":"pinned-child-result"}))
    );
    let parent_answer =
        disclose_journal(&world, &developer, parent_run, parent_event.sequence).await;
    assert_eq!(parent_answer.node_id.as_deref(), Some("child"));
    let PayloadDisclosureResult::Disclosed {
        schema, projection, ..
    } = &parent_answer.result
    else {
        panic!(
            "child.finished must use the parent's node outputSchema; actual={:?}",
            parent_answer.result
        )
    };
    assert_eq!(schema.carrier, DisclosureSchemaCarrier::NodeOutputSchema);
    assert_eq!(
        string_at(&projection.tree, &["versioned"]),
        Some("pinned-child-result")
    );
}

#[tokio::test]
async fn rerun_and_retry_keep_their_pinned_version_sequence_and_attempt() {
    let world = world().await;
    let mut v1 = document("classified-rerun", "done", vec![end_node("done")]);
    v1["inputSchema"] = public_schema("versioned");
    let (workflow, version_v1) = world.publish(v1).await;
    let original = world
        .start(workflow, &json!({"versioned":"rerun-v1-value"}))
        .await;
    let mut v2 = document("classified-rerun", "done", vec![end_node("done")]);
    v2["inputSchema"] = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{"versioned":{"type":"string","x-sagat-classification":"pii"}}
    });
    let version_v2 = world.publish_new_version(workflow, v2).await;
    assert_ne!(version_v1, version_v2);
    let rerun = world
        .runs
        .rerun(
            &world.admin,
            original,
            "verify the pinned classification version",
            &format!("classified-rerun-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("rerun terminal workflow");
    assert_eq!(rerun.info.version_id, version_v1);
    let answer = disclose_from(
        world.store.clone(),
        &world.admin,
        rerun.info.run_id,
        DisclosureSelector::Input,
    )
    .await;
    let PayloadDisclosureResult::Disclosed {
        schema, projection, ..
    } = answer.result
    else {
        panic!("rerun input classified by v1")
    };
    assert_eq!(schema.version_number, 1);
    assert_eq!(
        string_at(&projection.tree, &["versioned"]),
        Some("rerun-v1-value")
    );

    let retry = with(
        flaky("retry", 2),
        "outputSchema",
        public_schema("attemptValue"),
    );
    let (workflow, _) = world
        .publish(document("classified-retry", "retry", vec![retry]))
        .await;
    let retry_run = world.start(workflow, &json!({})).await;
    let session = world.session(&["act.retry"]).await;
    let first = world.one_task(&session).await;
    assert_eq!(first.fence.attempt, 1);
    world
        .fail(&session, &first, true)
        .await
        .expect("first attempt fails retryably");
    world.age_retry(retry_run, "retry").await;
    assert_eq!(world.scheduler.tick().await.retries_released, 1);
    let second = world.one_task(&session).await;
    assert_eq!(second.fence.attempt, 2);
    world
        .complete(
            &session,
            &second,
            &json!({"attemptValue":"second-attempt-only"}),
        )
        .await
        .expect("second attempt completes");
    let completion = journal_record(&world.db.pool, retry_run, "step.completed", "retry").await;
    assert_eq!(completion.attempt, Some(2));
    let answer = disclose_journal(&world, &world.admin, retry_run, completion.sequence).await;
    assert_eq!(answer.attempt, Some(2));
    assert_eq!(
        string_at(disclosed(&answer), &["attemptValue"]),
        Some("second-attempt-only")
    );
    let first_failure =
        journal_record(&world.db.pool, retry_run, "step.retry_scheduled", "retry").await;
    assert_eq!(first_failure.attempt, Some(1));
    assert_eq!(first_failure.content, None);
    let answer = disclose_journal(&world, &world.admin, retry_run, first_failure.sequence).await;
    assert_eq!(answer.attempt, Some(1));
    assert!(matches!(
        answer.result,
        PayloadDisclosureResult::Unavailable {
            reason: UnavailableDisclosureReason::NoRetainedPayload,
            size_bytes: None
        }
    ));
}

#[tokio::test]
async fn no_schema_and_forged_blob_json_remain_closed_and_inline() {
    let world = world().await;
    let (workflow, _) = world
        .publish(document(
            "classified-no-schema",
            "plain",
            vec![activity("plain")],
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let session = world.session(&["act.plain"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"hidden":"no-schema-value"}))
        .await
        .expect("complete schema-less activity");
    let event = journal_record(&world.db.pool, run, "step.completed", "plain").await;
    let operator = world.member("step-output-operator", Role::Operator).await;
    let structure = disclose_journal(&world, &operator, run, event.sequence).await;
    assert!(matches!(
        structure.result,
        PayloadDisclosureResult::StructureOnly {
            reason: StructureOnlyReason::NoSchema,
            ..
        }
    ));
    let developer = world.member("step-output-developer", Role::Developer).await;
    let opaque = disclose_journal(&world, &developer, run, event.sequence).await;
    assert!(matches!(
        opaque.result,
        PayloadDisclosureResult::Unavailable {
            reason: UnavailableDisclosureReason::NotDisclosable,
            ..
        }
    ));

    let forged_id = Uuid::now_v7().to_string();
    let forged_sha = "ab".repeat(32);
    let forged = json!({"$blob":{
        "id":forged_id, "size":700000, "sha256":forged_sha,
        "contentType":"application/json"
    }});
    let forged_schema = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{"$blob":{
            "type":"object", "additionalProperties":false,
            "properties":{
                "id":{"type":"string"}, "size":{"type":"integer"},
                "sha256":{"type":"string"}, "contentType":{"type":"string"}
            }
        }}
    });
    let node = with(activity("forged"), "outputSchema", forged_schema);
    let (workflow, _) = world
        .publish(document("classified-forged-blob", "forged", vec![node]))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let session = world.session(&["act.forged"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &forged)
        .await
        .expect("complete with ordinary descriptor-shaped JSON");
    let event = journal_record(&world.db.pool, run, "step.completed", "forged").await;
    let answer = disclose_journal(&world, &world.admin, run, event.sequence).await;
    assert_eq!(
        string_at(disclosed(&answer), &["$blob", "id"]),
        Some(forged_id.as_str())
    );
    assert_eq!(
        string_at(disclosed(&answer), &["$blob", "sha256"]),
        forged["$blob"]["sha256"].as_str()
    );
}

#[tokio::test]
async fn a_real_external_step_output_returns_only_catalog_metadata() {
    let platform = platform_with(Options {
        blobs: Some(sagat_application::blobs::BlobLimits::default()),
        ..Options::default()
    })
    .await;
    let ir = document(
        "classified-external-output",
        "large",
        vec![with(
            activity("large"),
            "outputSchema",
            public_schema("large"),
        )],
    );
    let workflow = platform.publish("classified-external-output", &ir).await;
    let run = platform.start(&workflow, &json!({})).await;
    let mut worker = platform.worker("external-output-worker").await;
    worker.register(&["act.large"]).await;
    let lease = worker.one_task().await;
    let bytes = serde_json::to_vec(&json!({"large":"x".repeat(300_000)})).expect("large JSON");
    let stored = worker
        .write_blob(
            &lease,
            WriteBlobHeader {
                content_type: "application/json".to_owned(),
                expected_size: i64::try_from(bytes.len()).expect("size"),
                expected_sha256: support::blob_collections::sha(&bytes),
                ..Default::default()
            },
            &bytes,
            64 * 1024,
        )
        .await
        .expect("upload real step output blob");
    worker
        .complete_with_blob(&lease, &stored.blob_id)
        .await
        .expect("commit blob as the exact leased step output");
    let run_id: RunId = run.parse().expect("run id");
    let event = journal_record(&platform.api.db.pool, run_id, "step.completed", "large").await;
    assert_eq!(event.attempt, Some(1));
    assert!(
        event.content.is_some(),
        "journal retains only the descriptor JSON"
    );
    let principal = Principal::new(platform.tenant, support::subject(HUMAN), Role::Admin);
    let answer = disclose_from(
        platform.api.executions.clone(),
        &principal,
        run_id,
        DisclosureSelector::Journal {
            sequence: event.sequence,
        },
    )
    .await;
    assert_eq!(answer.node_id.as_deref(), Some("large"));
    assert_eq!(answer.attempt, Some(1));
    assert!(matches!(
        answer.result,
        PayloadDisclosureResult::External {
            size_bytes,
            content_kind: sagat_application::payloads::ExternalContentKind::Json
        } if size_bytes == u64::try_from(bytes.len()).expect("size")
    ));

    let (status, wire) = platform
        .rest(
            Method::POST,
            &format!("/runs/{run}/payload-disclosures"),
            Some(&json!({"source":"journal","sequence":event.sequence})),
        )
        .await;
    assert_eq!(status, 200, "{wire}");
    assert_eq!(wire["kind"], "external");
    assert_eq!(wire["reason"], "EXTERNAL_CONTENT_NOT_INSPECTED");
    assert_eq!(wire["contentKind"], "json");
    assert_eq!(wire["sizeBytes"], bytes.len());
    assert!(wire.get("tree").is_none());
}
