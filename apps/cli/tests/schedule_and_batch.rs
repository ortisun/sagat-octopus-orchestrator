//! `sagat schedule …` and `sagat batch …` (`T018-069`, `FR-018-047`): the command line, and each command as one request
//! of the REST API with its answer in words. A mock platform, the real command line.
#![allow(clippy::expect_used, clippy::panic)]

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use sagat_cli::Env;
use sagat_cli::args::{
    ActionBusiness, BatchCommand, Command, IfExists, Reuse, ScheduleCommand, parse,
};
use serde_json::{Value, json};
use support::{Reply, request_body, request_line, server};

const NS: &str = "0194f5e2-7b1c-7000-8000-0000000000aa";
const ID: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8";
const RUN: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b9";
const TOKEN: &str = "secret-cli-token-must-never-appear";

fn argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
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

fn namespaces() -> Reply {
    Reply::ok(json!({"items": [{"id": NS, "slug": "dev", "environment": "dev"}]}).to_string())
}

fn reply(status: u16, body: &Value) -> Reply {
    Reply {
        status,
        body: body.to_string(),
    }
}

fn refusal(code: &str, reason: &str, message: &str) -> Value {
    json!({"code": code, "message": message, "retryable": false, "details": {"reason": reason}})
}

fn calls(requests: &mpsc::Receiver<Vec<u8>>) -> Vec<(String, Value)> {
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

fn key_of(request: &[u8]) -> Option<String> {
    std::str::from_utf8(request).ok()?.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("idempotency-key")
            .then(|| value.trim().to_owned())
    })
}

fn write_temp(text: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "sagat-cli-schedule-{}-{}.json",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&path, text).expect("write");
    path
}

