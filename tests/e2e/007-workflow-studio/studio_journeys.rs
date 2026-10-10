//! The Studio in a real browser against a real platform (SC-007-001, SC-007-002): this starts what
//! the journeys need and runs Playwright (`frontend/e2e`) against it. The platform is the one the
//! SDK conformance suite starts: PostgreSQL, the REST API with the Studio served under `/studio/`
//! (same origin, strict policy), and the execution engine with its worker API and its loops. The
//! journeys sign in with tokens like a user, and run workers of their own with the TypeScript SDK.
//!
//! ```text
//! cargo test -p sagat-tests --test e2e_007 -- --ignored journeys --nocapture
//! SAGAT_STUDIO_GREP="US03" …                         # a journey
//! SAGAT_STUDIO_REBUILD=0 …                           # use the build in frontend/apps/studio/dist
//! ```
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sagat_api_http::studio::StudioConfig;
use sagat_domain::identity::Role;
use tokio::process::Command;

use crate::support::api::Options;
use crate::support::scripts::lab_with;
use crate::support::worker::{HUMAN, NAMESPACE, RunningEngine, TENANT, engine_env};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn run(program: &str, args: &[&str], dir: &Path, env: &[(&str, &str)]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} {args:?} in {} exited with {status}",
            dir.display()
        ))
    }
}

/// The build the Studio is served from, made unless `SAGAT_STUDIO_REBUILD=0`.
async fn build_studio() -> PathBuf {
    let frontend = repo().join("frontend");
    if std::env::var("SAGAT_STUDIO_REBUILD").as_deref() != Ok("0") {
        if !frontend.join("node_modules").exists() {
            run(
                "npm",
                &["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
                &frontend,
                &[],
            )
            .await
            .expect("installing the frontend's dependencies");
        }
        run(
            "npm",
            &["run", "build", "--workspace", "@sagat/studio"],
            &frontend,
            &[],
        )
        .await
        .expect("building the Studio");
    }
    let dist = frontend.join("apps/studio/dist");
    assert!(
        dist.join("index.html").exists(),
        "no build at {}",
        dist.display()
    );
    dist
}

fn token(platform: &crate::support::worker::Platform, subject: &str, tenant: &str) -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        + 3500;
    platform
        .api
        .idp
        .token(subject, tenant)
        .claim("exp", serde_json::json!(exp))
        .build()
}

#[tokio::test]
#[ignore = "needs Node, the frontend's dependencies and Playwright's Chromium: the studio job of CI runs it with --ignored"]
async fn the_studio_journeys_pass_in_a_real_browser() {
    let dist = build_studio().await;
    // The TypeScript SDK the journeys' workers use (built by the conformance suite's own script).
    run(
        "bash",
        &[repo()
            .join("sdks/typescript/conformance/prepare.sh")
            .to_str()
            .expect("path")],
        &repo().join("sdks/typescript"),
        &[],
    )
    .await
    .expect("building the TypeScript SDK");

    // The platform of the script suites: the API, the engine's database and a sandbox runner (real
    // containers), because US02 builds and tests a script from the browser.
    let lab = lab_with(Options {
        studio: Some(StudioConfig {
            dir: dist,
            oidc: None,
        }),
        ..Options::default()
    })
    .await;
    let platform = &lab.p;
    // The runner works in the background, the way the production one does (the suites step it).
    let runner = std::sync::Arc::clone(&lab.runner);
    let pump = tokio::spawn(async move {
        loop {
            let _ = runner.build_once().await;
            let _ = runner.test_once().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    for (subject, role) in [
        ("olivia", Role::Operator),
        ("vera", Role::Viewer),
        ("dev", Role::Developer),
        ("studio-worker", Role::Worker),
    ] {
        platform.grant(platform.tenant, subject, role).await;
    }
    platform.bind_everything("studio-worker").await;
    let ring = format!("k1:{}", platform.api.key_k1);
    let engine = RunningEngine::start(engine_env(
        &platform.api.db,
        &platform.api.idp,
        &[
            ("SAGAT_SECRETS_KEYS", ring.as_str()),
            ("SAGAT_EVENT_POLL_MS", "50"),
            ("SAGAT_DELIVERY_POLL_MS", "50"),
        ],
    ))
    .await;

    let tokens = [
        ("STUDIO_TOKEN_ADMIN", token(platform, HUMAN, TENANT)),
        ("STUDIO_TOKEN_DEVELOPER", token(platform, "dev", TENANT)),
        ("STUDIO_TOKEN_OPERATOR", token(platform, "olivia", TENANT)),
        ("STUDIO_TOKEN_VIEWER", token(platform, "vera", TENANT)),
        (
            "STUDIO_TOKEN_WORKER",
            token(platform, "studio-worker", TENANT),
        ),
        ("STUDIO_TOKEN_FOREIGN", token(platform, "bob", "globex")),
    ];
    let origin = platform.api.origin().to_owned();
    let mut env: Vec<(String, String)> = tokens
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    env.push(("STUDIO_ORIGIN".into(), origin));
    env.push(("STUDIO_GRPC_URL".into(), engine.grpc_url.clone()));
    env.push(("STUDIO_NAMESPACE".into(), NAMESPACE.into()));
    env.push(("STUDIO_NAMESPACE_ID".into(), platform.namespace_id().await));

    let mut args = vec!["playwright".to_owned(), "test".to_owned()];
    if let Ok(grep) = std::env::var("SAGAT_STUDIO_GREP") {
        args.push("--grep".into());
        args.push(grep);
    }
    let child = Command::new("npx")
        .args(&args)
        .current_dir(repo().join("frontend/e2e"))
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("npx playwright");
    let finished = tokio::time::timeout(Duration::from_secs(40 * 60), async move {
        child.wait_with_output().await
    })
    .await
    .expect("the journeys finish within 40 minutes")
    .expect("playwright ran");
    pump.abort();
    assert!(
        finished.status.success(),
        "the journeys failed ({}): see the Playwright output above and frontend/e2e/test-results",
        finished.status
    );
}
