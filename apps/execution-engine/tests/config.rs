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
fn schedules_are_stored_but_fire_nowhere_until_the_flag_is_set() {
    let config = parse(&[]).expect("config");
    assert!(!config.schedules_enabled, "off by default");
    assert_eq!(config.schedules.batch, 100);
    assert_eq!(config.schedules.round_actions, 100);
    assert_eq!(config.schedule_poll, Duration::from_secs(1));

    let on = parse(&[
        ("SAGAT_SCHEDULES_ENABLED", "true"),
        ("SAGAT_SCHEDULES_BATCH", "250"),
        ("SAGAT_SCHEDULES_ROUND_ACTIONS", "1000"),
        ("SAGAT_SCHEDULES_POLL_MS", "400"),
    ])
    .expect("config");
    assert!(on.schedules_enabled);
    assert_eq!(on.schedules.batch, 250);
    assert_eq!(on.schedules.round_actions, 1000);
    assert_eq!(on.schedule_poll, Duration::from_millis(400));
}

#[test]
fn a_mistyped_schedule_setting_is_refused_not_guessed() {
    for (name, value) in [
        ("SAGAT_SCHEDULES_ENABLED", "maybe"),
        ("SAGAT_SCHEDULES_BATCH", "0"),
        ("SAGAT_SCHEDULES_BATCH", "5001"),
        ("SAGAT_SCHEDULES_ROUND_ACTIONS", "0"),
        ("SAGAT_SCHEDULES_ROUND_ACTIONS", "1001"),
        ("SAGAT_SCHEDULES_POLL_MS", "0"),
        ("SAGAT_SCHEDULES_POLL_MS", "soon"),
    ] {
        let error = parse(&[(name, value)]).expect_err("refused");
        assert!(
            error.to_string().contains(name),
            "{name}={value} was refused without naming the variable: {error}"
        );
    }
}

#[test]
fn batch_jobs_are_stored_but_run_nowhere_until_the_flag_is_set() {
    let config = parse(&[]).expect("config");
    assert!(!config.batches_enabled, "off by default");
    assert_eq!(config.batches.batch, 100);
    assert_eq!(config.batches.round_actions, 100);
    assert_eq!(config.batch_poll, Duration::from_secs(1));

    let on = parse(&[
        ("SAGAT_BATCHES_ENABLED", "true"),
        ("SAGAT_BATCHES_BATCH", "250"),
        ("SAGAT_BATCHES_ROUND_ACTIONS", "1000"),
        ("SAGAT_BATCHES_POLL_MS", "400"),
    ])
    .expect("config");
    assert!(on.batches_enabled);
    assert_eq!(on.batches.batch, 250);
    assert_eq!(on.batches.round_actions, 1000);
    assert_eq!(on.batch_poll, Duration::from_millis(400));
}

#[test]
fn a_mistyped_batch_setting_is_refused_not_guessed() {
    for (name, value) in [
        ("SAGAT_BATCHES_ENABLED", "maybe"),
        ("SAGAT_BATCHES_BATCH", "0"),
        ("SAGAT_BATCHES_BATCH", "5001"),
        ("SAGAT_BATCHES_ROUND_ACTIONS", "0"),
        ("SAGAT_BATCHES_ROUND_ACTIONS", "1001"),
        ("SAGAT_BATCHES_POLL_MS", "0"),
        ("SAGAT_BATCHES_POLL_MS", "soon"),
    ] {
        let error = parse(&[(name, value)]).expect_err("refused");
        assert!(
            error.to_string().contains(name),
            "{name}={value} was refused without naming the variable: {error}"
        );
    }
}

