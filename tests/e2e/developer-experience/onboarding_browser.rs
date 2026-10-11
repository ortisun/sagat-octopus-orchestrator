//! US01's complete first-day path in a real browser, against a native test platform and the
//! production development identity provider. This is separate from the Compose smoke so it can
//! prove the wiring on a machine whose Docker daemon is unavailable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sagat_api_http::studio::{StudioConfig, StudioLimits};
use sagat_cli::Env;
use sagat_control_plane::config::{
    Config as ControlConfig, DatabaseConfig, Environment, NamespaceSeed, SeedConfig,
};
use sagat_control_plane::{seed_dev, seed_dev_namespace};
use sagat_dev_idp::{Config as IdpConfig, DevKey};
use sagat_telemetry::LogFormat;
use serde_json::{Value, json};
use tokio::process::Command;

use crate::support::api::{Options, start_with};
use crate::support::worker::{RunningEngine, engine_env};
use crate::{Dir, run_with};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

struct DevIdp {
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl DevIdp {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind dev idp");
        let addr = listener.local_addr().expect("dev idp address");
        let origin = format!("http://{addr}");
        let values = [
            ("SAGAT_ENV", "dev".to_owned()),
            ("SAGAT_DEV_IDP_ADDR", addr.to_string()),
            ("SAGAT_DEV_IDP_ISSUER", format!("{origin}/")),
        ];
        let config = IdpConfig::from_lookup(&|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .expect("dev idp config");
        let app = sagat_dev_idp::router(config, DevKey::generate().expect("dev idp key"));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { origin, task }
    }

    fn issuer(&self) -> String {
        format!("{}/", self.origin)
    }

    fn jwks(&self) -> String {
        format!("{}/.well-known/jwks.json", self.origin)
    }

    async fn token(&self) -> String {
        let response = reqwest::Client::new()
            .post(format!("{}/token", self.origin))
            .json(&json!({"subject": "dev-admin", "tenant": "dev"}))
            .send()
            .await
            .expect("ask the development identity provider for a token");
        assert_eq!(
            response.status(),
            200,
            "development identity provider refused the test identity"
        );
        response.json::<Value>().await.expect("token response")["access_token"]
            .as_str()
            .expect("access token")
            .to_owned()
    }
}

impl Drop for DevIdp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn studio_dist() -> PathBuf {
    if let Some(path) = std::env::var_os("SAGAT_STUDIO_DIST") {
        return PathBuf::from(path);
    }
    let local = repo().join("frontend/apps/studio/dist");
    if !local.join("index.html").is_file() {
        let frontend = repo().join("frontend");
        let built = child(
            Path::new("npm"),
            &["run", "build", "--workspace", "@sagat/studio"],
            &frontend,
            &HashMap::new(),
            180,
        )
        .await;
        assert!(
            built.status.success(),
            "Studio build failed with {}",
            built.status
        );
    }
    local
}

async fn child(
    program: &Path,
    args: &[&str],
    dir: &Path,
    env: &HashMap<&str, String>,
    seconds: u64,
) -> std::process::Output {
    let process = Command::new(program)
        .args(args)
        .current_dir(dir)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap_or_else(|error| panic!("cannot start {}: {error}", program.display()));
    tokio::time::timeout(Duration::from_secs(seconds), process.wait_with_output())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{} did not finish within {seconds} seconds",
                program.display()
            )
        })
        .expect("wait for child")
}

