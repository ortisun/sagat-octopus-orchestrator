//! Actual-release compatibility gate for payload classification (T016-052).
//!
//! Run only through `scripts/test-payload-classification-compat.sh`: it supplies binaries built
//! from immutable release v0.1.0 and their SHA256 manifest. This test does not substitute service
//! reconstruction for process or binary compatibility.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read as _;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::support::fresh_db;
use super::support::oidc::{AUDIENCE, ISSUER, TestIdp};

const RELEASE_TAG: &str = "v0.1.0";
const RELEASE_COMMIT: &str = "6ede1c7849c4ac5a209c08da8ad25294fbdd7795";
const TENANT: &str = "compat-release";
const ADMIN: &str = "compat-admin";
const NAMESPACE: &str = "compat-payments";
const WORKSPACE: &str = "compat-workspace";
const VALUE_CANARY: &str = "release-input-value-canary";
const EVENT_CANARY: &str = "release-event-value-canary";
const HEADER_CANARY: &str = "release-header-value-canary";
const BODY_CANARY: &str = "release-body-value-canary";
const PRE_CLASSIFICATION_COMMIT: &str = "3637ee8c4b98d2e5809f1527a9b2bb1fcaf2406a";

/// Exact HTTP bytes from the integrated 014/017 server before feature 016, on the same persisted
/// fixture before/after migration. No normalization or golden blessing is used.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires frozen pre-classification binary; run scripts/test-payload-classification-surfaces.sh"]
async fn unchanged_surfaces_match_actual_pre_classification_response_bytes() {
    sagat_api_http::auth::install_crypto_provider();
    let baseline = required_path("SAGAT_SURFACE_BASELINE_CP");
    let current = required_path("SAGAT_SURFACE_CURRENT_CP");
    let source = required_path("SAGAT_SURFACE_BASELINE_SOURCE");
    let artifacts = required_path("SAGAT_SURFACE_ARTIFACT_DIR");
    let logs = required_path("SAGAT_SURFACE_LOG_DIR");
    let manifest_path = required_path("SAGAT_SURFACE_MANIFEST");
    let manifest = fs::read_to_string(manifest_path).expect("surface artifact manifest");
    for (key, value) in [
        ("baseline-commit", PRE_CLASSIFICATION_COMMIT.to_owned()),
        ("baseline-control-plane-sha256", sha256(&baseline)),
        ("current-control-plane-sha256", sha256(&current)),
    ] {
        assert!(
            manifest
                .lines()
                .any(|line| line == format!("{key} {value}")),
            "artifact identity: {key}"
        );
    }
    assert_eq!(
        command_output(
            Path::new("git"),
            &["-C", source.to_str().expect("source"), "rev-parse", "HEAD"],
            &[]
        ),
        PRE_CLASSIFICATION_COMMIT
    );
    assert!(
        command_output(
            Path::new("git"),
            &[
                "-C",
                source.to_str().expect("source"),
                "status",
                "--porcelain"
            ],
            &[]
        )
        .is_empty()
    );

    let db = fresh_db().await;
    let idp = TestIdp::start().await;
    let mut env = process_env(&db.url, &idp);
    command_output(&baseline, &["migrate"], &env);
    env.extend([
        ("SAGAT_SEED_TENANT", TENANT.to_owned()),
        ("SAGAT_SEED_ADMIN_SUBJECT", ADMIN.to_owned()),
        ("SAGAT_SEED_WORKSPACE", WORKSPACE.to_owned()),
        ("SAGAT_SEED_NAMESPACE", NAMESPACE.to_owned()),
        ("SAGAT_SEED_ENVIRONMENT", "dev".to_owned()),
    ]);
    command_output(&baseline, &["seed-dev"], &env);
    let (old_server, old_base) = start_control_plane(
        &baseline,
        &db.url,
        &idp,
        None,
        &logs,
        "pre-classification-surfaces",
    )
    .await;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("surface HTTP client");
    let token = idp.token(ADMIN, TENANT).build();
    let mut runs = Vec::new();
    for (name, stateful) in [("surface-terminal", false), ("surface-operations", true)] {
        let mut ir = json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
            "metadata":{"name":name},"entrypoint":"work","nodes":[{"id":"work","type":"end"}]});
        if stateful {
            ir["nodes"] = json!([{"id":"work","type":"activity","activity":"surface.hold","next":"done"},{"id":"done","type":"end"}]);
            let state_schema = json!({"type":"object","additionalProperties":false,"properties":{"word":{"type":"string","maxLength":256}},"required":["word"]});
            ir["state"] = json!({"schema":state_schema,"initial":"input.account",
                "queries":{"word.read":{"argsSchema":{"type":"object","additionalProperties":false},"resultSchema":{"type":"string","maxLength":256},"expression":"state.word","disclosure":"declared-business-result-v1"}},
                "updates":{"word.replace":{"argsSchema":state_schema,"validate":"true","state":"args","result":"newState.word","resultSchema":{"type":"string","maxLength":256},"disclosure":"declared-business-result-v1"}}});
        }
        let (status, created, _) = api(
            &client,
            &old_base,
            &token,
            Method::POST,
            "/workflows",
            Some(NAMESPACE),
            Some(&json!({"name":name,"ir":ir})),
        )
        .await;
        assert_status(status, StatusCode::CREATED, &created);
        let workflow = created["id"].as_str().expect("surface workflow");
        let (status, published, _) = api(
            &client,
            &old_base,
            &token,
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            None,
            Some(&json!({"draftRevision":created["draftRevision"]})),
        )
        .await;
        assert_status(status, StatusCode::CREATED, &published);
        let (status, started, _) = api(
            &client,
            &old_base,
            &token,
            Method::POST,
            "/runs",
            Some(NAMESPACE),
            Some(&json!({"workflowId":workflow,"input":{"account":{"word":VALUE_CANARY}}})),
        )
        .await;
        assert_status(status, StatusCode::ACCEPTED, &started);
        runs.push(started["runId"].as_str().expect("surface run").to_owned());
    }
    let terminal = &runs[0];
    let operations = &runs[1];
    let update = json!({"updateKey":"surface-golden-key","args":{"word":VALUE_CANARY},"expectedStateRevision":0});
    let update_path = format!("/runs/{operations}/updates/word.replace");
    let (status, applied, _) = api(
        &client,
        &old_base,
        &token,
        Method::POST,
        &update_path,
        None,
        Some(&update),
    )
    .await;
    assert_status(status, StatusCode::OK, &applied);
    assert_eq!(applied["replayed"], false);
    assert_eq!(applied["result"], VALUE_CANARY);

    // A caught-up telemetry projection is a legitimate stable fixture: compare the actual lag=0
    // bytes, without scrubbing the response's clock or substituting a test clock in either server.
    sqlx::query("UPDATE telemetry_pending SET applied_seq=head_seq,behind_since=NULL")
        .execute(&db.pool)
        .await
        .expect("caught-up telemetry fixture");
    let mut requests = Vec::new();
    for (label, suffix) in [
        ("run", ""),
        ("journal", "/events?limit=200"),
        ("graph", "/graph"),
        ("changes", "/graph/changes?since=0"),
        ("stream", "/stream"),
        ("telemetry", "/telemetry"),
        ("attempts", "/attempts"),
    ] {
        requests.push((
            label,
            Method::GET,
            format!("/runs/{terminal}{suffix}"),
            None,
        ));
    }
    requests.push((
        "inspection",
        Method::POST,
        format!("/runs/{terminal}/payload-inspections"),
        Some(json!({"source":"input"})),
    ));
    requests.push((
        "query",
        Method::POST,
        format!("/runs/{operations}/queries/word.read"),
        Some(json!({"args":{}})),
    ));
    requests.push(("update", Method::POST, update_path, Some(update)));
    let mut goldens = BTreeMap::new();
    for (label, method, path, body) in &requests {
        let (status, value, wire) = api(
            &client,
            &old_base,
            &token,
            method.clone(),
            path,
            None,
            body.as_ref(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "baseline surface: {label}");
        if matches!(*label, "query" | "update") {
            assert_eq!(value["result"], VALUE_CANARY);
        } else {
            assert!(
                !wire.contains(VALUE_CANARY),
                "baseline surface leaked: {label}"
            );
        }
        if *label == "stream" {
            assert!(
                wire.contains("event: end"),
                "terminal SSE must have completed"
            );
        }
        fs::write(artifacts.join(format!("{label}.before.http-body")), &wire)
            .expect("baseline response artifact");
        goldens.insert(*label, wire);
    }
    old_server.stop();
    command_output(&current, &["migrate"], &process_env(&db.url, &idp));
    let (_new_server, new_base) =
        start_control_plane(&current, &db.url, &idp, None, &logs, "current-surfaces").await;
    for (label, method, path, body) in &requests {
        let (status, _, wire) = api(
            &client,
            &new_base,
            &token,
            method.clone(),
            path,
            None,
            body.as_ref(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "current surface: {label}");
        fs::write(artifacts.join(format!("{label}.after.http-body")), &wire)
            .expect("current response artifact");
        assert!(
            goldens[*label].as_bytes() == wire.as_bytes(),
            "pre-classification response changed: {label}"
        );
    }
}

#[tokio::test]
async fn bounded_discovery_backfill_reindexes_existing_steps_atomically() {
    let world = super::support::engine::world().await;
    let source = super::support::engine::document(
        "nonempty-discovery-upgrade",
        "work",
        vec![super::support::engine::activity("work")],
    );
    let (workflow, _) = world.publish(source).await;
    let run = world.start(workflow, &json!({})).await;
    type Row = (String, String, Vec<String>, Vec<i32>);
    let before: Vec<Row> = sqlx::query_as(
        "SELECT node,status,instance_path_nodes,instance_path_indices FROM steps WHERE run_id=$1 ORDER BY node",
    ).bind(run.as_uuid()).fetch_all(&world.db.pool).await.expect("retained steps");
    assert!(!before.is_empty());
    sqlx::raw_sql(include_str!("../../../crates/infra-postgres/migrations/20261205000000_bounded_execution_discovery.down.sql"))
        .execute(&world.db.pool).await.expect("restore the pre-discovery fixture schema");
    let mut tx = world.db.pool.begin().await.expect("migration transaction");
    sqlx::raw_sql(include_str!("../../../crates/infra-postgres/migrations/20261205000000_bounded_execution_discovery.up.sql"))
        .execute(&mut *tx).await.expect("upgrade nonempty steps with deferred quota triggers");
    tx.commit().await.expect("atomic migration commit");
    let after: Vec<Row> = sqlx::query_as(
        "SELECT node,status,instance_path_nodes,instance_path_indices FROM steps WHERE run_id=$1 ORDER BY node",
    ).bind(run.as_uuid()).fetch_all(&world.db.pool).await.expect("preserved steps");
    assert_eq!(before, after);
    let kind: String =
        sqlx::query_scalar("SELECT node_kind FROM steps WHERE run_id=$1 AND node='work'")
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("backfilled node kind");
    assert_eq!(kind, "activity");
    let indexed: bool =
        sqlx::query_scalar("SELECT to_regclass('steps_path_actionable_idx') IS NOT NULL")
            .fetch_one(&world.db.pool)
            .await
            .expect("discovery index");
    assert!(indexed);
}

fn required_path(name: &'static str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| {
        panic!("{name} is required; run scripts/test-payload-classification-compat.sh")
    });
    let path = PathBuf::from(value);
    assert!(path.exists(), "{name} does not exist: {}", path.display());
    path
}

