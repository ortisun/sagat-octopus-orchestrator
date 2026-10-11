//! The history of the active version is the order in which the pointer moved, whatever the clocks
//! of the replicas that moved it say.

use futures_util::future::join_all;
use sagat_application::definitions::DefinitionService;
use sagat_application::error::Refusal;
use sagat_application::identity::PageRequest;
use sagat_application::publishing::PublishingService;
use sagat_infra_ir::JsonIrCodec;
use serde_json::json;

use crate::support::engine::{ctx, world};
use crate::{Skewed, assert_chain, chain, pointer};

fn doc(marker: &str) -> serde_json::Value {
    let mut doc =
        crate::support::engine::document("flow", "a", vec![crate::support::engine::activity("a")]);
    doc["metadata"]["description"] = json!(marker);
    doc
}

#[tokio::test]
async fn a_replica_whose_clock_is_behind_still_adds_its_change_at_the_end_of_the_history() {
    let w = world().await;
    let (id, _) = w.publish(doc("one")).await;
    w.publish_new_version(id, doc("two")).await;
    let ops = w
        .member("olga", sagat_domain::identity::Role::Operator)
        .await;

    // A replica whose clock is an hour behind rolls back, and then publishes another version.
    let behind = Skewed { behind: 3600 };
    let slow_publishing = PublishingService::new(w.store.clone(), behind, JsonIrCodec);
    slow_publishing
        .activate(&ops, id, 1, "two misbehaves", Some(2), false, &ctx())
        .await
        .expect("rollback");
    let slow_definitions = DefinitionService::new(w.store.clone(), behind, JsonIrCodec);
    let record = w
        .defs
        .get_workflow(&w.admin, id, &ctx())
        .await
        .expect("workflow");
    let edited = w
        .defs
        .update_draft(
            &w.admin,
            id,
            record.workflow.draft_revision(),
            &serde_json::to_vec(&doc("three")).expect("json"),
            &ctx(),
        )
        .await
        .expect("edit");
    slow_definitions
        .publish(
            &w.admin,
            id,
            edited.workflow.draft_revision(),
            None,
            "key-skewed-0123456789",
            &ctx(),
        )
        .await
        .expect("three");

    // The history, read the way the API reads it, ends with the last change that was made.
    let page = w
        .publishing
        .activations(
            &w.admin,
            id,
            &PageRequest {
                limit: Some(10),
                cursor: None,
            },
            &ctx(),
        )
        .await
        .expect("history");
    let newest = &page.items[0];
    assert_eq!(
        newest.version, 3,
        "the publication of three is the newest change: {:?}",
        page.items
    );
    assert_eq!(pointer(&w, id).await, Some(3));
    assert_chain(&chain(&w, id).await, pointer(&w, id).await);
}

#[tokio::test]
async fn operators_who_move_the_pointer_at_once_move_it_once_and_leave_a_chain() {
    let w = world().await;
    let (id, _) = w.publish(doc("one")).await;
    w.publish_new_version(id, doc("two")).await;
    w.publish_new_version(id, doc("three")).await;
    let ops = w
        .member("olga", sagat_domain::identity::Role::Operator)
        .await;

    // Eight operators all saw version 3 and all want to leave it, for 1 or for 2.
    let attempts = join_all((0..8u32).map(|i| {
        let (w, ops) = (&w, &ops);
        async move {
            w.publishing
                .activate(
                    ops,
                    id,
                    1 + i % 2,
                    "three misbehaves, going back",
                    Some(3),
                    false,
                    &ctx(),
                )
                .await
        }
    }))
    .await;
    let moved: Vec<_> = attempts.iter().filter(|a| a.is_ok()).collect();
    assert_eq!(
        moved.len(),
        1,
        "the pointer is moved by exactly one of them"
    );
    for refused in attempts.iter().filter_map(|a| a.as_ref().err()) {
        match refused {
            sagat_application::error::AppError::Versioning(Refusal::ActiveChanged { current }) => {
                assert_eq!(
                    current.map(i32::try_from).transpose().expect("number"),
                    pointer(&w, id).await,
                    "the refusal says where the pointer is now"
                );
            }
            other => panic!("expected ACTIVE_VERSION_CHANGED, got {other:?}"),
        }
    }
    let rows = chain(&w, id).await;
    assert_eq!(
        rows.len(),
        4,
        "three publications and one rollback: {rows:?}"
    );
    assert_chain(&rows, pointer(&w, id).await);
}

#[tokio::test]
async fn publications_and_rollbacks_racing_leave_a_chain_that_ends_at_the_active_version() {
    let w = world().await;
    let (id, _) = w.publish(doc("one")).await;
    w.publish_new_version(id, doc("two")).await;
    let ops = w
        .member("olga", sagat_domain::identity::Role::Operator)
        .await;

    // While a developer publishes more versions, an operator keeps moving the pointer back and
    // forth without saying what they expect (they just want it somewhere sane).
    let publisher = async {
        for n in 3..=8 {
            w.publish_new_version(id, doc(&format!("v{n}"))).await;
        }
    };
    let operator = async {
        for round in 0..12u32 {
            let target = 1 + round % 2;
            let _ = w
                .publishing
                .activate(
                    &ops,
                    id,
                    target,
                    "keeping the pointer on a safe version",
                    None,
                    true,
                    &ctx(),
                )
                .await;
        }
    };
    futures_util::future::join(publisher, operator).await;
    let rows = chain(&w, id).await;
    assert_chain(&rows, pointer(&w, id).await);
}
