use std::time::Duration;

use sagat_application::sandbox::{BuildFailure, BuildRequest, RunRequest, Sandbox};
use sagat_domain::scripts::{Language, Lockfile, Policy, Source};
use serde_json::Value;

use super::{Host, quick, sandbox};

const STUDIO_LANGUAGES: &str =
    include_str!("../../../../frontend/apps/studio/src/features/scripts/languages.ts");
const CYCLONEDX_1_5_SCHEMA: &str =
    include_str!("../../../contract/inline-code-sandbox/cyclonedx-bom-1.5.schema.json");
const CYCLONEDX_JSF_SCHEMA: &str =
    include_str!("../../../contract/inline-code-sandbox/cyclonedx-jsf-0.82.schema.json");
const CYCLONEDX_SPDX_SCHEMA: &str =
    include_str!("../../../contract/inline-code-sandbox/cyclonedx-spdx.schema.json");

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

const FRAMEWORK_SOURCE: &str = r#"
using System.Text.Json;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
    {
        var result = new { answer = input.GetProperty("n").GetInt32() + 1, key = octopus.IdempotencyKey };
        return ValueTask.FromResult(JsonSerializer.SerializeToElement(result));
    }
}
"#;

async fn build_csharp(
    sandbox: &sagat_infra_sandbox::DockerSandbox,
    source: &str,
    lockfile: Option<&str>,
) -> Result<sagat_application::sandbox::Built, BuildFailure> {
    sandbox
        .build(BuildRequest {
            language: Language::CSharp,
            source: Source::parse(source).expect("source"),
            lockfile: lockfile.map(|text| Lockfile::parse(text).expect("lockfile")),
            policy: Policy::default(),
            input_schema: None,
            output_schema: None,
        })
        .await
}

async fn run_csharp(
    sandbox: &sagat_infra_sandbox::DockerSandbox,
    artifact: Vec<u8>,
    input: &str,
    host: &Host,
    key: Option<&str>,
) -> sagat_application::sandbox::RunOutcome {
    sandbox
        .run(
            RunRequest {
                language: Language::CSharp,
                artifact,
                input: input.as_bytes().to_vec(),
                input_schema: None,
                output_schema: None,
                policy: Policy::default(),
                secrets_to_hide: Vec::new(),
                idempotency_key: key.map(str::to_owned),
            },
            host,
        )
        .await
}

#[tokio::test]
async fn roslyn_compiles_and_the_separate_dotnet_host_runs_a_framework_only_script() {
    let sandbox = sandbox();
    let built = build_csharp(&sandbox, FRAMEWORK_SOURCE, None)
        .await
        .expect("C# build");
    let bundle: Value = serde_json::from_slice(&built.artifact).expect("bundle");
    assert_eq!(bundle["v"], 2);
    assert_eq!(bundle["entry"], "script.dll");
    assert_eq!(bundle["encoding"], "base64");
    assert!(built.toolchain.contains("build=") && built.toolchain.contains("runtime="));

    let outcome = run_csharp(
        &sandbox,
        built.artifact,
        r#"{"n": 41}"#,
        &Host::default(),
        Some("run-1:node-1"),
    )
    .await;
    let result: Value = serde_json::from_slice(&outcome.result.expect("result")).expect("JSON");
    assert_eq!(
        result,
        serde_json::json!({"answer": 42, "key": "run-1:node-1"})
    );
    assert_eq!(sandbox.leftovers().await, 0);
}

#[tokio::test]
async fn the_studio_csharp_starter_compiles_and_runs_unchanged() {
    let source = STUDIO_LANGUAGES
        .split_once("  csharp: `")
        .and_then(|(_, rest)| rest.split_once("\n`,"))
        .map(|(source, _)| source)
        .expect("Studio C# starter source");
    let sandbox = sandbox();
    let built = build_csharp(&sandbox, source, None)
        .await
        .expect("Studio starter builds");
    let outcome = run_csharp(
        &sandbox,
        built.artifact,
        r#"{"n": 21}"#,
        &Host::default(),
        None,
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&outcome.result.expect("result")).expect("JSON"),
        serde_json::json!({"double": 42})
    );
}

