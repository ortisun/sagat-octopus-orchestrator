//! The machinery of the SDK conformance suite (`FR-006-006`): the same scenarios, played by the SDK of
//! each language against a real platform, must give the same normalized results, equal to the golden
//! files. `README.md` in this directory is the contract with the SDKs. The conformance test, the end to
//! end tests and the fault tests of feature 006 all play scenarios through this module.
#![allow(clippy::expect_used, clippy::panic, dead_code)]

#[path = "control.rs"]
mod control;
#[path = "normalize.rs"]
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

/// The languages to play: `SAGAT_SDK_LANGS` (a comma separated list) and `only` both restrict them.
fn languages(only: Option<&[&str]>) -> Vec<(&'static str, PathBuf)> {
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
        .filter(|(name, _)| only.is_none_or(|list| list.contains(name)))
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
    // The tokens of the run appear nowhere in what a runner wrote (errors, observations, the worker log, ...).
    let tokens: Vec<String> = [
        "clientToken",
        "workerToken",
        "otherTenantToken",
        "unboundWorkerToken",
        "badToken",
    ]
    .iter()
    .filter_map(|key| config[*key].as_str().map(str::to_owned))
    .filter(|token| !token.is_empty())
    .collect();
    // A text the scenario marked (a payload it sent) must not come back in an error: it is legitimately in
    // the worker's log, which handled it.
    let markers: Vec<String> = scenario["mustNotLeak"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .filter(|marker| !marker.is_empty())
        .collect();

    fn walk(value: &Value, path: &str, found: &mut Vec<(String, String)>) {
        match value {
            Value::String(text) => found.push((path.to_owned(), text.clone())),
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, v)| walk(v, &format!("{path}[{i}]"), found)),
            Value::Object(map) => map
                .iter()
                .for_each(|(k, v)| walk(v, &format!("{path}.{k}"), found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    let mut everything = Vec::new();
    walk(out, "$", &mut everything);
    for (path, text) in &everything {
        for token in &tokens {
            if text.contains(token.as_str()) {
                found.push(format!(
                    "{path}: a token of the run is in the output (\"{}…\")",
                    token.chars().take(8).collect::<String>()
                ));
            }
        }
    }
    for observation in out["observations"].as_array().into_iter().flatten() {
        let Some(error) = observation["error"].as_object() else {
            continue;
        };
        for (field, value) in error {
            let Some(text) = value.as_str() else { continue };
            for marker in &markers {
                if text.contains(marker.as_str()) {
                    found.push(format!(
                        "step {}: the error's {field} repeats \"{}…\"",
                        observation["id"],
                        marker.chars().take(12).collect::<String>()
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

/// Writes a file that only its owner may read: a config holds the tokens of the run.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)?.write_all(bytes)
}

/// A platform of its own for one scenario and one language, the workflows the scenario's builders wrote
/// published in it, and the `config.json` the runner is given.
async fn stage(scenario: &Value, built: &Value, lang: &str) -> (Run, Value, String) {
    let run = boot(scenario).await;
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
    if let Some(docs) = built["workflows"].as_object() {
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
    (run, config, unique)
}

/// One language's normalized result of one scenario.
pub struct Played {
    pub scenario: String,
    pub language: &'static str,
    pub doc: Value,
}

struct Outcome {
    failures: Vec<String>,
    played: Vec<Played>,
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
    // The canonical text of every workflow is part of the build output and is compared byte for byte (SPEC §7).
    for (lang, doc) in &built {
        let has_workflows = doc["workflows"].as_object().is_some_and(|w| !w.is_empty());
        if has_workflows
            && doc["canonical"]
                .as_object()
                .is_none_or(|c| c.len() != doc["workflows"].as_object().map_or(0, Map::len))
        {
            failures.push(format!(
                "{name}/{lang}: build printed no canonical text (toJson()) for every workflow"
            ));
        }
    }
    if !failures.is_empty() {
        return Outcome {
            failures,
            played: Vec::new(),
        };
    }
    let Some((first_lang, first)) = built.first().cloned() else {
        return Outcome {
            failures,
            played: Vec::new(),
        };
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
        return Outcome {
            failures,
            played: Vec::new(),
        };
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
        let (run, config, unique) = stage(&scenario, &first, lang).await;
        // The process id is in the names: two plays of the suite at once (two people, a CI job and a
        // developer) do not overwrite each other's configuration and output.
        let pid = std::process::id();
        let config_path = dir.join(format!("{lang}.{pid}.config.json"));
        let out_path = dir.join(format!("{lang}.{pid}.out.json"));
        write_private(
            &config_path,
            &serde_json::to_vec_pretty(&config).expect("json"),
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
                        dir.join(format!("{lang}.{pid}.normalized.json")),
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
    let played = results
        .into_iter()
        .map(|(language, doc)| Played {
            scenario: name.clone(),
            language,
            doc,
        })
        .collect();
    Outcome { failures, played }
}

/// What to play: all scenarios and all languages unless narrowed (the environment narrows further).
pub struct Selection {
    scenarios: Option<Vec<String>>,
    languages: Option<Vec<String>>,
    bless: bool,
}

impl Selection {
    /// Everything the environment asks for: `SAGAT_SDK_SCENARIO`, `SAGAT_SDK_LANGS`, `SAGAT_SDK_BLESS`.
    pub fn from_env() -> Self {
        Self {
            scenarios: std::env::var("SAGAT_SDK_SCENARIO").ok().map(|w| vec![w]),
            languages: None,
            bless: std::env::var("SAGAT_SDK_BLESS").is_ok(),
        }
    }

    /// These scenarios, in every language the environment allows. Never blesses.
    pub fn scenarios(names: &[&str]) -> Self {
        Self {
            scenarios: Some(names.iter().map(|n| (*n).to_owned()).collect()),
            languages: None,
            bless: false,
        }
    }

    /// Only these languages (and only those the environment allows).
    pub fn only_languages(mut self, names: &[&str]) -> Self {
        self.languages = Some(names.iter().map(|n| (*n).to_owned()).collect());
        self
    }
}

/// What a play of the suite came to.
pub struct Suite {
    pub failures: Vec<String>,
    pub played: Vec<Played>,
}

impl Suite {
    pub fn assert_conforms(&self) {
        assert!(
            self.failures.is_empty(),
            "the SDKs do not conform:\n{}",
            self.failures.join("\n")
        );
    }

    /// The normalized result of one scenario in one language.
    pub fn doc(&self, scenario: &str, language: &str) -> &Value {
        &self
            .played
            .iter()
            .find(|p| p.scenario == scenario && p.language == language)
            .unwrap_or_else(|| panic!("{scenario} was not played in {language}"))
            .doc
    }

    /// The languages that played `scenario`.
    pub fn languages_of(&self, scenario: &str) -> Vec<&'static str> {
        self.played
            .iter()
            .filter(|p| p.scenario == scenario)
            .map(|p| p.language)
            .collect()
    }
}

/// The observation of a step in a normalized result.
pub fn observation<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["observations"]
        .as_array()
        .and_then(|all| all.iter().find(|o| o["id"] == id))
        .unwrap_or_else(|| panic!("no observation {id}"))
}

/// The entries of a worker's log in a normalized result.
pub fn worker_log<'a>(doc: &'a Value, worker: &str) -> &'a [Value] {
    doc["workerLog"][worker]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// A node of a run as the platform holds it (`serverState`), found by its id in any run of the result.
pub fn node<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["serverState"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|run| run["nodes"].as_array().into_iter().flatten())
        .find(|node| node["id"] == id)
        .unwrap_or_else(|| panic!("no node {id} in the state of the platform"))
}

/// The status of the run whose observation is `step` (its `runId`), as the platform holds it.
pub fn run_status<'a>(doc: &'a Value, step: &str) -> &'a Value {
    let id = &observation(doc, step)["result"]["runId"];
    doc["serverState"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|run| run["runId"] == *id)
        .map(|run| &run["status"])
        .unwrap_or_else(|| panic!("the run of {step} is not in the state of the platform"))
}

/// The types of the journal events an `events` step observed, in order.
pub fn journal(doc: &Value, step: &str) -> Vec<String> {
    observation(doc, step)["result"]["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| event["type"].as_str().map(str::to_owned))
        .collect()
}

/// Runs `future` on a runtime that is given up a few seconds after the test ends: a runtime that is
/// dropped after a failure waits for ever for the blocking threads of the platform it started.
pub fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.block_on(future)));
    runtime.shutdown_timeout(Duration::from_secs(10));
    match outcome {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Languages whose `prepare.sh` this process ran (tests of one process share the build of the SDKs).
static PREPARED: tokio::sync::Mutex<std::collections::BTreeSet<&'static str>> =
    tokio::sync::Mutex::const_new(std::collections::BTreeSet::new());

/// One play at a time in a process: scenarios space their actions in time, and two platforms booting and
/// running side by side would perturb each other's timing (tests of a process run in parallel threads).
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn conform(selection: &Selection) -> Suite {
    let _alone = ONE_AT_A_TIME.lock().await;
    let _database = crate::support::hold_server();
    let only: Option<Vec<&str>> = selection
        .languages
        .as_ref()
        .map(|l| l.iter().map(String::as_str).collect());
    let langs = languages(only.as_deref());
    let all_five = std::env::var("SAGAT_SDK_LANGS").is_err() && only.is_none();
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
        let mut prepared = PREPARED.lock().await;
        for (lang, root) in &langs {
            if prepared.contains(lang) {
                continue;
            }
            match script(root, "prepare.sh", &[], Duration::from_secs(1800)).await {
                Ok(_) => {
                    prepared.insert(lang);
                }
                Err(e) => failures.push(format!("{lang}: prepare failed: {e}")),
            }
        }
    }

    let bless = selection.bless;
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
    let mut played = Vec::new();
    let mut count = 0;
    for file in files {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if selection
            .scenarios
            .as_ref()
            .is_some_and(|w| !w.contains(&stem))
        {
            continue;
        }
        println!("--- {stem} ({} languages)", langs.len());
        let outcome = play(&file, &langs, bless, &out_dir).await;
        count += 1;
        for failure in &outcome.failures {
            println!("FAIL {failure}");
        }
        failures.extend(outcome.failures);
        played.extend(outcome.played);
    }
    if count == 0 {
        failures.push("no scenario was played".to_owned());
    }
    Suite { failures, played }
}

/// The languages that have a runner (and that `SAGAT_SDK_LANGS` allows).
pub fn available_languages() -> Vec<&'static str> {
    languages(None)
        .into_iter()
        .filter(|(_, root)| root.join("run.sh").exists())
        .map(|(name, _)| name)
        .collect()
}

/// Runs `prepare.sh` of these languages, once per process (unless `SAGAT_SDK_REUSE` is set); returns
/// what failed.
pub async fn ensure_prepared(names: &[&'static str]) -> Vec<String> {
    let mut failures = Vec::new();
    if std::env::var("SAGAT_SDK_REUSE").is_ok() {
        return failures;
    }
    let mut prepared = PREPARED.lock().await;
    for (lang, root) in languages(Some(names)) {
        if prepared.contains(lang) {
            continue;
        }
        match script(&root, "prepare.sh", &[], Duration::from_secs(1800)).await {
            Ok(_) => {
                prepared.insert(lang);
            }
            Err(e) => failures.push(format!("{lang}: prepare failed: {e}")),
        }
    }
    failures
}

/// What a runner did to the platform and how long it took, for the benchmark.
pub struct Measured {
    pub language: &'static str,
    /// The wall time of `run.sh run`: the runner's start, its steps and its end.
    pub wall: Duration,
    /// Calls the worker service answered while the runner ran, by method (all codes).
    pub calls: std::collections::BTreeMap<String, u64>,
    /// The same, by method and code, for the ones that did not end in OK.
    pub refused: std::collections::BTreeMap<String, u64>,
    /// The raw output of the runner.
    pub out: Value,
}

/// The `grpc_requests_total` counters of the engine, by (method, code).
async fn grpc_counters(run: &Run) -> std::collections::BTreeMap<(String, String), u64> {
    let text = run.engine.body("/metrics").await;
    let mut counters = std::collections::BTreeMap::new();
    for line in text
        .lines()
        .filter(|l| l.starts_with("grpc_requests_total{"))
    {
        let label = |name: &str| {
            line.split(&format!("{name}=\""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_owned()
        };
        let value = line
            .rsplit(' ')
            .next()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        counters.insert((label("method"), label("code")), value as u64);
    }
    counters
}

/// Plays a scenario file (any path, not only the suite's) in one language on a platform of its own and
/// measures it. Nothing is compared: this is for the benchmark.
pub async fn measure(path: &Path, language: &'static str) -> Result<Measured, String> {
    let _alone = ONE_AT_A_TIME.lock().await;
    let _database = crate::support::hold_server();
    let (_, root) = languages(Some(&[language]))
        .into_iter()
        .next()
        .ok_or_else(|| format!("{language} is not a language of the suite"))?;
    if !root.join("run.sh").exists() {
        return Err(format!("{language} has no conformance runner"));
    }
    let scenario: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let name = scenario["name"]
        .as_str()
        .ok_or("a scenario without a name")?
        .to_owned();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("sdk-measure")
        .join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path_arg = path.to_str().ok_or("path")?;
    let (text, _) = script(
        &root,
        "run.sh",
        &["build", path_arg],
        Duration::from_secs(300),
    )
    .await?;
    let built: Value = serde_json::from_str(&text)
        .map_err(|e| format!("build printed something that is not JSON: {e}"))?;
    let (run, config, _) = stage(&scenario, &built, language).await;
    let config_path = dir.join(format!("{language}.config.json"));
    let out_path = dir.join(format!("{language}.out.json"));
    write_private(
        &config_path,
        &serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out_path);
    let before = grpc_counters(&run).await;
    let started = std::time::Instant::now();
    let result = script(
        &root,
        "run.sh",
        &[
            "run",
            path_arg,
            config_path.to_str().ok_or("path")?,
            out_path.to_str().ok_or("path")?,
        ],
        Duration::from_secs(900),
    )
    .await;
    let wall = started.elapsed();
    let after = grpc_counters(&run).await;
    run.stop().await;
    result?;
    let out: Value = std::fs::read(&out_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("{language} wrote no readable {}", out_path.display()))?;
    let mut calls = std::collections::BTreeMap::new();
    let mut refused = std::collections::BTreeMap::new();
    for ((method, code), n) in &after {
        let delta = n - before
            .get(&(method.clone(), code.clone()))
            .copied()
            .unwrap_or(0);
        *calls.entry(method.clone()).or_insert(0) += delta;
        if !code.eq_ignore_ascii_case("ok") && delta > 0 {
            refused.insert(format!("{method}:{code}"), delta);
        }
    }
    Ok(Measured {
        language,
        wall,
        calls,
        refused,
        out,
    })
}

/// Plays an optional mode of a runner (`run.sh <mode> config out`, see the README) on a platform of its own,
/// for the scenario file at `path`, and returns what the runner wrote.
pub async fn run_mode(path: &Path, language: &'static str, mode: &str) -> Result<Value, String> {
    let _alone = ONE_AT_A_TIME.lock().await;
    let _database = crate::support::hold_server();
    let (_, root) = languages(Some(&[language]))
        .into_iter()
        .next()
        .ok_or_else(|| format!("{language} is not a language of the suite"))?;
    if !root.join("run.sh").exists() {
        return Err(format!("{language} has no conformance runner"));
    }
    let scenario: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let name = scenario["name"]
        .as_str()
        .ok_or("a scenario without a name")?
        .to_owned();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("sdk-modes")
        .join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // The workflows of the scenario, written by the language's own builders.
    let path_arg = path.to_str().ok_or("path")?;
    let (text, _) = script(
        &root,
        "run.sh",
        &["build", path_arg],
        Duration::from_secs(300),
    )
    .await?;
    let built: Value = serde_json::from_str(&text)
        .map_err(|e| format!("build printed something that is not JSON: {e}"))?;
    let (run, config, _) = stage(&scenario, &built, language).await;
    let config_path = dir.join(format!("{language}.{mode}.config.json"));
    let out_path = dir.join(format!("{language}.{mode}.out.json"));
    write_private(
        &config_path,
        &serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out_path);
    let result = script(
        &root,
        "run.sh",
        &[
            mode,
            config_path.to_str().ok_or("path")?,
            out_path.to_str().ok_or("path")?,
        ],
        Duration::from_secs(300),
    )
    .await;
    run.stop().await;
    result?;
    std::fs::read(&out_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| format!("{language} wrote no readable {}", out_path.display()))
}
