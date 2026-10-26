//! Actual scoped-Finish reconciliation client process death and restart.
//!
//! Install as a sibling of `commit_loss` under `scoped_leaf_finish_native.rs`. The original
//! uncertain writer remains in the parent process; this proves only reconciliation-client process
//! death, durable complete-command reconstruction, and fresh scheduler-authority discovery.

use super::{
    commit_loss::{install_commit_probe, own_guards_held, remove_commit_probe, waiting_commit},
    *,
};
use crate::{
    capacity_finish::commit_proxy::{CommitAckLossProxy, CommitResolution},
    support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, eventually},
};
use sagat_application::shards::ShardStore;
use sagat_domain::{
    audit::AuditId,
    execution::RunId,
    identity::{NamespaceId, TenantId},
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{path::Path, process::Stdio};
use tokio::{process::Child, time::timeout};

const PROBE: &str =
    "leaf_certification_native::scoped_finish::process::native_reconciliation_probe";
const APP: &str = "sagat-test-scoped-finish-reconciliation-process";
const PROCESS_WAIT: Duration = Duration::from_secs(15);
const RUN_ENTRY: &str =
    "SELECT workflow_version_id,run_version,parent_run_id IS NULL AS parentless";

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("private metadata file"))
        .expect("private metadata JSON")
}

fn uuid_field(value: &Value, key: &str) -> Uuid {
    value[key]
        .as_str()
        .and_then(|text| Uuid::parse_str(text).ok())
        .unwrap_or_else(|| panic!("qualified UUID field {key}"))
}

fn u64_field(value: &Value, key: &str) -> u64 {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("nonnegative integer field {key}"))
}

fn command_json(command: &ScopedLeafFinishCommand) -> Value {
    let [segment] = command.identity.path().segments() else {
        panic!("durable scoped Finish command has exactly one path segment");
    };
    json!({
        "tenantId": command.identity.tenant_id().to_string(),
        "namespaceId": command.identity.namespace_id().to_string(),
        "runId": command.identity.run_id().to_string(),
        "ownerNode": segment.scope_node().as_str(),
        "iterationIndex": segment.iteration_index(),
        "groupId": command.group_id.to_string(),
        "memberId": command.member_id.to_string(),
        "operationKey": command.operation_key.to_string(),
        "expectedGroupRevision": command.expected.group_revision,
        "expectedGroupEpoch": command.expected.group_epoch,
        "expectedRunRevision": command.expected.run_revision,
    })
}

fn parse_durable_command(value: &Value, owner: String, generation: u64) -> ScopedLeafFinishCommand {
    assert_eq!(
        value.as_object().expect("durable command object").len(),
        11,
        "the private file is the complete closed scoped command and no credential"
    );
    let tenant = TenantId::new(uuid_field(value, "tenantId"));
    let namespace = NamespaceId::new(uuid_field(value, "namespaceId"));
    let run = RunId::new(uuid_field(value, "runId"));
    let identity = LeafCollectionPathIdentity::new(
        tenant,
        namespace,
        run,
        InstancePath::root()
            .child(
                NodeId::parse(value["ownerNode"].as_str().expect("owner node"))
                    .expect("qualified owner node"),
                u32::try_from(u64_field(value, "iterationIndex")).expect("bounded iteration index"),
            )
            .expect("depth-one durable path"),
    )
    .expect("qualified durable leaf identity");
    ScopedLeafFinishCommand {
        operation_key: uuid_field(value, "operationKey"),
        group_id: CapacityGroupId::new(uuid_field(value, "groupId")),
        member_id: CapacityMemberId::new(uuid_field(value, "memberId")),
        identity,
        expected: ScopedLeafFinishExpectations {
            group_revision: u64_field(value, "expectedGroupRevision"),
            group_epoch: u64_field(value, "expectedGroupEpoch"),
            run_revision: u64_field(value, "expectedRunRevision"),
        },
        scheduler: SchedulerAuthority::for_run(owner, run, generation),
    }
}

