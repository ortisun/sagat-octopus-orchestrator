//! Connections killed while versions are published and the pointer is moved: whatever a client saw
//! of the failures, the history is a chain, every version exists once, and the pointer is where the
//! last change left it.

use std::time::{Duration, Instant};

use sagat_application::error::AppError;
use sagat_domain::definition::WorkflowId;
use serde_json::json;

use crate::support::engine::{World, ctx, world};
use crate::{assert_chain, chain, pointer};

fn doc(marker: &str) -> serde_json::Value {
    let mut doc =
        crate::support::engine::document("flow", "a", vec![crate::support::engine::activity("a")]);
    doc["metadata"]["description"] = json!(marker);
    doc
}

fn transient(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Unavailable | AppError::Internal | AppError::Busy { .. }
    )
}

/// Puts `marker`'s document in the draft and publishes it, retrying as an SDK would: an edit whose
/// answer was lost is recognised by the draft already holding the document, and a publication
/// carries a key, so repeating it is the same operation.
async fn publish_marker(w: &World, id: WorkflowId, marker: &str, deadline: Instant) {
    let wanted = serde_json::to_vec(&doc(marker)).expect("json");
    let wanted_value: serde_json::Value = serde_json::from_slice(&wanted).expect("json");
    let revision = loop {
        assert!(Instant::now() < deadline, "gave up editing {marker}");
        let Ok(record) = w.defs.get_workflow(&w.admin, id, &ctx()).await else {
            continue;
        };
        if serde_json::from_slice::<serde_json::Value>(&record.draft).ok()
            == Some(wanted_value.clone())
        {
            break record.workflow.draft_revision();
        }
        match w
            .defs
            .update_draft(
                &w.admin,
                id,
                record.workflow.draft_revision(),
                &wanted,
                &ctx(),
            )
            .await
        {
            Ok(done) => break done.workflow.draft_revision(),
            Err(e) if transient(&e) || matches!(e, AppError::RevisionConflict { .. }) => {}
            Err(e) => panic!("editing {marker}: {e:?}"),
        }
    };
    loop {
        assert!(Instant::now() < deadline, "gave up publishing {marker}");
        match w
            .defs
            .publish(
                &w.admin,
                id,
                revision,
                None,
                &format!("storm-key-{marker}-0123456789"),
                &ctx(),
            )
            .await
        {
            Ok(_) => return,
            Err(e) if transient(&e) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(e) => panic!("publishing {marker}: {e:?}"),
        }
    }
}

#[tokio::test]
async fn publications_and_rollbacks_under_killed_connections_leave_one_chain_and_one_version_each()
{
    let w = world().await;
    let (id, _) = w.publish(doc("one")).await;
    let ops = w
        .member("olga", sagat_domain::identity::Role::Operator)
        .await;
    let deadline = Instant::now() + Duration::from_secs(90);

    let storm = {
        let pool = w.db.pool.clone();
        tokio::spawn(async move {
            let until = Instant::now() + Duration::from_secs(3);
            while Instant::now() < until {
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid()",
                )
                .execute(&pool)
                .await;
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
    };

    let developer = async {
        for n in 2..=8 {
            publish_marker(&w, id, &format!("v{n}"), deadline).await;
        }
    };
    let operator = async {
        // Moves the pointer between the first two versions for as long as the storm lasts, and
        // leaves it wherever it is: only that the chain is intact matters.
        let until = Instant::now() + Duration::from_secs(4);
        let mut target = 1;
        while Instant::now() < until {
            match w
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
                .await
            {
                Ok(_) => target = 3 - target,
                // A publication moved the pointer between the check and the change: look again.
                Err(e) if transient(&e) => {}
                Err(AppError::Versioning(sagat_application::error::Refusal::ActiveChanged {
                    ..
                })) => {}
                Err(AppError::NotFound) => {}
                Err(e) => panic!("moving the pointer: {e:?}"),
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    };
    futures_util::future::join(developer, operator).await;
    storm.await.expect("storm");

    let versions: Vec<i32> = sqlx::query_scalar(
        "SELECT version_number FROM workflow_versions WHERE workflow_id = $1 ORDER BY version_number",
    )
    .bind(id.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("versions");
    assert_eq!(
        versions,
        (1..=8).collect::<Vec<_>>(),
        "every version exists once, none was skipped"
    );
    assert_chain(&chain(&w, id).await, pointer(&w, id).await);
}
