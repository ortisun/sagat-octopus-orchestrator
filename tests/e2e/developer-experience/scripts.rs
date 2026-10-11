//! `sagat script test|publish` against a platform with a sandbox runner (`FR-012-001`). The runner is
//! the production code; what runs the code is a stand-in for the container engine in most of these
//! (the engine is not something every machine has), and the real thing in the one that needs it.

use std::time::Duration;

use sagat_application::sandbox::{
    BuildFailure, BuildRequest, Built, HostCalls, RunOutcome, RunRequest, Sandbox,
};
use sagat_domain::scripts::{Failure, Language};
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::scripts::{Lab, lab, lab_on};
use crate::support::worker::platform;
use crate::{Dir, Outcome, cli};

/// What the stand-in does with a run.
#[derive(Clone, Copy)]
enum Mode {
    /// The result is the input, wrapped; the log says it ran.
    Echoes,
    /// It is still running when its time runs out.
    Overruns,
}

/// A container engine that builds anything not marked `syntax error` and runs it as `Mode` says.
#[derive(Clone, Copy)]
struct Scripted(Mode);

impl Sandbox for Scripted {
    async fn toolchain(&self, _: Language) -> Result<String, BuildFailure> {
        Ok("scripted@sha256:00;shim=1".to_owned())
    }

    async fn build(&self, request: BuildRequest) -> Result<Built, BuildFailure> {
        if request.source.as_str().contains("syntax error") {
            return Err(BuildFailure::Rejected(
                "unexpected token at line 1".to_owned(),
            ));
        }
        Ok(Built {
            // The shape of a bundle the platform accepts: the language and the policy it was built for,
            // and the files; what runs them is the engine's business, and here the stand-in's.
            artifact: format!(
                r#"{{"v":1,"language":"{}","policy":"{}","entry":"main.mjs","files":{{"main.mjs":{}}}}}"#,
                request.language,
                request.policy.canonical(),
                json!(request.source.as_str())
            )
            .into_bytes(),
            sbom: b"{}".to_vec(),
            toolchain: "scripted@sha256:00;shim=1".to_owned(),
            log: "built by the stand-in".to_owned(),
        })
    }

    async fn run<H: HostCalls>(&self, request: RunRequest, _: &H) -> RunOutcome {
        let input: Value = serde_json::from_slice(&request.input).unwrap_or(Value::Null);
        let (result, log) = match self.0 {
            Mode::Echoes => (
                Ok(json!({ "echo": input }).to_string().into_bytes()),
                "ran in the stand-in\n".to_owned(),
            ),
            Mode::Overruns => (Err(Failure::Timeout), "still going\n".to_owned()),
        };
        RunOutcome {
            result,
            log,
            log_truncated: false,
            denied: Vec::new(),
            exit_code: Some(0),
            wall: Duration::from_millis(7),
            memory_peak_bytes: None,
            start: Duration::ZERO,
            inferred_input_schema: None,
            inferred_output_schema: None,
        }
    }
}

async fn script_cli<S: Sandbox + 'static>(lab: &Lab<S>, dir: &Dir, args: &[&str]) -> Outcome {
    cli(&lab.p, dir, args).await
}

/// The runner taking the builds and the test runs the way the executable does, until dropped.
struct Serving {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Serving {
    fn start<S: Sandbox + 'static>(lab: &Lab<S>) -> Self {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let runner = std::sync::Arc::clone(&lab.runner);
        let task = tokio::spawn(async move {
            runner
                .serve(async move {
                    let _ = stopped.await;
                })
                .await;
        });
        Self {
            stop: Some(stop),
            task,
        }
    }

    async fn end(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = (&mut self.task).await;
    }
}

