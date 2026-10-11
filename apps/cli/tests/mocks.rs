//! The answers `worker dev` gives, and what the file that holds them must say.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_cli::mocks::{Answer, Kind, Mocks, activities_of};
use serde_json::json;

fn parse(text: &str) -> Mocks {
    Mocks::parse(text, true).expect("a valid mock file")
}

fn problems(text: &str) -> String {
    Mocks::parse(text, true)
        .expect_err("a refused mock file")
        .join("\n")
}

#[test]
fn a_fixed_output_an_echo_and_a_failure_are_the_answers() {
    let mocks = parse(
        "activities:\n  a.fixed:\n    output: {x: 1}\n  a.echo:\n    echo: true\n  a.fails:\n    fail: {code: DECLINED, retryable: false, message: no}\n  a.lost:\n    lose: true\n",
    );
    assert_eq!(
        mocks.answer(Kind::Forward, "a.fixed").0,
        Answer::Output(json!({"x": 1}))
    );
    assert_eq!(mocks.answer(Kind::Forward, "a.echo").0, Answer::Echo);
    assert_eq!(
        mocks.answer(Kind::Forward, "a.fails").0,
        Answer::Fail {
            code: "DECLINED".into(),
            retryable: false,
            message: "no".into()
        }
    );
    assert_eq!(mocks.answer(Kind::Forward, "a.lost").0, Answer::Lose);
}

#[test]
fn a_sequence_advances_with_each_call_and_its_last_answer_repeats() {
    let mocks = parse(
        "activities:\n  pay:\n    sequence:\n      - fail: {code: TIMEOUT, retryable: true}\n      - output: {ok: true}\n",
    );
    assert!(matches!(
        mocks.answer(Kind::Forward, "pay").0,
        Answer::Fail {
            retryable: true,
            ..
        }
    ));
    assert_eq!(
        mocks.answer(Kind::Forward, "pay").0,
        Answer::Output(json!({"ok": true}))
    );
    assert_eq!(
        mocks.answer(Kind::Forward, "pay").0,
        Answer::Output(json!({"ok": true}))
    );
}

