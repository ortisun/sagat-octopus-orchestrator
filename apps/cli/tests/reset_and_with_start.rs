//! The commands of feature 018 for runs (`T018-069`, `FR-018-047`): the options of a start, the two
//! with-start operations and the reset. Each is a request of the REST API and its answer in words; the tool
//! decides nothing the platform decides. A mock platform, the real command line.
#![allow(clippy::expect_used, clippy::panic)]

mod support;

use std::sync::mpsc;

use sagat_cli::Env;
use serde_json::{Value, json};
use support::{Reply, request_body, request_line, server};

const RUN: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8";
const NEW: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b9";
const TOKEN: &str = "secret-cli-token-must-never-appear";
const WORKFLOW: &str = "5b3f9a16-0000-7000-8000-000000000001";

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

fn workflows() -> Reply {
    Reply::ok(
        json!({"items": [{
            "id": WORKFLOW, "name": "checkout", "draftRevision": 1,
            "activeVersion": {"versionNumber": 3, "digest": "sha256:abc"}
        }]})
        .to_string(),
    )
}

fn reply(status: u16, body: &Value) -> Reply {
    Reply {
        status,
        body: body.to_string(),
    }
}

fn error(code: &str, reason: &str, message: &str) -> Value {
    json!({"code": code, "message": message, "retryable": false, "details": {"reason": reason}})
}

/// The requests the mock received, as `(request line, body)`; a request without a body has `null`.
fn received(requests: &mpsc::Receiver<Vec<u8>>) -> Vec<(String, Value)> {
    requests
        .try_iter()
        .map(|request| {
            let at = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .expect("header end")
                + 4;
            let body = if request.len() > at {
                request_body(&request)
            } else {
                Value::Null
            };
            (request_line(&request).to_owned(), body)
        })
        .collect()
}