#[tokio::test]
async fn a_script_is_tried_and_published_from_a_file_and_the_digest_pins_it_in_a_workflow() {
    let lab = lab_on(Options::default(), Scripted(Mode::Echoes), |_| {}).await;
    let serving = Serving::start(&lab);
    let dir = Dir::new();
    dir.write("enrich.ts", "export default (input) => ({ ...input });\n");

    // The first time it creates the script from the file's name; the result and the log come back.
    script_cli(
        &lab,
        &dir,
        &["script", "test", "enrich.ts", "--input", r#"{"n":1}"#],
    )
    .await
    .expect(0)
    .mentions("created script enrich (typescript)")
    .mentions("test run SUCCEEDED")
    .mentions(r#"result: {"echo":{"n":1}}"#)
    .mentions("log: ran in the stand-in");

    // Again with the same source it says nothing about the source; with another it replaces it.
    script_cli(
        &lab,
        &dir,
        &["script", "test", "enrich.ts", "--input", "{}"],
    )
    .await
    .expect(0)
    .omits("created script")
    .omits("replaced the source");
    dir.write(
        "enrich.ts",
        "export default (input) => ({ ...input, seen: true });\n",
    );
    script_cli(&lab, &dir, &["script", "test", "enrich.ts"])
        .await
        .expect(0)
        .mentions("replaced the source of script enrich");

    // A version is built, and the digest is what a workflow pins.
    let published = script_cli(&lab, &dir, &["script", "publish", "enrich.ts"]).await;
    published
        .expect(0)
        .mentions("script enrich version 1 is ready")
        .mentions("pin it in a workflow with the node");
    let digest = published
        .out
        .lines()
        .find_map(|l| l.strip_prefix("artifactDigest "))
        .expect("digest")
        .to_owned();
    assert!(
        digest.starts_with("sha256:") && digest.len() == 71,
        "{digest}"
    );
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "enrich-flow"}, "entrypoint": "enrich",
        "nodes": [{"id": "enrich", "type": "script", "language": "typescript", "artifactDigest": digest}]
    });
    dir.write(
        "flow.json",
        &serde_json::to_string_pretty(&doc).expect("json"),
    );
    script_cli(&lab, &dir, &["workflow", "validate", "flow.json"])
        .await
        .expect(0);
    script_cli(&lab, &dir, &["workflow", "publish", "flow.json"])
        .await
        .expect(0)
        .mentions("published  enrich-flow v1");

    // Python is the other language of the sandbox; the rest are refused before anything is sent.
    dir.write("score.py", "def main(input):\n    return input\n");
    script_cli(&lab, &dir, &["script", "test", "score.py"])
        .await
        .expect(0)
        .mentions("created script score (python)");
    script_cli(
        &lab,
        &dir,
        &["script", "test", "score.py", "--language", "go"],
    )
    .await
    .expect(1)
    .mentions("go scripts are not built yet");
    script_cli(
        &lab,
        &dir,
        &["script", "test", "enrich.ts", "--language", "python"],
    )
    .await
    .expect(1)
    .mentions("a script keeps its language");
    serving.end().await;
}

#[tokio::test]
async fn a_script_that_runs_out_of_time_and_a_source_that_does_not_build_say_what_to_do() {
    let lab = lab_on(Options::default(), Scripted(Mode::Overruns), |_| {}).await;
    let serving = Serving::start(&lab);
    let dir = Dir::new();
    dir.write("slow.ts", "export default () => { for (;;) {} };\n");
    script_cli(&lab, &dir, &["script", "test", "slow.ts"])
        .await
        .expect(1)
        .mentions("failure: SCRIPT_TIMEOUT")
        .mentions("help: the script ran out of time")
        .mentions("log: still going");

    dir.write("broken.ts", "syntax error\n");
    script_cli(&lab, &dir, &["script", "publish", "broken.ts"])
        .await
        .expect(1)
        .mentions("version 1 was not built");
    serving.end().await;
}

#[tokio::test]
async fn without_a_sandbox_runner_the_tool_waits_for_the_time_it_was_given_and_says_why() {
    // A platform with no runner: nothing takes the test run.
    let p = platform().await;
    let dir = Dir::new();
    dir.write("enrich.ts", "export default (input) => input;\n");
    let waited = cli(&p, &dir, &["script", "test", "enrich.ts", "--timeout", "2"]).await;
    waited.expect(6).mentions("no sandbox runner");
    let publishing = cli(
        &p,
        &dir,
        &["script", "publish", "enrich.ts", "--timeout", "2"],
    )
    .await;
    publishing.expect(6).mentions("no sandbox runner");
}

#[tokio::test]
#[ignore = "needs the container engine: the sdks job of CI runs it with --ignored"]
async fn with_the_real_sandbox_a_script_is_run_and_its_result_comes_back() {
    let lab = lab().await;
    let serving = Serving::start(&lab);
    let dir = Dir::new();
    dir.write(
        "double.ts",
        "export default function double(input: { n: number }) {\n  return { doubled: input.n * 2 };\n}\n",
    );
    script_cli(
        &lab,
        &dir,
        &[
            "script",
            "test",
            "double.ts",
            "--input",
            r#"{"n":21}"#,
            "--timeout",
            "120",
        ],
    )
    .await
    .expect(0)
    .mentions("test run SUCCEEDED")
    .mentions(r#"result: {"doubled":42}"#);
    dir.write(
        "greedy.ts",
        "export default async function (_input: unknown, octopus: any) {\n  await octopus.fetch('http://169.254.169.254/latest/meta-data/');\n  return {};\n}\n",
    );
    // The template's own script, as `sagat new inline-script` writes it.
    cli(&lab.p, &dir, &["new", "inline-script"]).await.expect(0);
    script_cli(
        &lab,
        &dir,
        &[
            "script",
            "test",
            "inline-script/enrich.ts",
            "--input",
            r#"{"orderId":"o-1","items":2}"#,
            "--timeout",
            "120",
        ],
    )
    .await
    .expect(0)
    .mentions("test run SUCCEEDED")
    .mentions(r#""express":true"#);
    // Nothing leaves the sandbox: the attempt is refused and listed.
    script_cli(
        &lab,
        &dir,
        &["script", "test", "greedy.ts", "--timeout", "120"],
    )
    .await
    .expect(1)
    .mentions("failure: SCRIPT_DENIED");
    serving.end().await;
}
