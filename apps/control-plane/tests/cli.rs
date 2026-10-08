//! The real executable: exit codes and what it prints. No database is needed because every case
//! here is decided before a connection is attempted.
#![allow(clippy::expect_used, clippy::panic)]

use std::process::{Command, Output};
use std::time::{Duration, Instant};

const SECRET_URL: &str = "postgres://leaky:hunter2-password@127.0.0.1:1/never?sslmode=require";
const PLAINTEXT_URL: &str = "postgres://leaky:hunter2-password@127.0.0.1:1/never";

fn control_plane(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_control-plane"));
    command.args(args).env_clear();
    for (k, v) in env {
        command.env(k, v);
    }
    command.output().expect("run control-plane")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_and_version_succeed() {
    let help = control_plane(&["--help"], &[]);
    assert_eq!(help.status.code(), Some(0));
    let out = text(&help.stdout);
    assert!(
        out.contains("DATABASE_URL") && out.contains("seed-dev"),
        "{out}"
    );

    let version = control_plane(&["version"], &[]);
    assert_eq!(version.status.code(), Some(0));
    assert!(text(&version.stdout).starts_with("control-plane "));
}

/// Every setting `serve` reads must be documented in `--help` (the operator's only reference in a
/// container without the repository).
#[test]
fn help_documents_every_serve_setting() {
    let help = text(&control_plane(&["--help"], &[]).stdout);
    for variable in [
        "SAGAT_RATE_LIMIT_RPS",
        "SAGAT_RATE_LIMIT_BURST",
        "SAGAT_SUBJECT_RATE_LIMIT_RPS",
        "SAGAT_SUBJECT_RATE_LIMIT_BURST",
    ] {
        assert!(
            help.contains(variable),
            "--help does not mention {variable}"
        );
    }
}

#[test]
fn unknown_commands_are_a_usage_error() {
    let output = control_plane(&["detonate"], &[]);
    assert_eq!(output.status.code(), Some(2));
    let err = text(&output.stderr);
    assert!(
        err.contains("unknown command 'detonate'") && err.contains("USAGE"),
        "{err}"
    );
}

#[test]
fn a_missing_database_url_is_reported_without_guessing() {
    for command in ["serve", "migrate", "seed-dev"] {
        let output = control_plane(&[command], &[("SAGAT_ENV", "dev")]);
        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(text(&output.stderr).contains("DATABASE_URL"), "{command}");
    }
}

#[test]
fn serve_without_identity_settings_fails_fast() {
    let output = control_plane(&["serve"], &[("DATABASE_URL", SECRET_URL)]);
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("SAGAT_OIDC_ISSUER"));
}

#[test]
fn seed_dev_is_refused_outside_development_before_touching_the_database() {
    for env_value in [None, Some("prod"), Some("staging")] {
        let mut env = vec![("DATABASE_URL", SECRET_URL)];
        if let Some(value) = env_value {
            env.push(("SAGAT_ENV", value));
        }

        let started = Instant::now();
        let output = control_plane(&["seed-dev"], &env);

        assert_eq!(output.status.code(), Some(2), "{env_value:?}");
        assert!(text(&output.stderr).contains("only runs when SAGAT_ENV=dev"));
        // The URL points at a closed port: any connection attempt would add a visible delay or a
        // different error. Refusal must come first.
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!text(&output.stderr).contains("database error"));
    }
}

#[test]
fn the_database_credentials_never_appear_in_any_output() {
    let runs = [
        control_plane(
            &["migrate"],
            &[("DATABASE_URL", SECRET_URL), ("SAGAT_ENV", "dev")],
        ),
        control_plane(
            &["seed-dev"],
            &[("DATABASE_URL", SECRET_URL), ("SAGAT_ENV", "dev")],
        ),
        control_plane(&["seed-dev"], &[("DATABASE_URL", SECRET_URL)]),
        control_plane(&["serve"], &[("DATABASE_URL", SECRET_URL)]),
    ];
    for output in runs {
        let all = format!("{}{}", text(&output.stdout), text(&output.stderr));
        assert!(
            !all.contains("hunter2") && !all.contains("leaky"),
            "credentials leaked: {all}"
        );
    }
}

#[test]
fn an_unreachable_database_is_a_runtime_failure_not_a_usage_error() {
    let output = control_plane(&["migrate"], &[("DATABASE_URL", SECRET_URL)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("database error"));
}

#[test]
fn healthcheck_fails_when_nothing_listens_and_never_hangs() {
    let started = Instant::now();
    let output = control_plane(
        &["healthcheck"],
        &[("SAGAT_HEALTHCHECK_ADDR", "127.0.0.1:1")],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("health check failed"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_plaintext_database_url_is_refused_outside_development_without_leaking_it() {
    for environment in [None, Some("prod"), Some("staging")] {
        let mut env = vec![("DATABASE_URL", PLAINTEXT_URL)];
        if let Some(value) = environment {
            env.push(("SAGAT_ENV", value));
        }
        for command in ["migrate", "serve", "seed-dev"] {
            let output = control_plane(&[command], &env);
            assert_eq!(output.status.code(), Some(2), "{command} {environment:?}");
            let err = text(&output.stderr);
            assert!(err.contains("must require TLS"), "{err}");
            assert!(
                !err.contains("hunter2") && !err.contains("leaky"),
                "credentials leaked: {err}"
            );
        }
    }
}

#[test]
fn provisioning_a_tenant_needs_both_values_and_never_assumes_defaults() {
    let base = [("DATABASE_URL", SECRET_URL), ("SAGAT_ENV", "prod")];
    let none = control_plane(&["provision-tenant"], &base);
    assert_eq!(none.status.code(), Some(2));
    assert!(text(&none.stderr).contains("SAGAT_PROVISION_TENANT"));

    let mut only_slug = base.to_vec();
    only_slug.push(("SAGAT_PROVISION_TENANT", "acme"));
    let missing_admin = control_plane(&["provision-tenant"], &only_slug);
    assert_eq!(missing_admin.status.code(), Some(2));
    assert!(text(&missing_admin.stderr).contains("SAGAT_PROVISION_ADMIN"));
}

#[test]
fn provisioning_rejects_an_invalid_slug_before_touching_the_database() {
    let started = Instant::now();
    let output = control_plane(
        &["provision-tenant"],
        &[
            ("DATABASE_URL", SECRET_URL),
            ("SAGAT_ENV", "prod"),
            ("SAGAT_PROVISION_TENANT", "NOT A SLUG"),
            ("SAGAT_PROVISION_ADMIN", "alice"),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("SAGAT_PROVISION_TENANT"));
    assert!(started.elapsed() < Duration::from_secs(2));
}
