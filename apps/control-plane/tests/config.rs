//! Configuration parsing and command-line parsing. No environment, no network.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use sagat_control_plane::cli::{self, Command, UsageError};
use sagat_control_plane::config::{
    Config, ConfigError, Environment, ProvisionConfig, SeedConfig, ServeConfig,
    admin_addr_is_exposed, redact_url, requires_tls,
};
use sagat_telemetry::LogFormat;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

const DB: (&str, &str) = (
    "DATABASE_URL",
    "postgres://user:s3cret@db.internal/app?sslmode=require",
);
const OIDC: [(&str, &str); 3] = [
    ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
    ("SAGAT_OIDC_AUDIENCE", "sagat"),
    ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
];

#[test]
fn defaults_are_the_safe_ones() {
    let config = Config::from_lookup(&env(&[DB])).expect("config");

    assert_eq!(
        config.environment,
        Environment::Prod,
        "unset means production"
    );
    assert_eq!(config.log_format, LogFormat::Json);
    assert_eq!(config.database.max_connections, 10);
    assert_eq!(config.database.statement_timeout, Duration::from_secs(5));
}

#[test]
fn run_input_blobs_require_shared_storage_and_bounded_transfer_settings() {
    let off = ServeConfig::from_lookup(&env(&OIDC)).expect("default");
    assert!(off.blobs.is_none());
    for environment in ["dev", "prod"] {
        let mut pairs = OIDC.to_vec();
        pairs.extend([
            ("SAGAT_ENV", environment),
            ("SAGAT_BLOB_PROVIDER", "memory"),
        ]);
        assert!(
            ServeConfig::from_lookup(&env(&pairs)).is_err(),
            "private memory cannot reach an engine"
        );
    }
    let mut pairs = OIDC.to_vec();
    pairs.extend([
        ("SAGAT_BLOB_PROVIDER", "fs"),
        ("SAGAT_BLOB_FS_ROOT", "/tmp/shared-blobs"),
    ]);
    let settings = ServeConfig::from_lookup(&env(&pairs)).expect("shared fs");
    assert!(settings.blobs.is_some());
    assert_eq!(settings.blob_transfer.max_concurrent, 32);
    assert_eq!(settings.blob_transfer.timeout, Duration::from_secs(600));
    for (name, value) in [
        ("SAGAT_BLOB_MAX_TRANSFERS", "0"),
        ("SAGAT_BLOB_MAX_TRANSFERS", "4097"),
        ("SAGAT_BLOB_TRANSFER_TIMEOUT_SECS", "0"),
        ("SAGAT_BLOB_TRANSFER_TIMEOUT_SECS", "86401"),
    ] {
        let mut bad = pairs.clone();
        bad.push((name, value));
        assert!(ServeConfig::from_lookup(&env(&bad)).is_err(), "{name}");
    }
}

