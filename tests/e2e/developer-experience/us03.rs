//! US03 — the same example in every SDK (`US03-AC01`, `FR-012-002`): a developer copies the
//! hello-world of Go, C#, TypeScript, Python or Java, runs it against the platform, and the run
//! it makes has the contract and the trace the other SDKs' runs have.
//!
//! Each `examples/hello-world/<language>/` has two executables, as the conformance runners of the
//! SDKs have: `prepare.sh` (install the dependencies and build) and `run.sh` (serve the two
//! activities, start a run, wait for it). They are configured only through `SAGAT_SERVER`,
//! `SAGAT_GRPC`, `SAGAT_NAMESPACE` and `SAGAT_TOKEN`, and say the same three things:
//!
//! ```text
//! serving greeting.compose and greeting.deliver at protocol 1.2
//! run <id> started
//! run <id> SUCCEEDED
//! ```
//!
//! `#[ignore]` because it needs the five toolchains (Node, Python, Go, .NET and a JDK): the `sdks`
//! job of CI runs it with `--ignored`. `SAGAT_EXAMPLE_LANGS=go,python` runs some of them.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use reqwest::Method;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::support::worker::{Platform, platform};
use crate::{Dir, cli, engine, rest};

const LANGUAGES: [&str; 5] = ["typescript", "go", "python", "dotnet", "java"];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the repository")
        .to_path_buf()
}

fn selected() -> Vec<&'static str> {
    match std::env::var("SAGAT_EXAMPLE_LANGS") {
        Ok(list) => LANGUAGES
            .into_iter()
            .filter(|l| list.split(',').any(|s| s.trim() == *l))
            .collect(),
        Err(_) => LANGUAGES.to_vec(),
    }
}

pub struct Finished {
    pub code: Option<i32>,
    pub out: String,
    pub err: String,
}

