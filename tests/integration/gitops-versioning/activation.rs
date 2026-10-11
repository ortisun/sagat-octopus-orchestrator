//! The pointer to the active version: publishing moves it, a rollback moves it back, and every
//! move is a row of a history that is never rewritten (FR-011-001, US02, US04).

use sagat_application::error::Refusal;
use sagat_domain::versioning::ActivationKind;
use serde_json::json;

use crate::common::{doc, operator, refusal};
use crate::support::engine::{ctx, world};

#[tokio::test]
async fn every_publication_is_a_row_of_the_history_and_a_rollback_adds_one() {
    let w = world().await;
    let (id, _) = w.publish(doc("flow", "one")).await;
    for marker in ["two", "three"] {
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
            &doc("flow", marker),
        )
        .await;
        w.defs
            .publish(
                &w.admin,
                id,
                revision,
                None,
                &format!("publish-{marker}-0123456789"),
                &ctx(),
            )
            .await
            .expect("published");
    }
    let record = w
        .defs
        .get_workflow(&w.admin, id, &ctx())
        .await
        .expect("workflow");
    assert_eq!(record.active.as_ref().map(|v| v.number()), Some(3));

    let ops = operator(&w, "olivia").await;
    let rolled = w
        .publishing
        .activate(
            &ops,
            id,
            2,
            "version 3 shows a regression in billing",
            Some(3),
            false,
            &ctx(),
        )
        .await
        .expect("rollback");
    assert!(rolled.changed);
    assert_eq!(rolled.record.kind, ActivationKind::Rollback);
    assert_eq!(
        (rolled.record.version, rolled.record.previous),
        (2, Some(3))
    );
    assert_eq!(rolled.record.actor, "olivia");
    let record = w
        .defs
        .get_workflow(&w.admin, id, &ctx())
        .await
        .expect("workflow");
    assert_eq!(record.active.as_ref().map(|v| v.number()), Some(2));

    // The versions themselves are untouched: 3 is still there, as published.
    let versions = w
        .defs
        .list_versions(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("versions");
    assert_eq!(versions.items.len(), 3);

    // Forward again is an activation, not a rollback.
    let forward = w
        .publishing
        .activate(
            &ops,
            id,
            3,
            "fixed in staging, back to three",
            Some(2),
            false,
            &ctx(),
        )
        .await
        .expect("forward");
    assert_eq!(forward.record.kind, ActivationKind::Activate);

    let history = w
        .publishing
        .activations(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("history");
    let steps: Vec<(u32, Option<u32>, ActivationKind)> = history
        .items
        .iter()
        .map(|a| (a.version, a.previous, a.kind))
        .collect();
    assert_eq!(
        steps,
        [
            (3, Some(2), ActivationKind::Activate),
            (2, Some(3), ActivationKind::Rollback),
            (3, Some(2), ActivationKind::Publish),
            (2, Some(1), ActivationKind::Publish),
            (1, None, ActivationKind::Publish),
        ],
        "newest first"
    );
}

#[tokio::test]
async fn a_new_run_uses_the_active_version_and_one_in_flight_keeps_its_own() {
    let w = world().await;
    let (id, _) = w.publish(doc("flow", "one")).await;
    let before = w.start(id, &json!({})).await;
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
        &doc("flow", "two"),
    )
    .await;
    w.defs
        .publish(&w.admin, id, revision, None, "key-p2-0123456789", &ctx())
        .await
        .expect("v2");
    let on_two = w.start(id, &json!({})).await;

    let ops = operator(&w, "olivia").await;
    w.publishing
        .activate(&ops, id, 1, "version 2 misbehaves", Some(2), false, &ctx())
        .await
        .expect("rollback");
    let after = w.start(id, &json!({})).await;

    let number = |run| {
        let w = &w;
        async move {
            let record = w.run_record(run).await.expect("run");
            w.defs.get_version(&w.admin, id, 0, &ctx()).await.ok();
            record.info.version_id
        }
    };
    let v1 = w
        .defs
        .get_version(&w.admin, id, 1, &ctx())
        .await
        .expect("v1")
        .version
        .id();
    let v2 = w
        .defs
        .get_version(&w.admin, id, 2, &ctx())
        .await
        .expect("v2")
        .version
        .id();
    assert_eq!(number(before).await, v1);
    assert_eq!(
        number(on_two).await,
        v2,
        "the run that started on 2 keeps 2"
    );
    assert_eq!(
        number(after).await,
        v1,
        "new runs use the version the pointer was moved to"
    );
}