#[test]
fn the_database_url_is_required_and_never_printed() {
    assert_eq!(
        Config::from_lookup(&env(&[])).expect_err("missing"),
        ConfigError::Missing("DATABASE_URL")
    );
    assert_eq!(
        Config::from_lookup(&env(&[("DATABASE_URL", "   ")])).expect_err("blank"),
        ConfigError::Missing("DATABASE_URL")
    );

    let config = Config::from_lookup(&env(&[DB])).expect("config");
    let rendered = format!("{config:?} {:?}", config.database);
    assert!(
        !rendered.contains("s3cret") && !rendered.contains("user:"),
        "{rendered}"
    );
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn invalid_values_name_the_variable() {
    let bad: [(&str, &str); 4] = [
        ("SAGAT_ENV", "qa"),
        ("SAGAT_LOG_FORMAT", "xml"),
        ("SAGAT_DB_MAX_CONNECTIONS", "0"),
        ("SAGAT_DB_STATEMENT_TIMEOUT_MS", "soon"),
    ];
    for (name, value) in bad {
        let error = Config::from_lookup(&env(&[DB, (name, value)])).expect_err(name);
        assert!(
            matches!(&error, ConfigError::Invalid { name: n, .. } if *n == name),
            "{name}={value} -> {error:?}"
        );
    }
}

#[test]
fn serve_requires_the_identity_provider_settings() {
    for missing in [
        "SAGAT_OIDC_ISSUER",
        "SAGAT_OIDC_AUDIENCE",
        "SAGAT_OIDC_JWKS_URI",
    ] {
        let pairs: Vec<_> = OIDC
            .iter()
            .copied()
            .filter(|(k, _)| *k != missing)
            .collect();
        let error = ServeConfig::from_lookup(&env(&pairs)).expect_err(missing);
        assert_eq!(error, ConfigError::Missing(missing));
    }
}

#[test]
fn serve_defaults_bind_to_loopback_and_keep_the_secure_options() {
    let serve = ServeConfig::from_lookup(&env(&OIDC)).expect("serve config");

    assert_eq!(
        serve.bind_addr.to_string(),
        "127.0.0.1:8080",
        "not exposed on all interfaces"
    );
    assert!(!serve.auth.allow_insecure_jwks_http);
    assert!(
        !serve.migrate_on_start,
        "migrations are a separate, privileged step by default"
    );
    assert_eq!(serve.auth.tenant_claim, "tenant");
    assert!(serve.limits.max_body_bytes > 0 && serve.limits.request_timeout > Duration::ZERO);
    // Slow-client protection is on by default, with finite values.
    assert_eq!(serve.server.header_read_timeout, Duration::from_secs(10));
    assert_eq!(serve.server.max_connections, 1024);
    assert_eq!(serve.server.shutdown_grace, Duration::from_secs(30));
}

#[test]
fn serve_options_can_be_overridden() {
    let mut pairs = OIDC.to_vec();
    pairs.extend([
        ("SAGAT_BIND_ADDR", "0.0.0.0:9000"),
        ("SAGAT_OIDC_TENANT_CLAIM", "https://x/tenant"),
        ("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "true"),
        ("SAGAT_MIGRATE_ON_START", "1"),
        ("SAGAT_MAX_BODY_BYTES", "1024"),
        ("SAGAT_REQUEST_TIMEOUT_SECS", "3"),
        ("SAGAT_HEADER_READ_TIMEOUT_SECS", "4"),
        ("SAGAT_MAX_CONNECTIONS", "50"),
        ("SAGAT_SHUTDOWN_GRACE_SECS", "7"),
    ]);
    let serve = ServeConfig::from_lookup(&env(&pairs)).expect("serve config");

    assert_eq!(serve.bind_addr.to_string(), "0.0.0.0:9000");
    assert_eq!(serve.auth.tenant_claim, "https://x/tenant");
    assert!(serve.auth.allow_insecure_jwks_http && serve.migrate_on_start);
    assert_eq!(serve.limits.max_body_bytes, 1024);
    assert_eq!(serve.limits.request_timeout, Duration::from_secs(3));
    assert_eq!(serve.server.header_read_timeout, Duration::from_secs(4));
    assert_eq!(serve.server.max_connections, 50);
    assert_eq!(serve.server.shutdown_grace, Duration::from_secs(7));
}

#[test]
fn nonsense_serve_values_are_rejected() {
    let bad: [(&str, &str); 8] = [
        ("SAGAT_BIND_ADDR", "everywhere"),
        ("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "maybe"),
        ("SAGAT_MAX_BODY_BYTES", "0"),
        ("SAGAT_REQUEST_TIMEOUT_SECS", "0"),
        ("SAGAT_REQUEST_TIMEOUT_SECS", "-1"),
        ("SAGAT_HEADER_READ_TIMEOUT_SECS", "0"),
        ("SAGAT_MAX_CONNECTIONS", "0"),
        ("SAGAT_SHUTDOWN_GRACE_SECS", "0"),
    ];
    for (name, value) in bad {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, value));
        assert!(
            ServeConfig::from_lookup(&env(&pairs)).is_err(),
            "{name}={value}"
        );
    }
}

#[test]
fn seed_defaults_are_obviously_development_values() {
    let seed = SeedConfig::from_lookup(&env(&[])).expect("seed");
    assert_eq!(
        (seed.tenant_slug.as_str(), seed.admin_subject.as_str()),
        ("dev", "dev-admin")
    );
}

#[test]
fn the_command_line_maps_to_commands() {
    let parse =
        |args: &[&str]| cli::parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());

    assert_eq!(parse(&[]), Ok(Command::Serve), "serving is the default");
    assert_eq!(parse(&["serve"]), Ok(Command::Serve));
    assert_eq!(parse(&["migrate"]), Ok(Command::Migrate));
    assert_eq!(parse(&["seed-dev"]), Ok(Command::SeedDev));
    assert_eq!(parse(&["healthcheck"]), Ok(Command::Healthcheck));
    for help in ["help", "--help", "-h"] {
        assert_eq!(parse(&[help]), Ok(Command::Help));
    }
    for version in ["version", "--version", "-V"] {
        assert_eq!(parse(&[version]), Ok(Command::Version));
    }
    assert!(matches!(parse(&["rm", "-rf"]), Err(UsageError(_))));
    assert!(matches!(parse(&["migrate", "now"]), Err(UsageError(m)) if m.contains("now")));
    assert!(matches!(parse(&["--unknown"]), Err(UsageError(_))));
}

