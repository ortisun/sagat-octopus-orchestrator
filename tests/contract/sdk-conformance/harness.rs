//! The machinery of the SDK conformance suite (`FR-006-006`): the same scenarios, played by the SDK of
//! each language against a real platform, must give the same normalized results, equal to the golden
//! files. `README.md` in this directory is the contract with the SDKs. The conformance test, the end to
//! end tests and the fault tests of feature 006 all play scenarios through this module.
#![allow(clippy::expect_used, clippy::panic, dead_code)]

#[path = "control.rs"]
mod control;
#[path = "database.rs"]
mod database;
#[path = "normalize.rs"]
mod normalize;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use reqwest::Method;
use sagat_application::blobs::BlobLimits;
use sagat_application::worker::WorkerSettings;
use sagat_domain::identity::Role;
use sagat_infra_blobstore::BlobStoreConfig;
use serde_json::{Map, Value, json};
use tokio::process::Command;

use crate::support::api::Options;
use crate::support::worker::{
    HUMAN, NAMESPACE, Platform, RunningEngine, TENANT, engine_env, platform_with,
};
use normalize::{Normalizer, compared, first_difference};

/// (name used in `SAGAT_SDK_LANGS`, directory under `sdks/`)
const LANGUAGES: [(&str, &str); 6] = [
    ("typescript", "typescript"),
    ("go", "go"),
    ("python", "python"),
    ("dotnet", "dotnet"),
    ("java", "java"),
    ("rust", "rust"),
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Compatibility gates can run immutable released client sources against the current server.
/// Scenarios, normalization and reviewed goldens continue to come from this checkout.
fn sdk_source_root() -> PathBuf {
    std::env::var_os("SAGAT_SDK_SOURCE_ROOT").map_or_else(
        || repo().join("sdks"),
        |root| {
            let root = PathBuf::from(root);
            assert!(root.is_absolute(), "SAGAT_SDK_SOURCE_ROOT must be absolute");
            root
        },
    )
}

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contract")
        .join("sdk-conformance")
}

/// The languages to play: `SAGAT_SDK_LANGS` (a comma separated list) and `only` both restrict them.
fn languages(only: Option<&[&str]>) -> Vec<(&'static str, PathBuf)> {
    let chosen: Option<Vec<String>> = std::env::var("SAGAT_SDK_LANGS").ok().map(|list| {
        list.split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    });
    if let Some(chosen) = &chosen {
        assert!(
            !chosen.is_empty(),
            "SAGAT_SDK_LANGS must name at least one official SDK"
        );
        for name in chosen {
            assert!(
                LANGUAGES.iter().any(|(known, _)| known == name),
                "unknown SDK in SAGAT_SDK_LANGS: {name}"
            );
        }
    }
    LANGUAGES
        .iter()
        .filter(|(name, _)| {
            chosen
                .as_ref()
                .is_none_or(|list| list.iter().any(|l| l == name))
        })
        .filter(|(name, _)| only.is_none_or(|list| list.contains(name)))
        .map(|(name, dir)| (*name, sdk_source_root().join(dir).join("conformance")))
        .collect()
}

async fn script(
    dir: &Path,
    name: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(String, String), String> {
    let path = dir.join(name);
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    let child = Command::new("bash")
        .arg(&path)
        .args(args)
        .current_dir(dir)
        // Not UTC, and not the zone of any CI machine: an SDK that formats or parses an instant in local
        // time shows here.
        .env("TZ", "Asia/Kolkata")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", path.display()))?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("{} {args:?} took longer than {timeout:?}", path.display()))?
        .map_err(|e| format!("{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{} {args:?} exited with {}:\n{}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn process_tree_rss_kib(root: u32) -> u64 {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,rss="])
        .output()
    else {
        return 0;
    };
    let rows: Vec<(u32, u32, u64)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse().ok()?,
                fields.next()?.parse().ok()?,
                fields.next()?.parse().ok()?,
            ))
        })
        .collect();
    let mut tree = std::collections::BTreeSet::from([root]);
    loop {
        let before = tree.len();
        for (pid, parent, _) in &rows {
            if tree.contains(parent) {
                tree.insert(*pid);
            }
        }
        if tree.len() == before {
            break;
        }
    }
    rows.iter()
        .filter(|(pid, _, _)| tree.contains(pid))
        .map(|(_, _, rss)| *rss)
        .sum()
}

/// Runs one benchmark subject and samples the resident set of the runner process tree. The runner
/// scripts `exec` their language runtime, while the descendant walk also covers runtime helpers.
async fn measured_script(
    dir: &Path,
    name: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(String, String, u64), String> {
    let path = dir.join(name);
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    let child = Command::new("bash")
        .arg(&path)
        .args(args)
        .current_dir(dir)
        .env("TZ", "Asia/Kolkata")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", path.display()))?;
    let pid = child
        .id()
        .ok_or_else(|| format!("{} has no process id", path.display()))?;
    let sampling = Arc::new(AtomicBool::new(true));
    let peak = Arc::new(AtomicU64::new(0));
    let sampling_task = {
        let sampling = Arc::clone(&sampling);
        let peak = Arc::clone(&peak);
        tokio::spawn(async move {
            while sampling.load(Ordering::Relaxed) {
                peak.fetch_max(process_tree_rss_kib(pid), Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
    };
    let waited = tokio::time::timeout(timeout, child.wait_with_output()).await;
    sampling.store(false, Ordering::Relaxed);
    let _ = sampling_task.await;
    let output = waited
        .map_err(|_| format!("{} {args:?} took longer than {timeout:?}", path.display()))?
        .map_err(|e| format!("{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{} {args:?} exited with {}:\n{}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        peak.load(Ordering::Relaxed),
    ))
}

fn sorted_for_golden(value: &Value) -> Value {
    serde_json::from_str(&canonical(value)).expect("json")
}

fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|k| (k.clone(), sorted(&map[k])))
                        .collect::<Map<_, _>>(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sorted(value)).expect("json")
}

/// A platform of one scenario and one language: nothing a language did is left for the next to find.
struct Run {
    platform: Arc<Platform>,
    /// The execution engine, started the way the binary is: worker API, scheduler, event router, outbox.
    engine: RunningEngine,
    control: String,
    client_token: String,
    worker_token: String,
    other_token: String,
    /// A worker identity with no binding: the platform refuses to register its activities.
    unbound_token: String,
    /// Members of the tenant with one role each, for the scenarios that test who may do what.
    operator_token: String,
    viewer_token: String,
    runner_token: String,
    /// An operator the scenario revokes with the control endpoint.
    revoked_token: String,
    /// The control endpoint, which holds the platform until it is stopped.
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Both server processes use this directory, so it must outlive them.
    _blob_dir: TempBlobDir,
    database: Option<database::Fixture>,
}

struct TempBlobDir(PathBuf);

impl TempBlobDir {
    fn create() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sagat-sdk-conformance-blobs-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create shared blob directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempBlobDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Run {
    /// Stops the engine (it finishes what it holds) and the control endpoint.
    async fn stop(mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = tokio::time::timeout(Duration::from_secs(25), self.engine.finish()).await;
    }
}

fn token(platform: &Platform, subject: &str, tenant: &str) -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        + 3500;
    platform
        .api
        .idp
        .token(subject, tenant)
        .claim("exp", json!(exp))
        .build()
}

async fn boot(scenario: &Value) -> Run {
    let lease = scenario["server"]["leaseTtlSeconds"].as_u64().unwrap_or(30);
    let blob_dir = TempBlobDir::create();
    let mut database = if scenario["server"]["managedDatabase"] == true {
        Some(database::Fixture::start().await)
    } else {
        None
    };
    let platform = platform_with(Options {
        secret_provider: database.as_ref().map(|db| db.provider()),
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(lease),
            ..WorkerSettings::default()
        },
        blobs: Some(BlobLimits {
            max_bytes: 4 * 1024 * 1024,
            max_pending: 16,
            upload_ttl: Duration::from_secs(24 * 3600),
            abandoned_after: Duration::from_secs(3600),
        }),
        blob_store: Some(BlobStoreConfig::Fs {
            root: blob_dir.path().to_owned(),
        }),
        ..Options::default()
    })
    .await;
    if let Some(limit) = scenario["server"]
        .as_object()
        .and_then(|settings| settings.get("iterationConcurrency"))
    {
        let limit = limit
            .as_u64()
            .expect("server.iterationConcurrency is a non-negative integer");
        let (status, quota) = platform
            .rest(
                Method::PUT,
                "/quotas/iteration_concurrency",
                Some(&json!({"limit": limit})),
            )
            .await;
        assert_eq!(status, 200, "setting iteration concurrency: {quota}");
    }
    platform
        .grant(platform.tenant, "conformance-worker", Role::Worker)
        .await;
    platform.bind_everything("conformance-worker").await;
    platform
        .grant(platform.tenant, "conformance-unbound", Role::Worker)
        .await;
    for (subject, role) in [
        ("conformance-operator", Role::Operator),
        ("conformance-viewer", Role::Viewer),
        ("conformance-runner", Role::Runner),
        ("conformance-revoked", Role::Operator),
    ] {
        platform.grant(platform.tenant, subject, role).await;
    }
    // The engine the worker protocol is served by: timers, retries, leases that ran out and the wake-ups
    // (also the events that release a wait) are its loops, as in production.
    let ring = format!("k1:{}", platform.api.key_k1);
    let lease_secs = lease.to_string();
    let blob_root = blob_dir.path().to_string_lossy().into_owned();
    let mut env = engine_env(
        &platform.api.db,
        &platform.api.idp,
        &[
            ("SAGAT_SECRETS_KEYS", ring.as_str()),
            ("SAGAT_LEASE_TTL_SECS", lease_secs.as_str()),
            ("SAGAT_EVENT_POLL_MS", "50"),
            ("SAGAT_DELIVERY_POLL_MS", "50"),
            ("SAGAT_BLOB_PROVIDER", "fs"),
            ("SAGAT_BLOB_FS_ROOT", blob_root.as_str()),
        ],
    );
    if let Some(db) = &database {
        env.extend(db.engine_env());
    }
    let engine = RunningEngine::start(env).await;
    if let Some(db) = &mut database {
        db.configure(&platform, &engine.grpc_url).await;
    }
    let platform = Arc::new(platform);
    let (control, serving) = control::serve(Arc::clone(&platform)).await;
    Run {
        client_token: token(&platform, HUMAN, TENANT),
        worker_token: token(&platform, "conformance-worker", TENANT),
        other_token: token(&platform, "bob", "globex"),
        unbound_token: token(&platform, "conformance-unbound", TENANT),
        operator_token: token(&platform, "conformance-operator", TENANT),
        viewer_token: token(&platform, "conformance-viewer", TENANT),
        runner_token: token(&platform, "conformance-runner", TENANT),
        revoked_token: token(&platform, "conformance-revoked", TENANT),
        control: format!("http://{control}"),
        platform,
        engine,
        tasks: vec![serving],
        _blob_dir: blob_dir,
        database,
    }
}

async fn publish(run: &Run, name: &str, ir: &Value) -> Value {
    let (status, created) = run
        .platform
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": name, "ir": ir})),
        )
        .await;
    assert_eq!(status, 201, "creating {name}: {created}");
    let id = created["id"].as_str().expect("id").to_owned();
    let (status, published) = run
        .platform
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 201, "publishing {name}: {published}");
    let version = published["versionId"]
        .as_str()
        .or_else(|| published["id"].as_str())
        .unwrap_or_default()
        .to_owned();
    json!({"workflowId": id, "versionId": version, "name": name})
}