#[tokio::test]
async fn a_rollback_is_refused_when_the_active_version_is_not_the_one_the_operator_saw() {
    let w = world().await;
    let (id, _) = w.publish(doc("flow", "one")).await;
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
        &doc("flow", "two"),
    )
    .await;
    w.defs
        .publish(&w.admin, id, revision, None, "key-p2-0123456789", &ctx())
        .await
        .expect("v2");
    let ops = operator(&w, "olivia").await;

    // Another operator moved it while this one was deciding.
    w.publishing
        .activate(
            &ops,
            id,
            1,
            "someone else got there first",
            Some(2),
            false,
            &ctx(),
        )
        .await
        .expect("first");
    let stale = w
        .publishing
        .activate(
            &ops,
            id,
            1,
            "i also want version one",
            Some(2),
            false,
            &ctx(),
        )
        .await
        .expect_err("stale");
    assert_eq!(refusal(stale), Refusal::ActiveChanged { current: Some(1) });
    // Asking for what is already active is not an error and adds nothing.
    let same = w
        .publishing
        .activate(&ops, id, 1, "i want version one", None, false, &ctx())
        .await
        .expect("already active");
    assert!(!same.changed);
    let history = w
        .publishing
        .activations(&w.admin, id, &Default::default(), &ctx())
        .await
        .expect("history");
    assert_eq!(history.items.len(), 3);
}

#[tokio::test]
async fn only_those_with_the_permission_move_the_pointer_and_never_without_a_reason() {
    let w = world().await;
    let (id, _) = w.publish(doc("flow", "one")).await;
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
        &doc("flow", "two"),
    )
    .await;
    w.defs
        .publish(&w.admin, id, revision, None, "key-p2-0123456789", &ctx())
        .await
        .expect("v2");

    let dev = crate::common::developer(&w, "dev").await;
    let viewer = w.member("vera", sagat_domain::identity::Role::Viewer).await;
    for who in [&dev, &viewer] {
        assert!(matches!(
            w.publishing
                .activate(who, id, 1, "a perfectly good reason", None, false, &ctx())
                .await,
            Err(sagat_application::error::AppError::Forbidden { .. })
        ));
    }
    let ops = operator(&w, "olivia").await;
    assert!(
        w.publishing
            .activate(&ops, id, 1, "no", None, false, &ctx())
            .await
            .is_err(),
        "a reason is needed"
    );
    assert!(matches!(
        w.publishing
            .activate(
                &ops,
                id,
                9,
                "a version that is not there",
                None,
                false,
                &ctx()
            )
            .await,
        Err(sagat_application::error::AppError::NotFound)
    ));
}

fn triggered(name: &str, marker: &str, triggers: serde_json::Value) -> serde_json::Value {
    let mut d = doc(name, marker);
    d["triggers"] = triggers;
    d
}

