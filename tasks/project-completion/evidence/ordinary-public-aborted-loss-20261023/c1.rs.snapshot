//! Actual committed scoped-refusal COMMIT whose reply is lost before a fresh API replays it.

use super::*;
use sagat_api_grpc::pb::StepCommandResponse;
use tokio::task::JoinHandle;

// A timeout or assertion failure must cancel the original public reporting task.
struct PendingReport(JoinHandle<Result<tonic::Response<StepCommandResponse>, tonic::Status>>);

impl Drop for PendingReport {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn install_commit_gate(
    pool: &PgPool,
    tenant: uuid::Uuid,
    scope: &str,
    key: &str,
    advisory_key: i64,
    resolution: CommitResolution,
) {
    assert!(
        !scope.contains('\'') && !key.contains('\''),
        "fixed receipt identity is safe for test-only DDL"
    );
    let outcome = match resolution {
        CommitResolution::Committed => "",
        CommitResolution::Aborted23514 => {
            "RAISE EXCEPTION USING ERRCODE='23514',CONSTRAINT='capacity_finish_proxy_forced_abort',MESSAGE='test-only forced public refusal commit abort';"
        }
    };
    let ddl = format!(
        "CREATE FUNCTION public_refusal_commit_probe() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.tenant_id='{tenant}'::uuid AND NEW.scope='{scope}' \
              AND NEW.idempotency_key='{key}' THEN \
             PERFORM pg_advisory_xact_lock({advisory_key}); {outcome} \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE CONSTRAINT TRIGGER public_refusal_commit_gate AFTER INSERT ON command_dedupe \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public_refusal_commit_probe();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install exact deferred refusal COMMIT probe");
}

async fn wait_for_exact_commit(
    pool: &PgPool,
    writer_pid: i32,
    blocker_pid: i32,
    database: &str,
) -> i32 {
    eventually(
        "exact receipt writer is executing COMMIT behind gate B",
        || async {
            sqlx::query_scalar(
                "SELECT writer.pid FROM pg_stat_activity writer \
             JOIN pg_stat_activity blocker ON blocker.pid=$2 \
             WHERE writer.pid=$1 AND writer.datname=$3 AND blocker.datname=writer.datname \
               AND writer.usename='sagat_app' AND writer.application_name=$4 \
               AND writer.state='active' AND writer.query='COMMIT' \
               AND writer.wait_event_type='Lock' AND $2=ANY(pg_blocking_pids(writer.pid)) \
               AND position('pg_advisory_xact_lock' in blocker.query)>0",
            )
            .bind(writer_pid)
            .bind(blocker_pid)
            .bind(database)
            .bind(PROXY_APPLICATION)
            .fetch_optional(pool)
            .await
            .expect("observe exact unresolved refusal COMMIT")
        },
    )
    .await
}

async fn assert_unresolved(platform: &Platform, run: uuid::Uuid, before: &Value) {
    let tenant = platform.tenant.as_uuid();
    let visible = durable_snapshot(&platform.api.db.pool, tenant, run, WORKER).await;
    assert!(
        visible == *before,
        "unresolved COMMIT exposes none of its stop, success, receipt or other writes"
    );
    let error =
        match sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
            .bind(tenant)
            .bind(run)
            .fetch_one(&platform.api.db.pool)
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("unresolved refusal must retain the exact own-Run lock"),
        };
    assert!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref()
            == Some("55P03"),
        "the independent Run probe is refused specifically by the unresolved write lock"
    );
}