fn sha256(path: &Path) -> String {
    let mut file = File::open(path).unwrap_or_else(|error| {
        panic!("cannot hash {}: {error}", path.display());
    });
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).expect("artifact read");
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn artifact_manifest() -> BTreeMap<String, String> {
    let path = required_path("SAGAT_COMPAT_MANIFEST");
    let mut fields = BTreeMap::new();
    for line in fs::read_to_string(path)
        .expect("compatibility artifact manifest")
        .lines()
    {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name != "studio-sha256" {
            fields.insert(name.to_owned(), value.to_owned());
        }
    }
    fields
}

fn command_output(program: &Path, args: &[&str], env: &[(&str, String)]) -> String {
    let mut command = Command::new(program);
    command.args(args);
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().unwrap_or_else(|error| {
        panic!("cannot run {}: {error}", program.display());
    });
    assert!(
        output.status.success(),
        "{} {:?} failed with {}\nstdout:\n{}\nstderr:\n{}",
        program.display(),
        args,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn unused_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve loopback port")
        .local_addr()
        .expect("reserved address")
}

struct Process {
    child: Child,
    log: PathBuf,
}

impl Process {
    fn spawn(program: &Path, args: &[&str], env: &[(&str, String)], log: PathBuf) -> Self {
        let output = File::create(&log).expect("process log");
        let error = output.try_clone().expect("clone process log");
        let mut command = Command::new(program);
        command
            .args(args)
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(error));
        for (name, value) in env {
            command.env(name, value);
        }
        let child = command.spawn().unwrap_or_else(|spawn_error| {
            panic!("cannot start {}: {spawn_error}", program.display());
        });
        Self { child, log }
    }

    fn assert_running(&mut self, label: &str) {
        if let Some(status) = self.child.try_wait().expect("process status") {
            let log = fs::read_to_string(&self.log).unwrap_or_default();
            panic!("{label} exited with {status}\n{log}");
        }
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn process_env(db_url: &str, idp: &TestIdp) -> Vec<(&'static str, String)> {
    static KEY_RING: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "compat:{}",
            sagat_infra_secrets::generate_key().expect("test key")
        )
    });
    vec![
        ("SAGAT_ENV", "dev".to_owned()),
        ("SAGAT_LOG_FORMAT", "pretty".to_owned()),
        ("DATABASE_URL", db_url.to_owned()),
        ("SAGAT_SECRETS_KEYS", KEY_RING.clone()),
        ("SAGAT_DB_MAX_CONNECTIONS", "8".to_owned()),
        ("SAGAT_OIDC_ISSUER", ISSUER.to_owned()),
        ("SAGAT_OIDC_AUDIENCE", AUDIENCE.to_owned()),
        ("SAGAT_OIDC_JWKS_URI", idp.jwks_url()),
        ("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "true".to_owned()),
        ("SAGAT_MIGRATE_ON_START", "false".to_owned()),
        ("OTEL_SDK_DISABLED", "true".to_owned()),
    ]
}

