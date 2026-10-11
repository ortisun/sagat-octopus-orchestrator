//! A version depends on the script artifacts it pins, and the database does not let one be
//! removed while a published version does (FR-011-001, FR-011-006).

use serde_json::json;

use crate::common::operator;
use crate::support::db_error;
use crate::support::engine::{World, ctx, document, world};

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A script of the namespace `payments` with a version that was built into `digest`: what a
/// workflow can pin. Written directly, as a runner's report would leave it.
async fn artifact(w: &World, digest: &str) {
    sqlx::query(
        "INSERT INTO script_artifacts (tenant_id, digest, language, content, signature, key_id, sbom, toolchain) \
         VALUES ($1, $2, 'typescript', '\\x01', decode(repeat('ab', 64), 'hex'), 'key-1', '\\x7b7d', 'node@sha256:aa')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(digest)
    .execute(&w.db.pool)
    .await
    .expect("an artifact");
    let script = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scripts (id, tenant_id, namespace_id, name, language, source, policy, retention_days, created_by) \
         VALUES ($1, $2, $3, 'doubler', 'typescript', 'export default 1', 'v1', 30, 'alice')",
    )
    .bind(script)
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a script");
    sqlx::query(
        "INSERT INTO script_versions \
         (id, tenant_id, namespace_id, script_id, number, language, status, source, policy, retention_days, \
          content_digest, artifact_digest, created_by, created_at) \
         VALUES ($1, $2, $3, $4, 1, 'typescript', 'READY', 'export default 1', 'v1', 30, $5, $5, 'alice', now())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(script)
    .bind(digest)
    .execute(&w.db.pool)
    .await
    .expect("a built version");
}

/// What a retention policy built later would have to do before it could remove an artifact: lift
/// the immutability guards and remove the script versions that were built into it.
async fn retire_script_versions(w: &World) {
    for guard in [
        "ALTER TABLE script_artifacts DISABLE TRIGGER script_artifacts_immutable",
        "ALTER TABLE script_versions DISABLE TRIGGER script_versions_immutable",
        "DELETE FROM script_versions",
    ] {
        sqlx::query(guard)
            .execute(&w.db.pool)
            .await
            .expect("retire");
    }
}

fn pinning(name: &str, digest: &str) -> serde_json::Value {
    document(
        name,
        "s",
        vec![
            json!({"id": "s", "type": "script", "artifactDigest": digest, "language": "typescript"}),
        ],
    )
}

async fn pinned(w: &World, workflow: sagat_domain::definition::WorkflowId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT s.digest FROM workflow_version_scripts s \
         JOIN workflow_versions v ON v.id = s.workflow_version_id WHERE v.workflow_id = $1 ORDER BY 1",
    )
    .bind(workflow.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("dependencies")
}

#[tokio::test]
async fn a_version_records_the_artifacts_it_pins_and_an_artifact_in_use_cannot_be_removed() {
    let w = world().await;
    artifact(&w, DIGEST).await;
    let (id, _) = w.publish(pinning("scripted", DIGEST)).await;
    assert_eq!(pinned(&w, id).await, [DIGEST.to_owned()]);

    // The artifact table refuses a delete on its own (it is immutable)...
    let refused = sqlx::query("DELETE FROM script_artifacts WHERE digest = $1")
        .bind(DIGEST)
        .execute(&w.db.pool)
        .await
        .expect_err("immutable");
    let (code, _) = db_error(refused);
    assert_eq!(code, "23001", "restrict_violation");

    // ...and if a retention policy built later lifted that and removed the script versions that were
    // built into it, the dependency still stops the removal: the foreign key from the workflow
    // version restricts it, because a rollback can make that version active again.
    retire_script_versions(&w).await;
    let refused = sqlx::query("DELETE FROM script_artifacts WHERE digest = $1")
        .bind(DIGEST)
        .execute(&w.db.pool)
        .await
        .expect_err("in use");
    let (code, constraint) = db_error(refused);
    // What PostgreSQL calls a refusal by `ON DELETE RESTRICT` depends on its version: 18 reports
    // `restrict_violation` (23001), 16 reports `foreign_key_violation` (23503). That the
    // constraint is the one named is what is proved.
    assert!(
        code == "23001" || code == "23503",
        "a restrict or foreign key violation, got {code}"
    );
    assert_eq!(
        constraint.as_deref(),
        Some("workflow_version_scripts_artifact_fk")
    );
}

#[tokio::test]
async fn an_artifact_no_workflow_version_pins_is_not_held_by_the_dependencies() {
    let w = world().await;
    artifact(&w, DIGEST).await;
    retire_script_versions(&w).await;
    let removed = sqlx::query("DELETE FROM script_artifacts WHERE digest = $1")
        .bind(DIGEST)
        .execute(&w.db.pool)
        .await
        .expect("nothing pins it")
        .rows_affected();
    assert_eq!(removed, 1);
}

#[tokio::test]
async fn a_rollback_to_a_version_that_pins_an_artifact_finds_it_still_there() {
    let w = world().await;
    artifact(&w, DIGEST).await;
    let (id, _) = w.publish(pinning("scripted", DIGEST)).await;
    // Version 2 stops using the script; the artifact stays because version 1 pins it.
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
        &document("scripted", "a", vec![crate::support::engine::activity("a")]),
    )
    .await;
    w.defs
        .publish(&w.admin, id, revision, None, "key-two-0123456789", &ctx())
        .await
        .expect("v2");
    assert_eq!(
        pinned(&w, id).await,
        [DIGEST.to_owned()],
        "only version 1 pins it"
    );
    retire_script_versions(&w).await;
    assert!(
        sqlx::query("DELETE FROM script_artifacts WHERE digest = $1")
            .bind(DIGEST)
            .execute(&w.db.pool)
            .await
            .is_err()
    );
    let ops = operator(&w, "olivia").await;
    w.publishing
        .activate(
            &ops,
            id,
            1,
            "version 2 dropped the script by mistake",
            Some(2),
            true,
            &ctx(),
        )
        .await
        .expect("the rollback is possible");
}
