//! The tenant boundary in everything this feature added: what another tenant's administrator
//! cannot read, ask for or change, and what the store itself refuses (NFR-011-SEC).

use sagat_application::error::AppError;
use sagat_application::publishing::PublishingStore;
use sagat_domain::identity::Environment;
use sagat_domain::versioning::{ApprovalPolicy, Verdict};

use crate::common::{developer, doc, draft, namespace};
use crate::support::engine::{ctx, world};

#[tokio::test]
async fn nothing_of_publishing_is_readable_or_changeable_from_another_tenant() {
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
        .publish_request(&author, request.id(), "key-publish-0123456789", &ctx())
        .await
        .expect("published");

    let theirs = &w.other_admin;
    let not_found = |result: Result<(), AppError>, what: &str| {
        assert!(
            matches!(result, Err(AppError::NotFound)),
            "{what}: {result:?}"
        );
    };
    not_found(
        w.publishing
            .get_request(theirs, request.id(), &ctx())
            .await
            .map(|_| ()),
        "reading a request",
    );
    not_found(
        w.publishing
            .requests(theirs, id, None, &Default::default(), &ctx())
            .await
            .map(|_| ()),
        "listing the requests",
    );
    not_found(
        w.publishing
            .activations(theirs, id, &Default::default(), &ctx())
            .await
            .map(|_| ()),
        "reading the history",
    );
    not_found(
        w.publishing
            .activate(
                theirs,
                id,
                1,
                "an administrator of another tenant",
                None,
                true,
                &ctx(),
            )
            .await
            .map(|_| ()),
        "moving the pointer",
    );
    not_found(
        w.publishing
            .open_request(theirs, id, 1, None, &ctx())
            .await
            .map(|_| ()),
        "opening a request",
    );
    not_found(
        w.publishing
            .decide(
                theirs,
                request.id(),
                Verdict::Reject,
                "a rejection from elsewhere",
                &ctx(),
            )
            .await
            .map(|_| ()),
        "deciding a request",
    );
    not_found(
        w.publishing
            .withdraw(theirs, request.id(), &ctx())
            .await
            .map(|_| ()),
        "withdrawing a request",
    );
    // The namespace of the policy is another tenant's too.
    let ours = w.namespace.id();
    not_found(
        w.publishing.policy(theirs, ours, &ctx()).await.map(|_| ()),
        "reading the policy",
    );
    not_found(
        w.publishing
            .set_policy(
                theirs,
                ours,
                0,
                "an administrator of another tenant",
                &ctx(),
            )
            .await
            .map(|_| ()),
        "setting the policy",
    );
    // Promoting into a namespace of another tenant, or from a workflow of another tenant.
    not_found(
        w.publishing
            .promote(
                &w.admin,
                id,
                1,
                w.other_namespace.id(),
                "into somebody else's namespace",
                true,
                "promote-x-0123456789",
                &ctx(),
            )
            .await
            .map(|_| ()),
        "promoting into another tenant's namespace",
    );
    not_found(
        w.publishing
            .promote(
                theirs,
                id,
                1,
                w.other_namespace.id(),
                "out of somebody else's workflow",
                true,
                "promote-y-0123456789",
                &ctx(),
            )
            .await
            .map(|_| ()),
        "promoting another tenant's workflow",
    );

    // The store answers the same way when asked directly.
    assert!(
        w.store
            .request(theirs.tenant_id(), request.id())
            .await
            .expect("store")
            .is_none()
    );
    assert!(
        w.store
            .stored_policy(theirs.tenant_id(), ours)
            .await
            .expect("store")
            .is_none()
    );
    assert!(
        w.store
            .activations(theirs.tenant_id(), id, None, 10)
            .await
            .expect("store")
            .is_empty()
    );
    // And nothing of theirs moved ours.
    let history = w
        .publishing
        .activations(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("history");
    assert_eq!(history.items.len(), 1);
}

#[tokio::test]
async fn the_database_refuses_a_policy_a_request_or_a_dependency_that_points_into_another_tenant() {
    let w = world().await;
    let ours = w.namespace.id().as_uuid();
    let theirs = w.other_admin.tenant_id().as_uuid();
    // A policy for our namespace written under their tenant.
    let refused = sqlx::query(
        "INSERT INTO publication_policies (tenant_id, namespace_id, required_approvals, set_by, set_at) \
         VALUES ($1, $2, 1, 'mallory', now())",
    )
    .bind(theirs)
    .bind(ours)
    .execute(&w.db.pool)
    .await
    .expect_err("composite foreign key");
    let (code, constraint) = crate::support::db_error(refused);
    assert_eq!(code, "23503");
    assert_eq!(
        constraint.as_deref(),
        Some("publication_policies_namespace_fk")
    );

    // A policy of the right tenant outside the range.
    let refused = sqlx::query(
        "INSERT INTO publication_policies (tenant_id, namespace_id, required_approvals, set_by, set_at) \
         VALUES ($1, $2, 9, 'alice', now())",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(ours)
    .execute(&w.db.pool)
    .await
    .expect_err("range");
    let (code, constraint) = crate::support::db_error(refused);
    assert_eq!(code, "23514");
    assert_eq!(constraint.as_deref(), Some("publication_policies_range"));
    let _ = ApprovalPolicy::NONE;
}

#[tokio::test]
async fn the_history_of_the_pointer_cannot_be_rewritten() {
    let w = world().await;
    let (id, _) = w.publish(doc("flow", "one")).await;
    for statement in [
        "UPDATE workflow_activations SET reason = 'rewritten'",
        "DELETE FROM workflow_activations",
        "TRUNCATE workflow_activations",
    ] {
        let refused = sqlx::query(statement)
            .execute(&w.db.pool)
            .await
            .expect_err("append-only");
        let (code, _) = crate::support::db_error(refused);
        assert_eq!(code, "23001", "{statement}");
    }
    let history = w
        .publishing
        .activations(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("history");
    assert_eq!(history.items.len(), 1);
}

#[tokio::test]
async fn only_a_request_that_exists_elsewhere_leaves_a_trace_of_the_probe() {
    let w = world().await;
    let author = developer(&w, "author").await;
    let record = draft(&w, "payments", "flow", &doc("flow", "v1")).await;
    let request = w
        .publishing
        .open_request(
            &author,
            record.workflow.id(),
            record.workflow.draft_revision(),
            None,
            &ctx(),
        )
        .await
        .expect("request");
    let theirs = &w.other_admin;

    // Random identifiers by the thousand would otherwise be rows of the audit trail.
    for _ in 0..20 {
        let _ = w
            .publishing
            .get_request(
                theirs,
                sagat_domain::versioning::RequestId::generate(),
                &ctx(),
            )
            .await;
    }
    let _ = w.publishing.get_request(theirs, request.id(), &ctx()).await;
    let recorded = crate::support::eventually("the probe of a real request", || async {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant' \
             AND resource_type = 'publication_request'",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        (n > 0).then_some(n)
    })
    .await;
    assert_eq!(recorded, 1);
    // The writes are off the response path: give any other one the time to land.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let later: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant' \
         AND resource_type = 'publication_request'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        later, 1,
        "one for the request that exists, none for the 20 that do not"
    );
}

#[tokio::test]
async fn the_history_stays_append_only_where_triggers_are_switched_off_for_replication() {
    let w = world().await;
    w.publish(doc("flow", "one")).await;
    let mut conn = w.db.pool.acquire().await.expect("connection");
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("a superuser may");
    for statement in [
        "UPDATE workflow_activations SET reason = 'rewritten'",
        "DELETE FROM workflow_activations",
        "TRUNCATE workflow_activations",
        "UPDATE publication_requests SET digest = repeat('b', 64)",
    ] {
        let result = sqlx::query(statement).execute(&mut *conn).await;
        // The last one has no rows to refuse; the others are refused although triggers do not
        // fire in this session unless they were made to.
        if statement.starts_with("UPDATE publication_requests") {
            assert!(result.is_ok());
        } else {
            let (code, _) = crate::support::db_error(result.expect_err("append-only"));
            assert_eq!(code, "23001", "{statement}");
        }
    }
}

#[tokio::test]
async fn a_history_row_and_a_request_belong_to_the_workflow_they_name() {
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let (a, _version_a) = w.publish(doc("flow-a", "one")).await;
    let (_b, version_b) = w.publish(doc("flow-b", "one")).await;
    namespace(&w, "billing-prod", Environment::Prod).await;
    let prod_flow = draft(&w, "billing-prod", "prod-flow", &doc("prod-flow", "one")).await;
    let author = developer(&w, "author").await;
    let request = w
        .publishing
        .open_request(
            &author,
            prod_flow.workflow.id(),
            prod_flow.workflow.draft_revision(),
            None,
            &ctx(),
        )
        .await
        .expect("request");

    // A change of flow-a's pointer to a version of flow-b.
    let refused = sqlx::query(
        "INSERT INTO workflow_activations (id, tenant_id, workflow_id, version_id, kind, actor, created_at) \
         VALUES (gen_random_uuid(), $1, $2, $3, 'ACTIVATE', 'mallory', now())",
    )
    .bind(tenant)
    .bind(a.as_uuid())
    .bind(version_b.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("the version is of another workflow");
    let (code, constraint) = crate::support::db_error(refused);
    assert_eq!(
        (code.as_str(), constraint.as_deref()),
        ("23503", Some("workflow_activations_version_fk"))
    );

    // A change authorised by a request that was about another workflow.
    let refused = sqlx::query(
        "INSERT INTO workflow_activations (id, tenant_id, workflow_id, version_id, kind, actor, request_id, created_at) \
         SELECT gen_random_uuid(), $1, $2, v.id, 'PUBLISH', 'mallory', $3, now() \
         FROM workflow_versions v WHERE v.workflow_id = $2",
    )
    .bind(tenant)
    .bind(a.as_uuid())
    .bind(request.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("the request is about another workflow");
    let (code, constraint) = crate::support::db_error(refused);
    assert_eq!(
        (code.as_str(), constraint.as_deref()),
        ("23503", Some("workflow_activations_request_fk"))
    );

    // A request that says its workflow is in a namespace it is not in.
    let refused = sqlx::query(
        "INSERT INTO publication_requests (id, tenant_id, namespace_id, workflow_id, kind, draft_revision, digest, \
                                           requested_by, requested_at, required_approvals, state, updated_at) \
         VALUES (gen_random_uuid(), $1, $2, $3, 'PUBLISH', 1, repeat('c', 64), 'mallory', now(), 0, 'REJECTED', now())",
    )
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(prod_flow.workflow.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect_err("the workflow is in another namespace");
    let (code, constraint) = crate::support::db_error(refused);
    assert_eq!(
        (code.as_str(), constraint.as_deref()),
        ("23503", Some("publication_requests_workflow_fk"))
    );
}