async fn registered(
    w: &crate::support::engine::World,
    workflow: sagat_domain::definition::WorkflowId,
) -> Vec<(String, uuid::Uuid)> {
    sqlx::query_as(
        "SELECT match_type, version_id FROM workflow_triggers WHERE workflow_id = $1 ORDER BY match_type",
    )
    .bind(workflow.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("triggers")
}

#[tokio::test]
async fn the_triggers_in_force_are_those_of_the_active_version_and_follow_a_rollback() {
    let w = world().await;
    let created = json!([{"kind": "event", "eventType": "order.created"}]);
    let updated = json!([
        {"kind": "event", "eventType": "order.created"},
        {"kind": "event", "eventType": "order.updated"}
    ]);
    let (id, _) = w.publish(triggered("flow", "one", created)).await;
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
        &triggered("flow", "two", updated),
    )
    .await;
    w.defs
        .publish(&w.admin, id, revision, None, "key-two-0123456789", &ctx())
        .await
        .expect("v2");
    let v1 = w
        .defs
        .get_version(&w.admin, id, 1, &ctx())
        .await
        .expect("v1")
        .version
        .id()
        .as_uuid();
    let v2 = w
        .defs
        .get_version(&w.admin, id, 2, &ctx())
        .await
        .expect("v2")
        .version
        .id()
        .as_uuid();
    assert_eq!(
        registered(&w, id).await,
        [
            ("order.created".to_owned(), v2),
            ("order.updated".to_owned(), v2)
        ]
    );
    let created_row: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM workflow_triggers WHERE workflow_id = $1 AND match_type = 'order.created'",
    )
    .bind(id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("row");

    // Going back removes the trigger version 1 never had, and the one both have stays the same row,
    // so an event that was handled once is not handled again because the pointer moved.
    let ops = operator(&w, "olivia").await;
    // A trigger that goes away is a contract with emitters that ends: the operator is told.
    let refused = w
        .publishing
        .activate(
            &ops,
            id,
            1,
            "version 2 listens to too much",
            Some(2),
            false,
            &ctx(),
        )
        .await
        .expect_err("a trigger is removed");
    match refusal(refused) {
        Refusal::BreakingChange { codes, .. } => {
            assert!(codes.contains(&"TRIGGER_REMOVED"), "{codes:?}")
        }
        other => panic!("expected a breaking change, got {other:?}"),
    }
    assert_eq!(registered(&w, id).await.len(), 2, "and nothing moved");
    w.publishing
        .activate(
            &ops,
            id,
            1,
            "version 2 listens to too much; emitters are told",
            Some(2),
            true,
            &ctx(),
        )
        .await
        .expect("accepted");
    assert_eq!(registered(&w, id).await, [("order.created".to_owned(), v1)]);
    let after: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM workflow_triggers WHERE workflow_id = $1 AND match_type = 'order.created'",
    )
    .bind(id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(
        after, created_row,
        "the same trigger, pointed at the other version"
    );

    // And forward again: the trigger comes back.
    w.publishing
        .activate(&ops, id, 2, "version 2 is fixed", Some(1), false, &ctx())
        .await
        .expect("forward");
    assert_eq!(registered(&w, id).await.len(), 2);
}

#[tokio::test]
async fn a_rollback_that_breaks_what_callers_rely_on_must_be_accepted_by_the_operator() {
    let w = world().await;
    let with_note = |marker: &str, properties: serde_json::Value| {
        let mut d = doc("flow", marker);
        d["inputSchema"] = json!({"type": "object", "properties": properties});
        d
    };
    let (id, _) = w
        .publish(with_note("one", json!({"orderId": {"type": "string"}})))
        .await;
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
        &with_note(
            "two",
            json!({"orderId": {"type": "string"}, "note": {"type": "string"}}),
        ),
    )
    .await;
    w.defs
        .publish(&w.admin, id, revision, None, "key-two-0123456789", &ctx())
        .await
        .expect("v2");
    let ops = operator(&w, "olivia").await;

    // Callers have started to send `note` since version 2: going back drops it.
    let refused = w
        .publishing
        .activate(
            &ops,
            id,
            1,
            "version 2 shows a regression",
            Some(2),
            false,
            &ctx(),
        )
        .await
        .expect_err("breaking");
    match refusal(refused) {
        Refusal::BreakingChange { findings, codes } => {
            assert!(findings >= 1 && !codes.is_empty(), "{codes:?}");
        }
        other => panic!("expected a breaking change, got {other:?}"),
    }
    let record = w
        .defs
        .get_workflow(&w.admin, id, &ctx())
        .await
        .expect("workflow");
    assert_eq!(
        record.active.as_ref().map(|v| v.number()),
        Some(2),
        "nothing moved"
    );

    let accepted = w
        .publishing
        .activate(
            &ops,
            id,
            1,
            "version 2 shows a regression; callers will be told",
            Some(2),
            true,
            &ctx(),
        )
        .await
        .expect("accepted");
    assert_eq!(accepted.record.kind, ActivationKind::Rollback);
}
