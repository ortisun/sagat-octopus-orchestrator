//! Real PostgreSQL boundary tests for classified input disclosure.
#![allow(clippy::expect_used)]

use sagat_application::classification::{ClassifiedNode, ScalarValue};
use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::payload_disclosure::{
    DisclosureSelector, PayloadDisclosure, PayloadDisclosureError, PayloadDisclosureResult,
    PayloadDisclosureService, PayloadDisclosureUseCase,
};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Principal, Role};
use sagat_infra_ir::JsonPayloadProjector;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};

use super::support::engine::{World, ctx, document, end_node, world};

pub(crate) async fn disclose(
    world: &World,
    principal: &Principal,
    run: RunId,
    selector: DisclosureSelector,
) -> Result<PayloadDisclosure, PayloadDisclosureError> {
    disclose_with_reveal(world, principal, run, selector, false).await
}

async fn disclose_with_reveal(
    world: &World,
    principal: &Principal,
    run: RunId,
    selector: DisclosureSelector,
    reveal_pii: bool,
) -> Result<PayloadDisclosure, PayloadDisclosureError> {
    let service = PayloadDisclosureService::new(
        world.store.clone(),
        SystemClock,
        Classifier::default(),
        JsonPayloadProjector,
    );
    let prepared = service
        .prepare(principal, run, selector, reveal_pii, &ctx())
        .await
        .map_err(PayloadDisclosureError::from)?;
    service.disclose(prepared).await
}

