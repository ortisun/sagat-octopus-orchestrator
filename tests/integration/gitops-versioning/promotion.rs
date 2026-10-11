//! Promoting a version from one environment to the next: the same document, byte for byte, with
//! the approvals of the environment it reaches and a trail that says where it came from (US03,
//! FR-011-002, FR-011-005).

use sagat_application::error::{AppError, Refusal};
use sagat_application::publishing::Promotion;
use sagat_domain::identity::Environment;
use sagat_domain::versioning::{ActivationKind, RequestKind, RequestState, Verdict};
use serde_json::json;

use crate::common::{by_name, developer, doc, draft, edit, namespace, operator, refusal};
use crate::support::engine::{activity, ctx, document, with, world};

async fn published_in(
    w: &crate::support::engine::World,
    slug: &str,
    name: &str,
    document: &serde_json::Value,
) -> sagat_domain::definition::WorkflowId {
    let record = draft(w, slug, name, document).await;
    w.defs
        .publish(
            &w.admin,
            record.workflow.id(),
            record.workflow.draft_revision(),
            None,
            &format!("publish-{slug}-{name}-0123456789"),
            &ctx(),
        )
        .await
        .expect("published");
    record.workflow.id()
}

#[tokio::test]
async fn a_version_goes_from_dev_to_staging_to_production_as_the_same_document_with_its_trail() {
    let w = world().await;
    let dev = namespace(&w, "shop-dev", Environment::Dev).await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let prod = namespace(&w, "shop-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let reviewer = developer(&w, "reviewer").await;
    let source = published_in(&w, "shop-dev", "checkout", &doc("checkout", "release 7")).await;
    let source_version = w
        .defs
        .get_version(&w.admin, source, 1, &ctx())
        .await
        .expect("v1");

    // Staging asks for nothing: the same document is published there at once.
    let to_staging = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "tried in dev for a week",
            false,
            "promote-staging-0123456789",
            &ctx(),
        )
        .await
        .expect("promoted");
    let Promotion::Published(published) = to_staging else {
        panic!("expected a publication, got {to_staging:?}");
    };
    assert_eq!(published.version.digest(), source_version.version.digest());
    let staging_workflow = by_name(&w, staging.id(), "checkout").await;
    let staged = w
        .defs
        .get_version(&w.admin, staging_workflow, 1, &ctx())
        .await
        .expect("the version in staging");
    assert_eq!(staged.document, source_version.document, "the same bytes");

    // Production asks for an approval: the promotion becomes a request bound to that document.
    let to_prod = w
        .publishing
        .promote(
            &author,
            staging_workflow,
            1,
            prod.id(),
            "staging is green",
            false,
            "promote-prod-0123456789",
            &ctx(),
        )
        .await
        .expect("requested");
    let Promotion::Requested(request) = to_prod else {
        panic!("expected a request, got {to_prod:?}");
    };
    assert_eq!(request.kind(), RequestKind::Promote);
    assert_eq!(request.digest(), source_version.version.digest());
    assert_eq!(request.state(), RequestState::Pending);
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "the digest is the one from staging",
            &ctx(),
        )
        .await
        .expect("approved");
    let published = w
        .publishing
        .publish_request(&author, request.id(), "publish-req-0123456789", &ctx())
        .await
        .expect("published in production");
    assert_eq!(published.version.digest(), source_version.version.digest());

    // The history in production says where it came from.
    let prod_workflow = request.workflow_id();
    let history = w
        .publishing
        .activations(&w.admin, prod_workflow, &Default::default(), &ctx())
        .await
        .expect("history");
    let entry = &history.items[0];
    assert_eq!(entry.kind, ActivationKind::Promote);
    assert_eq!(entry.request, Some(request.id()));
    let from = entry.promoted_from.as_ref().expect("provenance");
    assert_eq!(
        (from.workflow_id, from.version, from.namespace_id),
        (staging_workflow, 1, staging.id())
    );
    let _ = dev;
}

#[tokio::test]
async fn a_version_is_not_promoted_backwards_nor_into_its_own_namespace() {
    let w = world().await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let dev = namespace(&w, "shop-dev", Environment::Dev).await;
    let author = developer(&w, "author").await;
    let source = published_in(
        &w,
        "shop-staging",
        "checkout",
        &doc("checkout", "release 7"),
    )
    .await;

    for (target, why) in [
        (staging.id(), "the same namespace"),
        (dev.id(), "an earlier environment"),
    ] {
        let error = w
            .publishing
            .promote(
                &author,
                source,
                1,
                target,
                "a reason to try it",
                false,
                "promote-bad-0123456789",
                &ctx(),
            )
            .await
            .expect_err(why);
        assert!(matches!(error, AppError::NotAllowed(_)), "{why}: {error:?}");
    }
}

