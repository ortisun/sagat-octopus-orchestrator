//! The SDK conformance suite (`FR-006-006`): the same scenarios, played by the SDK of each language
//! against a real platform, must give the same normalized results, equal to the golden files.
//! `README.md` in this directory is the contract with the SDKs.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod control;
mod normalize;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use sagat_application::worker::WorkerSettings;
use sagat_domain::identity::Role;
use serde_json::{Map, Value, json};
use tokio::process::Command;

use crate::support::api::Options;
use crate::support::worker::{
    HUMAN, NAMESPACE, Platform, RunningEngine, TENANT, engine_env, platform_with,
};
use normalize::{Normalizer, compared, first_difference};

/// (name used in `SAGAT_SDK_LANGS`, directory under `sdks/`)
const LANGUAGES: [(&str, &str); 5] = [
    ("typescript", "typescript"),
    ("go", "go"),
    ("python", "python"),
    ("dotnet", "dotnet"),
    ("java", "java"),
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contract")
        .join("sdk-conformance")
}

fn languages() -> Vec<(&'static str, PathBuf)> {
    let chosen: Option<Vec<String>> = std::env::var("SAGAT_SDK_LANGS").ok().map(|list| {
        list.split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    });
    LANGUAGES
        .iter()
        .filter(|(name, _)| {
            chosen
                .as_ref()
                .is_none_or(|list| list.iter().any(|l| l == name))
        })
        .map(|(name, dir)| (*name, repo().join("sdks").join(dir).join("conformance")))
        .collect()
}

async fn script(
    dir: &Path,
    name: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(String, String), String> {
    let path = dir.join(name);
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    let child = Command::new("bash")
        .arg(&path)
        .args(args)
        .current_dir(dir)
        // Not UTC, and not the zone of any CI machine: an SDK that formats or parses an instant in local
        // time shows here.
        .env("TZ", "Asia/Kolkata")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", path.display()))?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("{} {args:?} took longer than {timeout:?}", path.display()))?
        .map_err(|e| format!("{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{} {args:?} exited with {}:\n{}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn sorted_for_golden(value: &Value) -> Value {
    serde_json::from_str(&canonical(value)).expect("json")
}

fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|k| (k.clone(), sorted(&map[k])))
                        .collect::<Map<_, _>>(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sorted(value)).expect("json")
}

/// A platform of one scenario and one language: nothing a language did is left for the next to find.
struct Run {
    platform: Arc<Platform>,
    /// The execution engine, started the way the binary is: worker API, scheduler, event router, outbox.
    engine: RunningEngine,
    control: String,
    client_token: String,
    worker_token: String,
    other_token: String,
    /// A worker identity with no binding: the platform refuses to register its activities.
    unbound_token: String,
    /// The control endpoint, which holds the platform until it is stopped.
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Run {
    /// Stops the engine (it finishes what it holds) and the control endpoint.
    async fn stop(mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = tokio::time::timeout(Duration::from_secs(25), self.engine.finish()).await;
    }
}

fn token(platform: &Platform, subject: &str, tenant: &str) -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        + 3500;
    platform
        .api
        .idp
        .token(subject, tenant)
        .claim("exp", json!(exp))
        .build()
}

async fn boot(scenario: &Value) -> Run {
    let lease = scenario["server"]["leaseTtlSeconds"].as_u64().unwrap_or(30);
    let platform = platform_with(Options {
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(lease),
            ..WorkerSettings::default()
        },
        ..Options::default()
    })
    .await;
    platform
        .grant(platform.tenant, "conformance-worker", Role::Worker)
        .await;
    platform.bind_everything("conformance-worker").await;
    platform
        .grant(platform.tenant, "conformance-unbound", Role::Worker)
        .await;
    // The engine the worker protocol is served by: timers, retries, leases that ran out and the wake-ups
    // (also the events that release a wait) are its loops, as in production.
    let ring = format!("k1:{}", platform.api.key_k1);
    let lease_secs = lease.to_string();
    let engine = RunningEngine::start(engine_env(
        &platform.api.db,
        &platform.api.idp,
        &[
            ("SAGAT_SECRETS_KEYS", ring.as_str()),
            ("SAGAT_LEASE_TTL_SECS", lease_secs.as_str()),
            ("SAGAT_EVENT_POLL_MS", "50"),
            ("SAGAT_DELIVERY_POLL_MS", "50"),
        ],
    ))
    .await;
    let platform = Arc::new(platform);
    let (control, serving) = control::serve(Arc::clone(&platform)).await;
    Run {
        client_token: token(&platform, HUMAN, TENANT),
        worker_token: token(&platform, "conformance-worker", TENANT),
        other_token: token(&platform, "bob", "globex"),
        unbound_token: token(&platform, "conformance-unbound", TENANT),
        control: format!("http://{control}"),
        platform,
        engine,
        tasks: vec![serving],
    }
}

