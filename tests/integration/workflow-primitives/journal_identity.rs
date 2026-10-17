//! Root identity dual writes and legacy journal replay.

use sagat_application::execution::instance_step_id;
use sagat_domain::execution::ExecutionInstanceId;
use sagat_domain::workflow::NodeId;
use sagat_infra_postgres::{MIGRATOR, migrate};
use sqlx::Row;
use uuid::Uuid;

use super::identity_storage::insert_legacy_root_run;
use crate::support::engine::{chain, world};

const BEFORE: i64 = 20_261_026_000_000;

#[tokio::test]
async fn newly_persisted_root_steps_and_events_share_the_typed_identity() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("root-journal-identity")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let tenant = w.admin.tenant_id().as_uuid();

    let steps = sqlx::query(
        "SELECT node, step_id, instance_path_nodes, instance_path_indices FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("root steps");
    assert!(!steps.is_empty(), "the run projects at least one step");
    for row in steps {
        let node: String = row.get("node");
        let node = NodeId::parse_stored(&node).expect("stored node");
        let expected = instance_step_id(version, run, &ExecutionInstanceId::root(node));
        assert_eq!(row.get::<Uuid, _>("step_id"), expected);
        assert!(row.get::<Vec<String>, _>("instance_path_nodes").is_empty());
        assert!(row.get::<Vec<i32>, _>("instance_path_indices").is_empty());
    }

    let events = sqlx::query(
        "SELECT node, instance_id, instance_path_nodes, instance_path_indices FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("root journal");
    assert!(!events.is_empty(), "the run has journal events");
    assert!(
        events
            .iter()
            .any(|row| row.get::<Option<String>, _>("node").is_some()),
        "the journal contains a step event"
    );
    for row in events {
        let node: Option<String> = row.get("node");
        let stored_id: Option<Uuid> = row.get("instance_id");
        let path_nodes: Option<Vec<String>> = row.get("instance_path_nodes");
        let path_indices: Option<Vec<i32>> = row.get("instance_path_indices");
        match node {
            Some(node) => {
                let node = NodeId::parse_stored(&node).expect("stored event node");
                let expected = instance_step_id(version, run, &ExecutionInstanceId::root(node));
                assert_eq!(stored_id, Some(expected));
                assert_eq!(path_nodes, Some(Vec::new()));
                assert_eq!(path_indices, Some(Vec::new()));
            }
            None => {
                assert_eq!(stored_id, None);
                assert_eq!(path_nodes, None);
                assert_eq!(path_indices, None);
            }
        }
    }
}

#[tokio::test]
async fn legacy_null_journal_identity_still_replays_as_root() {
    let w = world().await;
    let definition = w.publish(chain("legacy-null-journal-identity")).await;
    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("return to the legacy journal schema");
    let (run, _) = insert_legacy_root_run(&w, definition).await;

    let legacy_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id = $1 AND run_id = $2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("legacy journal rows");
    assert!(legacy_rows > 0);

    migrate(&w.db.pool)
        .await
        .expect("upgrade the legacy journal");
    let identities: Vec<Option<Uuid>> = sqlx::query_scalar(
        "SELECT instance_id FROM run_events WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("upgraded legacy journal");
    assert!(identities.iter().all(Option::is_none));
    let legacy_node_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 AND node IS NOT NULL AND instance_id IS NULL",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("legacy node events");
    assert!(
        legacy_node_rows > 0,
        "a legacy step event retains NULL identity"
    );

    w.assert_consistent(run).await;
}
