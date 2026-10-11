//! A namespace that asks for approvals: who may publish into it, what each person decided, and
//! when a request can no longer be published (FR-011-003, FR-011-005).

use sagat_application::error::Refusal;
use sagat_domain::identity::Environment;
use sagat_domain::versioning::{ApprovalPolicy, RequestState, Verdict, VersioningError};

use crate::common::{developer, doc, draft, edit, namespace, refusal};
use crate::support::engine::{ctx, world};

#[tokio::test]
async fn production_asks_for_an_approval_until_an_administrator_says_otherwise() {
    let w = world().await;
    let prod = namespace(&w, "billing-prod", Environment::Prod).await;
    let dev = namespace(&w, "billing-dev", Environment::Dev).await;

    let asked = w
        .publishing
        .policy(&w.admin, prod.id(), &ctx())
        .await
        .expect("policy");
    assert_eq!(asked.policy.required(), 1);
    assert!(matches!(
        asked.source,
        sagat_application::publishing::PolicySource::Default(Environment::Prod)
    ));
    let free = w
        .publishing
        .policy(&w.admin, dev.id(), &ctx())
        .await
        .expect("policy");
    assert_eq!(free.policy, ApprovalPolicy::NONE);

    let set = w
        .publishing
        .set_policy(
            &w.admin,
            prod.id(),
            2,
            "two people for the payment flows",
            &ctx(),
        )
        .await
        .expect("set");
    assert_eq!(set.policy.required(), 2);
    assert!(matches!(
        set.source,
        sagat_application::publishing::PolicySource::Set { .. }
    ));
    // Only an administrator sets it, and not above the most a policy asks for.
    let dev1 = developer(&w, "dev1").await;
    assert!(
        w.publishing
            .set_policy(&dev1, prod.id(), 0, "i would rather not wait", &ctx())
            .await
            .is_err()
    );
    assert!(
        w.publishing
            .set_policy(&w.admin, prod.id(), 4, "everybody", &ctx())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_direct_publish_into_a_namespace_that_asks_is_refused_and_the_request_publishes_once_approved()
 {
    let w = world().await;
    namespace(&w, "billing-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let reviewer = developer(&w, "reviewer").await;
    let record = draft(
        &w,
        "billing-prod",
        "payments-flow",
        &doc("payments-flow", "v1"),
    )
    .await;
    let id = record.workflow.id();

    let refused = w
        .defs
        .publish(
            &author,
            id,
            record.workflow.draft_revision(),
            None,
            "key-k1-0123456789",
            &ctx(),
        )
        .await
        .expect_err("direct publish is refused");
    assert_eq!(refusal(refused), Refusal::ApprovalRequired { required: 1 });

    let request = w
        .publishing
        .open_request(
            &author,
            id,
            record.workflow.draft_revision(),
            Some("first release"),
            &ctx(),
        )
        .await
        .expect("request");
    assert_eq!(request.state(), RequestState::Pending);

    // Not before it is approved.
    let early = w
        .publishing
        .publish_request(&author, request.id(), "key-k2-0123456789", &ctx())
        .await
        .expect_err("not approved");
    assert_eq!(
        refusal(early),
        Refusal::Request(VersioningError::NotInState(RequestState::Pending))
    );
    // Not by its author.
    let own = w
        .publishing
        .decide(
            &author,
            request.id(),
            Verdict::Approve,
            "it is fine, i wrote it",
            &ctx(),
        )
        .await
        .expect_err("the author");
    assert_eq!(
        refusal(own),
        Refusal::Request(VersioningError::AuthorCannotDecide)
    );
    let approved = w
        .publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "reads well, tests pass",
            &ctx(),
        )
        .await
        .expect("approve");
    assert_eq!(approved.state(), RequestState::Approved);

    let published = w
        .publishing
        .publish_request(&author, request.id(), "key-k3-0123456789", &ctx())
        .await
        .expect("published");
    assert_eq!(published.version.number(), 1);
    let after = w
        .publishing
        .get_request(&author, request.id(), &ctx())
        .await
        .expect("the request");
    assert_eq!(after.state(), RequestState::Published);
    let (version, by, _) = after.published().expect("who published it");
    assert_eq!((*version, by.as_str()), (published.version.id(), "author"));
    assert_eq!(after.decisions().len(), 1);

    // The trail says who asked, who approved and who published.
    let actions: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, actor FROM audit_records \
         WHERE action IN ('publication.requested', 'publication.approved', 'workflow.published') \
         ORDER BY created_at, action",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert!(
        actions.contains(&("publication.requested".into(), "author".into())),
        "{actions:?}"
    );
    assert!(
        actions.contains(&("publication.approved".into(), "reviewer".into())),
        "{actions:?}"
    );
    assert!(
        actions.contains(&("workflow.published".into(), "author".into())),
        "{actions:?}"
    );

    // And the history of the pointer says it was this request.
    let history = w
        .publishing
        .activations(&author, id, &Default::default(), &ctx())
        .await
        .expect("history");
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].request, Some(request.id()));
}

