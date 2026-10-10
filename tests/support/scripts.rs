//! A platform with a sandbox runner, for the feature 005 suites: the API and the worker protocol
//! over a real PostgreSQL, a runner (the production code) that talks to them over the network and
//! runs scripts in real containers. Time and the scheduler are stepped by the test, so a scenario
//! does not depend on sleeping.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use sagat_application::sandbox::Sandbox;
use sagat_domain::identity::Role;
use sagat_domain::scripts::Policy;
use sagat_infra_sandbox::{DockerSandbox, Images};
use sagat_sandbox_runner::api::Credentials;
use sagat_sandbox_runner::{NamespaceTarget, Runner, RunnerConfig, StepOutcome};
use serde_json::{Value, json};

use super::api::Options;
use super::oidc::TokenBuilder;
use super::worker::{NAMESPACE, Platform, TENANT, platform_with};

pub const RUNNER: &str = "runner-1";

pub struct Lab<S: Sandbox + 'static = DockerSandbox> {
    pub p: Platform,
    pub runner: Arc<Runner<S>>,
    /// How the runner was configured, so a restart is the same runner.
    pub config: RunnerConfig,
    /// The id of acme's `payments` namespace.
    pub ns: String,
}

pub async fn lab() -> Lab {
    lab_with(Options::default()).await
}

pub async fn lab_with(options: Options) -> Lab {
    lab_configured(options, |_| {}).await
}

/// A lab whose runner's settings the test may change before it starts.
pub async fn lab_configured(options: Options, configure: impl FnOnce(&mut RunnerConfig)) -> Lab {
    let sandbox = DockerSandbox::connect(Images::default()).expect("the container engine");
    lab_on(options, sandbox, configure).await
}

/// A lab whose runner runs scripts on `sandbox`: the container engine, or a stand-in that fails in
/// the ways the engine can.
pub async fn lab_on<S: Sandbox + 'static>(
    options: Options,
    sandbox: S,
    configure: impl FnOnce(&mut RunnerConfig),
) -> Lab<S> {
    let ceiling = options.script_ceiling;
    let p = platform_with(options).await;
    p.grant(p.tenant, RUNNER, Role::Runner).await;
    p.bind_everything(RUNNER).await;
    let ns = p.namespace_id().await;
    let key = p.api.idp.key.lock().expect("key").clone();
    let mut config = RunnerConfig {
        api_url: p.api.base.clone(),
        grpc_url: p.api.grpc_url.clone(),
        credentials: Credentials::Provider(Arc::new(move || {
            TokenBuilder::new(key.clone(), RUNNER, TENANT).build()
        })),
        ca_pem: None,
        name: RUNNER.to_owned(),
        namespaces: vec![NamespaceTarget {
            slug: NAMESPACE.to_owned(),
            id: ns.parse().expect("namespace id"),
        }],
        max_concurrent: 4,
        poll_interval: Duration::from_millis(50),
        ceiling,
        trusted_keys: vec![p.api.script_key.clone()],
        images: Images::default(),
        admin_addr: None,
        sweep_after: Duration::from_secs(3600),
    };
    configure(&mut config);
    let runner = Runner::start(config.clone(), sandbox)
        .await
        .expect("the runner starts");
    Lab {
        p,
        runner,
        config,
        ns,
    }
}

impl Lab<DockerSandbox> {
    /// The runner dies and another starts in its place: nothing it knew survives.
    pub async fn restart(&mut self) {
        let sandbox =
            DockerSandbox::connect(self.config.images.clone()).expect("the container engine");
        self.runner = Runner::start(self.config.clone(), sandbox)
            .await
            .expect("the runner starts again");
    }
}