async fn wait_for_http(process: &mut Process, label: &str, url: &str) {
    let client = Client::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        process.assert_running(label);
        if let Ok(response) = client.get(url).send().await
            && response.status().is_success()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} did not become ready at {url}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn control_plane_env(
    db_url: &str,
    idp: &TestIdp,
    address: SocketAddr,
    studio: Option<&Path>,
) -> Vec<(&'static str, String)> {
    let mut env = process_env(db_url, idp);
    env.extend([
        ("SAGAT_BIND_ADDR", address.to_string()),
        ("SAGAT_ADMIN_ADDR", "off".to_owned()),
    ]);
    if let Some(studio) = studio {
        env.push(("SAGAT_STUDIO_DIR", studio.display().to_string()));
        env.push(("SAGAT_STUDIO_OIDC_ISSUER", ISSUER.to_owned()));
        env.push(("SAGAT_STUDIO_OIDC_CLIENT_ID", "compat-studio".to_owned()));
    }
    env
}

fn engine_env(
    db_url: &str,
    idp: &TestIdp,
    grpc: SocketAddr,
    admin: SocketAddr,
) -> Vec<(&'static str, String)> {
    let mut env = process_env(db_url, idp);
    env.extend([
        ("SAGAT_GRPC_ADDR", grpc.to_string()),
        ("SAGAT_ADMIN_ADDR", admin.to_string()),
        ("SAGAT_REPLICA_ID", "compat-release".to_owned()),
        ("SAGAT_SCHEDULER_INTERVAL_MS", "50".to_owned()),
    ]);
    env
}

