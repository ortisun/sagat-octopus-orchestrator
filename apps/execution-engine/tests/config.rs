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
        "SAGAT_GRPC_MAX_CONNECTIONS",
        "SAGAT_GRPC_MAX_CONNECTIONS_PER_PEER",
        "SAGAT_GRPC_FIRST_BYTE_TIMEOUT_SECS",
        "SAGAT_GRPC_IDLE_TIMEOUT_SECS",
        "SAGAT_GRPC_TLS_HANDSHAKE_TIMEOUT_SECS",
        "SAGAT_GRPC_MAX_CONNECTION_AGE_SECS",
        "SAGAT_GRPC_MAX_IN_FLIGHT",
        "SAGAT_LEASE_TTL_SECS",
        "SAGAT_SESSION_TTL_SECS",
        "SAGAT_COMPENSATION_BREAKER_OPEN_SECS",
        "SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS",
        "SAGAT_POLL_IDLE_RETRY_MS",
        "SAGAT_SCHEDULER_INTERVAL_MS",
        "SAGAT_SCHEDULER_BATCH",
        "SAGAT_SCHEDULER_CONCURRENCY",
        "SAGAT_SHARD_LEASE_SECS",
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
        ("SAGAT_COMPENSATION_BREAKER_THRESHOLD", "3"),
        ("SAGAT_COMPENSATION_BREAKER_OPEN_SECS", "10"),
        ("SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS", "90"),
        ("SAGAT_SCHEDULER_INTERVAL_MS", "50"),
        ("SAGAT_SCHEDULER_BATCH", "7"),
        ("SAGAT_SCHEDULER_CONCURRENCY", "3"),
        ("SAGAT_SHARD_LEASE_SECS", "21"),
        ("SAGAT_OUTBOX_BATCH", "9"),
        ("SAGAT_GRPC_MAX_CONNECTIONS", "11"),
        ("SAGAT_GRPC_MAX_CONNECTIONS_PER_PEER", "3"),
        ("SAGAT_GRPC_FIRST_BYTE_TIMEOUT_SECS", "4"),
        ("SAGAT_GRPC_IDLE_TIMEOUT_SECS", "21"),
        ("SAGAT_GRPC_TLS_HANDSHAKE_TIMEOUT_SECS", "6"),
        ("SAGAT_GRPC_MAX_CONNECTION_AGE_SECS", "900"),
        ("SAGAT_GRPC_MAX_IN_FLIGHT", "13"),
    ])
    .expect("config");
    assert_eq!(config.worker.lease_ttl, Duration::from_secs(5));
    assert_eq!(config.breaker.threshold, 3);
    assert_eq!(config.breaker.open_for, Duration::from_secs(10));
    assert_eq!(config.breaker.max_open, Duration::from_secs(90));
    assert_eq!(config.tick_interval, Duration::from_millis(50));
    assert_eq!(config.scheduler.batch, 7);
    assert_eq!(config.scheduler.concurrency, 3);
    assert_eq!(config.shards.ttl, Duration::from_secs(21));
    // A fifth of the lease: two rounds in a row can fail before it ends.
    assert_eq!(config.shards.interval(), Duration::from_millis(4_200));
    assert_eq!(config.dispatch.batch, 9);
    assert_eq!(config.grpc.max_connections, 11);
    assert_eq!(config.grpc.max_connections_per_peer, 3);
    assert_eq!(config.grpc.first_byte_timeout, Duration::from_secs(4));
    assert_eq!(config.grpc.idle_timeout, Duration::from_secs(21));
    assert_eq!(config.grpc.handshake_timeout, Duration::from_secs(6));
    assert_eq!(config.grpc.max_connection_age, Duration::from_secs(900));
    assert_eq!(config.grpc.max_in_flight, 13);
}