impl<S: Sandbox + 'static> Lab<S> {
    /// A REST call as the administrator of acme.
    pub async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
        self.p.rest(method, path, body).await
    }

    /// Creates a script; returns its id.
    pub async fn script(&self, name: &str, language: &str, source: &str) -> String {
        self.script_with(name, language, source, json!({})).await
    }

    pub async fn script_with(
        &self,
        name: &str,
        language: &str,
        source: &str,
        extra: Value,
    ) -> String {
        let mut body = json!({"name": name, "language": language, "source": source});
        for (key, value) in extra.as_object().expect("object") {
            body[key] = value.clone();
        }
        let (status, created) = self
            .call(
                Method::POST,
                &format!("/namespaces/{}/scripts", self.ns),
                Some(&body),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        created["id"].as_str().expect("id").to_owned()
    }

    /// Publishes the draft and lets the runner build it; returns the version.
    pub async fn publish(&self, script: &str) -> Value {
        let (status, published) = self
            .call(
                Method::POST,
                &format!("/namespaces/{}/scripts/{script}/versions", self.ns),
                None,
            )
            .await;
        assert!(status == 202 || status == 200, "{status}: {published}");
        let number = published["version"]["number"].as_u64().expect("number");
        while self.runner.build_once().await.expect("build") {}
        let (status, version) = self
            .call(
                Method::GET,
                &format!("/namespaces/{}/scripts/{script}/versions/{number}", self.ns),
                None,
            )
            .await;
        assert_eq!(status, 200, "{version}");
        version
    }

    /// Publishes and requires the build to have worked; returns the digest the workflows pin.
    pub async fn ready(&self, script: &str) -> String {
        let version = self.publish(script).await;
        assert_eq!(version["status"], "READY", "{version}");
        version["artifactDigest"]
            .as_str()
            .expect("digest")
            .to_owned()
    }

    /// Tries the draft with `input` and waits for the runner to have done it.
    pub async fn test(&self, script: &str, input: &Value) -> Value {
        let (status, queued) = self
            .call(
                Method::POST,
                &format!("/namespaces/{}/scripts/{script}/test-runs", self.ns),
                Some(&json!({"input": input})),
            )
            .await;
        assert_eq!(status, 202, "{queued}");
        let id = queued["id"].as_str().expect("id").to_owned();
        while self.runner.test_once().await.expect("test") {}
        let (status, done) = self
            .call(
                Method::GET,
                &format!("/namespaces/{}/scripts/{script}/test-runs/{id}", self.ns),
                None,
            )
            .await;
        assert_eq!(status, 200, "{done}");
        done
    }

    /// A workflow of one script node; returns its id.
    pub async fn workflow(&self, name: &str, digest: &str, extra: Value) -> String {
        let mut node = json!({"id": "s", "type": "script", "artifactDigest": digest,
                              "retry": {"maxAttempts": 1}});
        for (key, value) in extra.as_object().expect("object") {
            node[key] = value.clone();
        }
        let ir = json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": name}, "entrypoint": "s", "nodes": [node]
        });
        self.p.publish(name, &ir).await
    }

    /// Lets the platform and the runner work until the run has ended; returns its final status.
    pub async fn drive(&self, run: &str) -> String {
        let namespace = self.runner.config().namespaces[0].clone();
        for _ in 0..40 {
            self.p.tick().await;
            for step in self.runner.poll_steps(&namespace).await.expect("poll") {
                let _: StepOutcome = self.runner.run_step(step).await.expect("run the step");
            }
            self.p.tick().await;
            let status = self.p.status(run).await;
            if matches!(
                status.as_str(),
                "SUCCEEDED" | "FAILED" | "CANCELLED" | "TIMED_OUT"
            ) {
                return status;
            }
        }
        panic!("the run did not end: {}", self.p.status(run).await);
    }

    /// The output a node of a run kept.
    pub async fn output(&self, run: &str, node: &str) -> Option<Value> {
        let content: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT p.content FROM steps s JOIN payloads p \
               ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest \
             WHERE s.run_id = $1 AND s.node = $2",
        )
        .bind(uuid::Uuid::parse_str(run).expect("run id"))
        .bind(node)
        .fetch_optional(&self.p.api.db.pool)
        .await
        .expect("query")
        .flatten();
        content.and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// The failure code a node of a run ended with.
    pub async fn failure(&self, run: &str, node: &str) -> Option<String> {
        sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = $2")
            .bind(uuid::Uuid::parse_str(run).expect("run id"))
            .bind(node)
            .fetch_optional(&self.p.api.db.pool)
            .await
            .expect("query")
            .flatten()
    }

    /// The executions kept for an artifact.
    pub async fn executions(&self, digest: &str) -> Vec<Value> {
        let (status, listed) = self
            .call(
                Method::GET,
                &format!("/namespaces/{}/script-executions?digest={digest}", self.ns),
                None,
            )
            .await;
        assert_eq!(status, 200, "{listed}");
        // A listing has no logs: the records are read one by one.
        let mut full = Vec::new();
        for item in listed["items"].as_array().expect("items") {
            let id = item["id"].as_str().expect("id");
            let (status, record) = self
                .call(
                    Method::GET,
                    &format!("/namespaces/{}/script-executions/{id}", self.ns),
                    None,
                )
                .await;
            assert_eq!(status, 200, "{record}");
            full.push(record);
        }
        full
    }

    /// Rows of the audit trail of acme with this action.
    pub async fn audited(&self, action: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT coalesce(reason, '') FROM audit_records WHERE tenant_id = $1 AND action = $2 \
             ORDER BY created_at",
        )
        .bind(self.p.tenant.as_uuid())
        .bind(action)
        .fetch_all(&self.p.api.db.pool)
        .await
        .expect("audit")
    }

    /// Stores a secret of the namespace (as an administrator).
    pub async fn secret(&self, name: &str, value: &str) {
        let (status, body) = self
            .call(
                Method::PUT,
                &format!("/namespaces/{}/secrets/{name}", self.ns),
                Some(&json!({"value": value})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
}

/// The default policy, for tests that start from it.
#[must_use]
pub fn default_policy() -> Policy {
    Policy::default()
}

/// How a stand-in sandbox fails.
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    /// The engine cannot be used for anything.
    Down,
    /// Builds work (the artifact is a bundle that says nothing is run), running does not.
    CannotRun,
}

/// A sandbox that fails in the ways the container engine can, on demand: stopping the engine under
/// a test would break every other test sharing it.
#[derive(Debug, Clone, Copy)]
pub struct Faulty(pub Fault);

impl Sandbox for Faulty {
    async fn toolchain(
        &self,
        _: sagat_domain::scripts::Language,
    ) -> Result<String, sagat_application::sandbox::BuildFailure> {
        Ok("faulty@sha256:00;shim=1".to_owned())
    }

    async fn build(
        &self,
        request: sagat_application::sandbox::BuildRequest,
    ) -> Result<sagat_application::sandbox::Built, sagat_application::sandbox::BuildFailure> {
        match self.0 {
            Fault::Down => Err(sagat_application::sandbox::BuildFailure::Unavailable),
            Fault::CannotRun => Ok(sagat_application::sandbox::Built {
                artifact: format!(
                    r#"{{"v":1,"language":"{}","policy":"{}","entry":"main.mjs","files":{{"main.mjs":"export default () => 1"}}}}"#,
                    request.language,
                    request.policy.canonical()
                )
                .into_bytes(),
                sbom: b"{}".to_vec(),
                toolchain: "faulty@sha256:00;shim=1".to_owned(),
                log: String::new(),
            }),
        }
    }

    async fn run<H: sagat_application::sandbox::HostCalls>(
        &self,
        _: sagat_application::sandbox::RunRequest,
        _: &H,
    ) -> sagat_application::sandbox::RunOutcome {
        sagat_application::sandbox::RunOutcome {
            result: Err(sagat_domain::scripts::Failure::Unavailable),
            log: String::new(),
            log_truncated: false,
            denied: Vec::new(),
            exit_code: None,
            wall: Duration::ZERO,
            memory_peak_bytes: None,
            start: Duration::ZERO,
            inferred_input_schema: None,
            inferred_output_schema: None,
        }
    }
}
