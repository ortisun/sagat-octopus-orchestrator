//! Durable switch selection storage against real PostgreSQL.

use sagat_application::runs::RunStore;
use sagat_domain::execution::{RunStatus, SwitchSelection};
use sagat_domain::workflow::{NodeId, NodeKind};
use sagat_infra_postgres::{MIGRATOR, migrate};
use serde_json::{Value, json};
use sqlx::Row;

use crate::support::engine::{activity, ctx, document, world};

const BEFORE: i64 = 20_261_027_000_000;

fn switch(name: &str) -> Value {
    document(
        name,
        "choose",
        vec![
            json!({
                "id": "choose",
                "type": "switch",
                "cases": [
                    {"id": "first", "when": "input.amount > 0", "target": "accepted"},
                    {"id": "second", "when": "input.amount >= 0", "target": "review"}
                ],
                "default": "rejected"
            }),
            activity("accepted"),
            activity("review"),
            activity("rejected"),
        ],
    )
}

async fn column_exists(pool: &sqlx::PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'steps' \
           AND column_name = 'switch_case_id')",
    )
    .fetch_one(pool)
    .await
    .expect("column lookup")
}

#[tokio::test]
async fn case_and_default_choices_survive_projection_reload_and_duplicate_start() {
    let w = world().await;
    let (workflow, _) = w.publish(switch("switch-storage")).await;

    let selected = w
        .start_with(
            workflow,
            &json!({"amount": 5}),
            "switch-selected-once",
            None,
        )
        .await
        .expect("start selected case");
    let duplicate = w
        .start_with(
            workflow,
            &json!({"amount": 5}),
            "switch-selected-once",
            None,
        )
        .await
        .expect("replay duplicate start");
    assert_eq!(duplicate, selected);

    let defaulted = w.start(workflow, &json!({"amount": -1})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    for (run, stored_case, chosen, absent) in [
        (selected, "first", "accepted", ["review", "rejected"]),
        (defaulted, "", "rejected", ["accepted", "review"]),
    ] {
        let choice: String = sqlx::query_scalar(
            "SELECT switch_case_id FROM steps \
             WHERE tenant_id = $1 AND run_id = $2 AND node = 'choose'",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("switch projection");
        assert_eq!(choice, stored_case);

        let event = sqlx::query(
            "SELECT detail FROM run_events \
             WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'switch.evaluated'",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("switch journal");
        assert_eq!(event.len(), 1, "one durable selection");
        let detail: Value = event[0].get("detail");
        let expected = if stored_case.is_empty() {
            json!({"case_id": null})
        } else {
            json!({"case_id": stored_case})
        };
        assert_eq!(detail, expected);

        let chosen_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM steps \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(chosen)
        .fetch_one(&w.db.pool)
        .await
        .expect("chosen successor");
        assert_eq!(chosen_count, 1);
        for node in absent {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM steps \
                 WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
            )
            .bind(tenant)
            .bind(run.as_uuid())
            .bind(node)
            .fetch_one(&w.db.pool)
            .await
            .expect("unselected successor");
            assert_eq!(count, 0, "{node} must not activate");
        }

        let graph = w
            .store
            .graph(w.admin.tenant_id(), run)
            .await
            .expect("load graph")
            .expect("run graph");
        let switch_step = graph
            .steps
            .iter()
            .find(|step| step.node.as_str() == "choose")
            .expect("switch node");
        let expected_selection = if stored_case.is_empty() {
            SwitchSelection::Default
        } else {
            SwitchSelection::Case(NodeId::parse(stored_case).expect("case id"))
        };
        assert_eq!(switch_step.switch_selection, Some(expected_selection));

        let switch_definition = graph
            .definition
            .node(&NodeId::parse("choose").expect("switch id"))
            .expect("switch definition");
        let NodeKind::Switch { cases, default } = switch_definition.kind() else {
            panic!("switch kind");
        };
        assert_eq!(
            cases
                .iter()
                .map(|case| (case.id().as_str(), case.target().as_str()))
                .collect::<Vec<_>>(),
            [("first", "accepted"), ("second", "review")]
        );
        assert_eq!(default.as_str(), "rejected");

        let reset = w
            .runs
            .graph_changes(&w.admin, run, u64::MAX, &ctx())
            .await
            .expect("graph reset");
        assert!(reset.reset);
        let reset_switch = reset
            .graph
            .steps
            .iter()
            .find(|step| step.node.as_str() == "choose")
            .expect("reset switch node");
        assert_eq!(reset_switch.switch_selection, switch_step.switch_selection);

        w.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn predicate_error_fails_without_recording_a_choice_or_activating_a_successor() {
    let w = world().await;
    let mut definition = switch("switch-expression-error");
    definition["nodes"][0]["cases"][0]["when"] = json!("input.missing > 0");
    let (workflow, _) = w.publish(definition).await;
    let run = w.start(workflow, &json!({"amount": 5})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let switch_row: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status, failure_code, switch_case_id FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND node = 'choose'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("failed switch projection");
    assert_eq!(switch_row.0, "FAILED");
    assert_eq!(switch_row.1.as_deref(), Some("EXPRESSION_ERROR"));
    assert_eq!(switch_row.2, None);

    let choice_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'switch.evaluated'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("choice events");
    assert_eq!(choice_events, 0);
    let successors: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND node IN ('accepted', 'review', 'rejected')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("successor projections");
    assert_eq!(successors, 0);
    let hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox \
         WHERE tenant_id = $1 AND aggregate_id = $2 AND event_type = 'task.ready'",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("task hints");
    assert_eq!(hints, 0);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn failed_successor_hint_rolls_back_the_choice_and_retry_commits_it_once() {
    let w = world().await;
    let (workflow, _) = w.publish(switch("switch-rollback")).await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_switch_hint() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected task hint failure'; END $$; \
         CREATE TRIGGER reject_switch_hint BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type = 'task.ready') \
         EXECUTE FUNCTION reject_switch_hint()",
    )
    .execute(&w.db.pool)
    .await
    .expect("install rollback fault");

    w.start_with(
        workflow,
        &json!({"amount": 5}),
        "switch-rollback-once",
        None,
    )
    .await
    .expect_err("task hint failure rolls the transition back");

    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE workflow_id = $1")
        .bind(workflow.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("rolled back runs");
    assert_eq!(retained, 0, "the failed start leaves no partial run");

    sqlx::raw_sql(
        "DROP TRIGGER reject_switch_hint ON event_outbox; \
         DROP FUNCTION reject_switch_hint()",
    )
    .execute(&w.db.pool)
    .await
    .expect("remove rollback fault");

    let run = w
        .start_with(
            workflow,
            &json!({"amount": 5}),
            "switch-rollback-once",
            None,
        )
        .await
        .expect("retry after rollback");
    let tenant = w.admin.tenant_id().as_uuid();
    let durable: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM run_events \
            WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'switch.evaluated'), \
           (SELECT count(*) FROM steps \
            WHERE tenant_id = $1 AND run_id = $2 AND node = 'accepted'), \
           (SELECT count(*) FROM event_outbox \
            WHERE tenant_id = $1 AND aggregate_id = $2 AND event_type = 'task.ready')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("committed switch transition");
    assert_eq!(durable, (1, 1, 1));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn migration_preserves_legacy_nulls_rejects_invalid_cases_and_guards_rollback() {
    let w = world().await;
    let (legacy_workflow, _) = w
        .publish(crate::support::engine::chain("pre-switch-storage"))
        .await;
    let legacy = w.start(legacy_workflow, &json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let before: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT node, switch_case_id FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
    )
    .bind(tenant)
    .bind(legacy.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("legacy projections");
    assert!(before.iter().all(|(_, selection)| selection.is_none()));

    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("remove unused switch projection");
    assert!(!column_exists(&w.db.pool).await);
    migrate(&w.db.pool)
        .await
        .expect("restore switch projection migration");
    assert!(column_exists(&w.db.pool).await);
    let after: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT node, switch_case_id FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
    )
    .bind(tenant)
    .bind(legacy.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("upgraded projections");
    assert_eq!(after, before);

    let invalid = sqlx::query(
        "UPDATE steps SET switch_case_id = 'invalid/case' \
         WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(tenant)
    .bind(legacy.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("invalid case id must fail");
    assert_eq!(
        invalid
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("23514")
    );

    let (workflow, _) = w.publish(switch("rollback-guard")).await;
    w.start(workflow, &json!({"amount": 1})).await;
    let rollback = MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect_err("recorded switch decisions require a compatible reader");
    assert!(rollback.to_string().contains("switch selection is in use"));
}
