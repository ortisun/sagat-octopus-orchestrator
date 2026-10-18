//! Actual lost scoped-Finish COMMIT replies and cold same-command reconciliation.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. This is an in-process database/client
//! response-loss test. It deliberately makes no process-restart, production-caller, group
//! lifecycle, admission, accounting, or non-flat source-closure claim.

use super::*;
use crate::{
    capacity_finish::commit_proxy::{CommitAckLossProxy, CommitResolution},
    support::{api::json_of, eventually},
};
use sagat_application::shards::ShardStore;
use sagat_domain::execution::RunId;
use tokio::time::timeout;

const COMMIT_WAIT: Duration = Duration::from_secs(10);
const SHORT_PENDING_CHECK: Duration = Duration::from_millis(100);
const RUN_ENTRY: &str =
    "SELECT workflow_version_id,run_version,parent_run_id IS NULL AS parentless";

#[derive(Clone)]
struct UncertifiedPublicMember {
    member: CapacityMemberId,
    identity: LeafCollectionPathIdentity,
    run_revision: u64,
    scheduler: SchedulerAuthority,
}

fn positive_advisory_key() -> i64 {
    i64::try_from(
        u64::from_be_bytes(
            Uuid::new_v4().as_bytes()[..8]
                .try_into()
                .expect("eight random bytes"),
        ) >> 1,
    )
    .expect("positive advisory key")
}

pub(super) async fn install_commit_probe(f: &Flow, key: i64, resolution: CommitResolution) {
    let outcome = match resolution {
        CommitResolution::Committed => "RETURN NEW;",
        CommitResolution::Aborted23514 => {
            "RAISE EXCEPTION USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort',MESSAGE='test-only forced scoped Finish commit abort';"
        }
    };
    // The generated positive integer and the two fixed clauses above are the only dynamic DDL.
    let ddl = format!(
        "CREATE FUNCTION scoped_leaf_finish_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key}); {outcome} END $$; CREATE CONSTRAINT TRIGGER scoped_leaf_finish_commit_gate AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION scoped_leaf_finish_commit_probe();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.api.db.pool)
        .await
        .expect("labelled test-only scoped Finish COMMIT probe");
}

pub(super) async fn remove_commit_probe(f: &Flow) {
    sqlx::raw_sql(
        "DROP TRIGGER scoped_leaf_finish_commit_gate ON capacity_group_finish_receipts; \
         DROP FUNCTION scoped_leaf_finish_commit_probe();",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("remove labelled scoped Finish COMMIT probe");
}

pub(super) async fn waiting_commit(f: &Flow, gate_pid: i32) -> i32 {
    eventually(
        "actual scoped Finish COMMIT is executing behind the advisory gate",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity \
                 WHERE datname=current_database() \
                 AND application_name='sagat-test-capacity-commit-loss' \
                 AND query='COMMIT' AND wait_event_type='Lock' \
                 AND $1=ANY(pg_blocking_pids(pid))",
            )
            .bind(gate_pid)
            .fetch_optional(&f.api.db.pool)
            .await
            .expect("actual scoped COMMIT and blocker observation")
        },
    )
    .await
}

fn assert_sqlstate(error: sqlx::Error, expected: &str, label: &str) {
    assert_eq!(
        error
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some(expected),
        "unexpected {label} lock result: {error}"
    );
}

pub(super) async fn own_guards_held(f: &Flow, command: &ScopedLeafFinishCommand) {
    for (statement, resource, label) in [
        (
            "SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT",
            command.identity.run_id().as_uuid(),
            "original Run",
        ),
        (
            "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT",
            command.group_id.as_uuid(),
            "selected G",
        ),
    ] {
        let error = sqlx::query(statement)
            .bind(f.tenant.as_uuid())
            .bind(resource)
            .execute(&f.api.db.pool)
            .await
            .expect_err("unresolved server COMMIT retains native guard");
        assert_sqlstate(error, "55P03", label);
    }
    let shard = sqlx::query(
        "SELECT shard FROM scheduler_shards WHERE shard=run_shard($1) FOR UPDATE NOWAIT",
    )
    .bind(command.identity.run_id().as_uuid())
    .execute(&f.api.db.pool)
    .await
    .expect_err("unresolved server COMMIT retains scheduler SHARE guard");
    assert_sqlstate(shard, "55P03", "scheduler shard");
}

