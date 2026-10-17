//! US03 beyond hello-world (`US03-AC01`, `FR-012-002`) and feature 014's US01 (`FR-014-001`,
//! `FR-014-002`, `SC-014-001`, `T014-010`): every official SDK runs the same order-processing
//! examples through conditions, an all-branches parallel join and a durable timer. The platform
//! journal, rather than console output alone, is the semantic oracle.
//!
//! `#[ignore]` because the test needs Node, Python, Go, .NET and a JDK. CI's SDK job provides
//! those toolchains. `SAGAT_EXAMPLE_LANGS=go,python` selects a non-empty subset, which is a
//! diagnostic run and not five-language parity evidence.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use reqwest::Method;
use serde_json::Value;
use uuid::Uuid;

use crate::support::worker::{Platform, platform};
use crate::us03::{Finished, script};
use crate::{Dir, cli, engine, rest};

const LANGUAGES: [&str; 5] = ["typescript", "go", "python", "dotnet", "java"];

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    scenario: &'static str,
    reject: bool,
    fail_notify: bool,
    code: i32,
    status: &'static str,
    activities: &'static [&'static str],
}

const CASES: [Case; 5] = [
    Case {
        name: "routing-approved",
        scenario: "routing",
        reject: false,
        fail_notify: false,
        code: 0,
        status: "SUCCEEDED",
        activities: &["order.notify", "order.validate"],
    },
    Case {
        name: "routing-rejected",
        scenario: "routing",
        reject: true,
        fail_notify: false,
        code: 0,
        status: "SUCCEEDED",
        activities: &["order.validate"],
    },
    Case {
        name: "fulfillment",
        scenario: "fulfillment",
        reject: false,
        fail_notify: false,
        code: 0,
        status: "SUCCEEDED",
        activities: &["order.notify", "order.quote", "order.reserve"],
    },
    Case {
        name: "reminder-success",
        scenario: "reminder",
        reject: false,
        fail_notify: false,
        code: 0,
        status: "SUCCEEDED",
        activities: &["order.notify"],
    },
    Case {
        name: "reminder-notification-failure",
        scenario: "reminder",
        reject: false,
        fail_notify: true,
        code: 1,
        status: "FAILED",
        activities: &["order.notify"],
    },
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the repository")
        .to_path_buf()
}

fn selected_languages() -> Vec<&'static str> {
    let Ok(value) = std::env::var("SAGAT_EXAMPLE_LANGS") else {
        return LANGUAGES.to_vec();
    };
    let requested: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    assert!(
        !requested.is_empty(),
        "SAGAT_EXAMPLE_LANGS names no language"
    );
    for name in &requested {
        assert!(
            LANGUAGES.contains(name),
            "SAGAT_EXAMPLE_LANGS contains unknown language '{name}'"
        );
    }
    let selected: Vec<&'static str> = LANGUAGES
        .into_iter()
        .filter(|name| requested.contains(name))
        .collect();
    if selected.len() < LANGUAGES.len() {
        eprintln!(
            "SAGAT_EXAMPLE_LANGS selects {selected:?}: a subset run is not five-language parity evidence"
        );
    }
    selected
}

fn settings(
    p: &Platform,
    grpc: &str,
    token: &str,
    scenario: &str,
    reject: bool,
    fail_notify: bool,
) -> Vec<(&'static str, String)> {
    vec![
        ("SAGAT_SERVER", p.api.base.clone()),
        ("SAGAT_GRPC", grpc.to_owned()),
        ("SAGAT_NAMESPACE", "payments".to_owned()),
        ("SAGAT_TOKEN", token.to_owned()),
        ("SAGAT_SCENARIO", scenario.to_owned()),
        (
            "SAGAT_REJECT_ORDER",
            if reject { "1" } else { "0" }.to_owned(),
        ),
        (
            "SAGAT_FAIL_NOTIFY",
            if fail_notify { "1" } else { "0" }.to_owned(),
        ),
    ]
}

fn assertion_text<'a>(language: &str, purpose: &str, finished: &'a Finished) -> &'a str {
    assert!(
        !finished.out.is_empty() || !finished.err.is_empty(),
        "{language} {purpose} produced no diagnostic"
    );
    if finished.err.is_empty() {
        &finished.out
    } else {
        &finished.err
    }
}

fn run_line(language: &str, case: Case, finished: &Finished) -> String {
    let prefix = format!("scenario {} run ", case.scenario);
    let lines: Vec<&str> = finished
        .out
        .lines()
        .filter(|line| line.starts_with(&prefix))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "{language} {} must print one final run line:\n{}",
        case.name,
        finished.out
    );
    let fields: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(
        fields.len(),
        5,
        "{language} {} malformed final line: {}",
        case.name,
        lines[0]
    );
    assert_eq!(fields[4], case.status, "{language} {}", case.name);
    Uuid::parse_str(fields[3])
        .unwrap_or_else(|e| panic!("{language} {} invalid run id: {e}", case.name));
    fields[3].to_owned()
}

