#![allow(clippy::expect_used, clippy::panic)]

mod support;

use std::sync::mpsc;

use sagat_cli::Env;
use support::{Reply, request_line, server};

const NAMESPACE: &str = "018f7789-92ca-7f8b-a8e4-1d3f58e4a901";
const TOKEN: &str = "grant-token-must-never-appear";

fn namespace_reply() -> Reply {
    Reply::ok(
        r#"{"items":[{"id":"018f7789-92ca-7f8b-a8e4-1d3f58e4a901","slug":"dev","environment":"dev"}]}"#,
    )
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
fn list_reads_every_bounded_page_and_sanitizes_subjects_and_actors() {
    let (code, out, err, requests) = run(
        &["namespace", "reveal-grants", "list"],
        vec![
            namespace_reply(),
            Reply::ok(
                r#"{"items":[{"subject":"dev\u202e-one","grantedBy":"admin\nforged","grantedAt":"2026-10-17T10:00:00Z"}],"nextCursor":"next page"}"#,
            ),
            Reply::ok(
                r#"{"items":[{"subject":"dev-two","grantedBy":"admin","grantedAt":"2026-10-17T11:00:00Z"}]}"#,
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("payload reveal grants in dev:"), "{out}");
    assert!(
        out.contains("dev?-one") && out.contains("admin forged"),
        "{out}"
    );
    assert!(out.contains("dev-two"), "{out}");
    assert!(!out.contains('\u{202e}'));
    let lines: Vec<_> = requests
        .iter()
        .map(|request| request_line(&request).to_owned())
        .collect();
    assert_eq!(
        lines,
        [
            "GET /v1/namespaces?limit=50 HTTP/1.1".to_owned(),
            format!("GET /v1/namespaces/{NAMESPACE}/payload-reveal-grants?limit=200 HTTP/1.1"),
            format!(
                "GET /v1/namespaces/{NAMESPACE}/payload-reveal-grants?limit=200&cursor=next%20page HTTP/1.1"
            ),
        ]
    );
}

#[test]
fn grant_and_revoke_encode_the_subject_and_print_only_safe_receipts() {
    let subject = "oidc|developer@example.test";
    let (grant_code, grant_out, grant_err, grant_requests) = run(
        &["namespace", "reveal-grants", "grant", subject],
        vec![
            namespace_reply(),
            Reply::ok(
                r#"{"subject":"oidc|developer@example.test","grantedBy":"admin","grantedAt":"2026-10-17T10:00:00Z"}"#,
            ),
        ],
    );
    assert_eq!(grant_code, 0, "{grant_err}");
    assert_eq!(
        grant_out,
        "granted payload reveal to oidc|developer@example.test in dev at 2026-10-17T10:00:00Z by admin\n"
    );
    let grant_lines: Vec<_> = grant_requests
        .iter()
        .map(|request| request_line(&request).to_owned())
        .collect();
    assert_eq!(
        grant_lines[1],
        format!(
            "PUT /v1/namespaces/{NAMESPACE}/payload-reveal-grants/oidc%7Cdeveloper%40example.test HTTP/1.1"
        )
    );

    let (revoke_code, revoke_out, revoke_err, revoke_requests) = run(
        &["namespace", "reveal-grants", "revoke", subject],
        vec![namespace_reply(), Reply::ok("{}")],
    );
    assert_eq!(revoke_code, 0, "{revoke_err}");
    assert_eq!(
        revoke_out,
        "revoked payload reveal from oidc|developer@example.test in dev\n"
    );
    let revoke_lines: Vec<_> = revoke_requests
        .iter()
        .map(|request| request_line(&request).to_owned())
        .collect();
    assert_eq!(
        revoke_lines[1],
        format!(
            "DELETE /v1/namespaces/{NAMESPACE}/payload-reveal-grants/oidc%7Cdeveloper%40example.test HTTP/1.1"
        )
    );
}

#[test]
fn closed_grant_failures_have_failure_exit_codes_without_echoing_the_token() {
    for (status, code, reason) in [
        (422, "VALIDATION_FAILED", "UNKNOWN_SUBJECT"),
        (422, "VALIDATION_FAILED", "ROLE_CANNOT_REVEAL"),
        (429, "QUOTA_EXCEEDED", "GRANT_LIMIT"),
        (404, "NOT_FOUND", "MISSING"),
    ] {
        let operation = if status == 404 { "revoke" } else { "grant" };
        let body = format!(
            "{{\"code\":\"{code}\",\"message\":\"safe refusal\",\"retryable\":false,\"traceId\":\"0123456789abcdef0123456789abcdef\",\"details\":{{\"reason\":\"{reason}\"}}}}"
        );
        let (exit, out, err, _) = run(
            &["namespace", "reveal-grants", operation, "developer"],
            vec![namespace_reply(), Reply { status, body }],
        );
        assert_eq!(exit, 1, "{reason}: {out}\n{err}");
        assert!(!out.contains(TOKEN) && !err.contains(TOKEN));
        assert!(err.contains(code) && err.contains(reason), "{err}");
    }
}

#[test]
fn namespace_resolution_failure_does_not_echo_server_text_or_the_token() {
    let (exit, out, err, _) = run(
        &["namespace", "reveal-grants", "list"],
        vec![Reply {
            status: 403,
            body: format!(
                "{{\"code\":\"FORBIDDEN\",\"message\":\"{TOKEN}\",\"details\":{{\"reason\":\"NAMESPACE_FORBIDDEN\",\"value\":\"{TOKEN}\"}}}}"
            ),
        }],
    );
    assert_eq!(exit, 1);
    assert!(!out.contains(TOKEN) && !err.contains(TOKEN), "{out}\n{err}");
    assert!(
        err.contains("FORBIDDEN") && err.contains("NAMESPACE_FORBIDDEN"),
        "{err}"
    );
}

#[test]
fn namespace_resolution_follows_bounded_pages() {
    let (exit, out, err, requests) = run(
        &["namespace", "reveal-grants", "list"],
        vec![
            Reply::ok(r#"{"items":[],"nextCursor":"second page"}"#),
            namespace_reply(),
            Reply::ok(r#"{"items":[]}"#),
        ],
    );
    assert_eq!(exit, 0, "{out}\n{err}");
    let lines: Vec<_> = requests
        .iter()
        .map(|request| request_line(&request).to_owned())
        .collect();
    assert_eq!(
        lines[1],
        "GET /v1/namespaces?limit=50&cursor=second%20page HTTP/1.1"
    );
}
