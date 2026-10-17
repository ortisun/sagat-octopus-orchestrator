//! Immutable interpreter rules belong to the published version, never the current server.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::support::engine::{activity, document, world};
use serde_json::json;

const UP: &str = include_str!(
    "../../../crates/infra-postgres/migrations/20261201000000_payload_classification.up.sql"
);
const DOWN: &str = include_str!(
    "../../../crates/infra-postgres/migrations/20261201000000_payload_classification.down.sql"
);

#[tokio::test]
async fn migration_defaults_existing_rows_to_legacy_and_keeps_document_bytes() {
    let w = world().await;
    let source = document("classification-upgrade", "a", vec![activity("a")]);
    let (workflow, version) = w.publish(source.clone()).await;
    let before: (String, Vec<u8>) =
        sqlx::query_as("SELECT digest,document FROM workflow_versions WHERE id=$1")
            .bind(version.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    // Exercise the exact up/down DDL against retained rows, including the existing always-on
    // immutability trigger. Full old-binary compatibility is a separate release gate.
    sqlx::raw_sql(DOWN).execute(&w.db.pool).await.unwrap();
    let present: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='workflow_versions' AND column_name='classification_rules')")
        .fetch_one(&w.db.pool).await.unwrap();
    assert!(!present);
    sqlx::raw_sql(UP).execute(&w.db.pool).await.unwrap();
    let after: (String, Vec<u8>, i16) = sqlx::query_as(
        "SELECT digest,document,classification_rules FROM workflow_versions WHERE id=$1",
    )
    .bind(version.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!((&before.0, &before.1), (&after.0, &after.1));
    assert_eq!(after.2, 0);
    assert!(
        sqlx::query("UPDATE workflow_versions SET classification_rules=1 WHERE id=$1")
            .bind(version.as_uuid())
            .execute(&w.db.pool)
            .await
            .is_err()
    );

    let same = w.publish_new_version(workflow, source.clone()).await;
    assert_eq!(
        same, version,
        "digest deduplication preserves the existing immutable version"
    );
    let mut changed = source;
    changed["inputSchema"] = json!({"type":"object","x-sagat-classification":"public"});
    let next = w.publish_new_version(workflow, changed).await;
    let rules: Vec<i16> = sqlx::query_scalar("SELECT classification_rules FROM workflow_versions WHERE workflow_id=$1 ORDER BY version_number")
        .bind(workflow.as_uuid()).fetch_all(&w.db.pool).await.unwrap();
    assert_ne!(next, version);
    assert_eq!(rules, vec![0, 1]);
}

#[tokio::test]
async fn new_publications_pin_rules_without_reinterpreting_previous_versions() {
    let w = world().await;
    let first_document = document("classification-pins", "a", vec![activity("a")]);
    let (workflow, first) = w.publish(first_document.clone()).await;
    let rules: i16 = sqlx::query_scalar(
        "SELECT classification_rules FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(first.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(rules, 1);

    let before: (String, Vec<u8>) = sqlx::query_as(
        "SELECT digest,document FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(first.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    let mut next_document = first_document;
    next_document["inputSchema"] = json!({"type":"object","x-sagat-classification":"public"});
    let second = w.publish_new_version(workflow, next_document).await;
    assert_ne!(first, second);
    let rows: Vec<(i32,i16)> = sqlx::query_as(
        "SELECT version_number,classification_rules FROM workflow_versions WHERE tenant_id=$1 AND workflow_id=$2 ORDER BY version_number",
    ).bind(w.admin.tenant_id().as_uuid()).bind(workflow.as_uuid())
        .fetch_all(&w.db.pool).await.unwrap();
    assert_eq!(rows, vec![(1, 1), (2, 1)]);
    let after: (String, Vec<u8>) = sqlx::query_as(
        "SELECT digest,document FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(first.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    assert!(
        sqlx::query(
            "UPDATE workflow_versions SET classification_rules=0 WHERE tenant_id=$1 AND id=$2"
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(first.as_uuid())
        .execute(&w.db.pool)
        .await
        .is_err(),
        "published rules stay immutable"
    );
}