#[test]
fn the_connection_limits_default_to_values_that_close_a_silent_socket_in_seconds() {
    let grpc = parse(&[]).expect("config").grpc;
    // A socket that never speaks must not hold a place for long, a connection that is merely
    // between calls must not be cut off, and no address may hold more than a share of the places.
    assert!(grpc.first_byte_timeout <= Duration::from_secs(30));
    assert!(grpc.handshake_timeout <= Duration::from_secs(30));
    assert!(grpc.idle_timeout >= Duration::from_secs(60));
    assert!(grpc.max_connections_per_peer * 4 <= grpc.max_connections);
    assert!(grpc.max_connection_age >= grpc.idle_timeout);
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

// ---------------------------------------------------------------- TLS and secrets (feature 009)

/// A file with `contents` that lives as long as the test process, in a directory of its own.
fn pem(name: &str, contents: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "sagat-engine-config-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("file.pem");
    std::fs::write(&path, contents).expect("write");
    path.to_str().expect("utf-8 path").to_owned()
}

const FAKE_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
const KEYS: &str = "k1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn parse_with(
    pairs: &[(&str, &str)],
    environment: Environment,
) -> Result<EngineConfig, ConfigError> {
    let mut all: Vec<(&str, &str)> = OIDC.to_vec();
    all.extend_from_slice(pairs);
    EngineConfig::from_lookup(&env(&all), environment)
}

#[test]
fn tls_needs_the_certificate_and_the_key_together_and_mutual_tls_needs_both() {
    let cert = pem("cert", FAKE_PEM);
    let key = pem("key", FAKE_PEM);
    let ca = pem("ca", FAKE_PEM);

    let ok = parse_with(
        &[
            ("SAGAT_GRPC_TLS_CERT_FILE", &cert),
            ("SAGAT_GRPC_TLS_KEY_FILE", &key),
        ],
        Environment::Prod,
    )
    .expect("config");
    let tls = ok.grpc.tls.expect("tls");
    assert!(tls.client_ca_pem.is_none(), "server TLS only");
    assert!(!format!("{tls:?}").contains("MIIB"), "the key never prints");

    let mutual = parse_with(
        &[
            ("SAGAT_GRPC_TLS_CERT_FILE", &cert),
            ("SAGAT_GRPC_TLS_KEY_FILE", &key),
            ("SAGAT_GRPC_TLS_CLIENT_CA_FILE", &ca),
        ],
        Environment::Prod,
    )
    .expect("config");
    assert!(mutual.grpc.tls.expect("tls").client_ca_pem.is_some());

    for (what, pairs, name) in [
        (
            "a certificate without a key",
            vec![("SAGAT_GRPC_TLS_CERT_FILE", cert.as_str())],
            "SAGAT_GRPC_TLS_KEY_FILE",
        ),
        (
            "a key without a certificate",
            vec![("SAGAT_GRPC_TLS_KEY_FILE", key.as_str())],
            "SAGAT_GRPC_TLS_CERT_FILE",
        ),
        (
            "a client CA without a server certificate",
            vec![("SAGAT_GRPC_TLS_CLIENT_CA_FILE", ca.as_str())],
            "SAGAT_GRPC_TLS_CLIENT_CA_FILE",
        ),
    ] {
        match parse_with(&pairs, Environment::Prod) {
            Err(ConfigError::Invalid { name: reported, .. }) => {
                assert_eq!(reported, name, "{what}")
            }
            other => panic!("{what}: {other:?}"),
        }
    }
}

