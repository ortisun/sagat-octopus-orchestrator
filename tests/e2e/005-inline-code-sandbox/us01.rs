//! US01 — an author edits a script and tries it in the sandbox (`FR-005-005`, `FR-005-006`,
//! `FR-005-007`).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::json;

use crate::support::scripts::lab;
use crate::support::subject;

const DOUBLE_TS: &str = "export default async function (input: { n: number }, octopus: any) {\n  console.log('doubling', input.n);\n  return { double: input.n * 2, label: 'ok' };\n}\n";

#[tokio::test]
async fn an_author_tries_a_typescript_script_and_gets_the_result_the_log_and_the_inferred_schemas()
{
    let lab = lab().await;
    let script = lab.script("double-it", "typescript", DOUBLE_TS).await;
    let run = lab.test(&script, &json!({"n": 21})).await;
    assert_eq!(run["status"], "SUCCEEDED", "{run}");
    assert_eq!(run["result"], json!({"double": 42, "label": "ok"}));
    assert!(
        run["log"].as_str().expect("log").contains("doubling 21"),
        "{run}"
    );
    assert_eq!(run["exitCode"], 0, "{run}");
    assert!(
        run["wallMs"].as_u64().is_some() && run["startMs"].as_u64().is_some(),
        "{run}"
    );
    // The schemas of the input and of the result, inferred from what was seen, in JSON Schema.
    let input = &run["inferredInputSchema"];
    assert_eq!(input["type"], "object", "{run}");
    assert_eq!(input["properties"]["n"]["type"], "integer", "{run}");
    let output = &run["inferredOutputSchema"];
    assert_eq!(output["properties"]["double"]["type"], "integer", "{run}");
    assert_eq!(output["properties"]["label"]["type"], "string", "{run}");
    assert_eq!(run["denied"], json!([]), "{run}");
    // Nothing was published by trying.
    let (status, versions) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/versions", lab.ns),
            None,
        )
        .await;
    assert_eq!(
        (status, versions["items"].as_array().map(Vec::len)),
        (200, Some(0))
    );
}

#[tokio::test]
async fn a_script_that_returns_nothing_has_succeeded_with_a_null_result() {
    let lab = lab().await;
    let script = lab
        .script("nothing", "python", "def run(input, octopus):\n    pass\n")
        .await;
    let run = lab.test(&script, &json!({})).await;
    assert_eq!(run["status"], "SUCCEEDED", "{run}");
    assert!(run["result"].is_null(), "{run}");
    assert!(
        run.get("failure").is_none_or(serde_json::Value::is_null),
        "{run}"
    );
}

#[tokio::test]
async fn a_python_script_is_tried_the_same_way() {
    let lab = lab().await;
    let script = lab
        .script(
            "greet",
            "python",
            "def run(input, octopus):\n    print('hello', input['name'])\n    return {'greeting': 'hi ' + input['name']}\n",
        )
        .await;
    let run = lab.test(&script, &json!({"name": "ada"})).await;
    assert_eq!(run["status"], "SUCCEEDED", "{run}");
    assert_eq!(run["result"], json!({"greeting": "hi ada"}));
    assert!(run["log"].as_str().expect("log").contains("hello ada"));
    assert_eq!(
        run["inferredOutputSchema"]["properties"]["greeting"]["type"],
        "string"
    );
}

#[tokio::test]
async fn the_log_the_author_sees_is_filtered() {
    let lab = lab().await;
    let script = lab
        .script(
            "leaky",
            "typescript",
            "export default function () {\n  console.log('Authorization: Bearer abcdefghijklmnopqrstuvwx');\n  console.log('key AKIAIOSFODNN7EXAMPLE in use');\n  console.log('-----BEGIN PRIVATE KEY-----\\nMIIBVQIBADANBg\\n-----END PRIVATE KEY-----');\n  return {};\n}\n",
        )
        .await;
    let run = lab.test(&script, &json!({})).await;
    let log = run["log"].as_str().expect("log");
    for secret in [
        "abcdefghijklmnopqrstuvwx",
        "AKIAIOSFODNN7EXAMPLE",
        "MIIBVQIBADANBg",
    ] {
        assert!(!log.contains(secret), "{secret} reached the log: {log}");
    }
    assert!(log.matches("[REDACTED]").count() >= 3, "{log}");
}

