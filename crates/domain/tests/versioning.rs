//! The rules of approvals and of the pointer (FR-011-003, FR-011-005): who may decide a request to
//! publish, when it is approved, and when it can no longer be published.
#![allow(clippy::expect_used, clippy::panic)] // helpers fail the test by panicking

use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{Environment, NamespaceId, Subject, TenantId};
use sagat_domain::versioning::{
    ActivationKind, ApprovalPolicy, MAX_REQUIRED_APPROVALS, PublicationRequest, RequestId,
    RequestKind, RequestState, Verdict, VersioningError,
};
use sagat_domain::workflow::Digest;
use time::OffsetDateTime;
use uuid::Uuid;

fn who(name: &str) -> Subject {
    Subject::parse(name).expect("a subject")
}

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("time")
}

fn request(policy: ApprovalPolicy) -> PublicationRequest {
    PublicationRequest::open(
        RequestId::generate(),
        TenantId::new(Uuid::new_v4()),
        NamespaceId::new(Uuid::new_v4()),
        WorkflowId::generate(),
        RequestKind::Publish,
        7,
        Digest::new([9; 32]),
        Some("v3: the new payment route".to_owned()),
        None,
        who("alice"),
        policy,
        now(),
    )
}

fn asks(n: u8) -> ApprovalPolicy {
    ApprovalPolicy::new(n).expect("a policy")
}

#[test]
fn production_asks_for_one_approval_by_default_and_the_others_for_none() {
    assert_eq!(ApprovalPolicy::default_for(Environment::Prod).required(), 1);
    assert_eq!(
        ApprovalPolicy::default_for(Environment::Staging).required(),
        0
    );
    assert_eq!(ApprovalPolicy::default_for(Environment::Dev).required(), 0);
    assert!(ApprovalPolicy::new(MAX_REQUIRED_APPROVALS).is_ok());
    assert_eq!(
        ApprovalPolicy::new(MAX_REQUIRED_APPROVALS + 1),
        Err(VersioningError::PolicyOutOfRange)
    );
}

#[test]
fn a_namespace_that_asks_for_nothing_opens_requests_already_approved() {
    let r = request(ApprovalPolicy::NONE);
    assert_eq!(r.state(), RequestState::Approved);
    let r = request(asks(1));
    assert_eq!(r.state(), RequestState::Pending);
}

#[test]
fn the_request_is_approved_exactly_when_it_has_as_many_approvals_as_were_asked() {
    let mut r = request(asks(2));
    assert_eq!(
        r.approve(&who("bob"), "reads well", now()),
        Ok(RequestState::Pending)
    );
    assert_eq!(r.approvals(), 1);
    assert_eq!(
        r.approve(&who("carol"), "agreed", now()),
        Ok(RequestState::Approved)
    );
    assert_eq!(r.approvals(), 2);
    assert_eq!(r.decisions().len(), 2);
    assert!(r.decisions().iter().all(|d| d.verdict == Verdict::Approve));
}

#[test]
fn the_author_never_decides_their_own_request_and_nobody_decides_twice() {
    let mut r = request(asks(1));
    assert_eq!(
        r.approve(&who("alice"), "i wrote it", now()),
        Err(VersioningError::AuthorCannotDecide)
    );
    assert_eq!(
        r.reject(&who("alice"), "i changed my mind", now()),
        Err(VersioningError::AuthorCannotDecide)
    );
    assert_eq!(r.state(), RequestState::Pending);
    assert!(r.decisions().is_empty());

    let mut r = request(asks(2));
    r.approve(&who("bob"), "fine", now()).expect("first");
    assert_eq!(
        r.approve(&who("bob"), "fine again", now()),
        Err(VersioningError::AlreadyDecided)
    );
    assert_eq!(
        r.reject(&who("bob"), "no, wait", now()),
        Err(VersioningError::AlreadyDecided)
    );
    assert_eq!(r.approvals(), 1, "one person is one approval");
}

#[test]
fn one_rejection_ends_the_request_whatever_the_others_said() {
    let mut r = request(asks(2));
    r.approve(&who("bob"), "fine", now()).expect("approve");
    r.reject(&who("carol"), "breaks the contract", now())
        .expect("reject");
    assert_eq!(r.state(), RequestState::Rejected);
    // Final: nothing else can be done with it.
    for result in [
        r.approve(&who("dave"), "ok", now()).map(|_| ()),
        r.reject(&who("dave"), "no", now()),
        r.withdraw(&who("alice")),
        r.supersede(),
        r.publish(&who("alice"), WorkflowVersionId::generate(), asks(2), now()),
    ] {
        assert_eq!(
            result,
            Err(VersioningError::NotInState(RequestState::Rejected))
        );
    }
}