async fn start_control_plane(
    binary: &Path,
    db_url: &str,
    idp: &TestIdp,
    studio: Option<&Path>,
    log_dir: &Path,
    label: &str,
) -> (Process, String) {
    let address = unused_address();
    let mut process = Process::spawn(
        binary,
        &["serve"],
        &control_plane_env(db_url, idp, address, studio),
        log_dir.join(format!("{label}.log")),
    );
    let base = format!("http://{address}/v1");
    wait_for_http(&mut process, label, &format!("{base}/ready")).await;
    (process, base)
}

async fn prove_engine_starts(
    binary: &Path,
    db_url: &str,
    idp: &TestIdp,
    log_dir: &Path,
    label: &str,
    probe: Option<(Uuid, &str)>,
) {
    let grpc = unused_address();
    let admin = unused_address();
    let mut process = Process::spawn(
        binary,
        &["serve"],
        &engine_env(db_url, idp, grpc, admin),
        log_dir.join(format!("{label}.log")),
    );
    wait_for_http(&mut process, label, &format!("http://{admin}/readyz")).await;
    if let Some((run, token)) = probe {
        // The released heartbeat opens and restores the persisted run before checking its lease.
        // A deliberately unleased fence proves that read path without an old-server state write.
        let mut client = sagat_api_grpc::pb::worker_service_client::WorkerServiceClient::connect(
            format!("http://{grpc}"),
        )
        .await
        .expect("released engine gRPC connection");
        let mut request = tonic::Request::new(sagat_api_grpc::pb::HeartbeatRequest {
            task_id: format!("{run}/work/1"),
            worker_id: String::new(),
            generation: 1,
            lease_token: "read-probe-without-a-lease".into(),
            run_id: run.to_string(),
            step_id: String::new(),
            attempt: 1,
            instance: None,
        });
        request.metadata_mut().insert(
            "authorization",
            tonic::metadata::MetadataValue::try_from(format!("Bearer {token}"))
                .expect("test authorization"),
        );
        let status = tokio::time::timeout(Duration::from_secs(10), client.heartbeat(request))
            .await
            .expect("released engine read timeout")
            .expect_err("unleased probe must be fenced");
        assert_eq!(
            status.code(),
            tonic::Code::Aborted,
            "released engine refusal: {}",
            status.message()
        );
        assert_eq!(
            status
                .metadata()
                .get("x-sagat-error")
                .and_then(|value| value.to_str().ok()),
            Some("STALE_ATTEMPT")
        );
        process.assert_running(label);
    }
    process.stop();
}

async fn api(
    client: &Client,
    base: &str,
    token: &str,
    method: Method,
    path: &str,
    namespace: Option<&str>,
    body: Option<&Value>,
) -> (StatusCode, Value, String) {
    let mut request = client
        .request(method.clone(), format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(namespace) = namespace {
        request = request.header("x-octopus-namespace", namespace);
    }
    if matches!(method, Method::POST | Method::PUT | Method::PATCH) {
        request = request.header("idempotency-key", Uuid::now_v7().to_string());
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("HTTP compatibility request");
    let status = response.status();
    let text = response.text().await.expect("HTTP compatibility body");
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, json, text)
}

fn workflow_document(marker: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "release-compat", "description": marker},
        "entrypoint": "work",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "x-sagat-classification": "public",
            "properties": {"visible": {"type": "string"}}
        },
        "nodes": [{"id": "work", "type": "activity", "activity": "compat.wait"}]
    })
}