#[tokio::test]
async fn a_result_that_breaks_the_declared_schema_is_a_failure_that_names_the_rule_and_not_the_value()
 {
    let lab = lab().await;
    let script = lab
        .script_with(
            "strict",
            "typescript",
            "export default function () { return { total: 'twelve-secret-text' }; }\n",
            json!({"outputSchema": {"type": "object", "properties": {"total": {"type": "number"}}, "required": ["total"]}}),
        )
        .await;
    let run = lab.test(&script, &json!({})).await;
    assert_eq!(run["status"], "FAILED", "{run}");
    assert_eq!(run["failure"], "SCRIPT_SCHEMA_VIOLATION", "{run}");
    assert!(
        !run.to_string().contains("twelve-secret-text"),
        "the value leaked: {run}"
    );

    // And an input that does not satisfy its schema is refused before the script sees it.
    let script = lab
        .script_with(
            "picky",
            "python",
            "def run(input, octopus):\n    return input\n",
            json!({"inputSchema": {"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]}}),
        )
        .await;
    let run = lab.test(&script, &json!({"n": "not a number"})).await;
    assert_eq!(run["failure"], "SCRIPT_SCHEMA_VIOLATION", "{run}");
    let ok = lab.test(&script, &json!({"n": 3})).await;
    assert_eq!(ok["status"], "SUCCEEDED", "{ok}");
}

#[tokio::test]
async fn a_script_that_does_not_build_says_why_without_running() {
    let lab = lab().await;
    let ts = lab
        .script("broken-ts", "typescript", "export default function ( {\n")
        .await;
    let run = lab.test(&ts, &json!({})).await;
    assert_eq!(
        (run["status"].as_str(), run["failure"].as_str()),
        (Some("FAILED"), Some("SCRIPT_BUILD_REJECTED")),
        "{run}"
    );
    assert!(
        !run["log"].as_str().expect("log").is_empty(),
        "the toolchain's message: {run}"
    );
    let py = lab.script("broken-py", "python", "x = 1\n").await;
    let run = lab.test(&py, &json!({})).await;
    assert_eq!(run["failure"], "SCRIPT_BUILD_REJECTED", "{run}");
    assert!(
        run["log"].as_str().expect("log").contains("run"),
        "it names what is missing: {run}"
    );
}

#[tokio::test]
async fn a_script_that_throws_fails_with_its_error_in_the_log() {
    let lab = lab().await;
    let script = lab
        .script(
            "throws",
            "typescript",
            "export default function () { throw new Error('ledger is closed'); }\n",
        )
        .await;
    let run = lab.test(&script, &json!({})).await;
    assert_eq!(run["failure"], "SCRIPT_ERROR", "{run}");
    assert_eq!(run["retryable"], true, "{run}");
    assert!(
        run["log"]
            .as_str()
            .expect("log")
            .contains("ledger is closed"),
        "{run}"
    );
}

#[tokio::test]
async fn a_test_run_has_no_production_secrets_and_the_attempt_is_audited() {
    let lab = lab().await;
    lab.secret("payments-api-key", "prod-secret-value-0123456789")
        .await;
    let script = lab
        .script(
            "wants-a-secret",
            "typescript",
            "export default async function (_i: unknown, octopus: any) {\n  try { const v = await octopus.secret('payments-api-key'); console.log('got', v); return { leaked: v }; }\n  catch (e) { return { refused: String((e as Error).message) }; }\n}\n",
        )
        .await;
    let run = lab.test(&script, &json!({})).await;
    assert_eq!(run["result"]["refused"], "denied", "{run}");
    assert!(!run.to_string().contains("prod-secret-value"), "{run}");
    assert_eq!(run["denied"], json!(["secret payments-api-key"]), "{run}");
    let audited = lab.audited("script.violation").await;
    assert_eq!(audited, vec!["secret payments-api-key".to_owned()]);
}

#[tokio::test]
async fn a_test_that_is_never_taken_fails_as_unavailable() {
    let lab = lab().await;
    let script = lab.script("nobody-home", "typescript", DOUBLE_TS).await;
    let (status, queued) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts/{script}/test-runs", lab.ns),
            Some(&json!({"input": {"n": 1}})),
        )
        .await;
    assert_eq!(status, 202, "{queued}");
    assert_eq!(queued["status"], "QUEUED");
    sqlx::query(
        "UPDATE script_test_runs SET deadline_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&lab.p.api.db.pool)
    .await
    .expect("age");
    let id = queued["id"].as_str().expect("id");
    let (_, read) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/test-runs/{id}", lab.ns),
            None,
        )
        .await;
    assert_eq!(
        (read["status"].as_str(), read["failure"].as_str()),
        (Some("FAILED"), Some("SCRIPT_SANDBOX_UNAVAILABLE")),
        "{read}"
    );
}

#[tokio::test]
async fn only_someone_who_may_write_scripts_can_try_one() {
    let lab = lab().await;
    let script = lab.script("guarded", "typescript", DOUBLE_TS).await;
    lab.p.grant(lab.p.tenant, "viola", Role::Viewer).await;
    let (status, body) = lab
        .p
        .rest_as(
            "viola",
            Method::POST,
            &format!("/namespaces/{}/scripts/{script}/test-runs", lab.ns),
            Some(&json!({"input": {}})),
        )
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["details"]["requiredPermission"], "script:write");
    // A viewer reads the script and its code.
    let (status, read) = lab
        .p
        .rest_as(
            "viola",
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}", lab.ns),
            None,
        )
        .await;
    assert_eq!(status, 200, "{read}");
    // The runner's identity can neither read nor write scripts through the author's API.
    lab.p.grant(lab.p.tenant, "runner-x", Role::Runner).await;
    let (status, _) = lab
        .p
        .rest_as(
            "runner-x",
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}", lab.ns),
            None,
        )
        .await;
    assert_eq!(status, 403);
    let _ = subject("unused");
}