#[test]
fn only_the_author_withdraws_and_only_while_the_request_is_open() {
    let mut r = request(asks(1));
    assert_eq!(r.withdraw(&who("bob")), Err(VersioningError::NotTheAuthor));
    r.withdraw(&who("alice")).expect("the author takes it back");
    assert_eq!(r.state(), RequestState::Withdrawn);
    assert_eq!(
        r.withdraw(&who("alice")),
        Err(VersioningError::NotInState(RequestState::Withdrawn))
    );
}

#[test]
fn a_request_is_published_only_when_approved_and_only_once() {
    let version = WorkflowVersionId::generate();
    let mut r = request(asks(1));
    assert_eq!(
        r.publish(&who("alice"), version, asks(1), now()),
        Err(VersioningError::NotInState(RequestState::Pending)),
        "nothing is published on a request nobody approved"
    );
    r.approve(&who("bob"), "fine", now()).expect("approve");
    r.publish(&who("alice"), version, asks(1), now())
        .expect("published");
    assert_eq!(r.state(), RequestState::Published);
    let (published, by, _) = r.published().expect("who published it");
    assert_eq!((*published, by.as_str()), (version, "alice"));
    assert_eq!(
        r.publish(&who("alice"), version, asks(1), now()),
        Err(VersioningError::NotInState(RequestState::Published))
    );
}

#[test]
fn a_policy_raised_after_the_approvals_asks_for_more_before_it_publishes() {
    let mut r = request(asks(1));
    r.approve(&who("bob"), "fine", now()).expect("approve");
    assert_eq!(
        r.publish(&who("alice"), WorkflowVersionId::generate(), asks(2), now()),
        Err(VersioningError::NotEnoughApprovals { have: 1, need: 2 })
    );
    assert_eq!(r.state(), RequestState::Approved, "and it stays as it was");
    // A policy lowered is satisfied by what it has.
    assert!(r.satisfies(asks(1)));
    assert!(r.satisfies(ApprovalPolicy::NONE));
    assert!(!r.satisfies(asks(2)));
}

#[test]
fn a_draft_that_moved_supersedes_an_open_request_and_nothing_else() {
    let mut open = request(asks(1));
    open.supersede().expect("superseded");
    assert_eq!(open.state(), RequestState::Superseded);
    assert_eq!(
        open.approve(&who("bob"), "fine", now()),
        Err(VersioningError::NotInState(RequestState::Superseded))
    );
    assert_eq!(
        open.supersede(),
        Err(VersioningError::NotInState(RequestState::Superseded))
    );
}

#[test]
fn a_switch_by_hand_is_a_rollback_going_back_and_an_activation_going_forward() {
    assert_eq!(ActivationKind::of_switch(4, 3), ActivationKind::Rollback);
    assert_eq!(ActivationKind::of_switch(3, 4), ActivationKind::Activate);
    assert_eq!(ActivationKind::of_switch(3, 3), ActivationKind::Activate);
}

#[test]
fn names_survive_the_round_trip_through_storage() {
    for state in RequestState::ALL {
        assert_eq!(RequestState::parse(state.as_str()), Some(state));
    }
    for kind in ActivationKind::ALL {
        assert_eq!(kind.as_str().parse::<ActivationKind>(), Ok(kind));
    }
    for kind in [RequestKind::Publish, RequestKind::Promote] {
        assert_eq!(RequestKind::parse(kind.as_str()), Some(kind));
    }
    assert!(RequestState::parse("DONE").is_none());
    assert!(RequestState::Pending.is_open() && RequestState::Approved.is_open());
    for closed in [
        RequestState::Rejected,
        RequestState::Withdrawn,
        RequestState::Superseded,
        RequestState::Published,
    ] {
        assert!(!closed.is_open());
    }
}

#[test]
fn whoever_saved_the_document_cannot_approve_it_even_when_somebody_else_asked() {
    // A pipeline (or a colleague) opens the request for the document that Bob saved.
    let mut open = request(asks(1)).with_edited_by(Some(who("bob")));
    assert_eq!(open.edited_by(), Some(&who("bob")));
    assert_eq!(
        open.approve(&who("bob"), "i wrote it and it is fine", now()),
        Err(VersioningError::AuthorCannotDecide)
    );
    assert_eq!(
        open.reject(&who("bob"), "i changed my mind about it", now()),
        Err(VersioningError::AuthorCannotDecide)
    );
    assert_eq!(open.approvals(), 0);
    // The requester still cannot, and somebody else can.
    assert_eq!(
        open.approve(&who("alice"), "i asked for it myself", now()),
        Err(VersioningError::AuthorCannotDecide)
    );
    assert_eq!(
        open.approve(&who("carol"), "reviewed the document", now()),
        Ok(RequestState::Approved)
    );
}
