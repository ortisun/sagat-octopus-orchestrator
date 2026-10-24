//! Same real OpenBao + target PostgreSQL + isolated runner as the published execution path.
use super::*;

pub async fn run(p: &Platform, ns: &str) {
    let probe_path = format!("/namespaces/{ns}/connections/target/probe");
    let (status, probe) = p.rest(Method::POST, &probe_path, Some(&json!({}))).await;
    assert_eq!(status, 201, "{probe}");
    let run = probe["runId"].as_str().unwrap();
    finished(p, run, "SUCCEEDED").await;
    let (status, metadata) = p.rest(Method::GET, &probe_path, None).await;
    assert_eq!(status, 200);
    assert_eq!(metadata["run"]["runId"], run);
    assert_eq!(metadata["run"]["status"], "SUCCEEDED");
    assert_eq!(metadata["revision"], 1);
    let (status, output) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/runs/{run}/database-results/probe"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{output}");
    assert_eq!(output["rows"][0][0], json!({"type":"integer","value":"1"}));
    let body = json!({"mode":"workflow", "input":{"number":"42"}, "definition":document("managed-ad-hoc", "first", vec![
        json!({"id":"first", "type":"database", "connectionRef":{"name":"target"}, "operation":"query", "statement":"SELECT $1::bigint", "parameters":[{"type":"integer","expression":"input.number"}], "next":"second"}),
        json!({"id":"second", "type":"database", "connectionRef":{"name":"target"}, "operation":"query", "statement":"SELECT $1::bigint", "parameters":[{"type":"integer","expression":"steps.first.output.rows[0][0].value"}]})
    ])});
    let (status, execution) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/database-executions"),
            Some(&body),
        )
        .await;
    assert_eq!(status, 201, "{execution}");
    assert_eq!(execution["origin"], "AD_HOC_WORKFLOW");
    let run = execution["runId"].as_str().unwrap();
    finished(p, run, "SUCCEEDED").await;
    let (status, output) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/runs/{run}/database-results/second"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{output}");
    assert_eq!(output["rows"][0][0]["value"], "42");
    let secret =
        json!({"username":"managed_reader","password":"invalid-probe-password-canary"}).to_string();
    assert_eq!(p.rest(Method::PUT, &format!("/namespaces/{ns}/secrets/invalid-probe"), Some(&json!({"value":secret,"requestId":Uuid::new_v4().to_string(),"expectedVersion":0,"requiredProvider":"openbao"}))).await.0, 200);
    let (_, target) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/connections/target"),
            None,
        )
        .await;
    let mut config = target["config"].clone();
    config["secretName"] = json!("invalid-probe");
    assert_eq!(p.rest(Method::PUT, &format!("/namespaces/{ns}/connections/invalid-target"), Some(&json!({"config":config,"status":"ACTIVE","requestId":Uuid::new_v4().to_string(),"expectedRevision":0}))).await.0, 200);
    let path = format!("/namespaces/{ns}/connections/invalid-target/probe");
    let (status, probe) = p.rest(Method::POST, &path, Some(&json!({}))).await;
    assert_eq!(status, 201, "{probe}");
    failure(
        p,
        probe["runId"].as_str().unwrap(),
        "DATABASE_AUTHENTICATION_FAILED",
    )
    .await;
    let (_, metadata) = p.rest(Method::GET, &path, None).await;
    assert_eq!(metadata["failureCode"], "DATABASE_AUTHENTICATION_FAILED");
    assert!(
        !metadata
            .to_string()
            .contains("invalid-probe-password-canary")
    );
}
