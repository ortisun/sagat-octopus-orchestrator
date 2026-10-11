//! Requests to publish: the same person deciding twice, a request published twice, and an edit
//! racing an approval.

use futures_util::future::join_all;
use sagat_application::error::{AppError, Refusal};
use sagat_domain::identity::Environment;
use sagat_domain::versioning::{RequestState, Verdict, VersioningError};
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

#[tokio::test]
async fn a_request_is_never_left_open_for_a_revision_that_was_saved_over() {
    let w = world().await;
    w.add_namespace_in("billing-prod", Environment::Prod).await;
    let pipeline = w
        .member("pipeline", sagat_domain::identity::Role::Developer)
        .await;
    let editor = w
        .member("erin", sagat_domain::identity::Role::Developer)
        .await;
    // The same race many times: a pipeline asks for revision 1 while somebody saves revision 2.
    for n in 0..24 {
        let name = format!("flow-{n}");
        let mut first = doc(&format!("one-{n}"));
        first["metadata"]["name"] = json!(name);
        let created = w
            .defs
            .create_workflow(
                &w.admin,
                "billing-prod",
                &name,
                &serde_json::to_vec(&first).expect("json"),
                &format!("create-{n}-0123456789"),
                &ctx(),
            )
            .await
            .expect("draft");
        let mut renamed = doc(&format!("two-{n}"));
        renamed["metadata"]["name"] = json!(name);
        let renamed = serde_json::to_vec(&renamed).expect("json");
        let context = ctx();
        let open = w
            .publishing
            .open_request(&pipeline, created.workflow.id(), 1, None, &context);
        let save = w
            .defs
            .update_draft(&editor, created.workflow.id(), 1, &renamed, &context);
        let (opened, saved) = futures_util::future::join(open, save).await;
        saved.expect("the edit goes through");
        // Whichever came first, a request that is open is about the draft that is there.
        match opened {
            Ok(_) | Err(AppError::RevisionConflict { .. }) => {}
            Err(other) => panic!("opening: {other:?}"),
        }
    }
    let stale: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM publication_requests r JOIN workflows f ON f.id = r.workflow_id \
         WHERE r.state IN ('PENDING', 'APPROVED') AND r.draft_revision <> f.draft_revision",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        stale, 0,
        "no request waits for approval of a document that is gone"
    );
}

#[tokio::test]
async fn a_retry_storm_with_one_key_gets_one_answer_and_later_the_same_one() {
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

    // Eight clients publish it with the key of one publication whose answer was lost.
    let storm = join_all((0..8).map(|_| {
        let (w, author) = (&w, &author);
        async move {
            w.publishing
                .publish_request(author, request_id, "key-same-0123456789", &ctx())
                .await
        }
    }))
    .await;
    let answers: std::collections::BTreeSet<_> = storm
        .iter()
        .map(|r| r.as_ref().expect("an answer, not a refusal").version.id())
        .collect();
    assert_eq!(answers.len(), 1, "one answer for one key");
    assert_eq!(versions(&w, id).await, 1);

    // The answer that was lost is given again later with the same key, and no other key is a repeat.
    let again = w
        .publishing
        .publish_request(&author, request_id, "key-same-0123456789", &ctx())
        .await
        .expect("the key of the publication answers again");
    assert_eq!(Some(&again.version.id()), answers.iter().next());

    // The answer must remain recoverable after another editor saves a new, even invalid, draft.
    // Retrying a committed request neither validates that draft nor publishes it.
    w.defs
        .update_draft(&author, id, request.draft_revision(), b"{}", &ctx())
        .await
        .expect("save the next draft");
    let recovered = w
        .publishing
        .publish_request(&author, request_id, "key-same-0123456789", &ctx())
        .await
        .expect("a saved draft cannot erase the answer to a committed publication");
    assert_eq!(recovered.version.id(), again.version.id());
    assert!(!recovered.created);
    assert_eq!(versions(&w, id).await, 1);
    assert_eq!(crate::chain(&w, id).await, vec![(1, None)]);
    let publications: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'workflow.published'",
    )
    .bind(author.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("publication audit count");
    assert_eq!(
        publications, 1,
        "recovery does not audit another publication"
    );
    assert_eq!(
        w.publishing
            .get_request(&author, request_id, &ctx())
            .await
            .expect("request")
            .state(),
        RequestState::Published
    );
    let refused = w
        .publishing
        .publish_request(&author, request_id, "key-another-0123456789", &ctx())
        .await
        .expect_err("a published request is not published again by another key");
    assert!(
        matches!(
            refused,
            AppError::Versioning(Refusal::Request(VersioningError::NotInState(
                RequestState::Published
            )))
        ),
        "{refused:?}"
    );
}
