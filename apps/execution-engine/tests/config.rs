//! Configuration and command-line parsing of the engine. No environment, no network.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use sagat_control_plane::config::{ConfigError, Environment};
use sagat_execution_engine::EngineConfig;
use sagat_execution_engine::cli::{self, Command, UsageError};

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

const OIDC: [(&str, &str); 3] = [
    ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
    ("SAGAT_OIDC_AUDIENCE", "sagat"),
    ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
];

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    OIDC.iter().chain(extra.iter()).copied().collect()
}

fn parse(extra: &[(&'static str, &'static str)]) -> Result<EngineConfig, ConfigError> {
    EngineConfig::from_lookup(&env(&with(extra)), Environment::Prod)
}

#[test]
fn defaults_listen_on_loopback_and_publish_nothing() {
    let config = parse(&[]).expect("config");
    assert_eq!(config.grpc_addr, SocketAddr::from(([127, 0, 0, 1], 9000)));
    assert_eq!(
        config.admin_addr,
        Some(SocketAddr::from(([127, 0, 0, 1], 9091)))
    );
    assert!(config.nats.is_none(), "no NATS unless asked for");
    assert_eq!(config.worker.lease_ttl, Duration::from_secs(30));
    assert_eq!(config.tick_interval, Duration::from_millis(500));
    assert_eq!(config.scheduler.batch, 200);
    assert_eq!(config.dispatch.batch, 100);
}

#[test]
fn worker_budgets_are_larger_than_the_rest_api_s_but_finite() {
    let config = parse(&[]).expect("config");
    assert!(config.rate_limit.per_second >= 1000.0 && config.rate_limit.per_second.is_finite());
    assert!(config.subject_rate_limit.per_second >= 100.0);
    let tight = parse(&[
        ("SAGAT_RATE_LIMIT_RPS", "5"),
        ("SAGAT_RATE_LIMIT_BURST", "7"),
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "2"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "3"),
    ])
    .expect("config");
    assert_eq!(
        (tight.rate_limit.per_second, tight.rate_limit.burst),
        (5.0, 7.0)
    );
    assert_eq!(
        (
            tight.subject_rate_limit.per_second,
            tight.subject_rate_limit.burst
        ),
        (2.0, 3.0)
    );
}