#[tokio::test]
#[ignore = "needs Node, the built Studio, frontend dependencies and Playwright Chromium"]
async fn us01_compose_cli_sdk_and_studio_are_one_real_first_day_path() {
    let dist = studio_dist().await;
    assert!(
        dist.join("index.html").is_file(),
        "build the Studio or set SAGAT_STUDIO_DIST (looked in {})",
        dist.display()
    );

    let idp = DevIdp::start().await;
    let issuer = idp.issuer();
    let jwks = idp.jwks();
    let api = start_with(Options {
        auth: Box::new(move |auth| {
            auth.issuer = issuer;
            auth.jwks_uri = jwks;
        }),
        studio: Some(StudioConfig {
            dir: dist,
            oidc: None,
            limits: StudioLimits::default(),
        }),
        ..Options::default()
    })
    .await;

    let control = ControlConfig {
        environment: Environment::Dev,
        log_format: LogFormat::Json,
        database: DatabaseConfig {
            url: api.db.url.clone(),
            max_connections: 5,
            statement_timeout: Duration::from_secs(3),
        },
    };
    let seed = SeedConfig {
        tenant_slug: "dev".into(),
        admin_subject: "dev-admin".into(),
    };
    seed_dev(&control, &seed).await.expect("seed tenant");
    seed_dev_namespace(
        &control,
        &seed,
        &NamespaceSeed {
            workspace_slug: "dev".into(),
            namespace_slug: "payments".into(),
            environment: "dev".into(),
        },
    )
    .await
    .expect("seed namespace");

    let token = idp.token().await;
    let dir = Dir::new();
    let cli_env = Env {
        server: Some(api.base.clone()),
        token: Some(token.clone()),
        namespace: Some("payments".into()),
        cwd: dir.0.clone(),
        ..Env::default()
    };
    run_with(cli_env.clone(), &["new", "hello-world"])
        .await
        .expect(0);
    run_with(
        cli_env.clone(),
        &["workflow", "publish", "hello-world/workflow.json"],
    )
    .await
    .expect(0);
    run_with(
        cli_env,
        &["worker", "bind", "--workflow", "hello-world/workflow.json"],
    )
    .await
    .expect(0);

    let ring = format!("k1:{}", api.key_k1);
    let real_issuer = idp.issuer();
    let real_jwks = idp.jwks();
    let engine = RunningEngine::start(engine_env(
        &api.db,
        &api.idp,
        &[
            ("SAGAT_OIDC_ISSUER", real_issuer.as_str()),
            ("SAGAT_OIDC_JWKS_URI", real_jwks.as_str()),
            ("SAGAT_SECRETS_KEYS", ring.as_str()),
            ("SAGAT_EVENT_POLL_MS", "50"),
            ("SAGAT_DELIVERY_POLL_MS", "50"),
        ],
    ))
    .await;

    let example = repo().join("examples/hello-world/typescript");
    let empty = HashMap::new();
    let prepared = child(&example.join("prepare.sh"), &[], &example, &empty, 180).await;
    assert!(
        prepared.status.success(),
        "TypeScript example preparation failed with {}",
        prepared.status
    );
    let example_env = HashMap::from([
        ("SAGAT_SERVER", api.base.clone()),
        ("SAGAT_GRPC", engine.grpc_url.clone()),
        ("SAGAT_NAMESPACE", "payments".to_owned()),
        ("SAGAT_TOKEN", token.clone()),
    ]);
    let ran = child(
        &example.join("run.sh"),
        &["Ada"],
        &example,
        &example_env,
        90,
    )
    .await;
    assert!(
        ran.status.success(),
        "TypeScript SDK example failed with {}",
        ran.status
    );
    let output = String::from_utf8_lossy(&ran.stdout);
    assert!(
        output.contains("delivering Hello, Ada!"),
        "the SDK did not execute both real activities"
    );
    let run_id = output
        .lines()
        .find_map(|line| line.strip_prefix("run ")?.strip_suffix(" started"))
        .expect("SDK run id");
    assert!(
        output
            .lines()
            .any(|line| line == format!("run {run_id} SUCCEEDED")),
        "the SDK run did not succeed"
    );

    let frontend = repo().join("frontend");
    let browser_env = HashMap::from([
        ("STUDIO_ORIGIN", format!("{}/studio/", api.origin())),
        ("STUDIO_RUN_ID", run_id.to_owned()),
        ("STUDIO_TOKEN", token),
    ]);
    let browser = child(
        Path::new("npx"),
        &["playwright", "test", "--config", "e2e/compose.config.ts"],
        &frontend,
        &browser_env,
        180,
    )
    .await;
    if !browser.status.success() {
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&browser.stdout),
            String::from_utf8_lossy(&browser.stderr)
        )
        .replace(
            browser_env.get("STUDIO_TOKEN").expect("token in memory"),
            "<redacted>",
        );
        panic!(
            "compose onboarding browser journey failed with {}:\n{said}",
            browser.status
        );
    }
}