#[tokio::test]
async fn an_allowlisted_nuget_reference_is_compiled_embedded_and_loaded_without_a_registry() {
    let sandbox = sandbox();
    let source = r#"
using System.Text.Json;
using Newtonsoft.Json.Linq;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
    {
        var value = JObject.Parse(input.GetRawText()).Value<int>("n");
        return ValueTask.FromResult(JsonSerializer.SerializeToElement(new { answer = value + 1 }));
    }
}
"#;
    let built = build_csharp(&sandbox, source, Some(LOCK))
        .await
        .expect("allowlisted package build");
    let bundle: Value = serde_json::from_slice(&built.artifact).expect("bundle");
    assert!(bundle["files"]["script.dll"].as_str().is_some());
    assert!(bundle["files"]["Newtonsoft.Json.dll"].as_str().is_some());
    let sbom: Value = serde_json::from_slice(&built.sbom).expect("SBOM");
    let sbom_schema: Value = serde_json::from_str(CYCLONEDX_1_5_SCHEMA).expect("CycloneDX schema");
    let jsf_schema: Value = serde_json::from_str(CYCLONEDX_JSF_SCHEMA).expect("JSF schema");
    let spdx_schema: Value = serde_json::from_str(CYCLONEDX_SPDX_SCHEMA).expect("SPDX schema");
    let registry = jsonschema::Registry::new()
        .add(
            "http://cyclonedx.org/schema/jsf-0.82.schema.json",
            jsf_schema,
        )
        .expect("JSF schema URI")
        .add("http://cyclonedx.org/schema/spdx.schema.json", spdx_schema)
        .expect("SPDX schema URI")
        .prepare()
        .expect("JSF schema registry");
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&sbom_schema)
        .expect("official CycloneDX 1.5 schema compiles");
    let errors: Vec<_> = validator
        .iter_errors(&sbom)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "invalid CycloneDX 1.5 SBOM: {errors:?}");
    let components = sbom["components"].as_array().expect("components");
    let package = components
        .iter()
        .find(|component| component["name"] == "Newtonsoft.Json")
        .expect("NuGet package component");
    assert_eq!(package["version"], "13.0.4");
    assert_eq!(
        package["hashes"][0],
        serde_json::json!({
            "alg": "SHA-512",
            "content": "6d1faff84ff227a83b195dae5f0d8ead44a36187e32e438b0bc243e24092db79ac2daa672ad7493c1240ef97f01c7fbe12b21f7de22acd82132f102eaf82805c"
        })
    );
    assert_eq!(
        package["properties"][0],
        serde_json::json!({"name": "nuget:contentHash", "value": "pdgNNMai3zv51W5aq268sujXUyx7SNdE2bj1wZcWjAQrKMFZV260lbqYop1d2GM67JI1huLRwxo9ZqnfF/lC6A=="})
    );
    let assembly = components
        .iter()
        .find(|component| component["name"] == "Newtonsoft.Json.dll")
        .expect("managed assembly component");
    assert_eq!(
        assembly["hashes"][0],
        serde_json::json!({
            "alg": "SHA-256",
            "content": "a28c251dfe36d881e9e2462e171441b8b0ec156fe3f452602c9149b1b9efe05b"
        })
    );

    let outcome = run_csharp(
        &sandbox,
        built.artifact,
        r#"{"n": 41}"#,
        &Host::default(),
        None,
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&outcome.result.expect("result")).expect("JSON"),
        serde_json::json!({"answer": 42})
    );
}

#[tokio::test]
async fn floating_tampered_and_unknown_nuget_references_are_rejected_before_a_build() {
    let sandbox = sandbox();
    for (what, lockfile) in [
        ("floating", LOCK.replace("[13.0.4, 13.0.4]", "[13.0.4, )")),
        ("tampered", LOCK.replace("pdgNNM", "xxxxxx")),
        ("unknown", LOCK.replace("Newtonsoft.Json", "Other.Package")),
    ] {
        assert!(
            matches!(
                build_csharp(&sandbox, FRAMEWORK_SOURCE, Some(&lockfile)).await,
                Err(BuildFailure::Lockfile(_))
            ),
            "accepted {what} lockfile"
        );
    }
    assert_eq!(
        sandbox.leftovers().await,
        0,
        "no rejected build starts a container"
    );
}

