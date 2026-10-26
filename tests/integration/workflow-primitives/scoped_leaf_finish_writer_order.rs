//! Actual writer ordering controls for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. These tests prove only Run-tuple
//! serialization between the existing native writers and the dormant scoped Finish adapter.
//! They do not establish a production caller, source authority, nested settlement or capacity
//! admission/accounting.

use super::super::late_reports::{observe_first, observe_order};
use super::*;
use crate::support::scope_flow::RunHold;
use sagat_api_grpc::pb::TaskLease;
use tokio::time::timeout;

const WRITER_WAIT: Duration = Duration::from_secs(10);
const CERTIFIER: &str = "SELECT workflow_version_id,run_version,trace_id FROM runs";
const FINISHER: &str = "SELECT workflow_version_id,run_version,parent_run_id IS NULL AS parentless";
const REPORT: &str = "SELECT run_version FROM runs";

#[derive(Clone, Copy, Debug)]
enum LaterReport {
    Complete,
    Fail,
}

async fn uncertified_flow(name: &str) -> (Flow, CertifyLeafCollectionSuccess, PgPool) {
    let f = Flow::start(name, document(name), json!([{"v": 1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    let source_task = &tasks[0];
    let instance = source_task.instance.as_ref().expect("ordinary scoped task");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, "lines");
    assert_eq!(instance.instance_path[0].iteration_index, 0);
    f.complete(source_task, json!({"accepted": true, "index": 0}))
        .await;
    f.drive().await;
    f.verify().await;

    let source = command(&f).await;
    let certificate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM leaf_collection_success_certifications \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 \
         AND owner_node='lines' AND iteration_index=0",
    )
    .bind(f.tenant.as_uuid())
    .bind(source.identity.namespace_id().as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("no manufactured source certificate");
    assert_eq!(certificate_count, 0);

    let lease = source.scheduler.lease();
    let renewed = f
        .api
        .executions
        .renew(source.scheduler.owner(), &[lease], Duration::from_secs(600))
        .await
        .expect("keep current authority across the writer-order control");
    assert_eq!(renewed, vec![lease]);

    // The actual native certifier needs this temporary capability for the first creation. The
    // race ends and the grant is revoked before any retry/replay assertion below. Existing scoped
    // Finish tests separately prove that its positive path needs source SELECT only.
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("temporary first-source writer");
    sqlx::query(
        "GRANT INSERT (member_kind,original_run_id,root_owner_node,iteration_index) \
         ON capacity_group_finish_receipts TO sagat_app",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("test-only scoped receipt extensions");

    let pool = f.api.db.runtime_pool_with(2).await;
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("native adapter role");
    assert_eq!(user, "sagat_app");
    (f, source, pool)
}

async fn revoke_source_insert(f: &Flow, pool: &PgPool) {
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("remove temporary first-source writer");
    let allowed: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(\
         current_user,'leaf_collection_success_certifications','INSERT')",
    )
    .fetch_one(pool)
    .await
    .expect("effective source INSERT privilege after revocation");
    assert!(!allowed);
}

async fn certificate_count(f: &Flow) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM leaf_collection_success_certifications \
         WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("retained source certificate count")
}

async fn operation_receipt_count(f: &Flow, command: &ScopedLeafFinishCommand) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact logical-operation receipt count")
}

fn assert_retained_source(before: &SourceExecutionSnapshot, after: &SourceExecutionSnapshot) {
    assert_eq!(after.selected_source, before.selected_source);
    assert_eq!(after.certificates, before.certificates);
}

async fn report_later(
    f: &Flow,
    task: &TaskLease,
    report: LaterReport,
) -> Result<(), tonic::Status> {
    match report {
        LaterReport::Complete => f
            .try_complete(
                task,
                json!({"charged": true}),
                "scoped-finish-later-complete",
            )
            .await
            .map(|_| ()),
        LaterReport::Fail => f.try_fail(task, "SCOPED_FINISH_LATER_FAILED", false).await,
    }
}

#[tokio::test]
async fn actual_first_certifier_and_scoped_finish_obey_both_original_run_orders() {
    for certifier_first in [false, true] {
        let name = format!("scoped-finish-certifier-order-{certifier_first}");
        let (f, source, pool) = uncertified_flow(&name).await;
        let seeded = seed_group(
            &f,
            vec![active_member(
                CapacityMemberId::generate(),
                source.identity.clone(),
            )],
            10,
            4,
            Some(0),
        )
        .await;
        let finish_command = finish_command(&source, seeded, 10, 4);
        let capacity_before = capacity_snapshot(&f, seeded.group).await;
        let execution_before = source_execution_snapshot(&f).await;
        let certifier_store = PgLeafCollectionSuccessStore::new(
            f.api.api_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        );
        let finish_store = PgScopedLeafFinishStore::new(pool.clone());
        let hold = RunHold::take(&f).await;
        let certifier = certifier_store.certify_leaf_collection_success(&source);
        let finisher = finish_store.finish_scoped_leaf(&finish_command);
        tokio::pin!(certifier, finisher);

        let (first, second) = if certifier_first {
            observe_first(certifier.as_mut(), &f.api.db.pool, CERTIFIER).await;
            (CERTIFIER, FINISHER)
        } else {
            observe_first(finisher.as_mut(), &f.api.db.pool, FINISHER).await;
            (FINISHER, CERTIFIER)
        };
        observe_order(
            certifier.as_mut(),
            finisher.as_mut(),
            &f.api.db.pool,
            first,
            second,
        )
        .await;
        hold.release().await;

        let (created, applied) = if certifier_first {
            let created = timeout(WRITER_WAIT, certifier.as_mut())
                .await
                .expect("first certifier deadline")
                .expect("actual first certificate commits");
            assert!(!created.replayed);
            let applied = timeout(WRITER_WAIT, finisher.as_mut())
                .await
                .expect("following scoped Finish deadline")
                .expect("certificate-first scoped Finish applies");
            assert!(!applied.replayed);
            (created, applied)
        } else {
            assert_eq!(
                timeout(WRITER_WAIT, finisher.as_mut())
                    .await
                    .expect("source-missing scoped Finish deadline"),
                Err(ScopedLeafFinishError::SourceEvidenceMissing)
            );
            assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
            assert_eq!(operation_receipt_count(&f, &finish_command).await, 0);
            let created = timeout(WRITER_WAIT, certifier.as_mut())
                .await
                .expect("following certifier deadline")
                .expect("actual first certificate commits after refusal");
            assert!(!created.replayed);
            revoke_source_insert(&f, &pool).await;
            let applied = PgScopedLeafFinishStore::new(pool.clone())
                .finish_scoped_leaf(&finish_command)
                .await
                .expect("unchanged source-missing command applies after certification");
            assert!(!applied.replayed);
            (created, applied)
        };

        revoke_source_insert(&f, &pool).await;
        assert_eq!(certificate_count(&f).await, 1);
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        assert_linked_effect(&f, &finish_command, &applied).await;

        let execution_after = source_execution_snapshot(&f).await;
        assert_eq!(
            execution_after.selected_source,
            execution_before.selected_source
        );
        assert_eq!(
            execution_after.run_and_journal,
            execution_before.run_and_journal
        );
        assert_eq!(execution_after.steps, execution_before.steps);
        assert_eq!(execution_after.topology, execution_before.topology);
        assert_ne!(execution_after.certificates, execution_before.certificates);

        let source_replay = certifier_store
            .certify_leaf_collection_success(&source)
            .await
            .expect("genuine certificate replays after source INSERT revocation");
        assert!(source_replay.replayed);
        assert_eq!(source_replay.record, created.record);
        let mut finish_replay = PgScopedLeafFinishStore::new(pool)
            .finish_scoped_leaf(&finish_command)
            .await
            .expect("same scoped Finish returns its original result");
        assert!(finish_replay.replayed);
        finish_replay.replayed = false;
        assert_eq!(finish_replay, applied);
        assert_eq!(certificate_count(&f).await, 1);
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        f.verify().await;
    }
}

#[tokio::test]
async fn actual_later_worker_and_scoped_finish_obey_both_original_run_orders() {
    for report in [LaterReport::Complete, LaterReport::Fail] {
        for finish_first in [false, true] {
            let name =
                format!("scoped-finish-later-{report:?}-{finish_first}").to_ascii_lowercase();
            let (f, sources, pool) = certified_flow(&name, 1).await;
            let later = f.poll().await;
            assert_eq!(later.len(), 1);
            let later_task = later.into_iter().next().expect("leased later work");
            assert_eq!(
                later_task
                    .instance
                    .as_ref()
                    .expect("later scoped identity")
                    .instance_path[0]
                    .scope_node_id,
                "later"
            );
            let seeded = seed_group(
                &f,
                vec![active_member(
                    CapacityMemberId::generate(),
                    sources[0].identity.clone(),
                )],
                10,
                4,
                Some(0),
            )
            .await;
            // Claiming later work commits StepLeased and advances the Run revision. Discover
            // after that real mutation so staleness can only be caused by the observed race.
            let current_before_race = command(&f).await;
            assert!(current_before_race.expected_run_revision > sources[0].expected_run_revision);
            let original = finish_command(&current_before_race, seeded, 10, 4);
            let source_before = source_execution_snapshot(&f).await;
            let capacity_before = capacity_snapshot(&f, seeded.group).await;
            let finish_store = PgScopedLeafFinishStore::new(pool.clone());
            let hold = RunHold::take(&f).await;
            let writer = report_later(&f, &later_task, report);
            let finisher = finish_store.finish_scoped_leaf(&original);
            tokio::pin!(writer, finisher);

            let (first, second) = if finish_first {
                observe_first(finisher.as_mut(), &f.api.db.pool, FINISHER).await;
                (FINISHER, REPORT)
            } else {
                observe_first(writer.as_mut(), &f.api.db.pool, REPORT).await;
                (REPORT, FINISHER)
            };
            observe_order(
                writer.as_mut(),
                finisher.as_mut(),
                &f.api.db.pool,
                first,
                second,
            )
            .await;
            hold.release().await;

            let (applied_command, applied) = if finish_first {
                let applied = timeout(WRITER_WAIT, finisher.as_mut())
                    .await
                    .expect("leading scoped Finish deadline")
                    .expect("leading scoped Finish applies");
                assert!(!applied.replayed);
                timeout(WRITER_WAIT, writer.as_mut())
                    .await
                    .expect("following later writer deadline")
                    .expect("actual later worker report commits");

                let current = command(&f).await;
                assert!(current.expected_run_revision > original.expected.run_revision);
                let mut replay = PgScopedLeafFinishStore::new(pool.clone())
                    .finish_scoped_leaf(&original)
                    .await
                    .expect("original receipt survives the later Run mutation");
                assert!(replay.replayed);
                replay.replayed = false;
                assert_eq!(replay, applied);
                (original.clone(), applied)
            } else {
                timeout(WRITER_WAIT, writer.as_mut())
                    .await
                    .expect("leading later writer deadline")
                    .expect("actual later worker report commits");
                let current = command(&f).await;
                assert!(current.expected_run_revision > original.expected.run_revision);
                assert_eq!(
                    timeout(WRITER_WAIT, finisher.as_mut())
                        .await
                        .expect("following stale scoped Finish deadline"),
                    Err(ScopedLeafFinishError::StaleRunRevision {
                        current: current.expected_run_revision
                    })
                );
                assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
                assert_eq!(operation_receipt_count(&f, &original).await, 0);

                // The actual later write is a separate operation. Rediscovery supplies its new
                // Run revision and a fresh logical key; the refused original key remains absent.
                let fresh = finish_command(&current, seeded, 10, 4);
                assert_ne!(fresh.operation_key, original.operation_key);
                let applied = PgScopedLeafFinishStore::new(pool.clone())
                    .finish_scoped_leaf(&fresh)
                    .await
                    .expect("freshly discovered scoped Finish applies");
                assert!(!applied.replayed);
                assert_eq!(operation_receipt_count(&f, &original).await, 0);
                (fresh, applied)
            };

            assert_linked_effect(&f, &applied_command, &applied).await;
            assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
            assert_header(&f, seeded.group, (11, 5, 0, "RETIRED", None)).await;
            assert_retained_source(&source_before, &source_execution_snapshot(&f).await);

            f.drive().await;
            assert_eq!(
                f.run_status().await,
                match report {
                    LaterReport::Complete => "SUCCEEDED",
                    LaterReport::Fail => "FAILED",
                }
            );
            assert_retained_source(&source_before, &source_execution_snapshot(&f).await);
            let mut replay = PgScopedLeafFinishStore::new(pool)
                .finish_scoped_leaf(&applied_command)
                .await
                .expect("applied scoped Finish remains replayable after later planning");
            assert!(replay.replayed);
            replay.replayed = false;
            assert_eq!(replay, applied);
            assert_eq!(
                operation_receipt_count(&f, &original).await,
                if finish_first { 1 } else { 0 }
            );
            assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
            f.verify().await;
        }
    }
}

#[tokio::test]
async fn two_genuine_scoped_members_complete_in_either_observed_run_queue_order() {
    let (f, sources, pool) = certified_flow("scoped-finish-two-member-orders", 2).await;
    let source_before = source_execution_snapshot(&f).await;

    for first_index in [0_usize, 1] {
        let second_index = 1 - first_index;
        let first_member = CapacityMemberId::generate();
        let second_member = CapacityMemberId::generate();
        let member_for_source = [first_member, second_member];
        let seeded = seed_group(
            &f,
            vec![
                active_member(first_member, sources[0].identity.clone()),
                active_member(second_member, sources[1].identity.clone()),
            ],
            10,
            4,
            None,
        )
        .await;
        let first_seed = SeededGroup {
            group: seeded.group,
            selected: member_for_source[first_index],
        };
        let second_seed = SeededGroup {
            group: seeded.group,
            selected: member_for_source[second_index],
        };
        let first_command = finish_command(&sources[first_index], first_seed, 10, 4);
        let stale_second = finish_command(&sources[second_index], second_seed, 10, 4);
        let first_store = PgScopedLeafFinishStore::new(pool.clone());
        let second_store = PgScopedLeafFinishStore::new(pool.clone());
        let hold = RunHold::take(&f).await;
        let first = first_store.finish_scoped_leaf(&first_command);
        let second = second_store.finish_scoped_leaf(&stale_second);
        tokio::pin!(first, second);

        observe_first(first.as_mut(), &f.api.db.pool, FINISHER).await;
        observe_order(
            first.as_mut(),
            second.as_mut(),
            &f.api.db.pool,
            FINISHER,
            FINISHER,
        )
        .await;
        hold.release().await;

        let first_result = timeout(WRITER_WAIT, first.as_mut())
            .await
            .expect("leading genuine member deadline")
            .expect("leading genuine member applies");
        assert!(!first_result.replayed);
        assert_eq!(first_result.revision, 11);
        assert_eq!(first_result.epoch, 4);
        assert_eq!(first_result.active_members, 1);
        assert_eq!(first_result.possession, CapacityPossession::Vacant);
        assert_eq!(
            timeout(WRITER_WAIT, second.as_mut())
                .await
                .expect("following genuine member deadline"),
            Err(ScopedLeafFinishError::StaleGroupRevision { current: 11 })
        );
        assert_eq!(operation_receipt_count(&f, &stale_second).await, 0);

        let mut discovered_source = command(&f).await;
        discovered_source.identity = sources[second_index].identity.clone();
        let observed_group: (i64, i64) = sqlx::query_as(
            "SELECT revision,epoch FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(seeded.group.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("fresh group expectations after confirmed stale refusal");
        assert_eq!(observed_group, (11, 4));
        let fresh_second = finish_command(
            &discovered_source,
            second_seed,
            u64::try_from(observed_group.0).expect("observed group revision"),
            u64::try_from(observed_group.1).expect("observed group epoch"),
        );
        assert_ne!(fresh_second.operation_key, stale_second.operation_key);
        let second_result = PgScopedLeafFinishStore::new(pool.clone())
            .finish_scoped_leaf(&fresh_second)
            .await
            .expect("fresh second member applies");
        assert!(!second_result.replayed);
        assert_eq!(second_result.revision, 12);
        assert_eq!(second_result.epoch, 5);
        assert_eq!(second_result.active_members, 0);
        assert_eq!(second_result.possession, CapacityPossession::Retired);

        assert_linked_effect(&f, &first_command, &first_result).await;
        assert_linked_effect(&f, &fresh_second, &second_result).await;
        assert_eq!(operation_receipt_count(&f, &stale_second).await, 0);
        assert_eq!(finish_counts(&f, seeded.group).await, (2, 2, 2));
        assert_header(&f, seeded.group, (12, 5, 0, "RETIRED", None)).await;
        assert_retained_source(&source_before, &source_execution_snapshot(&f).await);
    }

    f.verify().await;
}