#[test]
fn a_tls_file_that_is_missing_or_not_pem_is_refused_by_name_without_its_contents() {
    let cert = pem("c2", FAKE_PEM);
    let not_pem = pem("not-pem", "this is not a certificate");
    let gone = "/nonexistent/sagat/engine/key.pem";
    for (pairs, name) in [
        (
            vec![
                ("SAGAT_GRPC_TLS_CERT_FILE", cert.as_str()),
                ("SAGAT_GRPC_TLS_KEY_FILE", gone),
            ],
            "SAGAT_GRPC_TLS_KEY_FILE",
        ),
        (
            vec![
                ("SAGAT_GRPC_TLS_CERT_FILE", not_pem.as_str()),
                ("SAGAT_GRPC_TLS_KEY_FILE", cert.as_str()),
            ],
            "SAGAT_GRPC_TLS_CERT_FILE",
        ),
    ] {
        match parse_with(&pairs, Environment::Prod) {
            Err(ConfigError::Invalid {
                name: reported,
                reason,
            }) => {
                assert_eq!(reported, name);
                assert!(!reason.contains("this is not a certificate"), "{reason}");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn secrets_are_not_served_over_plaintext_outside_development_unless_the_operator_says_tls_is_in_front()
 {
    let cert = pem("c3", FAKE_PEM);
    let key = pem("k3", FAKE_PEM);
    let secrets = [("SAGAT_SECRETS_KEYS", KEYS)];

    // Production without TLS and without the statement: refused, and the error names the way out.
    match parse_with(&secrets, Environment::Prod) {
        Err(ConfigError::Invalid { name, reason }) => {
            assert_eq!(name, "SAGAT_SECRETS_KEYS");
            assert!(reason.contains("SAGAT_GRPC_TLS_CERT_FILE"), "{reason}");
            assert!(!reason.contains("AAECAw"), "never the key: {reason}");
        }
        other => panic!("{other:?}"),
    }
    // With TLS, with the statement, or in development: fine.
    assert!(
        parse_with(
            &[
                secrets[0],
                ("SAGAT_GRPC_TLS_CERT_FILE", &cert),
                ("SAGAT_GRPC_TLS_KEY_FILE", &key),
            ],
            Environment::Prod
        )
        .is_ok()
    );
    assert!(
        parse_with(
            &[secrets[0], ("SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS", "true")],
            Environment::Prod
        )
        .is_ok()
    );
    assert!(parse_with(&secrets, Environment::Dev).is_ok());
    // No secrets configured, no requirement.
    assert!(parse_with(&[], Environment::Prod).is_ok());
}

#[test]
fn a_bad_key_ring_is_refused_without_echoing_it() {
    for bad in [
        "not-a-ring",
        "k1:AAAA",
        "K1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=",
    ] {
        match parse_with(&[("SAGAT_SECRETS_KEYS", bad)], Environment::Dev) {
            Err(ConfigError::Invalid { name, reason }) => {
                assert_eq!(name, "SAGAT_SECRETS_KEYS");
                assert!(!reason.contains(bad), "{reason}");
            }
            other => panic!("{bad}: {other:?}"),
        }
    }
    let config = parse_with(&[("SAGAT_SECRETS_KEYS", KEYS)], Environment::Dev).expect("config");
    let printed = format!("{config:?}");
    assert!(
        !printed.contains("AAECAwQF"),
        "the key ring never prints: {printed}"
    );
}

#[test]
fn the_compensation_breaker_can_be_turned_off_and_its_window_must_be_consistent() {
    let off = parse(&[("SAGAT_COMPENSATION_BREAKER_THRESHOLD", "0")]).expect("off is valid");
    assert_eq!(off.breaker.threshold, 0);
    assert_eq!(
        parse(&[]).expect("defaults").breaker,
        sagat_application::execution::BreakerPolicy::default()
    );
    for bad in ["-1", "often"] {
        let error = parse(&[("SAGAT_COMPENSATION_BREAKER_THRESHOLD", bad)]).expect_err(bad);
        assert!(
            matches!(
                error,
                ConfigError::Invalid {
                    name: "SAGAT_COMPENSATION_BREAKER_THRESHOLD",
                    ..
                }
            ),
            "{bad}: {error:?}"
        );
    }
    let error = parse(&[
        ("SAGAT_COMPENSATION_BREAKER_OPEN_SECS", "60"),
        ("SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS", "10"),
    ])
    .expect_err("the cap is below the window");
    assert!(matches!(
        error,
        ConfigError::Invalid {
            name: "SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS",
            ..
        }
    ));
}

#[test]
fn the_shard_lease_is_bounded_and_the_replica_is_named_safely() {
    let defaults = parse(&[]).expect("config");
    assert_eq!(defaults.shards.ttl, Duration::from_secs(15));
    assert_eq!(defaults.scheduler.concurrency, 8);
    assert_eq!(defaults.scheduler.discovery_rest, Duration::from_secs(2));
    assert_eq!(defaults.dispatch.discovery_rest, Duration::from_secs(2));
    let rested = parse(&[("SAGAT_SCHEDULER_DISCOVERY_REST_MS", "0")]).expect("config");
    assert!(rested.scheduler.discovery_rest.is_zero() && rested.dispatch.discovery_rest.is_zero());
    let clamped = parse(&[("SAGAT_SCHEDULER_DISCOVERY_REST_MS", "9999999")]).expect("config");
    assert_eq!(clamped.scheduler.discovery_rest, Duration::from_secs(60));
    assert!(parse(&[("SAGAT_SCHEDULER_DISCOVERY_REST_MS", "soon")]).is_err());
    assert!(
        defaults.scheduler.fair,
        "fairness is on unless it is turned off"
    );
    assert!(
        !parse(&[("SAGAT_SCHEDULER_FAIRNESS", "false")])
            .expect("config")
            .scheduler
            .fair
    );
    assert!(
        parse(&[("SAGAT_SCHEDULER_FAIRNESS", "true")])
            .expect("config")
            .scheduler
            .fair
    );
    assert!(parse(&[("SAGAT_SCHEDULER_FAIRNESS", "maybe")]).is_err());
    assert_eq!(defaults.replica, "engine");
    // Shorter and a pause loses the shards; longer and a crash leaves its work waiting for minutes.
    for bad in ["2", "601"] {
        match parse(&[("SAGAT_SHARD_LEASE_SECS", bad)]).expect_err(bad) {
            ConfigError::Invalid { name, .. } => assert_eq!(name, "SAGAT_SHARD_LEASE_SECS"),
            other => panic!("{bad}: {other:?}"),
        }
    }
    assert!(parse(&[("SAGAT_SHARD_LEASE_SECS", "3")]).is_ok());
    assert!(parse(&[("SAGAT_SHARD_LEASE_SECS", "600")]).is_ok());
    // A larger concurrency than a database connection each could carry is clamped, not refused.
    let big = parse(&[("SAGAT_SCHEDULER_CONCURRENCY", "100000")]).expect("config");
    assert_eq!(big.scheduler.concurrency, 64);
    // The replica's name goes into rows and logs: short, printable, no spaces.
    let named = parse(&[("SAGAT_REPLICA_ID", " engine-7f9c ")]).expect("config");
    assert_eq!(named.replica, "engine-7f9c");
    // 65 characters: one more than a replica's name may have.
    const LONG: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
    assert_eq!(LONG.len(), 65);
    for bad in ["has space", "tab\there", "ünï", LONG] {
        match parse(&[("SAGAT_REPLICA_ID", bad)]).expect_err(bad) {
            ConfigError::Invalid { name, .. } => assert_eq!(name, "SAGAT_REPLICA_ID"),
            other => panic!("{bad}: {other:?}"),
        }
    }
}

#[test]
fn worker_pools_are_named_by_the_operator_and_bounded() {
    assert!(
        parse(&[])
            .expect("config")
            .monitor
            .settings
            .pools
            .is_empty()
    );
    let config = parse(&[(
        "SAGAT_WORKER_POOLS",
        " gpu = act.render , act.upscale ; batch=act.export;",
    )])
    .expect("config");
    let pools = config.monitor.settings.pools;
    assert_eq!(pools.len(), 2);
    assert_eq!(pools[0].name, "gpu");
    assert_eq!(pools[0].activities, ["act.render", "act.upscale"]);
    assert_eq!(pools[1].name, "batch");
    for bad in [
        "no-equals-sign",
        "=act.a",
        "UPPER=act.a",
        "has space=act.a",
        "empty=",
        "empty=, ,",
        "twice=act.a;twice=act.b",
        "ünï=act.a",
        "p=act\u{1}",
    ] {
        match parse(&[("SAGAT_WORKER_POOLS", bad)]).expect_err(bad) {
            ConfigError::Invalid { name, .. } => assert_eq!(name, "SAGAT_WORKER_POOLS"),
            other => panic!("{bad}: {other:?}"),
        }
    }
}

#[test]
fn too_many_pools_or_activities_are_refused() {
    let many: String = (0..33).map(|n| format!("p{n}=act.a;")).collect();
    let too_many: &'static str = Box::leak(many.into_boxed_str());
    assert!(parse(&[("SAGAT_WORKER_POOLS", too_many)]).is_err());
    let list: String = (0..513).map(|n| format!("act.{n},")).collect();
    let spec: &'static str = Box::leak(format!("big={list}").into_boxed_str());
    assert!(parse(&[("SAGAT_WORKER_POOLS", spec)]).is_err());
    // Exactly at the limits is fine.
    let ok: String = (0..32).map(|n| format!("p{n}=act.a;")).collect();
    let ok: &'static str = Box::leak(ok.into_boxed_str());
    assert!(parse(&[("SAGAT_WORKER_POOLS", ok)]).is_ok());
}

#[test]
fn a_batch_a_database_should_not_be_asked_for_is_refused_by_name() {
    for name in ["SAGAT_SCHEDULER_BATCH", "SAGAT_OUTBOX_BATCH"] {
        assert!(parse(&[(name, "5000")]).is_ok(), "{name} at the limit");
        match parse(&[(name, "5001")]).expect_err(name) {
            ConfigError::Invalid { name: reported, .. } => assert_eq!(reported, name),
            other => panic!("{name}: {other:?}"),
        }
        assert!(
            parse(&[(name, "4294967295")]).is_err(),
            "{name} far beyond it"
        );
    }
}