#[test]
fn resets_that_wait_for_their_source_are_stored_but_completed_nowhere_until_the_flag_is_set() {
    let config = parse(&[]).expect("config");
    assert!(!config.reset_cancel_source_enabled, "off by default");
    assert_eq!(config.resets.batch, 100);
    assert_eq!(config.resets.per_tenant, 10);
    assert_eq!(config.reset_poll, Duration::from_secs(1));
    // it is not one of the other switches, and they are not it
    assert!(!config.batches_enabled && !config.schedules_enabled);

    let on = parse(&[
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true"),
        ("SAGAT_RESET_CANCEL_SOURCE_BATCH", "250"),
        ("SAGAT_RESET_CANCEL_SOURCE_PER_TENANT", "3"),
        ("SAGAT_RESET_CANCEL_SOURCE_POLL_MS", "400"),
    ])
    .expect("config");
    assert_eq!(on.resets.per_tenant, 3);
    assert!(on.reset_cancel_source_enabled);
    assert!(!on.batches_enabled && !on.schedules_enabled);
    assert_eq!(on.resets.batch, 250);
    assert_eq!(on.reset_poll, Duration::from_millis(400));
    let top = parse(&[("SAGAT_RESET_CANCEL_SOURCE_BATCH", "1000")]).expect("config");
    assert_eq!(top.resets.batch, 1000);
}

#[test]
fn a_mistyped_reset_setting_is_refused_not_guessed() {
    for (name, value) in [
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "maybe"),
        ("SAGAT_RESET_CANCEL_SOURCE_BATCH", "0"),
        ("SAGAT_RESET_CANCEL_SOURCE_BATCH", "1001"),
        ("SAGAT_RESET_CANCEL_SOURCE_BATCH", "many"),
        ("SAGAT_RESET_CANCEL_SOURCE_PER_TENANT", "0"),
        ("SAGAT_RESET_CANCEL_SOURCE_PER_TENANT", "1001"),
        ("SAGAT_RESET_CANCEL_SOURCE_PER_TENANT", "all"),
        ("SAGAT_RESET_CANCEL_SOURCE_POLL_MS", "0"),
        ("SAGAT_RESET_CANCEL_SOURCE_POLL_MS", "soon"),
    ] {
        let error = parse(&[(name, value)]).expect_err("refused");
        assert!(
            error.to_string().contains(name),
            "{name}={value} was refused without naming the variable: {error}"
        );
    }
}

/// The window of the expiry of a reset that waits (D2059, OD-2026-11-08-1): a day unless set, an hour to seven days
/// accepted, both ends included.
#[test]
fn a_reset_that_waits_expires_after_a_day_unless_the_deployment_sets_a_window_from_an_hour_to_seven_days()
 {
    const NAME: &str = "SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS";
    let hour = 3_600;
    let week = 7 * 24 * 3_600;
    assert_eq!(
        parse(&[]).expect("config").resets.expiry,
        Duration::from_secs(24 * 3_600),
        "a day by default"
    );
    // empty is unset, as for every other setting
    assert_eq!(
        parse(&[(NAME, "")]).expect("config").resets.expiry,
        Duration::from_secs(24 * 3_600)
    );
    for (given, held) in [
        ("3600", hour),
        ("3601", hour + 1),
        ("5400", 5_400),
        ("86400", 24 * 3_600),
        (" 7200 ", 7_200),
        ("604799", week - 1),
        ("604800", week),
    ] {
        let config = parse(&[(NAME, given)]).expect("config");
        assert_eq!(
            config.resets.expiry,
            Duration::from_secs(held),
            "{NAME}={given}"
        );
    }
    // the window does not move the other settings of the pass, and they do not move it
    let tuned = parse(&[
        ("SAGAT_RESET_CANCEL_SOURCE_BATCH", "7"),
        ("SAGAT_RESET_CANCEL_SOURCE_PER_TENANT", "2"),
    ])
    .expect("config");
    assert_eq!(tuned.resets.expiry, Duration::from_secs(24 * 3_600));
    let wide = parse(&[(NAME, "7200")]).expect("config");
    assert_eq!((wide.resets.batch, wide.resets.per_tenant), (100, 10));
    // the flag is not what sets it, and turning the pass off does not change what the window reads
    let on = parse(&[("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true")]).expect("config");
    assert_eq!(on.resets.expiry, Duration::from_secs(24 * 3_600));
}

