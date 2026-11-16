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

/// The settings the inventory runs with in production: the runtime role, which must hold every grant it reads with.
fn as_runtime(w: &World) -> DatabaseConfig {
    database(w.db.url_for(
        crate::support::RUNTIME_ROLE,
        crate::support::RUNTIME_ROLE_PASSWORD,
    ))
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
    let only_inside = input_schema_inventory(&as_runtime(&w)).await;
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
    let found = input_schema_inventory(&as_runtime(&w)).await;
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
    let unreadable = input_schema_inventory(&as_runtime(&w)).await;
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

/// OD-2026-11-16-5 (round 2 of 2026-11-16, critic 1, intermediate zoom-out 4; D043-179): a version outside the subset
/// that nothing starts any more (not its workflow's active version, no open run, none created in the last
/// `SUPERSEDED_QUIET_DAYS` days) does not make the inventory exit 1, since it is immutable and nobody would start it;
/// `--all` lists it, still without changing the exit status. A superseded version that still has an open run counts.
#[tokio::test]
async fn a_superseded_version_nothing_starts_does_not_fail_the_inventory_and_all_lists_it() {
    use sagat_control_plane::inventory::{Scope, input_schema_inventory_in};

    let w = world().await;
    let defs = DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec)
        .with_input_validation(InputValidationMode::Report);
    // two workflows, each published first outside the subset (as 0.2.x stored it) and then fixed
    let mut ids = Vec::new();
    for name in ["quiet-flow", "busy-flow"] {
        let old = with_schema(name, outside(), None);
        let created = defs
            .create_workflow(
                &w.admin,
                "payments",
                name,
                &bytes(&old),
                &format!("create-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("a draft");
        let first = defs
            .publish(
                &w.admin,
                created.workflow.id(),
                created.workflow.draft_revision(),
                None,
                &format!("publish-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("stored outside the subset");
        ids.push((created.workflow.id(), first.version.id()));
    }
    // `busy-flow` keeps an open run of its old version (admitted in report mode before the fix)
    let reporting = sagat_application::runs::RunService::new(
        w.store.clone(),
        SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(std::sync::Arc::new(
        sagat_application::run_input::RunInputGate::new(
            sagat_infra_schema::run_input::RunInputValidator::process(),
            InputValidationMode::Report,
        ),
    ));
    reporting
        .start(
            &w.admin,
            &sagat_application::runs::StartRun {
                namespace: "payments",
                workflow: ids[1].0,
                version: None,
                input: b"{}",
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::now_v7()),
            },
            &ctx(),
        )
        .await
        .expect("an open run of the old version");
    for (name, (workflow, _)) in ["quiet-flow", "busy-flow"].into_iter().zip(&ids) {
        let record = defs
            .get_workflow(&w.admin, *workflow, &ctx())
            .await
            .expect("the workflow");
        let record = defs
            .update_draft(
                &w.admin,
                *workflow,
                record.workflow.draft_revision(),
                &bytes(&with_schema(name, json!({"type": "object"}), None)),
                &ctx(),
            )
            .await
            .expect("fixed");
        defs.publish(
            &w.admin,
            *workflow,
            record.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("the fixed version is active");
    }

    let default = input_schema_inventory(&as_runtime(&w)).await;
    let inventory = default.as_ref().expect("the inventory runs");
    assert_eq!(
        inventory
            .refused
            .iter()
            .map(|refused| refused.ids.version)
            .collect::<Vec<_>>(),
        vec![ids[1].1],
        "the old version with an open run still counts: {inventory:?}"
    );
    assert_eq!(
        inventory
            .superseded
            .iter()
            .map(|refused| refused.ids.version)
            .collect::<Vec<_>>(),
        vec![ids[0].1],
        "the quiet old version is superseded: {inventory:?}"
    );
    assert_eq!(exit_code(&default), 1, "for the busy one");
    let printed = inventory.to_string();
    assert!(
        !printed.contains(&ids[0].1.to_string()),
        "not listed by default: {printed}"
    );
    assert!(
        printed.contains("1 superseded"),
        "but counted in the summary: {printed}"
    );

    let all = input_schema_inventory_in(&as_runtime(&w), Scope::All).await;
    let listed = all.as_ref().expect("the inventory runs").to_string();
    assert!(
        listed.contains(&format!("superseded tenant={}", w.admin.tenant_id())),
        "{listed}"
    );
    assert!(listed.contains(&ids[0].1.to_string()), "{listed}");
    assert_eq!(exit_code(&all), 1, "--all changes no exit status");

    // finished, but created within the quiet days: something started it lately, so it still counts
    sqlx::query("UPDATE runs SET status = 'CANCELLED', finished_at = now() WHERE workflow_id = $1")
        .bind(ids[1].0.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("the run finished, written as the owner");
    let recent = input_schema_inventory(&as_runtime(&w)).await;
    assert_eq!(exit_code(&recent), 1, "a run created lately: {recent:?}");
    // created longer ago than the quiet days, nothing starts the old version of `busy-flow` either: exit 0
    sqlx::query("UPDATE runs SET created_at = now() - interval '40 days' WHERE workflow_id = $1")
        .bind(ids[1].0.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("the run was created long ago, written as the owner");
    let quiet = input_schema_inventory(&as_runtime(&w)).await;
    assert_eq!(exit_code(&quiet), 0, "{quiet:?}");
    assert_eq!(quiet.as_ref().expect("runs").superseded.len(), 2);
}

/// Re-read of design 043 after round 2 (correctness 3, architecture 4): a schedule that pins a version keeps starting it
/// whatever its workflow's active version is and however seldom it fires, so a version outside the subset that a live
/// schedule pins is never superseded: the inventory counts it and exits 1.
#[tokio::test]
async fn a_version_a_live_schedule_pins_is_never_superseded() {
    let f = crate::support::schedules::fixture().await;
    let w = &*f.w;
    let defs = DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec)
        .with_input_validation(InputValidationMode::Report);
    let old = with_schema("pinned-flow", outside(), None);
    let created = defs
        .create_workflow(
            &w.admin,
            "payments",
            "pinned-flow",
            &bytes(&old),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("a draft");
    let workflow = created.workflow.id();
    let first = defs
        .publish(
            &w.admin,
            workflow,
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("stored outside the subset")
        .version
        .id();
    let record = defs
        .get_workflow(&w.admin, workflow, &ctx())
        .await
        .expect("the workflow");
    let record = defs
        .update_draft(
            &w.admin,
            workflow,
            record.workflow.draft_revision(),
            &bytes(&with_schema("pinned-flow", json!({"type": "object"}), None)),
            &ctx(),
        )
        .await
        .expect("fixed");
    let second = defs
        .publish(
            &w.admin,
            workflow,
            record.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("the fixed version is active")
        .version
        .id();
    // a quarterly schedule pinned to the old version (written pinned to the new one, which the write check admits, then
    // re-pinned to the old one as a schedule written before 0.3.0 would be)
    let schedule = f
        .create(
            "pinned-quarterly",
            &json!({"intervals": [{"everyMs": 7_776_000_000_u64}]}),
            &json!({}),
            Some(json!({
                "workflowId": workflow.to_string(),
                "versionPolicy": "PINNED",
                "versionId": second.to_string(),
            })),
        )
        .await;
    // the revisions are append-only: the owner re-pins this one with that guard off for its transaction alone
    let mut tx = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("ALTER TABLE schedule_revisions DISABLE TRIGGER schedule_revisions_append_only")
        .execute(&mut *tx)
        .await
        .expect("the guard off");
    sqlx::query(
        "UPDATE schedule_revisions SET action = jsonb_set(action, '{versionId}', to_jsonb($1::text)) WHERE schedule_id = $2",
    )
    .bind(first.to_string())
    .bind(schedule.id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("re-pinned, written as the owner");
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .expect("the deferred quota flush runs now");
    sqlx::query(
        "ALTER TABLE schedule_revisions ENABLE ALWAYS TRIGGER schedule_revisions_append_only",
    )
    .execute(&mut *tx)
    .await
    .expect("the guard on again");
    tx.commit().await.expect("committed");

    let found = input_schema_inventory(&as_runtime(w)).await;
    let inventory = found.as_ref().expect("the inventory runs");
    assert!(
        inventory
            .refused
            .iter()
            .any(|refused| refused.ids.version == first),
        "the pinned old version still counts: {inventory:?}"
    );
    assert!(
        !inventory
            .superseded
            .iter()
            .any(|refused| refused.ids.version == first),
        "{inventory:?}"
    );
    assert_eq!(exit_code(&found), 1);
}

/// The stamp of the last migration of v0.2.0, the schema the inventory reads before the upgrade.
const V0_2_0_LAST_MIGRATION: i64 = 20_261_220_000_000;

/// Second re-read of design 043 (correctness, new Major): the inventory runs before the migration, on a 0.2.x database,
/// which has no `schedules` table (feature 018 is not in a 0.2.x release). There a version outside the subset is listed
/// and the command exits 1, never 2 for a missing table.
#[tokio::test]
async fn the_inventory_runs_on_a_database_of_0_2_0_before_the_migration() {
    let w = world().await;
    publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema("old-schema-flow", outside(), None),
    )
    .await
    .expect("stored outside the subset");
    sagat_infra_postgres::MIGRATOR
        .undo(&w.db.pool, V0_2_0_LAST_MIGRATION)
        .await
        .expect("back to the schema of v0.2.0");
    let schedules: bool = sqlx::query_scalar("SELECT to_regclass('public.schedules') IS NOT NULL")
        .fetch_one(&w.db.pool)
        .await
        .expect("catalog");
    assert!(!schedules, "a database of v0.2.0 has no schedules");
    let found = input_schema_inventory(&as_runtime(&w)).await;
    assert_eq!(exit_code(&found), 1, "{found:?}");
    assert_eq!(found.as_ref().expect("runs").refused.len(), 1);
}
