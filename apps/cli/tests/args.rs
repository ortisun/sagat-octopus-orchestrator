//! The command line: every command, its required flags and what is refused.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_cli::args::{Command, parse};

fn argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn the_commands_take_the_flags_they_document() {
    let publish = parse(&argv(
        "publish --dry-run --accept-breaking --server https://p.example/v1 -f deploy/m.yaml",
    ))
    .expect("parses");
    assert_eq!(
        publish.command,
        Command::Publish {
            dry_run: true,
            accept_breaking: true
        }
    );
    assert_eq!(
        publish.common.server.as_deref(),
        Some("https://p.example/v1")
    );
    assert_eq!(
        publish.common.file.as_deref().and_then(|p| p.to_str()),
        Some("deploy/m.yaml")
    );

    let promote = parse(&argv(
        "promote --workflow checkout --to staging --reason=release-train-42 --version 3",
    ))
    .expect("parses");
    assert_eq!(
        promote.command,
        Command::Promote {
            workflow: "checkout".into(),
            to: "staging".into(),
            reason: "release-train-42".into(),
            version: Some(3),
            accept_breaking: false
        }
    );

    let rollback =
        parse(&argv("rollback --workflow checkout --reason bad-deploy")).expect("parses");
    assert_eq!(
        rollback.command,
        Command::Rollback {
            workflow: "checkout".into(),
            reason: "bad-deploy".into(),
            to: None,
            accept_breaking: false
        }
    );

    assert_eq!(
        parse(&argv("diff --check")).expect("parses").command,
        Command::Diff { check: true }
    );
    assert_eq!(
        parse(&argv("history --workflow checkout --limit 9000"))
            .expect("parses")
            .command,
        Command::History {
            workflow: "checkout".into(),
            limit: 200
        },
        "the page asked for is clamped to what the platform serves"
    );
    assert_eq!(
        parse(&argv("requests --workflow checkout --state PENDING"))
            .expect("parses")
            .command,
        Command::Requests {
            workflow: "checkout".into(),
            state: Some("PENDING".into())
        }
    );
    assert_eq!(
        parse(&argv("approve --request 0192 --reason looks-right"))
            .expect("parses")
            .command,
        Command::Decide {
            request: "0192".into(),
            reason: "looks-right".into(),
            approve: true
        }
    );
    assert_eq!(
        parse(&argv("reject --request 0192 --reason not-ready"))
            .expect("parses")
            .command,
        Command::Decide {
            request: "0192".into(),
            reason: "not-ready".into(),
            approve: false
        }
    );
    assert_eq!(parse(&[]).expect("parses").command, Command::Help);
    assert_eq!(
        parse(&argv("--help")).expect("parses").command,
        Command::Help
    );
}

#[test]
fn what_the_command_line_gets_wrong_is_named() {
    for (line, mentions) in [
        ("frobnicate", "not a command"),
        (
            "promote --workflow checkout --to staging",
            "--reason is required",
        ),
        ("rollback --reason bad-deploy", "--workflow is required"),
        ("export", "--out is required"),
        ("approve --reason fine-by-me", "--request is required"),
        ("reject --request 0192", "--reason is required"),
        ("validate --dry-run", "--dry-run is not a flag"),
        ("diff --server", "--server needs a value"),
        ("diff extra", "unexpected argument"),
        (
            "promote --workflow a --to b --reason c --version 0",
            "--version is a number",
        ),
        ("history --workflow a --limit many", "--limit is a number"),
        ("diff --check=yes", "does not take a value"),
    ] {
        let error = parse(&argv(line)).expect_err(line);
        assert!(error.0.contains(mentions), "{line}: {}", error.0);
    }
}

#[test]
fn a_token_cannot_be_given_as_an_argument() {
    // Arguments are visible to every user of the machine: the token comes from a file or the
    // environment, so there is no flag for it to be mistaken for.
    for line in ["diff --token abc", "diff --token=abc", "diff --bearer abc"] {
        assert!(parse(&argv(line)).is_err(), "{line}");
    }
}