#[test]
fn a_window_outside_an_hour_to_seven_days_or_not_a_whole_number_of_seconds_refuses_to_start_naming_the_variable()
 {
    const NAME: &str = "SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS";
    for given in [
        "0",
        "1",
        "3599",
        "604801",
        "31536000",
        "-3600",
        "24h",
        "1d",
        "86400s",
        "1.5",
        "86400.0",
        "soon",
        "99999999999999999999999",
    ] {
        let error = parse(&[(NAME, given)]).expect_err(given);
        assert!(
            error.to_string().contains(NAME),
            "{NAME}={given} was refused without naming the variable: {error}"
        );
    }
    // an out-of-range value is a refusal and not a clamp: the message says the bounds
    let message = parse(&[(NAME, "3599")]).expect_err("too short").to_string();
    assert!(
        message.contains("3600") && message.contains("604800"),
        "{message}"
    );
    // it refuses even where the pass is off: a deployment that sets a window means it
    assert!(parse(&[(NAME, "60"), ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "false")]).is_err());
}

#[test]
fn the_usage_lists_the_settings_of_the_pass_that_completes_pending_resets() {
    for variable in [
        "SAGAT_RESET_CANCEL_SOURCE_ENABLED",
        "SAGAT_RESET_CANCEL_SOURCE_BATCH",
        "SAGAT_RESET_CANCEL_SOURCE_PER_TENANT",
        "SAGAT_RESET_CANCEL_SOURCE_POLL_MS",
        "SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS",
    ] {
        assert!(
            cli::USAGE.contains(variable),
            "--help does not mention {variable}"
        );
    }
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
    assert_eq!(
        defaults.scheduler.discovery_rest,
        Duration::from_millis(100)
    );
    assert_eq!(defaults.dispatch.discovery_rest, Duration::from_millis(100));
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

#[test]
fn blob_storage_is_opt_in_and_ephemeral_storage_is_development_only() {
    assert!(parse(&[]).expect("defaults").blobs.is_none());
    assert!(parse(&[("SAGAT_BLOB_PROVIDER", "memory")]).is_err());
    let configured = EngineConfig::from_lookup(
        &env(&with(&[("SAGAT_BLOB_PROVIDER", "memory")])),
        Environment::Dev,
    )
    .expect("development memory backend");
    assert!(configured.blobs.is_some());
    assert_eq!(
        configured.blob_transfer.timeout,
        configured.grpc.transfer_timeout
    );
    assert!(parse(&[("SAGAT_BLOB_MAX_TRANSFERS", "0")]).is_err());
    assert!(parse(&[("SAGAT_BLOB_TRANSFER_TIMEOUT_SECS", "0")]).is_err());
}

#[test]
fn insecure_blob_endpoints_cannot_be_enabled_outside_development() {
    let settings = with(&[
        ("SAGAT_BLOB_PROVIDER", "s3"),
        ("SAGAT_BLOB_S3_BUCKET", "test"),
        ("SAGAT_BLOB_S3_REGION", "us-east-1"),
        ("SAGAT_BLOB_S3_ENDPOINT", "http://127.0.0.1:9001"),
        ("SAGAT_BLOB_S3_ALLOW_HTTP", "true"),
    ]);
    for environment in [Environment::Prod, Environment::Staging] {
        let error =
            EngineConfig::from_lookup(&env(&settings), environment).expect_err("TLS required");
        assert!(error.to_string().contains("SAGAT_BLOB_S3_ALLOW_HTTP"));
    }
    assert!(EngineConfig::from_lookup(&env(&settings), Environment::Dev).is_ok());
}

#[test]
fn openbao_workers_require_secret_transport_without_a_local_keyring() {
    let settings = [
        ("SAGAT_OPENBAO_ADDRESS", "https://bao:8200"),
        ("SAGAT_OPENBAO_TOKEN_FILE", "/run/openbao/token"),
    ];
    assert!(parse(&settings).is_err());
    let configured = parse(&[
        settings[0],
        settings[1],
        ("SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS", "true"),
    ])
    .expect("TLS terminated at ingress");
    assert!(configured.openbao.is_some());
    assert!(configured.secrets.is_none());
}

#[test]
fn redis_selection_does_not_publish_to_the_legacy_nats_destination() {
    let config = parse(&[
        ("SAGAT_EVENT_TRANSPORT", "redis"),
        ("SAGAT_REDIS_URL", "rediss://redis.example:6379/0"),
        ("SAGAT_NATS_URL", "nats://legacy.example:4222"),
    ])
    .expect("explicit Redis selection");
    assert!(
        config.nats.is_none(),
        "Redis selection must not start a NATS publisher"
    );
    let redis = config
        .redis
        .expect("selected Redis publisher configuration");
    assert_eq!(redis.stream, "sagat:v1:events");
    assert_eq!(redis.max_len, 100_000);
}

#[test]
fn redis_selection_requires_its_endpoint() {
    assert!(
        parse(&[("SAGAT_EVENT_TRANSPORT", "redis")]).is_err(),
        "a selected transport without an endpoint is a configuration error"
    );
}

#[test]
fn redis_unknown_transport_selection_is_refused() {
    assert!(
        parse(&[("SAGAT_EVENT_TRANSPORT", "redsi")]).is_err(),
        "a misspelled selector must not silently disable publishing"
    );
}

#[test]
fn redis_selection_is_explicit_and_disabled_transport_keeps_both_publishers_off() {
    assert!(parse(&[("SAGAT_REDIS_URL", "rediss://redis.example:6379/0")]).is_err());
    let disabled = parse(&[
        ("SAGAT_EVENT_TRANSPORT", "none"),
        ("SAGAT_NATS_URL", "nats://legacy:4222"),
        ("SAGAT_REDIS_URL", "redis://inactive:6379/0"),
    ])
    .expect("disabled");
    assert!(disabled.nats.is_none() && disabled.redis.is_none());
    assert!(parse(&[("SAGAT_EVENT_TRANSPORT", "nats")]).is_err());
}

#[test]
fn redis_settings_refuse_plaintext_production_and_invalid_bounds() {
    for extra in [
        vec![("SAGAT_REDIS_URL", "redis://redis.example:6379/0")],
        vec![
            ("SAGAT_REDIS_URL", "rediss://redis.example:6379/0"),
            ("SAGAT_REDIS_STREAM_MAXLEN", "0"),
        ],
        vec![
            ("SAGAT_REDIS_URL", "rediss://redis.example:6379/0"),
            ("SAGAT_REDIS_STREAM", "bad key"),
        ],
        vec![
            ("SAGAT_REDIS_URL", "rediss://redis.example:6379/0"),
            ("SAGAT_REDIS_TIMEOUT_SECS", "31"),
        ],
    ] {
        let mut config = vec![("SAGAT_EVENT_TRANSPORT", "redis")];
        config.extend(extra);
        assert!(parse(&config).is_err());
    }
}

// ------------------------------------------------------------------- which optional passes run (D2009)

#[test]
fn the_pass_that_completes_resets_is_absent_unless_the_operator_turned_it_on() {
    use sagat_execution_engine::settings::{
        PASS_BATCHES, PASS_RESETS, PASS_SCHEDULES, optional_passes,
    };
    // nothing is on by default: the engine runs none of the three
    assert_eq!(
        optional_passes(&parse(&[]).expect("config")),
        Vec::<&str>::new()
    );
    // each is on by its own variable and by no other
    for (variable, pass) in [
        ("SAGAT_SCHEDULES_ENABLED", PASS_SCHEDULES),
        ("SAGAT_BATCHES_ENABLED", PASS_BATCHES),
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", PASS_RESETS),
    ] {
        assert_eq!(
            optional_passes(&parse(&[(variable, "true")]).expect("config")),
            [pass],
            "{variable}"
        );
        // and off said out loud is off
        assert!(
            optional_passes(&parse(&[(variable, "false")]).expect("config")).is_empty(),
            "{variable}"
        );
    }
    // the settings of the pass alone do not turn it on
    let tuned = parse(&[
        ("SAGAT_RESET_CANCEL_SOURCE_BATCH", "5"),
        ("SAGAT_RESET_CANCEL_SOURCE_PER_TENANT", "2"),
        ("SAGAT_RESET_CANCEL_SOURCE_POLL_MS", "50"),
    ])
    .expect("config");
    assert!(!optional_passes(&tuned).contains(&PASS_RESETS));
    // all three, in the order the engine starts them
    let all = parse(&[
        ("SAGAT_SCHEDULES_ENABLED", "true"),
        ("SAGAT_BATCHES_ENABLED", "true"),
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true"),
    ])
    .expect("config");
    assert_eq!(
        optional_passes(&all),
        [PASS_SCHEDULES, PASS_BATCHES, PASS_RESETS]
    );
}

/// The expiry of a reset that waits is a step of the reset pass, not a pass of its own (D2080): the flag governs the
/// completion and the expiry together. With it off in an engine nothing there expires an entry, however the window is set,
/// so an entry accepted while some engine had the flag on holds its place until an engine with the flag on runs (and
/// every entry older than the window then expires, a few at a time). The runbook says it; this is the test that no
/// other path exists.
#[test]
fn with_the_flag_off_the_engine_runs_no_pass_that_expires_a_reset_either() {
    use sagat_execution_engine::settings::{
        PASS_BATCHES, PASS_RESETS, PASS_SCHEDULES, optional_passes,
    };
    const FLAG: &str = "SAGAT_RESET_CANCEL_SOURCE_ENABLED";
    const WINDOW: &str = "SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS";
    // a window set, the flag not: the window is read (a deployment that sets it means it) and starts nothing
    for off in [
        vec![(WINDOW, "3600")],
        vec![(WINDOW, "3600"), (FLAG, "false")],
    ] {
        let config = parse(&off).expect("config");
        assert!(!config.reset_cancel_source_enabled, "{off:?}");
        assert_eq!(config.resets.expiry, Duration::from_secs(3_600), "{off:?}");
        assert!(
            optional_passes(&config).is_empty(),
            "nothing runs with the flag off, so nothing expires: {off:?}"
        );
    }
    // the flag on: one pass, the one that completes and expires
    let on = parse(&[(WINDOW, "3600"), (FLAG, "true")]).expect("config");
    assert_eq!(optional_passes(&on), [PASS_RESETS]);
    // there is no pass of the expiry besides it, whatever else is on
    let everything = parse(&[
        (FLAG, "true"),
        (WINDOW, "7200"),
        ("SAGAT_SCHEDULES_ENABLED", "true"),
        ("SAGAT_BATCHES_ENABLED", "true"),
    ])
    .expect("config");
    assert_eq!(
        optional_passes(&everything),
        [PASS_SCHEDULES, PASS_BATCHES, PASS_RESETS],
        "three passes and no fourth that expires"
    );
}

// ------------------------------------------------------------ no shipped manifest turns the switch on (ADR-0080)

/// Every file under `dir`, depth first.
fn files_under(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, found);
        } else {
            found.push(path);
        }
    }
}

