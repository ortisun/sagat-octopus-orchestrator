//! Duplicates and timeouts on the REST channel: a call is repeated only when that is safe, and a repeat
//! is never a second effect (`FR-006-005`).

use serde_json::json;

use crate::harness::{self, Selection, journal, observation};

#[test]
fn a_start_whose_answer_is_lost_or_that_never_left_is_one_run_and_a_refusal_is_not_repeated() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["lost-responses"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("lost-responses") {
            let doc = suite.doc("lost-responses", language);
            // 1 a start that never left, 1 a start whose answer was thrown away, 1 completion whose
            // answer was thrown away, 2 refusals of a read, 1 refusal of a start (which is not repeated),
            // 1 request timeout (408) on a read, which is repeated.
            assert_eq!(
                doc["faultsApplied"],
                json!([1, 1, 1, 2, 1, 1]),
                "{language}"
            );
            // Two starts, two runs: the repeats did not make a third.
            let runs = observation(doc, "runs")["result"]["items"]
                .as_array()
                .expect("items");
            assert_eq!(runs.len(), 2, "{language}: {runs:?}");
            // The run whose completion was repeated was completed once, on its first attempt.
            let events = journal(doc, "journal");
            assert_eq!(
                events.iter().filter(|e| *e == "step.completed").count(),
                1,
                "{language}: {events:?}"
            );
            assert_eq!(
                observation(doc, "refused")["error"]["code"],
                "VALIDATION_FAILED",
                "{language}"
            );
        }
    });
}

#[test]
fn a_transport_that_is_down_is_retried_as_far_as_the_policy_goes_and_ends_as_the_canonical_error() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["errors"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("errors") {
            let doc = suite.doc("errors", language);
            // The default policy is four attempts: the fault answered four calls, then the SDK gave up.
            assert_eq!(doc["faultsApplied"], json!([4]), "{language}");
            let error = &observation(doc, "unreachable")["error"];
            assert_eq!(error["code"], "TRANSPORT", "{language}: {error}");
            assert_eq!(error["retryable"], json!(true), "{language}");
        }
    });
}
