//! Later ordinary work and run cancellation preserve the already closed selected source.

use super::{
    late_reports::{observe_first, observe_order},
    refusals::{closed, counts},
    *,
};
use crate::support::scope_flow::RunHold;
use sagat_api_grpc::pb::TaskLease;
use tokio::time::timeout;

const CERTIFIER: &str = "SELECT workflow_version_id,run_version,trace_id FROM runs";
const REPORT: &str = "SELECT run_version FROM runs";
const CANCEL: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id,";

#[derive(Clone, Copy, Debug)]
enum Later {
    Complete,
    Fail,
    CancelReady,
}

async fn produce(f: &Flow, task: Option<&TaskLease>, kind: Later) {
    match kind {
        Later::Complete => {
            f.try_complete(
                task.expect("leased later work"),
                json!({"charged":true}),
                "later-complete",
            )
            .await
            .expect("actual later completion accepted");
        }
        Later::Fail => f
            .try_fail(task.expect("leased later work"), "LATER_DECLINED", false)
            .await
            .expect("actual later failure accepted"),
        Later::CancelReady => {
            let (status, body) = f
                .operate(json!({"action":"CANCEL","reason":"leaf preservation test"}))
                .await;
            assert_eq!(status, reqwest::StatusCode::ACCEPTED, "{body}");
        }
    }
}

#[tokio::test]
async fn later_success_failure_and_ready_cancel_preserve_source_in_both_run_queue_orders() {
    for kind in [Later::Complete, Later::Fail, Later::CancelReady] {
        for certifier_first in [false, true] {
            let name = format!("leaf-later-{kind:?}-{certifier_first}").to_ascii_lowercase();
            let (f, store, _) = closed(&name).await;
            let ready: Vec<(String, String, String, String, bool)> = sqlx::query_as("SELECT t.node,t.status,s.lifecycle,i.lifecycle,(t.leased_by IS NULL AND t.lease_token_hash IS NULL AND t.lease_expires_at IS NULL) FROM steps t JOIN iteration_scopes s ON s.tenant_id=t.tenant_id AND s.run_id=t.run_id AND s.owner_node='later' AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' JOIN collection_iterations i ON i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.scope_id=s.scope_id AND i.iteration_index=0 WHERE t.tenant_id=$1 AND t.run_id=$2 AND t.instance_path_nodes=ARRAY['later']::text[] AND t.instance_path_indices=ARRAY[0]::integer[] AND t.kind='FORWARD'")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_all(&f.api.db.pool).await.expect("actual later admission and READY work before cancellation or claim");
            assert_eq!(
                ready,
                vec![(
                    "work".into(),
                    "READY".into(),
                    "OPEN".into(),
                    "ADMITTED".into(),
                    true
                )]
            );
            let task = if matches!(kind, Later::CancelReady) {
                None
            } else {
                let tasks = f.poll().await;
                assert_eq!(tasks.len(), 1);
                let task = tasks.into_iter().next().expect("later lease");
                assert_eq!(
                    task.instance.as_ref().expect("later path").instance_path[0].scope_node_id,
                    "later"
                );
                Some(task)
            };
            let membership: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND run_id=$2",
            )
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("flat topology");
            assert_eq!(membership, 0);
            let c = command(&f).await;
            let source = selected_sources(&f).await;
            assert_eq!(counts(&f).await, (0, 0));
            let hold = RunHold::take(&f).await;
            let producer = produce(&f, task.as_ref(), kind);
            let certifier = store.certify_leaf_collection_success(&c);
            tokio::pin!(producer, certifier);
            let writer = if matches!(kind, Later::CancelReady) {
                CANCEL
            } else {
                REPORT
            };
            let (first, second) = if certifier_first {
                observe_first(certifier.as_mut(), &f.api.db.pool, CERTIFIER).await;
                (CERTIFIER, writer)
            } else {
                observe_first(producer.as_mut(), &f.api.db.pool, writer).await;
                (writer, CERTIFIER)
            };
            observe_order(
                producer.as_mut(),
                certifier.as_mut(),
                &f.api.db.pool,
                first,
                second,
            )
            .await;
            hold.release().await;
            // Poll the observed leader to completion first. Capture creation before the later
            // mutation or prove the old creation revision is rejected after that real mutation.
            let created = if certifier_first {
                let created = timeout(Duration::from_secs(10), certifier.as_mut())
                    .await
                    .expect("certifier deadline")
                    .expect("first certifies");
                assert!(!created.replayed);
                timeout(Duration::from_secs(10), producer.as_mut())
                    .await
                    .expect("producer deadline");
                created
            } else {
                timeout(Duration::from_secs(10), producer.as_mut())
                    .await
                    .expect("producer deadline");
                let fresh = command(&f).await;
                assert!(
                    fresh.expected_run_revision > c.expected_run_revision,
                    "actual producer changed the Run"
                );
                assert_eq!(
                    timeout(Duration::from_secs(10), certifier.as_mut())
                        .await
                        .expect("certifier deadline"),
                    Err(LeafCollectionSuccessError::StaleRunRevision {
                        current: fresh.expected_run_revision
                    })
                );
                assert_eq!(counts(&f).await, (0, 0));
                let created = store
                    .certify_leaf_collection_success(&fresh)
                    .await
                    .expect("fresh CAS certifies retained source");
                assert!(!created.replayed);
                created
            };
            assert_eq!(selected_sources(&f).await, source);
            assert_eq!(counts(&f).await, (1, 1));
            // Real checkpoint/closure or drain/saga planning proceeds after the racing mutation.
            f.drive().await;
            let expected = match kind {
                Later::Complete => "SUCCEEDED",
                Later::Fail => "FAILED",
                Later::CancelReady => "CANCELLED",
            };
            assert_eq!(f.run_status().await, expected);
            assert_eq!(
                selected_sources(&f).await,
                source,
                "later planning did not reopen or create obligations in selected source"
            );
            let replay = store
                .certify_leaf_collection_success(&c)
                .await
                .expect("original canonical identity survives terminal later work");
            assert!(replay.replayed);
            assert_eq!(replay.record, created.record);
            assert_eq!(counts(&f).await, (1, 1));
            f.verify().await;
        }
    }
}