#[test]
fn outside_development_the_database_connection_must_require_tls() {
    let insecure = [
        "postgres://u:p@db/app",
        "postgres://u:p@db/app?sslmode=prefer",
        "postgres://u:p@db/app?sslmode=allow",
        "postgres://u:p@db/app?sslmode=disable",
        "postgres://u:p@db/app?application_name=x",
        "postgres://u:p@db/app?sslmode=",
    ];
    for environment in ["prod", "staging"] {
        for url in insecure {
            let error =
                Config::from_lookup(&env(&[("SAGAT_ENV", environment), ("DATABASE_URL", url)]))
                    .expect_err(url);
            assert!(
                matches!(&error, ConfigError::Invalid { name: "DATABASE_URL", reason } if reason.contains("TLS")),
                "{environment} {url} -> {error:?}"
            );
            assert!(
                !error.to_string().contains("u:p"),
                "credentials must not be echoed: {error}"
            );
        }
        // The unset default is production, so it is covered too.
    }
    assert!(Config::from_lookup(&env(&[("DATABASE_URL", "postgres://u:p@db/app")])).is_err());

    for url in [
        "postgres://u:p@db/app?sslmode=require",
        "postgres://u:p@db/app?sslmode=verify-ca",
        "postgres://u:p@db/app?sslmode=verify-full",
        "postgres://u:p@db/app?application_name=x&sslmode=require",
        "postgres://u:p@db/app?ssl-mode=require",
    ] {
        assert!(
            Config::from_lookup(&env(&[("SAGAT_ENV", "prod"), ("DATABASE_URL", url)])).is_ok(),
            "{url}"
        );
    }

    // Development keeps working against a plain local database.
    assert!(
        Config::from_lookup(&env(&[
            ("SAGAT_ENV", "dev"),
            ("DATABASE_URL", "postgres://u:p@localhost/app")
        ]))
        .is_ok()
    );
}

#[test]
fn tls_detection_fails_closed_on_odd_urls() {
    assert!(requires_tls("postgres://u:p@h/db?sslmode=require"));
    assert!(!requires_tls("postgres://u:p@h/db"));
    // A password containing '?' must not be mistaken for the start of a query.
    assert!(!requires_tls("postgres://u:pa?sslmode=require@h/db"));
    assert!(!requires_tls("sslmode=require"));
    assert!(
        !requires_tls("postgres://u:p@h/db?sslmode=REQUIRE"),
        "case-sensitive: refuse what we cannot recognise"
    );
}

#[test]
fn logged_urls_never_carry_credentials_or_query_strings() {
    let cases = [
        (
            "https://idp.example/.well-known/jwks.json",
            "https://idp.example/.well-known/jwks.json",
        ),
        (
            "https://user:hunter2@idp.example/jwks.json?token=abc#frag",
            "https://idp.example/jwks.json",
        ),
        ("https://idp.example:8443/", "https://idp.example:8443"),
        ("https://a:b@c@idp.example/x", "https://idp.example/x"),
        ("not a url", "<unparseable url>"),
    ];
    for (input, expected) in cases {
        let shown = redact_url(input);
        assert_eq!(shown, expected, "{input}");
        assert!(!shown.contains("hunter2") && !shown.contains("token=") && !shown.contains("a:b"));
    }
}

#[test]
fn overload_protection_is_on_by_default_with_finite_positive_values() {
    let serve = ServeConfig::from_lookup(&env(&OIDC)).expect("serve config");

    assert!(serve.rate_limit.per_second > 0.0 && serve.rate_limit.burst >= 1.0);
    assert!(serve.limits.max_in_flight > 0);
    assert_eq!(
        serve.admin_addr.map(|a| a.to_string()).as_deref(),
        Some("127.0.0.1:9090"),
        "metrics on loopback"
    );
    assert_eq!(serve.auth.required_token_type, None);
}

#[test]
fn overload_and_observability_settings_can_be_tuned() {
    let mut pairs = OIDC.to_vec();
    pairs.extend([
        ("SAGAT_RATE_LIMIT_RPS", "2.5"),
        ("SAGAT_RATE_LIMIT_BURST", "10"),
        ("SAGAT_MAX_IN_FLIGHT", "64"),
        ("SAGAT_ADMIN_ADDR", "127.0.0.1:19090"),
        ("SAGAT_OIDC_REQUIRED_TYP", "at+jwt"),
    ]);
    let serve = ServeConfig::from_lookup(&env(&pairs)).expect("serve config");

    assert!(
        (serve.rate_limit.per_second - 2.5).abs() < f64::EPSILON
            && (serve.rate_limit.burst - 10.0).abs() < f64::EPSILON
    );
    assert_eq!(serve.limits.max_in_flight, 64);
    assert_eq!(
        serve.admin_addr.map(|a| a.to_string()).as_deref(),
        Some("127.0.0.1:19090")
    );
    assert_eq!(serve.auth.required_token_type.as_deref(), Some("at+jwt"));

    let mut off = OIDC.to_vec();
    off.push(("SAGAT_ADMIN_ADDR", "off"));
    assert_eq!(
        ServeConfig::from_lookup(&env(&off))
            .expect("config")
            .admin_addr,
        None
    );
}