#[tokio::test]
async fn editing_the_draft_supersedes_the_request_and_nothing_is_published_from_it() {
    let w = world().await;
    namespace(&w, "billing-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let reviewer = developer(&w, "reviewer").await;
    let record = draft(
        &w,
        "billing-prod",
        "payments-flow",
        &doc("payments-flow", "v1"),
    )
    .await;
    let id = record.workflow.id();
    let revision = record.workflow.draft_revision();
    let request = w
        .publishing
        .open_request(&author, id, revision, None, &ctx())
        .await
        .expect("request");
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "reads well",
            &ctx(),
        )
        .await
        .expect("approve");

    // The author changes the draft after the approval: what was approved is not what is there.
    edit(
        &w,
        &author,
        id,
        revision,
        &doc("payments-flow", "v1 changed"),
    )
    .await;
    let stale = w
        .publishing
        .get_request(&author, request.id(), &ctx())
        .await
        .expect("the request");
    assert_eq!(stale.state(), RequestState::Superseded);
    let refused = w
        .publishing
        .publish_request(&author, request.id(), "key-k-0123456789", &ctx())
        .await
        .expect_err("superseded");
    assert_eq!(
        refusal(refused),
        Refusal::Request(VersioningError::NotInState(RequestState::Superseded))
    );
    let versions = w
        .defs
        .list_versions(&author, id, &Default::default(), &ctx())
        .await
        .expect("versions");
    assert!(versions.items.is_empty(), "nothing was published");
}

#[tokio::test]
async fn there_is_one_open_request_for_a_workflow_at_a_time_and_a_rejection_ends_it() {
    let w = world().await;
    namespace(&w, "billing-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let reviewer = developer(&w, "reviewer").await;
    let record = draft(
        &w,
        "billing-prod",
        "payments-flow",
        &doc("payments-flow", "v1"),
    )
    .await;
    let id = record.workflow.id();
    let revision = record.workflow.draft_revision();
    let first = w
        .publishing
        .open_request(&author, id, revision, None, &ctx())
        .await
        .expect("request");
    let again = w
        .publishing
        .open_request(&author, id, revision, None, &ctx())
        .await
        .expect_err("one at a time");
    assert_eq!(
        refusal(again),
        Refusal::RequestOpen {
            request: first.id()
        }
    );

    w.publishing
        .decide(
            &reviewer,
            first.id(),
            Verdict::Reject,
            "breaks the contract with billing",
            &ctx(),
        )
        .await
        .expect("reject");
    // The way is free for another request on the same draft.
    let second = w
        .publishing
        .open_request(&author, id, revision, None, &ctx())
        .await
        .expect("a new request");
    assert_ne!(second.id(), first.id());
    // The author takes it back; others cannot.
    assert_eq!(
        refusal(
            w.publishing
                .withdraw(&reviewer, second.id(), &ctx())
                .await
                .expect_err("not theirs")
        ),
        Refusal::Request(VersioningError::NotTheAuthor)
    );
    let withdrawn = w
        .publishing
        .withdraw(&author, second.id(), &ctx())
        .await
        .expect("withdrawn");
    assert_eq!(withdrawn.state(), RequestState::Withdrawn);
}

#[tokio::test]
async fn a_policy_raised_after_the_approvals_asks_for_one_more_before_it_publishes() {
    let w = world().await;
    let prod = namespace(&w, "billing-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let reviewer = developer(&w, "reviewer").await;
    let record = draft(
        &w,
        "billing-prod",
        "payments-flow",
        &doc("payments-flow", "v1"),
    )
    .await;
    let id = record.workflow.id();
    let request = w
        .publishing
        .open_request(&author, id, record.workflow.draft_revision(), None, &ctx())
        .await
        .expect("request");
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "reads well",
            &ctx(),
        )
        .await
        .expect("approve");
    w.publishing
        .set_policy(
            &w.admin,
            prod.id(),
            2,
            "the payment flows need two people",
            &ctx(),
        )
        .await
        .expect("raised");
    let refused = w
        .publishing
        .publish_request(&author, request.id(), "key-k-0123456789", &ctx())
        .await
        .expect_err("one is not enough now");
    assert_eq!(
        refusal(refused),
        Refusal::Request(VersioningError::NotEnoughApprovals { have: 1, need: 2 })
    );
    // A second approver is what is missing.
    w.publishing
        .decide(
            &w.admin,
            request.id(),
            Verdict::Approve,
            "second pair of eyes",
            &ctx(),
        )
        .await
        .expect("second approval");
    let published = w
        .publishing
        .publish_request(&author, request.id(), "key-k2-0123456789", &ctx())
        .await
        .expect("published");
    assert_eq!(published.version.number(), 1);
}
