//! Design 043 against a real PostgreSQL: publication runs the pre-check of the run-input subset on every new
//! `inputSchema` in the mode it is given (T043-13, FR-043-012).

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{DefinitionService, InputValidationMode, IrError};
use sagat_application::error::AppError;
use sagat_domain::workflow::{DefinitionWarning, InputSchemaIssue};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::{bytes, doc};
use crate::support::engine::{World, ctx, world};

fn with_schema(name: &str, schema: Value, triggers: Option<Value>) -> Value {
    let mut document = doc(name, "schema");
    document["inputSchema"] = schema;
    if let Some(triggers) = triggers {
        document["triggers"] = triggers;
    }
    document
}

fn outside() -> Value {
    json!({"type": "object", "patternProperties": {"^x-": {"type": "string"}}})
}

/// Creates the workflow and publishes it with the definition service of `mode`.
async fn publish_in(
    w: &World,
    mode: InputValidationMode,
    document: &Value,
) -> Result<Vec<DefinitionWarning>, AppError> {
    let defs = DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec)
        .with_input_validation(mode);
    let name = document["metadata"]["name"].as_str().expect("name");
    let created = defs
        .create_workflow(
            &w.admin,
            "payments",
            name,
            &bytes(document),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("a draft");
    defs.publish(
        &w.admin,
        created.workflow.id(),
        created.workflow.draft_revision(),
        None,
        &format!("publish-{}", Uuid::now_v7()),
        &ctx(),
    )
    .await
    .map(|published| published.warnings)
}

#[tokio::test]
async fn publication_refuses_a_schema_outside_the_subset_in_enforce_and_warns_in_report() {
    let w = world().await;
    let refused = publish_in(
        &w,
        InputValidationMode::Enforce,
        &with_schema("strict-flow", outside(), None),
    )
    .await
    .expect_err("refused in enforce mode");
    let AppError::InvalidDefinition(error) = refused else {
        panic!("an invalid definition, got {refused:?}")
    };
    assert_eq!(
        *error,
        IrError::InputSchema {
            issue: InputSchemaIssue::KeywordUnsupported,
            keyword: "patternProperties".to_owned(),
            path: "/inputSchema/patternProperties".to_owned(),
        }
    );
    // the default of the services the platform builds is enforce
    let default = w
        .try_publish_as(&w.admin, with_schema("default-flow", outside(), None))
        .await;
    assert!(
        matches!(default, Err(AppError::InvalidDefinition(_))),
        "{default:?}"
    );

    let warnings = publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema("lenient-flow", outside(), None),
    )
    .await
    .expect("published in report mode");
    assert_eq!(
        warnings,
        vec![DefinitionWarning::InputSchemaOutsideSubset {
            issue: InputSchemaIssue::KeywordUnsupported,
            keyword: "patternProperties".to_owned(),
            at: "/patternProperties".to_owned(),
        }]
    );
    let triggered = publish_in(
        &w,
        InputValidationMode::Enforce,
        &with_schema(
            "triggered-flow",
            json!({"type": "object", "required": ["event"]}),
            Some(json!([{"kind": "event", "eventType": "order.created"}])),
        ),
    )
    .await
    .expect("a schema inside the subset publishes");
    assert_eq!(
        triggered,
        vec![DefinitionWarning::InputSchemaAppliesToTriggers]
    );
}