fn header(request: &[u8], name: &str) -> Option<String> {
    std::str::from_utf8(request).ok()?.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

#[test]
fn a_run_sends_the_identifier_the_policies_and_the_delay_in_the_names_of_the_api() {
    let (code, out, err, requests) = run(
        &[
            "workflow",
            "run",
            "checkout",
            "--business-id",
            "order-1001",
            "--if-exists",
            "use-existing",
            "--reuse",
            "reject-duplicate",
            "--start-delay",
            "90",
        ],
        vec![
            workflows(),
            reply(
                202,
                &json!({"runId": RUN, "status": "WAITING", "startDelayedUntil": "2026-11-06T12:01:30Z"}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("run {RUN} started: checkout v3")),
        "{out}"
    );
    assert!(
        out.contains("its first step is held until 2026-11-06T12:01:30Z"),
        "{out}"
    );
    let calls = received(&requests);
    assert_eq!(calls[1].0, "POST /v1/runs HTTP/1.1");
    assert_eq!(
        calls[1].1,
        json!({
            "workflowId": WORKFLOW, "input": {}, "businessId": "order-1001",
            "businessIdConflictPolicy": "USE_EXISTING", "businessIdReusePolicy": "REJECT_DUPLICATE",
            "startDelayMs": 90_000
        })
    );
}

#[test]
fn a_run_without_the_options_sends_none_of_them() {
    let (code, _, err, requests) = run(
        &["workflow", "run", "checkout"],
        vec![
            workflows(),
            reply(202, &json!({"runId": RUN, "status": "RUNNING"})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        received(&requests)[1].1,
        json!({"workflowId": WORKFLOW, "input": {}})
    );
}

#[test]
fn an_open_run_that_holds_the_identifier_is_said_and_nothing_is_started() {
    let (code, out, err, _) = run(
        &[
            "workflow",
            "run",
            "checkout",
            "--business-id",
            "order-1001",
            "--if-exists",
            "use-existing",
        ],
        vec![
            workflows(),
            reply(200, &json!({"runId": RUN, "status": "RUNNING"})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!(
            "run {RUN} already holds that business id: nothing was started"
        )),
        "{out}"
    );
    assert!(!out.contains("started: checkout"), "{out}");
}

#[test]
fn a_refused_start_says_why_and_never_repeats_the_identifier() {
    let (code, out, err, _) = run(
        &["workflow", "run", "checkout", "--business-id", "order-1001"],
        vec![
            workflows(),
            reply(
                409,
                &error(
                    "CONFLICT",
                    "BUSINESS_ID_IN_USE",
                    "Another open execution holds this id.",
                ),
            ),
        ],
    );
    assert_ne!(code, 0);
    let all = format!("{out}{err}");
    assert!(
        all.contains("Another open execution holds this id"),
        "{all}"
    );
    assert!(!all.contains("order-1001"), "{all}");
    assert!(!all.contains(TOKEN));
}

#[test]
fn signal_with_start_sends_the_signal_with_the_start_and_says_which_it_did() {
    let (code, out, err, requests) = run(
        &[
            "workflow",
            "signal-with-start",
            "checkout",
            "--business-id",
            "order-1001",
            "--signal",
            "order.paid",
            "--payload",
            r#"{"paid":true}"#,
            "--input",
            r#"{"item":"book"}"#,
            "--reuse",
            "allow-duplicate",
            "--key",
            "swstart-key-0001",
        ],
        vec![
            workflows(),
            reply(
                202,
                &json!({"runId": RUN, "started": true, "deliveredTo": ["wait-payment"]}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!(
            "run {RUN} was started and signalled: 1 step(s) woke"
        )),
        "{out}"
    );
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[1]),
        "POST /v1/runs/signal-with-start HTTP/1.1"
    );
    assert_eq!(
        header(&raw[1], "idempotency-key").as_deref(),
        Some("swstart-key-0001")
    );
    assert_eq!(
        request_body(&raw[1]),
        json!({
            "workflowId": WORKFLOW, "businessId": "order-1001", "input": {"item": "book"},
            "signal": {"name": "order.paid", "payload": {"paid": true}},
            "businessIdReusePolicy": "ALLOW_DUPLICATE"
        })
    );

    // an open run that was already there
    let (_, out, _, _) = run(
        &[
            "workflow",
            "signal-with-start",
            "checkout",
            "--business-id",
            "o",
            "--signal",
            "s",
        ],
        vec![
            workflows(),
            reply(
                202,
                &json!({"runId": RUN, "started": false, "deliveredTo": ["a", "b"]}),
            ),
        ],
    );
    assert!(
        out.contains(&format!(
            "run {RUN} was already open and was signalled: 2 step(s) woke"
        )),
        "{out}"
    );
}

#[test]
fn signal_with_start_that_is_refused_says_so_and_fails_and_a_payload_that_is_not_an_object_is_not_sent()
 {
    let (code, out, err, _) = run(
        &[
            "workflow",
            "signal-with-start",
            "checkout",
            "--business-id",
            "o",
            "--signal",
            "s",
        ],
        vec![
            workflows(),
            reply(
                409,
                &error(
                    "CONFLICT",
                    "SIGNAL_NOT_ACCEPTED_AT_START",
                    "The workflow's entry does not wait for this signal.",
                ),
            ),
        ],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("nothing was signalled"),
        "{out}{err}"
    );
    assert!(format!("{out}{err}").contains("does not wait for this signal"));

    let (code, out, err, requests) = run(
        &[
            "workflow",
            "signal-with-start",
            "checkout",
            "--business-id",
            "o",
            "--signal",
            "s",
            "--payload",
            "[1]",
        ],
        vec![workflows()],
    );
    assert_ne!(code, 0);
    assert!(format!("{out}{err}").contains("JSON object"), "{out}{err}");
    // nothing was asked of the platform but the list of workflows
    assert!(
        received(&requests)
            .iter()
            .all(|(line, _)| !line.contains("signal-with-start"))
    );
}

#[test]
fn update_with_start_sends_the_update_and_reports_applied_and_rejected() {
    let (code, out, err, requests) = run(
        &[
            "workflow",
            "update-with-start",
            "checkout",
            "--business-id",
            "order-1001",
            "--update",
            "balance.replace",
            "--update-key",
            "u-0001",
            "--args",
            r#"{"allowed":true}"#,
            "--if-exists",
            "fail",
            "--start-delay",
            "5",
            "--expected-revision",
            "3",
        ],
        vec![
            workflows(),
            reply(
                200,
                &json!({"runId": RUN, "started": true, "update": {"outcome": "APPLIED", "receipt": {}}}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("run {RUN} was started and the update was applied")),
        "{out}"
    );
    let calls = received(&requests);
    assert_eq!(calls[1].0, "POST /v1/runs/update-with-start HTTP/1.1");
    assert_eq!(
        calls[1].1,
        json!({
            "workflowId": WORKFLOW, "businessId": "order-1001", "input": {},
            "update": {"name": "balance.replace", "updateKey": "u-0001", "args": {"allowed": true}, "expectedStateRevision": 3},
            "businessIdConflictPolicy": "FAIL", "startDelayMs": 5_000
        })
    );

    // a business rejection is an answer: the run exists, the update did not apply, the command fails
    let (code, out, err, _) = run(
        &[
            "workflow",
            "update-with-start",
            "checkout",
            "--business-id",
            "o",
            "--update",
            "u",
            "--update-key",
            "k",
        ],
        vec![
            workflows(),
            reply(
                200,
                &json!({"runId": RUN, "started": false, "update": {"outcome": "REJECTED", "reason": "UPDATE_REJECTED"}}),
            ),
        ],
    );
    assert_eq!(code, 1, "{out}{err}");
    let all = format!("{out}{err}");
    assert!(
        all.contains(&format!(
            "run {RUN} was already open but the update was rejected: UPDATE_REJECTED"
        )),
        "{all}"
    );
}

#[test]
fn the_points_of_a_run_are_listed_in_words_with_their_sequence() {
    let (code, out, err, requests) = run(
        &["workflow", "reset-points", RUN],
        vec![Reply::ok(
            json!({"items": [
                {"sequence": 1, "kind": "RUN_STARTED", "node": null, "occurredAt": "2026-11-06T10:00:00Z"},
                {"sequence": 5, "kind": "STEP_COMPLETED", "node": "charge", "occurredAt": "2026-11-06T10:00:03Z"},
                {"sequence": 9, "kind": "LOOP_FINISHED", "node": "retry", "occurredAt": "2026-11-06T10:00:05Z"}
            ]})
            .to_string(),
        )],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("the start of the run  (2026-11-06T10:00:00Z)"),
        "{out}"
    );
    assert!(
        out.contains("5  after step charge  (2026-11-06T10:00:03Z)"),
        "{out}"
    );
    assert!(out.contains("9  after loop retry"), "{out}");
    assert!(
        out.contains(&format!(
            "sagat workflow reset-preview {RUN} --sequence <N>"
        )),
        "{out}"
    );
    assert!(
        received(&requests)[0]
            .0
            .starts_with(&format!("GET /v1/runs/{RUN}/reset-points?"))
    );

    // a run with none, and a run that cannot be reset at all
    let (code, out, _, _) = run(
        &["workflow", "reset-points", RUN],
        vec![Reply::ok(r#"{"items":[]}"#)],
    );
    assert_eq!(code, 0);
    assert!(out.contains("cannot be reset to any point"), "{out}");
    let (code, out, err, _) = run(
        &["workflow", "reset-points", RUN],
        vec![reply(
            422,
            &error(
                "VALIDATION_FAILED",
                "RESET_NOT_SUPPORTED_FOR_SOURCE",
                "Child runs cannot be reset in this version.",
            ),
        )],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("Child runs cannot be reset"),
        "{out}{err}"
    );
}

#[test]
fn a_preview_says_what_is_kept_what_runs_again_and_what_to_acknowledge() {
    let (code, out, err, requests) = run(
        &["workflow", "reset-preview", RUN, "--sequence", "5"],
        vec![Reply::ok(
            json!({
                "sequence": 5, "prefixEvents": 5, "reapply": [], "requiresAcknowledgement": true,
                "restored": [{"node": "charge", "kind": "STEP"}],
                "reexecuted": [{"node": "ship", "completedInSource": true, "markers": ["irreversible", "highRisk"]}]
            })
            .to_string(),
        )],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("a reset of run"), "{out}");
    assert!(out.contains("keep as they were: charge"), "{out}");
    assert!(out.contains("run again: ship (it completed in this run: running it again repeats what it did) [irreversible, highRisk]"), "{out}");
    assert!(out.contains("add --acknowledge"), "{out}");
    let calls = received(&requests);
    assert_eq!(
        calls[0].0,
        format!("POST /v1/runs/{RUN}/reset-previews HTTP/1.1")
    );
    assert_eq!(calls[0].1, json!({"sequence": 5}));

    // nothing to run again, nothing to acknowledge
    let (_, out, _, _) = run(
        &["workflow", "reset-preview", RUN, "--sequence", "9"],
        vec![Reply::ok(
            json!({"sequence": 9, "restored": [{"node": "charge", "kind": "STEP"}], "reexecuted": [], "reapply": [], "requiresAcknowledgement": false, "prefixEvents": 9})
                .to_string(),
        )],
    );
    assert!(out.contains("run nothing again"), "{out}");
    assert!(!out.contains("--acknowledge"), "{out}");
    // and the start keeps nothing
    let (_, out, _, _) = run(
        &["workflow", "reset-preview", RUN, "--sequence", "1"],
        vec![Reply::ok(
            json!({"sequence": 1, "restored": [], "reexecuted": [{"node": "charge", "completedInSource": true, "markers": []}], "reapply": [], "requiresAcknowledgement": true, "prefixEvents": 1})
                .to_string(),
        )],
    );
    assert!(
        out.contains("keep nothing: the new run starts from the beginning"),
        "{out}"
    );
}

#[test]
fn a_reset_sends_the_request_with_a_key_and_names_the_run_it_made() {
    let (code, out, err, requests) = run(
        &[
            "workflow",
            "reset",
            RUN,
            "--sequence",
            "5",
            "--reason",
            "wrong price list",
            "--acknowledge",
            "--drop-business-id",
            "--key",
            "reset-key-0001",
        ],
        vec![reply(
            202,
            &json!({"resetId": "r-1", "sourceRunId": RUN, "sequence": 5, "state": "CREATED", "newRunId": NEW, "reapplyReport": []}),
        )],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!(
            "reset r-1: run {NEW} was made from point 5 of run {RUN}"
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!("sagat workflow inspect {NEW}")),
        "{out}"
    );
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[0]),
        format!("POST /v1/runs/{RUN}/resets HTTP/1.1")
    );
    assert_eq!(
        header(&raw[0], "idempotency-key").as_deref(),
        Some("reset-key-0001")
    );
    assert_eq!(
        request_body(&raw[0]),
        json!({"sequence": 5, "reason": "wrong price list", "acknowledgeReexecution": true, "businessId": "DROP"})
    );

    // without the switches: not acknowledged, the identifier kept (not sent), a key of its own
    let (code, _, err, requests) = run(
        &["workflow", "reset", RUN, "--sequence", "9", "--reason", "x"],
        vec![reply(
            202,
            &json!({"resetId": "r-2", "newRunId": NEW, "state": "CREATED"}),
        )],
    );
    assert_eq!(code, 0, "{err}");
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_body(&raw[0]),
        json!({"sequence": 9, "reason": "x", "acknowledgeReexecution": false})
    );
    assert!(header(&raw[0], "idempotency-key").is_some_and(|key| key.len() >= 8));
}

#[test]
fn a_reset_that_is_refused_says_why_and_fails_without_the_token() {
    for (status, body) in [
        (
            409,
            error(
                "CONFLICT",
                "RESET_SOURCE_NOT_TERMINAL",
                "Wait for the run to end.",
            ),
        ),
        (
            422,
            error(
                "VALIDATION_FAILED",
                "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
                "Steps that completed in the source will run again; set acknowledgeReexecution.",
            ),
        ),
        (
            404,
            json!({"code": "NOT_FOUND", "message": "No such run.", "retryable": false}),
        ),
    ] {
        let (code, out, err, _) = run(
            &["workflow", "reset", RUN, "--sequence", "5", "--reason", "x"],
            vec![reply(status, &body)],
        );
        assert_eq!(code, 1, "{status}: {out}{err}");
        let all = format!("{out}{err}");
        assert!(all.contains("the run could not be reset"), "{all}");
        assert!(
            all.contains(body["message"].as_str().expect("message")),
            "{all}"
        );
        assert!(!all.contains(TOKEN));
    }
    // a failure of the platform is an error, not a refusal
    let (code, out, err, _) = run(
        &["workflow", "reset", RUN, "--sequence", "5", "--reason", "x"],
        vec![reply(
            500,
            &json!({"code": "INTERNAL", "message": "Something broke.", "retryable": false}),
        )],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(!format!("{out}{err}").contains("the run could not be reset"));
}