async fn publish(run: &Run, name: &str, ir: &Value) -> Value {
    let (status, created) = run
        .platform
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": name, "ir": ir})),
        )
        .await;
    assert_eq!(status, 201, "creating {name}: {created}");
    let id = created["id"].as_str().expect("id").to_owned();
    let (status, published) = run
        .platform
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 201, "publishing {name}: {published}");
    let version = published["versionId"]
        .as_str()
        .or_else(|| published["id"].as_str())
        .unwrap_or_default()
        .to_owned();
    json!({"workflowId": id, "versionId": version, "name": name})
}

/// What the platform holds of every run once a language has played, read over REST without the SDK: the
/// effect of what the SDK did (the status of each node, the attempts, the code a failure was recorded with).
async fn server_state(run: &Run) -> Result<Value, String> {
    let (status, page) = run
        .platform
        .rest(Method::GET, "/runs?limit=200", None)
        .await;
    if status != 200 {
        return Err(format!("listing the runs answered {status}: {page}"));
    }
    let mut runs = Vec::new();
    for item in page["items"].as_array().cloned().unwrap_or_default() {
        let id = item["runId"]
            .as_str()
            .ok_or("a run without an id")?
            .to_owned();
        // The graph is a projection of the journal: it is read once it has caught up with the run.
        let mut graph = Value::Null;
        for _ in 0..100 {
            let (status, found) = run
                .platform
                .rest(Method::GET, &format!("/runs/{id}/graph"), None)
                .await;
            if status == 200 && found["status"] == item["status"] {
                graph = found;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if graph.is_null() {
            return Err(format!(
                "the graph of run {id} did not reach the status {}",
                item["status"]
            ));
        }
        let nodes: Vec<Value> = graph["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|node| {
                json!({
                    "id": node["id"], "type": node["type"], "kind": node["kind"], "forward": node["forward"],
                    "status": node["status"], "attempt": node["attempt"], "failureCode": node["failureCode"],
                    "waitingOn": node["waitingOn"], "cancelRequested": node["cancelRequested"],
                })
            })
            .collect();
        runs.push(json!({"runId": id, "status": item["status"], "paused": item["paused"], "nodes": nodes}));
    }
    Ok(Value::Array(runs))
}

/// The places in a runner's observations where an error repeats something it must not: a token of the run
/// or a text the scenario marked `mustNotLeak` (a payload it sent, say). The message is not compared with
/// the other SDKs', so this is where its content is held to the contract.
fn leaks(out: &Value, config: &Value, scenario: &Value) -> Vec<String> {
    let mut secrets: Vec<String> = ["clientToken", "workerToken", "otherTenantToken", "badToken"]
        .iter()
        .filter_map(|key| config[*key].as_str().map(str::to_owned))
        .collect();
    secrets.extend(
        scenario["mustNotLeak"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned)),
    );
    let mut found = Vec::new();
    for observation in out["observations"].as_array().into_iter().flatten() {
        let Some(error) = observation["error"].as_object() else {
            continue;
        };
        for (field, value) in error {
            let Some(text) = value.as_str() else { continue };
            for secret in &secrets {
                if !secret.is_empty() && text.contains(secret.as_str()) {
                    found.push(format!(
                        "step {}: the error's {field} repeats \"{}…\"",
                        observation["id"],
                        secret.chars().take(12).collect::<String>()
                    ));
                }
            }
        }
    }
    found
}

