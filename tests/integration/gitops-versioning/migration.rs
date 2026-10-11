//! The migration of feature 011 goes back and forward, and the history that starts with it is
//! built from what existed (T011-006).

use sagat_infra_postgres::{MIGRATOR, migrate};
use serde_json::json;
use sqlx::PgPool;

use crate::common::{doc, operator};
use crate::support::engine::{ctx, world};
use crate::support::migrated_db;

/// The last migration before this feature's.
const BEFORE: i64 = 20_261_015_000_300;

const TABLES: [&str; 5] = [
    "publication_policies",
    "publication_requests",
    "publication_decisions",
    "workflow_activations",
    "workflow_version_scripts",
];

async fn exists(pool: &PgPool, table: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{table}"))
        .fetch_one(pool)
        .await
        .expect("query")
}

#[tokio::test]
async fn rolling_back_removes_what_the_feature_added_and_going_forward_restores_it() {
    let db = migrated_db().await;
    for table in TABLES {
        assert!(exists(&db.pool, table).await, "{table}");
    }
    MIGRATOR.undo(&db.pool, BEFORE).await.expect("roll back");
    for table in TABLES {
        assert!(!exists(&db.pool, table).await, "{table} is gone");
    }
    let functions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc WHERE proname = 'workflow_activations_reject_mutation'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count");
    assert_eq!(functions, 0, "the guard goes with the table");
    migrate(&db.pool).await.expect("forward again");
    for table in TABLES {
        assert!(exists(&db.pool, table).await, "{table} is back");
    }
}

#[tokio::test]
async fn the_history_that_begins_with_the_migration_is_what_the_versions_already_say() {
    let w = world().await;
    // Three versions, the last one rolled back from: the pointer is on version 2.
    let (id, _) = w.publish(doc("flow", "one")).await;
    for (marker, key) in [
        ("two", "key-two-0123456789"),
        ("three", "key-three-0123456789"),
    ] {
        let record = w
            .defs
            .get_workflow(&w.admin, id, &ctx())
            .await
            .expect("workflow");
        let revision = crate::common::edit(
            &w,
            &w.admin,
            id,
            record.workflow.draft_revision(),
            &doc("flow", marker),
        )
        .await;
        w.defs
            .publish(
                &w.admin,
                id,
                revision,
                Some(&format!("release {marker}")),
                key,
                &ctx(),
            )
            .await
            .expect("published");
    }
    let ops = operator(&w, "olivia").await;
    w.publishing
        .activate(
            &ops,
            id,
            2,
            "three misbehaves in production",
            Some(3),
            true,
            &ctx(),
        )
        .await
        .expect("rollback");

    // The migration is undone and applied again, as an upgrade of a platform with this data would.
    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("roll back with data");
    for table in TABLES {
        assert!(!exists(&w.db.pool, table).await, "{table}");
    }
    migrate(&w.db.pool).await.expect("forward again");

    // Every publication was an activation then; the pointer that was moved by hand is closed with
    // one row that says so.
    let rows: Vec<(i32, Option<i32>, String, String)> = sqlx::query_as(
        "SELECT v.version_number, pv.version_number, a.kind, a.actor FROM workflow_activations a \
         JOIN workflow_versions v ON v.id = a.version_id \
         LEFT JOIN workflow_versions pv ON pv.id = a.previous_version_id \
         WHERE a.workflow_id = $1 ORDER BY a.created_at, a.id",
    )
    .bind(id.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("history");
    assert_eq!(
        rows.iter()
            .map(|(v, p, k, _)| (*v, *p, k.as_str()))
            .collect::<Vec<_>>(),
        [
            (1, None, "PUBLISH"),
            (2, Some(1), "PUBLISH"),
            (3, Some(2), "PUBLISH"),
            (2, Some(3), "ACTIVATE")
        ]
    );
    assert_eq!(rows[3].3, "system:migration");
    let history = w
        .publishing
        .activations(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("history");
    assert_eq!(history.items[0].version, 2, "the last row is the pointer");

    // And the platform goes on: a rollback after the upgrade is one more row.
    w.publishing
        .activate(
            &ops,
            id,
            1,
            "back to the first after the upgrade",
            Some(2),
            true,
            &ctx(),
        )
        .await
        .expect("after");
    let history = w
        .publishing
        .activations(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("history");
    assert_eq!(history.items.len(), 5);
    let _ = json!(null);
}