#[test]
fn nonsensical_overload_settings_are_rejected() {
    let bad: [(&str, &str); 8] = [
        ("SAGAT_RATE_LIMIT_RPS", "0"),
        ("SAGAT_RATE_LIMIT_RPS", "-1"),
        ("SAGAT_RATE_LIMIT_RPS", "NaN"),
        ("SAGAT_RATE_LIMIT_RPS", "inf"),
        ("SAGAT_RATE_LIMIT_BURST", "0.5"),
        ("SAGAT_MAX_IN_FLIGHT", "0"),
        ("SAGAT_ADMIN_ADDR", "everywhere"),
        ("SAGAT_RATE_LIMIT_BURST", "lots"),
    ];
    for (name, value) in bad {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, value));
        assert!(
            ServeConfig::from_lookup(&env(&pairs)).is_err(),
            "{name}={value}"
        );
    }
}

#[test]
fn rate_limit_values_that_would_break_the_limiter_name_their_variable() {
    // `1e-300` used to be accepted and made the first over-budget request panic (review finding 6).
    let bad: [(&str, &str); 9] = [
        ("SAGAT_RATE_LIMIT_RPS", "1e-300"),
        ("SAGAT_RATE_LIMIT_RPS", "0.0005"),
        ("SAGAT_RATE_LIMIT_RPS", "NaN"),
        ("SAGAT_RATE_LIMIT_RPS", "-inf"),
        ("SAGAT_RATE_LIMIT_BURST", "0.99"),
        ("SAGAT_RATE_LIMIT_BURST", "NaN"),
        ("SAGAT_RATE_LIMIT_BURST", "inf"),
        ("SAGAT_RATE_LIMIT_BURST", "0"),
        ("SAGAT_RATE_LIMIT_BURST", "-5"),
    ];
    for (name, value) in bad {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, value));
        let error = ServeConfig::from_lookup(&env(&pairs)).expect_err(name);
        assert!(
            matches!(&error, ConfigError::Invalid { name: n, .. } if *n == name),
            "{name}={value} -> {error:?}"
        );
    }

    // The documented minimums themselves are accepted.
    let mut edge = OIDC.to_vec();
    edge.extend([
        ("SAGAT_RATE_LIMIT_RPS", "0.001"),
        ("SAGAT_RATE_LIMIT_BURST", "1"),
    ]);
    let serve = ServeConfig::from_lookup(&env(&edge)).expect("minimums are valid");
    assert!(
        (serve.rate_limit.per_second - 0.001).abs() < f64::EPSILON
            && (serve.rate_limit.burst - 1.0).abs() < f64::EPSILON
    );
}

#[test]
fn only_loopback_addresses_keep_the_unauthenticated_metrics_listener_private() {
    let loopback = [
        "127.0.0.1:9090",
        "127.0.0.2:9090",
        "[::1]:9090",
        "[::ffff:127.0.0.1]:9090",
    ];
    for addr in loopback {
        let addr: SocketAddr = addr.parse().expect("addr");
        assert!(!admin_addr_is_exposed(addr), "{addr} is loopback");
    }
    let exposed = [
        "0.0.0.0:9090",
        "[::]:9090",
        "10.1.2.3:9090",
        "192.168.0.10:9090",
        "203.0.113.9:9090",
        "[2001:db8::1]:9090",
        "[::ffff:10.0.0.1]:9090",
    ];
    for addr in exposed {
        let addr: SocketAddr = addr.parse().expect("addr");
        assert!(
            admin_addr_is_exposed(addr),
            "{addr} is reachable from a network"
        );
    }
}

#[test]
fn the_required_token_type_is_trimmed_like_every_other_setting() {
    // A value mounted from a file ends in a newline; with the newline kept, no token ever matched.
    for (raw, expected) in [
        ("at+jwt\n", Some("at+jwt")),
        ("  application/at+jwt \r\n", Some("application/at+jwt")),
        ("at+jwt", Some("at+jwt")),
        ("   \n", None),
        ("", None),
    ] {
        let mut pairs = OIDC.to_vec();
        pairs.push(("SAGAT_OIDC_REQUIRED_TYP", raw));
        let serve = ServeConfig::from_lookup(&env(&pairs)).expect("serve config");
        assert_eq!(
            serve.auth.required_token_type.as_deref(),
            expected,
            "{raw:?}"
        );
    }
}

