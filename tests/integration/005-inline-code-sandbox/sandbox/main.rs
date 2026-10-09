//! Feature 005 sandbox suite against a real container engine: what a script can and cannot do from
//! inside the container the platform runs it in. These tests need Docker and the two runtime images
//! (pulled on first use); there is no way to prove an isolation property without a real container.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

mod isolation;
mod lifecycle;

use std::sync::Mutex;
use std::time::Duration;

use sagat_application::sandbox::{
    BuildFailure, BuildRequest, Built, HostCalls, HttpReply, HttpRequest, Refusal, RunOutcome,
    RunRequest, Sandbox,
};
use sagat_domain::scripts::{Language, Limits, Policy, Source};
use sagat_infra_sandbox::{DockerSandbox, Images};

/// What the supervisor was asked for, and how it answered.
#[derive(Default)]
pub struct Host {
    pub fetched: Mutex<Vec<String>>,
    pub secrets: Mutex<Vec<String>>,
    pub allow: Vec<(&'static str, &'static str)>,
    pub known_secrets: Vec<(&'static str, &'static str)>,
}

impl HostCalls for Host {
    async fn fetch(&self, request: HttpRequest) -> Result<HttpReply, Refusal> {
        self.fetched.lock().expect("lock").push(request.url.clone());
        match self.allow.iter().find(|(url, _)| *url == request.url) {
            Some((_, body)) => Ok(HttpReply {
                status: 200,
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body: body.as_bytes().to_vec(),
            }),
            None => Err(Refusal::Denied),
        }
    }

    async fn secret(&self, name: &str) -> Result<String, Refusal> {
        self.secrets.lock().expect("lock").push(name.to_owned());
        self.known_secrets
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| (*v).to_owned())
            .ok_or(Refusal::Denied)
    }
}

pub fn sandbox() -> DockerSandbox {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init();
    DockerSandbox::connect(Images::default()).expect("connect to the container engine")
}

/// The policy with a short wall clock so a test that should be stopped is stopped quickly.
pub fn quick(wall: Duration, memory_mib: u64) -> Policy {
    Policy {
        limits: Limits::new(
            memory_mib * 1024 * 1024,
            1000,
            wall,
            64,
            64 * 1024,
            16 * 1024 * 1024,
        )
        .expect("limits"),
        ..Policy::default()
    }
}

pub async fn build(
    sandbox: &DockerSandbox,
    language: Language,
    source: &str,
) -> Result<Built, BuildFailure> {
    sandbox
        .build(BuildRequest {
            language,
            source: Source::parse(source).expect("source"),
            lockfile: None,
            policy: Policy::default(),
        })
        .await
}

pub async fn run_with(
    sandbox: &DockerSandbox,
    language: Language,
    source: &str,
    input: &str,
    policy: Policy,
    host: &Host,
) -> RunOutcome {
    let built = build(sandbox, language, source)
        .await
        .expect("the script builds");
    sandbox
        .run(
            RunRequest {
                language,
                artifact: built.artifact,
                input: input.as_bytes().to_vec(),
                input_schema: None,
                output_schema: None,
                policy,
                secrets_to_hide: Vec::new(),
            },
            host,
        )
        .await
}

pub async fn run(
    sandbox: &DockerSandbox,
    language: Language,
    source: &str,
    input: &str,
) -> RunOutcome {
    run_with(
        sandbox,
        language,
        source,
        input,
        Policy::default(),
        &Host::default(),
    )
    .await
}

/// The result of a run as text, or the failure it had.
pub fn result_of(outcome: &RunOutcome) -> String {
    match &outcome.result {
        Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Err(failure) => format!("FAILED {}", failure.code()),
    }
}