/// The instants of a Run an SDK hands back must be UTC with a `Z`, whatever the zone it runs in.
fn instants_not_utc(out: &Value) -> Vec<String> {
    fn walk(id: &Value, value: &Value, found: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    if matches!(
                        key.as_str(),
                        "createdAt" | "updatedAt" | "finishedAt" | "deadline"
                    ) && let Some(text) = item.as_str()
                        && !text.ends_with('Z')
                    {
                        found.push(format!("step {id}: {key} is \"{text}\", not UTC with a Z"));
                    } else {
                        walk(id, item, found);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(id, item, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for observation in out["observations"].as_array().into_iter().flatten() {
        walk(&observation["id"], &observation["result"], &mut found);
    }
    found
}

/// The traces a scenario puts in its steps, which the SDKs must carry as they are.
fn given_traces(scenario: &Value) -> Vec<String> {
    let mut found = Vec::new();
    fn walk(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(text) => {
                if let Some(rest) = text.strip_prefix("00-")
                    && let Some(trace) = rest.get(..32)
                {
                    found.push(trace.to_owned());
                }
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, found)),
            Value::Object(map) => map.values().for_each(|v| walk(v, found)),
            _ => {}
        }
    }
    walk(&scenario["steps"], &mut found);
    found
}

struct Outcome {
    failures: Vec<String>,
}

async fn play(
    path: &Path,
    langs: &[(&'static str, PathBuf)],
    bless: bool,
    out_dir: &Path,
) -> Outcome {
    let scenario: Value = serde_json::from_slice(&std::fs::read(path).expect("read the scenario"))
        .expect("scenario JSON");
    let name = scenario["name"].as_str().expect("name").to_owned();
    let mut failures = Vec::new();
    let timeout = Duration::from_secs(300);
    let dir = out_dir.join(&name);
    std::fs::create_dir_all(&dir).expect("output directory");

    // 1. The builders: every SDK writes the same documents.
    let mut built: Vec<(&str, Value)> = Vec::new();
    for (lang, root) in langs {
        match script(
            root,
            "run.sh",
            &["build", path.to_str().expect("path")],
            timeout,
        )
        .await
        {
            Ok((text, _)) => match serde_json::from_str::<Value>(&text) {
                Ok(doc) => built.push((lang, doc)),
                Err(e) => failures.push(format!(
                    "{name}/{lang}: build printed something that is not JSON ({e}): {text}"
                )),
            },
            Err(e) => failures.push(format!("{name}/{lang}: build failed: {e}")),
        }
    }
    let Some((first_lang, first)) = built.first().cloned() else {
        return Outcome { failures };
    };
    for (lang, doc) in &built[1..] {
        if canonical(doc) != canonical(&first) {
            let at = first_difference(&first, doc).unwrap_or_default();
            failures.push(format!(
                "{name}: the builders of {first_lang} and {lang} disagree at {at}"
            ));
        }
    }
    if !failures.is_empty() {
        return Outcome { failures };
    }

    // What they built is also the golden document: five SDKs do not drift the same way unnoticed.
    if first
        .get("workflows")
        .is_some_and(|w| w.as_object().is_some_and(|o| !o.is_empty()))
    {
        let golden = suite()
            .join("expected")
            .join(format!("{name}.workflows.json"));
        if bless {
            std::fs::write(
                &golden,
                serde_json::to_vec_pretty(&sorted_for_golden(&first)).expect("json"),
            )
            .expect("bless");
        } else {
            match std::fs::read(&golden)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                Some(expected) if expected == sorted_for_golden(&first) => {}
                Some(expected) => {
                    let at =
                        first_difference(&expected, &sorted_for_golden(&first)).unwrap_or_default();
                    failures.push(format!(
                        "{name}: the documents the builders write differ from {} at {at}",
                        golden.display()
                    ));
                }
                None => failures.push(format!(
                    "{name}: {} does not exist (SAGAT_SDK_BLESS=1 writes it)",
                    golden.display()
                )),
            }
        }
    }

    // 2. Play it, language by language, each on a platform of its own: the runs, the leases, the events
    //    and the quotas of one language are not there for the next to meet.
    let expected_path = suite().join("expected").join(format!("{name}.json"));
    let mut results: Vec<(&str, Value)> = Vec::new();
    for (lang, root) in langs {
        let run = boot(&scenario).await;
        // The secrets a scenario's workflows declare must exist before they can be published.
        if let Some(secrets) = scenario["secrets"].as_object() {
            let namespace = run.platform.namespace_id().await;
            for (secret, value) in secrets {
                let (status, stored) = run
                    .platform
                    .rest(
                        Method::PUT,
                        &format!("/namespaces/{namespace}/secrets/{secret}"),
                        Some(&json!({"value": value})),
                    )
                    .await;
                assert_eq!(status, 200, "storing the secret {secret}: {stored}");
            }
        }
        let mut workflows = Map::new();
        if let Some(docs) = first["workflows"].as_object() {
            for (key, ir) in docs {
                let workflow_name = ir["metadata"]["name"]
                    .as_str()
                    .expect("workflow name")
                    .to_owned();
                workflows.insert(key.clone(), publish(&run, &workflow_name, ir).await);
            }
        }
        let unique = format!(
            "{lang}-{}",
            &uuid::Uuid::now_v7().simple().to_string()[20..]
        );
        let config = json!({
            "apiUrl": run.platform.api.base, "grpcUrl": run.engine.grpc_url, "namespace": NAMESPACE,
            "clientToken": run.client_token, "workerToken": run.worker_token, "otherTenantToken": run.other_token,
            "unboundWorkerToken": run.unbound_token, "badToken": "not-a-jwt", "controlUrl": run.control, "unique": unique, "workflows": workflows,
            "leaseTtlSeconds": scenario["server"]["leaseTtlSeconds"].as_u64().unwrap_or(30),
        });
        let config_path = dir.join(format!("{lang}.config.json"));
        let out_path = dir.join(format!("{lang}.out.json"));
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("json"),
        )
        .expect("write config");
        let _ = std::fs::remove_file(&out_path);
        match script(
            root,
            "run.sh",
            &[
                "run",
                path.to_str().expect("path"),
                config_path.to_str().expect("path"),
                out_path.to_str().expect("path"),
            ],
            timeout,
        )
        .await
        {
            Ok((_, stderr)) => match std::fs::read(&out_path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                Some(mut out) => {
                    for leak in leaks(&out, &config, &scenario)
                        .into_iter()
                        .chain(instants_not_utc(&out))
                    {
                        failures.push(format!("{name}/{lang}: {leak}"));
                    }
                    match server_state(&run).await {
                        Ok(state) => out["serverState"] = state,
                        Err(e) => failures.push(format!("{name}/{lang}: {e}")),
                    }
                    let mut normalizer = Normalizer::new(&unique, given_traces(&scenario));
                    let doc = compared(&out, &mut normalizer);
                    std::fs::write(
                        dir.join(format!("{lang}.normalized.json")),
                        serde_json::to_vec_pretty(&doc).expect("json"),
                    )
                    .expect("write");
                    results.push((lang, doc));
                }
                None => failures.push(format!(
                    "{name}/{lang}: the runner wrote no readable {} (stderr: {})",
                    out_path.display(),
                    stderr.trim()
                )),
            },
            Err(e) => failures.push(format!("{name}/{lang}: run failed: {e}")),
        }
        run.stop().await;
    }

    // 3. Compare: with each other, and with the golden file.
    if let Some((first_lang, first)) = results.first().cloned() {
        for (lang, doc) in &results[1..] {
            if doc != &first {
                let at = first_difference(&first, doc).unwrap_or_default();
                failures.push(format!(
                    "{name}: {first_lang} and {lang} differ at {at} (see {})",
                    dir.display()
                ));
            }
        }
        if bless {
            std::fs::write(
                &expected_path,
                serde_json::to_vec_pretty(&first).expect("json"),
            )
            .expect("bless");
            println!("blessed {} from {first_lang}", expected_path.display());
        } else {
            match std::fs::read(&expected_path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            {
                Some(expected) => {
                    for (lang, doc) in &results {
                        if doc != &expected {
                            let at = first_difference(&expected, doc).unwrap_or_default();
                            failures.push(format!(
                                "{name}/{lang}: differs from expected/{name}.json at {at}"
                            ));
                        }
                    }
                }
                None => failures.push(format!(
                    "{name}: {} does not exist (SAGAT_SDK_BLESS=1 writes it)",
                    expected_path.display()
                )),
            }
        }
    }
    Outcome { failures }
}

/// The test owns its runtime so that it can give it up after a few seconds: a runtime that is dropped
/// after a failure waits for ever for the blocking threads of the platform it started.
#[test]
fn every_sdk_conforms_to_every_scenario() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.block_on(conform())));
    runtime.shutdown_timeout(Duration::from_secs(10));
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

