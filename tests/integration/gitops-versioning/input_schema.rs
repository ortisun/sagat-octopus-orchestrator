//! Design 043 against a real PostgreSQL: publication runs the pre-check of the run-input subset on every new
//! `inputSchema` in the mode it is given (T043-13, FR-043-012), and `control-plane input-schema-inventory` lists the
//! stored versions that would refuse every start (T043-24, FR-043-018).

use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{DefinitionService, InputValidationMode, IrError};
use sagat_application::error::AppError;
use sagat_control_plane::config::DatabaseConfig;
use sagat_control_plane::inventory::{exit_code, input_schema_inventory};
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

fn database(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn the_inventory_lists_the_versions_that_would_be_refused_and_exits_by_what_it_found() {
    let w = world().await;
    // one version inside the subset with an event trigger: information only
    publish_in(
        &w,
        InputValidationMode::Enforce,
        &with_schema(
            "envelope-flow",
            json!({"type": "object", "required": ["orderId"]}),
            Some(json!([{"kind": "event", "eventType": "order.created"}])),
        ),
    )
    .await
    .expect("inside the subset");
    // more versions than one page of the inventory (64), so its keyset cursor is crossed (review of 2026-11-15)
    for n in 0..70 {
        publish_in(
            &w,
            InputValidationMode::Enforce,
            &with_schema(
                &format!("plain-flow-{n:02}"),
                json!({"type": "object"}),
                None,
            ),
        )
        .await
        .expect("inside the subset");
    }
    let only_inside = input_schema_inventory(&database(w.db.url.clone())).await;
    let inventory = only_inside.as_ref().expect("the inventory runs");
    assert!(inventory.refused.is_empty(), "{inventory:?}");
    assert_eq!(inventory.triggered.len(), 1, "{inventory:?}");
    assert_eq!(exit_code(&only_inside), 0, "nothing would be refused");

    // one with patternProperties, stored the way 0.2.x stored it (here: a publication in report mode)
    let secret_pattern = "^x-sentinel-";
    let mut schema = outside();
    schema["patternProperties"] = json!({secret_pattern: {"const": "sentinel-value"}});
    publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema("legacy-flow", schema, None),
    )
    .await
    .expect("stored");
    let found = input_schema_inventory(&database(w.db.url.clone())).await;
    let inventory = found.as_ref().expect("the inventory runs");
    assert_eq!(
        exit_code(&found),
        1,
        "at least one version would be refused"
    );
    assert_eq!(inventory.refused.len(), 1, "{inventory:?}");
    let refused = &inventory.refused[0];
    assert_eq!(
        (refused.issue, refused.keyword.as_str(), refused.at.as_str()),
        (
            InputSchemaIssue::KeywordUnsupported,
            "patternProperties",
            "/inputSchema/patternProperties"
        )
    );
    assert_eq!(refused.ids.tenant, w.admin.tenant_id());
    assert_eq!(
        inventory.triggered.len(),
        1,
        "the version with the trigger stays informational"
    );
    assert!(
        inventory.scanned >= 72,
        "every page is read: {}",
        inventory.scanned
    );
    let printed = inventory.to_string();
    assert!(printed.contains("refused tenant="), "{printed}");
    assert!(
        printed.contains(&refused.ids.version.to_string()),
        "{printed}"
    );
    assert!(
        printed.contains(
            r#"issue=INPUT_SCHEMA_KEYWORD_UNSUPPORTED keyword="patternProperties" at="/inputSchema/patternProperties""#
        ),
        "the fields are quoted: {printed}"
    );
    assert!(printed.contains("information tenant="), "{printed}");
    assert!(!printed.contains("sentinel"), "no schema text: {printed}");
    assert!(!printed.contains("orderId"), "no schema text: {printed}");

    // a stored document that is not a JSON object: the command cannot tell what it would do, so it fails (exit 2)
    let legacy: (uuid::Uuid, uuid::Uuid) =
        sqlx::query_as("SELECT tenant_id, id FROM workflows WHERE name = 'legacy-flow'")
            .fetch_one(&w.db.pool)
            .await
            .expect("the legacy workflow");
    sqlx::query(
        "INSERT INTO workflow_versions (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1, $2, $3, 99, repeat('0', 64), convert_to('not json', 'UTF8'), 'test', now())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(legacy.0)
    .bind(legacy.1)
    .execute(&w.db.pool)
    .await
    .expect("an unreadable stored document, written as the owner");
    let unreadable = input_schema_inventory(&database(w.db.url.clone())).await;
    let inventory = unreadable.as_ref().expect("the inventory runs");
    assert_eq!(inventory.unreadable.len(), 1, "{inventory:?}");
    assert_eq!(
        inventory.refused.len(),
        1,
        "and still lists what it could read"
    );
    assert_eq!(
        exit_code(&unreadable),
        2,
        "an unreadable document fails the command"
    );

    let unreachable = input_schema_inventory(&database(
        "postgres://sagat:none@127.0.0.1:1/none".to_owned(),
    ))
    .await;
    assert!(unreachable.is_err(), "{unreachable:?}");
    assert_eq!(exit_code(&unreachable), 2, "the command failed");
}
