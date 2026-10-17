//! Execute the checked-in quickstart with the shipped CLI/curl and actual HTTP/gRPC/PostgreSQL.
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::canary::{ADMIN, DEVELOPER, Fixture, NAMESPACE, TENANT, fixture};
use crate::cli::{cli, output_bytes};
use crate::support::worker::{RunningEngine, engine_env};

const SECRET: &str = "canary-016-secret-value";
const PERSONAL: &str = "canary-016-pii-value";

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../specs/016-payload-classification/examples")
}

async fn curl(
    fixture: &Fixture,
    actor: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    expected: u16,
) -> Value {
    let token = fixture.api.idp.token(actor, TENANT).build();
    let url = format!("{}{path}", fixture.api.base);
    let method = method.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("curl");
        command.args([
            "--silent",
            "--show-error",
            "--max-time",
            "15",
            "--request",
            &method,
            "--header",
            "@-",
            "--header",
            "Content-Type: application/json",
            "--header",
            &format!("X-Octopus-Namespace: {NAMESPACE}"),
            "--write-out",
            "\n%{http_code}",
        ]);
        if let Some(body) = body {
            command.arg("--data").arg(body.to_string());
        }
        let mut child = command
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("curl process");
        child
            .stdin
            .take()
            .expect("private curl headers")
            .write_all(format!("Authorization: Bearer {token}\n").as_bytes())
            .expect("curl header stdin");
        child.wait_with_output().expect("curl completion")
    })
    .await
    .expect("curl join");
    assert!(output.status.success(), "curl transport failed");
    let text = String::from_utf8(output.stdout).expect("curl UTF-8 response");
    let (wire, status) = text.rsplit_once('\n').expect("curl status trailer");
    assert_eq!(status.parse::<u16>().expect("HTTP status"), expected);
    assert!(
        !wire.contains(SECRET),
        "a secret crossed the quickstart HTTP boundary"
    );
    serde_json::from_str(wire).expect("curl JSON response")
}

