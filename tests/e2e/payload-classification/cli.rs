//! The shipped `sagat` process calls the real grant and disclosure APIs without leaking canaries.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::canary::{
    ADMIN, DEVELOPER, Fixture, NAMESPACE, TENANT, configure_matrix_rate, fixture, variant_runs,
};
use crate::canary_support::{assert_absent, assert_present, audience_cases};

fn target_dir() -> PathBuf {
    std::env::current_exe()
        .expect("test executable")
        .parent()
        .and_then(Path::parent)
        .expect("target debug directory")
        .parent()
        .expect("Cargo target directory")
        .to_owned()
}

pub(crate) async fn cli(fixture: &Fixture, subject: &str, args: &[&str]) -> Output {
    let token = fixture.api.idp.token(subject, TENANT).build();
    let server = fixture.api.base.clone();
    let args = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || {
        Command::new("cargo")
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
            .env("CARGO_TARGET_DIR", target_dir())
            .env("CARGO_INCREMENTAL", "0")
            .env("SAGAT_SERVER", server)
            .env("SAGAT_TOKEN", token)
            .env("SAGAT_NAMESPACE", NAMESPACE)
            .args([
                "run",
                "--quiet",
                "--locked",
                "-p",
                "sagat-cli",
                "--bin",
                "sagat",
                "--",
            ])
            .args(args)
            .output()
            .expect("run the sagat process")
    })
    .await
    .expect("join sagat process")
}

pub(crate) fn output_bytes(output: &Output) -> Vec<u8> {
    let mut all = output.stdout.clone();
    all.extend_from_slice(&output.stderr);
    all
}

#[tokio::test]
async fn every_cli_audience_keeps_invalid_legacy_and_unrequested_data_out_of_both_streams() {
    let fixture = fixture("cli-matrix").await;
    configure_matrix_rate(&fixture).await;
    let mut runs = variant_runs(&fixture).await;
    runs.push(("classified", fixture.run.clone()));
    let mut calls = 0;
    for case in audience_cases() {
        fixture.set_grant(case.subject, case.grant).await;
        for (kind, run) in &runs {
            let mut args = vec!["workflow", "inspect", run.as_str(), "--values"];
            if case.reveal {
                args.push("--reveal-pii");
            }
            let output = cli(&fixture, case.subject, &args).await;
            calls += 1;
            let allowed = case.may_read_runs() && (!case.reveal || case.may_reveal());
            assert_eq!(output.status.success(), allowed, "CLI audience status");
            let wire = output_bytes(&output);
            assert_absent(
                "CLI forbidden values and keys",
                &wire,
                &fixture.canaries.always_forbidden(),
            );
            match *kind {
                "invalid" | "legacy" => {
                    assert_absent("CLI invalid or legacy", &wire, &fixture.canaries.all())
                }
                "unmarked" if allowed && case.may_reveal() => {
                    assert_present(
                        "CLI unmarked reveal",
                        &wire,
                        &fixture.canaries.unmarked_value,
                    );
                    assert_present(
                        "CLI unmarked key reveal",
                        &wire,
                        &fixture.canaries.unmarked_key,
                    );
                }
                "unmarked" => assert_absent("CLI unrequested data", &wire, &fixture.canaries.all()),
                "classified" if allowed => {
                    assert_present(
                        "CLI public projection",
                        &wire,
                        &fixture.canaries.public_value,
                    );
                    if case.may_read_internal() {
                        assert_present(
                            "CLI internal projection",
                            &wire,
                            &fixture.canaries.internal_value,
                        );
                    } else {
                        assert_absent(
                            "CLI hidden internal",
                            &wire,
                            &[&fixture.canaries.internal_value],
                        );
                    }
                    if case.may_reveal() {
                        assert_present(
                            "CLI personal projection",
                            &wire,
                            &fixture.canaries.pii_value,
                        );
                        for key in [
                            &fixture.canaries.public_key,
                            &fixture.canaries.internal_key,
                            &fixture.canaries.pii_key,
                            &fixture.canaries.undeclared_key,
                            &fixture.canaries.undeclared_value,
                        ] {
                            assert_present("CLI explicitly revealed key or value", &wire, key);
                        }
                    } else {
                        assert_absent(
                            "CLI hidden personal",
                            &wire,
                            &[
                                &fixture.canaries.pii_value,
                                &fixture.canaries.pii_key,
                                &fixture.canaries.public_key,
                                &fixture.canaries.internal_key,
                                &fixture.canaries.undeclared_key,
                                &fixture.canaries.undeclared_value,
                            ],
                        );
                    }
                }
                "classified" => assert_absent("CLI refused", &wire, &fixture.canaries.all()),
                _ => panic!("unknown canary fixture class"),
            }
        }
    }
    assert_eq!(calls, 6 * 2 * 2 * 4);
}

#[tokio::test]
async fn actual_cli_grants_discloses_revokes_and_never_prints_secret_values() {
    let fixture = fixture("cli").await;
    let before = cli(
        &fixture,
        DEVELOPER,
        &["workflow", "inspect", &fixture.run, "--values"],
    )
    .await;
    assert!(before.status.success());
    let before = output_bytes(&before);
    assert_present("CLI public values", &before, &fixture.canaries.public_value);
    assert_present(
        "CLI internal values",
        &before,
        &fixture.canaries.internal_value,
    );
    assert_absent(
        "CLI default values",
        &before,
        &[
            &fixture.canaries.pii_value,
            &fixture.canaries.secret_value,
            &fixture.canaries.undeclared_key,
            &fixture.canaries.undeclared_value,
        ],
    );

    let grant = cli(
        &fixture,
        ADMIN,
        &["namespace", "reveal-grants", "grant", DEVELOPER],
    )
    .await;
    assert!(grant.status.success());
    assert_absent(
        "CLI grant output",
        output_bytes(&grant),
        &fixture.canaries.all(),
    );

    let revealed = cli(
        &fixture,
        DEVELOPER,
        &[
            "workflow",
            "inspect",
            &fixture.run,
            "--values",
            "--reveal-pii",
        ],
    )
    .await;
    assert!(revealed.status.success());
    let revealed = output_bytes(&revealed);
    for expected in [
        &fixture.canaries.public_value,
        &fixture.canaries.internal_value,
        &fixture.canaries.pii_value,
        &fixture.canaries.undeclared_key,
        &fixture.canaries.undeclared_value,
    ] {
        assert_present("CLI explicit reveal", &revealed, expected);
    }
    assert_absent(
        "CLI explicit reveal",
        &revealed,
        &[&fixture.canaries.secret_value],
    );

    let revoke = cli(
        &fixture,
        ADMIN,
        &["namespace", "reveal-grants", "revoke", DEVELOPER],
    )
    .await;
    assert!(revoke.status.success());
    assert_absent(
        "CLI revoke output",
        output_bytes(&revoke),
        &fixture.canaries.all(),
    );
    let denied = cli(
        &fixture,
        DEVELOPER,
        &[
            "workflow",
            "inspect",
            &fixture.run,
            "--values",
            "--reveal-pii",
        ],
    )
    .await;
    assert!(!denied.status.success());
    assert_absent(
        "CLI reveal denial",
        output_bytes(&denied),
        &fixture.canaries.all(),
    );
}
