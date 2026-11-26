//! The Job of the automatic handover (feature 018, `T018-083`, `T018-085`, ADR-0091 decision 1, D2251, D2263, D2268, D2270): a plain
//! Job of the release, named by its revision within the 63 characters of the `job-name` label, that cannot be retried into a loop,
//! takes its run id from its own UID and its credential from a Secret the chart requires, and is not rendered at all when off.

use sha2::{Digest as _, Sha256};

use super::*;
use serde_json::json;

fn jobs(docs: &[Value]) -> Vec<&Value> {
    of_kind(docs, "Job")
}

fn helm_named(release: &str, values: &Value) -> Result<Vec<Value>, String> {
    let dir = tempdir();
    let file = dir.join("values.json");
    std::fs::write(&file, serde_json::to_vec(values).expect("json")).expect("write values");
    let out = Command::new("helm")
        .args(["template", release])
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
        .map(|doc| serde_saphyr::from_str::<Value>(doc).map_err(|e| format!("yaml: {e}")))
        .filter(|doc| !matches!(doc, Ok(Value::Null)))
        .collect()
}

#[test]
fn the_job_is_a_plain_job_of_the_release_that_cannot_loop_and_runs_as_the_handover_role() {
    let docs = render(&base());
    let found = jobs(&docs);
    assert_eq!(found.len(), 1, "one Job of the handover");
    let job = found[0];
    assert!(
        job["metadata"]["annotations"]["helm.sh/hook"].is_null(),
        "a plain Job, not a hook: helm upgrade does not wait for it and a rollback deletes it"
    );
    let spec = &job["spec"];
    assert_eq!(spec["backoffLimit"], 2);
    assert_eq!(spec["ttlSecondsAfterFinished"], 3600);
    assert_eq!(
        spec["activeDeadlineSeconds"],
        7200 + 300,
        "the wait plus five minutes"
    );
    let rules = spec["podFailurePolicy"]["rules"].as_array().expect("rules");
    assert!(rules.iter().any(|rule| rule["action"] == "FailJob"
        && rule["onExitCodes"]["operator"] == "In"
        && rule["onExitCodes"]["values"] == json!([2, 3])
        && rule["onExitCodes"]["containerName"] == "schedule-trigger-handover"));
    assert!(rules.iter().any(|rule| rule["action"] == "Ignore"
        && rule["onPodConditions"][0]["type"] == "DisruptionTarget"));
    let pod = &spec["template"]["spec"];
    assert_eq!(pod["restartPolicy"], "Never");
    assert_eq!(pod["terminationGracePeriodSeconds"], 30);
    assert_eq!(pod["automountServiceAccountToken"], false);
    let account = pod["serviceAccountName"]
        .as_str()
        .expect("its own service account");
    assert_eq!(
        account,
        "sagat-octopus-orchestrator-schedule-trigger-handover"
    );
    let sa = named(&docs, "ServiceAccount", account);
    assert_eq!(sa["automountServiceAccountToken"], false);
    let container = &pod["containers"][0];
    let context = &container["securityContext"];
    assert_eq!(context["readOnlyRootFilesystem"], true);
    assert_eq!(context["allowPrivilegeEscalation"], false);
    let args: Vec<&str> = container["args"]
        .as_array()
        .expect("args")
        .iter()
        .map(|a| a.as_str().expect("text"))
        .collect();
    assert_eq!(&args[..2], ["schedule-trigger-handover", "--auto"]);
    let flag = |name: &str| args.iter().position(|a| *a == name).map(|at| args[at + 1]);
    assert_eq!(
        flag("--min-live"),
        Some("2"),
        "the autoscaler's minimum of the engines"
    );
    assert_eq!(flag("--wait"), Some("7200s"));
    assert!(!args.contains(&"--retire-role"), "off by default");
    // the command line is one the binary accepts
    let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let parsed =
        sagat_control_plane::cli::parse(&owned).expect("the binary accepts the Job's arguments");
    assert!(
        matches!(parsed, sagat_control_plane::cli::Command::ScheduleTriggerHandover(a) if a.auto && a.min_live == Some(2))
    );
    let env = container["env"].as_array().expect("env");
    let entry = |name: &str| {
        env.iter()
            .find(|e| e["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    assert_eq!(
        entry("SAGAT_HANDOVER_RUN_ID")["valueFrom"]["fieldRef"]["fieldPath"],
        "metadata.labels['batch.kubernetes.io/controller-uid']",
        "the run id is the Job's UID: a retried pod resumes, a recreated Job starts afresh"
    );
    assert_eq!(
        entry("DATABASE_URL")["valueFrom"]["secretKeyRef"]["name"],
        "sagat-handover"
    );
    // a Secret deleted after the handover makes the pod exit 2 at once, never hang on its container's configuration
    // (architecture review Major 1)
    for key in ["DATABASE_URL", "SAGAT_HANDOVER_PASSWORD"] {
        assert_eq!(
            entry(key)["valueFrom"]["secretKeyRef"]["optional"],
            true,
            "{key}"
        );
    }
    assert_eq!(
        entry("SAGAT_HANDOVER_PASSWORD")["valueFrom"]["secretKeyRef"],
        json!({"name": "sagat-handover", "key": "SAGAT_HANDOVER_PASSWORD", "optional": true}),
        "the password apart from the URL"
    );
    // its pod answers nobody and reaches only DNS and PostgreSQL
    let policy = named(
        &docs,
        "NetworkPolicy",
        "sagat-octopus-orchestrator-schedule-trigger-handover",
    );
    assert_eq!(policy["spec"]["ingress"], json!([]));
    assert!(has_rule_for_port(policy, "egress", 5432, "TCP"));
    assert_eq!(
        policy["spec"]["egress"].as_array().expect("egress").len(),
        2
    );
    // the engines are told it runs, for their gauge only
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    assert_eq!(env_of(engine)["SAGAT_SCHEDULES_AUTO_HANDOVER"], "true");
}

#[test]
fn the_name_of_the_job_fits_the_job_name_label_for_a_long_release_and_is_the_revisions() {
    let release = "a".repeat(53);
    let docs = helm_named(&release, &base()).expect("renders");
    let name = jobs(&docs)[0]["metadata"]["name"]
        .as_str()
        .expect("name")
        .to_owned();
    let full = format!("{release}-schedule-trigger-handover-1");
    let hash: String = Sha256::digest(full.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let expected = format!("{}-{}", full[..52].trim_end_matches('-'), &hash[..10]);
    assert_eq!(
        name, expected,
        "cut to 52 characters, then - and 10 hex digits of the SHA-256 of the whole name"
    );
    assert!(name.len() <= 63, "{} characters", name.len());
    // a short release keeps its whole name before the hash
    let docs = render(&base());
    let short = jobs(&docs)[0]["metadata"]["name"].as_str().expect("name");
    assert!(
        short.starts_with("sagat-schedule-trigger-handover-1-"),
        "{short}"
    );
}

#[test]
fn the_opt_out_renders_neither_the_job_nor_any_reference_to_its_secret_and_on_requires_it() {
    for off in [
        json!({"schedules": {"autoHandover": false}}),
        json!({"schedules": {"autoHandover": {"enabled": false}}}),
    ] {
        let docs = render(&with(&off));
        assert!(jobs(&docs).is_empty(), "{off}");
        let text = serde_json::to_string(&docs).expect("json");
        assert!(
            !text.contains("sagat-handover"),
            "no reference to the Secret: {off}"
        );
        assert!(!text.contains("schedule-trigger-handover"), "{off}");
        let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
        assert_eq!(env_of(engine)["SAGAT_SCHEDULES_AUTO_HANDOVER"], "false");
    }
    let refused = helm(&with(
        &json!({"schedules": {"autoHandover": {"databaseSecret": ""}}}),
    ))
    .expect_err("required");
    assert!(
        refused.contains("schedules.autoHandover.databaseSecret is required"),
        "{refused}"
    );
    // retiring the role is the operator's choice
    let docs = render(&with(
        &json!({"schedules": {"autoHandover": {"retireRole": true}}}),
    ));
    let args = jobs(&docs)[0]["spec"]["template"]["spec"]["containers"][0]["args"].clone();
    assert!(
        args.as_array()
            .expect("args")
            .iter()
            .any(|a| a == "--retire-role")
    );
}

#[test]
fn the_compose_service_of_the_handover_is_one_shot_with_the_role_apart_from_its_password() {
    let compose: Value = serde_saphyr::from_str(
        &std::fs::read_to_string(repo().join("infra/docker/compose.yaml")).expect("compose"),
    )
    .expect("compose is YAML");
    let service = &compose["services"]["schedule-trigger-handover"];
    assert_eq!(service["restart"], "no");
    let command: Vec<&str> = service["command"]
        .as_array()
        .expect("command")
        .iter()
        .map(|a| a.as_str().expect("text"))
        .collect();
    assert_eq!(&command[..2], ["schedule-trigger-handover", "--auto"]);
    let owned: Vec<String> = command.iter().map(|a| (*a).to_owned()).collect();
    sagat_control_plane::cli::parse(&owned).expect("the binary accepts the service's command");
    let env = &service["environment"];
    let url = env["DATABASE_URL"].as_str().expect("url");
    assert!(
        url.starts_with("postgres://sagat_handover@"),
        "no password in the URL: {url}"
    );
    assert!(env["SAGAT_HANDOVER_PASSWORD"].is_string());
    assert!(
        env["SAGAT_SCHEDULES_AUTO_HANDOVER"]
            .as_str()
            .expect("switch")
            .contains(":-true")
    );
    // the job starts after the control plane and the engine are healthy (G2247): both images have a HEALTHCHECK (their
    // `healthcheck` subcommand), so compose can wait on it instead of on their start
    let dockerfile =
        std::fs::read_to_string(repo().join("infra/docker/Dockerfile")).expect("Dockerfile");
    for after in ["control-plane", "execution-engine"] {
        assert_eq!(
            service["depends_on"][after]["condition"], "service_healthy",
            "after {after} is healthy"
        );
        assert!(
            dockerfile.contains(&format!(
                "CMD [\"/usr/local/bin/{after}\", \"healthcheck\"]"
            )),
            "the image of {after} has the health check compose waits on"
        );
    }
    // the job's container is not a server: the image's probe of an API it does not serve is switched off for it
    assert_eq!(service["healthcheck"]["disable"], true);
    assert!(compose["services"]["migrate"]["environment"]["SAGAT_HANDOVER_PASSWORD"].is_string());
    assert!(
        compose["services"]["execution-engine"]["environment"]["SAGAT_SCHEDULES_AUTO_HANDOVER"]
            .is_string(),
        "the engines export whether the job runs"
    );
    let init =
        std::fs::read_to_string(repo().join("infra/docker/postgres-init/01-runtime-role.sh"))
            .expect("init");
    assert!(
        !init.contains("sagat_handover"),
        "postgres-init does not create it: a fresh volume needs no password"
    );
}