#[tokio::test]
async fn effective_namespace_grants_reveal_and_audit_every_success_then_revoke_immediately() {
    let world = world().await;
    let (workflow, _) = world
        .publish(classified_document(
            "classified-reveal",
            json!({
                "type":"object", "additionalProperties":false,
                "x-sagat-classification":"pii",
                "properties":{"personal":{"type":"string"}}
            }),
        ))
        .await;
    let run = world
        .start(workflow, &json!({"personal":"revealed-value"}))
        .await;
    let developer = world.member("pii-developer", Role::Developer).await;
    sqlx::query(
        "INSERT INTO payload_reveal_grants \
         (tenant_id,namespace_id,subject,granted_by,granted_at) VALUES ($1,$2,$3,$4,clock_timestamp())",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(developer.subject().as_str())
    .bind(world.admin.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("grant reveal");

    for _ in 0..2 {
        let answer = disclose_with_reveal(&world, &developer, run, DisclosureSelector::Input, true)
            .await
            .expect("granted reveal");
        let PayloadDisclosureResult::Disclosed { projection, .. } = answer.result else {
            panic!("PII projection")
        };
        assert_eq!(
            disclosed_string(&projection.tree, "personal"),
            Some("revealed-value")
        );
    }
    let reveal_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND actor=$2 AND resource_id=$3 \
         AND action='payload.pii_revealed' AND reason='classified-v1:input:reveal=pii'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(developer.subject().as_str())
    .bind(run.to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("reveal audits");
    assert_eq!(reveal_audits, 2, "successful reveals never collapse");

    sqlx::query(
        "DELETE FROM payload_reveal_grants WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(developer.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("revoke reveal");
    assert_eq!(
        disclose_with_reveal(&world, &developer, run, DisclosureSelector::Input, true,).await,
        Err(PayloadDisclosureError::PiiRevealNotGranted)
    );
    let denied_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND actor=$2 AND resource_id=$3 \
         AND action='payload.reveal_denied' AND reason='permission'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(developer.subject().as_str())
    .bind(run.to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("denied audit");
    assert_eq!(denied_audits, 1);
}

pub(crate) fn disclosed_string<'a>(node: &'a ClassifiedNode, name: &str) -> Option<&'a str> {
    let ClassifiedNode::Container { children, .. } = node else {
        return None;
    };
    children.iter().find_map(|child| {
        if child.name.as_deref() != Some(name) {
            return None;
        }
        match &*child.node {
            ClassifiedNode::Value {
                value: ScalarValue::String(value),
                ..
            } => Some(value.as_str()),
            _ => None,
        }
    })
}

fn classified_document(name: &str, schema: Value) -> Value {
    let mut document = document(name, "done", vec![end_node("done")]);
    document["inputSchema"] = schema;
    document
}

#[tokio::test]
async fn postgres_uses_the_current_role_and_commits_required_audit_before_values_return() {
    let world = world().await;
    let schema = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{
            "open":{"type":"string"},
            "inside":{"type":"string","x-sagat-classification":"internal"},
            "personal":{"type":"string","x-sagat-classification":"pii"},
            "sealed":{"type":"string","x-sagat-classification":"secret"}
        }
    });
    let (workflow, _) = world
        .publish(classified_document("classified-input", schema))
        .await;
    let run = world
        .start(
            workflow,
            &json!({"open":"shown","inside":"staff","personal":"pii-canary","sealed":"secret-canary"}),
        )
        .await;
    let viewer = world.member("input-viewer", Role::Viewer).await;
    let developer = world.member("input-developer", Role::Developer).await;

    let viewer_answer = disclose(&world, &viewer, run, DisclosureSelector::Input)
        .await
        .expect("viewer disclosure");
    let developer_answer = disclose(&world, &developer, run, DisclosureSelector::Input)
        .await
        .expect("developer disclosure");
    let PayloadDisclosureResult::Disclosed { projection, .. } = viewer_answer.result else {
        panic!("viewer gets classified projection")
    };
    assert_eq!(disclosed_string(&projection.tree, "open"), Some("shown"));
    assert_eq!(disclosed_string(&projection.tree, "inside"), None);
    assert_eq!(disclosed_string(&projection.tree, "personal"), None);
    assert_eq!(disclosed_string(&projection.tree, "sealed"), None);
    let PayloadDisclosureResult::Disclosed { projection, .. } = developer_answer.result else {
        panic!("developer gets classified projection")
    };
    assert_eq!(disclosed_string(&projection.tree, "inside"), Some("staff"));
    assert_eq!(disclosed_string(&projection.tree, "personal"), None);
    assert_eq!(disclosed_string(&projection.tree, "sealed"), None);

    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND actor=$2 \
         AND resource_id=$3 AND action='payload.disclosed' \
         AND reason='classified-v1:input'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(developer.subject().as_str())
    .bind(run.to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("audit count");
    assert_eq!(audits, 1);

    let failing = world.member("input-audit-failure", Role::Developer).await;
    sqlx::raw_sql(
        "CREATE FUNCTION refuse_classified_disclosure() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action='payload.disclosed' THEN RAISE EXCEPTION 'injected audit failure'; \
         END IF; RETURN NEW; END $$; \
         CREATE TRIGGER refuse_classified_disclosure BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION refuse_classified_disclosure();",
    )
    .execute(&world.db.pool)
    .await
    .expect("install failure");
    let failed = disclose(&world, &failing, run, DisclosureSelector::Input).await;
    assert_eq!(
        failed,
        Err(PayloadDisclosureError::Application(AppError::Unavailable))
    );
}

#[tokio::test]
async fn invalid_payloads_are_structural_only_for_inspectors_and_opaque_to_other_readers() {
    let world = world().await;
    let schema = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{"declared":{"type":"string"}}
    });
    let (workflow, _) = world
        .publish(classified_document("invalid-classified-input", schema))
        .await;
    let run = world.start(workflow, &json!({"declared":7})).await;
    let developer = world.member("invalid-developer", Role::Developer).await;
    let operator = world.member("invalid-operator", Role::Operator).await;

    assert!(matches!(
        disclose(&world, &developer, run, DisclosureSelector::Input)
            .await
            .expect("opaque answer")
            .result,
        PayloadDisclosureResult::Unavailable { .. }
    ));
    assert!(matches!(
        disclose(&world, &operator, run, DisclosureSelector::Input)
            .await
            .expect("structural answer")
            .result,
        PayloadDisclosureResult::StructureOnly { .. }
    ));
}

#[tokio::test]
async fn missing_and_foreign_runs_are_the_same_opaque_not_found() {
    let world = world().await;
    let foreign_workflow = world
        .publish_as_other_tenant(classified_document(
            "foreign-classified-input",
            json!({"type":"object","x-sagat-classification":"public"}),
        ))
        .await;
    let foreign = world
        .start_as_other_tenant(foreign_workflow, &json!({}))
        .await
        .expect("foreign run");
    for run in [RunId::generate(), foreign] {
        assert_eq!(
            disclose(&world, &world.admin, run, DisclosureSelector::Input).await,
            Err(PayloadDisclosureError::Application(AppError::NotFound))
        );
    }
}
