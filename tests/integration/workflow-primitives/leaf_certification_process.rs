//! Actual reconciliation client process death/restart; the original uncertain writer stays alive.

use super::{
    commit_loss::{install_commit_probe, own_guards_held, waiting_commit},
    commit_proxy::{CommitAckLossProxy, CommitResolution},
    refusals::{closed, counts, refusal_state},
    *,
};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, eventually};
use sagat_domain::{execution::RunId, identity::TenantId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{path::Path, process::Stdio};
use tokio::{process::Child, time::timeout};

const PROBE: &str = "leaf_certification_native::process::native_reconciliation_probe";
const APP: &str = "sagat-test-leaf-reconciliation-process";

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("private metadata file"))
        .expect("private metadata JSON")
}

fn uuid_field(v: &Value, key: &str) -> Uuid {
    v[key]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("qualified identity UUID")
}

fn identity_json(i: &LeafCollectionPathIdentity) -> Value {
    let path = &i.path().segments()[0];
    json!({"tenantId":i.tenant_id().to_string(),"namespaceId":i.namespace_id().to_string(),
        "runId":i.run_id().to_string(),"ownerNode":path.scope_node().as_str(),
        "index":path.iteration_index()})
}

fn launch(f: &Flow, identity: &Path, result: &Path) -> Child {
    // No credential in arguments, fixture/result files or output. Each re-exec owns a new runtime,
    // pool and definition cache; it cannot borrow the parent's command or in-memory receipt.
    tokio::process::Command::new(std::env::current_exe().expect("integration executable"))
        .args([
            "--exact",
            PROBE,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SAGAT_LEAF_PROCESS_PROBE", "1")
        .env(
            "SAGAT_LEAF_PROCESS_DATABASE_URL",
            f.api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD),
        )
        .env("SAGAT_LEAF_PROCESS_IDENTITY", identity)
        .env("SAGAT_LEAF_PROCESS_RESULT", result)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn private native reconciliation process")
}

#[tokio::test]
#[ignore = "private subprocess probe; requires parent-provided database and metadata paths"]
async fn native_reconciliation_probe() {
    assert_eq!(
        std::env::var("SAGAT_LEAF_PROCESS_PROBE").ok().as_deref(),
        Some("1")
    );
    let path = std::env::var_os("SAGAT_LEAF_PROCESS_IDENTITY").expect("identity path");
    let v = read_json(Path::new(&path));
    assert_eq!(v.as_object().expect("identity object").len(), 5);
    let tenant = TenantId::new(uuid_field(&v, "tenantId"));
    let ns = NamespaceId::new(uuid_field(&v, "namespaceId"));
    let run = RunId::new(uuid_field(&v, "runId"));
    let identity = LeafCollectionPathIdentity::new(
        tenant,
        ns,
        run,
        InstancePath::root()
            .child(
                NodeId::parse(v["ownerNode"].as_str().expect("owner node")).expect("node"),
                u32::try_from(v["index"].as_u64().expect("index")).expect("index bound"),
            )
            .expect("path"),
    )
    .expect("depth1");
    let url = std::env::var("SAGAT_LEAF_PROCESS_DATABASE_URL")
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
    // Discovery is current, fully qualified and local to this child. No supplied authority or CAS.
    let (revision, owner, generation): (i64, String, i64) = sqlx::query_as(
        "SELECT r.run_version,s.owner,s.generation FROM runs r JOIN scheduler_shards s ON s.shard=run_shard(r.id) WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.id=$3")
        .bind(tenant.as_uuid()).bind(ns.as_uuid()).bind(run.as_uuid()).fetch_one(&pool).await
        .unwrap_or_else(|_| panic!("qualified current authority discovery failed"));
    let command = CertifyLeafCollectionSuccess {
        identity,
        expected_run_revision: u64::try_from(revision).expect("revision"),
        scheduler: SchedulerAuthority::for_run(
            &owner,
            run,
            u64::try_from(generation).expect("generation"),
        ),
    };
    let result = PgLeafCollectionSuccessStore::new(
        pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
    .certify_leaf_collection_success(&command)
    .await
    .unwrap_or_else(|_| panic!("native reconciliation failed"));
    let output = json!({"identity":identity_json(&result.record.identity),"replayed":result.replayed,
        "auditId":result.record.audit_id.to_string(),"certifiedRunRevision":result.record.certified_run_revision,
        "discoveredRunRevision":command.expected_run_revision,"discoveredOwner":owner,"discoveredGeneration":generation});
    let result_path = std::env::var_os("SAGAT_LEAF_PROCESS_RESULT").expect("result path");
    std::fs::write(
        result_path,
        serde_json::to_vec(&output).expect("metadata JSON"),
    )
    .expect("write closed result metadata");
    pool.close().await;
}

#[tokio::test]
async fn killed_native_reconciler_restarts_same_identity_after_actual_commit_or_abort() {
    for resolution in [CommitResolution::Committed, CommitResolution::Aborted23514] {
        let (f, store, c) = closed("leaf-process-recovery").await;
        drop(store);
        let before = refusal_state(&f).await;
        let files = tempfile::tempdir().expect("private process metadata directory");
        let identity_path = files.path().join("identity.json");
        let killed_result = files.path().join("killed.json");
        let recovered_result = files.path().join("recovered.json");
        let identity = identity_json(&c.identity);
        std::fs::write(
            &identity_path,
            serde_json::to_vec(&identity).expect("identity JSON"),
        )
        .expect("retain original identity");
        let key = i64::try_from(
            u64::from_be_bytes(
                Uuid::new_v4().as_bytes()[..8]
                    .try_into()
                    .expect("eight bytes"),
            ) >> 1,
        )
        .expect("positive key");
        install_commit_probe(&f, key, resolution).await;
        let mut gate = f.api.db.pool.begin().await.expect("test COMMIT gate");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *gate)
            .await
            .expect("hold gate");
        let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .expect("gate pid");
        let mut proxy = CommitAckLossProxy::start(&f.api.db, resolution).await;
        proxy.arm();
        let pool = proxy.pool.clone();
        let original_command = c.clone();
        let original = tokio::spawn(async move {
            PgLeafCollectionSuccessStore::new(pool, DefinitionCache::new(Arc::new(JsonIrCodec), 8))
                .certify_leaf_collection_success(&original_command)
                .await
        });
        proxy.wait_sent().await;
        let committing_pid = waiting_commit(&f, gate_pid).await;
        proxy.lose_reply().await;
        assert_eq!(
            timeout(Duration::from_secs(5), original)
                .await
                .expect("unknown deadline")
                .expect("original task"),
            Err(LeafCollectionSuccessError::CommitOutcomeUnknown {
                identity: c.identity.clone()
            })
        );
        let mut child = launch(&f, &identity_path, &killed_result);
        let child_pid = child.id().expect("live OS child id");
        let backend_pid = eventually("actual OS child waits on unresolved original Run", || async {
            sqlx::query_scalar::<_,i32>("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND usename='sagat_app' AND application_name=$1 AND position('SELECT workflow_version_id,run_version,trace_id FROM runs' in query)>0 AND wait_event_type='Lock' AND $2=ANY(pg_blocking_pids(pid))")
                .bind(APP).bind(committing_pid).fetch_optional(&f.api.db.pool).await.expect("child original-Run blocker")
        }).await;
        assert!(child.try_wait().expect("child state").is_none());
        own_guards_held(&f).await;
        child
            .start_kill()
            .expect("kill real reconciliation process");
        let killed = timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("reap killed child deadline")
            .expect("reap killed child");
        assert!(!killed.success());
        assert!(
            !killed_result.exists(),
            "blocked killed client produced no result"
        );
        assert_eq!(waiting_commit(&f, gate_pid).await, committing_pid);
        own_guards_held(&f).await;
        assert_eq!(counts(&f).await, (0, 0));
        assert_eq!(refusal_state(&f).await, before);
        // The killed backend may remain in its SELECT lock wait until the original writer ends.
        // Release only the original gate: no backend termination substitutes for process death.
        gate.rollback().await.expect("release original COMMIT gate");
        assert_eq!(proxy.resolution().await, resolution);
        proxy.close().await;
        eventually("killed client backend disappears after original resolution", || async {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND datname=current_database() AND application_name=$2)")
                .bind(backend_pid).bind(APP).fetch_one(&f.api.db.pool).await.expect("old backend state");
            (!exists).then_some(())
        }).await;
        assert_eq!(
            counts(&f).await,
            if resolution == CommitResolution::Committed {
                (1, 1)
            } else {
                (0, 0)
            }
        );
        assert_eq!(refusal_state(&f).await, before);
        let retained_audit: Option<Uuid> = sqlx::query_scalar("SELECT audit_id FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND run_id=$2")
            .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_optional(&f.api.db.pool).await.expect("actual original receipt");
        sqlx::raw_sql("DROP TRIGGER leaf_certification_commit_gate ON leaf_collection_success_certifications; DROP FUNCTION leaf_certification_commit_probe();")
            .execute(&f.api.db.pool).await.expect("remove test fault");
        let lease = c.scheduler.lease();
        f.api
            .executions
            .release(c.scheduler.owner(), &[lease])
            .await
            .expect("release old authority");
        let fresh = f
            .api
            .executions
            .acquire(
                "leaf-process-fresh-owner",
                &[lease.shard],
                Duration::from_secs(60),
            )
            .await
            .expect("actual takeover");
        assert_eq!(fresh.len(), 1);
        assert!(fresh[0].generation > lease.generation);
        let mut replacement = launch(&f, &identity_path, &recovered_result);
        assert_ne!(replacement.id().expect("new OS child"), child_pid);
        let status = timeout(Duration::from_secs(15), replacement.wait())
            .await
            .expect("replacement deadline")
            .expect("reap replacement");
        assert!(status.success(), "replacement native probe failed");
        let recovered = read_json(&recovered_result);
        assert_eq!(recovered["identity"], identity);
        assert_eq!(
            recovered["replayed"],
            resolution == CommitResolution::Committed
        );
        assert_eq!(recovered["discoveredOwner"], "leaf-process-fresh-owner");
        assert_eq!(recovered["discoveredGeneration"], fresh[0].generation);
        assert_eq!(recovered["discoveredRunRevision"], c.expected_run_revision);
        assert_eq!(recovered["certifiedRunRevision"], c.expected_run_revision);
        let audit = uuid_field(&recovered, "auditId");
        if let Some(original) = retained_audit {
            assert_eq!(audit, original);
        }
        let canonical_audit: Uuid = sqlx::query_scalar("SELECT audit_id FROM leaf_collection_success_certifications WHERE tenant_id=$1 AND run_id=$2")
            .bind(f.tenant.as_uuid()).bind(f.run.as_uuid()).fetch_one(&f.api.db.pool).await.expect("recovered canonical receipt");
        assert_eq!(audit, canonical_audit);
        assert_eq!(counts(&f).await, (1, 1));
        assert_eq!(refusal_state(&f).await, before);
        assert!(!killed_result.exists());
        f.verify().await;
    }
}