#[tokio::test]
async fn csharp_has_only_supervised_host_calls_and_no_direct_metadata_network() {
    let sandbox = sandbox();
    let source = r#"
using System.Net.Http;
using System.Text.Json;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public async ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
    {
        var secret = await octopus.SecretAsync("allowed", cancellationToken);
        var reply = await octopus.FetchAsync("https://allowed.example/data", cancellationToken: cancellationToken);
        var metadataBlocked = false;
        try
        {
            using var client = new HttpClient();
            await client.GetStringAsync("http://169.254.169.254/latest/meta-data/", cancellationToken);
        }
        catch (HttpRequestException)
        {
            metadataBlocked = true;
        }
        return JsonSerializer.SerializeToElement(new { secret, body = reply.Body, metadataBlocked });
    }
}
"#;
    let built = build_csharp(&sandbox, source, None).await.expect("build");
    let host = Host {
        allow: vec![("https://allowed.example/data", "from host")],
        known_secrets: vec![("allowed", "not-for-logs")],
        ..Host::default()
    };
    let outcome = run_csharp(&sandbox, built.artifact, "{}", &host, None).await;
    let result: Value = serde_json::from_slice(&outcome.result.expect("result")).expect("JSON");
    assert_eq!(result["body"], "from host");
    assert_eq!(result["secret"], "[REDACTED]");
    assert_eq!(result["metadataBlocked"], true);
    assert_eq!(host.fetched.lock().expect("fetch lock").len(), 1);
    assert_eq!(
        host.secrets.lock().expect("secret lock").as_slice(),
        ["allowed"]
    );
}

#[tokio::test]
async fn a_csharp_cpu_burn_is_stopped_by_the_same_container_quota() {
    let sandbox = sandbox();
    let source = r#"
using System.Text.Json;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
    {
        while (true) { }
    }
}
"#;
    let built = build_csharp(&sandbox, source, None).await.expect("build");
    let outcome = sandbox
        .run(
            RunRequest {
                language: Language::CSharp,
                artifact: built.artifact,
                input: b"{}".to_vec(),
                input_schema: None,
                output_schema: None,
                policy: quick(Duration::from_secs(1), 128),
                secrets_to_hide: Vec::new(),
                idempotency_key: None,
            },
            &Host::default(),
        )
        .await;
    assert_eq!(outcome.result, Err(sagat_domain::scripts::Failure::Timeout));
    assert_eq!(sandbox.leftovers().await, 0);
}

#[tokio::test]
async fn csharp_result_framing_keeps_html_and_unicode_compact_but_enforces_the_real_limit() {
    let sandbox = sandbox();
    let source = r#"
using System.Linq;
using System.Text.Json;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
    {
        var length = input.GetProperty("length").GetInt32();
        var text = input.TryGetProperty("emoji", out _)
            ? string.Concat(Enumerable.Repeat("😀", length))
            : input.TryGetProperty("delete", out _)
                ? new string('\u007f', length)
                : new string('<', length) + "😀";
        return ValueTask.FromResult(JsonSerializer.SerializeToElement(text));
    }
}
"#;
    let built = build_csharp(&sandbox, source, None).await.expect("build");

    let below = run_csharp(
        &sandbox,
        built.artifact.clone(),
        r#"{"length":45000}"#,
        &Host::default(),
        None,
    )
    .await;
    let below: Value =
        serde_json::from_slice(&below.result.expect("compact result")).expect("JSON");
    let below = below.as_str().expect("string result");
    assert_eq!(below.chars().count(), 45_001);
    assert!(below.starts_with('<') && below.ends_with('😀'));

    let astral = run_csharp(
        &sandbox,
        built.artifact.clone(),
        r#"{"length":30000,"emoji":true}"#,
        &Host::default(),
        None,
    )
    .await;
    let astral: Value =
        serde_json::from_slice(&astral.result.expect("astral result")).expect("JSON");
    let astral = astral.as_str().expect("string result");
    assert_eq!(astral.chars().count(), 30_000);
    assert!(astral.chars().all(|character| character == '😀'));

    let delete = run_csharp(
        &sandbox,
        built.artifact.clone(),
        r#"{"length":45000,"delete":true}"#,
        &Host::default(),
        None,
    )
    .await;
    let delete: Value = serde_json::from_slice(&delete.result.expect("DEL result")).expect("JSON");
    let delete = delete.as_str().expect("string result");
    assert_eq!(delete.chars().count(), 45_000);
    assert!(delete.chars().all(|character| character == '\u{7f}'));

    let excess = run_csharp(
        &sandbox,
        built.artifact,
        r#"{"length":250000}"#,
        &Host::default(),
        None,
    )
    .await;
    assert_eq!(
        excess.result,
        Err(sagat_domain::scripts::Failure::OutputLimit)
    );
    assert_eq!(sandbox.leftovers().await, 0);
}