/// Publication is a shared server HTTP phase: SDKs expose builders, not a publisher.
async fn rejected_publication(run: &Run, key: &str, ir: &Value, reason: &str) -> Value {
    let name = ir["metadata"]["name"].as_str().expect("workflow name");
    let (status, created) = run
        .platform
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": name, "ir": ir})),
        )
        .await;
    assert_eq!(
        status, 201,
        "creating rejected-publication draft {key}: {created}"
    );
    let id = created["id"].as_str().expect("draft id");
    let (status, refused) = run
        .platform
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 422, "expected publication refusal {key}: {refused}");
    assert_eq!(refused["code"], "VALIDATION_FAILED", "{key}: {refused}");
    assert_eq!(refused["retryable"], false, "{key}: {refused}");
    assert_eq!(refused["details"]["reason"], reason, "{key}: {refused}");
    let trace = refused["traceId"]
        .as_str()
        .expect("publication refusal trace");
    assert_eq!(trace.len(), 32);
    assert!(trace.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let (active, versions): (Option<uuid::Uuid>, i64) = sqlx::query_as(
        "SELECT active_version_id, (SELECT count(*) FROM workflow_versions \
         WHERE tenant_id=$1 AND workflow_id=$2) FROM workflows WHERE tenant_id=$1 AND id=$2",
    )
    .bind(run.platform.tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(id).expect("draft UUID"))
    .fetch_one(&run.platform.api.db.pool)
    .await
    .expect("independent rejected publication projection");
    assert_eq!(
        (active, versions),
        (None, 0),
        "{key}: rejected draft published"
    );
    json!({
        "workflow": key, "status": status, "code": refused["code"],
        "reason": refused["details"]["reason"], "retryable": refused["retryable"],
        "activeVersion": active.map(|id| id.to_string()), "publishedVersions": versions,
    })
}

