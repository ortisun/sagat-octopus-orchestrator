//! The command line: every command, its required flags and what is refused.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_cli::args::{Command, parse};

fn argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn the_version_of_the_tool_is_asked_for_in_either_way() {
    for line in ["version", "--version", "-V"] {
        assert_eq!(
            parse(&argv(line)).expect("parses").command,
            Command::Version,
            "{line}"
        );
    }
    // And as a flag with a value it is still the version of a workflow.
    assert!(matches!(
        parse(&argv(
            "promote --workflow w --to prod --reason r --version 3"
        ))
        .expect("parses")
        .command,
        Command::Promote {
            version: Some(3),
            ..
        }
    ));
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
        parse(&argv(
            "approve --request 01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8 --reason looks-right"
        ))
        .expect("parses")
        .command,
        Command::Decide {
            request: "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8".into(),
            reason: "looks-right".into(),
            approve: true
        }
    );
    assert_eq!(
        parse(&argv(
            "reject --request 01A12ACA-0D9D-7589-BF01-BC7B7CC0A7B8 --reason not-ready"
        ))
        .expect("parses")
        .command,
        Command::Decide {
            request: "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8".into(),
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
        (
            "reject --request 01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8",
            "--reason is required",
        ),
        ("validate --dry-run", "--dry-run is not a flag"),
        ("diff --server", "--server needs a value"),
        ("diff extra", "unexpected argument"),
        (
            "promote --workflow a --to b --reason c --version 0",
            "--version is a number",
        ),
        ("history --workflow a --limit many", "--limit is a number"),
        ("diff --check=yes", "does not take a value"),
        (
            "approve --request ../../x --reason fine-by-me",
            "--request is the id of a request",
        ),
        (
            "reject --request 0192 --reason fine-by-me",
            "--request is the id of a request",
        ),
        (
            "approve --request 01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8/reject?x --reason fine-by-me",
            "--request is the id of a request",
        ),
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

#[test]
fn what_is_shown_on_a_terminal_has_no_control_characters_but_the_layout() {
    use sagat_cli::printable;
    assert_eq!(
        printable("plain text, with accents é and tabs\t"),
        "plain text, with accents é and tabs\t"
    );
    assert_eq!(
        printable("\u{1b}[2J\u{1b}[31mred\u{1b}[0m"),
        "?[2J?[31mred?[0m"
    );
    assert_eq!(printable("hidden\rAPPROVED"), "hidden?APPROVED");
    assert_eq!(printable("a\u{7f}b\u{85}c\u{0}d"), "a?b?c?d");
    assert_eq!(printable("two\nlines"), "two\nlines");
}

#[test]
fn text_that_reorders_or_hides_other_text_is_not_shown_and_a_new_line_is_not_forged() {
    use sagat_cli::{one_line, printable};
    // A bidirectional override, a zero-width space, a line separator and the byte order mark.
    assert_eq!(
        printable("a\u{202e}b\u{200b}c\u{2028}d\u{feff}e"),
        "a?b?c?d?e"
    );
    assert_eq!(printable("an é and a 日本語"), "an é and a 日本語");
    assert_eq!(
        one_line("fix\nPUBLISH v9 by root\tx"),
        "fix PUBLISH v9 by root x"
    );
}

const RUN: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8";

#[test]
fn the_commands_for_one_workflow_take_the_words_and_flags_they_document() {
    assert_eq!(
        parse(&argv("workflow validate flows/checkout.json"))
            .expect("parses")
            .command,
        Command::WorkflowValidate {
            file: "flows/checkout.json".into()
        }
    );
    assert_eq!(
        parse(&argv(
            "workflow publish checkout.yaml --notes first-release --dry-run --accept-breaking"
        ))
        .expect("parses")
        .command,
        Command::WorkflowPublish {
            file: "checkout.yaml".into(),
            notes: Some("first-release".into()),
            dry_run: true,
            accept_breaking: true
        }
    );
    assert_eq!(
        parse(&argv(
            "workflow run checkout --input @in.json --wait --timeout 30 --key k-0123456789"
        ))
        .expect("parses")
        .command,
        Command::WorkflowRun {
            name: "checkout".into(),
            input: Some("@in.json".into()),
            wait: true,
            timeout: 30,
            key: Some("k-0123456789".into())
        }
    );
    assert_eq!(
        parse(&argv("workflow run checkout"))
            .expect("parses")
            .command,
        Command::WorkflowRun {
            name: "checkout".into(),
            input: None,
            wait: false,
            timeout: 300,
            key: None
        }
    );
    assert_eq!(
        parse(&argv(&format!("workflow inspect {RUN}")))
            .expect("parses")
            .command,
        Command::WorkflowInspect {
            run: RUN.into(),
            values: false,
            reveal_pii: false,
            sequence: None,
        }
    );
    assert_eq!(
        parse(&argv(&format!(
            "workflow inspect {RUN} --values --reveal-pii --sequence 42"
        )))
        .expect("parses")
        .command,
        Command::WorkflowInspect {
            run: RUN.into(),
            values: true,
            reveal_pii: true,
            sequence: Some(42),
        }
    );
    assert_eq!(
        parse(&argv(&format!(
            "workflow cancel {RUN} --reason changed-our-minds"
        )))
        .expect("parses")
        .command,
        Command::WorkflowCancel {
            run: RUN.into(),
            reason: "changed-our-minds".into()
        }
    );
    assert_eq!(
        parse(&argv(&format!(
            "workflow rerun {RUN} --reason fixed-the-worker"
        )))
        .expect("parses")
        .command,
        Command::WorkflowRerun {
            run: RUN.into(),
            reason: "fixed-the-worker".into()
        }
    );
    assert_eq!(
        parse(&argv("workflow runs checkout --limit 500"))
            .expect("parses")
            .command,
        Command::WorkflowRuns {
            name: "checkout".into(),
            limit: 200
        }
    );
}

#[test]
fn reveal_grant_commands_are_closed_and_subjects_are_validated() {
    assert_eq!(
        parse(&argv("namespace reveal-grants list"))
            .expect("parses")
            .command,
        Command::RevealGrantsList
    );
    assert_eq!(
        parse(&argv(
            "namespace reveal-grants grant oidc|developer@example.test"
        ))
        .expect("parses")
        .command,
        Command::RevealGrant {
            subject: "oidc|developer@example.test".into()
        }
    );
    assert_eq!(
        parse(&argv("namespace reveal-grants revoke dev-developer"))
            .expect("parses")
            .command,
        Command::RevealRevoke {
            subject: "dev-developer".into()
        }
    );
    for (line, mentions) in [
        ("namespace reveal-grants grant system:engine", "subject"),
        ("namespace reveal-grants revoke", "subject is required"),
        (
            "namespace reveal-grants unknown",
            "not a command (list, grant, revoke)",
        ),
    ] {
        let error = parse(&argv(line)).expect_err(line);
        assert!(error.0.contains(mentions), "{line}: {}", error.0);
    }
}

#[test]
fn disclosure_only_flags_require_values_and_sequence_is_a_positive_i64() {
    let cases = [
        (
            format!("workflow inspect {RUN} --reveal-pii"),
            "--reveal-pii needs --values",
        ),
        (
            format!("workflow inspect {RUN} --sequence 1"),
            "--sequence needs --values",
        ),
        (
            format!("workflow inspect {RUN} --values --sequence 0"),
            "--sequence is a positive sequence",
        ),
        (
            format!("workflow inspect {RUN} --values --sequence 9223372036854775808"),
            "--sequence is a positive sequence",
        ),
    ];
    for (line, mentions) in cases {
        let error = parse(&argv(&line)).expect_err(&line);
        assert!(error.0.contains(mentions), "{line}: {}", error.0);
    }
}

#[test]
fn input_that_is_json_survives_the_equals_form_and_spaces() {
    let args = vec![
        "workflow".to_owned(),
        "run".to_owned(),
        "x-flow".to_owned(),
        r#"--input={"name":"Ada Lovelace","n":[1,2]}"#.to_owned(),
    ];
    match parse(&args).expect("parses").command {
        Command::WorkflowRun { input, .. } => assert_eq!(
            input.as_deref(),
            Some(r#"{"name":"Ada Lovelace","n":[1,2]}"#)
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn scripts_workers_doctor_and_templates_have_their_commands() {
    assert_eq!(
        parse(&argv(
            "script test enrich.ts --input {} --language typescript --name enrich --timeout 20"
        ))
        .expect("parses")
        .command,
        Command::ScriptTest {
            file: "enrich.ts".into(),
            input: Some("{}".into()),
            language: Some("typescript".into()),
            name: Some("enrich".into()),
            timeout: 20
        }
    );
    assert_eq!(
        parse(&argv("script publish enrich.py"))
            .expect("parses")
            .command,
        Command::ScriptPublish {
            file: "enrich.py".into(),
            language: None,
            name: None,
            timeout: 150
        }
    );
    assert_eq!(
        parse(&argv("worker dev --mocks m.yaml --workflow w.json --bind --idle-exit 5 --max-tasks 3 --show-payloads --allow-non-dev --grpc http://127.0.0.1:19000")).expect("parses"),
        sagat_cli::args::Invocation {
            common: sagat_cli::args::Common { grpc: Some("http://127.0.0.1:19000".into()), ..Default::default() },
            command: Command::WorkerDev {
                mocks: "m.yaml".into(),
                workflow: Some("w.json".into()),
                bind: true,
                idle_exit: Some(5),
                max_tasks: Some(3),
                show_payloads: true,
                allow_non_dev: true
            }
        }
    );
    assert_eq!(
        parse(&argv(
            "worker bind --workflow w.json payments.charge payments.refund"
        ))
        .expect("parses")
        .command,
        Command::WorkerBind {
            workflow: Some("w.json".into()),
            activities: vec!["payments.charge".into(), "payments.refund".into()],
            allow_non_dev: false
        }
    );
    assert_eq!(
        parse(&argv("dev token")).expect("parses").command,
        Command::DevToken {
            idp: None,
            subject: "dev-admin".into(),
            tenant: "dev".into(),
            ttl: None,
            out: None
        }
    );
    assert_eq!(
        parse(&argv("dev token --idp http://127.0.0.1:18090 --subject alice --tenant acme --ttl 600 --out t.txt"))
            .expect("parses")
            .command,
        Command::DevToken {
            idp: Some("http://127.0.0.1:18090".into()),
            subject: "alice".into(),
            tenant: "acme".into(),
            ttl: Some(600),
            out: Some("t.txt".into())
        }
    );
    assert_eq!(
        parse(&argv("worker bind payments.charge"))
            .expect("parses")
            .command,
        Command::WorkerBind {
            workflow: None,
            activities: vec!["payments.charge".into()],
            allow_non_dev: false
        }
    );
    assert_eq!(
        parse(&argv("doctor --json --nats nats://127.0.0.1:14222"))
            .expect("parses")
            .command,
        Command::Doctor {
            json: true,
            nats: Some("nats://127.0.0.1:14222".into())
        }
    );
    assert_eq!(
        parse(&argv("new compensation ./payments --force"))
            .expect("parses")
            .command,
        Command::New {
            template: Some("compensation".into()),
            dir: Some("./payments".into()),
            force: true
        }
    );
    assert_eq!(
        parse(&argv("new --list")).expect("parses").command,
        Command::New {
            template: None,
            dir: None,
            force: false
        }
    );
}

#[test]
fn the_new_commands_name_what_is_wrong() {
    for (line, mentions) in [
        ("workflow", "workflow needs a command"),
        (
            "workflow frobnicate",
            "'workflow frobnicate' is not a command",
        ),
        ("workflow validate", "the document (a file) is required"),
        (
            "workflow validate a.json b.json",
            "unexpected argument 'b.json'",
        ),
        ("workflow run", "the name of the workflow is required"),
        (
            "workflow run checkout --timeout 0",
            "--timeout is a number of seconds",
        ),
        (
            "workflow run checkout --timeout 999999",
            "--timeout is a number of seconds",
        ),
        ("workflow inspect not-an-id", "the run is given by its id"),
        ("workflow inspect ../../x", "the run is given by its id"),
        (
            "workflow cancel 01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8",
            "--reason is required",
        ),
        (
            "workflow rerun 01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8",
            "--reason is required",
        ),
        (
            "workflow rerun not-an-id --reason x",
            "the run is given by its id",
        ),
        ("script", "script needs a command"),
        ("script frobnicate", "'script frobnicate' is not a command"),
        ("script test", "the script (a file) is required"),
        ("worker", "worker needs a command"),
        ("worker dev", "--mocks is required"),
        ("worker nap", "'worker nap' is not a command"),
        ("worker bind", "worker bind needs a --workflow or the names"),
        ("dev", "dev needs a command"),
        ("dev login", "'dev login' is not a command"),
        (
            "dev token --ttl 0",
            "--ttl is a number of seconds from 1 to 3600",
        ),
        ("dev token extra", "unexpected argument 'extra'"),
        ("new", "a template is required"),
        ("doctor extra", "unexpected argument 'extra'"),
        ("validate extra", "unexpected argument 'extra'"),
    ] {
        let error = parse(&argv(line)).expect_err(line);
        assert!(error.0.contains(mentions), "{line}: {}", error.0);
    }
}

#[test]
fn the_usage_names_every_command_the_parser_takes() {
    for command in [
        "doctor",
        "new <TEMPLATE>",
        "workflow validate",
        "workflow publish",
        "workflow run",
        "workflow runs",
        "workflow inspect",
        "workflow cancel",
        "workflow rerun",
        "namespace reveal-grants list",
        "namespace reveal-grants grant",
        "namespace reveal-grants revoke",
        "script test",
        "script publish",
        "worker dev",
        "worker bind",
        "dev token",
        "validate",
        "diff",
        "export",
        "publish",
        "promote",
        "rollback",
        "history",
        "requests",
        "approve",
        "reject",
    ] {
        assert!(
            sagat_cli::args::USAGE.contains(command),
            "the usage does not mention {command}"
        );
    }
    assert!(sagat_cli::args::USAGE.contains("6 it did not finish in time"));
}