async fn same_run_waiter(f: &Flow, committing_pid: i32) -> i32 {
    eventually(
        "same complete scoped command waits on the actual original Run before G",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT waiter.pid FROM pg_stat_activity waiter \
                 WHERE waiter.datname=current_database() AND waiter.usename='sagat_app' \
                 AND waiter.wait_event_type='Lock' AND position($1 in waiter.query)>0 \
                 AND $2=ANY(pg_blocking_pids(waiter.pid)) LIMIT 1",
            )
            .bind(RUN_ENTRY)
            .bind(committing_pid)
            .fetch_optional(&f.api.db.pool)
            .await
            .expect("same-command original-Run waiter observation")
        },
    )
    .await
}

async fn waiter_no_longer_claims(f: &Flow, waiter_pid: i32) {
    eventually(
        "cancelled same-command claimant leaves no active lock wait",
        || async {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
             WHERE pid=$1 AND state='active' AND wait_event_type='Lock')",
            )
            .bind(waiter_pid)
            .fetch_one(&f.api.db.pool)
            .await
            .expect("cancelled claimant activity observation");
            (!waiting).then_some(())
        },
    )
    .await;
}

async fn start_uncertified_public_member(f: &Flow, label: &str) -> UncertifiedPublicMember {
    let name = format!("sfcl-{label}-{}", Uuid::new_v4());
    let created = f
        .api
        .as_user(reqwest::Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": super::super::document(&name)}))
        .send()
        .await
        .expect("create distinct public collection workflow");
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("distinct public workflow id")
        .to_owned();
    let published = f
        .api
        .as_user(
            reqwest::Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish distinct public collection workflow");
    assert_eq!(published.status(), reqwest::StatusCode::CREATED);
    let started = f
        .api
        .as_user(reqwest::Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {"lines": [{"peer": true}]}}))
        .send()
        .await
        .expect("start distinct public collection Run");
    assert_eq!(started.status(), reqwest::StatusCode::ACCEPTED);
    let run: RunId = json_of(started).await["runId"]
        .as_str()
        .expect("distinct public Run id")
        .parse()
        .expect("distinct public Run id");

    // Admit its real root collection member, but do not lease/complete/certify it. The command
    // below therefore has a genuine public root Run and binding with no source certificate.
    let namespace = sources_namespace(f).await;
    let mut observed_revision = None;
    for _ in 0..20 {
        let ready: Option<i64> = sqlx::query_scalar(
            "SELECT r.run_version FROM runs r \
             WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.id=$3 \
             AND r.parent_run_id IS NULL AND r.run_version>0 \
             AND EXISTS(SELECT 1 FROM iteration_scopes s \
                 JOIN collection_iterations i \
                   ON i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.scope_id=s.scope_id \
                 WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id \
                   AND s.owner_node='lines' AND i.iteration_index=0)",
        )
        .bind(f.tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&f.api.db.pool)
        .await
        .expect("real public peer admission observation");
        if let Some(revision) = ready {
            observed_revision =
                Some(u64::try_from(revision).expect("positive public peer Run revision"));
            break;
        }
        let report = f.scheduler.tick().await;
        assert_eq!(report.failed, 0, "public peer admission failed: {report:?}");
    }
    let run_revision = observed_revision.expect("bounded scheduler admits public peer member");
    let identity = LeafCollectionPathIdentity::new(
        f.tenant,
        namespace,
        run,
        InstancePath::root()
            .child(NodeId::parse("lines").expect("ordinary owner"), 0)
            .expect("depth-one public peer path"),
    )
    .expect("qualified public peer identity");
    let certificate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM leaf_collection_success_certifications \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("public peer certificate absence");
    assert_eq!(certificate_count, 0);
    let chain_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("public peer is a flat root Run");
    assert_eq!(chain_count, 0);

    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=run_shard($1)")
            .bind(run.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("public peer current scheduler authority");
    let scheduler = SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("nonnegative scheduler generation"),
    );
    let renewed = f
        .api
        .executions
        .renew(
            scheduler.owner(),
            &[scheduler.lease()],
            Duration::from_secs(600),
        )
        .await
        .expect("renew distinct public Run authority");
    assert_eq!(renewed, vec![scheduler.lease()]);

    UncertifiedPublicMember {
        member: CapacityMemberId::generate(),
        identity,
        run_revision,
        scheduler,
    }
}

