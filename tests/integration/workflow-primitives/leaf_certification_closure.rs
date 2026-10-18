//! Real collection closure versus certification, from a committed final checkpoint.

use super::{
    late_reports::{observe_first, observe_order},
    refusals::{counts, refusal_state},
    *,
};
use crate::support::scope_flow::RunHold;
use sagat_application::primitives::{CollectionFinish, CollectionStore, CollectionTransition};
use sagat_domain::{execution::ExecutionInstanceId, identity::Subject};
use sagat_scheduler::CollectionAdvance;
use time::OffsetDateTime;
use tokio::time::timeout;

const FINISHER: &str =
    "SELECT namespace_id,workflow_version_id,run_version,status,stop_reason,stop_code";
const CERTIFIER: &str = "SELECT workflow_version_id,run_version,trace_id FROM runs";

#[tokio::test]
async fn actual_collection_closure_and_certification_obey_both_original_run_queue_orders() {
    for certifier_first in [false, true] {
        let name = format!("leaf-close-{certifier_first}");
        let f = Flow::start(&name, document(&name), json!([{"v":1}])).await;
        sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
            .execute(&f.api.db.pool)
            .await
            .expect("test-only dormant activation");
        f.drive().await;
        let tasks = f.poll().await;
        assert_eq!(tasks.len(), 1);
        f.complete(&tasks[0], json!({"accepted":true})).await;
        let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
        // Exactly one production scheduler transition commits the checkpoint, without running
        // the later finish transition or manufacturing a SUCCEEDED source row.
        assert_eq!(
            f.scheduler
                .advance_collection_from_run(f.tenant, f.run, &owner)
                .await
                .expect("actual checkpoint writer"),
            CollectionAdvance::Checkpointed { index: 0 }
        );
        let boundary = sqlx::query("SELECT s.lifecycle AS scope_lifecycle,s.item_count,s.next_admission_index,s.admitted_count,s.completed_count,s.active_count,i.lifecycle AS iteration_lifecycle,i.slot_held,i.remaining_reservations,(SELECT count(*) FROM iteration_checkpoints c WHERE c.tenant_id=s.tenant_id AND c.run_id=s.run_id AND c.scope_id=s.scope_id) AS checkpoint_count,t.status,t.waiting_kind,t.routed FROM iteration_scopes s JOIN collection_iterations i USING(tenant_id,run_id,scope_id) JOIN steps t ON t.tenant_id=s.tenant_id AND t.run_id=s.run_id AND t.node=s.owner_node AND t.instance_path_nodes='{}' AND t.instance_path_indices='{}' WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='lines' AND s.owner_path_nodes='{}' AND s.owner_path_indices='{}' AND i.iteration_index=0")
            .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("actual checkpoint-ready boundary");
        assert_eq!(
            (
                boundary.get::<String, _>("scope_lifecycle"),
                boundary.get::<i32, _>("item_count"),
                boundary.get::<i32, _>("next_admission_index"),
                boundary.get::<i32, _>("admitted_count"),
                boundary.get::<i32, _>("completed_count"),
                boundary.get::<i32, _>("active_count")
            ),
            ("OPEN".into(), 1, 1, 1, 1, 0)
        );
        assert_eq!(
            (
                boundary.get::<String, _>("iteration_lifecycle"),
                boundary.get::<bool, _>("slot_held"),
                boundary.get::<i32, _>("remaining_reservations"),
                boundary.get::<i64, _>("checkpoint_count")
            ),
            ("CHECKPOINTED".into(), false, 0, 1)
        );
        assert_eq!(
            (
                boundary.get::<String, _>("status"),
                boundary.get::<Option<String>, _>("waiting_kind"),
                boundary.get::<bool, _>("routed")
            ),
            ("WAITING".into(), Some("COLLECTION".into()), false)
        );
        let membership: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("actual flat topology");
        assert_eq!(membership, 0);
        f.verify().await;
        let c = command(&f).await;
        let progress = f
            .api
            .executions
            .collection_progress(f.tenant, f.run, &owner)
            .await
            .expect("actual progress")
            .expect("existing scope");
        // This pinned fixture's expression is bodyResult, the ordered checkpoint result array.
        // Only the evaluated input is supplied; finish_collection executes the real writer.
        let aggregate = serde_json::to_vec(
            &progress
                .results
                .iter()
                .map(|(_, content)| {
                    serde_json::from_slice::<Value>(content).expect("member result")
                })
                .collect::<Vec<_>>(),
        )
        .expect("bodyResult aggregate");
        let (trace, now): (Option<String>, OffsetDateTime) = sqlx::query_as(
            "SELECT trace_id,clock_timestamp() FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("transition metadata");
        let finish = CollectionFinish::from_evaluated_result(
            CollectionTransition::new(
                f.tenant,
                c.identity.namespace_id(),
                f.run,
                c.expected_run_revision,
                owner,
                c.scheduler.clone(),
                Subject::engine(),
                trace,
                now,
                None,
            ),
            progress.scope_id,
            aggregate,
            &progress.results,
            Uuid::now_v7(),
        );
        let store = PgLeafCollectionSuccessStore::new(
            f.api.api_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let checkpoint_state = refusal_state(&f).await;
        let hold = RunHold::take(&f).await;
        let finisher = f.api.executions.finish_collection(&finish);
        let certifier = store.certify_leaf_collection_success(&c);
        tokio::pin!(finisher, certifier);
        let (first, second) = if certifier_first {
            observe_first(certifier.as_mut(), &f.api.db.pool, CERTIFIER).await;
            (CERTIFIER, FINISHER)
        } else {
            observe_first(finisher.as_mut(), &f.api.db.pool, FINISHER).await;
            (FINISHER, CERTIFIER)
        };
        observe_order(
            finisher.as_mut(),
            certifier.as_mut(),
            &f.api.db.pool,
            first,
            second,
        )
        .await;
        hold.release().await;
        let written = if certifier_first {
            assert_eq!(
                timeout(Duration::from_secs(10), certifier.as_mut())
                    .await
                    .expect("certifier deadline"),
                Err(LeafCollectionSuccessError::SourceNotClosed)
            );
            assert_eq!(counts(&f).await, (0, 0));
            assert_eq!(refusal_state(&f).await, checkpoint_state);
            timeout(Duration::from_secs(10), finisher.as_mut())
                .await
                .expect("finisher deadline")
                .expect("actual closure commits")
        } else {
            let written = timeout(Duration::from_secs(10), finisher.as_mut())
                .await
                .expect("finisher deadline")
                .expect("actual closure commits");
            let closed_state = refusal_state(&f).await;
            assert_eq!(
                timeout(Duration::from_secs(10), certifier.as_mut())
                    .await
                    .expect("certifier deadline"),
                Err(LeafCollectionSuccessError::StaleRunRevision {
                    current: written.run_version
                })
            );
            assert_eq!(counts(&f).await, (0, 0));
            assert_eq!(refusal_state(&f).await, closed_state);
            written
        };
        assert!(written.applied);
        let fresh = command(&f).await;
        assert_eq!(fresh.expected_run_revision, written.run_version);
        let source = refusal_state(&f).await;
        let created = store
            .certify_leaf_collection_success(&fresh)
            .await
            .expect("fresh CAS certifies actual closed source");
        assert!(!created.replayed);
        assert_eq!(counts(&f).await, (1, 1));
        assert_eq!(refusal_state(&f).await, source);
        let repeated = f
            .api
            .executions
            .finish_collection(&finish)
            .await
            .expect("original producer descriptor replays after certification");
        assert!(!repeated.applied);
        assert_eq!(repeated.run_version, written.run_version);
        assert_eq!(refusal_state(&f).await, source);
        let replay = store
            .certify_leaf_collection_success(&fresh)
            .await
            .expect("same canonical fact remains stable");
        assert!(replay.replayed);
        assert_eq!(replay.record, created.record);
        assert_eq!(counts(&f).await, (1, 1));
        f.verify().await;
    }
}
