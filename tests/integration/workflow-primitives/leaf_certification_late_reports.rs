//! Actual stale public reports versus first certification, only the flat ordinary source slice.

use super::{
    refusals::{counts, refusal_state},
    *,
};
use crate::support::{scope_flow::RunHold, worker::machine_code};
use sagat_api_grpc::pb::TaskLease;
use std::{future::Future, pin::Pin};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);
const REPORT: &str = "SELECT run_version FROM runs";
const CERTIFIER: &str = "SELECT workflow_version_id,run_version,trace_id FROM runs";

#[derive(Clone, Copy, Debug)]
enum Report {
    Complete,
    Fail,
}

async fn late_report(f: &Flow, task: &TaskLease, kind: Report) -> Result<(), tonic::Status> {
    match kind {
        Report::Complete => f
            .try_complete(task, json!({"late":true}), "late-complete-new-key")
            .await
            .map(|_| ()),
        // The original key was complete-{step_id}; Flow's fail key is independently new.
        Report::Fail => f.try_fail(task, "LATE_REPORT", false).await,
    }
}

async fn first_waiter(pool: &sqlx::PgPool, fragment: &str) {
    loop {
        let observed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity leader JOIN pg_stat_activity holder ON holder.pid=ANY(pg_blocking_pids(leader.pid)) WHERE leader.datname=current_database() AND holder.datname=leader.datname AND leader.usename='sagat_app' AND leader.wait_event_type='Lock' AND position($1 in leader.query)>0 AND position('SELECT 1 FROM runs' in holder.query)>0)")
            .bind(fragment).fetch_one(pool).await.expect("first original-Run waiter behind actual holder");
        if observed {
            return;
        }
        tokio::task::yield_now().await;
    }
}

async fn ordered_waiters(pool: &sqlx::PgPool, first: &str, second: &str) {
    loop {
        // Observe the actual soft blocker, not just two sessions waiting somewhere in the DB.
        let observed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity leader JOIN pg_stat_activity follower ON leader.pid=ANY(pg_blocking_pids(follower.pid)) WHERE leader.datname=current_database() AND follower.datname=leader.datname AND leader.usename='sagat_app' AND follower.usename='sagat_app' AND leader.wait_event_type='Lock' AND follower.wait_event_type='Lock' AND position($1 in leader.query)>0 AND position($2 in follower.query)>0 AND EXISTS(SELECT 1 FROM pg_stat_activity holder WHERE holder.datname=leader.datname AND holder.pid=ANY(pg_blocking_pids(leader.pid)) AND position('SELECT 1 FROM runs' in holder.query)>0))")
            .bind(first).bind(second).fetch_one(pool).await.expect("observed original-Run queue direction");
        if observed {
            return;
        }
        tokio::task::yield_now().await;
    }
}

pub(super) async fn observe_first<F: Future>(
    operation: Pin<&mut F>,
    pool: &sqlx::PgPool,
    fragment: &str,
) {
    tokio::select! {
        _ = operation => panic!("operation bypassed the held original Run"),
        waited = timeout(WAIT, first_waiter(pool, fragment)) => waited.expect("first waiter deadline"),
    }
}

pub(super) async fn observe_order<A: Future, B: Future>(
    a: Pin<&mut A>,
    b: Pin<&mut B>,
    pool: &sqlx::PgPool,
    first: &str,
    second: &str,
) {
    tokio::select! {
        _ = a => panic!("first operation bypassed the held original Run"),
        _ = b => panic!("second operation bypassed the held original Run"),
        waited = timeout(WAIT, ordered_waiters(pool, first, second)) => waited.expect("queue direction deadline"),
    }
}

#[tokio::test]
async fn stale_new_key_reports_preserve_closed_source_in_both_run_lock_orders() {
    for kind in [Report::Complete, Report::Fail] {
        for certifier_first in [false, true] {
            let name = format!("leaf-late-{kind:?}-{certifier_first}").to_ascii_lowercase();
            let f = Flow::start(&name, document(&name), json!([{"v":1}])).await;
            sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
                .execute(&f.api.db.pool)
                .await
                .expect("test-only dormant source grant");
            f.drive().await;
            let tasks = f.poll().await;
            assert_eq!(tasks.len(), 1);
            let source_task = tasks[0].clone();
            f.complete(&source_task, json!({"accepted":true})).await;
            f.drive().await;
            let scopes: Vec<(String, String)> = sqlx::query_as("SELECT owner_node,lifecycle FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes='{}' AND owner_path_indices='{}' ORDER BY owner_node")
                .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_all(&f.api.db.pool).await.expect("actual closed prefix and active later scope");
            assert_eq!(
                scopes,
                vec![
                    ("later".into(), "OPEN".into()),
                    ("lines".into(), "SUCCEEDED".into())
                ]
            );
            // Ordinary root starts without continuation do not get a chain: this proves only Run
            // tuple order. A chain-member source needs a separately traced barrier.
            let membership: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND run_id=$2",
            )
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("actual chain topology");
            assert_eq!(membership, 0);
            let c = command(&f).await;
            let before = refusal_state(&f).await;
            assert_eq!(counts(&f).await, (0, 0));
            let store = PgLeafCollectionSuccessStore::new(
                f.api.api_pool.clone(),
                DefinitionCache::new(Arc::new(JsonIrCodec), 8),
            );
            let hold = RunHold::take(&f).await;
            let producer = late_report(&f, &source_task, kind);
            let certifier = store.certify_leaf_collection_success(&c);
            tokio::pin!(producer, certifier);
            // Constructing the second future has no effect; only the selected leader is polled
            // until PostgreSQL observes it blocked by RunHold.
            let (first, second) = if certifier_first {
                observe_first(certifier.as_mut(), &f.api.db.pool, CERTIFIER).await;
                (CERTIFIER, REPORT)
            } else {
                observe_first(producer.as_mut(), &f.api.db.pool, REPORT).await;
                (REPORT, CERTIFIER)
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
            let (report, certification) = timeout(WAIT, async {
                tokio::join!(producer.as_mut(), certifier.as_mut())
            })
            .await
            .expect("both actual operations finish after release");
            let status =
                report.expect_err("new-key late report must not replay original completion");
            assert_eq!(status.code(), tonic::Code::Aborted);
            assert_eq!(machine_code(&status), "STALE_ATTEMPT");
            let created = certification.expect("first certification remains valid");
            assert!(!created.replayed);
            assert_eq!(refusal_state(&f).await, before);
            assert_eq!(counts(&f).await, (1, 1));
            let replay = store
                .certify_leaf_collection_success(&c)
                .await
                .expect("same exact source replays");
            assert!(replay.replayed);
            assert_eq!(replay.record, created.record);
            assert_eq!(counts(&f).await, (1, 1));
            assert_eq!(refusal_state(&f).await, before);
            f.verify().await;
        }
    }
}
