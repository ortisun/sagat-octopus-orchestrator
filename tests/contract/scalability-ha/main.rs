//! What the Kubernetes chart promises, held to what the platform does (FR-010-001, FR-010-004):
//! rendered with `helm template` and read back, so that a value the engine would refuse, a metric
//! the autoscalers scale on that nothing emits, or a pod that is not hardened is a failing test and
//! not a surprise in a cluster. The chart needs `helm`; a machine without it fails these tests on
//! purpose (the CI runners have it), because a chart nobody renders is a chart nobody checked.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contract/scalability-ha/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The values the tests install the chart with: `fixtures/base.yaml`.
fn base() -> Value {
    serde_yaml_ng::from_str(&fixture("base.yaml")).expect("base.yaml is YAML")
}

/// `base()` with `changes` laid over it (objects are merged, anything else replaced).
fn with(changes: &Value) -> Value {
    fn merge(into: &mut Value, from: &Value) {
        match (into, from) {
            (Value::Object(a), Value::Object(b)) => {
                for (key, value) in b {
                    merge(a.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
            (slot, value) => *slot = value.clone(),
        }
    }
    let mut values = base();
    merge(&mut values, changes);
    values
}

fn helm(values: &Value) -> Result<Vec<Value>, String> {
    let dir = tempdir();
    let file = dir.join("values.json");
    std::fs::write(&file, serde_json::to_vec(values).expect("json")).expect("write values");
    let out = Command::new("helm")
        .args(["template", "sagat"])
        .arg(repo().join("infra/helm"))
        .arg("-f")
        .arg(&file)
        .output()
        .unwrap_or_else(|e| panic!("helm must be installed to check the chart: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let text = String::from_utf8(out.stdout).expect("utf8");
    text.split("\n---\n")
        .map(|doc| serde_yaml_ng::from_str::<Value>(doc).map_err(|e| format!("yaml: {e}")))
        .filter(|doc| !matches!(doc, Ok(Value::Null)))
        .collect()
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sagat-chart-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn render(values: &Value) -> Vec<Value> {
    helm(values).unwrap_or_else(|why| panic!("the chart does not render: {why}"))
}

fn of_kind<'a>(docs: &'a [Value], kind: &str) -> Vec<&'a Value> {
    docs.iter().filter(|d| d["kind"] == kind).collect()
}

fn named<'a>(docs: &'a [Value], kind: &str, name: &str) -> &'a Value {
    docs.iter()
        .find(|d| d["kind"] == kind && d["metadata"]["name"] == name)
        .unwrap_or_else(|| panic!("no {kind} named {name}"))
}

/// The environment of the first container, name to value (a reference to a Secret or a field is
/// given a stand-in, as the pod would resolve it).
fn env_of(deployment: &Value) -> BTreeMap<String, String> {
    deployment["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array()
        .expect("env")
        .iter()
        .map(|e| {
            let name = e["name"].as_str().expect("name").to_owned();
            let value = e["value"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    e["valueFrom"]["fieldRef"]["fieldPath"]
                        .as_str()
                        .map(|_| "pod-name-0".to_owned())
                })
                .or_else(|| {
                    e["valueFrom"]["secretKeyRef"]["name"].as_str().map(|_| {
                        if name == "SAGAT_SECRETS_KEYS" {
                            // 32 bytes, base64: what a key ring in the Secret holds.
                            "k1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_owned()
                        } else {
                            "postgres://sagat_app:x@db/sagat".to_owned()
                        }
                    })
                })
                .unwrap_or_else(|| panic!("{name} has no value"));
            (name, value)
        })
        .collect()
}

#[test]
fn the_chart_renders_every_object_with_its_labels() {
    let docs = render(&base());
    let kinds: BTreeSet<&str> = docs.iter().filter_map(|d| d["kind"].as_str()).collect();
    for kind in [
        "Deployment",
        "Service",
        "PodDisruptionBudget",
        "NetworkPolicy",
        "ServiceAccount",
        "ScaledObject",
    ] {
        assert!(kinds.contains(kind), "no {kind} was rendered: {kinds:?}");
    }
    for doc in &docs {
        let labels = &doc["metadata"]["labels"];
        assert_eq!(
            labels["app.kubernetes.io/name"], "sagat-octopus-orchestrator",
            "{}",
            doc["metadata"]["name"]
        );
        assert!(labels["app.kubernetes.io/version"].is_string());
    }
    // 1 control plane, 1 engine, 2 pools.
    assert_eq!(of_kind(&docs, "Deployment").len(), 4);
    // The engine's and the pool that asked for autoscaling: the other pool is not scaled.
    assert_eq!(of_kind(&docs, "ScaledObject").len(), 2);
}

#[test]
fn the_engine_is_configured_the_way_the_scheduler_needs_and_the_binary_accepts_it() {
    let docs = render(&base());
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    let env = env_of(engine);

    // Each replica holds its shards under the name of its pod; the lease is the operator's.
    assert_eq!(env["SAGAT_SHARD_LEASE_SECS"], "21");
    assert_eq!(env["SAGAT_SCHEDULER_CONCURRENCY"], "6");
    assert_eq!(env["SAGAT_SCHEDULER_BATCH"], "150");
    let replica = engine["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array()
        .expect("env")
        .iter()
        .find(|e| e["name"] == "SAGAT_REPLICA_ID")
        .expect("SAGAT_REPLICA_ID");
    assert_eq!(
        replica["valueFrom"]["fieldRef"]["fieldPath"], "metadata.name",
        "two pods must never be told they are the same replica"
    );
    assert_eq!(
        env["SAGAT_WORKER_POOLS"],
        "render=act.render,act.upscale;export=act.export"
    );

    // And the engine takes what the chart gives it: the same parser the process runs.
    let lookup = |name: &str| env.get(name).cloned();
    let config = sagat_execution_engine::EngineConfig::from_lookup(
        &lookup,
        sagat_control_plane::config::Environment::Prod,
    )
    .unwrap_or_else(|why| panic!("the engine refuses the chart's configuration: {why}"));
    assert_eq!(config.shards.ttl, std::time::Duration::from_secs(21));
    assert_eq!(config.scheduler.concurrency, 6);
    assert_eq!(config.scheduler.batch, 150);
    assert_eq!(config.monitor.settings.pools.len(), 2);
    assert_eq!(config.monitor.settings.pools[0].name, "render");
}

/// Whether `usage` names `variable`, whole or in the short form the help uses for a family of them
/// (`SAGAT_OIDC_ISSUER / _AUDIENCE / _JWKS_URI`).
fn documented(usage: &str, variable: &str) -> bool {
    usage.contains(variable)
        || variable.match_indices('_').any(|(at, _)| {
            usage.contains(&variable[..at]) && usage.contains(&format!("/ {}", &variable[at..]))
        })
}

#[test]
fn every_variable_the_chart_sets_is_one_the_binaries_document() {
    let docs = render(&base());
    let engine_usage = sagat_execution_engine::cli::USAGE;
    let control_usage = sagat_control_plane::cli::USAGE;
    // Set by the chart for the platform to read; the others (DATABASE_URL, the key ring) are the
    // interface the Dockerfile and the compose file already use.
    let always_known = ["DATABASE_URL"];
    for (name, usage) in [
        ("sagat-octopus-orchestrator-engine", engine_usage),
        ("sagat-octopus-orchestrator-control-plane", control_usage),
    ] {
        let deployment = named(&docs, "Deployment", name);
        for variable in env_of(deployment).keys() {
            if always_known.contains(&variable.as_str()) {
                continue;
            }
            // SAGAT_SECRETS_KEYS and the TLS and NATS files are documented in the runbooks and the
            // help of the process that reads them.
            assert!(
                documented(usage, variable),
                "{name} is given {variable}, which its help does not mention"
            );
        }
    }
}

#[test]
fn every_pod_is_hardened_like_the_compose_file_hardens_its_containers() {
    let docs = render(&base());
    for deployment in of_kind(&docs, "Deployment") {
        let name = deployment["metadata"]["name"].as_str().expect("name");
        let pod = &deployment["spec"]["template"]["spec"];
        assert_eq!(pod["automountServiceAccountToken"], false, "{name}");
        assert_eq!(pod["securityContext"]["runAsNonRoot"], true, "{name}");
        assert_eq!(
            pod["securityContext"]["seccompProfile"]["type"], "RuntimeDefault",
            "{name}"
        );
        for container in pod["containers"].as_array().expect("containers") {
            let context = &container["securityContext"];
            assert_eq!(context["readOnlyRootFilesystem"], true, "{name}");
            assert_eq!(context["allowPrivilegeEscalation"], false, "{name}");
            assert_eq!(context["capabilities"]["drop"][0], "ALL", "{name}");
        }
    }
}

#[test]
fn the_engine_can_be_stopped_cleanly_and_never_all_at_once() {
    let docs = render(&base());
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    let pod = &engine["spec"]["template"]["spec"];
    // Time to stop the loops, give the shards back and drain the calls (30 s is the gRPC default).
    assert!(
        pod["terminationGracePeriodSeconds"]
            .as_u64()
            .expect("grace")
            >= 30
    );
    // A new replica takes shards only once the old one gives them back: surge, never lose capacity.
    assert_eq!(
        engine["spec"]["strategy"]["rollingUpdate"]["maxUnavailable"],
        0
    );
    // A voluntary disruption never takes the last replica.
    let budget = named(
        &docs,
        "PodDisruptionBudget",
        "sagat-octopus-orchestrator-engine",
    );
    assert_eq!(budget["spec"]["minAvailable"], 1);
    // Replicas are spread over zones and nodes, so that a lost node is not most of the shards.
    let spread: Vec<&str> = pod["topologySpreadConstraints"]
        .as_array()
        .expect("spread")
        .iter()
        .filter_map(|c| c["topologyKey"].as_str())
        .collect();
    assert_eq!(
        spread,
        ["topology.kubernetes.io/zone", "kubernetes.io/hostname"]
    );
    // With an autoscaler in charge the chart does not also fix the number of replicas.
    assert!(engine["spec"].get("replicas").is_none());
    let fixed = render(&with(&serde_json::json!({
        "engine": {"autoscaling": {"keda": {"enabled": false}}}
    })));
    let engine = named(&fixed, "Deployment", "sagat-octopus-orchestrator-engine");
    assert_eq!(engine["spec"]["replicas"], 2);
}

#[test]
fn the_admin_ports_answer_the_monitoring_namespace_only() {
    let docs = render(&base());
    for (policy, admin) in [
        ("sagat-octopus-orchestrator-engine", 9091),
        ("sagat-octopus-orchestrator-control-plane", 9090),
    ] {
        let policy = named(&docs, "NetworkPolicy", policy);
        let rules = policy["spec"]["ingress"].as_array().expect("ingress");
        let admin_rule = rules
            .iter()
            .find(|r| r["ports"][0]["port"] == admin)
            .unwrap_or_else(|| panic!("no rule for {admin}"));
        assert_eq!(
            admin_rule["from"][0]["namespaceSelector"]["matchLabels"]["kubernetes.io/metadata.name"],
            "monitoring"
        );
    }
    // The pods listen on those ports only because the NetworkPolicy keeps others out.
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    assert_eq!(env_of(engine)["SAGAT_ADMIN_ADDR"], "0.0.0.0:9091");
    let off = render(&with(
        &serde_json::json!({"networkPolicy": {"enabled": false}}),
    ));
    assert!(of_kind(&off, "NetworkPolicy").is_empty());
}

/// The names of the metrics the code records.
fn emitted() -> BTreeSet<String> {
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | "tests" | "node_modules") {
                    sources(&path, out);
                }
            } else if name.ends_with(".rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    for dir in ["crates", "apps"] {
        sources(&repo().join(dir), &mut files);
    }
    let call =
        Regex::new(r#"(?:counter|gauge|histogram)!\(\s*"([a-z][a-z0-9_]*)""#).expect("regex");
    files
        .iter()
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).expect("source");
            call.captures_iter(&text)
                .map(|c| c[1].to_owned())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_autoscalers_scale_on_metrics_the_platform_emits_and_on_nothing_else() {
    let docs = render(&base());
    let emitted = emitted();
    let identifier = Regex::new(r"[a-z][a-z0-9_]*").expect("regex");
    let matchers = Regex::new(r"\{[^}]*\}").expect("regex");
    let functions = ["max", "sum", "min", "avg", "pool"];
    let mut queries = 0;
    for scaled in of_kind(&docs, "ScaledObject") {
        for trigger in scaled["spec"]["triggers"].as_array().expect("triggers") {
            assert_eq!(trigger["type"], "prometheus");
            let query = trigger["metadata"]["query"].as_str().expect("query");
            queries += 1;
            // Drop the label matchers, then every remaining word is a function or a metric.
            let bare = matchers.replace_all(query, "");
            for word in identifier.find_iter(&bare).map(|m| m.as_str()) {
                assert!(
                    functions.contains(&word) || emitted.contains(word),
                    "{query}: {word} is not a metric the code emits"
                );
            }
            // The threshold is a number of something per replica, not zero.
            let threshold: f64 = trigger["metadata"]["threshold"]
                .as_str()
                .expect("threshold")
                .parse()
                .expect("a number");
            assert!(threshold > 0.0, "{query}");
        }
    }
    assert_eq!(queries, 3, "the engine's, and the two of the pool");
    // The label a pool's queries select by is the label the monitor puts on them.
    let pool_queries: Vec<String> = of_kind(&docs, "ScaledObject")
        .iter()
        .flat_map(|s| s["spec"]["triggers"].as_array().expect("triggers").clone())
        .filter_map(|t| t["metadata"]["query"].as_str().map(str::to_owned))
        .filter(|q| q.contains("pool_"))
        .collect();
    assert_eq!(pool_queries.len(), 2);
    assert!(pool_queries.iter().all(|q| q.contains(r#"pool="render""#)));
    let monitor =
        std::fs::read_to_string(repo().join("crates/scheduler/src/monitor.rs")).expect("monitor");
    assert!(monitor.contains(r#""pool" =>"#));
}

#[test]
fn the_chart_refuses_what_the_platform_would_refuse() {
    let cases: Vec<Value> =
        serde_yaml_ng::from_str(&fixture("refused.yaml")).expect("refused.yaml is YAML");
    assert!(
        cases.len() >= 14,
        "the list of refusals shrank to {}",
        cases.len()
    );
    for case in &cases {
        let name = case["name"].as_str().expect("a name");
        let expected = case["message"].as_str().expect("a message");
        let why = helm(&with(&case["changes"])).expect_err(name);
        assert!(
            why.contains(expected),
            "{name}: expected the render to fail with {expected:?}, got: {why}"
        );
    }
}

#[test]
fn what_the_chart_renders_for_pools_is_accepted_by_the_engine_for_any_pool_it_allows() {
    // 32 pools is the most the engine takes (SAGAT_WORKER_POOLS): the chart must render them all
    // and the engine must accept the result, so the two limits cannot drift apart.
    let pools: Vec<Value> = (0..32)
        .map(|n| {
            serde_json::json!({
                "name": format!("pool-{n}"),
                "image": "registry.example.com/team/w:1",
                "activities": [format!("act.{n}")]
            })
        })
        .collect();
    let docs = render(&with(&serde_json::json!({"workerPools": pools})));
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    let env = env_of(engine);
    let lookup = |name: &str| env.get(name).cloned();
    let config = sagat_execution_engine::EngineConfig::from_lookup(
        &lookup,
        sagat_control_plane::config::Environment::Prod,
    )
    .expect("the engine takes 32 pools");
    assert_eq!(config.monitor.settings.pools.len(), 32);
    assert_eq!(of_kind(&docs, "Deployment").len(), 2 + 32);
}

/// The bound of an index range has to be a stable expression: the planner can use `now()` as the
/// end of a range and cannot use `clock_timestamp()`, which is volatile, so a pass bounded with it
/// reads every waiting step in order to find the few that are due (`FR-010-003`). The plan of the
/// due-timer query is in the benchmark's report; this keeps the mistake from coming back.
#[test]
fn no_candidate_query_bounds_an_index_range_with_a_volatile_clock() {
    let comparison = Regex::new(r"(<=|<|>=|>)\s*clock_timestamp\(\)").expect("regex");
    for file in [
        "crates/infra-postgres/src/maintenance.rs",
        "crates/infra-postgres/src/operations.rs",
    ] {
        let text = std::fs::read_to_string(repo().join(file)).expect("source");
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            assert!(
                !comparison.is_match(code),
                "{file}:{}: compares with clock_timestamp(), which the planner cannot use as the bound of an index range; use now(): {line}",
                number + 1
            );
        }
    }
}

#[test]
fn the_example_an_operator_starts_from_renders_and_the_engine_accepts_it() {
    let text =
        std::fs::read_to_string(repo().join("infra/helm/examples/ha.yaml")).expect("example");
    let example: Value = serde_yaml_ng::from_str(&text).expect("the example is YAML");
    let docs = render(&example);
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    let env = env_of(engine);
    // The example has the engine terminate TLS: the certificate and the key are files of a Secret the
    // pod mounts, which the test stands in for with files of its own at the places the pod would read.
    assert_eq!(
        env.get("SAGAT_GRPC_TLS_CERT_FILE").map(String::as_str),
        Some("/etc/sagat/tls/tls.crt")
    );
    let volumes = engine["spec"]["template"]["spec"]["volumes"]
        .as_array()
        .expect("volumes");
    assert!(
        volumes
            .iter()
            .any(|v| v["secret"]["secretName"] == "sagat-engine-tls"),
        "the Secret with the certificate is mounted"
    );
    let dir = tempdir();
    let fake = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
    for file in ["tls.crt", "tls.key"] {
        std::fs::write(dir.join(file), fake).expect("a stand-in for the mounted Secret");
    }
    let lookup = |name: &str| match name {
        "SAGAT_GRPC_TLS_CERT_FILE" => Some(dir.join("tls.crt").to_string_lossy().into_owned()),
        "SAGAT_GRPC_TLS_KEY_FILE" => Some(dir.join("tls.key").to_string_lossy().into_owned()),
        other => env.get(other).cloned(),
    };
    let config = sagat_execution_engine::EngineConfig::from_lookup(
        &lookup,
        sagat_control_plane::config::Environment::Prod,
    )
    .unwrap_or_else(|why| panic!("the engine refuses the example's configuration: {why}"));
    assert_eq!(config.monitor.settings.pools.len(), 1);
    assert_eq!(config.monitor.settings.pools[0].name, "payments");
    // The example asks for more engines than the shards it divides: three, 22 + 21 + 21.
    assert!(
        example["engine"]["replicas"]
            .as_u64()
            .is_some_and(|n| n <= 64)
    );
    // And what it scales is scaled on what the platform emits (the same check as for the test values).
    assert_eq!(of_kind(&docs, "ScaledObject").len(), 2);
}

#[test]
fn the_queries_of_the_autoscalers_carry_the_installs_own_matchers_when_it_has_some() {
    let docs = render(&with(
        &serde_json::json!({"metrics": {"selector": "namespace=\"prod\""}}),
    ));
    let queries: Vec<String> = of_kind(&docs, "ScaledObject")
        .iter()
        .flat_map(|s| s["spec"]["triggers"].as_array().expect("triggers").clone())
        .filter_map(|t| t["metadata"]["query"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(queries.len(), 3, "{queries:?}");
    for query in &queries {
        assert!(query.contains(r#"namespace="prod""#), "{query}");
    }
    assert!(
        queries
            .iter()
            .any(|q| q.contains(r#"pool="render",namespace="prod""#)),
        "{queries:?}"
    );
    // And without it they are what they were.
    let plain = render(&base());
    let bare: Vec<String> = of_kind(&plain, "ScaledObject")
        .iter()
        .flat_map(|s| s["spec"]["triggers"].as_array().expect("triggers").clone())
        .filter_map(|t| t["metadata"]["query"].as_str().map(str::to_owned))
        .collect();
    assert!(
        bare.contains(&"max(scheduler_lag_seconds)".to_owned()),
        "{bare:?}"
    );
}

#[test]
fn every_probe_of_the_chart_asks_for_a_path_its_process_serves() {
    // The control plane answers /v1/health and /v1/ready on its API port and the engine /livez and
    // /readyz on its admin port; a probe for the other's paths would keep a pod from ever starting.
    let docs = render(&base());
    for (deployment, source, prefix) in [
        (
            "sagat-octopus-orchestrator-control-plane",
            "crates/api-http/src/lib.rs",
            "/v1",
        ),
        (
            "sagat-octopus-orchestrator-engine",
            "apps/execution-engine/src/lib.rs",
            "",
        ),
    ] {
        let text = std::fs::read_to_string(repo().join(source)).expect(source);
        let container =
            &named(&docs, "Deployment", deployment)["spec"]["template"]["spec"]["containers"][0];
        for probe in ["startupProbe", "livenessProbe", "readinessProbe"] {
            let path = container[probe]["httpGet"]["path"].as_str().expect(probe);
            let route = path
                .strip_prefix(prefix)
                .expect("under the process's prefix");
            assert!(
                text.contains(&format!(".route(\"{route}\"")),
                "{deployment}'s {probe} asks for {path}, and {source} registers no route {route}"
            );
            let port = container[probe]["httpGet"]["port"]
                .as_str()
                .expect("a named port");
            assert!(
                container["ports"]
                    .as_array()
                    .expect("ports")
                    .iter()
                    .any(|declared| declared["name"] == port),
                "{deployment}'s {probe} names the port {port}, which the container does not declare"
            );
        }
    }
}
