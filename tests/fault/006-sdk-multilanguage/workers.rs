//! Workers that fail: they go silent, the network under them refuses, their activity fails or is
//! cancelled. The platform must come to the same state whatever the language of the worker
//! (`FR-006-003`, `NFR-006-DUR`).

use serde_json::json;

use crate::harness::{self, Selection, journal, node, observation, worker_log};

#[test]
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
            let second = worker_log(doc, "w2");
            assert_eq!(second.len(), 1, "{language}: {second:?}");
            assert_eq!(second[0]["attempt"], json!(2), "{language}");
            // The step was completed once, by the second attempt.
            let events = journal(doc, "journal");
            assert_eq!(
                events.iter().filter(|e| *e == "step.completed").count(),
                1,
                "{language}: {events:?}"
            );
            assert_eq!(node(doc, "work")["attempt"], json!(2), "{language}");
            assert_eq!(
                observation(doc, "done")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
        }
    });
}

#[test]
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