/// What the platform holds of every run once a language has played, read over REST without the SDK: the
/// effect of what the SDK did (the status of each node, the attempts, the code a failure was recorded with).
async fn server_state(run: &Run) -> Result<Value, String> {
    let (status, page) = run
        .platform
        .rest(Method::GET, "/runs?limit=200", None)
        .await;
    if status != 200 {
        return Err(format!("listing the runs answered {status}: {page}"));
    }
    let mut runs = Vec::new();
    // Newest first by creation, as the list gives them, but not by id alone: the first run of a child
    // chain has an id derived from its parent's, which sorts anywhere among time-ordered ids.
    let mut items = page["items"].as_array().cloned().unwrap_or_default();
    items.sort_by(|a, b| {
        b["createdAt"]
            .as_str()
            .cmp(&a["createdAt"].as_str())
            .then_with(|| b["runId"].as_str().cmp(&a["runId"].as_str()))
    });
    for item in items {
        let id = item["runId"]
            .as_str()
            .ok_or("a run without an id")?
            .to_owned();
        // The graph is a projection of the journal: it is read once it has caught up with the run.
        let mut graph = Value::Null;
        for _ in 0..100 {
            let (status, found) = run
                .platform
                .rest(Method::GET, &format!("/runs/{id}/graph"), None)
                .await;
            if status == 200 && found["status"] == item["status"] {
                graph = found;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if graph.is_null() {
            return Err(format!(
                "the graph of run {id} did not reach the status {}",
                item["status"]
            ));
        }
        let nodes: Vec<Value> = graph["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|node| {
                json!({
                    "id": node["id"], "type": node["type"], "kind": node["kind"], "forward": node["forward"],
                    "status": node["status"], "attempt": node["attempt"], "failureCode": node["failureCode"],
                    "waitingOn": node["waitingOn"], "cancelRequested": node["cancelRequested"],
                })
            })
            .collect();
        runs.push(json!({"runId": id, "status": item["status"], "paused": item["paused"], "nodes": nodes}));
    }
    Ok(Value::Array(runs))
}

/// The collection and loop events of every run, read from the journal and the payload store rather than
/// from what an SDK reports: the order in which iterations were admitted and checkpointed, the most that
/// were active at once, the carry of every loop checkpoint and the aggregate a collection finished with.
/// The compensation events of a scope (`scope.compensation_planned`, `scope.compensated`) are part of it.
/// A scenario asks for it with `"scopeEvidence": "ordered"` (every event, in journal order: for scopes that
/// run one iteration at a time) or `"summary"` (admission order, the sorted checkpoints and the entry and
/// finish: for scopes whose iterations complete concurrently, where the scheduler's order of looking at
/// completions that arrive together is not part of the contract).
async fn scope_evidence(run: &Run, state: &Value, summary: bool) -> Result<Value, String> {
    let pool = &run.platform.api.db.pool;
    let mut runs = Vec::new();
    for item in state.as_array().into_iter().flatten() {
        let id = item["runId"].as_str().ok_or("a run without an id")?;
        let run_id = uuid::Uuid::parse_str(id).map_err(|e| format!("run id {id}: {e}"))?;
        let rows: Vec<(String, Value)> = sqlx::query_as(
            "SELECT event_type, detail FROM run_events WHERE run_id = $1 \
             AND (event_type LIKE 'collection.%' OR event_type LIKE 'loop.%' \
             OR event_type LIKE 'scope.%') ORDER BY seq",
        )
        .bind(run_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("reading the scope events of run {id}: {e}"))?;
        let (mut active, mut peak) = (0_i64, 0_i64);
        let mut events = Vec::new();
        let (mut admitted, mut checkpointed) = (Vec::new(), Vec::new());
        for (kind, detail) in rows {
            if kind.ends_with(".iteration_admitted") {
                active += 1;
                peak = peak.max(active);
            }
            if kind.ends_with(".iteration_checkpointed") || kind.ends_with(".iteration_drained") {
                active -= 1;
            }
            if summary && kind.ends_with(".iteration_admitted") {
                admitted.push(detail["index"].clone());
                continue;
            }
            if summary && kind.ends_with(".iteration_checkpointed") {
                checkpointed.push(detail["index"].as_u64().unwrap_or_default());
                continue;
            }
            if summary
                && (kind.ends_with(".iteration_returned") || kind.ends_with(".iteration_drained"))
            {
                continue;
            }
            let mut entry = Map::new();
            entry.insert("event".into(), json!(kind));
            for key in [
                "index",
                "kind",
                "outcome",
                "code",
                "condition",
                "next_iteration",
                "item_count",
                "max_items",
                "max_concurrency",
                "max_iterations",
                "initial_condition",
            ] {
                if let Some(value) = detail.get(key).filter(|v| !v.is_null()) {
                    entry.insert(key.into(), value.clone());
                }
            }
            if kind.starts_with("scope.") {
                // The compensation of a scope: why it was planned, which instances it undoes and after
                // which others, which effects cannot be undone, and how it ended.
                for key in ["cause", "entries", "irreversible"] {
                    if let Some(value) = detail.get(key).filter(|v| !v.is_null()) {
                        entry.insert(key.into(), value.clone());
                    }
                }
            }
            if let Some(kind) = detail["continuation"]["kind"].as_str() {
                entry.insert("continuation".into(), json!(kind));
            }
            for (field, label) in [
                ("item_digest", "item"),
                ("result_digest", "result"),
                ("carry_digest", "carry"),
                ("initial_carry_digest", "initialCarry"),
            ] {
                if let Some(digest) = detail[field].as_str() {
                    entry.insert(label.into(), payload_json(pool, digest).await?);
                }
            }
            events.push(Value::Object(entry));
        }
        if summary {
            checkpointed.sort_unstable();
            runs.push(json!({
                "runId": id, "peakActive": peak, "admitted": admitted,
                "checkpointed": checkpointed, "events": events,
            }));
        } else {
            runs.push(json!({"runId": id, "peakActive": peak, "events": events}));
        }
    }
    Ok(Value::Array(runs))
}

/// How each run of the scenario ended and where its collection took its items from, read from the
/// platform's own rows: the run's status and stop code, and, when a collection was entered, its item
/// count, whether the membership came from a blob, and how many times the engine's read of that blob
/// was audited. A scenario asks for it with `"sourceEvidence": true`.
async fn source_evidence(run: &Run, state: &Value) -> Result<Value, String> {
    let pool = &run.platform.api.db.pool;
    let mut runs = Vec::new();
    for item in state.as_array().into_iter().flatten() {
        let id = item["runId"].as_str().ok_or("a run without an id")?;
        let run_id = uuid::Uuid::parse_str(id).map_err(|e| format!("run id {id}: {e}"))?;
        let (status, stop_code): (String, Option<String>) =
            sqlx::query_as("SELECT status, stop_code FROM runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(pool)
                .await
                .map_err(|e| format!("reading run {id}: {e}"))?;
        let scope: Option<(i32, Option<uuid::Uuid>)> = sqlx::query_as(
            "SELECT item_count, source_blob_id FROM iteration_scopes WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("reading the scope of run {id}: {e}"))?;
        let scope = match scope {
            None => Value::Null,
            Some((item_count, blob)) => {
                let reads: i64 = match blob {
                    Some(blob) => sqlx::query_scalar(
                        "SELECT count(*) FROM audit_records WHERE action = 'blob.read' \
                           AND resource_id = $1",
                    )
                    .bind(blob.to_string())
                    .fetch_one(pool)
                    .await
                    .map_err(|e| format!("reading the blob audit of run {id}: {e}"))?,
                    None => 0,
                };
                json!({"itemCount": item_count, "fromBlob": blob.is_some(), "blobReads": reads})
            }
        };
        runs.push(json!({
            "runId": id, "status": status, "stopCode": stop_code, "collection": scope,
        }));
    }
    Ok(Value::Array(runs))
}

/// The approval audit rows of the tenant, in the order they were written, from the audit trail itself:
/// who was refused or allowed, with the trace of the call that caused it. A scenario asks for it with
/// `"auditEvidence": true`.
async fn audit_evidence(run: &Run) -> Result<Value, String> {
    /// Action, actor, resource type, reason and trace of one audit row.
    type AuditRow = (String, String, String, Option<String>, Option<String>);
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT action, actor, resource_type, reason, trace_id FROM audit_records \
         WHERE tenant_id = $1 AND action LIKE 'approval.%' ORDER BY created_at, id",
    )
    .bind(run.platform.tenant.as_uuid())
    .fetch_all(&run.platform.api.db.pool)
    .await
    .map_err(|e| format!("reading the approval audit trail: {e}"))?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|(action, actor, resource_type, reason, trace)| {
                json!({
                    "action": action, "actor": actor, "resourceType": resource_type,
                    "reason": reason, "traceId": trace,
                })
            })
            .collect(),
    ))
}

/// The JSON payload stored under a hex SHA-256 digest.
async fn payload_json(pool: &sqlx::PgPool, digest: &str) -> Result<Value, String> {
    let content: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE digest = decode($1, 'hex') LIMIT 1")
            .bind(digest)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("reading payload {digest}: {e}"))?;
    let content = content.ok_or_else(|| format!("payload {digest} is not stored"))?;
    serde_json::from_slice(&content).map_err(|e| format!("payload {digest} is not JSON: {e}"))
}

/// Durable evidence for the blob scenario. This is read independently of the SDK result so blessing
/// cannot turn a failed transfer, a wrong digest or an accidentally auto-promoted JSON object into
/// the expected contract.
async fn blob_evidence(run: &Run) -> Result<Value, String> {
    let stored: Vec<(String, i64, String, String)> = sqlx::query_as(
        "SELECT state, size_bytes, encode(sha256, 'hex'), content_type \
         FROM blobs ORDER BY encode(sha256, 'hex')",
    )
    .fetch_all(&run.platform.api.db.pool)
    .await
    .map_err(|e| format!("reading blob evidence: {e}"))?;
    let stored: Vec<Value> = stored
        .into_iter()
        .map(|(state, size, sha256, content_type)| {
            json!({"state": state, "size": size, "sha256": sha256, "contentType": content_type})
        })
        .collect();
    let inline: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM steps s \
         JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest \
         WHERE s.node='inline'",
    )
    .fetch_one(&run.platform.api.db.pool)
    .await
    .map_err(|e| format!("reading inline blob-shaped output: {e}"))?;
    let inline: Value = serde_json::from_slice(&inline)
        .map_err(|e| format!("inline blob-shaped output is not JSON: {e}"))?;
    Ok(json!({"stored": stored, "inlineOutput": inline}))
}

