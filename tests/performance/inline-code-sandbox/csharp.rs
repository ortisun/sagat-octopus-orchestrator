//! Explicit C# measurements; image pulls and platform/database calls are outside this benchmark.

use super::{ms, row, sh};
use sagat_application::sandbox::{BuildRequest, RunRequest, Sandbox};
use sagat_domain::scripts::{Language, Lockfile, Policy, Source};
use sagat_infra_sandbox::{DockerSandbox, Images};
use sagat_sandbox_runner::host::ScriptHost;
use std::time::Instant;

const SAMPLES: usize = 10;
const LOCK: &str = include_str!("../../../runners/dotnet/Catalog/packages.lock.json");
const SOURCE: &str = r#"
using System.Threading;
using System.Threading.Tasks;
using System.Text.Json;
using Sagat.ScriptContract;
public sealed class Script : IInlineScript {
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken) {
        return ValueTask.FromResult(JsonSerializer.SerializeToElement(new { answer = 42 }));
    }
}
"#;

#[tokio::test]
#[ignore = "real C# container benchmark: run explicitly in release mode"]
async fn csharp_build_and_execution_benchmark() {
    assert!(!cfg!(debug_assertions), "run this benchmark with --release");
    let mut report = format!(
        "# C# sandbox measurements\n\nDate: {}\n\n- Host: {}\n- Commit: {} (working-tree changes may be present)\n- Docker: {}\n- Images: {}\n- Load at start: {}\n- Rust profile: release\n- Policy: {:?}\n\nEach sample creates a new isolated container. Cold means the first sample in this process with images already present; warm samples reuse image layers, not containers or compiled results. Build measurements include compilation and artifact preparation in the sandbox adapter, but not platform signing, REST, PostgreSQL or build-cache lookup. Runtime measurements include creation, loading, execution and removal. This is a small local sample, not a production SLO.\n\n",
        sh("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
        sh("uname", &["-srm"]),
        sh("git", &["rev-parse", "HEAD"]),
        sh("docker", &["version", "--format", "{{.Server.Version}}"]),
        sh(
            "docker",
            &[
                "image",
                "inspect",
                "sagat/dotnet-builder:10.0.401",
                "sagat/dotnet-runtime:10.0.12",
                "--format",
                "{{.RepoTags}} {{.Id}}"
            ]
        ),
        sh("uptime", &[]),
        Policy::default().limits,
    );
    for with_package in [false, true] {
        let sandbox = DockerSandbox::connect(Images::default()).expect("container engine");
        let source = if with_package {
            SOURCE.replace("answer = 42", "answer = Newtonsoft.Json.Linq.JObject.Parse(\"{\\\"answer\\\":42}\").Value<int>(\"answer\")")
        } else {
            SOURCE.to_owned()
        };
        let mut builds = Vec::new();
        let mut starts = Vec::new();
        let mut walls = Vec::new();
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let built = sandbox
                .build(BuildRequest {
                    language: Language::CSharp,
                    source: Source::parse(&source).expect("source"),
                    lockfile: with_package.then(|| Lockfile::parse(LOCK).expect("catalog lock")),
                    policy: Policy::default(),
                    input_schema: None,
                    output_schema: None,
                })
                .await
                .expect("C# compiles");
            builds.push(started.elapsed());
            let outcome = sandbox
                .run(
                    RunRequest {
                        language: Language::CSharp,
                        artifact: built.artifact,
                        input: b"{}".to_vec(),
                        input_schema: None,
                        output_schema: None,
                        policy: Policy::default(),
                        secrets_to_hide: Vec::new(),
                        idempotency_key: None,
                    },
                    &ScriptHost::closed(),
                )
                .await;
            let result = outcome.result.expect("C# executes");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&result).expect("JSON"),
                serde_json::json!({"answer":42})
            );
            starts.push(outcome.start);
            walls.push(outcome.wall);
        }
        assert_eq!(sandbox.leftovers().await, 0, "containers removed");
        let label = if with_package {
            "Newtonsoft.Json 13.0.4"
        } else {
            "framework only"
        };
        report.push_str(&format!("## {label}\n\nFirst build: {:.0} ms; first runtime start: {:.0} ms; first whole runtime: {:.0} ms.\n\n| Measurement | Samples | p50 | p95 | p99 | max |\n|---|---|---|---|---|---|\n", ms(builds[0]), ms(starts[0]), ms(walls[0])));
        for (name, samples) in [
            ("build", builds),
            ("runtime start", starts),
            ("whole runtime", walls),
        ] {
            report.push_str(&row(name, samples[1..].to_vec()).0);
        }
        report.push('\n');
    }
    println!("{report}");
    if let Ok(path) = std::env::var("SAGAT_CSHARP_PERF_REPORT") {
        std::fs::write(path, report).expect("write report");
    }
}