#[test]
fn slow_client_protection_has_finite_defaults() {
    let serve = ServeConfig::from_lookup(&env(&OIDC)).expect("serve config");

    assert_eq!(serve.limits.body_idle_timeout, Duration::from_secs(5));
    // A quarter of the global cap (512), so one tenant cannot hold every slot.
    assert_eq!(serve.limits.max_in_flight, 512);
    assert_eq!(serve.limits.max_in_flight_per_tenant, 128);
}

#[test]
fn the_per_tenant_cap_follows_the_global_cap_unless_set() {
    let mut derived = OIDC.to_vec();
    derived.push(("SAGAT_MAX_IN_FLIGHT", "64"));
    let serve = ServeConfig::from_lookup(&env(&derived)).expect("derived");
    assert_eq!(serve.limits.max_in_flight_per_tenant, 16);

    let mut tiny = OIDC.to_vec();
    tiny.push(("SAGAT_MAX_IN_FLIGHT", "2"));
    assert_eq!(
        ServeConfig::from_lookup(&env(&tiny))
            .expect("tiny")
            .limits
            .max_in_flight_per_tenant,
        1,
        "never zero"
    );

    let mut explicit = OIDC.to_vec();
    explicit.extend([
        ("SAGAT_MAX_IN_FLIGHT", "64"),
        ("SAGAT_MAX_IN_FLIGHT_PER_TENANT", "7"),
        ("SAGAT_BODY_IDLE_TIMEOUT_SECS", "2"),
    ]);
    let serve = ServeConfig::from_lookup(&env(&explicit)).expect("explicit");
    assert_eq!(serve.limits.max_in_flight_per_tenant, 7);
    assert_eq!(serve.limits.body_idle_timeout, Duration::from_secs(2));
}

#[test]
fn nonsensical_slow_client_settings_name_their_variable() {
    let bad: [(&str, &str); 6] = [
        ("SAGAT_MAX_IN_FLIGHT_PER_TENANT", "0"),
        ("SAGAT_MAX_IN_FLIGHT_PER_TENANT", "-1"),
        ("SAGAT_MAX_IN_FLIGHT_PER_TENANT", "many"),
        ("SAGAT_BODY_IDLE_TIMEOUT_SECS", "0"),
        ("SAGAT_BODY_IDLE_TIMEOUT_SECS", "-3"),
        ("SAGAT_BODY_IDLE_TIMEOUT_SECS", "soon"),
    ];
    for (name, value) in bad {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, value));
        let error = ServeConfig::from_lookup(&env(&pairs)).expect_err(name);
        assert!(
            matches!(&error, ConfigError::Invalid { name: n, .. } if *n == name),
            "{name}={value} -> {error:?}"
        );
    }
}

#[test]
fn event_streams_may_take_at_most_half_of_the_connections_the_server_accepts() {
    // By default the platform's streams are the lesser of 2 000 and half of the connections (1 024).
    let serve = ServeConfig::from_lookup(&env(&OIDC)).expect("serve config");
    assert_eq!(serve.streams.global, 512);
    assert_eq!(serve.streams.per_tenant, 200);
    assert_eq!(serve.streams.per_subject, 20);
    assert_eq!(serve.server.write_stall_timeout, Duration::from_secs(30));

    // More connections, more streams, up to the ceiling that is set.
    let mut big = OIDC.to_vec();
    big.push(("SAGAT_MAX_CONNECTIONS", "8192"));
    assert_eq!(
        ServeConfig::from_lookup(&env(&big))
            .expect("big")
            .streams
            .global,
        2000
    );

    // A cap above half the connections would let streams starve the API, the probe and the files.
    let mut starving = OIDC.to_vec();
    starving.extend([
        ("SAGAT_MAX_CONNECTIONS", "100"),
        ("SAGAT_STREAMS_MAX", "51"),
    ]);
    match ServeConfig::from_lookup(&env(&starving)) {
        Err(ConfigError::Invalid { name, reason }) => {
            assert_eq!(name, "SAGAT_STREAMS_MAX");
            assert!(reason.contains("half"), "{reason}");
        }
        other => panic!("a stream cap that starves the server was accepted: {other:?}"),
    }
    let mut fits = OIDC.to_vec();
    fits.extend([
        ("SAGAT_MAX_CONNECTIONS", "100"),
        ("SAGAT_STREAMS_MAX", "50"),
        ("SAGAT_STREAMS_PER_TENANT", "10"),
        ("SAGAT_STREAMS_PER_SUBJECT", "3"),
        ("SAGAT_WRITE_STALL_TIMEOUT_SECS", "5"),
    ]);
    let serve = ServeConfig::from_lookup(&env(&fits)).expect("fits");
    assert_eq!(
        (
            serve.streams.global,
            serve.streams.per_tenant,
            serve.streams.per_subject
        ),
        (50, 10, 3)
    );
    assert_eq!(serve.server.write_stall_timeout, Duration::from_secs(5));

    // Zero is not a limit.
    for name in [
        "SAGAT_STREAMS_MAX",
        "SAGAT_STREAMS_PER_TENANT",
        "SAGAT_STREAMS_PER_SUBJECT",
        "SAGAT_WRITE_STALL_TIMEOUT_SECS",
    ] {
        let mut zero = OIDC.to_vec();
        zero.push((name, "0"));
        match ServeConfig::from_lookup(&env(&zero)) {
            Err(ConfigError::Invalid { name: named, .. }) => assert_eq!(named, name),
            other => panic!("{name}=0 was accepted: {other:?}"),
        }
    }
}