/// Independent durable evidence that the HTTP upload was claimed by the run and that the worker
/// verified the whole lease-bound stream before completing the activity.
async fn run_input_blob_evidence(run: &Run) -> Result<Value, String> {
    let (state, size, sha256, content_type, claimed): (String, i64, String, String, bool) =
        sqlx::query_as(
            "SELECT state, size_bytes, encode(sha256, 'hex'), content_type, run_id IS NOT NULL \
             FROM blobs WHERE purpose='RUN_INPUT'",
        )
        .fetch_one(&run.platform.api.db.pool)
        .await
        .map_err(|e| format!("reading run-input blob evidence: {e}"))?;
    let output: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM steps s \
         JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest \
         WHERE s.node='input'",
    )
    .fetch_one(&run.platform.api.db.pool)
    .await
    .map_err(|e| format!("reading run-input activity output: {e}"))?;
    let output: Value = serde_json::from_slice(&output)
        .map_err(|e| format!("run-input activity output is not JSON: {e}"))?;
    Ok(json!({
        "stored": {"state": state, "size": size, "sha256": sha256,
                   "contentType": content_type, "claimed": claimed},
        "verifiedOutput": output,
    }))
}

fn validate_blob_result(doc: &Value) -> Result<(), String> {
    for id in ["round-trip-done", "output-done", "inline-done"] {
        let status = doc["observations"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == id))
            .and_then(|item| item["result"]["status"].as_str());
        if status != Some("SUCCEEDED") {
            return Err(format!("{id} must finish SUCCEEDED, got {status:?}"));
        }
    }
    let expected = [
        (
            "UPLOADED",
            "29927e273accc68286005017f7fa6e4f27bddb4db3083ff8b8d4c3667905b7fa",
        ),
        (
            "UPLOADED",
            "29aa987eab9aee830fdd20595bcedef67e8674f4eeac6f017598e7ff8719da23",
        ),
        (
            "COMMITTED",
            "d9c30921ab850a0e81b8a21eabf78cb38e4ff43fdf46d2e023275a064ed1a7f9",
        ),
    ];
    let rows = doc["blobEvidence"]["stored"]
        .as_array()
        .ok_or("blob evidence has no stored rows")?;
    if rows.len() != expected.len() {
        return Err(format!("expected three stored blobs, got {}", rows.len()));
    }
    for (row, (state, sha256)) in rows.iter().zip(expected) {
        if row["state"] != state
            || row["size"] != 300_000
            || row["sha256"] != sha256
            || row["contentType"] != "application/octet-stream"
        {
            return Err(format!("unexpected stored blob evidence: {row}"));
        }
    }
    let inline = &doc["blobEvidence"]["inlineOutput"]["$blob"];
    if !inline["id"].is_string()
        || inline["size"] != 300_000
        || inline["sha256"] != expected[1].1
        || inline["contentType"] != "application/octet-stream"
    {
        return Err(format!(
            "ordinary blob-shaped JSON was not preserved inline: {inline}"
        ));
    }
    Ok(())
}

fn validate_run_input_blob_result(doc: &Value) -> Result<(), String> {
    let status = doc["observations"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "input-done"))
        .and_then(|item| item["result"]["status"].as_str());
    if status != Some("SUCCEEDED") {
        return Err(format!("input-done must finish SUCCEEDED, got {status:?}"));
    }
    let expected_sha = "e327a74e9a14ae13e010f3614e258c50b6150b6845e691691f54768f8bb3b5ee";
    let stored = &doc["runInputBlobEvidence"]["stored"];
    if stored["state"] != "COMMITTED"
        || stored["size"] != 300_000
        || stored["sha256"] != expected_sha
        || stored["contentType"] != "application/octet-stream"
        || stored["claimed"] != true
    {
        return Err(format!("unexpected run-input blob evidence: {stored}"));
    }
    let output = &doc["runInputBlobEvidence"]["verifiedOutput"];
    if output["inputBlobSize"] != 300_000 || output["inputBlobSha256"] != expected_sha {
        return Err(format!(
            "the worker did not verify the whole run input: {output}"
        ));
    }
    Ok(())
}

