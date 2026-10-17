use super::*;
use std::path::Path;

async fn call(p: &Platform, method: Method, path: &str, body: Value, expected: u16) -> Value {
    let response = p
        .api
        .as_user(method, path, "alice", "acme")
        .header("x-octopus-namespace", "secondary")
        .header("idempotency-key", Uuid::new_v4().to_string())
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, expected, "{value}");
    value
}
pub async fn run(
    p: &Platform,
    config: &mut Value,
    binary: &Path,
    config_file: &Path,
    log_file: &Path,
) {
    let workspace = p
        .api
        .create_workspace("alice", "acme", "second-workspace")
        .await;
    let ns = p
        .api
        .create_namespace("alice", "acme", &workspace, "secondary", "dev")
        .await;
    let secret =
        json!({"username":"managed_reader","password":"runner-password-canary"}).to_string();
    call(p,Method::PUT,&format!("/namespaces/{ns}/secrets/credential"),json!({
        "value":secret,"requestId":Uuid::new_v4().to_string(),"expectedVersion":0,"requiredProvider":"openbao"
    }),200).await;
    let port = config["deployment"]["allowPorts"][0].clone();
    call(
        p,
        Method::PUT,
        &format!("/namespaces/{ns}/connections/target"),
        json!({
            "requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"status":"ACTIVE","config":{
                "driver":"postgresql","host":"127.0.0.1","port":port,"database":"postgres",
                "tls":{"mode":"development-plaintext"},"secretName":"credential"
            }
        }),
        200,
    )
    .await;
    sqlx::query("INSERT INTO worker_bindings (tenant_id,namespace_id,subject,pattern,updated_by) VALUES($1,$2,'isolated-runner','*','test')")
        .bind(p.tenant.as_uuid()).bind(ns.parse::<Uuid>().unwrap()).execute(&p.api.db.pool).await.unwrap();
    let ir = document(
        "second-namespace",
        "database",
        vec![
            json!({"id":"database","type":"database","connectionRef":{"name":"target"},"operation":"query","statement":"SELECT 1::bigint AS value"}),
        ],
    );
    let workflow = call(
        p,
        Method::POST,
        "/workflows",
        json!({"name":"second-namespace","ir":ir}),
        201,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        p,
        Method::POST,
        &format!("/workflows/{workflow}/publish"),
        json!({"draftRevision":1}),
        201,
    )
    .await;
    let queued=p.publish("always-busy",&document("always-busy","database",vec![json!({"id":"database","type":"database","connectionRef":{"name":"target"},"operation":"query","statement":"SELECT 1::bigint AS value FROM pg_sleep(0.3)"})])).await;
    let mut first = Vec::new();
    for _ in 0..8 {
        first.push(p.start(&queued, &json!({})).await);
    }
    let second = call(
        p,
        Method::POST,
        "/runs",
        json!({"workflowId":workflow,"input":{}}),
        202,
    )
    .await["runId"]
        .as_str()
        .unwrap()
        .to_owned();
    config["namespaces"][0]["allowCidrs"] = json!(["127.0.0.0/8"]);
    let mut policy = config["namespaces"][0].clone();
    policy["id"] = json!(ns);
    policy["slug"] = json!("secondary");
    config["namespaces"].as_array_mut().unwrap().push(policy);
    config["maxConcurrent"] = json!(1);
    std::fs::write(config_file, config.to_string()).unwrap();
    let logs = std::fs::OpenOptions::new()
        .append(true)
        .open(log_file)
        .unwrap();
    let mut child = Process(
        Command::new(binary)
            .env_clear()
            .env("SAGAT_DATABASE_RUNNER_CONFIG", config_file)
            .env("SAGAT_DATABASE_RUNNER_METRICS_ADDR", "127.0.0.1:0")
            .env("RUST_LOG", "trace")
            .stdout(Stdio::from(logs.try_clone().unwrap()))
            .stderr(Stdio::from(logs))
            .spawn()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value = call(p, Method::GET, &format!("/runs/{second}"), Value::Null, 200).await;
            if value["status"] != "RUNNING" {
                assert_eq!(value["status"], "SUCCEEDED");
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let mut waiting = 0;
    for id in &first {
        if p.status(id).await == "RUNNING" {
            waiting += 1;
        }
    }
    assert!(
        waiting > 0,
        "second namespace progresses while first still has backlog"
    );
    for id in &first {
        finished(p, id, "SUCCEEDED").await;
    }
    child.stop().await;
}