fn activity_lines(finished: &Finished) -> Vec<&str> {
    finished
        .out
        .lines()
        .filter_map(|line| line.strip_prefix("activity "))
        .collect()
}

async fn journal(p: &Platform, run: &str) -> (Value, Vec<Value>) {
    let (run_status, run_body) = rest(p, Method::GET, &format!("/runs/{run}"), None).await;
    assert_eq!(run_status, 200, "{run_body}");
    let (event_status, event_body) = rest(
        p,
        Method::GET,
        &format!("/runs/{run}/events?limit=200"),
        None,
    )
    .await;
    assert_eq!(event_status, 200, "{event_body}");
    let events = event_body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("run {run} has no journal: {event_body}"))
        .clone();
    (run_body, events)
}

fn event_position(events: &[Value], kind: &str, step: &str) -> usize {
    events
        .iter()
        .position(|event| event["eventType"] == kind && event["stepId"] == step)
        .unwrap_or_else(|| panic!("no {kind} for {step} in {events:?}"))
}

fn assert_persisted_case(language: &str, case: Case, run: &Value, events: &[Value]) {
    assert_eq!(
        run["status"], case.status,
        "{language} {}: {run}",
        case.name
    );
    let trace = run["traceId"]
        .as_str()
        .unwrap_or_else(|| panic!("{language} {} has no trace: {run}", case.name));
    assert!(
        trace.len() == 32 && trace.chars().all(|c| c.is_ascii_hexdigit()),
        "{language} {} has invalid trace {trace}",
        case.name
    );
    assert!(!events.is_empty(), "{language} {} has no events", case.name);
    assert_eq!(
        events.last().and_then(|event| event["eventType"].as_str()),
        Some("run.finished"),
        "{language} {} journal has no terminal event: {events:?}",
        case.name
    );
    for event in events {
        assert_eq!(
            event["traceId"].as_str(),
            Some(trace),
            "{language} {} event is outside the run trace: {event}",
            case.name
        );
    }

    match case.name {
        "routing-approved" => {
            let decided = event_position(events, "condition.evaluated", "accepted");
            event_position(events, "step.completed", "validate");
            let notify = event_position(events, "step.leased", "notify");
            assert!(
                decided < notify,
                "{language} notify was leased before the routing decision: {events:?}"
            );
            event_position(events, "step.completed", "notify");
            assert!(
                !events.iter().any(|event| event["stepId"] == "rejected"),
                "{language} approved order reached the rejected end: {events:?}"
            );
        }
        "routing-rejected" => {
            event_position(events, "condition.evaluated", "accepted");
            event_position(events, "end.reached", "rejected");
            assert!(
                !events.iter().any(|event| event["stepId"] == "notify"),
                "rejected order reached notify: {events:?}"
            );
        }
        "fulfillment" => {
            event_position(events, "parallel.forked", "fan");
            let joined = event_position(events, "parallel.joined", "fan");
            let notify = event_position(events, "step.leased", "notify");
            assert!(
                event_position(events, "step.completed", "reserve") < notify
                    && event_position(events, "step.completed", "quote") < notify
                    && joined < notify,
                "{language} notify ran before both branches completed: {events:?}"
            );
        }
        "reminder-success" => {
            let fired = event_position(events, "timer.fired", "reminder-delay");
            let notify = event_position(events, "step.leased", "notify");
            assert!(
                fired < notify,
                "{language} notify preceded the timer: {events:?}"
            );
            event_position(events, "step.completed", "notify");
        }
        "reminder-notification-failure" => {
            let fired = event_position(events, "timer.fired", "reminder-delay");
            let notify = event_position(events, "step.leased", "notify");
            assert!(
                fired < notify,
                "{language} notify preceded the timer: {events:?}"
            );
            let failed = event_position(events, "step.failed", "notify");
            assert_eq!(
                events[failed]["cause"]["code"], "NOTIFICATION_REJECTED",
                "{language} failure lacks its typed code: {}",
                events[failed]
            );
            assert!(
                !events
                    .iter()
                    .any(|event| event["eventType"] == "step.completed"
                        && event["stepId"] == "notify"),
                "{language} reported a rejected notification as completed: {events:?}"
            );
            // The run resource carries only the status; the run's own terminal event names it.
            let finished = events.last().expect("a terminal event");
            assert_eq!(
                finished["cause"]["code"], "FAILED",
                "{language} run did not finish FAILED: {finished}"
            );
        }
        other => panic!("unhandled test case {other}"),
    }
}