/// The places in a runner's observations where an error repeats something it must not: a token of the run
/// or a text the scenario marked `mustNotLeak` (a payload it sent, say). The message is not compared with
/// the other SDKs', so this is where its content is held to the contract.
fn leaks(out: &Value, config: &Value, scenario: &Value) -> Vec<String> {
    // The tokens of the run appear nowhere in what a runner wrote (errors, observations, the worker log, ...).
    let tokens: Vec<String> = [
        "clientToken",
        "workerToken",
        "otherTenantToken",
        "unboundWorkerToken",
        "badToken",
        "operatorToken",
        "viewerToken",
        "runnerToken",
        "revokedToken",
    ]
    .iter()
    .filter_map(|key| config[*key].as_str().map(str::to_owned))
    .filter(|token| !token.is_empty())
    .collect();
    // A text the scenario marked (a payload it sent) must not come back in an error: it is legitimately in
    // the worker's log, which handled it.
    let markers: Vec<String> = scenario["mustNotLeak"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .filter(|marker| !marker.is_empty())
        .collect();

    fn walk(value: &Value, path: &str, found: &mut Vec<(String, String)>) {
        match value {
            Value::String(text) => found.push((path.to_owned(), text.clone())),
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, v)| walk(v, &format!("{path}[{i}]"), found)),
            Value::Object(map) => map
                .iter()
                .for_each(|(k, v)| walk(v, &format!("{path}.{k}"), found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    let mut everything = Vec::new();
    walk(out, "$", &mut everything);
    for (path, text) in &everything {
        for token in &tokens {
            if text.contains(token.as_str()) {
                found.push(format!(
                    "{path}: a token of the run is in the output (\"{}…\")",
                    token.chars().take(8).collect::<String>()
                ));
            }
        }
    }
    for observation in out["observations"].as_array().into_iter().flatten() {
        let Some(error) = observation["error"].as_object() else {
            continue;
        };
        for (field, value) in error {
            let Some(text) = value.as_str() else { continue };
            for marker in &markers {
                if text.contains(marker.as_str()) {
                    found.push(format!(
                        "step {}: the error's {field} repeats \"{}…\"",
                        observation["id"],
                        marker.chars().take(12).collect::<String>()
                    ));
                }
            }
        }
    }
    found
}

/// Where a text the scenario listed under `mustNotAppear` (a secret-shaped value it put into a run's input)
/// shows up in what the run reports: the observations, the server state and the evidence read from the
/// journal and the audit trail. The worker's log is not searched: a handler is meant to see its input.
fn marker_appearances(out: &Value, scenario: &Value) -> Vec<String> {
    fn walk(value: &Value, path: &str, marker: &str, found: &mut Vec<String>) {
        match value {
            Value::String(text) if text.contains(marker) => found.push(path.to_owned()),
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, v)| walk(v, &format!("{path}[{i}]"), marker, found)),
            Value::Object(map) => map
                .iter()
                .for_each(|(k, v)| walk(v, &format!("{path}.{k}"), marker, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for marker in scenario["mustNotAppear"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|marker| !marker.is_empty())
    {
        for key in [
            "observations",
            "serverState",
            "auditEvidence",
            "scopeEvidence",
        ] {
            let mut at = Vec::new();
            walk(&out[key], key, marker, &mut at);
            for path in at {
                found.push(format!(
                    "{path} repeats \"{}…\", which the scenario marked as never to appear",
                    marker.chars().take(8).collect::<String>()
                ));
            }
        }
    }
    found
}

/// The instants of a Run an SDK hands back must be UTC with a `Z`, whatever the zone it runs in.
fn instants_not_utc(out: &Value) -> Vec<String> {
    fn walk(id: &Value, value: &Value, found: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    if matches!(
                        key.as_str(),
                        "createdAt" | "updatedAt" | "finishedAt" | "deadline"
                    ) && let Some(text) = item.as_str()
                        && !text.ends_with('Z')
                    {
                        found.push(format!("step {id}: {key} is \"{text}\", not UTC with a Z"));
                    } else {
                        walk(id, item, found);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(id, item, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for observation in out["observations"].as_array().into_iter().flatten() {
        walk(&observation["id"], &observation["result"], &mut found);
    }
    found
}

/// The traces a scenario puts in its steps, which the SDKs must carry as they are.
fn given_traces(scenario: &Value) -> Vec<String> {
    let mut found = Vec::new();
    fn walk(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(text) => {
                if let Some(rest) = text.strip_prefix("00-")
                    && let Some(trace) = rest.get(..32)
                {
                    found.push(trace.to_owned());
                }
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, found)),
            Value::Object(map) => map.values().for_each(|v| walk(v, found)),
            _ => {}
        }
    }
    walk(&scenario["steps"], &mut found);
    found
}

/// Writes a file that only its owner may read: a config holds the tokens of the run.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)?.write_all(bytes)
}

/// A platform of its own for one scenario and one language, the workflows the scenario's builders wrote
/// published in it, and the `config.json` the runner is given.
async fn stage(scenario: &Value, built: &Value, lang: &str) -> (Run, Value, String, Vec<Value>) {
    let rejections = scenario.get("publicationRejections").map(|value| {
        let entries = value
            .as_object()
            .expect("publicationRejections must be an object");
        for (key, reason) in entries {
            assert!(
                built["workflows"].get(key).is_some(),
                "unknown rejected workflow {key}"
            );
            assert!(
                reason.as_str().is_some_and(|reason| !reason.is_empty()),
                "invalid publication reason for {key}"
            );
        }
        entries
    });
    let mut publication_evidence = Vec::new();
    let run = boot(scenario).await;
    // The secrets a scenario's workflows declare must exist before they can be published.
    if let Some(secrets) = scenario["secrets"].as_object() {
        let namespace = run.platform.namespace_id().await;
        for (secret, value) in secrets {
            let (status, stored) = run
                .platform
                .rest(
                    Method::PUT,
                    &format!("/namespaces/{namespace}/secrets/{secret}"),
                    Some(&json!({"value": value})),
                )
                .await;
            assert_eq!(status, 200, "storing the secret {secret}: {stored}");
        }
    }
    let mut workflows = Map::new();
    if let Some(docs) = built["workflows"].as_object() {
        for (key, ir) in docs {
            let workflow_name = ir["metadata"]["name"]
                .as_str()
                .expect("workflow name")
                .to_owned();
            if let Some(reason) = rejections.and_then(|entries| entries.get(key)) {
                publication_evidence.push(
                    rejected_publication(&run, key, ir, reason.as_str().expect("reason")).await,
                );
            } else {
                workflows.insert(key.clone(), publish(&run, &workflow_name, ir).await);
            }
        }
    }
    let unique = format!(
        "{lang}-{}",
        &uuid::Uuid::now_v7().simple().to_string()[20..]
    );
    let namespace_id = run.platform.namespace_id().await;
    let config = json!({
        "apiUrl": run.platform.api.base, "grpcUrl": run.engine.grpc_url, "namespace": NAMESPACE,
        "namespaceId": namespace_id,
        "clientToken": run.client_token, "workerToken": run.worker_token, "otherTenantToken": run.other_token,
        "unboundWorkerToken": run.unbound_token, "badToken": "not-a-jwt",
        "operatorToken": run.operator_token, "viewerToken": run.viewer_token,
        "runnerToken": run.runner_token, "revokedToken": run.revoked_token, "controlUrl": run.control, "unique": unique, "workflows": workflows,
        "leaseTtlSeconds": scenario["server"]["leaseTtlSeconds"].as_u64().unwrap_or(30),
    });
    (run, config, unique, publication_evidence)
}

/// One language's normalized result of one scenario.
pub struct Played {
    pub scenario: String,
    pub language: &'static str,
    pub doc: Value,
}

struct Outcome {
    failures: Vec<String>,
    played: Vec<Played>,
}

async fn play(
    path: &Path,
    langs: &[(&'static str, PathBuf)],
    bless: bool,
    out_dir: &Path,
) -> Outcome {
    let scenario: Value = serde_json::from_slice(&std::fs::read(path).expect("read the scenario"))
        .expect("scenario JSON");
    let name = scenario["name"].as_str().expect("name").to_owned();
    let mut failures = Vec::new();
    let timeout = Duration::from_secs(300);
    let dir = out_dir.join(&name);
    std::fs::create_dir_all(&dir).expect("output directory");

    // 1. The builders: every SDK writes the same documents.
    let mut built: Vec<(&str, Value)> = Vec::new();
    for (lang, root) in langs {
        match script(
            root,
            "run.sh",
            &["build", path.to_str().expect("path")],
            timeout,
        )
        .await
        {
            Ok((text, _)) => match serde_json::from_str::<Value>(&text) {
                Ok(doc) => built.push((lang, doc)),
                Err(e) => failures.push(format!(
                    "{name}/{lang}: build printed something that is not JSON ({e}): {text}"
                )),
            },
            Err(e) => failures.push(format!("{name}/{lang}: build failed: {e}")),
        }
    }
    // The canonical text of every workflow is part of the build output and is compared byte for byte (SPEC §7).
    for (lang, doc) in &built {
        let has_workflows = doc["workflows"].as_object().is_some_and(|w| !w.is_empty());
        if has_workflows
            && doc["canonical"]
                .as_object()
                .is_none_or(|c| c.len() != doc["workflows"].as_object().map_or(0, Map::len))
        {
            failures.push(format!(
                "{name}/{lang}: build printed no canonical text (toJson()) for every workflow"
            ));
        }
    }
    if !failures.is_empty() {
        return Outcome {
            failures,
            played: Vec::new(),
        };
    }
    let Some((first_lang, first)) = built.first().cloned() else {
        return Outcome {
            failures,
            played: Vec::new(),
        };
    };
    for (lang, doc) in &built[1..] {
        if canonical(doc) != canonical(&first) {
            let at = first_difference(&first, doc).unwrap_or_default();
            failures.push(format!(
                "{name}: the builders of {first_lang} and {lang} disagree at {at}"
            ));
        }
    }
    if !failures.is_empty() {
        return Outcome {
            failures,
            played: Vec::new(),
        };
    }

    // What they built is also the golden document: five SDKs do not drift the same way unnoticed.
    if first
        .get("workflows")
        .is_some_and(|w| w.as_object().is_some_and(|o| !o.is_empty()))
    {
        let golden = suite()
            .join("expected")
            .join(format!("{name}.workflows.json"));
        if bless {
            std::fs::write(
                &golden,
                serde_json::to_vec_pretty(&sorted_for_golden(&first)).expect("json"),
            )
            .expect("bless");
        } else {
            match std::fs::read(&golden)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                Some(expected) if expected == sorted_for_golden(&first) => {}
                Some(expected) => {
                    let at =
                        first_difference(&expected, &sorted_for_golden(&first)).unwrap_or_default();
                    failures.push(format!(
                        "{name}: the documents the builders write differ from {} at {at}",
                        golden.display()
                    ));
                }
                None => failures.push(format!(
                    "{name}: {} does not exist (SAGAT_SDK_BLESS=1 writes it)",
                    golden.display()
                )),
            }
        }
    }

    // Builder-only scenarios pin additive authoring contracts before the server capability is enabled.
    // They are never published or executed, so passing one does not claim runtime support.
    if scenario["buildOnly"].as_bool() == Some(true) {
        return Outcome {
            failures,
            played: Vec::new(),
        };
    }

    // 2. Play it, language by language, each on a platform of its own: the runs, the leases, the events
    //    and the quotas of one language are not there for the next to meet.
    let expected_path = suite().join("expected").join(format!("{name}.json"));
    let mut results: Vec<(&str, Value)> = Vec::new();
    for (lang, root) in langs {
        let (run, config, unique, publication_evidence) = stage(&scenario, &first, lang).await;
        // The process id is in the names: two plays of the suite at once (two people, a CI job and a
        // developer) do not overwrite each other's configuration and output.
        let pid = std::process::id();
        let config_path = dir.join(format!("{lang}.{pid}.config.json"));
        let out_path = dir.join(format!("{lang}.{pid}.out.json"));
        write_private(
            &config_path,
            &serde_json::to_vec_pretty(&config).expect("json"),
        )
        .expect("write config");
        let _ = std::fs::remove_file(&out_path);
        match script(
            root,
            "run.sh",
            &[
                "run",
                path.to_str().expect("path"),
                config_path.to_str().expect("path"),
                out_path.to_str().expect("path"),
            ],
            timeout,
        )
        .await
        {
            Ok((_, stderr)) => match std::fs::read(&out_path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                Some(mut out) => {
                    if !publication_evidence.is_empty() {
                        out["publicationEvidence"] = json!(publication_evidence);
                    }
                    for leak in leaks(&out, &config, &scenario)
                        .into_iter()
                        .chain(instants_not_utc(&out))
                    {
                        failures.push(format!("{name}/{lang}: {leak}"));
                    }
                    match server_state(&run).await {
                        Ok(state) => out["serverState"] = state,
                        Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                    }
                    if scenario["auditEvidence"].as_bool() == Some(true) {
                        match audit_evidence(&run).await {
                            Ok(evidence) => out["auditEvidence"] = evidence,
                            Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                        }
                    }
                    if let Some(mode) = scenario["scopeEvidence"].as_str() {
                        match scope_evidence(&run, &out["serverState"], mode == "summary").await {
                            Ok(evidence) => out["scopeEvidence"] = evidence,
                            Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                        }
                    }
                    if scenario["sourceEvidence"].as_bool() == Some(true) {
                        match source_evidence(&run, &out["serverState"]).await {
                            Ok(evidence) => out["sourceEvidence"] = evidence,
                            Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                        }
                    }
                    if let Some(db) = &run.database {
                        match db.evidence(&run.platform, &out).await {
                            Ok(evidence) => out["databaseEvidence"] = evidence,
                            Err(error) => failures.push(format!("{name}/{lang}: {error}")),
                        }
                    }
                    if name == "blobs" {
                        match blob_evidence(&run).await {
                            Ok(evidence) => out["blobEvidence"] = evidence,
                            Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                        }
                    }
                    if name == "run-input-blobs" {
                        match run_input_blob_evidence(&run).await {
                            Ok(evidence) => out["runInputBlobEvidence"] = evidence,
                            Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                        }
                    }
                    for appearance in marker_appearances(&out, &scenario) {
                        failures.push(format!("{name}/{lang}: {appearance}"));
                    }
                    let mut normalizer = Normalizer::new(&unique, given_traces(&scenario));
                    let doc = compared(&out, &mut normalizer);
                    if name == "blobs"
                        && let Err(e) = validate_blob_result(&doc)
                    {
                        failures.push(format!("{name}/{lang}: {e}"));
                    }
                    if name == "run-input-blobs"
                        && let Err(e) = validate_run_input_blob_result(&doc)
                    {
                        failures.push(format!("{name}/{lang}: {e}"));
                    }
                    std::fs::write(
                        dir.join(format!("{lang}.{pid}.normalized.json")),
                        serde_json::to_vec_pretty(&doc).expect("json"),
                    )
                    .expect("write");
                    results.push((lang, doc));
                }
                None => failures.push(format!(
                    "{name}/{lang}: the runner wrote no readable {} (stderr: {})",
                    out_path.display(),
                    stderr.trim()
                )),
            },
            Err(e) => failures.push(format!("{name}/{lang}: run failed: {e}")),
        }
        run.stop().await;
    }

    // 3. Compare: with each other, and with the golden file.
    if let Some((first_lang, first)) = results.first().cloned() {
        for (lang, doc) in &results[1..] {
            if doc != &first {
                let at = first_difference(&first, doc).unwrap_or_default();
                failures.push(format!(
                    "{name}: {first_lang} and {lang} differ at {at} (see {})",
                    dir.display()
                ));
            }
        }
        if bless && failures.is_empty() {
            std::fs::write(
                &expected_path,
                serde_json::to_vec_pretty(&first).expect("json"),
            )
            .expect("bless");
            println!("blessed {} from {first_lang}", expected_path.display());
        } else if bless {
            failures.push(format!(
                "{name}: refusing to bless a scenario with failed contract assertions"
            ));
        } else {
            match std::fs::read(&expected_path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                Some(expected) => {
                    for (lang, doc) in &results {
                        if doc != &expected {
                            let at = first_difference(&expected, doc).unwrap_or_default();
                            failures.push(format!(
                                "{name}/{lang}: differs from expected/{name}.json at {at}"
                            ));
                        }
                    }
                }
                None => failures.push(format!(
                    "{name}: {} does not exist (SAGAT_SDK_BLESS=1 writes it)",
                    expected_path.display()
                )),
            }
        }
    }
    let played = results
        .into_iter()
        .map(|(language, doc)| Played {
            scenario: name.clone(),
            language,
            doc,
        })
        .collect();
    Outcome { failures, played }
}

/// What to play: all scenarios and all languages unless narrowed (the environment narrows further).
pub struct Selection {
    scenarios: Option<Vec<String>>,
    languages: Option<Vec<String>>,
    bless: bool,
}

impl Selection {
    /// Everything the environment asks for: `SAGAT_SDK_SCENARIO`, `SAGAT_SDK_LANGS`, `SAGAT_SDK_BLESS`.
    pub fn from_env() -> Self {
        Self {
            scenarios: std::env::var("SAGAT_SDK_SCENARIO").ok().map(|w| vec![w]),
            languages: None,
            bless: std::env::var("SAGAT_SDK_BLESS").is_ok(),
        }
    }

    /// These scenarios, in every language the environment allows. Never blesses.
    pub fn scenarios(names: &[&str]) -> Self {
        Self {
            scenarios: Some(names.iter().map(|n| (*n).to_owned()).collect()),
            languages: None,
            bless: false,
        }
    }

    /// Only these languages (and only those the environment allows).
    pub fn only_languages(mut self, names: &[&str]) -> Self {
        self.languages = Some(names.iter().map(|n| (*n).to_owned()).collect());
        self
    }
}

/// What a play of the suite came to.
pub struct Suite {
    pub failures: Vec<String>,
    pub played: Vec<Played>,
}

impl Suite {
    pub fn assert_conforms(&self) {
        assert!(
            self.failures.is_empty(),
            "the SDKs do not conform:\n{}",
            self.failures.join("\n")
        );
    }

    /// The normalized result of one scenario in one language.
    pub fn doc(&self, scenario: &str, language: &str) -> &Value {
        &self
            .played
            .iter()
            .find(|p| p.scenario == scenario && p.language == language)
            .unwrap_or_else(|| panic!("{scenario} was not played in {language}"))
            .doc
    }

    /// The languages that played `scenario`.
    pub fn languages_of(&self, scenario: &str) -> Vec<&'static str> {
        self.played
            .iter()
            .filter(|p| p.scenario == scenario)
            .map(|p| p.language)
            .collect()
    }
}

/// The observation of a step in a normalized result.
pub fn observation<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["observations"]
        .as_array()
        .and_then(|all| all.iter().find(|o| o["id"] == id))
        .unwrap_or_else(|| panic!("no observation {id}"))
}

/// The entries of a worker's log in a normalized result.
pub fn worker_log<'a>(doc: &'a Value, worker: &str) -> &'a [Value] {
    doc["workerLog"][worker]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// A node of a run as the platform holds it (`serverState`), found by its id in any run of the result.
pub fn node<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["serverState"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|run| run["nodes"].as_array().into_iter().flatten())
        .find(|node| node["id"] == id)
        .unwrap_or_else(|| panic!("no node {id} in the state of the platform"))
}

/// The status of the run whose observation is `step` (its `runId`), as the platform holds it.
pub fn run_status<'a>(doc: &'a Value, step: &str) -> &'a Value {
    let id = &observation(doc, step)["result"]["runId"];
    doc["serverState"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|run| run["runId"] == *id)
        .map(|run| &run["status"])
        .unwrap_or_else(|| panic!("the run of {step} is not in the state of the platform"))
}