async fn sources_namespace(f: &Flow) -> sagat_domain::identity::NamespaceId {
    let namespace: Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(f.run.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("original public Run namespace");
    sagat_domain::identity::NamespaceId::new(namespace)
}

async fn assert_proxy_role_and_fixture_grants(proxy: &CommitAckLossProxy) {
    let privileges: (String, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT current_user, \
         has_table_privilege(current_user,'leaf_collection_success_certifications','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','member_kind','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','original_run_id','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','root_owner_node','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','iteration_index','INSERT')",
    )
    .fetch_one(&proxy.pool)
    .await
    .expect("proxied scoped Finish role and exact fixture grants");
    assert_eq!(
        privileges,
        ("sagat_app".into(), false, true, true, true, true),
        "proxy uses the runtime role with only the existing four scoped fixture extensions"
    );
}

#[tokio::test]
async fn unknown_scoped_commit_reconciles_real_commit_and_abort_with_the_complete_old_command() {
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        let label = match resolution {
            CommitResolution::Committed => "committed",
            CommitResolution::Aborted23514 => "aborted-23514",
        };
        let (f, sources, fixture_pool) =
            certified_flow(&format!("scoped-finish-commit-loss-{label}"), 1).await;
        drop(fixture_pool);
        let peer = start_uncertified_public_member(&f, label).await;
        let current_source_revision: i64 =
            sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(f.tenant.as_uuid())
                .bind(f.run.as_uuid())
                .fetch_one(&f.api.db.pool)
                .await
                .expect("genuine source Run revision after public peer admission");
        assert_eq!(
            u64::try_from(current_source_revision).expect("nonnegative source Run revision"),
            sources[0].expected_run_revision,
            "public peer admission must not change the genuine source command"
        );
        let selected = CapacityMemberId::generate();
        let seeded = seed_group(
            &f,
            vec![
                active_member(selected, sources[0].identity.clone()),
                active_member(peer.member, peer.identity.clone()),
            ],
            10,
            4,
            Some(0),
        )
        .await;
        let command = finish_command(&sources[0], seeded, 10, 4);
        let original_key = command.operation_key;
        let original_fingerprint = command.fingerprint().expect("complete original command");
        let peer_command = ScopedLeafFinishCommand {
            operation_key: Uuid::new_v4(),
            group_id: seeded.group,
            member_id: peer.member,
            identity: peer.identity.clone(),
            expected: ScopedLeafFinishExpectations {
                group_revision: 10,
                group_epoch: 4,
                run_revision: peer.run_revision,
            },
            scheduler: peer.scheduler.clone(),
        };
        peer_command
            .fingerprint()
            .expect("public uncertified peer command is structurally complete");
        let source_before = source_execution_snapshot(&f).await;
        let capacity_before = capacity_snapshot(&f, seeded.group).await;

        let advisory_key = positive_advisory_key();
        install_commit_probe(&f, advisory_key, resolution).await;
        let mut gate = f
            .api
            .db
            .pool
            .begin()
            .await
            .expect("scoped COMMIT advisory gate");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(advisory_key)
            .execute(&mut *gate)
            .await
            .expect("hold scoped COMMIT gate");
        let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("scoped COMMIT gate pid");

        let mut proxy = CommitAckLossProxy::start(&f.api.db, resolution).await;
        assert_proxy_role_and_fixture_grants(&proxy).await;
        proxy.arm();
        let original_store = PgScopedLeafFinishStore::new(proxy.pool.clone());
        let mut original = Box::pin(original_store.finish_scoped_leaf(&command));
        tokio::select! {
            result = original.as_mut() => panic!("scoped Finish resolved before COMMIT dispatch: {result:?}"),
            () = proxy.wait_sent() => {}
        }
        let committing_pid = timeout(COMMIT_WAIT, waiting_commit(&f, gate_pid))
            .await
            .expect("actual scoped COMMIT wait deadline");
        own_guards_held(&f, &command).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
        assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
        assert_eq!(source_execution_snapshot(&f).await, source_before);

        // Lose only the acknowledged-reply direction while PostgreSQL is still executing the
        // deferred COMMIT. The typed client result is unknown; the server outcome is not yet known.
        proxy.lose_reply().await;
        let unknown = timeout(COMMIT_WAIT, original.as_mut())
            .await
            .expect("scoped unknown outcome deadline")
            .expect_err("dispatched scoped COMMIT reply was lost");
        assert_eq!(
            unknown,
            ScopedLeafFinishError::CommitOutcomeUnknown {
                operation_key: original_key,
            }
        );
        drop(original);
        assert_eq!(command.operation_key, original_key);
        assert_eq!(command.fingerprint(), Ok(original_fingerprint));
        assert_eq!(waiting_commit(&f, gate_pid).await, committing_pid);
        own_guards_held(&f, &command).await;

        let claimant_pool = f.api.db.runtime_pool_with(2).await;
        let claimant_user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&claimant_pool)
            .await
            .expect("direct claimant runtime role");
        assert_eq!(claimant_user, "sagat_app");
        let claimant_store = PgScopedLeafFinishStore::new(claimant_pool.clone());

        // A different real public Run and valid same-G member reaches G NOWAIT before its own
        // scheduler/source stages. Its deliberately absent source certificate is not consulted.
        assert_eq!(
            timeout(
                COMMIT_WAIT,
                claimant_store.finish_scoped_leaf(&peer_command),
            )
            .await
            .expect("distinct public Run reaches selected G"),
            Err(ScopedLeafFinishError::Busy)
        );
        let peer_state: (String, Option<OffsetDateTime>) = sqlx::query_as(
            "SELECT state,finished_at FROM capacity_group_members \
             WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3",
        )
        .bind(f.tenant.as_uuid())
        .bind(seeded.group.as_uuid())
        .bind(peer.member.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("Busy public peer remains active");
        assert_eq!(peer_state, ("ACTIVE".into(), None));
        sqlx::query(
            "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 \
             FOR UPDATE NOWAIT",
        )
        .bind(f.tenant.as_uuid())
        .bind(peer.identity.namespace_id().as_uuid())
        .bind(peer.identity.run_id().as_uuid())
        .execute(&f.api.db.pool)
        .await
        .expect("Busy rollback releases the distinct public Run");

        // Poll the exact original command locally. It must wait at the original Run, before G.
        // Dropping this still-pending claimant is cancellation while the server outcome is unknown;
        // it does not manufacture an aborted resolution or a completed claimant result.
        let same_store = PgScopedLeafFinishStore::new(claimant_pool.clone());
        let mut same = Box::pin(same_store.finish_scoped_leaf(&command));
        let waiter_pid = tokio::select! {
            result = same.as_mut() => panic!("same command resolved before original server outcome: {result:?}"),
            waiter = timeout(COMMIT_WAIT, same_run_waiter(&f, committing_pid)) => {
                waiter.expect("same-command original-Run waiter deadline")
            }
        };
        assert!(
            timeout(SHORT_PENDING_CHECK, same.as_mut()).await.is_err(),
            "no claimant completes before the observed server-side outcome"
        );
        drop(same);
        // Dropping the claimant queues rollback; the blocked server SELECT may remain until
        // the original transaction resolves. Cancellation cannot establish its outcome.
        assert_eq!(waiting_commit(&f, gate_pid).await, committing_pid);
        own_guards_held(&f, &command).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
        assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
        assert_eq!(source_execution_snapshot(&f).await, source_before);

        gate.rollback()
            .await
            .expect("release scoped deferred COMMIT gate");
        assert_eq!(
            proxy.resolution().await,
            resolution,
            "actual backend C/E plus ReadyForQuery idle was never delivered to the client"
        );
        proxy.close().await;
        timeout(COMMIT_WAIT, waiter_no_longer_claims(&f, waiter_pid))
            .await
            .expect("cancelled claimant cleanup after original resolution");
        timeout(COMMIT_WAIT, claimant_pool.close())
            .await
            .expect("cancelled claimant pool closes after server resolution");

        match resolution {
            CommitResolution::Committed => {
                assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
            }
            CommitResolution::Aborted23514 => {
                assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
                assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
            }
        }
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        // Retain the actual settled receipt IDs/time and all atomic effects before cold replay.
        // The absent-receipt abort branch instead captures its first acknowledged application.
        let original_committed = if resolution == CommitResolution::Committed {
            Some(capacity_snapshot(&f, seeded.group).await)
        } else {
            None
        };
        remove_commit_probe(&f).await;

        // Cold pool/store reconstruction retains the complete original command, operation key,
        // identity and expectations. Only current scheduler authority is renewable, and it has not
        // changed in this focused test.
        assert_eq!(command.operation_key, original_key);
        assert_eq!(command.fingerprint(), Ok(original_fingerprint));
        let fresh_pool = f.api.db.runtime_pool_with(2).await;
        let fresh_user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&fresh_pool)
            .await
            .expect("fresh reconciliation role");
        assert_eq!(fresh_user, "sagat_app");
        let fresh_store = PgScopedLeafFinishStore::new(fresh_pool.clone());
        let recovered = timeout(COMMIT_WAIT, fresh_store.finish_scoped_leaf(&command))
            .await
            .expect("cold same-command reconciliation deadline")
            .expect("complete old command resolves the actual server outcome");
        assert_eq!(
            recovered.replayed,
            resolution == CommitResolution::Committed
        );
        assert_eq!(
            (
                recovered.group_id,
                recovered.member_id,
                recovered.original_run_id,
                recovered.revision,
                recovered.epoch,
                recovered.active_members,
                recovered.possession,
            ),
            (
                seeded.group,
                selected,
                f.run,
                11,
                5,
                1,
                CapacityPossession::Vacant,
            )
        );
        assert_linked_effect(&f, &command, &recovered).await;
        let resolved_capacity = capacity_snapshot(&f, seeded.group).await;
        if let Some(original_committed) = original_committed {
            assert_eq!(
                resolved_capacity, original_committed,
                "cold replay retains the exact pre-recovery original receipt and effects"
            );
        }
        assert_header(&f, seeded.group, (11, 5, 1, "VACANT", None)).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        assert_eq!(source_execution_snapshot(&f).await, source_before);

        let mut replay = fresh_store
            .finish_scoped_leaf(&command)
            .await
            .expect("resolved original command has one stable receipt");
        assert!(replay.replayed);
        let mut normalized = recovered.clone();
        normalized.replayed = false;
        replay.replayed = false;
        assert_eq!(replay, normalized);
        assert_eq!(
            capacity_snapshot(&f, seeded.group).await,
            resolved_capacity,
            "subsequent replay retains the exact original or first recovered application"
        );
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        assert_eq!(command.operation_key, original_key);
        assert_eq!(command.fingerprint(), Ok(original_fingerprint));
        crate::hint_contract::assert_outbox_matches_the_contract(&f.api.db.pool).await;
        f.verify().await;
        fresh_pool.close().await;
    }
}