#[tokio::test]
async fn the_documented_classified_orders_complete_and_keep_their_original_pin() {
    let fixture = fixture("quickstart").await;
    let root = examples();
    let workflow = root
        .join("orders-classified.json")
        .to_string_lossy()
        .into_owned();
    let relaxed = root
        .join("orders-classified-relaxed.json")
        .to_string_lossy()
        .into_owned();
    let defect = root
        .join("classification-loosened.json")
        .to_string_lossy()
        .into_owned();
    let mocks = root.join("mocks.json").to_string_lossy().into_owned();
    for file in [&workflow, &relaxed] {
        assert!(
            cli(&fixture, ADMIN, &["workflow", "validate", file])
                .await
                .status
                .success()
        );
    }
    let refused = cli(&fixture, ADMIN, &["workflow", "validate", &defect]).await;
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&output_bytes(&refused)).contains("CLASSIFICATION_LOOSENED"));
    assert!(
        cli(&fixture, ADMIN, &["workflow", "publish", &workflow])
            .await
            .status
            .success()
    );
    let ring = format!("k1:{}", fixture.api.key_k1);
    let mut engine = RunningEngine::start(engine_env(
        &fixture.api.db,
        &fixture.api.idp,
        &[("SAGAT_SECRETS_KEYS", &ring)],
    ))
    .await;
    let input = std::fs::read_to_string(root.join("input.json")).expect("checked-in input");
    let worker_args = [
        "worker",
        "dev",
        "--mocks",
        &mocks,
        "--workflow",
        &workflow,
        "--bind",
        "--max-tasks",
        "1",
        "--idle-exit",
        "20",
        "--grpc",
        &engine.grpc_url,
    ];
    let run_args = [
        "workflow",
        "run",
        "orders-classified",
        "--input",
        &input,
        "--wait",
        "--timeout",
        "30",
    ];
    let (worker, run) = tokio::join!(
        cli(&fixture, ADMIN, &worker_args),
        cli(&fixture, ADMIN, &run_args)
    );
    assert!(worker.status.success(), "the mock worker failed");
    assert!(!String::from_utf8_lossy(&output_bytes(&worker)).contains(SECRET));
    assert!(run.status.success(), "the documented run failed");
    let run_wire = String::from_utf8(run.stdout).expect("CLI run output");
    let run_id = run_wire
        .lines()
        .find_map(|line| line.strip_prefix("run ")?.split_whitespace().next())
        .expect("started run id");
    let run_path = format!("/runs/{run_id}");
    let before = curl(&fixture, ADMIN, "GET", &run_path, None, 200).await;
    assert_eq!(before["status"], "SUCCEEDED");
    let disclosure_path = format!("{run_path}/payload-disclosures");
    for (actor, internal) in [
        ("viewer-canary", false),
        (DEVELOPER, true),
        ("operator-canary", true),
    ] {
        let body = curl(
            &fixture,
            actor,
            "POST",
            &disclosure_path,
            Some(json!({"source":"input"})),
            200,
        )
        .await;
        assert!(!body.to_string().contains(PERSONAL));
        assert_eq!(body.to_string().contains("\"value\":42"), internal);
    }
    let denied = curl(
        &fixture,
        DEVELOPER,
        "POST",
        &disclosure_path,
        Some(json!({"source":"input","reveal":["pii"]})),
        403,
    )
    .await;
    assert_eq!(denied["details"]["reason"], "PII_REVEAL_NOT_GRANTED");
    let grant = cli(
        &fixture,
        ADMIN,
        &["namespace", "reveal-grants", "grant", DEVELOPER],
    )
    .await;
    assert!(grant.status.success());
    let revealed = curl(
        &fixture,
        DEVELOPER,
        "POST",
        &disclosure_path,
        Some(json!({"source":"input","reveal":["pii"]})),
        200,
    )
    .await;
    assert!(revealed.to_string().contains(PERSONAL));
    let result = curl(
        &fixture,
        DEVELOPER,
        "POST",
        &disclosure_path,
        Some(json!({"source":"result","reveal":["pii"]})),
        200,
    )
    .await;
    assert!(result.to_string().contains(PERSONAL));
    assert!(result.to_string().contains("accepted"));
    let journal: Value = curl(
        &fixture,
        ADMIN,
        "GET",
        &format!("{run_path}/events?limit=200"),
        None,
        200,
    )
    .await;
    let sequence = journal["items"]
        .as_array()
        .expect("journal items")
        .iter()
        .find(|entry| entry["eventType"] == "step.completed" && entry["stepId"] == "charge")
        .expect("actual charge completion")["seq"]
        .clone();
    let step = curl(
        &fixture,
        DEVELOPER,
        "POST",
        &disclosure_path,
        Some(json!({"source":"journal","sequence":sequence,"reveal":["pii"]})),
        200,
    )
    .await;
    assert!(step.to_string().contains("accepted"));
    let inspected = cli(
        &fixture,
        DEVELOPER,
        &["workflow", "inspect", run_id, "--values", "--reveal-pii"],
    )
    .await;
    assert!(inspected.status.success());
    let inspected = String::from_utf8_lossy(&inspected.stdout);
    assert!(inspected.contains(PERSONAL));
    assert!(!inspected.contains(SECRET));
    let revoked = cli(
        &fixture,
        ADMIN,
        &["namespace", "reveal-grants", "revoke", DEVELOPER],
    )
    .await;
    assert!(revoked.status.success());
    curl(
        &fixture,
        DEVELOPER,
        "POST",
        &disclosure_path,
        Some(json!({"source":"input","reveal":["pii"]})),
        403,
    )
    .await;

    let published = cli(&fixture, ADMIN, &["workflow", "publish", &relaxed]).await;
    assert!(published.status.success());
    assert!(String::from_utf8_lossy(&output_bytes(&published)).contains("CLASSIFICATION_RELAXED"));
    let workflow_id = before["workflowId"].as_str().expect("workflow id");
    curl(
        &fixture,
        ADMIN,
        "PUT",
        &format!("/workflows/{workflow_id}/active-version"),
        Some(json!({"version":2,"reason":"classification demo activation"})),
        200,
    )
    .await;
    let after = curl(&fixture, ADMIN, "GET", &run_path, None, 200).await;
    assert_eq!(before["versionId"], after["versionId"]);
    let pinned = curl(
        &fixture,
        DEVELOPER,
        "POST",
        &disclosure_path,
        Some(json!({"source":"input"})),
        200,
    )
    .await;
    assert!(!pinned.to_string().contains(PERSONAL));

    curl(&fixture, ADMIN, "POST", "/events", Some(json!({"eventId":"evt-canary-1","type":"classification.demo","data":{"personal":PERSONAL}})), 202).await;
    let event_path = format!("/namespaces/{}/events/api/evt-canary-1", fixture.namespace);
    let structure = curl(&fixture, "viewer-canary", "GET", &event_path, None, 200).await;
    assert_eq!(structure["dataDisclosure"], "structure-only");
    assert!(!structure.to_string().contains(PERSONAL));
    curl(
        &fixture,
        DEVELOPER,
        "GET",
        &format!("{event_path}?reveal=pii"),
        None,
        403,
    )
    .await;

    // A retained connector letter is the documented isolated fixture. It does not claim a live
    // Kafka delivery; that transport has its own real-broker integration suite.
    let connector = uuid::Uuid::now_v7();
    let letter = uuid::Uuid::now_v7();
    let namespace = uuid::Uuid::parse_str(&fixture.namespace).expect("namespace UUID");
    sqlx::query("INSERT INTO event_connectors (id,tenant_id,namespace_id,slug,kind,settings,status,rate_per_second,created_by) VALUES ($1,$2,$3,'quickstart-letter','KAFKA','{\"servers\":[\"tls://broker.invalid:9092\"],\"topic\":\"quickstart\",\"startFrom\":\"LATEST\"}','PAUSED',10,$4)")
        .bind(connector).bind(fixture.tenant.as_uuid()).bind(namespace).bind(ADMIN)
        .execute(&fixture.api.db.pool).await.expect("retained connector fixture");
    sqlx::query("INSERT INTO connector_dead_letters (id,tenant_id,namespace_id,connector_id,position,reason,headers,body) VALUES ($1,$2,$3,$4,'quickstart/0/1','BAD_ENVELOPE',$5,$6)")
        .bind(letter).bind(fixture.tenant.as_uuid()).bind(namespace).bind(connector)
        .bind(json!({"diagnostic":PERSONAL})).bind(serde_json::to_vec(&json!({"personal":PERSONAL})).expect("letter body"))
        .execute(&fixture.api.db.pool).await.expect("retained letter fixture");
    let letters_path = format!("/namespaces/{namespace}/connectors/{connector}/dead-letters");
    let list = curl(&fixture, "viewer-canary", "GET", &letters_path, None, 200).await;
    assert!(!list.to_string().contains(PERSONAL));
    assert!(list["items"][0].get("body").is_none());
    assert!(list["items"][0].get("headers").is_none());
    assert!(
        cli(
            &fixture,
            ADMIN,
            &["namespace", "reveal-grants", "grant", DEVELOPER]
        )
        .await
        .status
        .success()
    );
    let event = curl(
        &fixture,
        DEVELOPER,
        "GET",
        &format!("{event_path}?reveal=pii"),
        None,
        200,
    )
    .await;
    assert_eq!(event["data"]["personal"], PERSONAL);
    let letter = curl(
        &fixture,
        DEVELOPER,
        "GET",
        &format!("{letters_path}/{letter}?reveal=pii"),
        None,
        200,
    )
    .await;
    assert_eq!(letter["headers"]["diagnostic"], PERSONAL);
    let audit = curl(&fixture, ADMIN, "GET", "/audit?limit=200", None, 200).await;
    assert!(!audit.to_string().contains(PERSONAL));
    engine.finish().await.expect("engine shutdown");
}