#[tokio::test]
async fn a_draft_with_changes_nobody_published_is_not_overwritten_by_a_promotion() {
    let w = world().await;
    let dev = namespace(&w, "shop-dev", Environment::Dev).await;
    namespace(&w, "shop-staging", Environment::Staging).await;
    let author = developer(&w, "author").await;
    let source = published_in(&w, "shop-dev", "checkout", &doc("checkout", "release 7")).await;
    let staged = published_in(
        &w,
        "shop-staging",
        "checkout",
        &doc("checkout", "release 6"),
    )
    .await;
    // Somebody is working in staging.
    let record = w
        .defs
        .get_workflow(&w.admin, staged, &ctx())
        .await
        .expect("workflow");
    edit(
        &w,
        &w.admin,
        staged,
        record.workflow.draft_revision(),
        &doc("checkout", "work in progress"),
    )
    .await;

    let staging_id = record.workflow.namespace_id();
    let error = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging_id,
            "release 7 is ready",
            false,
            "promote-wip-0123456789",
            &ctx(),
        )
        .await
        .expect_err("refused");
    assert_eq!(refusal(error), Refusal::DraftHasChanges);
    let after = w
        .defs
        .get_workflow(&w.admin, staged, &ctx())
        .await
        .expect("workflow");
    assert!(
        String::from_utf8_lossy(&after.draft).contains("work in progress"),
        "the work is still there"
    );
    let _ = dev;
}

#[tokio::test]
async fn promoting_what_the_target_already_runs_changes_nothing() {
    let w = world().await;
    namespace(&w, "shop-dev", Environment::Dev).await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let author = developer(&w, "author").await;
    let source = published_in(&w, "shop-dev", "checkout", &doc("checkout", "release 7")).await;
    w.publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "first promotion",
            false,
            "promote-one-0123456789",
            &ctx(),
        )
        .await
        .expect("first");
    let again = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "second promotion",
            false,
            "promote-two-0123456789",
            &ctx(),
        )
        .await
        .expect("second");
    assert!(
        matches!(again, Promotion::AlreadyActive { version: 1 }),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_promotion_stops_at_a_secret_the_target_does_not_have() {
    let w = world().await;
    namespace(&w, "shop-dev", Environment::Dev).await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let author = developer(&w, "author").await;
    w.put_secret("stripe-key", "sk-test-0123456789").await;
    let needs_secret = document(
        "checkout",
        "a",
        vec![with(activity("a"), "secrets", json!(["stripe-key"]))],
    );
    // `payments` is where the secret lives; the source workflow is published there by an administrator.
    let source = published_in(&w, "payments", "checkout", &needs_secret).await;
    let error = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "to staging",
            false,
            "promote-sec-0123456789",
            &ctx(),
        )
        .await
        .expect_err("the target has no such secret");
    assert!(
        matches!(error, AppError::SecretUnavailable { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_promotion_that_breaks_what_callers_rely_on_needs_to_be_accepted() {
    let w = world().await;
    namespace(&w, "shop-dev", Environment::Dev).await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let author = developer(&w, "author").await;
    let with_note = |marker: &str, properties: serde_json::Value| {
        let mut d = doc("checkout", marker);
        d["inputSchema"] = json!({"type": "object", "properties": properties});
        d
    };
    // Staging runs a version that takes `orderId` and `note`; dev has one that dropped `note`.
    published_in(
        &w,
        "shop-staging",
        "checkout",
        &with_note(
            "staging",
            json!({"orderId": {"type": "string"}, "note": {"type": "string"}}),
        ),
    )
    .await;
    let source = published_in(
        &w,
        "shop-dev",
        "checkout",
        &with_note("dev", json!({"orderId": {"type": "string"}})),
    )
    .await;

    let refused = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "dev dropped the note",
            false,
            "promote-brk-0123456789",
            &ctx(),
        )
        .await
        .expect_err("breaking");
    match refusal(refused) {
        Refusal::BreakingChange { findings, codes } => {
            assert!(findings >= 1, "{codes:?}");
            assert!(!codes.is_empty());
        }
        other => panic!("expected a breaking change, got {other:?}"),
    }
    let accepted = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "dev dropped the note, accepted",
            true,
            "promote-brk-0123456789",
            &ctx(),
        )
        .await
        .expect("accepted");
    assert!(matches!(accepted, Promotion::Published(_)));
}

#[tokio::test]
async fn promoting_needs_the_permission_to_publish() {
    let w = world().await;
    namespace(&w, "shop-dev", Environment::Dev).await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let ops = operator(&w, "olivia").await;
    let source = published_in(&w, "shop-dev", "checkout", &doc("checkout", "release 7")).await;
    let error = w
        .publishing
        .promote(
            &ops,
            source,
            1,
            staging.id(),
            "an operator promotes",
            false,
            "promote-ops-0123456789",
            &ctx(),
        )
        .await
        .expect_err("forbidden");
    assert!(matches!(error, AppError::Forbidden { .. }), "{error:?}");
}

#[tokio::test]
async fn a_promotion_with_the_longest_key_creates_the_workflow_in_the_target() {
    let w = world().await;
    let dev = namespace(&w, "shop-dev", Environment::Dev).await;
    let staging = namespace(&w, "shop-staging", Environment::Staging).await;
    let author = developer(&w, "author").await;
    let source = published_in(&w, "shop-dev", "checkout", &doc("checkout", "release 7")).await;
    let _ = dev;
    // The key the API takes at most; the step that creates the workflow derives its own from it.
    let key = format!("promote-{}", "k".repeat(160 - "promote-".len()));
    assert_eq!(key.len(), 160);
    let promoted = w
        .publishing
        .promote(
            &author,
            source,
            1,
            staging.id(),
            "tried in dev for a week",
            false,
            &key,
            &ctx(),
        )
        .await
        .expect("promoted");
    assert!(matches!(promoted, Promotion::Published(_)));
}
