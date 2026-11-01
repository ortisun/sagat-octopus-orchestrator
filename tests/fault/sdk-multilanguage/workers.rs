//! Workers that fail: they go silent, the network under them refuses, their activity fails or is
//! cancelled. The platform must come to the same state whatever the language of the worker
//! (`FR-006-003`, `NFR-006-DUR`).

use serde_json::json;

use crate::harness::{self, Selection, journal, node, observation, worker_log};

fn assert_human_resolution_precedes_successor(
    doc: &serde_json::Value,
    language: &str,
    activity: &str,
) {
    let review = &observation(doc, "review-state")["result"];
    assert_eq!(review["status"], "MANUAL_REVIEW", "{language}");
    assert!(review["finishedAt"].is_null(), "{language}");
    let refused = &observation(doc, "retry-refused")["error"];
    assert_eq!(refused["code"], "CONFLICT", "{language}");
    assert_eq!(refused["reason"], "STATE_CONFLICT", "{language}");
    assert_eq!(refused["retryable"], false, "{language}");
    let waiting = journal(doc, "review-journal");
    assert!(
        waiting.contains(&"step.needs_review".to_owned()),
        "{language}"
    );
    for forbidden in [
        "step.completed",
        "step.resolved",
        "step.retry_scheduled",
        "run.finished",
    ] {
        assert!(
            !waiting.iter().any(|event| event == forbidden),
            "{language}: {waiting:?}"
        );
    }

    let second = worker_log(doc, "w2");
    assert_eq!(second.len(), 1, "{language}: {second:?}");
    assert_eq!(second[0]["activity"], activity, "{language}");
    assert_eq!(second[0]["node"], "after", "{language}");
    assert_eq!(second[0]["attempt"], json!(1), "{language}");
    assert_eq!(node(doc, "work")["attempt"], json!(1), "{language}");
    assert_eq!(node(doc, "work")["status"], "SUCCEEDED", "{language}");
    assert_eq!(node(doc, "after")["status"], "SUCCEEDED", "{language}");
    assert_eq!(
        observation(doc, "done")["result"]["status"],
        "SUCCEEDED",
        "{language}"
    );

    let events = journal(doc, "journal");
    for forbidden in ["step.failed", "step.retry_scheduled", "step.retry_due"] {
        assert!(
            !events.iter().any(|event| event == forbidden),
            "{language}: {events:?}"
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "step.completed")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "step.resolved")
            .count(),
        1
    );
    let needed = events
        .iter()
        .position(|event| event == "step.needs_review")
        .expect("review");
    let resolved = events
        .iter()
        .position(|event| event == "step.resolved")
        .expect("human resolution");
    let completed = events
        .iter()
        .position(|event| event == "step.completed")
        .expect("successor completion");
    assert!(
        needed < resolved && resolved < completed,
        "{language}: {events:?}"
    );
    assert_eq!(
        doc["reviewEvidence"],
        json!([{
            "action": "saga.step_resolved",
            "actor": "alice",
            "reason": "declared SUCCEEDED: simulated human assertion of final success; no provider verification is claimed"
        }]),
        "{language}: the simulated human decision does not claim provider verification"
    );
}

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_worker_that_goes_silent_loses_its_lease_and_its_result_is_never_reported() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["lease-lost"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("lease-lost") {
            let doc = suite.doc("lease-lost", language);
            let first = worker_log(doc, "w1");
            assert_eq!(first.len(), 1, "{language}: {first:?}");
            assert_eq!(first[0]["abortedReason"], "lease-lost", "{language}");
            assert_eq!(first[0]["outcome"], "cancelled", "{language}");
            assert_human_resolution_precedes_successor(doc, language, "lease.after");
        }
    });
}

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_network_that_refuses_every_call_of_the_protocol_once_or_twice_costs_the_worker_nothing() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["worker-transport-faults"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("worker-transport-faults") {
            let doc = suite.doc("worker-transport-faults", language);
            assert_eq!(doc["faultsApplied"], json!([1, 2, 1, 2, 2]), "{language}");
            // One attempt: the lease was kept through the refused heartbeats and the step completed once.
            assert_eq!(worker_log(doc, "w1").len(), 1, "{language}");
            assert_eq!(node(doc, "work")["attempt"], json!(1), "{language}");
            let events = journal(doc, "journal");
            assert_eq!(
                events.iter().filter(|e| *e == "step.completed").count(),
                1,
                "{language}: {events:?}"
            );
        }
    });
}

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_cancelled_run_reaches_the_activity_and_a_wait_that_times_out_takes_its_branch() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&[
            "cancellation",
            "signals-and-events",
        ]))
        .await;
        suite.assert_conforms();
        for language in suite.languages_of("cancellation") {
            let doc = suite.doc("cancellation", language);
            assert_eq!(
                observation(doc, "cancelled")["result"]["status"],
                "CANCELLED",
                "{language}"
            );
            let slow = &worker_log(doc, "w1")[0];
            assert_eq!(slow["abortedReason"], "run-cancelled", "{language}");
            // Cancelling a run that finished is the canonical conflict.
            assert_eq!(
                observation(doc, "cancel-finished")["error"]["code"],
                "CONFLICT",
                "{language}"
            );
        }
        for language in suite.languages_of("signals-and-events") {
            let doc = suite.doc("signals-and-events", language);
            let events = journal(doc, "timeout-journal");
            assert!(
                events.contains(&"wait.timed_out".to_owned()),
                "{language}: {events:?}"
            );
            assert!(
                !events.contains(&"wait.satisfied".to_owned()),
                "{language}: {events:?}"
            );
            // A signal nobody waits for is the canonical conflict, and an event id seen twice is a duplicate.
            assert_eq!(
                observation(doc, "nobody-waits-for-this")["error"]["code"],
                "CONFLICT",
                "{language}"
            );
            assert_eq!(
                observation(doc, "event-again")["result"]["status"],
                "DUPLICATE",
                "{language}"
            );
        }
    });
}

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_worker_that_is_stopped_while_it_works_does_not_fail_the_step_for_good() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["worker-shutdown"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("worker-shutdown") {
            let doc = suite.doc("worker-shutdown", language);
            // The host cancelled the handler's token because it was stopping; its cancellation
            // was not reported as a known failure. The expired lease requires human resolution.
            let first = worker_log(doc, "w1");
            assert_eq!(first.len(), 1, "{language}: {first:?}");
            assert_eq!(first[0]["abortedReason"], "shutdown", "{language}");
            assert_eq!(first[0]["outcome"], "cancelled", "{language}");
            assert_human_resolution_precedes_successor(doc, language, "shutdown.after");
        }
    });
}

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_handler_that_ignores_its_cancellation_is_left_to_finish_and_what_it_returned_is_reported() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["cancellation"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("cancellation") {
            let doc = suite.doc("cancellation", language);
            let stubborn = &worker_log(doc, "w1")[1];
            assert_eq!(stubborn["activity"], "cancel.stubborn", "{language}");
            assert_eq!(
                stubborn["abortedReason"], "run-cancelled",
                "{language}: the token was cancelled"
            );
            assert_eq!(
                stubborn["outcome"], "returned",
                "{language}: the handler was not interrupted"
            );
            // Its result reached the platform: the step is SUCCEEDED in a run that was cancelled.
            assert_eq!(node(doc, "stubborn")["status"], "SUCCEEDED", "{language}");
            assert_eq!(
                observation(doc, "stubborn-state")["result"]["status"],
                "CANCELLED",
                "{language}"
            );
        }
    });
}