fn semantic_shape(case: Case, events: &[Value]) -> Vec<String> {
    let mut shape: Vec<String> = events
        .iter()
        .map(|event| {
            format!(
                "{} {} {} {} {}",
                event["eventType"].as_str().unwrap_or("?"),
                event["stepId"].as_str().unwrap_or("-"),
                event["attempt"],
                event["cause"]["code"].as_str().unwrap_or("-"),
                event["payloadRef"].as_str().unwrap_or("-")
            )
        })
        .collect();
    if case.scenario == "fulfillment" {
        // The two branches can be leased and completed in either order; their event multiset is
        // the stable semantic contract. `assert_persisted_case` checks the required join order.
        shape.sort();
    }
    shape
}

#[tokio::test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job runs it with --ignored"]
async fn order_processing_examples_have_one_semantic_contract_in_all_five_sdks() {
    let languages = selected_languages();
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();

    let workflows = repo().join("examples/order-processing/workflows");
    for name in ["routing", "fulfillment", "reminder"] {
        let file = workflows.join(format!("{name}.workflow.json"));
        let file = file
            .to_str()
            .unwrap_or_else(|| panic!("non-UTF-8 workflow path: {}", file.display()));
        cli(&p, &dir, &["workflow", "publish", file])
            .await
            .expect(0);
    }
    cli(
        &p,
        &dir,
        &[
            "worker",
            "bind",
            "order.validate",
            "order.reserve",
            "order.quote",
            "order.notify",
        ],
    )
    .await
    .expect(0)
    .mentions("bound alice to 4 activities in payments");

    let mut parity: HashMap<&str, Vec<String>> = HashMap::new();
    for language in languages {
        let prepared = script(
            "order-processing",
            language,
            "prepare.sh",
            900,
            &settings(&p, &engine.grpc_url, "unused", "routing", false, false),
        )
        .await;
        assert_eq!(
            prepared.code,
            Some(0),
            "{language} prepare.sh:\n{}\n{}",
            prepared.out,
            prepared.err
        );

        let missing = script(
            "order-processing",
            language,
            "run.sh",
            60,
            &settings(&p, &engine.grpc_url, "", "routing", false, false),
        )
        .await;
        assert_eq!(
            missing.code,
            Some(2),
            "{language}: {}{}",
            missing.out,
            missing.err
        );
        assert!(
            assertion_text(language, "missing-token check", &missing).contains("SAGAT_TOKEN"),
            "{language} did not identify the missing setting: {}{}",
            missing.out,
            missing.err
        );

        let scenario_check_token = p.api.idp.token("alice", "acme").build();
        let invalid = script(
            "order-processing",
            language,
            "run.sh",
            60,
            &settings(
                &p,
                &engine.grpc_url,
                &scenario_check_token,
                "not-a-scenario",
                false,
                false,
            ),
        )
        .await;
        assert_eq!(
            invalid.code,
            Some(2),
            "{language}: {}{}",
            invalid.out,
            invalid.err
        );
        assert!(
            assertion_text(language, "unknown-scenario check", &invalid).contains("SAGAT_SCENARIO"),
            "{language} did not identify the bad scenario: {}{}",
            invalid.out,
            invalid.err
        );
        assert!(
            !invalid.out.contains(&scenario_check_token)
                && !invalid.err.contains(&scenario_check_token),
            "{language} printed its token during validation"
        );

        for case in CASES {
            let token = p.api.idp.token("alice", "acme").build();
            let finished = script(
                "order-processing",
                language,
                "run.sh",
                180,
                &settings(
                    &p,
                    &engine.grpc_url,
                    &token,
                    case.scenario,
                    case.reject,
                    case.fail_notify,
                ),
            )
            .await;
            assert_eq!(
                finished.code,
                Some(case.code),
                "{language} {}:\n{}\n{}",
                case.name,
                finished.out,
                finished.err
            );
            assert!(
                !finished.out.contains(&token) && !finished.err.contains(&token),
                "{language} {} printed its token",
                case.name
            );

            let actual: BTreeSet<&str> = activity_lines(&finished).into_iter().collect();
            let expected: BTreeSet<&str> = case.activities.iter().copied().collect();
            assert_eq!(actual, expected, "{language} {} output", case.name);
            assert_eq!(
                activity_lines(&finished).len(),
                expected.len(),
                "{language} {} printed an activity more than once",
                case.name
            );

            let run_id = run_line(language, case, &finished);
            let (run, events) = journal(&p, &run_id).await;
            assert_persisted_case(language, case, &run, &events);
            let shape = semantic_shape(case, &events);
            if let Some(reference) = parity.get(case.name) {
                assert_eq!(
                    &shape, reference,
                    "{language} made a different {} journal",
                    case.name
                );
            } else {
                parity.insert(case.name, shape);
            }
        }
    }
}
