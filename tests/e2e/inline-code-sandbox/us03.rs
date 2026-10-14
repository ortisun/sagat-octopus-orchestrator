//! US03 — an author can test, publish, and run a C# script against an exactly pinned dependency.

use reqwest::Method;
use serde_json::json;

use crate::support::scripts::lab;

const LOCK: &str = r#"{
  "version": 1,
  "dependencies": {
    "net10.0": {
      "Newtonsoft.Json": {
        "type": "Direct",
        "requested": "[13.0.4, 13.0.4]",
        "resolved": "13.0.4",
        "contentHash": "pdgNNMai3zv51W5aq268sujXUyx7SNdE2bj1wZcWjAQrKMFZV260lbqYop1d2GM67JI1huLRwxo9ZqnfF/lC6A=="
      }
    }
  }
}"#;

const SOURCE: &str = r#"
using System.Text.Json;
using Newtonsoft.Json.Linq;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
    {
        var value = JObject.Parse(input.GetRawText()).Value<int>("n");
        return ValueTask.FromResult(JsonSerializer.SerializeToElement(new
        {
            answer = value + 1,
            idempotencyKey = octopus.IdempotencyKey
        }));
    }
}
"#;

#[tokio::test]
async fn csharp_with_an_allowlisted_nuget_package_is_tested_published_and_run_by_a_workflow() {
    let lab = lab().await;
    let script = lab
        .script_with(
            "csharp-newtonsoft",
            "csharp",
            SOURCE,
            json!({"lockfile": LOCK}),
        )
        .await;

    let tested = lab.test(&script, &json!({"n": 1})).await;
    assert_eq!(tested["status"], "SUCCEEDED", "{tested}");
    assert_eq!(tested["result"]["answer"], 2, "{tested}");

    let version = lab.publish(&script).await;
    assert_eq!(version["status"], "READY", "{version}");
    assert!(
        version["toolchain"]
            .as_str()
            .is_some_and(|toolchain| toolchain.contains("build=sha256:")
                && toolchain.contains(";runtime=sha256:")
                && toolchain.ends_with(";protocol=artifact-v2")),
        "{version}"
    );
    let digest = version["artifactDigest"].as_str().expect("digest");
    let workflow = lab
        .workflow("uses-csharp", digest, json!({"input": {"n": 41}}))
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    let output = lab.output(&run, "s").await.expect("output");
    assert_eq!(
        output["answer"], 1,
        "an entry script receives the node input"
    );
    assert_eq!(output["idempotencyKey"], format!("{run}:s"));
}

#[tokio::test]
async fn dedicated_go_and_java_builders_are_accepted_by_the_registry() {
    let lab = lab().await;
    for language in ["go", "java"] {
        let (status, body) = lab
            .call(
                Method::POST,
                &format!("/namespaces/{}/scripts", lab.ns),
                Some(&json!({"name": format!("later-{language}"), "language": language, "source": "x"})),
            )
            .await;
        assert_eq!(status, 201, "{language}: {body}");
    }
}
