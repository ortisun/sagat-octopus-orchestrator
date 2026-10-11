//! A namespace that asks for approvals: who may publish into it, what each person decided, and
//! when a request can no longer be published (FR-011-003, FR-011-005).

use sagat_application::error::{AppError, ConflictKind, Refusal};
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
    // A second approver is what is missing. (Not the administrator: they saved the draft.)
    let second = developer(&w, "second").await;
    w.publishing
        .decide(
            &second,
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

async fn audit_count(w: &crate::support::engine::World, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = $1")
        .bind(action)
        .fetch_one(&w.db.pool)
        .await
        .expect("audit")
}

#[tokio::test]
async fn whoever_saved_the_draft_cannot_approve_a_request_somebody_else_opened() {
    let w = world().await;
    namespace(&w, "billing-prod", Environment::Prod).await;
    let editor = developer(&w, "erin").await;
    let pipeline = developer(&w, "pipeline").await;
    let reviewer = developer(&w, "reviewer").await;
    let created = w
        .defs
        .create_workflow(
            &editor,
            "billing-prod",
            "payments-flow",
            &crate::common::bytes(&doc("payments-flow", "v1")),
            "create-erin-0123456789",
            &ctx(),
        )
        .await
        .expect("a draft by erin");
    let id = created.workflow.id();

    // A pipeline asks for the document Erin saved; she may not approve it, and neither may it.
    let request = w
        .publishing
        .open_request(
            &pipeline,
            id,
            created.workflow.draft_revision(),
            None,
            &ctx(),
        )
        .await
        .expect("request");
    assert_eq!(request.edited_by().map(|s| s.as_str()), Some("erin"));
    for who in [&editor, &pipeline] {
        let refused = w
            .publishing
            .decide(
                who,
                request.id(),
                Verdict::Approve,
                "it is fine, i know it",
                &ctx(),
            )
            .await
            .expect_err("not theirs to approve");
        assert_eq!(
            refusal(refused),
            Refusal::Request(VersioningError::AuthorCannotDecide)
        );
    }
    // And rejecting is deciding too.
    assert!(
        w.publishing
            .decide(
                &editor,
                request.id(),
                Verdict::Reject,
                "i would rather not",
                &ctx()
            )
            .await
            .is_err()
    );
    // The attempts are on the trail, once for each person however often they try (the denials are
    // written off the path of the answer, so give them a moment).
    let mut written = 0;
    for _ in 0..100 {
        written = audit_count(&w, "publication.denied").await;
        if written >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(written, 2);
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "reviewed the document",
            &ctx(),
        )
        .await
        .expect("somebody else can");

    // The person who saves the draft next is the one who wrote the next document: Erin may now
    // approve what Frank wrote, and the request that was about her document is over, on the trail.
    let frank = developer(&w, "frank").await;
    let revision = crate::common::edit(
        &w,
        &frank,
        id,
        created.workflow.draft_revision(),
        &doc("payments-flow", "v2"),
    )
    .await;
    assert_eq!(audit_count(&w, "publication.superseded").await, 1);
    let next = w
        .publishing
        .open_request(&pipeline, id, revision, None, &ctx())
        .await
        .expect("a new request");
    assert_eq!(next.edited_by().map(|s| s.as_str()), Some("frank"));
    assert!(
        w.publishing
            .decide(
                &editor,
                next.id(),
                Verdict::Approve,
                "frank wrote it, i read it",
                &ctx()
            )
            .await
            .is_ok()
    );

    // The publication names the request on the trail.
    let published = w
        .publishing
        .publish_request(&pipeline, next.id(), "key-erin-0123456789", &ctx())
        .await
        .expect("published");
    assert_eq!(published.version.number(), 1);
    let named: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'workflow.published' AND reason LIKE '%by request ' || $1",
    )
    .bind(next.id().to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(named, 1);
}

#[tokio::test]
async fn a_policy_change_says_from_what_to_what() {
    let w = world().await;
    let prod = namespace(&w, "billing-prod", Environment::Prod).await;
    w.publishing
        .set_policy(&w.admin, prod.id(), 0, "we trust the pipeline here", &ctx())
        .await
        .expect("lowered");
    let reason: String = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'publication.policy_changed' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert!(reason.starts_with("from 1 to 0 approval(s)"), "{reason}");
}

#[tokio::test]
async fn a_request_publishes_the_revision_and_the_document_it_names_and_no_other() {
    let w = world().await;
    namespace(&w, "billing-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let reviewer = developer(&w, "reviewer").await;
    let created = w
        .defs
        .create_workflow(
            &author,
            "billing-prod",
            "payments-flow",
            &crate::common::bytes(&doc("payments-flow", "v1")),
            "create-author-0123456789",
            &ctx(),
        )
        .await
        .expect("a draft");
    let id = created.workflow.id();
    let request = w
        .publishing
        .open_request(
            &developer(&w, "pipeline").await,
            id,
            created.workflow.draft_revision(),
            None,
            &ctx(),
        )
        .await
        .expect("request");
    w.publishing
        .decide(
            &reviewer,
            request.id(),
            Verdict::Approve,
            "reviewed the document",
            &ctx(),
        )
        .await
        .expect("approved");

    // What a request is about cannot be changed under it: the database refuses.
    let changed =
        sqlx::query("UPDATE publication_requests SET digest = repeat('a', 64) WHERE id = $1")
            .bind(request.id().as_uuid())
            .execute(&w.db.pool)
            .await
            .expect_err("frozen");
    assert!(changed.to_string().contains("cannot change"), "{changed}");

    // Even if it were (the trigger is switched off for a moment, as a bug or a restore could),
    // the transaction that publishes compares what it publishes with what was approved.
    sqlx::query("ALTER TABLE publication_requests DISABLE TRIGGER publication_requests_freeze")
        .execute(&w.db.pool)
        .await
        .expect("off");
    sqlx::query("UPDATE publication_requests SET digest = repeat('a', 64) WHERE id = $1")
        .bind(request.id().as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("forged");
    sqlx::query(
        "ALTER TABLE publication_requests ENABLE ALWAYS TRIGGER publication_requests_freeze",
    )
    .execute(&w.db.pool)
    .await
    .expect("on");
    let refused = w
        .publishing
        .publish_request(&author, request.id(), "key-forged-0123456789", &ctx())
        .await
        .expect_err("not the document that was approved");
    assert!(
        matches!(
            refused,
            AppError::Conflict(ConflictKind::RequestNotApproved)
        ),
        "{refused:?}"
    );
    let versions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workflow_versions WHERE workflow_id = $1")
            .bind(id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(versions, 0, "nothing was published");
}

#[tokio::test]
async fn release_notes_are_plain_text_because_they_are_read_on_terminals_and_in_the_history() {
    let w = world().await;
    let record = draft(&w, "payments", "notes-flow", &doc("notes-flow", "v1")).await;
    let id = record.workflow.id();
    let revision = record.workflow.draft_revision();
    for (notes, why) in [
        ("fix\u{1b}[2J", "an escape sequence"),
        ("a\u{202e}b", "a bidirectional override"),
        ("a\r\nb", "a carriage return"),
        ("a\u{2028}b", "a line separator"),
    ] {
        let refused = w
            .defs
            .publish(
                &w.admin,
                id,
                revision,
                Some(notes),
                "key-notes-0123456789",
                &ctx(),
            )
            .await
            .expect_err(why);
        assert!(
            matches!(refused, AppError::NotAllowed(_)),
            "{why}: {refused:?}"
        );
    }
    // New lines and tabs are layout, and stay.
    w.defs
        .publish(
            &w.admin,
            id,
            revision,
            Some("line one\n\tline two"),
            "key-notes-1-0123456789",
            &ctx(),
        )
        .await
        .expect("notes with lines");
}

#[tokio::test]
async fn a_request_is_not_born_for_a_revision_that_was_saved_over() {
    use sagat_application::publishing::{Opened, PublishingStore};
    use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
    use sagat_domain::versioning::{PublicationRequest, RequestId, RequestKind};

    let w = world().await;
    let prod = namespace(&w, "billing-prod", Environment::Prod).await;
    let author = developer(&w, "author").await;
    let record = draft(
        &w,
        "billing-prod",
        "payments-flow",
        &doc("payments-flow", "v1"),
    )
    .await;
    let id = record.workflow.id();
    let one = record.workflow.draft_revision();
    // The service read revision 1 and prepared a request; before it is stored, somebody saves 2.
    let two = edit(&w, &w.admin, id, one, &doc("payments-flow", "v2")).await;
    assert_eq!(two, one + 1);

    let now = time::OffsetDateTime::now_utc();
    let request = PublicationRequest::open(
        RequestId::generate(),
        author.tenant_id(),
        prod.id(),
        id,
        RequestKind::Publish,
        one,
        sagat_domain::workflow::Digest::new([7; 32]),
        None,
        None,
        author.subject().clone(),
        ApprovalPolicy::new(1).expect("a policy"),
        now,
    );
    let audit = AuditRecord::by(
        &author,
        AuditAction::PublicationRequested,
        ResourceType::PublicationRequest,
        request.id().to_string(),
        None,
        now,
    );
    let opened = w
        .store
        .open_request(&request, &audit)
        .await
        .expect("answered");
    assert_eq!(opened, Opened::Stale { current: two });
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM publication_requests WHERE workflow_id = $1")
            .bind(id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(stored, 0, "nothing was stored, so nothing holds the slot");
}