/// The value a line gives `name`, when it gives one (`NAME=value`, `NAME: value`, `- name: NAME` is not one).
fn value_given(line: &str, name: &str) -> Option<String> {
    let after = line.split_once(name)?.1.trim_start();
    let value = after
        .strip_prefix('=')
        .or_else(|| after.strip_prefix(':'))?;
    Some(
        value
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_ascii_lowercase(),
    )
}

#[test]
fn no_shipped_manifest_turns_the_reset_switch_on() {
    // Until the instance-capability registry exists the switch is an operator's act, engine first (ADR-0080): no
    // compose file, chart or example of this repository may set it to anything but off.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra");
    let mut files = Vec::new();
    files_under(&root, &mut files);
    assert!(
        files.len() > 10,
        "the manifests were not found under {root:?}"
    );
    let mut on = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            if let Some(value) = value_given(line, "SAGAT_RESET_CANCEL_SOURCE_ENABLED")
                && !["", "false", "0", "no"].contains(&value.as_str())
            {
                on.push(format!("{}:{}: {line}", path.display(), number + 1));
            }
        }
    }
    assert!(on.is_empty(), "a manifest turns the switch on: {on:#?}");
}

#[test]
fn a_line_that_gives_the_switch_a_value_is_read_as_one() {
    let name = "SAGAT_RESET_CANCEL_SOURCE_ENABLED";
    for (line, value) in [
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED=true", Some("true")),
        (
            "  - SAGAT_RESET_CANCEL_SOURCE_ENABLED: \"True\"",
            Some("true"),
        ),
        ("      SAGAT_RESET_CANCEL_SOURCE_ENABLED: 1", Some("1")),
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED = false", Some("false")),
        (
            "the engine does not have SAGAT_RESET_CANCEL_SOURCE_ENABLED on",
            None,
        ),
        ("nothing here", None),
    ] {
        assert_eq!(value_given(line, name).as_deref(), value, "{line}");
    }
}