#[test]
fn a_budget_that_could_never_admit_a_call_is_refused_by_name() {
    for (name, value) in [
        ("SAGAT_RATE_LIMIT_RPS", "0"),
        ("SAGAT_RATE_LIMIT_BURST", "0.5"),
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "-1"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "nan"),
    ] {
        match parse(&[(name, value)]).expect_err(name) {
            ConfigError::Invalid { name: reported, .. } => assert_eq!(reported, name),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn identity_settings_are_required_like_the_api_s() {
    let err = EngineConfig::from_lookup(&env(&[]), Environment::Prod).expect_err("missing");
    assert_eq!(err, ConfigError::Missing("SAGAT_OIDC_ISSUER"));
}

#[test]
fn intervals_and_sizes_must_be_positive_numbers() {
    for name in [
        "SAGAT_GRPC_REQUEST_TIMEOUT_SECS",
        "SAGAT_LEASE_TTL_SECS",
        "SAGAT_SESSION_TTL_SECS",
        "SAGAT_POLL_IDLE_RETRY_MS",
        "SAGAT_SCHEDULER_INTERVAL_MS",
        "SAGAT_SCHEDULER_BATCH",
        "SAGAT_HOUSEKEEPING_INTERVAL_SECS",
        "SAGAT_LIVENESS_GRACE_SECS",
        "SAGAT_OUTBOX_BATCH",
        "SAGAT_OUTBOX_POLL_MS",
    ] {
        for bad in ["0", "-3", "soon"] {
            let extra: &[(&'static str, &'static str)] = &[(name, bad)];
            match parse(extra).expect_err(name) {
                ConfigError::Invalid { name: reported, .. } => assert_eq!(reported, name),
                other => panic!("{name}={bad}: {other:?}"),
            }
        }
    }
    let config = parse(&[
        ("SAGAT_LEASE_TTL_SECS", "5"),
        ("SAGAT_SCHEDULER_INTERVAL_MS", "50"),
        ("SAGAT_SCHEDULER_BATCH", "7"),
        ("SAGAT_OUTBOX_BATCH", "9"),
    ])
    .expect("config");
    assert_eq!(config.worker.lease_ttl, Duration::from_secs(5));
    assert_eq!(config.tick_interval, Duration::from_millis(50));
    assert_eq!(config.scheduler.batch, 7);
    assert_eq!(config.dispatch.batch, 9);
}

#[test]
fn nats_is_optional_and_plaintext_nats_is_a_development_convenience() {
    let url = [("SAGAT_NATS_URL", "nats://user:pw@nats.internal:4222")];
    let prod = EngineConfig::from_lookup(&env(&with(&url)), Environment::Prod).expect("config");
    let nats = prod.nats.expect("configured");
    assert!(
        nats.require_tls,
        "TLS is required outside development by default"
    );
    assert_eq!(nats.stream, "SAGAT");
    let dev = EngineConfig::from_lookup(&env(&with(&url)), Environment::Dev).expect("config");
    assert!(!dev.nats.expect("configured").require_tls);
    let explicit = EngineConfig::from_lookup(
        &env(&with(&[
            ("SAGAT_NATS_URL", "nats://x:4222"),
            ("SAGAT_NATS_REQUIRE_TLS", "false"),
            ("SAGAT_NATS_STREAM", "OTHER"),
            ("SAGAT_NATS_CREDENTIALS_FILE", " /etc/nats/engine.creds "),
        ])),
        Environment::Prod,
    )
    .expect("config");
    let nats = explicit.nats.expect("configured");
    assert!(!nats.require_tls, "an operator may opt out explicitly");
    assert_eq!(nats.stream, "OTHER");
    assert_eq!(
        nats.credentials_file.as_deref(),
        Some("/etc/nats/engine.creds")
    );
    assert!(matches!(
        EngineConfig::from_lookup(
            &env(&with(&[
                ("SAGAT_NATS_URL", "nats://x"),
                ("SAGAT_NATS_REQUIRE_TLS", "maybe")
            ])),
            Environment::Prod
        ),
        Err(ConfigError::Invalid {
            name: "SAGAT_NATS_REQUIRE_TLS",
            ..
        })
    ));
    // A blank URL means "not configured", not "configured with nothing".
    assert!(
        parse(&[("SAGAT_NATS_URL", "   ")])
            .expect("config")
            .nats
            .is_none()
    );
}

#[test]
fn the_nats_url_never_appears_in_debug_output() {
    let config =
        parse(&[("SAGAT_NATS_URL", "nats://user:s3cret@nats.internal:4222")]).expect("config");
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("s3cret") && !shown.contains("nats.internal"),
        "{shown}"
    );
}

#[test]
fn the_admin_listener_can_be_moved_or_switched_off() {
    assert_eq!(
        parse(&[("SAGAT_ADMIN_ADDR", "off")])
            .expect("config")
            .admin_addr,
        None
    );
    assert_eq!(
        parse(&[("SAGAT_ADMIN_ADDR", "0.0.0.0:9191")])
            .expect("config")
            .admin_addr,
        Some(SocketAddr::from(([0, 0, 0, 0], 9191)))
    );
    assert!(matches!(
        parse(&[("SAGAT_ADMIN_ADDR", "nowhere")]),
        Err(ConfigError::Invalid {
            name: "SAGAT_ADMIN_ADDR",
            ..
        })
    ));
    assert!(matches!(
        parse(&[("SAGAT_GRPC_ADDR", "not an address")]),
        Err(ConfigError::Invalid {
            name: "SAGAT_GRPC_ADDR",
            ..
        })
    ));
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn commands_are_parsed_strictly() {
    assert_eq!(cli::parse(&args(&[])), Ok(Command::Serve));
    assert_eq!(cli::parse(&args(&["serve"])), Ok(Command::Serve));
    assert_eq!(
        cli::parse(&args(&["healthcheck"])),
        Ok(Command::Healthcheck)
    );
    assert_eq!(cli::parse(&args(&["--help"])), Ok(Command::Help));
    assert_eq!(cli::parse(&args(&["-V"])), Ok(Command::Version));
    assert_eq!(
        cli::parse(&args(&["migrate"])),
        Err(UsageError("unknown command 'migrate'".into())),
        "the engine never migrates: that needs the schema owner"
    );
    assert_eq!(
        cli::parse(&args(&["serve", "--now"])),
        Err(UsageError("unexpected argument '--now'".into()))
    );
}
