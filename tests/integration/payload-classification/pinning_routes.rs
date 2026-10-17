//! Rules-version pinning through approval and promotion publication paths.
#![allow(clippy::expect_used)]

use sagat_application::publishing::Promotion;
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{Environment, Role};
use sagat_domain::versioning::Verdict;
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::engine::{World, ctx, document, end_node, world};

#[derive(Debug, PartialEq, Eq)]
struct StoredVersion {
    id: WorkflowVersionId,
    number: i32,
    digest: String,
    document: Vec<u8>,
    rules: i16,
}

fn classified_document(name: &str, marker: &str) -> Value {
    let mut value = document(name, "done", vec![end_node("done")]);
    value["metadata"]["description"] = json!(marker);
    value["inputSchema"] = json!({
        "type": "object",
        "additionalProperties": false,
        "x-sagat-classification": "public",
        "properties": {
            "personal": {
                "type": "string",
                "x-sagat-classification": "pii"
            }
        }
    });
    value
}

async fn stored_version(w: &World, id: WorkflowVersionId) -> StoredVersion {
    let (stored_id, number, digest, document, rules): (uuid::Uuid, i32, String, Vec<u8>, i16) =
        sqlx::query_as(
            "SELECT id,version_number,digest,document,classification_rules \
             FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("stored workflow version");
    StoredVersion {
        id: WorkflowVersionId::new(stored_id),
        number,
        digest,
        document,
        rules,
    }
}

async fn approved_publish(
    w: &World,
    workflow: WorkflowId,
    revision: u64,
    key: &str,
) -> WorkflowVersionId {
    let author = w.member(&format!("author-{key}"), Role::Developer).await;
    let reviewer = w.member(&format!("reviewer-{key}"), Role::Developer).await;
    let request = w
        .publishing
        .open_request(
            &author,
            workflow,
            revision,
            Some("classified release"),
            &ctx(),
        )
        .await
        .expect("publication request");
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "classification reviewed",
            &ctx(),
        )
        .await
        .expect("approved publication request");
    w.publishing
        .publish_request(
            &author,
            request.id(),
            &format!("publish-{key}-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("approved publication")
        .version
        .id()
}

#[tokio::test]
async fn approved_publications_pin_rules_one_and_never_rewrite_the_old_version() {
    let w = world().await;
    let prod = w
        .add_namespace_in("classified-prod", Environment::Prod)
        .await;
    let first_bytes = serde_json::to_vec(&classified_document("approval-pin", "first"))
        .expect("first document bytes");
    let workflow = w
        .defs
        .create_workflow(
            &w.admin,
            prod.slug().as_str(),
            "approval-pin",
            &first_bytes,
            "create-approval-pin",
            &ctx(),
        )
        .await
        .expect("production workflow");
    let workflow_id = workflow.workflow.id();
    let first = approved_publish(
        &w,
        workflow_id,
        workflow.workflow.draft_revision(),
        "approval-v1",
    )
    .await;
    let first_snapshot = stored_version(&w, first).await;
    assert_eq!(first_snapshot.rules, 1);
    assert_eq!(first_snapshot.document, first_bytes);

    let current = w
        .defs
        .get_workflow(&w.admin, workflow_id, &ctx())
        .await
        .expect("current workflow");
    let second_bytes = serde_json::to_vec(&classified_document("approval-pin", "second"))
        .expect("second document bytes");
    let edited = w
        .defs
        .update_draft(
            &w.admin,
            workflow_id,
            current.workflow.draft_revision(),
            &second_bytes,
            &ctx(),
        )
        .await
        .expect("second draft");
    let second = approved_publish(
        &w,
        workflow_id,
        edited.workflow.draft_revision(),
        "approval-v2",
    )
    .await;
    let second_stored = stored_version(&w, second).await;

    assert_ne!(first, second);
    assert_eq!((second_stored.number, second_stored.rules), (2, 1));
    assert_eq!(second_stored.document, second_bytes);
    assert_eq!(
        stored_version(&w, first).await,
        first_snapshot,
        "a later approved publication cannot rewrite the old bytes, digest, or rules pin"
    );
}

#[tokio::test]
async fn direct_and_approved_promotions_keep_identical_bytes_and_rules_one() {
    let w = world().await;
    let staging = w
        .add_namespace_in("classified-staging", Environment::Staging)
        .await;
    let prod = w
        .add_namespace_in("classified-prod", Environment::Prod)
        .await;
    let author = w.member("promotion-author", Role::Developer).await;
    let reviewer = w.member("promotion-reviewer", Role::Developer).await;
    let source_document = classified_document("promotion-pin", "release");
    let (source_workflow, source_version_id) = w.publish(source_document).await;
    let source = stored_version(&w, source_version_id).await;
    assert_eq!(source.rules, 1);

    let to_staging = w
        .publishing
        .promote(
            &author,
            source_workflow,
            1,
            staging.id(),
            "classified payload verified in development",
            false,
            "promote-classified-staging",
            &ctx(),
        )
        .await
        .expect("direct staging promotion");
    let Promotion::Published(staging_publication) = to_staging else {
        panic!("staging promotion must publish immediately")
    };
    let staged = stored_version(&w, staging_publication.version.id()).await;
    assert_eq!(staged.rules, 1);
    assert_eq!(
        (&staged.digest, &staged.document),
        (&source.digest, &source.document)
    );

    let to_prod = w
        .publishing
        .promote(
            &author,
            staging_publication.version.workflow_id(),
            1,
            prod.id(),
            "classified payload verified in staging",
            false,
            "promote-classified-production",
            &ctx(),
        )
        .await
        .expect("production promotion request");
    let Promotion::Requested(request) = to_prod else {
        panic!("production promotion must require approval")
    };
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "classification and digest reviewed",
            &ctx(),
        )
        .await
        .expect("production promotion approval");
    let production = w
        .publishing
        .publish_request(
            &author,
            request.id(),
            "publish-classified-production",
            &ctx(),
        )
        .await
        .expect("approved production promotion");
    let production = stored_version(&w, production.version.id()).await;
    assert_eq!(production.rules, 1);
    assert_eq!(
        (&production.digest, &production.document),
        (&source.digest, &source.document),
        "promotion must keep the source version byte-for-byte"
    );
    assert_eq!(stored_version(&w, source_version_id).await, source);
}
