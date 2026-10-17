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
use uuid::Uuid;

use super::support::fresh_db;
use super::support::oidc::{AUDIENCE, ISSUER, TestIdp};

const RELEASE_TAG: &str = "v0.1.0";
const RELEASE_COMMIT: &str = "e06532dd0e288273f63966e0d950bb4d7c743459";
const TENANT: &str = "compat-release";
const ADMIN: &str = "compat-admin";
const NAMESPACE: &str = "compat-payments";
const WORKSPACE: &str = "compat-workspace";
const VALUE_CANARY: &str = "release-input-value-canary";
const EVENT_CANARY: &str = "release-event-value-canary";
const HEADER_CANARY: &str = "release-header-value-canary";
const BODY_CANARY: &str = "release-body-value-canary";

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
    format!("{:x}", digest.finalize())
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
    vec![
        ("SAGAT_ENV", "dev".to_owned()),
        ("SAGAT_LOG_FORMAT", "pretty".to_owned()),
        ("DATABASE_URL", db_url.to_owned()),
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

    let logs = tempfile::tempdir().expect("compatibility logs");
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
        logs.path(),
        "old-engine-before-upgrade",
    )
    .await;
    let (old_server, old_base) = start_control_plane(
        &old_control_plane,
        &db.url,
        &idp,
        Some(&old_studio),
        logs.path(),
        "old-control-plane-before-upgrade",
    )
    .await;
    let client = Client::new();
    let token = idp.token(ADMIN, TENANT).build();
    let first_document = workflow_document("published by v0.1.0");
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
        logs.path(),
        "old-engine-after-upgrade",
    )
    .await;
    let (old_after, old_after_base) = start_control_plane(
        &old_control_plane,
        &db.url,
        &idp,
        Some(&old_studio),
        logs.path(),
        "old-control-plane-after-upgrade",
    )
    .await;
    for path in [
        format!("/workflows/{workflow_id}"),
        format!("/runs/{old_run}"),
    ] {
        let (status, body, _) = api(
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
        logs.path(),
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