#[test]
fn the_per_subject_budget_has_its_own_defaults_and_validation() {
    let serve = ServeConfig::from_lookup(&env(&OIDC)).expect("serve config");
    assert!(
        (serve.subject_rate_limit.per_second - 20.0).abs() < f64::EPSILON
            && (serve.subject_rate_limit.burst - 40.0).abs() < f64::EPSILON,
        "{:?}",
        serve.subject_rate_limit
    );
    assert!(
        serve.subject_rate_limit.per_second < serve.rate_limit.per_second,
        "one subject gets less than a whole tenant"
    );

    let mut tuned = OIDC.to_vec();
    tuned.extend([
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "5.5"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "11"),
    ]);
    let serve = ServeConfig::from_lookup(&env(&tuned)).expect("tuned");
    assert!(
        (serve.subject_rate_limit.per_second - 5.5).abs() < f64::EPSILON
            && (serve.subject_rate_limit.burst - 11.0).abs() < f64::EPSILON
    );
    // The tenant budget is independent of it.
    assert!((serve.rate_limit.per_second - 100.0).abs() < f64::EPSILON);

    let bad: [(&str, &str); 6] = [
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "0"),
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "1e-300"),
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "NaN"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "0.5"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "inf"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "lots"),
    ];
    for (name, value) in bad {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, value));
        let error = ServeConfig::from_lookup(&env(&pairs)).expect_err(name);
        assert!(
            matches!(&error, ConfigError::Invalid { name: n, .. } if *n == name),
            "{name}={value} -> {error:?}"
        );
    }
}

#[test]
fn tenant_provisioning_has_no_defaults() {
    assert_eq!(
        ProvisionConfig::from_lookup(&env(&[])),
        Err(ConfigError::Missing("SAGAT_PROVISION_TENANT"))
    );
    assert_eq!(
        ProvisionConfig::from_lookup(&env(&[("SAGAT_PROVISION_TENANT", "acme")])),
        Err(ConfigError::Missing("SAGAT_PROVISION_ADMIN"))
    );
    let ok = ProvisionConfig::from_lookup(&env(&[
        ("SAGAT_PROVISION_TENANT", "acme"),
        ("SAGAT_PROVISION_ADMIN", "alice"),
    ]));
    assert_eq!(
        ok.map(|c| (c.tenant_slug, c.admin_subject)),
        Ok(("acme".into(), "alice".into()))
    );
}