/// The types of the journal events an `events` step observed, in order.
pub fn journal(doc: &Value, step: &str) -> Vec<String> {
    observation(doc, step)["result"]["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| event["type"].as_str().map(str::to_owned))
        .collect()
}

/// Runs `future` on a runtime that is given up a few seconds after the test ends: a runtime that is
/// dropped after a failure waits for ever for the blocking threads of the platform it started.
pub fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.block_on(future)));
    runtime.shutdown_timeout(Duration::from_secs(10));
    match outcome {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Languages whose `prepare.sh` this process ran (tests of one process share the build of the SDKs).
static PREPARED: tokio::sync::Mutex<std::collections::BTreeSet<&'static str>> =
    tokio::sync::Mutex::const_new(std::collections::BTreeSet::new());

/// One play at a time in a process: scenarios space their actions in time, and two platforms booting and
/// running side by side would perturb each other's timing (tests of a process run in parallel threads).
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn conform(selection: &Selection) -> Suite {
    let _alone = ONE_AT_A_TIME.lock().await;
    let _database = crate::support::hold_server();
    let only: Option<Vec<&str>> = selection
        .languages
        .as_ref()
        .map(|l| l.iter().map(String::as_str).collect());
    let langs = languages(only.as_deref());
    let all_official = std::env::var("SAGAT_SDK_LANGS").is_err() && only.is_none();
    let mut failures = Vec::new();
    if all_official {
        for (name, dir) in LANGUAGES {
            let run = sdk_source_root()
                .join(dir)
                .join("conformance")
                .join("run.sh");
            if !run.exists() {
                failures.push(format!(
                    "the {name} SDK has no conformance runner ({})",
                    run.display()
                ));
            }
        }
    }
    let langs: Vec<_> = langs
        .into_iter()
        .filter(|(_, root)| root.join("run.sh").exists())
        .collect();
    assert!(
        !langs.is_empty(),
        "no SDK has a conformance runner: {failures:?}"
    );

    if std::env::var("SAGAT_SDK_REUSE").is_err() {
        let mut prepared = PREPARED.lock().await;
        for (lang, root) in &langs {
            if prepared.contains(lang) {
                continue;
            }
            match script(root, "prepare.sh", &[], Duration::from_secs(1800)).await {
                Ok(_) => {
                    prepared.insert(lang);
                }
                Err(e) => failures.push(format!("{lang}: prepare failed: {e}")),
            }
        }
    }

    let bless = selection.bless;
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("sdk-conformance");
    let mut files: Vec<PathBuf> = std::fs::read_dir(suite().join("scenarios"))
        .expect("scenarios")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut played = Vec::new();
    let mut count = 0;
    for file in files {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if selection
            .scenarios
            .as_ref()
            .is_some_and(|w| !w.contains(&stem))
        {
            continue;
        }
        println!("--- {stem} ({} languages)", langs.len());
        let outcome = play(&file, &langs, bless, &out_dir).await;
        count += 1;
        for failure in &outcome.failures {
            println!("FAIL {failure}");
        }
        failures.extend(outcome.failures);
        played.extend(outcome.played);
    }
    if count == 0 {
        failures.push("no scenario was played".to_owned());
    }
    Suite { failures, played }
}

/// The languages that have a runner (and that `SAGAT_SDK_LANGS` allows).
pub fn available_languages() -> Vec<&'static str> {
    languages(None)
        .into_iter()
        .filter(|(_, root)| root.join("run.sh").exists())
        .map(|(name, _)| name)
        .collect()
}