async fn conform() {
    let langs = languages();
    let all_five = std::env::var("SAGAT_SDK_LANGS").is_err();
    let mut failures = Vec::new();
    if all_five {
        for (name, dir) in LANGUAGES {
            let run = repo()
                .join("sdks")
                .join(dir)
                .join("conformance")
                .join("run.sh");
            if !run.exists() {
                failures.push(format!(
                    "the {name} SDK has no conformance runner ({})",
                    run.display()
                ));
            }
        }
    }
    let langs: Vec<_> = langs
        .into_iter()
        .filter(|(_, root)| root.join("run.sh").exists())
        .collect();
    assert!(
        !langs.is_empty(),
        "no SDK has a conformance runner: {failures:?}"
    );

    if std::env::var("SAGAT_SDK_REUSE").is_err() {
        for (lang, root) in &langs {
            if let Err(e) = script(root, "prepare.sh", &[], Duration::from_secs(1800)).await {
                failures.push(format!("{lang}: prepare failed: {e}"));
            }
        }
    }

    let wanted = std::env::var("SAGAT_SDK_SCENARIO").ok();
    let bless = std::env::var("SAGAT_SDK_BLESS").is_ok();
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("sdk-conformance");
    let mut files: Vec<PathBuf> = std::fs::read_dir(suite().join("scenarios"))
        .expect("scenarios")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut played = 0;
    for file in files {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if wanted.as_ref().is_some_and(|w| *w != stem) {
            continue;
        }
        println!("--- {stem} ({} languages)", langs.len());
        let outcome = play(&file, &langs, bless, &out_dir).await;
        played += 1;
        for failure in &outcome.failures {
            println!("FAIL {failure}");
        }
        failures.extend(outcome.failures);
    }
    assert!(played > 0, "no scenario was played");
    assert!(
        failures.is_empty(),
        "the SDKs do not conform:\n{}",
        failures.join("\n")
    );
}
