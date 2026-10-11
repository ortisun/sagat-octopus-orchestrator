//! US01: the compensation plan is recorded when a version is published (FR-003-001, FR-003-002).

use sagat_application::definitions::IrCodec;
use sagat_application::execution::compensation_id;
use sagat_domain::saga::compensation_plans;
use sagat_infra_ir::JsonIrCodec;
use serde_json::json;

use crate::settlement;
use crate::support::engine::{activity, document, effect_node, saga_node, world};

#[derive(sqlx::FromRow)]
struct EntryRow {
    position: i32,
    node: String,
    compensation_id: uuid::Uuid,
    compensation_activity: String,
    after: Vec<String>,
}

#[tokio::test]
async fn publishing_a_saga_records_its_plan_with_durable_compensation_ids() {
    let w = world().await;
    let (workflow, version) = w.publish(settlement()).await;

    let plans: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT saga_node, on_compensation_failure, cancel_policy FROM saga_plans \
         WHERE workflow_version_id = $1",
    )
    .bind(version.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("plans");
    assert_eq!(
        plans,
        vec![(
            "settlement".to_owned(),
            "manual_review".to_owned(),
            "compensate_completed".to_owned()
        )]
    );

    let entries: Vec<EntryRow> = sqlx::query_as(
        "SELECT position, node, compensation_id, compensation_activity, after FROM saga_plan_entries \
         WHERE workflow_version_id = $1 ORDER BY position",
    )
    .bind(version.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("entries");
    // Reverse order: `submit` is undone first and `reserve` waits for it.
    let rows: Vec<_> = entries
        .iter()
        .map(|e| {
            (
                e.position,
                e.node.as_str(),
                e.compensation_activity.as_str(),
                e.after.clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (0, "submit", "undo.submit", vec![]),
            (1, "reserve", "undo.reserve", vec!["submit".to_owned()]),
        ]
    );
    // The ids are those a run uses, stable for the version and different for every step.
    for entry in &entries {
        let node = sagat_domain::workflow::NodeId::parse(&entry.node).expect("node");
        assert_eq!(entry.compensation_id, compensation_id(version, &node));
    }
    assert_ne!(entries[0].compensation_id, entries[1].compensation_id);
    let _ = workflow;
}

#[tokio::test]
async fn the_stored_plan_is_exactly_what_the_definition_derives() {
    let w = world().await;
    let (_, version) = w.publish(settlement()).await;
    let document: Vec<u8> =
        sqlx::query_scalar("SELECT document FROM workflow_versions WHERE id = $1")
            .bind(version.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("document");
    let parsed = JsonIrCodec
        .parse(&document)
        .expect("the stored document parses");
    let derived = compensation_plans(&parsed.definition);
    assert_eq!(derived.len(), 1);
    let stored: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT node, after FROM saga_plan_entries WHERE workflow_version_id = $1 ORDER BY position",
    )
    .bind(version.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("entries");
    let expected: Vec<(String, Vec<String>)> = derived[0]
        .entries
        .iter()
        .map(|e| {
            (
                e.node.as_str().to_owned(),
                e.after.iter().map(|n| n.as_str().to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(stored, expected);
}

#[tokio::test]
async fn publishing_the_same_draft_again_does_not_duplicate_the_plan() {
    let w = world().await;
    let (workflow, version) = w.publish(settlement()).await;
    // The same document: the existing version is re-activated, and nothing is stored twice.
    let again = w
        .defs
        .publish(
            &w.admin,
            workflow,
            1,
            None,
            "publish-again-0001",
            &crate::support::engine::ctx(),
        )
        .await
        .expect("publish again");
    assert!(!again.created);
    assert_eq!(again.version.id(), version);
    let plans: i64 = sqlx::query_scalar("SELECT count(*) FROM saga_plans")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(plans, 1);
}

#[tokio::test]
async fn a_new_version_gets_a_plan_and_ids_of_its_own() {
    let w = world().await;
    let (workflow, first) = w.publish(settlement()).await;
    // A saga gets a third compensable step in the second version.
    let second_doc = document(
        "settlement",
        "settlement",
        vec![
            saga_node("settlement", "reserve", Some("notify")),
            effect_node("reserve", Some("book"), false),
            effect_node("book", Some("submit"), false),
            effect_node("submit", None, true),
            activity("notify"),
        ],
    );
    let second = w.publish_new_version(workflow, second_doc).await;
    let count = |version: sagat_domain::definition::WorkflowVersionId| {
        let pool = w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM saga_plan_entries WHERE workflow_version_id = $1",
            )
            .bind(version.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count")
        }
    };
    assert_eq!((count(first).await, count(second).await), (2, 3));
    // The first version's plan did not change, and `reserve` has a different id in each.
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT compensation_id FROM saga_plan_entries WHERE node = 'reserve' ORDER BY workflow_version_id",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("ids");
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "an id belongs to a version");
}

#[tokio::test]
async fn a_workflow_without_a_saga_has_no_plan() {
    let w = world().await;
    let (_, version) = w.publish(document("plain", "a", vec![activity("a")])).await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saga_plans WHERE workflow_version_id = $1")
            .bind(version.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_stored_plan_can_be_neither_changed_nor_removed_by_anyone() {
    let w = world().await;
    let (_, version) = w.publish(settlement()).await;
    for statement in [
        "UPDATE saga_plan_entries SET compensation_activity = 'undo.other'",
        "DELETE FROM saga_plan_entries",
        "UPDATE saga_plans SET cancel_policy = 'abort_no_compensation'",
        "DELETE FROM saga_plans",
        "TRUNCATE saga_plan_entries",
        "TRUNCATE saga_plans",
    ] {
        let refused = sqlx::query(statement).execute(&w.db.pool).await;
        let message = refused.expect_err(statement).to_string();
        // The trigger refuses it, or (for TRUNCATE of a table another one references) PostgreSQL
        // does before the trigger is reached: either way it does not happen.
        assert!(
            message.contains("append-only")
                || message.contains("cannot truncate a table referenced"),
            "{statement}: {message}"
        );
    }
    let still: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saga_plan_entries WHERE workflow_version_id = $1")
            .bind(version.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(still, 2);
}

#[tokio::test]
async fn a_plan_cannot_point_into_another_tenants_version() {
    let w = world().await;
    let (_, version) = w.publish(settlement()).await;
    // The composite foreign key refuses a plan whose tenant is not the version's.
    let other = w.other_admin.tenant_id();
    let refused = sqlx::query(
        "INSERT INTO saga_plans (tenant_id, workflow_version_id, saga_node, on_compensation_failure, cancel_policy) \
         VALUES ($1, $2, 'ghost', 'fail', 'compensate_completed')",
    )
    .bind(other.as_uuid())
    .bind(version.as_uuid())
    .execute(&w.db.pool)
    .await;
    assert!(refused.is_err());
    let _ = json!({});
}