/// The builds of the examples that share something (the SDK of a language that is built in its own
/// directory) take turns: two tests of this target build the TypeScript SDK.
static BUILDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Runs one of the scripts of an example, with the settings of the platform added to the
/// environment of the test (the toolchains need their own variables): the token is in the
/// environment of the child and never in an argument.
pub async fn script(
    example: &str,
    language: &str,
    name: &str,
    seconds: u64,
    setting: &[(&str, String)],
) -> Finished {
    let path = repo()
        .join("examples")
        .join(example)
        .join(language)
        .join(name);
    let _turn = if name == "prepare.sh" {
        Some(BUILDING.lock().await)
    } else {
        None
    };
    let mut command = Command::new(&path);
    command
        .current_dir(path.parent().expect("dir"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in setting {
        command.env(key, value);
    }
    let child = command
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let done = tokio::time::timeout(Duration::from_secs(seconds), child.wait_with_output())
        .await
        .unwrap_or_else(|_| panic!("{language}/{name} did not end within {seconds} s"))
        .expect("wait");
    Finished {
        code: done.status.code(),
        out: String::from_utf8_lossy(&done.stdout).into_owned(),
        err: String::from_utf8_lossy(&done.stderr).into_owned(),
    }
}

/// What the journal of a run says without the things that differ from one run to the next:
/// the type of each event, the step it concerns and the attempt, and the fingerprint of the output
/// it refers to when `with_outputs`.
async fn journal(p: &Platform, run: &str, with_outputs: bool) -> Vec<String> {
    let (status, body) = rest(
        p,
        Method::GET,
        &format!("/runs/{run}/events?limit=200"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|e| {
            let mut line = format!(
                "{} {} #{}",
                e["eventType"].as_str().unwrap_or("?"),
                e["stepId"].as_str().unwrap_or("-"),
                e["attempt"]
            );
            if with_outputs {
                line.push_str(&format!(" {}", e["payloadRef"].as_str().unwrap_or("-")));
            }
            line
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn line_of<'a>(out: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    out.lines()
        .find(|l| l.starts_with(prefix) && l.ends_with(suffix))
}

#[tokio::test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
async fn us03_the_hello_world_of_each_sdk_makes_the_same_run_as_the_others() {
    let languages = selected();
    assert!(
        !languages.is_empty(),
        "SAGAT_EXAMPLE_LANGS names no language"
    );
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();

    // The workflow every example serves, published as the quickstart says, and the identity of the
    // token allowed to serve its activities.
    cli(&p, &dir, &["new", "hello-world"]).await.expect(0);
    let file = "hello-world/workflow.json";
    cli(&p, &dir, &["workflow", "publish", file])
        .await
        .expect(0);
    cli(&p, &dir, &["worker", "bind", "--workflow", file])
        .await
        .expect(0)
        .mentions("bound alice to 2 activities in payments");

    // A token lives five minutes in the tests and a clean build of the later languages can take longer
    // than that on a slow machine: each language gets a fresh one, just before it runs.
    let settings = |token: &str| {
        vec![
            ("SAGAT_SERVER", p.api.base.clone()),
            ("SAGAT_GRPC", engine.grpc_url.clone()),
            ("SAGAT_NAMESPACE", "payments".to_owned()),
            ("SAGAT_TOKEN", token.to_owned()),
        ]
    };

    let mut runs: Vec<(&str, String)> = Vec::new();
    for language in &languages {
        let prepared = script(
            "hello-world",
            language,
            "prepare.sh",
            900,
            &settings("unused"),
        )
        .await;
        assert_eq!(
            prepared.code,
            Some(0),
            "{language} prepare.sh:\n{}\n{}",
            prepared.out,
            prepared.err
        );
        // Without a token the example says so and ends with the code of a usage error, in every language.
        let without = script("hello-world", language, "run.sh", 60, &settings("")).await;
        assert_eq!(
            without.code,
            Some(2),
            "{language}: {}{}",
            without.out,
            without.err
        );
        assert!(
            without.err.contains("SAGAT_TOKEN is not set"),
            "{language}: {}",
            without.err
        );
        let token = p.api.idp.token("alice", "acme").build();
        let ran = script("hello-world", language, "run.sh", 180, &settings(&token)).await;
        assert_eq!(
            ran.code,
            Some(0),
            "{language} run.sh:\n{}\n{}",
            ran.out,
            ran.err
        );
        // The example says what it did, in the same words in every language.
        let serving = line_of(
            &ran.out,
            "serving greeting.compose and greeting.deliver at protocol 1.",
            "",
        );
        assert!(serving.is_some(), "{language}:\n{}", ran.out);
        let started = ran
            .out
            .lines()
            .find_map(|l| l.strip_prefix("run ")?.strip_suffix(" started"))
            .unwrap_or_else(|| panic!("{language}: no 'run <id> started' in:\n{}", ran.out))
            .to_owned();
        assert!(
            line_of(&ran.out, &format!("run {started} SUCCEEDED"), "SUCCEEDED").is_some(),
            "{language}:\n{}",
            ran.out
        );
        // What compose made from the input of the run reached deliver through the platform.
        assert!(
            ran.out.lines().any(|l| l == "delivering Hello, Ada!"),
            "{language}: deliver did not get the greeting of compose:\n{}",
            ran.out
        );
        // A secret is not something an example prints.
        assert!(
            !ran.out.contains(&token) && !ran.err.contains(&token),
            "{language} printed the token"
        );
        runs.push((language, started));
    }

    // The platform holds one story, whichever SDK told it.
    let mut stories: Vec<(&str, Vec<String>, String)> = Vec::new();
    for (language, run) in &runs {
        let (status, body) = rest(&p, Method::GET, &format!("/runs/{run}"), None).await;
        assert_eq!(status, 200, "{language}: {body}");
        assert_eq!(body["status"], "SUCCEEDED", "{language}: {body}");
        let trace = body["traceId"].as_str().unwrap_or("");
        assert!(
            trace.len() == 32 && trace.chars().all(|c| c.is_ascii_hexdigit()),
            "{language}: the run has no trace: {body}"
        );
        // The trace of the run is the trace of every event of its journal.
        let (_, events) = rest(
            &p,
            Method::GET,
            &format!("/runs/{run}/events?limit=200"),
            None,
        )
        .await;
        for event in events["items"].as_array().expect("items") {
            assert_eq!(
                event["traceId"].as_str(),
                Some(trace),
                "{language}: an event is not in the trace of the run: {event}"
            );
        }
        stories.push((language, journal(&p, run, true).await, trace.to_owned()));
    }
    let (first_language, first, _) = &stories[0];
    assert!(
        first.len() >= 6,
        "a two-step run has a journal of its own: {first:?}"
    );
    for (language, story, _) in &stories[1..] {
        assert_eq!(
            story, first,
            "{language} made another journal than {first_language}"
        );
    }
    // And the journal refers to the outputs the example is meant to make, byte for byte: what the
    // platform keeps of an output is its digest.
    for output in [
        r#"{"greeting":"Hello, Ada!"}"#,
        r#"{"delivered":true,"greeting":"Hello, Ada!"}"#,
    ] {
        let digest = hex(&Sha256::digest(output.as_bytes()));
        assert!(
            first.iter().any(|line| line.ends_with(&digest)),
            "no event of the journal refers to {output} ({digest}): {first:?}"
        );
    }
    let mut traces: Vec<&String> = stories.iter().map(|(_, _, t)| t).collect();
    traces.sort();
    traces.dedup();
    assert_eq!(
        traces.len(),
        stories.len(),
        "each run has a trace of its own"
    );

    // And it is the story `sagat worker dev` tells: the same steps in the same order, whoever
    // answers them (the outputs are the mock file's, so they are left out).
    let serve = [
        "worker",
        "dev",
        "--mocks",
        "hello-world/mocks.yaml",
        "--workflow",
        file,
        "--grpc",
        &engine.grpc_url,
        "--max-tasks",
        "2",
        "--idle-exit",
        "40",
    ];
    let start = [
        "workflow",
        "run",
        "hello-world",
        "--input",
        r#"{"name":"Ada"}"#,
        "--wait",
        "--timeout",
        "60",
    ];
    let (served, ran) = tokio::join!(cli(&p, &dir, &serve), cli(&p, &dir, &start));
    served.expect(0);
    ran.expect(0);
    let reference = journal(&p, &ran.run_id(), false).await;
    let shape: Vec<String> = first
        .iter()
        .map(|l| l.rsplit_once(' ').map_or(l.clone(), |(a, _)| a.to_owned()))
        .collect();
    assert_eq!(
        shape, reference,
        "an SDK worker and `worker dev` made different journals"
    );
}
