#![allow(clippy::expect_used, clippy::panic)]

mod support;

use std::sync::mpsc;

use sagat_cli::Env;
use serde_json::json;
use support::{Reply, request_body, request_line, server};

const RUN: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8";
const TOKEN: &str = "secret-cli-token-must-never-appear";

fn metadata_replies() -> Vec<Reply> {
    vec![
        Reply::ok(
            r#"{"status":"SUCCEEDED","workflowId":"workflow-1","versionId":"version-1","createdAt":"2026-10-17T10:00:00Z","startedBy":"alice","finishedAt":"2026-10-17T10:00:01Z"}"#,
        ),
        Reply::ok(r#"{"items":[]}"#),
        Reply::ok(r#"{"items":[]}"#),
    ]
}

fn run(arguments: &[&str], replies: Vec<Reply>) -> (u8, String, String, mpsc::Receiver<Vec<u8>>) {
    let (server, requests) = server(replies);
    let env = Env {
        server: Some(format!("{server}/v1")),
        token: Some(TOKEN.to_owned()),
        namespace: Some("dev".to_owned()),
        ..Env::default()
    };
    let args: Vec<String> = arguments.iter().map(|value| (*value).to_owned()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(sagat_cli::run(&args, &env, &mut out, &mut err));
    (
        code,
        String::from_utf8(out).expect("stdout"),
        String::from_utf8(err).expect("stderr"),
        requests,
    )
}

#[test]
fn inspect_without_values_keeps_the_existing_output_and_makes_no_disclosure_call() {
    let (code, out, err, requests) = run(&["workflow", "inspect", RUN], metadata_replies());
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        format!(
            "run {RUN}  SUCCEEDED  workflow workflow-1  version version-1\n  started 2026-10-17T10:00:00Z by alice, finished 2026-10-17T10:00:01Z\njournal (0 entries):\n"
        )
    );
    assert!(err.is_empty());
    let lines: Vec<_> = requests
        .iter()
        .map(|request| request_line(&request).to_owned())
        .collect();
    assert_eq!(
        lines,
        [
            format!("GET /v1/runs/{RUN} HTTP/1.1"),
            format!("GET /v1/runs/{RUN}/events?limit=200 HTTP/1.1"),
            format!("GET /v1/runs/{RUN}/attempts HTTP/1.1"),
        ]
    );
}

#[test]
fn values_render_only_the_server_projection_and_neutralise_terminal_controls() {
    let mut replies = metadata_replies();
    replies.push(Reply::ok(
        r#"{"runId":"01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8","runVersion":3,"selection":{"source":"input"},"policy":"classified-v1","caller":{"disclosedLevels":["public","internal"],"revealPii":"not_granted","revealed":false},"kind":"disclosed","schema":{"carrier":"inputSchema","versionNumber":3,"rules":1},"sizeBytes":99,"truncated":true,"tree":{"kind":"container","type":"object","level":"public","childCount":4,"omittedChildren":1,"children":[{"ordinal":0,"name":"customer\u202e","node":{"kind":"value","type":"string","level":"public","value":"Ada\n\u001b[2J\u202e"}},{"ordinal":1,"name":"profile","node":{"kind":"withheld","type":"object","level":"pii","reason":"LEVEL_NOT_PERMITTED"}},{"ordinal":2,"node":{"kind":"withheld","type":"string","level":"secret","reason":"SECRET"}}]}}"#,
    ));
    replies.push(Reply::ok(
        r#"{"runId":"01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8","runVersion":3,"selection":{"source":"result"},"policy":"classified-v1","caller":{"disclosedLevels":["public","internal"],"revealPii":"not_granted","revealed":false},"kind":"unavailable","reason":"NO_RETAINED_PAYLOAD"}"#,
    ));
    let (code, out, err, requests) = run(&["workflow", "inspect", RUN, "--values"], replies);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("input values:"), "{out}");
    assert!(
        out.contains("result values: [unavailable: NO_RETAINED_PAYLOAD]"),
        "{out}"
    );
    assert!(out.contains("[withheld: pii]"), "{out}");
    assert!(out.contains("[withheld: secret]"), "{out}");
    assert!(out.contains("1 omitted"), "{out}");
    assert!(out.contains("customer?"), "{out}");
    assert!(out.contains(r#""Ada\n?[2J?""#), "{out}");
    assert!(!out.contains('\u{1b}') && !out.contains('\u{202e}'));
    assert!(err.is_empty());

    let recorded: Vec<_> = requests.iter().collect();
    assert_eq!(recorded.len(), 5);
    assert_eq!(request_body(&recorded[3]), json!({"source": "input"}));
    assert_eq!(request_body(&recorded[4]), json!({"source": "result"}));
}

#[test]
fn sequence_selects_only_that_journal_payload_and_reveal_is_explicit() {
    let mut replies = metadata_replies();
    replies.push(Reply::ok(
        r#"{"runId":"01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8","runVersion":3,"selection":{"source":"journal","sequence":42},"policy":"classified-v1","caller":{"disclosedLevels":["public","internal","pii"],"revealPii":"granted","revealed":true},"kind":"disclosed","schema":{"carrier":"nodeOutputSchema","versionNumber":3,"rules":1},"sizeBytes":4,"truncated":false,"tree":{"kind":"value","type":"boolean","level":"pii","value":true}}"#,
    ));
    let (code, out, err, requests) = run(
        &[
            "workflow",
            "inspect",
            RUN,
            "--values",
            "--reveal-pii",
            "--sequence",
            "42",
        ],
        replies,
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("journal 42 values:"), "{out}");
    let recorded: Vec<_> = requests.iter().collect();
    assert_eq!(recorded.len(), 4);
    assert_eq!(
        request_body(&recorded[3]),
        json!({"source": "journal", "sequence": 42, "reveal": ["pii"]})
    );
}

#[test]
fn a_disclosure_error_never_echoes_a_token_or_server_supplied_payload_value() {
    let canary = "pii-canary-must-not-appear";
    let mut replies = metadata_replies();
    replies.push(Reply {
        status: 403,
        body: r#"{"code":"FORBIDDEN","message":"pii-canary-must-not-appear","retryable":false,"traceId":"0123456789abcdef0123456789abcdef","details":{"reason":"PII_REVEAL_NOT_GRANTED","value":"pii-canary-must-not-appear"}}"#.to_owned(),
    });
    let (code, out, err, _) = run(
        &["workflow", "inspect", RUN, "--values", "--reveal-pii"],
        replies,
    );
    assert_eq!(code, 1);
    assert!(
        !out.contains(canary) && !err.contains(canary),
        "{out}\n{err}"
    );
    assert!(!out.contains(TOKEN) && !err.contains(TOKEN), "{out}\n{err}");
    assert!(
        err.contains("FORBIDDEN") && err.contains("PII_REVEAL_NOT_GRANTED"),
        "{err}"
    );
}

#[test]
fn disclosed_numbers_keep_the_exact_server_lexeme() {
    let mut replies = metadata_replies();
    replies.push(Reply::ok(
        r#"{"kind":"disclosed","tree":{"kind":"container","type":"array","level":"public","childCount":2,"omittedChildren":0,"children":[{"ordinal":0,"node":{"kind":"value","type":"number","level":"public","value":9007199254740993}},{"ordinal":1,"node":{"kind":"value","type":"number","level":"public","value":1.0000000000000000000000000001}}]}}"#,
    ));
    replies.push(Reply::ok(
        r#"{"kind":"unavailable","reason":"NO_RETAINED_PAYLOAD"}"#,
    ));
    let (code, out, err, _) = run(&["workflow", "inspect", RUN, "--values"], replies);
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("9007199254740993"), "{out}");
    assert!(out.contains("1.0000000000000000000000000001"), "{out}");
    assert!(!out.contains("9007199254740992"), "{out}");
}