#[test]
fn the_schedule_commands_take_the_words_and_flags_they_document() {
    let schedule = |line: &str| match parse(&argv(line)).expect(line).command {
        Command::Schedule(command) => command,
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(
        schedule("schedule list --state paused --limit 500"),
        ScheduleCommand::List {
            state: Some("PAUSED".into()),
            limit: 200
        }
    );
    assert_eq!(
        schedule(&format!("schedule get {ID}")),
        ScheduleCommand::Get { id: ID.into() }
    );
    assert_eq!(
        schedule("schedule create --definition s.json --key k-0123456789"),
        ScheduleCommand::Create {
            file: "s.json".into(),
            key: Some("k-0123456789".into()),
            business: ActionBusiness::default()
        }
    );
    assert_eq!(
        schedule(&format!(
            "schedule update {ID} --definition s.json --revision 3"
        )),
        ScheduleCommand::Update {
            id: ID.into(),
            file: "s.json".into(),
            revision: 3,
            key: None,
            business: ActionBusiness::default()
        }
    );
    assert_eq!(
        schedule(&format!("schedule delete {ID} --revision 3")),
        ScheduleCommand::Delete {
            id: ID.into(),
            revision: 3,
            key: None
        }
    );
    assert_eq!(
        schedule(&format!("schedule pause {ID} --note maintenance")),
        ScheduleCommand::Pause {
            id: ID.into(),
            note: "maintenance".into(),
            key: None
        }
    );
    assert_eq!(
        schedule(&format!("schedule resume {ID} --note done")),
        ScheduleCommand::Resume {
            id: ID.into(),
            note: "done".into(),
            key: None
        }
    );
    assert_eq!(
        schedule(&format!("schedule trigger {ID} --overlap buffer-all")),
        ScheduleCommand::Trigger {
            id: ID.into(),
            overlap: Some("BUFFER_ALL".into()),
            key: None
        }
    );
    assert_eq!(
        schedule(&format!(
            "schedule backfill {ID} --from 2026-01-01T00:00:00Z --to 2026-01-02T00:00:00Z --overlap allow-all"
        )),
        ScheduleCommand::Backfill {
            id: ID.into(),
            from: "2026-01-01T00:00:00Z".into(),
            to: "2026-01-02T00:00:00Z".into(),
            overlap: Some("ALLOW_ALL".into()),
            key: None
        }
    );
    assert_eq!(
        schedule(&format!("schedule actions {ID}")),
        ScheduleCommand::Actions {
            id: ID.into(),
            limit: 20
        }
    );
}

#[test]
fn the_batch_commands_take_the_words_and_flags_they_document() {
    let batch = |line: &str| match parse(&argv(line)).expect(line).command {
        Command::Batch(command) => command,
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(
        batch("batch preview --selector {} --max-items 5"),
        BatchCommand::Preview {
            selector: Some("{}".into()),
            business_id_prefix: None,
            max_items: Some(5)
        }
    );
    let BatchCommand::Create(created) = batch(
        "batch create --selector @s.json --action retry-step --reason because --max-items 50 --rate 2.5 --step-id charge --drop-business-id --key k-0123456789",
    ) else {
        panic!("a creation");
    };
    assert_eq!(created.selector.as_deref(), Some("@s.json"));
    assert_eq!(created.business_id_prefix, None);
    assert_eq!(created.action, "RETRY_STEP");
    assert_eq!(created.reason, "because");
    assert_eq!(created.max_items, Some(50));
    assert_eq!(created.rate.as_deref(), Some("2.5"));
    assert_eq!(created.step_id.as_deref(), Some("charge"));
    assert!(created.drop_business_id);
    assert_eq!(created.key.as_deref(), Some("k-0123456789"));
    assert_eq!(
        batch(&format!("batch get {ID}")),
        BatchCommand::Get { id: ID.into() }
    );
    assert_eq!(
        batch("batch list --state running"),
        BatchCommand::List {
            state: Some("RUNNING".into()),
            limit: 20
        }
    );
    assert_eq!(
        batch(&format!("batch items {ID} --state failed --limit 10")),
        BatchCommand::Items {
            id: ID.into(),
            state: Some("FAILED".into()),
            limit: 10
        }
    );
    assert_eq!(
        batch(&format!("batch cancel {ID} --reason because")),
        BatchCommand::Cancel {
            id: ID.into(),
            reason: "because".into(),
            key: None
        }
    );
    assert_eq!(
        batch(&format!(
            "batch cancel {ID} --reason stop --key k-0123456789"
        )),
        BatchCommand::Cancel {
            id: ID.into(),
            reason: "stop".into(),
            key: Some("k-0123456789".into())
        }
    );
}

#[test]
fn what_the_schedule_and_batch_commands_get_wrong_is_named() {
    for (line, mentions) in [
        ("schedule", "schedule needs a command"),
        (
            "schedule frobnicate",
            "'schedule frobnicate' is not a command",
        ),
        ("schedule get not-an-id", "the schedule is given by its id"),
        ("schedule get ../x", "the schedule is given by its id"),
        (
            "schedule list --state sleeping",
            "--state is one of active, paused",
        ),
        ("schedule create", "--definition is required"),
        ("schedule list extra", "unexpected argument 'extra'"),
        (
            &format!("schedule update {ID} --definition s.json"),
            "--revision is required",
        ),
        (
            &format!("schedule update {ID} --revision 1"),
            "--definition is required",
        ),
        (
            &format!("schedule delete {ID} --revision 0"),
            "--revision is the revision you saw",
        ),
        (&format!("schedule pause {ID}"), "--note is required"),
        (
            &format!("schedule trigger {ID} --overlap never"),
            "--overlap is skip, buffer-one",
        ),
        (
            &format!("schedule backfill {ID} --from a"),
            "--to is required",
        ),
        (
            &format!("schedule backfill {ID} --to a"),
            "--from is required",
        ),
        ("batch", "batch needs a command"),
        ("batch frobnicate", "'batch frobnicate' is not a command"),
        ("batch preview", "--selector is required"),
        (
            "batch create --selector {} --reason r",
            "--action is required",
        ),
        (
            "batch create --selector {} --action explode --reason r",
            "--action is one of cancel, signal",
        ),
        (
            "batch create --selector {} --action cancel",
            "--reason is required",
        ),
        (
            "batch create --selector {} --action cancel --reason r --max-items 0",
            "--max-items is a number from 1",
        ),
        ("batch get not-an-id", "the job is given by its id"),
        (&format!("batch cancel {ID}"), "--reason is required"),
        (
            "batch list --state sleeping",
            "--state is one of selecting, running",
        ),
        (
            &format!("batch items {ID} --state asleep"),
            "--state is one of pending",
        ),
    ] {
        let error = parse(&argv(line)).expect_err(line);
        assert!(error.0.contains(mentions), "{line}: {}", error.0);
    }
    for command in [
        "schedule create",
        "schedule trigger",
        "schedule backfill",
        "batch create",
        "batch preview",
        "batch items",
    ] {
        assert!(
            sagat_cli::args::USAGE.contains(command),
            "the usage does not mention {command}"
        );
    }
}

#[test]
fn a_schedule_is_created_from_a_file_with_a_key_and_the_platforms_words() {
    let file = write_temp(
        r#"{"name":"nightly","spec":{"intervals":[{"everyMs":3600000}]},"action":{"workflowId":"w","input":{}}}"#,
    );
    let (code, out, err, requests) = run(
        &[
            "schedule",
            "create",
            "--definition",
            file.to_str().expect("path"),
            "--key",
            "schedule-key-01",
        ],
        vec![
            namespaces(),
            reply(
                201,
                &json!({"scheduleId": ID, "name": "nightly", "revision": 1, "state": "ACTIVE"}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("schedule {ID} created: nightly (revision 1)")),
        "{out}"
    );
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[0]),
        "GET /v1/namespaces?limit=50 HTTP/1.1"
    );
    assert_eq!(
        request_line(&raw[1]),
        format!("POST /v1/namespaces/{NS}/schedules HTTP/1.1")
    );
    assert_eq!(key_of(&raw[1]).as_deref(), Some("schedule-key-01"));
    assert_eq!(request_body(&raw[1])["name"], "nightly");
    assert!(!out.contains(TOKEN));

    // a document that is not JSON, or not an object, is refused before anything is asked of the platform
    for text in ["not json", "[1]"] {
        let bad = write_temp(text);
        let (code, out, err, requests) = run(
            &[
                "schedule",
                "create",
                "--definition",
                bad.to_str().expect("path"),
            ],
            // a reply is ready for a POST that must not be sent: if it is, it is recorded
            vec![namespaces(), reply(201, &json!({"scheduleId": ID}))],
        );
        assert_ne!(code, 0, "{out}{err}");
        assert!(
            calls(&requests)
                .iter()
                .all(|(line, _)| !line.starts_with("POST")),
            "{text}"
        );
    }
}

#[test]
fn a_schedule_that_is_refused_says_why_and_fails() {
    let file = write_temp(r#"{"name":"x"}"#);
    for (status, body) in [
        (
            422,
            refusal(
                "VALIDATION_FAILED",
                "SCHEDULE_SPEC_INVALID",
                "The schedule spec is not valid.",
            ),
        ),
        (
            409,
            refusal("CONFLICT", "REVISION_CONFLICT", "The revision is stale."),
        ),
        (
            429,
            refusal(
                "QUOTA_EXCEEDED",
                "SCHEDULE_LIMIT",
                "A namespace ceiling is reached.",
            ),
        ),
    ] {
        let (code, out, err, _) = run(
            &[
                "schedule",
                "create",
                "--definition",
                file.to_str().expect("path"),
            ],
            vec![namespaces(), reply(status, &body)],
        );
        assert_eq!(code, 1, "{status}: {out}{err}");
        let all = format!("{out}{err}");
        assert!(all.contains("the schedule was not created"), "{all}");
        assert!(
            all.contains(body["message"].as_str().expect("message")),
            "{all}"
        );
        assert!(!all.contains(TOKEN));
    }
}

#[test]
fn a_schedule_a_cron_trigger_owns_is_refused_in_words_before_the_handover_and_says_whose_it_is() {
    let trigger = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7c0";
    for (verb, extra) in [
        ("pause", vec!["--note", "maintenance"]),
        ("resume", vec!["--note", "maintenance"]),
        ("trigger", vec![]),
    ] {
        let mut arguments = vec!["schedule", verb, ID];
        arguments.extend(extra);
        let (code, out, err, _) = run(
            &arguments,
            vec![
                namespaces(),
                reply(
                    409,
                    &json!({
                        "code": "CONFLICT",
                        "message": "A cron trigger owns this schedule.",
                        "remediation": "The legacy cron pass still runs; the operator completes the handover once every engine is upgraded.",
                        "retryable": false,
                        "details": {"reason": "SCHEDULE_TRIGGER_HANDOVER_PENDING"}
                    }),
                ),
            ],
        );
        assert_eq!(code, 1, "{verb}: {out}{err}");
        let all = format!("{out}{err}");
        assert!(
            all.contains("SCHEDULE_TRIGGER_HANDOVER_PENDING"),
            "{verb}: {all}"
        );
        assert!(
            all.contains("the operator completes the handover"),
            "{verb}: {all}"
        );
        assert!(!all.contains(TOKEN));
    }
    let (code, out, _, _) = run(
        &["schedule", "get", ID],
        vec![
            namespaces(),
            Reply::ok(
                json!({
                    "scheduleId": ID, "name": "trigger-5d0c", "state": "ACTIVE", "revision": 1,
                    "origin": "TRIGGER", "triggerId": trigger,
                    "nextActions": [], "recentActions": [], "openRuns": [], "bufferedCount": 0
                })
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!("owned by the cron trigger {trigger}")),
        "{out}"
    );
    assert!(
        out.contains("once an operator has handed the cron triggers over"),
        "{out}"
    );
    for (controllable, words) in [
        (true, "are open (the cron triggers were handed over)"),
        (
            false,
            "are refused until an operator has handed the cron triggers over",
        ),
    ] {
        let (code, out, _, _) = run(
            &["schedule", "get", ID],
            vec![
                namespaces(),
                Reply::ok(
                    json!({
                        "scheduleId": ID, "name": "trigger-5d0c", "state": "ACTIVE", "revision": 1,
                        "origin": "TRIGGER", "triggerId": trigger, "controllable": controllable,
                        "nextActions": [], "recentActions": [], "openRuns": [], "bufferedCount": 0
                    })
                    .to_string(),
                ),
            ],
        );
        assert_eq!(code, 0, "{out}");
        assert!(out.contains(words), "{controllable}: {out}");
    }
}

#[test]
fn a_schedule_is_updated_deleted_paused_resumed_and_triggered_with_the_requests_the_contract_says()
{
    let file = write_temp(r#"{"name":"nightly"}"#);
    let (code, out, err, requests) = run(
        &[
            "schedule",
            "update",
            ID,
            "--definition",
            file.to_str().expect("path"),
            "--revision",
            "3",
            "--key",
            "update-key-001",
        ],
        vec![
            namespaces(),
            reply(200, &json!({"scheduleId": ID, "revision": 4})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("schedule {ID} updated: revision 4")),
        "{out}"
    );
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[1]),
        format!("PUT /v1/namespaces/{NS}/schedules/{ID} HTTP/1.1")
    );
    assert_eq!(request_body(&raw[1])["expectedRevision"], 3);
    assert_eq!(key_of(&raw[1]).as_deref(), Some("update-key-001"));

    let (code, out, err, requests) = run(
        &["schedule", "delete", ID, "--revision", "4"],
        vec![
            namespaces(),
            Reply {
                status: 204,
                body: String::new(),
            },
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains(&format!("schedule {ID} deleted")), "{out}");
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[1]),
        format!("DELETE /v1/namespaces/{NS}/schedules/{ID}?expectedRevision=4 HTTP/1.1")
    );
    assert!(key_of(&raw[1]).is_some_and(|key| key.len() >= 8));

    for (verb, state) in [("pause", "PAUSED"), ("resume", "ACTIVE")] {
        let (code, out, err, requests) = run(
            &["schedule", verb, ID, "--note", "maintenance window"],
            vec![
                namespaces(),
                reply(200, &json!({"scheduleId": ID, "state": state})),
            ],
        );
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains(&format!("schedule {ID} is now {state}")),
            "{out}"
        );
        let c = calls(&requests);
        assert_eq!(
            c[1].0,
            format!("POST /v1/namespaces/{NS}/schedules/{ID}/{verb} HTTP/1.1")
        );
        assert_eq!(c[1].1, json!({"note": "maintenance window"}));
    }

    let (code, out, err, requests) = run(
        &["schedule", "trigger", ID, "--overlap", "skip"],
        vec![
            namespaces(),
            reply(
                202,
                &json!({"kind": "TRIGGERED", "outcome": "STARTED", "runId": RUN}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("schedule {ID} was triggered: STARTED, run {RUN}")),
        "{out}"
    );
    let c = calls(&requests);
    assert_eq!(
        c[1].0,
        format!("POST /v1/namespaces/{NS}/schedules/{ID}/trigger HTTP/1.1")
    );
    assert_eq!(c[1].1, json!({"overlap": "SKIP"}));
    // and without an overlap the body names none
    let (_, _, _, requests) = run(
        &["schedule", "trigger", ID],
        vec![namespaces(), reply(202, &json!({"outcome": "BUFFERED"}))],
    );
    assert_eq!(calls(&requests)[1].1, json!({}));
}

#[test]
fn a_backfill_names_the_range_and_the_policy() {
    let (code, out, err, requests) = run(
        &[
            "schedule",
            "backfill",
            ID,
            "--from",
            "2026-01-01T00:00:00Z",
            "--to",
            "2026-01-02T00:00:00Z",
            "--overlap",
            "buffer-one",
        ],
        vec![
            namespaces(),
            reply(
                202,
                &json!({"backfillId": "b-1", "state": "PENDING", "start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!(
            "backfill b-1 of schedule {ID} is PENDING: 2026-01-01T00:00:00Z to 2026-01-02T00:00:00Z"
        )),
        "{out}"
    );
    let c = calls(&requests);
    assert_eq!(
        c[1].0,
        format!("POST /v1/namespaces/{NS}/schedules/{ID}/backfills HTTP/1.1")
    );
    assert_eq!(
        c[1].1,
        json!({"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z", "overlap": "BUFFER_ONE"})
    );
}

#[test]
fn schedules_are_listed_described_and_their_actions_shown_in_words() {
    let (code, out, err, requests) = run(
        &["schedule", "list", "--state", "active"],
        vec![
            namespaces(),
            Reply::ok(
                json!({"items": [
                    {"scheduleId": ID, "name": "nightly", "state": "ACTIVE", "revision": 2, "origin": "API"},
                    {"scheduleId": RUN, "name": "cron-trigger", "state": "ACTIVE", "revision": 1, "origin": "TRIGGER"}
                ]})
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("nightly") && out.contains("revision 2") && out.contains("TRIGGER"),
        "{out}"
    );
    assert!(calls(&requests)[1].0.contains("state=ACTIVE"));
    let (_, out, _, _) = run(
        &["schedule", "list"],
        vec![namespaces(), Reply::ok(r#"{"items":[]}"#)],
    );
    assert!(out.contains("dev has no schedules"), "{out}");

    let (code, out, err, requests) = run(
        &["schedule", "get", ID],
        vec![
            namespaces(),
            Reply::ok(
                json!({
                    "scheduleId": ID, "name": "nightly", "state": "ACTIVE", "revision": 2,
                    "nextActions": [{"nominalTime": "t1", "actualTime": "2026-11-07T00:00:00Z"}],
                    "recentActions": [{"nominalTime": "2026-11-06T00:00:00Z", "outcome": "STARTED"}],
                    "openRuns": [RUN], "bufferedCount": 1, "counters": {}
                })
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("schedule {ID}  nightly  ACTIVE  revision 2")),
        "{out}"
    );
    assert!(out.contains("next: 2026-11-07T00:00:00Z"), "{out}");
    assert!(
        out.contains("recent: 2026-11-06T00:00:00Z  STARTED"),
        "{out}"
    );
    assert!(out.contains("1 open run(s), 1 buffered"), "{out}");
    assert!(calls(&requests)[1].0.contains("nextCount=5"));

    let (code, out, err, _) = run(
        &["schedule", "actions", ID],
        vec![
            namespaces(),
            Reply::ok(
                json!({"items": [
                    {"nominalTime": "2026-11-06T00:00:00Z", "kind": "SCHEDULED", "outcome": "STARTED", "runId": RUN},
                    {"nominalTime": "2026-11-05T00:00:00Z", "kind": "BACKFILL", "outcome": "SKIPPED_OVERLAP"}
                ]})
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("2026-11-06T00:00:00Z  SCHEDULED") && out.contains(RUN),
        "{out}"
    );
    assert!(
        out.contains("SKIPPED_OVERLAP") && out.contains(" -"),
        "{out}"
    );
}

#[test]
fn a_selection_is_previewed_and_a_job_created_with_what_the_contract_names() {
    let (code, out, err, requests) = run(
        &[
            "batch",
            "preview",
            "--selector",
            r#"{"runIds":["a"]}"#,
            "--max-items",
            "10",
        ],
        vec![
            namespaces(),
            Reply::ok(json!({"matched": 3, "capped": true, "sample": [RUN]}).to_string()),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("3 run(s) match (more than the limit"), "{out}");
    assert!(out.contains(RUN), "{out}");
    let c = calls(&requests);
    assert_eq!(
        c[1].0,
        format!("POST /v1/namespaces/{NS}/batch-operations/preview HTTP/1.1")
    );
    assert_eq!(
        c[1].1,
        json!({"selector": {"runIds": ["a"]}, "maxItems": 10})
    );

    let (code, out, err, requests) = run(
        &[
            "batch",
            "create",
            "--selector",
            r#"{"filter":{"status":["FAILED"]}}"#,
            "--action",
            "signal",
            "--reason",
            "release the held orders",
            "--max-items",
            "100",
            "--rate",
            "5",
            "--signal",
            "order.release",
            "--payload",
            r#"{"ok":true}"#,
            "--key",
            "batch-key-0001",
        ],
        vec![
            namespaces(),
            reply(
                202,
                &json!({"batchId": ID, "action": "SIGNAL", "state": "SELECTING"}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("batch job {ID} created: SIGNAL (SELECTING)")),
        "{out}"
    );
    assert!(out.contains(&format!("sagat batch get {ID}")), "{out}");
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[1]),
        format!("POST /v1/namespaces/{NS}/batch-operations HTTP/1.1")
    );
    assert_eq!(key_of(&raw[1]).as_deref(), Some("batch-key-0001"));
    assert_eq!(
        request_body(&raw[1]),
        json!({
            "selector": {"filter": {"status": ["FAILED"]}}, "action": "SIGNAL", "reason": "release the held orders",
            "maxItems": 100, "itemsPerSecond": 5.0, "signal": {"name": "order.release", "payload": {"ok": true}}
        })
    );

    // a rerun that drops the business identifier, and a retry of a step
    let (_, _, _, requests) = run(
        &[
            "batch",
            "create",
            "--selector",
            "{}",
            "--action",
            "rerun",
            "--reason",
            "run them again",
            "--drop-business-id",
        ],
        vec![
            namespaces(),
            reply(
                202,
                &json!({"batchId": ID, "action": "RERUN", "state": "SELECTING"}),
            ),
        ],
    );
    assert_eq!(calls(&requests)[1].1["businessId"], "DROP");
    let (_, _, _, requests) = run(
        &[
            "batch",
            "create",
            "--selector",
            "{}",
            "--action",
            "retry-step",
            "--reason",
            "retry the charge",
            "--step-id",
            "charge",
        ],
        vec![
            namespaces(),
            reply(
                202,
                &json!({"batchId": ID, "action": "RETRY_STEP", "state": "SELECTING"}),
            ),
        ],
    );
    let created = calls(&requests);
    assert_eq!(created[1].1["action"], "RETRY_STEP");
    assert_eq!(created[1].1["stepId"], "charge");
}

#[test]
fn what_is_wrong_with_a_job_is_said_before_anything_is_sent_or_by_the_platform() {
    // a payload that is not an object, a rate that is not a number, a selector that is not JSON
    for arguments in [
        vec![
            "batch",
            "create",
            "--selector",
            "{}",
            "--action",
            "signal",
            "--reason",
            "r",
            "--signal",
            "s",
            "--payload",
            "[1]",
        ],
        vec![
            "batch",
            "create",
            "--selector",
            "{}",
            "--action",
            "cancel",
            "--reason",
            "r",
            "--rate",
            "fast",
        ],
        vec![
            "batch",
            "create",
            "--selector",
            "not json",
            "--action",
            "cancel",
            "--reason",
            "r",
        ],
    ] {
        // a reply is ready for a POST that must not be sent: if it is, it is recorded
        let (code, out, err, requests) = run(
            &arguments,
            vec![
                namespaces(),
                reply(
                    202,
                    &json!({"batchId": ID, "action": "CANCEL", "state": "SELECTING"}),
                ),
            ],
        );
        assert_ne!(code, 0, "{arguments:?}: {out}{err}");
        assert!(
            calls(&requests)
                .iter()
                .all(|(line, _)| !line.starts_with("POST")),
            "{arguments:?}"
        );
    }
    // the platform's refusal is its own words
    let (code, out, err, _) = run(
        &[
            "batch",
            "create",
            "--selector",
            "{}",
            "--action",
            "update",
            "--reason",
            "update them please",
        ],
        vec![
            namespaces(),
            reply(
                422,
                &refusal(
                    "VALIDATION_FAILED",
                    "BATCH_ACTION_UNAVAILABLE",
                    "Enable the feature or choose another action.",
                ),
            ),
        ],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("the batch job was not created"),
        "{out}{err}"
    );
    assert!(
        format!("{out}{err}").contains("choose another action"),
        "{out}{err}"
    );
    // someone who may not run a job is told so, not shown an error of the tool
    let (code, out, err, _) = run(
        &[
            "batch",
            "create",
            "--selector",
            "{}",
            "--action",
            "cancel",
            "--reason",
            "stop them all",
        ],
        vec![
            namespaces(),
            reply(
                403,
                &json!({"code": "FORBIDDEN", "message": "You may not run batch operations.", "retryable": false}),
            ),
        ],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("the batch job was not created"),
        "{out}{err}"
    );
    assert!(
        format!("{out}{err}").contains("You may not run batch operations"),
        "{out}{err}"
    );
}

#[test]
fn a_job_is_read_listed_its_items_shown_and_cancelled() {
    let job = json!({
        "batchId": ID, "action": "CANCEL", "state": "RUNNING",
        "counts": {"selected": 10, "succeeded": 4, "skipped": 1, "failed": 2, "cancelled": 0, "deferred": 3}
    });
    let (code, out, err, requests) = run(
        &["batch", "get", ID],
        vec![namespaces(), Reply::ok(job.to_string())],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("batch job {ID}  CANCEL  RUNNING")),
        "{out}"
    );
    assert!(
        out.contains("10 selected: 4 succeeded, 1 skipped, 2 failed, 0 cancelled, 3 deferred"),
        "{out}"
    );
    assert_eq!(
        calls(&requests)[1].0,
        format!("GET /v1/namespaces/{NS}/batch-operations/{ID} HTTP/1.1")
    );
    let failed = json!({"batchId": ID, "action": "CANCEL", "state": "FAILED", "failureReason": "SELECTION_FAILED", "counts": {}});
    let (_, out, _, _) = run(
        &["batch", "get", ID],
        vec![namespaces(), Reply::ok(failed.to_string())],
    );
    assert!(out.contains("failed: SELECTION_FAILED"), "{out}");

    let (code, out, err, requests) = run(
        &["batch", "list", "--state", "running"],
        vec![namespaces(), Reply::ok(json!({"items": [job]}).to_string())],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(ID) && out.contains("RUNNING") && out.contains("10 selected"),
        "{out}"
    );
    assert!(calls(&requests)[1].0.contains("state=RUNNING"));
    let (_, out, _, _) = run(
        &["batch", "list"],
        vec![namespaces(), Reply::ok(r#"{"items":[]}"#)],
    );
    assert!(out.contains("there are no batch jobs"), "{out}");

    let (code, out, err, requests) = run(
        &["batch", "items", ID, "--state", "failed"],
        vec![
            namespaces(),
            Reply::ok(
                json!({"items": [
                    {"ordinal": 0, "runId": RUN, "state": "FAILED", "errorCode": "CONFLICT"},
                    {"ordinal": 1, "runId": ID, "state": "SKIPPED", "reason": "ALREADY_TERMINAL"}
                ]})
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(RUN) && out.contains("CONFLICT") && out.contains("ALREADY_TERMINAL"),
        "{out}"
    );
    let asked = calls(&requests);
    assert!(
        asked[1]
            .0
            .contains(&format!("batch-operations/{ID}/items?"))
    );
    assert!(asked[1].0.contains("state=FAILED"), "{}", asked[1].0);
    assert!(asked[1].0.contains("limit=50"), "{}", asked[1].0);
    let (_, out, _, _) = run(
        &["batch", "items", ID],
        vec![namespaces(), Reply::ok(r#"{"items":[]}"#)],
    );
    assert!(out.contains("has no items"), "{out}");

    let (code, out, err, requests) = run(
        &["batch", "cancel", ID, "--reason", "stop now please"],
        vec![
            namespaces(),
            reply(202, &json!({"batchId": ID, "state": "CANCELLING"})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains(&format!("batch job {ID} is CANCELLING")),
        "{out}"
    );
    let raw: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        request_line(&raw[1]),
        format!("POST /v1/namespaces/{NS}/batch-operations/{ID}/cancel HTTP/1.1")
    );
    assert!(key_of(&raw[1]).is_some_and(|key| key.len() >= 8));
    // the contract (`cancelBatchOperation`) requires a JSON body with the reason, and the platform refuses a POST
    // that says no content type (400 "content type must be application/json"): a request without them never cancelled a job
    assert_eq!(request_body(&raw[1]), json!({"reason": "stop now please"}));
    assert!(
        String::from_utf8_lossy(&raw[1])
            .to_ascii_lowercase()
            .contains("content-type: application/json"),
        "the cancel of a job says its content type"
    );
    // a job that already ended is refused, and said
    let (code, out, err, _) = run(
        &["batch", "cancel", ID, "--reason", "stop now please"],
        vec![
            namespaces(),
            reply(
                409,
                &refusal(
                    "CONFLICT",
                    "BATCH_NOT_CANCELLABLE",
                    "The job already ended.",
                ),
            ),
        ],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("The job already ended"),
        "{out}{err}"
    );
}

#[test]
fn a_namespace_the_tenant_does_not_have_is_said_and_nothing_else_is_asked() {
    let (code, out, err, requests) = run(
        &["batch", "list"],
        vec![Reply::ok(
            r#"{"items":[{"id":"x","slug":"other","environment":"dev"}]}"#,
        )],
    );
    assert_ne!(code, 0);
    assert!(
        format!("{out}{err}").contains("the tenant has no namespace 'dev'"),
        "{out}{err}"
    );
    assert_eq!(calls(&requests).len(), 1);
}
// --- the business-id prefix of a schedule's action and of a batch selector (FR-018-016, G2027) --------------------------

#[test]
fn the_flags_for_the_business_id_of_an_action_and_of_a_selector_are_taken_and_checked_before_anything_is_sent()
 {
    let schedule = |line: &str| match parse(&argv(line)).expect(line).command {
        Command::Schedule(command) => command,
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(
        schedule(
            "schedule create --definition s.json --business-id-prefix order --if-exists use-existing --reuse reject-duplicate"
        ),
        ScheduleCommand::Create {
            file: "s.json".into(),
            key: None,
            business: ActionBusiness {
                prefix: Some("order".into()),
                if_exists: Some(IfExists::UseExisting),
                reuse: Some(Reuse::RejectDuplicate)
            }
        }
    );
    assert_eq!(
        schedule(&format!(
            "schedule update {ID} --definition s.json --revision 2 --business-id-prefix o"
        )),
        ScheduleCommand::Update {
            id: ID.into(),
            file: "s.json".into(),
            revision: 2,
            key: None,
            business: ActionBusiness {
                prefix: Some("o".into()),
                if_exists: None,
                reuse: None
            }
        }
    );
    let long = "o".repeat(161);
    for (line, mentions) in [
        (
            format!("schedule create --definition s.json --business-id-prefix {long}"),
            "--business-id-prefix is 1 to 160 visible ASCII characters",
        ),
        (
            "batch preview --max-items 5".to_owned(),
            "--selector is required (or --business-id-prefix",
        ),
        (
            format!("batch preview --business-id-prefix {}", "o".repeat(201)),
            "--business-id-prefix is 1 to 200 visible ASCII characters",
        ),
    ] {
        let error = parse(&argv(&line)).expect_err(&line);
        let said = error.0.clone();
        assert!(said.contains(mentions), "{line} -> {said}");
        assert!(
            !said.contains(&long),
            "the typed text is never echoed: {said}"
        );
    }
    // a prefix with a character an id cannot hold is refused without the text
    let bad = vec![
        "schedule".to_owned(),
        "create".to_owned(),
        "--definition".to_owned(),
        "s.json".to_owned(),
        "--business-id-prefix".to_owned(),
        "has space canary-text".to_owned(),
    ];
    let said = parse(&bad).expect_err("a space").0;
    assert!(
        said.contains("visible ASCII") && !said.contains("canary-text"),
        "{said}"
    );

    let BatchCommand::Create(created) = (match parse(&argv(
        "batch create --business-id-prefix lot- --action cancel --reason because",
    ))
    .expect("a creation")
    .command
    {
        Command::Batch(command) => command,
        other => panic!("{other:?}"),
    }) else {
        panic!("a creation");
    };
    assert_eq!(created.selector, None);
    assert_eq!(created.business_id_prefix.as_deref(), Some("lot-"));
}

#[test]
fn a_schedule_created_or_updated_with_a_prefix_sends_it_in_the_action_over_what_the_file_says() {
    let file = write_temp(
        r#"{"name":"orders","spec":{"intervals":[{"everyMs":3600000}]},"action":{"workflowId":"w","input":{},"businessIdPrefix":"from-the-file","businessIdConflictPolicy":"FAIL"}}"#,
    );
    let (code, _, err, requests) = run(
        &[
            "schedule",
            "create",
            "--definition",
            file.to_str().expect("path"),
            "--business-id-prefix",
            "order",
            "--if-exists",
            "use-existing",
            "--reuse",
            "allow-duplicate-failed-only",
        ],
        vec![
            namespaces(),
            reply(
                201,
                &json!({"scheduleId": ID, "name": "orders", "revision": 1, "state": "ACTIVE"}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    let c = calls(&requests);
    let action = &c[1].1["action"];
    assert_eq!(
        action["businessIdPrefix"], "order",
        "the flag goes over the file"
    );
    assert_eq!(action["businessIdConflictPolicy"], "USE_EXISTING");
    assert_eq!(
        action["businessIdReusePolicy"],
        "ALLOW_DUPLICATE_FAILED_ONLY"
    );
    assert_eq!(
        action["workflowId"], "w",
        "the rest of the action is the file's"
    );

    // without the flags the document is sent as it is
    let (code, _, err, requests) = run(
        &[
            "schedule",
            "update",
            ID,
            "--definition",
            file.to_str().expect("path"),
            "--revision",
            "1",
        ],
        vec![
            namespaces(),
            reply(200, &json!({"scheduleId": ID, "revision": 2})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    let c = calls(&requests);
    assert_eq!(c[1].1["action"]["businessIdPrefix"], "from-the-file");
    assert_eq!(c[1].1["expectedRevision"], 1);

    // and an update with a prefix puts it in the action too
    let (code, _, err, requests) = run(
        &[
            "schedule",
            "update",
            ID,
            "--definition",
            file.to_str().expect("path"),
            "--revision",
            "1",
            "--business-id-prefix",
            "new",
        ],
        vec![
            namespaces(),
            reply(200, &json!({"scheduleId": ID, "revision": 2})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(calls(&requests)[1].1["action"]["businessIdPrefix"], "new");

    // a document with no action has nowhere to put it, and nothing is sent
    let bare = write_temp(r#"{"name":"x"}"#);
    let (code, _, err, requests) = run(
        &[
            "schedule",
            "create",
            "--definition",
            bare.to_str().expect("path"),
            "--business-id-prefix",
            "order",
        ],
        vec![namespaces(), reply(201, &json!({"scheduleId": ID}))],
    );
    assert_ne!(code, 0);
    assert!(err.contains("no action"), "{err}");
    assert!(
        calls(&requests)
            .iter()
            .all(|(line, _)| !line.starts_with("POST"))
    );
}

#[test]
fn a_policy_without_a_prefix_goes_with_the_prefix_of_the_file_and_means_nothing_without_one() {
    // the file names the prefix: the policy flags alone change its policies (found by the independent review of this slice)
    let named = write_temp(
        r#"{"name":"orders","spec":{"intervals":[{"everyMs":3600000}]},"action":{"workflowId":"w","input":{},"businessIdPrefix":"order"}}"#,
    );
    let (code, _, err, requests) = run(
        &[
            "schedule",
            "create",
            "--definition",
            named.to_str().expect("path"),
            "--if-exists",
            "use-existing",
        ],
        vec![
            namespaces(),
            reply(
                201,
                &json!({"scheduleId": ID, "name": "orders", "revision": 1}),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    let action = &calls(&requests)[1].1["action"];
    assert_eq!(action["businessIdPrefix"], "order", "the file's");
    assert_eq!(
        action["businessIdConflictPolicy"], "USE_EXISTING",
        "the flag's"
    );

    // no prefix anywhere: refused in words before anything is sent
    let bare = write_temp(
        r#"{"name":"x","spec":{"intervals":[{"everyMs":3600000}]},"action":{"workflowId":"w","input":{}}}"#,
    );
    let (code, _, err, requests) = run(
        &[
            "schedule",
            "create",
            "--definition",
            bare.to_str().expect("path"),
            "--reuse",
            "reject-duplicate",
        ],
        vec![namespaces(), reply(201, &json!({"scheduleId": ID}))],
    );
    assert_ne!(code, 0);
    assert!(err.contains("go with a business id prefix"), "{err}");
    assert!(
        calls(&requests)
            .iter()
            .all(|(line, _)| !line.starts_with("POST"))
    );
}

#[test]
fn a_schedule_with_a_prefix_says_so_and_an_action_that_found_its_id_held_says_why() {
    let (code, out, err, _) = run(
        &["schedule", "get", ID],
        vec![
            namespaces(),
            Reply::ok(
                json!({
                    "scheduleId": ID, "name": "orders", "state": "ACTIVE", "revision": 1,
                    "action": {"workflowId": "w", "businessIdPrefix": "order",
                               "businessIdConflictPolicy": "USE_EXISTING", "businessIdReusePolicy": "REJECT_DUPLICATE"},
                    "nextActions": [], "recentActions": [], "openRuns": [], "bufferedCount": 0, "counters": {}
                })
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("business id: order-<time of the action>")
            && out.contains("held by an open run: USE_EXISTING")
            && out.contains("remembered by a closed one: REJECT_DUPLICATE"),
        "{out}"
    );
    // a schedule without one says nothing of it
    let (_, out, _, _) = run(
        &["schedule", "get", ID],
        vec![
            namespaces(),
            Reply::ok(
                json!({"scheduleId": ID, "name": "plain", "state": "ACTIVE", "revision": 1,
                       "action": {"workflowId": "w"}, "nextActions": [], "recentActions": [],
                       "openRuns": [], "bufferedCount": 0, "counters": {}})
                .to_string(),
            ),
        ],
    );
    assert!(!out.contains("business id"), "{out}");

    let (code, out, err, _) = run(
        &["schedule", "actions", ID],
        vec![
            namespaces(),
            Reply::ok(
                json!({"items": [
                    {"nominalTime": "2026-11-06T13:00:00Z", "kind": "SCHEDULED", "outcome": "BUSINESS_ID_CONFLICT",
                     "reason": "BUSINESS_ID_IN_USE"},
                    {"nominalTime": "2026-11-06T12:00:00Z", "kind": "SCHEDULED", "outcome": "ALREADY_ACTED",
                     "reason": "BUSINESS_ID_IN_USE", "runId": RUN},
                    {"nominalTime": "2026-11-06T11:00:00Z", "kind": "SCHEDULED", "outcome": "STARTED", "runId": RUN}
                ]})
                .to_string(),
            ),
        ],
    );
    assert_eq!(code, 0, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].contains("BUSINESS_ID_CONFLICT") && lines[0].ends_with("BUSINESS_ID_IN_USE"),
        "{out}"
    );
    assert!(
        lines[1].contains(RUN) && lines[1].ends_with("BUSINESS_ID_IN_USE"),
        "{out}"
    );
    assert!(
        lines[2].ends_with(RUN),
        "a started action has no reason to give: {out}"
    );
}

#[test]
fn a_selection_by_the_prefix_of_a_business_id_is_previewed_and_a_job_made_from_it() {
    // the prefix alone is a filter of its own
    let (code, out, err, requests) = run(
        &["batch", "preview", "--business-id-prefix", "lot-"],
        vec![
            namespaces(),
            Reply::ok(json!({"matched": 2, "capped": false, "sample": [RUN]}).to_string()),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("2 run(s) match"), "{out}");
    assert_eq!(
        calls(&requests)[1].1,
        json!({"selector": {"filter": {"businessIdPrefix": "lot-"}}})
    );

    // it joins the filter of a selector that has one
    let (code, _, err, requests) = run(
        &[
            "batch",
            "create",
            "--selector",
            r#"{"filter":{"status":["RUNNING"]}}"#,
            "--business-id-prefix",
            "lot-",
            "--action",
            "cancel",
            "--reason",
            "cancel the runs of the lot",
        ],
        vec![
            namespaces(),
            reply(202, &json!({"batchId": ID, "state": "SELECTING"})),
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        calls(&requests)[1].1["selector"],
        json!({"filter": {"status": ["RUNNING"], "businessIdPrefix": "lot-"}})
    );

    // a selector with no filter to join it to, or with a prefix already, is refused before anything is sent
    for selector in [
        r#"{"runIds":["a"]}"#,
        r#"{"filter":{"businessIdPrefix":"x"}}"#,
    ] {
        let (code, out, err, requests) = run(
            &[
                "batch",
                "preview",
                "--selector",
                selector,
                "--business-id-prefix",
                "lot-",
            ],
            vec![namespaces(), reply(200, &json!({"matched": 0}))],
        );
        assert_ne!(code, 0, "{selector}: {out}{err}");
        assert!(
            calls(&requests)
                .iter()
                .all(|(line, _)| !line.starts_with("POST")),
            "{selector}"
        );
    }
}