/// Runs `prepare.sh` of these languages, once per process (unless `SAGAT_SDK_REUSE` is set); returns
/// what failed.
pub async fn ensure_prepared(names: &[&'static str]) -> Vec<String> {
    let mut failures = Vec::new();
    if std::env::var("SAGAT_SDK_REUSE").is_ok() {
        return failures;
    }
    let mut prepared = PREPARED.lock().await;
    for (lang, root) in languages(Some(names)) {
        if prepared.contains(lang) {
            continue;
        }
        match script(&root, "prepare.sh", &[], Duration::from_secs(1800)).await {
            Ok(_) => {
                prepared.insert(lang);
            }
            Err(e) => failures.push(format!("{lang}: prepare failed: {e}")),
        }
    }
    failures
}

/// What a runner did to the platform and how long it took, for the benchmark.
pub struct Measured {
    pub language: &'static str,
    /// The wall time of `run.sh run`: the runner's start, its steps and its end.
    pub wall: Duration,
    /// Calls the worker service answered while the runner ran, by method (all codes).
    pub calls: std::collections::BTreeMap<String, u64>,
    /// The same, by method and code, for the ones that did not end in OK.
    pub refused: std::collections::BTreeMap<String, u64>,
    /// The raw output of the runner.
    pub out: Value,
    /// Server-side task claim attempts made while this runner was active.
    pub stream_claim_attempts: f64,
    /// Persisted ready-to-lease delay (not network delivery latency), milliseconds.
    pub ready_to_lease_ms: Vec<f64>,
    /// Peak concurrent durable leases from journal intervals in a successful workload.
    pub peak_leases: i64,
    /// Durable ready event to public handler entry, using the runner's wall-clock timestamp.
    pub delivery_latency_ms: Vec<f64>,
    /// Peak handlers concurrently executing inside the SDK host.
    pub peak_local_inflight: u64,
    /// Peak resident set sampled for the runner process and its descendants, in KiB.
    pub peak_rss_kib: u64,
}

/// The `grpc_requests_total` counters of the engine, by (method, code).
async fn grpc_counters(run: &Run) -> std::collections::BTreeMap<(String, String), u64> {
    let text = run.engine.body("/metrics").await;
    let mut counters = std::collections::BTreeMap::new();
    for line in text
        .lines()
        .filter(|l| l.starts_with("grpc_requests_total{"))
    {
        let label = |name: &str| {
            line.split(&format!("{name}=\""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_owned()
        };
        let value = line
            .rsplit(' ')
            .next()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        counters.insert((label("method"), label("code")), value as u64);
    }
    counters
}

fn prometheus_counter(text: &str, metric: &str) -> f64 {
    text.lines()
        .filter(|line| {
            line.starts_with(&format!("{metric}{{")) || line.starts_with(&format!("{metric} "))
        })
        .filter_map(|line| line.split_whitespace().last()?.parse::<f64>().ok())
        .sum()
}

fn prometheus_counter_delta(before: &str, after: &str, metric: &str) -> f64 {
    prometheus_counter(after, metric) - prometheus_counter(before, metric)
}

#[test]
fn a_measurement_subtracts_process_global_stream_claim_history() {
    let previous_row_after = r#"
worker_task_stream_claims_total{outcome="empty"} 100
worker_task_stream_claims_total{outcome="ready"} 20
"#;
    let before = r#"
worker_task_stream_claims_total{outcome="empty"} 102
worker_task_stream_claims_total{outcome="ready"} 21
worker_task_stream_claims_total_created{outcome="ready"} 900
"#;
    let after = r#"
worker_task_stream_claims_total{outcome="empty"} 109
worker_task_stream_claims_total{outcome="ready"} 26
worker_task_stream_claims_total_created{outcome="ready"} 901
"#;

    assert_eq!(
        prometheus_counter_delta(
            previous_row_after,
            before,
            "worker_task_stream_claims_total"
        ),
        3.0,
        "late cleanup before this row remains in the cumulative source"
    );
    assert_eq!(
        prometheus_counter_delta(before, after, "worker_task_stream_claims_total"),
        12.0,
        "this row reports only increments after its own before snapshot"
    );
}

/// Plays a scenario file (any path, not only the suite's) in one language on a platform of its own and
/// measures it. Nothing is compared: this is for the benchmark.
pub async fn measure(path: &Path, language: &'static str) -> Result<Measured, String> {
    let _alone = ONE_AT_A_TIME.lock().await;
    let _database = crate::support::hold_server();
    let (_, root) = languages(Some(&[language]))
        .into_iter()
        .next()
        .ok_or_else(|| format!("{language} is not a language of the suite"))?;
    if !root.join("run.sh").exists() {
        return Err(format!("{language} has no conformance runner"));
    }
    let scenario: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let name = scenario["name"]
        .as_str()
        .ok_or("a scenario without a name")?
        .to_owned();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("sdk-measure")
        .join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path_arg = path.to_str().ok_or("path")?;
    let (text, _) = script(
        &root,
        "run.sh",
        &["build", path_arg],
        Duration::from_secs(300),
    )
    .await?;
    let built: Value = serde_json::from_str(&text)
        .map_err(|e| format!("build printed something that is not JSON: {e}"))?;
    let (run, config, _, _) = stage(&scenario, &built, language).await;
    let config_path = dir.join(format!("{language}.config.json"));
    let out_path = dir.join(format!("{language}.out.json"));
    write_private(
        &config_path,
        &serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out_path);
    let before = grpc_counters(&run).await;
    // `ONE_AT_A_TIME` is held until the preceding platform has stopped. Its late cleanup is therefore
    // already part of this process-global baseline, and cannot be charged to the new runner.
    let metrics_before = run.engine.body("/metrics").await;
    let started = std::time::Instant::now();
    let result = measured_script(
        &root,
        "run.sh",
        &[
            "run",
            path_arg,
            config_path.to_str().ok_or("path")?,
            out_path.to_str().ok_or("path")?,
        ],
        Duration::from_secs(900),
    )
    .await;
    let wall = started.elapsed();
    let after = grpc_counters(&run).await;
    // Deliberately close the window when the runner exits. Claims made only while stopping this row's
    // platform are outside client-active measurement and become part of the next row's baseline.
    let metrics = run.engine.body("/metrics").await;
    let stream_claim_attempts =
        prometheus_counter_delta(&metrics_before, &metrics, "worker_task_stream_claims_total");
    let database_measurements = async {
        let ready_to_lease_ms: Vec<f64> = sqlx::query_scalar(
            "SELECT (extract(epoch FROM (l.occurred_at - r.occurred_at))*1000)::float8 \
             FROM run_events l JOIN LATERAL (SELECT occurred_at FROM run_events \
             WHERE run_id=l.run_id AND node=l.node AND seq<l.seq \
             AND event_type IN ('step.released','step.retry_due') ORDER BY seq DESC LIMIT 1) r ON true \
             WHERE l.event_type='step.leased' ORDER BY l.occurred_at",
        )
        .fetch_all(&run.platform.api.db.pool)
        .await
        .map_err(|e| e.to_string())?;
        let peak_leases: i64 = sqlx::query_scalar(
            "SELECT coalesce(max(active),0)::bigint FROM ( \
             SELECT sum(delta) OVER (ORDER BY at,delta) active FROM ( \
             SELECT occurred_at at, 1 delta FROM run_events WHERE event_type='step.leased' UNION ALL \
             SELECT c.occurred_at at, -1 delta FROM run_events c \
             WHERE c.event_type IN ('step.completed','step.failed','step.lease_expired') \
             AND EXISTS (SELECT 1 FROM run_events l WHERE l.run_id=c.run_id AND l.node=c.node \
             AND l.seq<c.seq AND l.event_type='step.leased')) changes) counts",
        )
        .fetch_one(&run.platform.api.db.pool)
        .await
        .map_err(|e| e.to_string())?;
        let ready_by_step: std::collections::BTreeMap<(String, String), i64> =
            sqlx::query_as::<_, (String, String, i64)>(
            "SELECT l.run_id::text, l.node, floor(extract(epoch FROM r.occurred_at)*1000)::bigint \
             FROM run_events l JOIN LATERAL (SELECT occurred_at FROM run_events \
             WHERE run_id=l.run_id AND node=l.node AND seq<l.seq \
             AND event_type IN ('step.released','step.retry_due') ORDER BY seq DESC LIMIT 1) r ON true \
             WHERE l.event_type='step.leased'",
            )
            .fetch_all(&run.platform.api.db.pool)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(run_id, node, ready_at)| ((run_id, node), ready_at))
            .collect();
        Ok::<_, String>((ready_to_lease_ms, peak_leases, ready_by_step))
    }
    .await;
    run.stop().await;
    let (_, _, peak_rss_kib) = result?;
    let (ready_to_lease_ms, peak_leases, ready_by_step) = database_measurements?;
    let out: Value = std::fs::read(&out_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("{language} wrote no readable {}", out_path.display()))?;
    let mut delivery_latency_ms = Vec::new();
    let mut peak_local_inflight = 0;
    for measurement in out["workerMeasurements"]
        .as_object()
        .into_iter()
        .flat_map(|workers| workers.values())
    {
        peak_local_inflight =
            peak_local_inflight.max(measurement["peakInflight"].as_u64().unwrap_or_default());
        for delivery in measurement["deliveries"].as_array().into_iter().flatten() {
            let Some(run_id) = delivery["runId"].as_str() else {
                continue;
            };
            let Some(node) = delivery["node"].as_str() else {
                continue;
            };
            let Some(taken_at) = delivery["takenAtUnixMs"].as_i64() else {
                continue;
            };
            if let Some(ready_at) = ready_by_step.get(&(run_id.to_owned(), node.to_owned())) {
                delivery_latency_ms.push((taken_at - ready_at) as f64);
            }
        }
    }
    let mut calls = std::collections::BTreeMap::new();
    let mut refused = std::collections::BTreeMap::new();
    for ((method, code), n) in &after {
        let delta = n - before
            .get(&(method.clone(), code.clone()))
            .copied()
            .unwrap_or(0);
        *calls.entry(method.clone()).or_insert(0) += delta;
        if !code.eq_ignore_ascii_case("ok") && delta > 0 {
            refused.insert(format!("{method}:{code}"), delta);
        }
    }
    Ok(Measured {
        language,
        wall,
        calls,
        refused,
        out,
        stream_claim_attempts,
        ready_to_lease_ms,
        peak_leases,
        delivery_latency_ms,
        peak_local_inflight,
        peak_rss_kib,
    })
}

/// Plays an optional mode of a runner (`run.sh <mode> config out`, see the README) on a platform of its own,
/// for the scenario file at `path`, and returns what the runner wrote.
pub async fn run_mode(path: &Path, language: &'static str, mode: &str) -> Result<Value, String> {
    let _alone = ONE_AT_A_TIME.lock().await;
    let _database = crate::support::hold_server();
    let (_, root) = languages(Some(&[language]))
        .into_iter()
        .next()
        .ok_or_else(|| format!("{language} is not a language of the suite"))?;
    if !root.join("run.sh").exists() {
        return Err(format!("{language} has no conformance runner"));
    }
    let scenario: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let name = scenario["name"]
        .as_str()
        .ok_or("a scenario without a name")?
        .to_owned();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("sdk-modes")
        .join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // The workflows of the scenario, written by the language's own builders.
    let path_arg = path.to_str().ok_or("path")?;
    let (text, _) = script(
        &root,
        "run.sh",
        &["build", path_arg],
        Duration::from_secs(300),
    )
    .await?;
    let built: Value = serde_json::from_str(&text)
        .map_err(|e| format!("build printed something that is not JSON: {e}"))?;
    let (run, config, _, _) = stage(&scenario, &built, language).await;
    let config_path = dir.join(format!("{language}.{mode}.config.json"));
    let out_path = dir.join(format!("{language}.{mode}.out.json"));
    write_private(
        &config_path,
        &serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out_path);
    let result = script(
        &root,
        "run.sh",
        &[
            mode,
            config_path.to_str().ok_or("path")?,
            out_path.to_str().ok_or("path")?,
        ],
        Duration::from_secs(300),
    )
    .await;
    run.stop().await;
    result?;
    std::fs::read(&out_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("{language} wrote no readable {}", out_path.display()))
}