async fn assert_rolled_back(
    platform: &Platform,
    completion: &CompleteStepRequest,
    before: &Value,
    lease: &LeaseWitness,
    original_bytes: &[u8],
    phase: &'static str,
) {
    let tenant = platform.tenant.as_uuid();
    let run = uuid::Uuid::parse_str(&completion.run_id).expect("original run id");
    let after = durable_snapshot(&platform.api.db.pool, tenant, run, WORKER).await;
    let before_bytes = serde_json::to_vec(before).expect("serialize original snapshot");
    let after_bytes = serde_json::to_vec(&after).expect("serialize actual rollback snapshot");
    assert!(
        before_bytes == after_bytes,
        "actual aborted COMMIT restores every canonical snapshot byte"
    );
    let original_lease = lease_witness(&platform.api.db.pool, tenant, run).await;
    let lease_unchanged = original_lease == *lease;
    assert!(
        lease_unchanged && original_lease.status == "LEASED",
        "abort preserves every original leased-row field"
    );
    assert!(
        completion.encode_to_vec() == original_bytes,
        "aborted original command remains byte-identical"
    );
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM command_dedupe WHERE tenant_id=$1 AND scope=$3 AND idempotency_key=$4), \
                (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='run.stop_requested' AND detail->>'code'='RUN_DATA_LIMIT'), \
                (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='step.completed' AND node='reserve' AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[16]::integer[]), \
                (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2::text AND action='run.data_limit_exceeded')",
    ).bind(tenant).bind(run).bind(receipt_scope(completion)).bind(&completion.idempotency_key)
        .fetch_one(&platform.api.db.pool).await.expect("read exact absent aborted report effects");
    assert!(
        counts == (0, 0, 0, 0),
        "aborted report has no receipt, stop, success or refusal audit"
    );
    let live: (bool, bool) = sqlx::query_as(
        "SELECT (SELECT lease_expires_at>clock_timestamp() FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3), \
                (SELECT expires_at>clock_timestamp() FROM worker_sessions WHERE tenant_id=$1 AND id=$4)",
    ).bind(tenant).bind(run).bind(lease.step_id)
        .bind(uuid::Uuid::parse_str(&lease.leased_by).expect("original worker id"))
        .fetch_one(&platform.api.db.pool).await.expect("read unchanged live original authority");
    assert!(
        live == (true, true),
        "original session and lease stay live without expiry rewriting"
    );
    let heads: (i64, i64, i64) = sqlx::query_as(
        "SELECT run_version,(SELECT max(seq) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
                (SELECT head_seq FROM telemetry_pending WHERE tenant_id=$1 AND run_id=$2) \
         FROM runs WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant)
    .bind(run)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("read original rollback heads");
    let original_head = before["run"]["run_version"]
        .as_i64()
        .expect("original persisted revision");
    assert!(
        heads == (original_head, original_head, original_head),
        "abort preserves Run, journal and telemetry heads"
    );
    let unlocked: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE NOWAIT")
            .bind(tenant)
            .bind(run)
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("confirmed abort releases the Run guard");
    assert!(
        unlocked == run,
        "independent no-write Run guard probe addresses the exact original Run"
    );
    // Only safe digests/counts are retained before recovery overwrites the rollback state.
    let digest = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    println!(
        "public_refusal_abort_rollback={}",
        json!({
            "phase": phase, "before_sha256": digest(&before_bytes), "after_sha256": digest(&after_bytes),
            "lease_unchanged": lease_unchanged, "receipt_count": counts.0, "stop_count": counts.1,
            "success_count": counts.2, "audit_count": counts.3, "session_and_lease_live": live.0 && live.1,
            "original_journal_head": original_head, "run_guard_released": unlocked == run
        })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_public_scoped_refusal_commit_reply_loss_replays_after_api_reopen() {
    assert!(
        timeout(
            Duration::from_secs(90),
            reply_loss_scenario(CommitResolution::Committed)
        )
        .await
        .is_ok(),
        "public committed refusal response-loss scenario deadline"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_public_scoped_refusal_aborted_commit_reply_loss_applies_after_api_reopen() {
    assert!(
        timeout(
            Duration::from_secs(90),
            reply_loss_scenario(CommitResolution::Aborted23514)
        )
        .await
        .is_ok(),
        "public aborted refusal response-loss scenario deadline"
    );
}

async fn reply_loss_scenario(resolution: CommitResolution) {
    let ReopenedRefusalFixture {
        _server,
        reopened,
        mut proxy,
        completion,
        worker_id,
        session,
        before,
        lease,
        backend_pid,
    } = reopened_refusal_fixture(resolution).await;
    let tenant = reopened.tenant;
    let tenant_uuid = tenant.as_uuid();
    let other_tenant = reopened.other_tenant;
    let run_uuid = uuid::Uuid::parse_str(&completion.run_id).expect("original run id");
    let database = reopened.api.db.name.clone();
    let admin_url = reopened.api.db.admin_url.clone();
    let original_bytes = completion.encode_to_vec();
    let scope = receipt_scope(&completion);
    let key_a = positive_advisory_key().clamp(1, i64::MAX - 1);
    let key_b = key_a + 1;

    install_exact_receipt_gate(
        &reopened.api.db.pool,
        tenant_uuid,
        &scope,
        &completion.idempotency_key,
        key_a,
    )
    .await;
    install_commit_gate(
        &reopened.api.db.pool,
        tenant_uuid,
        &scope,
        &completion.idempotency_key,
        key_b,
        resolution,
    )
    .await;

    let mut gate_a = reopened
        .api
        .db
        .pool
        .begin()
        .await
        .expect("gate A owner transaction");
    let pid_a: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate_a)
        .await
        .expect("gate A backend");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key_a)
        .execute(&mut *gate_a)
        .await
        .expect("hold exact receipt gate A");
    let mut gate_b = reopened
        .api
        .db
        .pool
        .begin()
        .await
        .expect("gate B owner transaction");
    let pid_b: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate_b)
        .await
        .expect("gate B backend");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key_b)
        .execute(&mut *gate_b)
        .await
        .expect("hold exact deferred COMMIT gate B");
    assert!(
        pid_a != pid_b && key_a != key_b,
        "the two gates have independent owners and keys"
    );

    let lookups_before = reopened.api.authentication_lookups();
    let mut worker = reopened.connect(WORKER, TENANT).await;
    worker.worker_id = worker_id.clone();
    worker.session = session.clone();
    let request = worker.req(completion.clone());
    let mut client = worker.client.clone();
    let mut reporting = PendingReport(tokio::spawn(
        async move { client.complete_step(request).await },
    ));

    let receipt_writer =
        wait_for_exact_public_writer(&reopened.api.db.pool, backend_pid, pid_a, &database).await;
    assert!(
        receipt_writer == backend_pid,
        "gate A identifies the exact original refusal receipt writer"
    );
    // All prior authentication/read/checked-path work has completed before arming next-COMMIT loss.
    proxy.arm();
    gate_a
        .rollback()
        .await
        .expect("release receipt gate A after proxy arming");
    proxy.wait_sent().await;
    let committing =
        wait_for_exact_commit(&reopened.api.db.pool, backend_pid, pid_b, &database).await;
    assert!(
        committing == receipt_writer,
        "the same scoped-refusal transaction reaches actual COMMIT"
    );
    assert_unresolved(&reopened, run_uuid, &before).await;

    proxy.lose_reply().await;
    let error = match timeout(Duration::from_secs(5), &mut reporting.0).await {
        Ok(Ok(Err(error))) => error,
        _ => panic!("lost COMMIT reply must return a bounded public error, never an ACK"),
    };
    assert!(
        error.code() == tonic::Code::Unavailable
            && error.message() == "service temporarily unavailable"
            && error
                .metadata()
                .get("x-sagat-error")
                .and_then(|value| value.to_str().ok())
                == Some("UNAVAILABLE")
            && error
                .metadata()
                .get("retry-after-ms")
                .and_then(|value| value.to_str().ok())
                == Some("5000"),
        "public response loss keeps the existing safe typed UNAVAILABLE and retry delay"
    );
    assert!(
        reopened.api.authentication_lookups() > lookups_before,
        "original command uses current authentication"
    );
    assert!(
        wait_for_exact_commit(&reopened.api.db.pool, backend_pid, pid_b, &database).await
            == committing,
        "public failure leaves the exact backend still unresolved at COMMIT"
    );
    assert_unresolved(&reopened, run_uuid, &before).await;

    gate_b
        .rollback()
        .await
        .expect("release deferred COMMIT gate B after observing client failure");
    assert!(
        proxy.resolution().await == resolution,
        "actual backend result and idle resolution are independently observed"
    );
    // The committed branch freezes its independent ACK BEFORE any recovery request.
    let frozen = match resolution {
        CommitResolution::Committed => {
            let (post, receipt, snapshot) =
                inspect_committed_refusal(&reopened, &completion, &before, &lease).await;
            Some((
                format!("{}:{}", completion.run_id, post.run_version),
                post.step_status,
                post.run_status,
                receipt,
                snapshot,
            ))
        }
        CommitResolution::Aborted23514 => {
            assert_rolled_back(
                &reopened,
                &completion,
                &before,
                &lease,
                &original_bytes,
                "after_abort",
            )
            .await;
            None
        }
    };
    sqlx::raw_sql(
        "DROP TRIGGER public_refusal_commit_gate ON command_dedupe; DROP FUNCTION public_refusal_commit_probe();",
    ).execute(&reopened.api.db.pool).await.expect("remove resolved COMMIT probe");
    remove_exact_receipt_gate(&reopened.api.db.pool).await;
    let probes_absent: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM pg_trigger WHERE tgname IN ('public_refusal_receipt_gate','public_refusal_commit_gate')) \
            AND NOT EXISTS(SELECT 1 FROM pg_proc WHERE proname IN ('public_refusal_receipt_gate','public_refusal_commit_probe'))",
    ).fetch_one(&reopened.api.db.pool).await.expect("inspect resolved test probe removal");
    assert!(
        probes_absent,
        "all receipt and deferred COMMIT probes are absent before recovery"
    );
    drop(reporting);
    drop(worker);
    let owner_pool = reopened.api.db.pool.clone();
    let faulted_pool = reopened.api.api_pool.clone();
    drop(reopened);
    proxy.close().await;
    close_pool(faulted_pool, "faulted runtime pool closes").await;
    close_pool(owner_pool, "faulted API owner pool closes").await;

    let db = attach_existing_db(&admin_url, &database).await;
    let direct_pool = db.runtime_pool_with(2).await;
    let api = start_on_db_with_pool(db, runtime_options(), direct_pool).await;
    let recovery = Platform {
        api,
        tenant,
        other_tenant,
    };
    let actual: (String, String) =
        sqlx::query_as("SELECT current_database()::text,current_user::text")
            .fetch_one(&recovery.api.api_pool)
            .await
            .expect("fresh direct recovery pool identity");
    assert!(
        actual.0 == database && actual.1 == RUNTIME_ROLE,
        "second API uses the same DB and a fresh direct sagat_app pool"
    );
    let restored = durable_snapshot(&recovery.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        restored
            == *frozen
                .as_ref()
                .map(|(_, _, _, _, snapshot)| snapshot)
                .unwrap_or(&before),
        "complete second API reopen performs no durable mutation"
    );
    assert!(
        completion.encode_to_vec() == original_bytes,
        "every original public command byte is retained"
    );

    if resolution == CommitResolution::Aborted23514 {
        assert_rolled_back(
            &recovery,
            &completion,
            &before,
            &lease,
            &original_bytes,
            "after_reopen",
        )
        .await;
    }

    let auth_before = recovery.api.authentication_lookups();
    let mut resumed = recovery.connect(WORKER, TENANT).await;
    resumed.worker_id = worker_id;
    resumed.session = session;
    let request = resumed.req(completion.clone());
    let ack = match resumed.client.complete_step(request).await {
        Ok(response) => response.into_inner(),
        Err(_) => {
            panic!(
                "original command recovers after confirmed backend resolution and full API reconstruction"
            )
        }
    };
    assert!(
        ack.already_applied == (resolution == CommitResolution::Committed),
        "committed recovery replays while aborted recovery newly applies the original command"
    );
    assert!(
        recovery.api.authentication_lookups() > auth_before,
        "first recovery command uses current authentication"
    );
    let (
        expected_command,
        expected_step_status,
        expected_run_status,
        committed_receipt,
        committed_snapshot,
    ) = match frozen {
        Some(witness) => witness,
        None => {
            // The newly applied ACK is retained as actual output; expectation comes from owner SQL.
            let (post, receipt, snapshot) =
                inspect_committed_refusal(&recovery, &completion, &before, &lease).await;
            (
                format!("{}:{}", completion.run_id, post.run_version),
                post.step_status,
                post.run_status,
                receipt,
                snapshot,
            )
        }
    };
    assert!(
        ack.command_id == expected_command
            && ack.step_status == expected_step_status
            && ack.run_status == expected_run_status,
        "first delivered ACK matches independently verified receipt and journal head"
    );
    if resolution == CommitResolution::Aborted23514 {
        let lookups = recovery.api.authentication_lookups();
        let request = resumed.req(completion.clone());
        let replay = match resumed.client.complete_step(request).await {
            Ok(response) => response.into_inner(),
            Err(_) => panic!("the original command replays after its first successful recovery"),
        };
        assert!(
            replay.already_applied
                && replay.command_id == expected_command
                && replay.step_status == expected_step_status
                && replay.run_status == expected_run_status,
            "second exact command replays the independently validated new receipt"
        );
        assert!(
            recovery.api.authentication_lookups() > lookups,
            "second recovery command also uses current authentication"
        );
    }
    let replay_receipt = receipt(
        &recovery.api.db.pool,
        tenant_uuid,
        &scope,
        &completion.idempotency_key,
    )
    .await;
    assert!(
        replay_receipt == committed_receipt,
        "recovery preserves every receipt field including its original timestamp"
    );
    let replayed = durable_snapshot(&recovery.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        replayed == committed_snapshot,
        "exact replay creates no execution, payload, quota, audit, outbox or authority effect"
    );

    let cold_store = PgRunStore::new(
        recovery.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(sagat_infra_ir::JsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let run_id = RunId::new(run_uuid);
    assert!(
        cold_store.verify_run(tenant, run_id).await.is_ok(),
        "fresh runtime-role store verifies the committed refusal journal"
    );
    let path = InstancePath::root()
        .child(NodeId::parse("lines").expect("owner node"), 16)
        .expect("original path");
    assert!(
        matches!(
            cold_store.load_path(tenant, run_id, path).await,
            Ok(Some(_))
        ),
        "fresh runtime-role store reads the exact original path"
    );
    let verified = durable_snapshot(&recovery.api.db.pool, tenant_uuid, run_uuid, WORKER).await;
    assert!(
        verified == committed_snapshot,
        "fresh runtime-role verification is read-only"
    );
    drop(cold_store);
    drop(resumed);
    let runtime_pool = recovery.api.api_pool.clone();
    let owner_pool = recovery.api.db.pool.clone();
    drop(recovery);
    close_pool(runtime_pool, "recovery runtime pool closes").await;
    close_pool(owner_pool, "recovery owner pool closes").await;
}