#[test]
fn an_api_that_receives_secret_values_in_plaintext_outside_development_must_say_tls_is_in_front() {
    let key = sagat_infra_secrets::generate_key().expect("key");
    let ring = format!("k1:{key}");
    let with = |extra: &[(&'static str, &str)]| {
        let mut pairs: Vec<(&str, &str)> = OIDC.to_vec();
        pairs.push(("SAGAT_SECRETS_KEYS", ring.as_str()));
        pairs.extend_from_slice(extra);
        ServeConfig::from_lookup(&env(&pairs))
    };

    // Production by default: refused, naming the variable that states the fact, and never the ring.
    let refused = with(&[]).expect_err("plaintext secrets");
    match &refused {
        ConfigError::Invalid { name, reason } => {
            assert_eq!(*name, "SAGAT_API_ALLOW_PLAINTEXT_SECRETS");
            assert!(!reason.contains(&key), "never the ring: {reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!refused.to_string().contains(&key));
    assert!(
        with(&[("SAGAT_ENV", "staging")]).is_err(),
        "staging is not development"
    );

    // Stated, or in development, it is accepted and the ring is kept.
    assert!(
        with(&[("SAGAT_API_ALLOW_PLAINTEXT_SECRETS", "true")])
            .expect("stated")
            .secrets
            .is_some()
    );
    assert!(
        with(&[("SAGAT_ENV", "dev")])
            .expect("development")
            .secrets
            .is_some()
    );

    // No ring, nothing to state.
    assert!(ServeConfig::from_lookup(&env(&OIDC)).is_ok());
}

// ------------------------------------------------------------------ the Studio (D143, D144)

fn with_oidc(extra: &[(&str, &str)]) -> Vec<(&'static str, String)> {
    let mut pairs: Vec<(&'static str, String)> =
        OIDC.iter().map(|(k, v)| (*k, (*v).to_owned())).collect();
    pairs.extend(extra.iter().map(|(k, v)| (leak(k), (*v).to_owned())));
    pairs
}

fn leak(name: &str) -> &'static str {
    Box::leak(name.to_owned().into_boxed_str())
}

fn serve_with(extra: &[(&str, &str)]) -> Result<ServeConfig, ConfigError> {
    let owned = with_oidc(extra);
    let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
    ServeConfig::from_lookup(&env(&pairs))
}

#[test]
fn the_studio_is_off_unless_a_directory_is_named() {
    assert!(serve_with(&[]).expect("config").studio.is_none());
    assert!(
        serve_with(&[("SAGAT_STUDIO_DIR", "  ")])
            .expect("config")
            .studio
            .is_none(),
        "a blank value is no value"
    );
}

#[test]
fn a_directory_alone_serves_the_studio_with_a_pasted_token_sign_in() {
    let studio = serve_with(&[("SAGAT_STUDIO_DIR", "/srv/studio")])
        .expect("config")
        .studio
        .expect("on");
    assert_eq!(studio.dir, std::path::PathBuf::from("/srv/studio"));
    assert!(studio.oidc.is_none());
    assert!(studio.trace_url.is_none());
}

#[test]
fn a_safe_trace_url_template_is_published_to_the_studio_at_runtime() {
    let template = "https://traces.example.test/explore?trace={traceId}";
    let studio = serve_with(&[
        ("SAGAT_STUDIO_DIR", "/srv/studio"),
        ("SAGAT_STUDIO_TRACE_URL", template),
    ])
    .expect("config")
    .studio
    .expect("on");
    assert_eq!(studio.trace_url.as_deref(), Some(template));

    let local = serve_with(&[
        ("SAGAT_STUDIO_DIR", "/srv/studio"),
        (
            "SAGAT_STUDIO_TRACE_URL",
            "http://127.0.0.2:16686/trace/{traceId}",
        ),
    ])
    .expect("loopback trace tool")
    .studio
    .expect("on");
    assert!(local.trace_url.is_some());
}

#[test]
fn unsafe_or_malformed_trace_url_templates_are_refused() {
    for template in [
        "http://traces.example.test/{traceId}",
        "https://user:password@traces.example.test/{traceId}",
        "traces.example.test/{traceId}",
        "https://traces.example.test/no-placeholder",
        "https://traces.example.test/{traceId}/{traceId}",
    ] {
        let error = serve_with(&[
            ("SAGAT_STUDIO_DIR", "/srv/studio"),
            ("SAGAT_STUDIO_TRACE_URL", template),
        ])
        .expect_err(template);
        assert!(
            error.to_string().contains("SAGAT_STUDIO_TRACE_URL"),
            "{template}: {error}"
        );
    }
}

#[test]
fn an_issuer_and_a_client_id_configure_the_browser_sign_in() {
    let studio = serve_with(&[
        ("SAGAT_STUDIO_DIR", "/srv/studio"),
        (
            "SAGAT_STUDIO_OIDC_ISSUER",
            "https://login.example/realms/acme",
        ),
        ("SAGAT_STUDIO_OIDC_CLIENT_ID", "studio"),
    ])
    .expect("config")
    .studio
    .expect("on");
    let oidc = studio.oidc.expect("sign-in");
    assert_eq!(oidc.issuer, "https://login.example/realms/acme");
    assert_eq!(oidc.client_id, "studio");
    assert_eq!(oidc.scope, "openid profile", "the default scope");

    let scoped = serve_with(&[
        ("SAGAT_STUDIO_DIR", "/srv/studio"),
        (
            "SAGAT_STUDIO_OIDC_ISSUER",
            "https://login.example/realms/acme",
        ),
        ("SAGAT_STUDIO_OIDC_CLIENT_ID", "studio"),
        ("SAGAT_STUDIO_OIDC_SCOPE", "openid email"),
    ])
    .expect("config")
    .studio
    .expect("on")
    .oidc
    .expect("sign-in");
    assert_eq!(scoped.scope, "openid email");
}

#[test]
fn a_half_configured_sign_in_or_one_that_would_leak_a_token_is_refused() {
    for (extra, name) in [
        (
            vec![("SAGAT_STUDIO_OIDC_ISSUER", "https://login.example/")],
            "SAGAT_STUDIO_OIDC_CLIENT_ID",
        ),
        (
            vec![("SAGAT_STUDIO_OIDC_CLIENT_ID", "studio")],
            "SAGAT_STUDIO_OIDC_ISSUER",
        ),
        (
            vec![
                ("SAGAT_STUDIO_OIDC_ISSUER", "http://login.example/"),
                ("SAGAT_STUDIO_OIDC_CLIENT_ID", "studio"),
            ],
            "SAGAT_STUDIO_OIDC_ISSUER",
        ),
        (
            vec![
                ("SAGAT_STUDIO_OIDC_ISSUER", "https://user:pw@login.example/"),
                ("SAGAT_STUDIO_OIDC_CLIENT_ID", "studio"),
            ],
            "SAGAT_STUDIO_OIDC_ISSUER",
        ),
        (
            vec![
                ("SAGAT_STUDIO_OIDC_ISSUER", "https://login.example/"),
                ("SAGAT_STUDIO_OIDC_CLIENT_ID", "stu dio"),
            ],
            "SAGAT_STUDIO_OIDC_CLIENT_ID",
        ),
    ] {
        let mut pairs = vec![("SAGAT_STUDIO_DIR", "/srv/studio")];
        pairs.extend(extra);
        let error = serve_with(&pairs).expect_err(name);
        assert!(error.to_string().contains(name), "{name}: {error}");
    }
    // A loopback issuer over http is how development works.
    assert!(
        serve_with(&[
            ("SAGAT_STUDIO_DIR", "/srv/studio"),
            (
                "SAGAT_STUDIO_OIDC_ISSUER",
                "http://localhost:8081/realms/dev"
            ),
            ("SAGAT_STUDIO_OIDC_CLIENT_ID", "studio"),
        ])
        .is_ok()
    );
}

#[test]
fn native_http_tls_never_accepts_partial_or_unreadable_credentials() {
    for name in [
        "SAGAT_HTTP_TLS_CERT",
        "SAGAT_HTTP_TLS_KEY",
        "SAGAT_HTTP_TLS_CLIENT_CA",
    ] {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, "/nonexistent/sagat-test-tls.pem"));
        assert!(ServeConfig::from_lookup(&env(&pairs)).is_err(), "{name}");
    }
    let defaults = ServeConfig::from_lookup(&env(&OIDC)).expect("config");
    assert!(defaults.tls.is_none() && defaults.webhook_oidc.is_empty());
    let mut pairs = OIDC.to_vec();
    pairs.push((
        "SAGAT_WEBHOOK_OIDC_PROFILES_FILE",
        "/nonexistent/sagat-profiles.json",
    ));
    assert!(ServeConfig::from_lookup(&env(&pairs)).is_err());
}

#[test]
fn native_http_timers_and_permits_are_bounded_before_runtime_creation() {
    for (name, value) in [
        ("SAGAT_MAX_CONNECTIONS", usize::MAX.to_string()),
        ("SAGAT_HEADER_READ_TIMEOUT_SECS", u64::MAX.to_string()),
        ("SAGAT_WRITE_STALL_TIMEOUT_SECS", "86401".to_owned()),
        ("SAGAT_SHUTDOWN_GRACE_SECS", "86401".to_owned()),
    ] {
        let mut pairs = OIDC.to_vec();
        pairs.push((name, &value));
        assert!(ServeConfig::from_lookup(&env(&pairs)).is_err(), "{name}");
    }
}

#[test]
fn openbao_alone_requires_secret_transport_protection() {
    let mut pairs = OIDC.to_vec();
    pairs.extend([
        ("SAGAT_OPENBAO_ADDRESS", "https://bao:8200"),
        ("SAGAT_OPENBAO_TOKEN_FILE", "/run/openbao/token"),
    ]);
    assert!(ServeConfig::from_lookup(&env(&pairs)).is_err());
    pairs.push(("SAGAT_API_ALLOW_PLAINTEXT_SECRETS", "true"));
    let config = ServeConfig::from_lookup(&env(&pairs)).expect("TLS terminated at ingress");
    assert!(config.openbao.is_some());
    assert!(config.secrets.is_none());
}
