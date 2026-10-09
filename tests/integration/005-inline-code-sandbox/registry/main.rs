//! Feature 005 registry suite against a real PostgreSQL: scripts, immutable versions, signed
//! artifacts, the builds and test runs a runner takes under a lease, and what ran. The sandbox itself
//! is in the `integration_005_sandbox` suite; here the runner is played by the test, which hands back
//! what a sandbox would.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../../support/mod.rs"]
mod support;

mod builds;
mod executions;
mod migration;
mod scripts;
mod trials;

use sagat_application::scripts::{
    BuildAssignment, BuildReport, PolicyRequest, Published, ScriptRecord, ScriptSpec, VersionRecord,
};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::scripts::{Digest, Language, Lockfile, Source};
use support::engine::{World, ctx};
use support::{slug, subject};

pub const TS_SOURCE: &str =
    "export default function (input: { n: number }) { return { double: input.n * 2 }; }\n";
pub const PY_SOURCE: &str = "def run(input, octopus):\n    return {'double': input['n'] * 2}\n";

/// What a runner reports it has: one toolchain per language.
pub fn toolchains() -> Vec<(Language, String)> {
    vec![
        (Language::TypeScript, "node@sha256:aa;shim=1".to_owned()),
        (Language::Python, "python@sha256:bb;shim=1".to_owned()),
    ]
}

pub fn spec(name: &str, language: Language, source: &str) -> ScriptSpec {
    ScriptSpec {
        name: slug(name),
        language,
        source: Source::parse(source).expect("source"),
        lockfile: None,
        input_schema: None,
        output_schema: None,
        policy: PolicyRequest::default(),
        retention: None,
    }
}

pub fn ts(name: &str) -> ScriptSpec {
    spec(name, Language::TypeScript, TS_SOURCE)
}

pub fn lockfile(text: &str) -> Lockfile {
    Lockfile::parse(text).expect("lockfile")
}

pub fn developer(w: &World) -> Principal {
    Principal::new(w.admin.tenant_id(), subject("dev-1"), Role::Developer)
}

pub fn viewer(w: &World) -> Principal {
    Principal::new(w.admin.tenant_id(), subject("viewer-1"), Role::Viewer)
}

pub fn other_runner(w: &World) -> Principal {
    Principal::new(w.other_admin.tenant_id(), subject("runner-2"), Role::Runner)
}

pub async fn create(w: &World, name: &str) -> ScriptRecord {
    w.scripts
        .create(&w.admin, w.namespace.id(), ts(name), &ctx())
        .await
        .expect("create the script")
}

pub async fn publish(w: &World, script: &ScriptRecord) -> Published {
    w.scripts
        .publish(&w.admin, w.namespace.id(), script.id, &ctx())
        .await
        .expect("publish")
}

/// Some bytes a build could have made: the control plane treats an artifact as opaque.
pub fn artifact(label: &str) -> Vec<u8> {
    format!(
        r#"{{"v":1,"language":"typescript","entry":"main.mjs","files":{{"main.mjs":"{label}"}}}}"#
    )
    .into_bytes()
}

pub fn built(label: &str, toolchain: &str) -> BuildReport {
    BuildReport::Built {
        artifact: artifact(label),
        sbom: br#"{"bomFormat":"CycloneDX"}"#.to_vec(),
        toolchain: toolchain.to_owned(),
        log: "built".to_owned(),
    }
}

pub async fn claim(w: &World) -> Option<BuildAssignment> {
    w.runner
        .claim_build(&w.runner_identity, &toolchains(), &ctx())
        .await
        .expect("claim a build")
}

pub async fn complete(w: &World, job: &BuildAssignment, report: BuildReport) {
    w.runner
        .complete_build(
            &w.runner_identity,
            job.job,
            &job.lease_token,
            report,
            &ctx(),
        )
        .await
        .expect("complete the build");
}

/// A script, published and built by the runner: its ready version.
pub async fn ready(w: &World, name: &str, label: &str) -> (ScriptRecord, VersionRecord) {
    let script = create(w, name).await;
    let published = publish(w, &script).await;
    let job = claim(w).await.expect("a build is waiting");
    assert_eq!(job.version, published.version.id);
    complete(w, &job, built(label, &toolchains()[0].1)).await;
    let version = w
        .scripts
        .version(
            &w.admin,
            w.namespace.id(),
            script.id,
            published.version.number,
            &ctx(),
        )
        .await
        .expect("the version");
    assert_eq!(version.status, sagat_domain::scripts::VersionStatus::Ready);
    (script, version)
}

pub fn digest_of(version: &VersionRecord) -> Digest {
    version
        .artifact
        .clone()
        .expect("a built version has an artifact")
}
