//! Managed credentials never become runner deployment credentials (FR-017-006/012/013).
use super::*;
use serde_json::json;

fn values() -> Value {
    let yaml = std::fs::read_to_string(repo().join("infra/helm/examples/database.yaml"))
        .expect("database example");
    let mut overlay: Value = serde_saphyr::from_str(&yaml).expect("yaml");
    overlay["databaseRunner"]["image"] = json!(format!("example/runner@sha256:{}", "a".repeat(64)));
    with(&overlay)
}

#[test]
fn managed_workloads_use_distinct_identities_and_bounded_cleanup() {
    let docs = render(&values());
    let prefix = "sagat-octopus-orchestrator";
    for (deployment, component) in [
        ("control-plane", "control-plane"),
        ("engine", "execution-engine"),
    ] {
        let pod = &named(&docs, "Deployment", &format!("{prefix}-{deployment}"))["spec"]["template"]
            ["spec"];
        assert_eq!(pod["serviceAccountName"], format!("{prefix}-{component}"));
        assert_eq!(pod["automountServiceAccountToken"], false);
        assert_eq!(pod["initContainers"][0]["restartPolicy"], "Always");
        let app = &pod["containers"][0];
        assert!(!app.to_string().contains("openbao-identity"));
        assert!(app.to_string().contains("SAGAT_OPENBAO_TOKEN_FILE"));
        assert_eq!(
            app.to_string().contains("SAGAT_OPENBAO_INTENT_KEY_FILE"),
            component == "control-plane"
        );
        assert_eq!(pod["volumes"][0]["emptyDir"]["medium"], "Memory");
    }
    let job = named(&docs, "CronJob", &format!("{prefix}-secret-cleanup"));
    assert_eq!(job["spec"]["concurrencyPolicy"], "Forbid");
    assert_eq!(
        job["spec"]["jobTemplate"]["spec"]["activeDeadlineSeconds"],
        180
    );
    let pod = &job["spec"]["jobTemplate"]["spec"]["template"]["spec"];
    assert!(pod["initContainers"][0]["restartPolicy"].is_null());
    assert_eq!(pod["containers"][0]["args"], json!(["cleanup-secrets"]));
    for forbidden in [
        "SAGAT_OIDC",
        "SAGAT_SECRETS_KEYS",
        "intent-key",
        "root-token",
    ] {
        assert!(!pod.to_string().contains(forbidden));
    }
    let config = named(&docs, "ConfigMap", &format!("{prefix}-openbao-agent"));
    assert!(
        config["data"]["secret-cleanup.hcl"]
            .as_str()
            .expect("hcl")
            .contains("exit_after_auth = true")
    );
    for key in ["authMount", "roles"] {
        let mut changed = values();
        if key == "roles" {
            changed["openbao"][key]["control-plane"] = json!("sagat-management-v2");
        } else {
            changed["openbao"][key] = json!("kubernetes-v2");
        }
        let updated = render(&changed);
        for component in ["control-plane", "engine"] {
            let name = format!("{prefix}-{component}");
            assert_ne!(
                named(&docs, "Deployment", &name)["spec"]["template"],
                named(&updated, "Deployment", &name)["spec"]["template"]
            );
        }
    }
}

#[test]
fn runner_has_only_workload_identity_and_explicit_database_network() {
    let docs = render(&values());
    let name = "sagat-octopus-orchestrator-database-runner";
    let pod = &named(&docs, "StatefulSet", name)["spec"]["template"]["spec"];
    assert_eq!(pod["automountServiceAccountToken"], false);
    assert_eq!(pod["volumes"].as_array().expect("volumes").len(), 3);
    for forbidden in [
        "DATABASE_URL",
        "openbao",
        "sagat-db",
        "sagat-keys",
        "SAGAT_SECRETS_KEYS",
    ] {
        assert!(!pod.to_string().contains(forbidden), "{forbidden}");
    }
    let container = &pod["containers"][0];
    assert_eq!(container["securityContext"]["readOnlyRootFilesystem"], true);
    assert!(container["resources"]["limits"]["cpu"].is_number());
    assert_eq!(
        container["env"][2]["valueFrom"]["fieldRef"]["fieldPath"],
        "metadata.name"
    );
    let config: Value = serde_json::from_str(
        named(&docs, "ConfigMap", name)["data"]["database-runner.json"]
            .as_str()
            .expect("config"),
    )
    .expect("json");
    assert_eq!(config["tokenFile"], "/run/sagat-identity/token");
    let policy = named(&docs, "NetworkPolicy", name);
    assert_eq!(policy["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
    assert_eq!(policy["spec"]["egress"].as_array().expect("rules").len(), 3);
    assert!(!has_rule_for_port(policy, "egress", 8200, "TCP"));
    assert!(!has_rule_for_port(policy, "egress", 8080, "TCP"));
    assert_eq!(
        rule_for_port(policy, "egress", 5432, "TCP")["to"],
        json!([{"ipBlock":{"cidr":"10.32.4.0/24"}}])
    );
    let engine = named(&docs, "NetworkPolicy", "sagat-octopus-orchestrator-engine");
    assert!(
        rule_for_port(engine, "ingress", 9000, "TCP")["from"]
            .to_string()
            .contains("database-runner")
    );
}

#[test]
fn managed_deployment_refuses_unsafe_identity_transport_and_network() {
    for change in [
        |v: &mut Value| v["openbao"]["address"] = json!("http://bao"),
        |v: &mut Value| v["openbao"]["intentKeySecret"] = json!(""),
        |v: &mut Value| v["openbao"]["agentImage"] = json!("openbao:latest"),
        |v: &mut Value| v["openbao"]["network"]["to"] = json!([{}]),
        |v: &mut Value| v["databaseRunner"]["image"] = json!("runner:latest"),
        |v: &mut Value| v["databaseRunner"]["config"]["grpcUrl"] = json!("http://engine"),
        |v: &mut Value| v["databaseRunner"]["config"]["developmentLoopback"] = json!(true),
        |v: &mut Value| v["databaseRunner"]["config"]["deployment"]["denyCidrs"] = json!([]),
        |v: &mut Value| v["databaseRunner"]["targetEgress"] = json!([{}]),
        |v: &mut Value| {
            v["databaseRunner"]["targetEgress"] =
                json!([{"ports":[{"port":5432}],"to":[{"ipBlock":{"cidr":"0.0.0.0/0"}}]}])
        },
        |v: &mut Value| v["databaseRunner"]["replicas"] = json!(0),
        |v: &mut Value| {
            v["databaseRunner"]["telemetry"]["tracesEndpoint"] =
                json!("http://collector:4318/v1/traces")
        },
        |v: &mut Value| {
            v["databaseRunner"]["telemetry"]["tracesEndpoint"] =
                json!("https://collector:4318/v1/traces")
        },
        |v: &mut Value| v["networkPolicy"]["enabled"] = json!(false),
    ] {
        let mut v = values();
        change(&mut v);
        assert!(helm(&v).is_err(), "unsafe values rendered");
    }
}