fn result_json(result: &ScopedLeafFinishResult) -> Value {
    let (possession, holder) = match result.possession {
        CapacityPossession::Held(holder) => ("HELD", Some(holder.to_string())),
        CapacityPossession::Vacant => ("VACANT", None),
        CapacityPossession::Retired => ("RETIRED", None),
    };
    json!({
        "groupId": result.group_id.to_string(),
        "memberId": result.member_id.to_string(),
        "originalRunId": result.original_run_id.to_string(),
        "revision": result.revision,
        "epoch": result.epoch,
        "activeMembers": result.active_members,
        "possession": possession,
        "holderMemberId": holder,
        "finishedAtUnixSeconds": result.finished_at.unix_timestamp(),
        "finishedAtNanosecond": result.finished_at.nanosecond(),
        "auditId": result.audit_id.to_string(),
        "outboxId": result.outbox_id.to_string(),
        "replayed": result.replayed,
    })
}

fn result_from_json(value: &Value) -> ScopedLeafFinishResult {
    let possession = match value["possession"].as_str().expect("closed possession") {
        "HELD" => {
            CapacityPossession::Held(CapacityMemberId::new(uuid_field(value, "holderMemberId")))
        }
        "VACANT" if value["holderMemberId"].is_null() => CapacityPossession::Vacant,
        "RETIRED" if value["holderMemberId"].is_null() => CapacityPossession::Retired,
        other => panic!("invalid closed result possession {other}"),
    };
    let seconds = value["finishedAtUnixSeconds"]
        .as_i64()
        .expect("result timestamp seconds");
    let nanosecond = u32::try_from(u64_field(value, "finishedAtNanosecond"))
        .expect("result timestamp nanosecond");
    let finished_at = OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(seconds) * 1_000_000_000 + i128::from(nanosecond),
    )
    .expect("representable result timestamp");
    ScopedLeafFinishResult {
        group_id: CapacityGroupId::new(uuid_field(value, "groupId")),
        member_id: CapacityMemberId::new(uuid_field(value, "memberId")),
        original_run_id: RunId::new(uuid_field(value, "originalRunId")),
        revision: u64_field(value, "revision"),
        epoch: u64_field(value, "epoch"),
        active_members: u32::try_from(u64_field(value, "activeMembers"))
            .expect("bounded active member count"),
        possession,
        finished_at,
        audit_id: AuditId::new(uuid_field(value, "auditId")),
        outbox_id: uuid_field(value, "outboxId"),
        replayed: value["replayed"].as_bool().expect("replayed result flag"),
    }
}

