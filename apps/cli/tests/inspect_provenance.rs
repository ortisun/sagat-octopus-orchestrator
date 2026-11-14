//! `sagat workflow inspect` says, in words, which schedule started a run and which run a reset made it from (`G2057`,
//! `FR-018-016`, `FR-018-037`): the facts the run resource carries. A run that has neither prints what it always printed, and
//! the command asks for nothing more to say it (the schedule is named by its id: `sagat schedule get <id>` is one command away).

#![allow(clippy::expect_used, clippy::panic)]

mod support;

use sagat_cli::Env;
use support::{Reply, request_line, server};

const RUN: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7b8";
const SCHEDULE: &str = "0194f5e2-7b1c-7000-8000-0000000000c3";
const SOURCE: &str = "01a12aca-0d9d-7589-bf01-bc7b7cc0a7aa";
const TOKEN: &str = "secret-cli-token-must-never-appear";

fn inspect(run_body: &str) -> (u8, String, Vec<String>) {
    let (server, requests) = server(vec![
        Reply::ok(run_body),
        Reply::ok(r#"{"items":[]}"#),
        Reply::ok(r#"{"items":[]}"#),
    ]);
    let env = Env {
        server: Some(format!("{server}/v1")),
        token: Some(TOKEN.to_owned()),
        namespace: Some("dev".to_owned()),
        ..Env::default()
    };
    let args: Vec<String> = ["workflow", "inspect", RUN]
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(sagat_cli::run(&args, &env, &mut out, &mut err));
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    (
        code,
        String::from_utf8(out).expect("stdout"),
        requests
            .iter()
            .map(|request| request_line(&request).to_owned())
            .collect(),
    )
}

fn run_with(extra: &str) -> String {
    format!(
        r#"{{"status":"SUCCEEDED","workflowId":"workflow-1","versionId":"version-1","createdAt":"2026-11-04T02:00:03Z","startedBy":"system:schedule:{SCHEDULE}","finishedAt":"2026-11-04T02:00:04Z"{extra}}}"#
    )
}

#[test]
fn a_run_a_schedule_started_says_which_schedule_by_what_kind_of_action_for_which_nominal_time_and_under_which_revision()
 {
    for (kind, words) in [
        ("SCHEDULED", "scheduled"),
        ("TRIGGERED", "run now"),
        ("BACKFILL", "backfill"),
    ] {
        let body = run_with(&format!(
            r#","schedule":{{"scheduleId":"{SCHEDULE}","nominalTime":"2026-11-04T02:00:00Z","kind":"{kind}","revision":3}}"#
        ));
        let (code, out, requests) = inspect(&body);
        assert_eq!(code, 0, "{out}");
        assert_eq!(
            out,
            format!(
                "run {RUN}  SUCCEEDED  workflow workflow-1  version version-1\n  started 2026-11-04T02:00:03Z by system:schedule:{SCHEDULE}, finished 2026-11-04T02:00:04Z\n  started by schedule {SCHEDULE} ({words}), nominal time 2026-11-04T02:00:00Z, revision 3\njournal (0 entries):\n"
            ),
            "{kind}"
        );
        assert_eq!(
            requests,
            [
                format!("GET /v1/runs/{RUN} HTTP/1.1"),
                format!("GET /v1/runs/{RUN}/events?limit=200 HTTP/1.1"),
                format!("GET /v1/runs/{RUN}/attempts HTTP/1.1"),
            ],
            "nothing more is asked to say it"
        );
    }
}

#[test]
fn a_kind_this_tool_does_not_know_is_said_as_the_platform_sent_it_and_a_hostile_value_cannot_forge_a_line()
 {
    let body = run_with(
        r#","schedule":{"scheduleId":"0194f5e2-7b1c-7000-8000-0000000000c3","nominalTime":"2026-11-04T02:00:00Z","kind":"SOMETHING\nelse\u001b[2J","revision":1}"#,
    );
    let (code, out, _) = inspect(&body);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("(SOMETHING else?[2J), nominal time 2026-11-04T02:00:00Z, revision 1\n"),
        "{out}"
    );
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert_eq!(
        out.lines().filter(|line| line.contains("else")).count(),
        1,
        "{out}"
    );
}

#[test]
fn a_run_a_reset_made_says_the_run_it_came_from_and_how_much_of_it_it_keeps() {
    let body = run_with(&format!(
        r#","rerunOf":"{SOURCE}","resetFrom":{{"runId":"{SOURCE}","sequence":4}}"#
    ));
    let (code, out, _) = inspect(&body);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!(
            "\n  reset from run {SOURCE}: it keeps the first 4 events of that run\njournal (0 entries):\n"
        )),
        "{out}"
    );
    assert!(!out.contains("by schedule"), "{out}");
}

#[test]
fn a_reset_that_keeps_one_event_says_event_and_not_events() {
    let body = run_with(&format!(
        r#","rerunOf":"{SOURCE}","resetFrom":{{"runId":"{SOURCE}","sequence":1}}"#
    ));
    let (code, out, _) = inspect(&body);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!(
            "\n  reset from run {SOURCE}: it keeps the first 1 event of that run\n"
        )),
        "{out}"
    );
    assert!(!out.contains("1 events"), "{out}");
}

#[test]
fn a_kind_longer_than_forty_characters_is_clipped_where_the_studio_clips_it() {
    let kind = "A_KIND_A_NEWER_PLATFORM_ADDS_WITH_A_VERY_LONG_NAME";
    assert!(kind.chars().count() > 40);
    let body = run_with(&format!(
        r#","schedule":{{"scheduleId":"{SCHEDULE}","nominalTime":"2026-11-04T02:00:00Z","kind":"{kind}","revision":2}}"#
    ));
    let (code, out, _) = inspect(&body);
    assert_eq!(code, 0, "{out}");
    let clipped: String = kind.chars().take(40).collect();
    assert!(
        out.contains(&format!(
            "({clipped}…), nominal time 2026-11-04T02:00:00Z, revision 2\n"
        )),
        "{out}"
    );
    assert!(!out.contains(kind), "{out}");
}

#[test]
fn a_run_with_neither_prints_what_it_always_printed() {
    let (code, out, _) = inspect(&run_with(""));
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        format!(
            "run {RUN}  SUCCEEDED  workflow workflow-1  version version-1\n  started 2026-11-04T02:00:03Z by system:schedule:{SCHEDULE}, finished 2026-11-04T02:00:04Z\njournal (0 entries):\n"
        )
    );
}
