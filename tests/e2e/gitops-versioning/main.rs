//! The four user stories of feature 011 through the public interfaces: the `sagat` command line
//! (the library the executable is a thin wrapper of) publishes and promotes from a manifest, and
//! REST shows what the platform made of it. Real PostgreSQL, the real HTTP API, the engine as a
//! process where a run has to be woken.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod cli;
mod us01;
mod us02;
mod us03;
mod us04;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reqwest::Method;
use sagat_cli::Env;
use serde_json::{Value, json};

use support::worker::{Platform, RunningEngine, engine_env, single_ir};

/// A directory of the test's own that goes away with it.
pub struct Dir(pub PathBuf);

impl Default for Dir {
    fn default() -> Self {
        Self::new()
    }
}

impl Dir {
    pub fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sagat-e2e-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    pub fn write(&self, name: &str, text: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("write");
    }

    pub fn document(&self, name: &str, doc: &Value) {
        self.write(
            &format!("workflows/{name}.json"),
            &serde_json::to_string_pretty(doc).expect("json"),
        );
    }

    /// The manifest listing `names`, whose documents are `workflows/<name>.json`.
    pub fn manifest(&self, namespace: &str, names: &[&str]) {
        let mut text = format!(
            "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: {namespace}\nworkflows:\n"
        );
        for name in names {
            text.push_str(&format!(
                "  - name: {name}\n    file: workflows/{name}.json\n"
            ));
        }
        self.write("sagat.yaml", &text);
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a command printed and the code it ended with.
pub struct Outcome {
    pub code: u8,
    pub out: String,
    pub err: String,
}

impl Outcome {
    pub fn all(&self) -> String {
        format!("{}{}", self.out, self.err)
    }

    #[track_caller]
    pub fn expect(&self, code: u8) -> &Self {
        assert_eq!(
            self.code, code,
            "exit code; it said:\n{}\n{}",
            self.out, self.err
        );
        self
    }

    #[track_caller]
    pub fn mentions(&self, text: &str) -> &Self {
        assert!(
            self.all().contains(text),
            "expected '{text}' in:\n{}\n{}",
            self.out,
            self.err
        );
        self
    }
}

/// Runs `sagat` as `sub` of acme against the platform, from `dir`.
pub async fn cli(p: &Platform, dir: &Dir, sub: &str, args: &[&str]) -> Outcome {
    let env = Env {
        server: Some(p.api.base.clone()),
        token: Some(p.api.idp.token(sub, "acme").build()),
        namespace: None,
        cwd: dir.0.clone(),
    };
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = sagat_cli::run(&args, &env, &mut out, &mut err).await;
    Outcome {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

/// A one-activity workflow whose digest differs with `marker`.
pub fn flow(name: &str, marker: &str) -> Value {
    let mut ir = single_ir(name, None);
    ir["metadata"]["description"] = json!(marker);
    ir
}

/// The digest of a document, as the platform computes it.
pub fn digest_of(doc: &Value) -> String {
    use sagat_application::definitions::IrCodec;
    sagat_infra_ir::JsonIrCodec
        .parse(&serde_json::to_vec(doc).expect("json"))
        .expect("a valid document")
        .digest
        .to_hex()
}

/// An engine started the way the binary is, with loops that look often.
pub async fn engine(p: &Platform) -> RunningEngine {
    let ring = format!("k1:{}", p.api.key_k1);
    let env = vec![
        ("SAGAT_SECRETS_KEYS", ring.as_str()),
        ("SAGAT_EVENT_POLL_MS", "50"),
        ("SAGAT_DELIVERY_POLL_MS", "50"),
    ];
    RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &env)).await
}

/// Waits for `check` to return something, polling.
pub async fn within<T, F, Fut>(what: &str, seconds: u64, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what} did not happen within {seconds} s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// REST as `sub` of acme in `namespace`.
pub async fn rest_in(
    p: &Platform,
    sub: &str,
    namespace: &str,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> (u16, Value) {
    let mut request = p
        .api
        .as_user(method, path, sub, "acme")
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", format!("key-{}", uuid::Uuid::now_v7()));
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// The id of the workflow called `name` in `namespace`.
pub async fn workflow_id(p: &Platform, namespace: &str, name: &str) -> String {
    let (status, page) = rest_in(
        p,
        "alice",
        namespace,
        Method::GET,
        "/workflows?limit=200",
        None,
    )
    .await;
    assert_eq!(status, 200, "{page}");
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|w| w["name"] == name)
        .and_then(|w| w["id"].as_str())
        .unwrap_or_else(|| panic!("{namespace} has no workflow {name}: {page}"))
        .to_owned()
}

/// A published version of a workflow, as REST returns it.
pub async fn version(p: &Platform, namespace: &str, workflow: &str, number: u32) -> Value {
    let (status, body) = rest_in(
        p,
        "alice",
        namespace,
        Method::GET,
        &format!("/workflows/{workflow}/versions/{number}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

/// The active version number of a workflow.
pub async fn active(p: &Platform, namespace: &str, workflow: &str) -> u64 {
    let (status, body) = rest_in(
        p,
        "alice",
        namespace,
        Method::GET,
        &format!("/workflows/{workflow}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["activeVersion"]["versionNumber"]
        .as_u64()
        .expect("an active version")
}

pub fn path_str(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}
