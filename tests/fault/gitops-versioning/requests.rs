//! Requests to publish: the same person deciding twice, a request published twice, and an edit
//! racing an approval.

use futures_util::future::join_all;
use sagat_application::error::{AppError, Refusal};
use sagat_domain::identity::Environment;
use sagat_domain::versioning::{RequestState, Verdict};
use serde_json::json;

use crate::support::engine::{World, ctx, world};

fn doc(marker: &str) -> serde_json::Value {
    let mut doc =
        crate::support::engine::document("flow", "a", vec![crate::support::engine::activity("a")]);
    doc["metadata"]["description"] = json!(marker);
    doc
}

async fn requested(
    w: &World,
) -> (
    sagat_domain::definition::WorkflowId,
    sagat_domain::versioning::PublicationRequest,
    sagat_domain::identity::Principal,
) {
    w.add_namespace_in("billing-prod", Environment::Prod).await;
    let author = w
        .member("author", sagat_domain::identity::Role::Developer)
        .await;
    let record = w
        .defs
        .create_workflow(
            &w.admin,
            "billing-prod",
            "flow",
            &serde_json::to_vec(&doc("one")).expect("json"),
            "create-0123456789",
            &ctx(),
        )
        .await
        .expect("draft");
    let id = record.workflow.id();
    let request = w
        .publishing
        .open_request(&author, id, record.workflow.draft_revision(), None, &ctx())
        .await
        .expect("request");
    (id, request, author)
}

async fn versions(w: &World, id: sagat_domain::definition::WorkflowId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM workflow_versions WHERE workflow_id = $1")
        .bind(id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn a_person_who_approves_at_once_from_two_tabs_approves_once() {
    let w = world().await;
    let (_, request, _) = requested(&w).await;
    let request_id = request.id();
    w.publishing
        .set_policy(
            &w.admin,
            request.namespace_id(),
            3,
            "three for this one",
            &ctx(),
        )
        .await
        .expect("policy");
    let reviewer = w
        .member("reviewer", sagat_domain::identity::Role::Developer)
        .await;

    let decisions = join_all((0..8).map(|_| {
        let (w, reviewer) = (&w, &reviewer);
        async move {
            w.publishing
                .decide(
                    reviewer,
                    request_id,
                    Verdict::Approve,
                    "looks right to me",
                    &ctx(),
                )
                .await
        }
    }))
    .await;
    assert_eq!(decisions.iter().filter(|d| d.is_ok()).count(), 1);
    for refused in decisions.iter().filter_map(|d| d.as_ref().err()) {
        assert!(
            matches!(refused, AppError::Versioning(Refusal::Request(_))),
            "{refused:?}"
        );
    }
    let stored = w
        .publishing
        .get_request(&w.admin, request_id, &ctx())
        .await
        .expect("request");
    assert_eq!(stored.approvals(), 1, "one person is one approval");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'publication.approved'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1, "and one line of the trail");
}

#[tokio::test]
async fn an_approved_request_published_at_once_makes_one_version() {
    let w = world().await;
    let (id, request, author) = requested(&w).await;
    let request_id = request.id();
    let reviewer = w
        .member("reviewer", sagat_domain::identity::Role::Developer)
        .await;
    w.publishing
        .decide(
            &reviewer,
            request_id,
            Verdict::Approve,
            "looks right to me",
            &ctx(),
        )
        .await
        .expect("approved");

    // Six clients publish it with the same key (a retry storm), six with different keys (six
    // people clicking the same button).
    let same = join_all((0..6).map(|_| {
        let (w, author) = (&w, &author);
        async move {
            w.publishing
                .publish_request(author, request_id, "key-same-0123456789", &ctx())
                .await
        }
    }));
    let different = join_all((0..6).map(|i| {
        let (w, author) = (&w, &author);
        async move {
            w.publishing
                .publish_request(
                    author,
                    request_id,
                    &format!("key-different-{i}-0123456789"),
                    &ctx(),
                )
                .await
        }
    }));
    let (same, different) = futures_util::future::join(same, different).await;
    let created = same
        .iter()
        .chain(different.iter())
        .filter_map(|r| r.as_ref().ok())
        .filter(|p| p.created)
        .count();
    assert!(created <= 1, "at most one publication made the version");
    assert_eq!(versions(&w, id).await, 1);
    let stored = w
        .publishing
        .get_request(&w.admin, request_id, &ctx())
        .await
        .expect("request");
    assert_eq!(stored.state(), RequestState::Published);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workflow_activations WHERE workflow_id = $1")
            .bind(id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("rows");
    assert_eq!(rows, 1, "and one change of the pointer");
    for refused in same
        .iter()
        .chain(different.iter())
        .filter_map(|r| r.as_ref().err())
    {
        assert!(
            matches!(
                refused,
                AppError::Versioning(_) | AppError::Conflict(_) | AppError::IdempotencyKeyReused
            ),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn an_edit_that_races_an_approval_never_lets_the_request_publish_what_nobody_saw() {
    for _ in 0..4 {
        let w = world().await;
        let (id, request, author) = requested(&w).await;
        let request_id = request.id();
        let reviewer = w
            .member("reviewer", sagat_domain::identity::Role::Developer)
            .await;
        let current = w
            .defs
            .get_workflow(&w.admin, id, &ctx())
            .await
            .expect("workflow");

        let context = ctx();
        let changed = serde_json::to_vec(&doc("changed after the request")).expect("json");
        let edit = w.defs.update_draft(
            &author,
            id,
            current.workflow.draft_revision(),
            &changed,
            &context,
        );
        let approve = w.publishing.decide(
            &reviewer,
            request_id,
            Verdict::Approve,
            "looks right to me",
            &context,
        );
        let (edited, _) = futures_util::future::join(edit, approve).await;
        edited.expect("the edit goes through");

        // Whatever happened to the approval, the request is over and publishing it is refused.
        let stored = w
            .publishing
            .get_request(&w.admin, request_id, &ctx())
            .await
            .expect("request");
        assert_eq!(stored.state(), RequestState::Superseded);
        let refused = w
            .publishing
            .publish_request(&author, request_id, "key-late-0123456789", &ctx())
            .await;
        assert!(refused.is_err(), "a superseded request does not publish");
        assert_eq!(versions(&w, id).await, 0, "nothing was published");
    }
}
