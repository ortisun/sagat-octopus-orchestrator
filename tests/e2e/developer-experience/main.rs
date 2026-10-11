//! The four user stories of feature 012 through the public interfaces: `sagat` (the library the
//! executable wraps) writes a workflow from a template, publishes it, serves it with canned
//! answers and runs it against a real platform (PostgreSQL, the HTTP API, the engine as a process),
//! and REST shows what the platform made of it.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod dev_environment;
mod diagnostics;
mod doctor;
mod saga;
mod stories;
mod us03;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reqwest::Method;
use sagat_cli::Env;
use serde_json::Value;

use support::worker::{Platform, RunningEngine, engine_env};

/// A directory of the test's own that goes away with it.
pub struct Dir(pub PathBuf);

impl Dir {
    pub fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sagat-e2e-dx-{}-{}",
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

    pub fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.0.join(name)).expect("read")
    }
}

impl Default for Dir {
    fn default() -> Self {
        Self::new()
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

    #[track_caller]
    pub fn omits(&self, text: &str) -> &Self {
        assert!(
            !self.all().contains(text),
            "did not expect '{text}' in:\n{}\n{}",
            self.out,
            self.err
        );
        self
    }

    /// The id after `run ` in the line `run <id> started`.
    pub fn run_id(&self) -> String {
        self.out
            .lines()
            .find_map(|l| {
                l.strip_prefix("run ")?
                    .split_whitespace()
                    .next()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| panic!("no run id in:\n{}", self.out))
    }
}

/// Runs `sagat` as `sub` of acme against the platform, from `dir`.
pub async fn cli_as(p: &Platform, dir: &Dir, sub: &str, args: &[&str]) -> Outcome {
    let env = Env {
        server: Some(p.api.base.clone()),
        token: Some(p.api.idp.token(sub, "acme").build()),
        namespace: Some("payments".to_owned()),
        cwd: dir.0.clone(),
    };
    run_with(env, args).await
}

/// Runs `sagat` as the administrator of acme.
pub async fn cli(p: &Platform, dir: &Dir, args: &[&str]) -> Outcome {
    cli_as(p, dir, "alice", args).await
}

pub async fn run_with(env: Env, args: &[&str]) -> Outcome {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = sagat_cli::run(&args, &env, &mut out, &mut err).await;
    Outcome {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
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

/// REST as `alice` of acme in the `payments` namespace.
pub async fn rest(p: &Platform, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
    p.rest(method, path, body).await
}
