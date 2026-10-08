//! Configuration parsing and command-line parsing. No environment, no network.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::time::Duration;

use sagat_control_plane::cli::{self, Command, UsageError};
use sagat_control_plane::config::{
    Config, ConfigError, Environment, ProvisionConfig, SeedConfig, ServeConfig, redact_url,
    requires_tls,
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