fn launch(f: &Flow, command: &Path, result: &Path) -> Child {
    // The database URL is a child-only environment credential. It never enters argv, either
    // private metadata file, stdout, or stderr. Each re-exec creates its own runtime pool.
    tokio::process::Command::new(std::env::current_exe().expect("integration executable"))
        .args([
            "--exact",
            PROBE,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SAGAT_SCOPED_FINISH_PROCESS_PROBE", "1")
        .env(
            "SAGAT_SCOPED_FINISH_PROCESS_DATABASE_URL",
            f.api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD),
        )
        .env("SAGAT_SCOPED_FINISH_PROCESS_COMMAND", command)
        .env("SAGAT_SCOPED_FINISH_PROCESS_RESULT", result)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn private scoped Finish reconciliation process")
}

async fn stored_result(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    replayed: bool,
) -> Option<ScopedLeafFinishResult> {
    let row = sqlx::query(
        "SELECT resulting_revision,resulting_epoch,resulting_active_members, \
         resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id \
         FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .fetch_optional(&f.api.db.pool)
    .await
    .expect("canonical scoped Finish receipt lookup")?;
    let possession_text: String = row.get("resulting_possession");
    let holder: Option<Uuid> = row.get("resulting_holder_member_id");
    let possession = match (possession_text.as_str(), holder) {
        ("HELD", Some(holder)) => CapacityPossession::Held(CapacityMemberId::new(holder)),
        ("VACANT", None) => CapacityPossession::Vacant,
        ("RETIRED", None) => CapacityPossession::Retired,
        _ => panic!("invalid stored scoped Finish possession"),
    };
    Some(ScopedLeafFinishResult {
        group_id: command.group_id,
        member_id: command.member_id,
        original_run_id: command.identity.run_id(),
        revision: u64::try_from(row.get::<i64, _>("resulting_revision"))
            .expect("positive stored revision"),
        epoch: u64::try_from(row.get::<i64, _>("resulting_epoch")).expect("positive stored epoch"),
        active_members: u32::try_from(row.get::<i64, _>("resulting_active_members"))
            .expect("bounded stored active count"),
        possession,
        finished_at: row.get("finished_at"),
        audit_id: AuditId::new(row.get::<Uuid, _>("audit_id")),
        outbox_id: row.get::<Uuid, _>("outbox_id"),
        replayed,
    })
}

#[tokio::test]
#[ignore = "private subprocess probe; requires parent-provided database and metadata paths"]
async fn native_reconciliation_probe() {
    assert_eq!(
        std::env::var("SAGAT_SCOPED_FINISH_PROCESS_PROBE")
            .ok()
            .as_deref(),
        Some("1")
    );
    let command_path =
        std::env::var_os("SAGAT_SCOPED_FINISH_PROCESS_COMMAND").expect("command path");
    let durable = read_json(Path::new(&command_path));
    assert_eq!(
        durable.as_object().expect("durable command object").len(),
        11
    );
    let tenant = TenantId::new(uuid_field(&durable, "tenantId"));
    let namespace = NamespaceId::new(uuid_field(&durable, "namespaceId"));
    let run = RunId::new(uuid_field(&durable, "runId"));

    let url = std::env::var("SAGAT_SCOPED_FINISH_PROCESS_DATABASE_URL")
        .unwrap_or_else(|_| panic!("missing private database URL"));
    let options: PgConnectOptions = url
        .parse()
        .unwrap_or_else(|_| panic!("invalid private database URL"));
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.application_name(APP))
        .await
        .unwrap_or_else(|_| panic!("private runtime connection failed"));
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| panic!("runtime role discovery failed"));
    assert_eq!(user, RUNTIME_ROLE);

    // Rediscover only renewable current scheduler authority. Every logical identity, key and CAS
    // expectation comes byte-for-byte from the durable command file above.
    let (owner, generation): (String, i64) = sqlx::query_as(
        "SELECT s.owner,s.generation FROM runs r \
         JOIN scheduler_shards s ON s.shard=run_shard(r.id) \
         WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.id=$3",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap_or_else(|_| panic!("qualified current scheduler authority discovery failed"));
    let command = parse_durable_command(
        &durable,
        owner.clone(),
        u64::try_from(generation).expect("nonnegative current generation"),
    );
    assert_eq!(command_json(&command), durable);
    let fingerprint = command
        .fingerprint()
        .unwrap_or_else(|_| panic!("durable command fingerprint failed"));
    let result = PgScopedLeafFinishStore::new(pool.clone())
        .finish_scoped_leaf(&command)
        .await
        .unwrap_or_else(|_| panic!("native scoped Finish reconciliation failed"));
    let output = json!({
        "command": command_json(&command),
        "fingerprint": fingerprint,
        "discoveredOwner": owner,
        "discoveredGeneration": generation,
        "result": result_json(&result),
    });
    let result_path = std::env::var_os("SAGAT_SCOPED_FINISH_PROCESS_RESULT").expect("result path");
    std::fs::write(
        result_path,
        serde_json::to_vec(&output).expect("closed result JSON"),
    )
    .expect("write closed scoped Finish result metadata");
    pool.close().await;
}

#[tokio::test]
async fn killed_scoped_reconciler_restarts_complete_old_command_after_real_commit_or_abort() {
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        let label = match resolution {
            CommitResolution::Committed => "committed",
            CommitResolution::Aborted23514 => "aborted-23514",
        };
        let (f, sources, fixture_pool) =
            certified_flow(&format!("scoped-finish-process-{label}"), 1).await;
        drop(fixture_pool);
        let selected = CapacityMemberId::generate();
        let seeded = seed_group(
            &f,
            vec![active_member(selected, sources[0].identity.clone())],
            10,
            4,
            Some(0),
        )
        .await;
        let command = finish_command(&sources[0], seeded, 10, 4);
        let fingerprint = command
            .fingerprint()
            .expect("complete original fingerprint");
        let durable = command_json(&command);
        let durable_bytes = serde_json::to_vec(&durable).expect("durable command JSON");
        let source_before = source_execution_snapshot(&f).await;
        let capacity_before = capacity_snapshot(&f, seeded.group).await;
        let files = tempfile::tempdir().expect("private scoped process metadata directory");
        let command_path = files.path().join("command.json");
        let killed_result = files.path().join("killed.json");
        let recovered_result = files.path().join("recovered.json");
        std::fs::write(&command_path, &durable_bytes).expect("persist complete original command");
        assert_eq!(
            std::fs::read(&command_path).expect("durable command"),
            durable_bytes
        );

        let advisory_key = i64::try_from(
            u64::from_be_bytes(
                Uuid::new_v4().as_bytes()[..8]
                    .try_into()
                    .expect("eight random bytes"),
            ) >> 1,
        )
        .expect("positive advisory key");
        install_commit_probe(&f, advisory_key, resolution).await;
        let mut gate = f.api.db.pool.begin().await.expect("original COMMIT gate");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(advisory_key)
            .execute(&mut *gate)
            .await
            .expect("hold original scoped COMMIT gate");
        let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("original scoped COMMIT gate pid");

        let mut proxy = CommitAckLossProxy::start(&f.api.db, resolution).await;
        let proxy_user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&proxy.pool)
            .await
            .expect("proxied original runtime role");
        assert_eq!(proxy_user, RUNTIME_ROLE);
        proxy.arm();
        let original_store = PgScopedLeafFinishStore::new(proxy.pool.clone());
        let mut original = Box::pin(original_store.finish_scoped_leaf(&command));
        tokio::select! {
            result = original.as_mut() => panic!("original scoped writer resolved before COMMIT dispatch: {result:?}"),
            () = proxy.wait_sent() => {}
        }
        let committing_pid = waiting_commit(&f, gate_pid).await;
        proxy.lose_reply().await;
        assert_eq!(
            timeout(Duration::from_secs(5), original.as_mut())
                .await
                .expect("original unknown deadline"),
            Err(ScopedLeafFinishError::CommitOutcomeUnknown {
                operation_key: command.operation_key,
            })
        );
        drop(original);
        assert_eq!(command.fingerprint(), Ok(fingerprint));

        let mut child = launch(&f, &command_path, &killed_result);
        let child_pid = child.id().expect("live scoped reconciliation OS child id");
        let backend_pid = eventually(
            "actual scoped OS child waits on unresolved original Run",
            || async {
                sqlx::query_scalar::<_, i32>(
                    "SELECT pid FROM pg_stat_activity \
                     WHERE datname=current_database() AND usename='sagat_app' \
                     AND application_name=$1 AND position($2 in query)>0 \
                     AND wait_event_type='Lock' AND $3=ANY(pg_blocking_pids(pid))",
                )
                .bind(APP)
                .bind(RUN_ENTRY)
                .bind(committing_pid)
                .fetch_optional(&f.api.db.pool)
                .await
                .expect("scoped child original-Run blocker")
            },
        )
        .await;
        assert!(child.try_wait().expect("scoped child state").is_none());
        own_guards_held(&f, &command).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
        assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
        assert_eq!(source_execution_snapshot(&f).await, source_before);

        child
            .start_kill()
            .expect("kill real scoped reconciliation process");
        let killed = timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("reap killed scoped child deadline")
            .expect("reap killed scoped child");
        assert!(!killed.success());
        assert!(
            !killed_result.exists(),
            "killed blocked child produced no result"
        );
        assert_ne!(
            child_pid,
            std::process::id(),
            "child is a separate OS process"
        );
        assert_eq!(
            std::fs::read(&command_path).expect("unchanged command"),
            durable_bytes
        );
        assert_eq!(waiting_commit(&f, gate_pid).await, committing_pid);
        own_guards_held(&f, &command).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
        assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
        assert_eq!(source_execution_snapshot(&f).await, source_before);

        // Releasing only the original advisory gate lets PostgreSQL resolve the original writer.
        // Backend termination is never used as a substitute for the observed OS process death.
        gate.rollback()
            .await
            .expect("release original scoped COMMIT gate");
        assert_eq!(proxy.resolution().await, resolution);
        proxy.close().await;
        eventually(
            "killed scoped client backend disappears after original resolution",
            || async {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                     WHERE pid=$1 AND datname=current_database() AND application_name=$2)",
                )
                .bind(backend_pid)
                .bind(APP)
                .fetch_one(&f.api.db.pool)
                .await
                .expect("old scoped backend state");
                (!exists).then_some(())
            },
        )
        .await;
        assert!(!killed_result.exists());
        assert!(
            !recovered_result.exists(),
            "no replacement result exists before the replacement process"
        );
        assert_eq!(
            std::fs::read(&command_path).expect("unchanged command"),
            durable_bytes
        );

        let retained = stored_result(&f, &command, true).await;
        match resolution {
            CommitResolution::Committed => {
                assert!(
                    retained.is_some(),
                    "actual committed writer retained its receipt"
                );
                assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
            }
            CommitResolution::Aborted23514 => {
                assert!(
                    retained.is_none(),
                    "actual aborted writer retained no receipt"
                );
                assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
                assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
            }
        }
        let capacity_after_original = capacity_snapshot(&f, seeded.group).await;
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        remove_commit_probe(&f).await;

        // Replace only renewable scheduler authority. The durable file's logical identity, key,
        // and all three expectations remain byte-for-byte unchanged.
        let old_lease = command.scheduler.lease();
        f.api
            .executions
            .release(command.scheduler.owner(), &[old_lease])
            .await
            .expect("release original scheduler authority");
        let fresh = f
            .api
            .executions
            .acquire(
                "scoped-finish-process-fresh-owner",
                &[old_lease.shard],
                Duration::from_secs(60),
            )
            .await
            .expect("actual scoped scheduler takeover");
        assert_eq!(fresh.len(), 1);
        assert!(fresh[0].generation > old_lease.generation);
        assert_eq!(
            std::fs::read(&command_path).expect("unchanged command"),
            durable_bytes
        );

        let mut replacement = launch(&f, &command_path, &recovered_result);
        let replacement_pid = replacement.id().expect("replacement scoped OS child id");
        assert_ne!(replacement_pid, child_pid);
        let status = timeout(PROCESS_WAIT, replacement.wait())
            .await
            .expect("replacement scoped process deadline")
            .expect("reap replacement scoped process");
        assert!(status.success(), "replacement scoped native probe failed");
        assert_eq!(
            std::fs::read(&command_path).expect("unchanged command"),
            durable_bytes
        );
        assert!(!killed_result.exists());

        let recovered_output = read_json(&recovered_result);
        assert_eq!(recovered_output["command"], durable);
        assert_eq!(recovered_output["fingerprint"], json!(fingerprint));
        assert_eq!(
            recovered_output["discoveredOwner"],
            "scoped-finish-process-fresh-owner"
        );
        assert_eq!(
            recovered_output["discoveredGeneration"],
            fresh[0].generation
        );
        let recovered = result_from_json(&recovered_output["result"]);
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
                0,
                CapacityPossession::Retired,
            )
        );
        if let Some(retained) = retained {
            assert_eq!(
                recovered, retained,
                "committed reconciliation returns exact original IDs, time, and counters"
            );
            assert_eq!(
                capacity_snapshot(&f, seeded.group).await,
                capacity_after_original,
                "committed replacement is a pure receipt replay"
            );
        } else {
            assert_ne!(recovered.audit_id.as_uuid(), Uuid::nil());
            assert_ne!(recovered.outbox_id, Uuid::nil());
        }
        let canonical = stored_result(&f, &command, recovered.replayed)
            .await
            .expect("one canonical scoped Finish receipt after restart");
        assert_eq!(canonical, recovered);
        let capacity_after_recovery = capacity_snapshot(&f, seeded.group).await;
        let replay_command = parse_durable_command(
            &durable,
            "scoped-finish-process-fresh-owner".to_owned(),
            fresh[0].generation,
        );
        assert_eq!(command_json(&replay_command), durable);
        assert_eq!(replay_command.fingerprint(), Ok(fingerprint));
        let replay_pool = f.api.db.runtime_pool_with(1).await;
        let replay_user: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&replay_pool)
            .await
            .expect("post-recovery replay runtime role");
        assert_eq!(replay_user, RUNTIME_ROLE);
        let stable_replay = PgScopedLeafFinishStore::new(replay_pool.clone())
            .finish_scoped_leaf(&replay_command)
            .await
            .expect("same durable command replays after process recovery");
        replay_pool.close().await;
        let mut expected_replay = recovered.clone();
        expected_replay.replayed = true;
        assert_eq!(
            stable_replay, expected_replay,
            "subsequent replay preserves the recovered IDs, time, and counters"
        );
        assert_eq!(
            capacity_snapshot(&f, seeded.group).await,
            capacity_after_recovery
        );
        assert_linked_effect(&f, &command, &recovered).await;
        assert_header(&f, seeded.group, (11, 5, 0, "RETIRED", None)).await;
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        assert_eq!(source_execution_snapshot(&f).await, source_before);
        assert_eq!(command.fingerprint(), Ok(fingerprint));
        assert_eq!(
            std::fs::read(&command_path).expect("unchanged command"),
            durable_bytes
        );
        crate::hint_contract::assert_outbox_matches_the_contract(&f.api.db.pool).await;
        f.verify().await;
    }
}