#[test]
fn what_the_file_does_not_say_is_answered_the_way_that_shows_the_gap() {
    let mocks = parse("activities:\n  known:\n    echo: true\n");
    // An activity nobody gave an answer for fails, naming the ones that have one.
    match mocks.answer(Kind::Forward, "unknown").0 {
        Answer::Fail {
            code,
            retryable,
            message,
        } => {
            assert_eq!(code, "NO_MOCK");
            assert!(!retryable);
            assert!(
                message.contains("'unknown'") && message.contains("known"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    // A compensation nobody mocked succeeds; a reconciliation nobody mocked does not know.
    assert_eq!(
        mocks.answer(Kind::Compensation, "undo").0,
        Answer::Output(json!({}))
    );
    assert_eq!(
        mocks.answer(Kind::Reconciliation, "find").0,
        Answer::Output(json!({"effect": "unknown"}))
    );
}

#[test]
fn a_reconciler_says_whether_the_effect_happened() {
    let mocks = parse(
        "reconcilers:\n  find:\n    effect: applied\n    output: {chargeId: ch-1}\n  none:\n    effect: not_applied\n",
    );
    assert_eq!(
        mocks.answer(Kind::Reconciliation, "find").0,
        Answer::Output(json!({"effect": "applied", "output": {"chargeId": "ch-1"}}))
    );
    assert_eq!(
        mocks.answer(Kind::Reconciliation, "none").0,
        Answer::Output(json!({"effect": "not_applied"}))
    );
}

#[test]
fn the_delay_is_the_entry_s_own_or_the_file_s_and_never_more_than_the_most() {
    let mocks = parse(
        "delayMs: 50\nactivities:\n  slow:\n    output: {}\n    delayMs: 200\n  normal:\n    output: {}\n",
    );
    assert_eq!(mocks.answer(Kind::Forward, "slow").1, 200);
    assert_eq!(mocks.answer(Kind::Forward, "normal").1, 50);
    assert!(
        problems("delayMs: 99999\nactivities: {a: {output: {}}}")
            .contains("delayMs is at most 10000")
    );
}

#[test]
fn every_mistake_in_the_file_is_reported_at_once() {
    let report = problems(
        "activities:\n  Bad Name:\n    output: {}\n  two:\n    output: {}\n    echo: true\n  none: {}\n  empty:\n    sequence: []\n  mixed:\n    output: {}\n    sequence:\n      - output: {}\n  inner:\n    sequence:\n      - output: {}\n        fail: {code: X}\nreconcilers:\n  find:\n    effect: maybe\n  other:\n    effect: not_applied\n    output: {a: 1}\n",
    );
    for expect in [
        "activities.Bad Name: not an activity name",
        "activities.two: exactly one of output, echo, fail, lose (or a sequence)",
        "activities.none: exactly one of output, echo, fail, lose (or a sequence)",
        "activities.empty: a sequence has at least one answer",
        "activities.mixed: a sequence and an answer of its own",
        "activities.inner.sequence[0]: exactly one of output, echo, fail, lose",
        "reconcilers.find: effect is applied, not_applied or unknown, not 'maybe'",
        "reconcilers.other: only an applied effect has an output",
    ] {
        assert!(report.contains(expect), "{expect} in\n{report}");
    }
}

#[test]
fn an_unknown_field_is_refused_rather_than_ignored_and_the_file_is_not_quoted_back() {
    assert!(problems("activites:\n  a: {output: {}}").contains("unknown field `activites`"));
    // A file that is something else entirely (a line that must stay secret) is not echoed.
    let secret = "ghp_0123456789abcdefSECRETSECRETSECRET";
    assert!(!problems(secret).contains("SECRET"));
    assert!(!problems(&format!("activities: \"{secret}\"")).contains("SECRET"));
}

#[test]
fn the_names_it_serves_are_every_activity_it_answers_for_and_the_workflow_s_own() {
    let mocks = parse(
        "activities:\n  a.run: {echo: true}\ncompensations:\n  a.undo: {output: {}}\nreconcilers:\n  a.find: {effect: unknown}\n",
    );
    assert_eq!(
        mocks.names().into_iter().collect::<Vec<_>>(),
        ["a.find", "a.run", "a.undo"]
    );
    let document = json!({"nodes": [
        {"id": "x", "type": "activity", "activity": "a.run", "compensateWith": "a.undo", "reconcileWith": "a.find"},
        {"id": "y", "type": "activity", "activity": "b.other"},
        {"id": "z", "type": "timer", "durationMs": 5}
    ]});
    assert_eq!(
        activities_of(&document).into_iter().collect::<Vec<_>>(),
        ["a.find", "a.run", "a.undo", "b.other"]
    );
}

#[test]
fn the_kind_of_a_task_is_read_from_its_lease() {
    assert_eq!(Kind::of(""), Some(Kind::Forward));
    assert_eq!(Kind::of("FORWARD"), Some(Kind::Forward));
    assert_eq!(Kind::of("COMPENSATION"), Some(Kind::Compensation));
    assert_eq!(Kind::of("RECONCILIATION"), Some(Kind::Reconciliation));
    assert_eq!(Kind::of("SOMETHING_NEW"), None);
}

#[test]
fn a_lost_answer_is_an_answer_in_a_sequence_too() {
    let mocks = parse(
        "activities:\n  charge:\n    sequence:\n      - lose: true\n      - output: {ok: true}\n",
    );
    assert_eq!(mocks.answer(Kind::Forward, "charge").0, Answer::Lose);
    assert_eq!(
        mocks.answer(Kind::Forward, "charge").0,
        Answer::Output(json!({"ok": true}))
    );
    assert!(
        problems("activities:\n  charge:\n    lose: true\n    output: {}")
            .contains("exactly one of")
    );
}

#[test]
fn a_failure_code_the_platform_would_refuse_is_refused_before_the_worker_starts() {
    for code in [
        "lower",
        "WITH SPACE",
        "",
        "X".repeat(65).as_str(),
        "NEWLINE\n",
        "ACENTUAÇÃO",
    ] {
        let text = format!(
            "activities:\n  a.b:\n    fail: {{ code: {}, retryable: false }}\n",
            serde_json::to_string(code).expect("json")
        );
        let problems = Mocks::parse(&text, true).expect_err(code);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("a failure code is 1 to 64 capital letters")),
            "{code:?}: {problems:?}"
        );
    }
    // In a sequence too, and a good one passes.
    let sequence =
        "activities:\n  a.b:\n    sequence:\n      - fail: { code: bad, retryable: true }\n";
    assert!(Mocks::parse(sequence, true).is_err());
    let good = "activities:\n  a.b:\n    fail: { code: CARD_DECLINED_2, retryable: false }\n";
    assert!(Mocks::parse(good, true).is_ok());
}