fn assert_status(status: StatusCode, expected: StatusCode, body: &Value) {
    assert_eq!(status, expected, "unexpected HTTP answer: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires immutable v0.1.0 artifacts; run scripts/test-payload-classification-compat.sh"]
async fn actual_v0_1_0_processes_survive_the_additive_upgrade() {
    sagat_api_http::auth::install_crypto_provider();
    assert_eq!(
        std::env::var("SAGAT_COMPAT_RELEASE_TAG").as_deref(),
        Ok(RELEASE_TAG)
    );
    assert_eq!(
        std::env::var("SAGAT_COMPAT_RELEASE_COMMIT").as_deref(),
        Ok(RELEASE_COMMIT)
    );
    let old_source = required_path("SAGAT_COMPAT_OLD_SOURCE");
    let old_control_plane = required_path("SAGAT_COMPAT_OLD_CONTROL_PLANE");
    let old_engine = required_path("SAGAT_COMPAT_OLD_EXECUTION_ENGINE");
    let old_studio = required_path("SAGAT_COMPAT_OLD_STUDIO");
    let current_control_plane = required_path("SAGAT_COMPAT_CURRENT_CONTROL_PLANE");
    let manifest = artifact_manifest();
    let control_plane_sha256 = sha256(&old_control_plane);
    let execution_engine_sha256 = sha256(&old_engine);
    let current_control_plane_sha256 = sha256(&current_control_plane);
    assert_eq!(
        manifest.get("release-tag").map(String::as_str),
        Some(RELEASE_TAG)
    );
    assert_eq!(
        manifest.get("release-commit").map(String::as_str),
        Some(RELEASE_COMMIT)
    );
    assert_eq!(
        manifest.get("control-plane-sha256").map(String::as_str),
        Some(control_plane_sha256.as_str())
    );
    assert_eq!(
        manifest.get("execution-engine-sha256").map(String::as_str),
        Some(execution_engine_sha256.as_str())
    );
    assert_eq!(
        manifest
            .get("current-control-plane-sha256")
            .map(String::as_str),
        Some(current_control_plane_sha256.as_str())
    );
    let source_head = command_output(
        &PathBuf::from("git"),
        &[
            "-C",
            old_source.to_str().expect("source path"),
            "rev-parse",
            "HEAD",
        ],
        &[],
    );
    assert_eq!(source_head, RELEASE_COMMIT);
    assert!(
        command_output(
            &PathBuf::from("git"),
            &[
                "-C",
                old_source.to_str().expect("source path"),
                "status",
                "--porcelain"
            ],
            &[],
        )
        .is_empty(),
        "release worktree must remain clean"
    );
    assert_eq!(
        command_output(&old_control_plane, &["version"], &[]),
        "control-plane 0.1.0"
    );
    assert_eq!(
        command_output(&old_engine, &["version"], &[]),
        "execution-engine 0.1.0"
    );

    let logs = required_path("SAGAT_COMPAT_LOG_DIR");
    let db = fresh_db().await;
    let idp = TestIdp::start().await;
    let mut base_env = process_env(&db.url, &idp);
    command_output(&old_control_plane, &["migrate"], &base_env);
    base_env.extend([
        ("SAGAT_SEED_TENANT", TENANT.to_owned()),
        ("SAGAT_SEED_ADMIN_SUBJECT", ADMIN.to_owned()),
        ("SAGAT_SEED_WORKSPACE", WORKSPACE.to_owned()),
        ("SAGAT_SEED_NAMESPACE", NAMESPACE.to_owned()),
        ("SAGAT_SEED_ENVIRONMENT", "dev".to_owned()),
    ]);
    command_output(&old_control_plane, &["seed-dev"], &base_env);
    let (tenant_id, namespace_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT t.id,n.id FROM tenants t JOIN namespaces n ON n.tenant_id=t.id \
         WHERE t.slug=$1 AND n.slug=$2",
    )
    .bind(TENANT)
    .bind(NAMESPACE)
    .fetch_one(&db.pool)
    .await
    .expect("release seed tenant and namespace");

    prove_engine_starts(
        &old_engine,
        &db.url,
        &idp,
        &logs,
        "old-engine-before-upgrade",
        None,
    )
    .await;
    let (old_server, old_base) = start_control_plane(
        &old_control_plane,
        &db.url,
        &idp,
        Some(&old_studio),
        &logs,
        "old-control-plane-before-upgrade",
    )
    .await;
    let client = Client::new();
    let token = idp.token(ADMIN, TENANT).build();
    let mut first_document = workflow_document("published by v0.1.0");
    // The released parser accepted extension metadata without assigning classification meaning.
    // An upgrade must not retroactively reject its pinned executable definition. A new publication
    // still uses the strict current annotation rules (the second document below is valid).
    first_document["inputSchema"]["properties"]["visible"]["x-sagat-classification"] =
        json!("pre-016-note");
    let (status, created, _) = api(
        &client,
        &old_base,
        &token,
        Method::POST,
        "/workflows",
        Some(NAMESPACE),
        Some(&json!({"name":"release-compat", "ir":first_document})),
    )
    .await;
    assert_status(status, StatusCode::CREATED, &created);
    let workflow_id = created["id"].as_str().expect("workflow id");
    let revision = created["draftRevision"].as_u64().expect("draft revision");
    let (status, published, _) = api(
        &client,
        &old_base,
        &token,
        Method::POST,
        &format!("/workflows/{workflow_id}/publish"),
        None,
        Some(&json!({"draftRevision":revision})),
    )
    .await;
    assert_status(status, StatusCode::CREATED, &published);
    let old_version = Uuid::parse_str(published["versionId"].as_str().expect("old version id"))
        .expect("old version UUID");
    let (status, started, _) = api(
        &client,
        &old_base,
        &token,
        Method::POST,
        "/runs",
        Some(NAMESPACE),
        Some(&json!({"workflowId":workflow_id, "input":{"visible":VALUE_CANARY}})),
    )
    .await;
    assert_status(status, StatusCode::ACCEPTED, &started);
    let old_run =
        Uuid::parse_str(started["runId"].as_str().expect("old run id")).expect("old run UUID");
    let (status, absent, absent_wire) = api(
        &client,
        &old_base,
        &token,
        Method::POST,
        &format!("/runs/{old_run}/payload-disclosures"),
        None,
        Some(&json!({"source":"input"})),
    )
    .await;
    assert_status(status, StatusCode::NOT_FOUND, &absent);
    assert!(
        !absent_wire.contains(VALUE_CANARY),
        "an old server must not interpret an annotation as disclosure authority"
    );

    let (old_digest, old_document): (String, Vec<u8>) = sqlx::query_as(
        "SELECT digest,document FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_id)
    .bind(old_version)
    .fetch_one(&db.pool)
    .await
    .expect("release version before upgrade");
    let old_run_pin: Uuid =
        sqlx::query_scalar("SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant_id)
            .bind(old_run)
            .fetch_one(&db.pool)
            .await
            .expect("release run pin before upgrade");
    assert_eq!(old_run_pin, old_version);

    let (status, accepted, _) = api(
        &client,
        &old_base,
        &token,
        Method::POST,
        "/events",
        Some(NAMESPACE),
        Some(&json!({"eventId":"compat-event", "type":"compat.created", "data":{"canary":EVENT_CANARY}})),
    )
    .await;
    assert_status(status, StatusCode::ACCEPTED, &accepted);
    let event_source = accepted["source"].as_str().expect("event source");
    let connector = Uuid::now_v7();
    let dead_letter = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_connectors \
         (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) \
         VALUES ($1,$2,$3,'compat-connector','KAFKA',\
         '{\"servers\":[\"tls://broker.invalid:9092\"],\"topic\":\"compat\",\"startFrom\":\"LATEST\"}',10,$4)",
    )
    .bind(connector)
    .bind(tenant_id)
    .bind(namespace_id)
    .bind(ADMIN)
    .execute(&db.pool)
    .await
    .expect("release connector row");
    sqlx::query(
        "INSERT INTO connector_dead_letters \
         (id,tenant_id,namespace_id,connector_id,position,reason,headers,body) \
         VALUES ($1,$2,$3,$4,'compat/0/1','BAD_ENVELOPE',$5,$6)",
    )
    .bind(dead_letter)
    .bind(tenant_id)
    .bind(namespace_id)
    .bind(connector)
    .bind(json!({"authorization": HEADER_CANARY}))
    .bind(format!("{{\"canary\":\"{BODY_CANARY}\"}}").into_bytes())
    .execute(&db.pool)
    .await
    .expect("release dead letter row");
    let event_path = format!("/namespaces/{namespace_id}/events/{event_source}/compat-event");
    let dead_letter_path =
        format!("/namespaces/{namespace_id}/connectors/{connector}/dead-letters/{dead_letter}");
    let (status, old_event, old_event_wire) = api(
        &client,
        &old_base,
        &token,
        Method::GET,
        &event_path,
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &old_event);
    assert_eq!(old_event["data"]["canary"], EVENT_CANARY);
    assert!(old_event_wire.contains(EVENT_CANARY));
    let (status, old_letter, old_letter_wire) = api(
        &client,
        &old_base,
        &token,
        Method::GET,
        &dead_letter_path,
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &old_letter);
    assert_eq!(old_letter["headers"]["authorization"], HEADER_CANARY);
    assert!(
        old_letter["body"]
            .as_str()
            .is_some_and(|body| body.contains(BODY_CANARY))
    );
    assert!(old_letter_wire.contains(HEADER_CANARY));
    assert!(old_letter_wire.contains(BODY_CANARY));
    old_server.stop();

    sagat_infra_postgres::migrate(&db.pool)
        .await
        .expect("apply current additive migrations over the release database");
    let (upgraded_digest, upgraded_document, rules): (String, Vec<u8>, i16) = sqlx::query_as(
        "SELECT digest,document,classification_rules FROM workflow_versions \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_id)
    .bind(old_version)
    .fetch_one(&db.pool)
    .await
    .expect("release version after upgrade");
    assert_eq!(
        (upgraded_digest, upgraded_document),
        (old_digest.clone(), old_document.clone())
    );
    assert_eq!(rules, 0, "the release version must become legacy rules 0");
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant_id)
        .bind(old_run)
        .fetch_one(&db.pool)
        .await
        .expect("release run pin after upgrade"),
        old_version
    );

    prove_engine_starts(
        &old_engine,
        &db.url,
        &idp,
        &logs,
        "old-engine-after-upgrade",
        Some((old_run, &token)),
    )
    .await;
    let (old_after, old_after_base) = start_control_plane(
        &old_control_plane,
        &db.url,
        &idp,
        Some(&old_studio),
        &logs,
        "old-control-plane-after-upgrade",
    )
    .await;
    for path in [
        format!("/workflows/{workflow_id}"),
        format!("/runs/{old_run}"),
    ] {
        let (status, body, wire) = api(
            &client,
            &old_after_base,
            &token,
            Method::GET,
            &path,
            None,
            None,
        )
        .await;
        assert_status(status, StatusCode::OK, &body);
        assert!(
            !wire.contains(VALUE_CANARY),
            "the old payload-free reader must stay value-free"
        );
    }
    // The released server continues to expose the three old fields after the additive migration.
    // The current response change below is the documented event-read break, not wire compatibility.
    let (status, old_event_after, old_event_after_wire) = api(
        &client,
        &old_after_base,
        &token,
        Method::GET,
        &event_path,
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &old_event_after);
    assert_eq!(old_event_after["data"]["canary"], EVENT_CANARY);
    let (status, old_letter_after, old_letter_after_wire) = api(
        &client,
        &old_after_base,
        &token,
        Method::GET,
        &dead_letter_path,
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &old_letter_after);
    assert_eq!(old_letter_after["headers"]["authorization"], HEADER_CANARY);
    assert!(
        old_letter_after["body"]
            .as_str()
            .is_some_and(|body| body.contains(BODY_CANARY))
    );
    assert!(old_event_after_wire.contains(EVENT_CANARY));
    assert!(old_letter_after_wire.contains(HEADER_CANARY));
    assert!(old_letter_after_wire.contains(BODY_CANARY));
    old_after.stop();

    let (current, current_base) = start_control_plane(
        &current_control_plane,
        &db.url,
        &idp,
        Some(&old_studio),
        &logs,
        "current-control-plane",
    )
    .await;
    let current_origin = current_base.strip_suffix("/v1").expect("current API base");
    let studio_index = client
        .get(format!("{current_origin}/studio/"))
        .send()
        .await
        .expect("old Studio from current server");
    assert_eq!(studio_index.status(), StatusCode::OK);
    assert!(
        !studio_index
            .bytes()
            .await
            .expect("old Studio index")
            .is_empty()
    );

    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/e2e/scripts/prior-release-studio.mjs");
    let mut browser = tokio::process::Command::new("node")
        .arg(driver)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("old Studio Chromium driver");
    let input = serde_json::to_vec(&json!({
        "origin":current_origin, "token":token, "runId":old_run.to_string(),
        "forbidden":[VALUE_CANARY, EVENT_CANARY, HEADER_CANARY, BODY_CANARY]
    }))
    .expect("private browser configuration");
    let mut stdin = browser.stdin.take().expect("browser stdin");
    stdin
        .write_all(&input)
        .await
        .expect("browser configuration input");
    drop(stdin);
    let browser = tokio::time::timeout(Duration::from_secs(90), browser.wait_with_output())
        .await
        .expect("browser gate timeout")
        .expect("browser process output");
    fs::write(
        logs.join("old-studio-browser.log"),
        [&browser.stdout[..], &browser.stderr[..]].concat(),
    )
    .expect("retain browser gate log");
    assert!(
        browser.status.success(),
        "old Studio browser gate failed; inspect retained stage log"
    );

    let (status, legacy, legacy_wire) = api(
        &client,
        &current_base,
        &token,
        Method::POST,
        &format!("/runs/{old_run}/payload-disclosures"),
        None,
        Some(&json!({"source":"input"})),
    )
    .await;
    assert_status(status, StatusCode::OK, &legacy);
    assert_eq!(legacy["kind"], "structure-only");
    assert_eq!(legacy["reason"], "LEGACY_VERSION");
    assert!(!legacy_wire.contains(VALUE_CANARY));

    // Mutations must cold-restore the released executable definition, not just read run metadata.
    // This exercises the current process's DefinitionCache with the inert legacy annotation.
    for (action, paused) in [("PAUSE", true), ("RESUME", false)] {
        let (status, body, wire) = api(
            &client,
            &current_base,
            &token,
            Method::POST,
            &format!("/runs/{old_run}/operations"),
            None,
            Some(&json!({"action":action,"reason":"legacy restore compatibility"})),
        )
        .await;
        assert_status(status, StatusCode::ACCEPTED, &body);
        assert!(!wire.contains(VALUE_CANARY));
        assert_eq!(
            sqlx::query_scalar::<_, bool>("SELECT paused FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(tenant_id)
                .bind(old_run)
                .fetch_one(&db.pool)
                .await
                .expect("legacy pause projection"),
            paused
        );
    }

    let (status, current_event, current_event_wire) = api(
        &client,
        &current_base,
        &token,
        Method::GET,
        &event_path,
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &current_event);
    assert_eq!(current_event["dataDisclosure"], "structure-only");
    assert!(current_event.get("data").is_none() || current_event["data"].is_null());
    assert!(current_event.get("dataStructure").is_some());
    assert!(!current_event_wire.contains(EVENT_CANARY));
    let (status, current_letter, current_letter_wire) = api(
        &client,
        &current_base,
        &token,
        Method::GET,
        &dead_letter_path,
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &current_letter);
    assert_eq!(current_letter["bodyDisclosure"], "structure-only");
    assert!(current_letter.get("headers").is_none() || current_letter["headers"].is_null());
    assert!(current_letter.get("body").is_none() || current_letter["body"].is_null());
    assert!(current_letter.get("bodyStructure").is_some());
    assert!(!current_letter_wire.contains(HEADER_CANARY));
    assert!(!current_letter_wire.contains(BODY_CANARY));

    let (status, current_workflow, _) = api(
        &client,
        &current_base,
        &token,
        Method::GET,
        &format!("/workflows/{workflow_id}"),
        None,
        None,
    )
    .await;
    assert_status(status, StatusCode::OK, &current_workflow);
    let current_revision = current_workflow["draftRevision"]
        .as_u64()
        .expect("current draft revision");
    let second_document = workflow_document("published by current server");
    let (status, updated, _) = api(
        &client,
        &current_base,
        &token,
        Method::PUT,
        &format!("/workflows/{workflow_id}"),
        None,
        Some(&json!({"draftRevision":current_revision, "ir":second_document})),
    )
    .await;
    assert_status(status, StatusCode::OK, &updated);
    let second_revision = updated["draftRevision"].as_u64().expect("second revision");
    let (status, second, _) = api(
        &client,
        &current_base,
        &token,
        Method::POST,
        &format!("/workflows/{workflow_id}/publish"),
        None,
        Some(&json!({"draftRevision":second_revision})),
    )
    .await;
    assert_status(status, StatusCode::CREATED, &second);
    let second_version = Uuid::parse_str(second["versionId"].as_str().expect("second version"))
        .expect("second version UUID");
    let second_rules: i16 = sqlx::query_scalar(
        "SELECT classification_rules FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant_id)
    .bind(second_version)
    .fetch_one(&db.pool)
    .await
    .expect("current version rules pin");
    assert_eq!(second_rules, 1);
    assert_eq!(
        sqlx::query_as::<_, (String, Vec<u8>, i16)>(
            "SELECT digest,document,classification_rules FROM workflow_versions \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant_id)
        .bind(old_version)
        .fetch_one(&db.pool)
        .await
        .expect("old version after current publication"),
        (old_digest, old_document, 0)
    );
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant_id)
        .bind(old_run)
        .fetch_one(&db.pool)
        .await
        .expect("old run remains pinned"),
        old_version
    );

    let (status, new_run_body, _) = api(
        &client,
        &current_base,
        &token,
        Method::POST,
        "/runs",
        Some(NAMESPACE),
        Some(&json!({"workflowId":workflow_id, "input":{"visible":VALUE_CANARY}})),
    )
    .await;
    assert_status(status, StatusCode::ACCEPTED, &new_run_body);
    let new_run = new_run_body["runId"].as_str().expect("current run id");
    let (status, classified, classified_wire) = api(
        &client,
        &current_base,
        &token,
        Method::POST,
        &format!("/runs/{new_run}/payload-disclosures"),
        None,
        Some(&json!({"source":"input"})),
    )
    .await;
    assert_status(status, StatusCode::OK, &classified);
    assert_eq!(classified["kind"], "disclosed");
    assert!(classified_wire.contains(VALUE_CANARY));
    let (_, legacy_again, legacy_again_wire) = api(
        &client,
        &current_base,
        &token,
        Method::POST,
        &format!("/runs/{old_run}/payload-disclosures"),
        None,
        Some(&json!({"source":"input"})),
    )
    .await;
    assert_eq!(legacy_again["reason"], "LEGACY_VERSION");
    assert!(!legacy_again_wire.contains(VALUE_CANARY));
    current.stop();
}
