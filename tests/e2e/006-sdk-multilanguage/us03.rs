//! US03 — a typed saga in any SDK (`FR-006-004`, `US03-AC01`): an activity with a compensation or a
//! reconciliation, when the workflow fails, gives every SDK the same semantics and the same canonical
//! errors.

use serde_json::json;

use crate::harness::{self, Selection, journal, node, observation, worker_log};

#[test]
fn a_saga_that_fails_is_compensated_in_reverse_order_and_an_unknown_outcome_is_reconciled_in_every_sdk()
 {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["saga-compensation"])).await;
        suite.assert_conforms();
        let languages = suite.languages_of("saga-compensation");
        assert!(!languages.is_empty());
        for language in languages {
            let doc = suite.doc("saga-compensation", language);
            // The last step fails for good: the saga is compensated, newest effect first.
            assert_eq!(
                observation(doc, "saga-done")["result"]["status"],
                "COMPENSATED",
                "{language}"
            );
            let log = worker_log(doc, "w1");
            let compensations: Vec<_> = log
                .iter()
                .filter(|e| e["taskKind"] == "COMPENSATION")
                .collect();
            assert_eq!(compensations.len(), 2, "{language}: {compensations:?}");
            assert_eq!(compensations[0]["activity"], "saga.refund", "{language}");
            assert_eq!(compensations[0]["targetNode"], "charge", "{language}");
            assert_eq!(compensations[1]["activity"], "saga.release", "{language}");
            assert_eq!(compensations[1]["targetNode"], "reserve", "{language}");
            for compensation in &compensations {
                // The compensation names the key of the effect it undoes, which is the provider's key.
                let target = compensation["targetIdempotencyKey"]
                    .as_str()
                    .expect("target key");
                assert!(
                    target.ends_with(&format!(
                        ":{}",
                        compensation["targetNode"].as_str().expect("node")
                    )),
                    "{language}: {target}"
                );
                assert_ne!(
                    compensation["idempotencyKey"], compensation["targetIdempotencyKey"],
                    "{language}"
                );
            }
            // The platform holds the same story: the step failed with the code the activity gave, the step
            // after it never started, and both compensations ran.
            assert_eq!(node(doc, "ship")["status"], "FAILED", "{language}");
            assert_eq!(node(doc, "ship")["failureCode"], "NO_STOCK", "{language}");
            assert_eq!(node(doc, "notify")["status"], "NOT_STARTED", "{language}");
            for undone in ["charge~c", "reserve~c"] {
                assert_eq!(
                    node(doc, undone)["status"],
                    "SUCCEEDED",
                    "{language}/{undone}"
                );
                assert_eq!(
                    node(doc, undone)["kind"],
                    "COMPENSATION",
                    "{language}/{undone}"
                );
            }
            // A step whose outcome is unknown is reconciled; the reconciler's verdict continues the run.
            assert_eq!(
                observation(doc, "reconcile-done")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
            let kinds = journal(doc, "reconcile-journal");
            assert!(
                kinds.contains(&"reconciliation.requested".to_owned()),
                "{language}: {kinds:?}"
            );
            let lookup = log
                .iter()
                .find(|e| e["taskKind"] == "RECONCILIATION")
                .expect("the lookup ran");
            assert_eq!(lookup["activity"], "saga.lookup", "{language}");
            assert_eq!(lookup["targetNode"], "pay", "{language}");
            // A reconciler that finds the effect did NOT happen: the platform tries the step again, and it succeeds.
            assert_eq!(
                observation(doc, "not-applied-done")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
            assert_eq!(node(doc, "pay2")["attempt"], json!(2), "{language}");
            assert_eq!(
                node(doc, "pay2")["failureCode"],
                "RECONCILED_NOT_APPLIED",
                "{language}"
            );
            // A reconciler that cannot say: nothing is retried blindly, a person has to decide.
            let undecided = observation(doc, "undecided-state")["result"].clone();
            assert_eq!(
                undecided["status"], "MANUAL_REVIEW",
                "{language}: {undecided}"
            );
            assert_eq!(node(doc, "pay3")["status"], "WAITING", "{language}");
            assert_eq!(node(doc, "pay3")["waitingOn"], "operator", "{language}");
            assert_eq!(
                node(doc, "pay3")["failureCode"],
                "RECONCILIATION_UNKNOWN",
                "{language}"
            );
            assert_eq!(
                node(doc, "pay3")["attempt"],
                json!(1),
                "{language}: not tried again"
            );
        }
    });
}

#[test]
fn the_failures_of_an_activity_reach_the_run_as_the_same_canonical_codes_in_every_sdk() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["failures-and-retries"])).await;
        suite.assert_conforms();
        let languages = suite.languages_of("failures-and-retries");
        assert!(!languages.is_empty());
        for language in languages {
            let doc = suite.doc("failures-and-retries", language);
            // (node, attempts, the code the platform recorded, the status of the run)
            for (id, attempts, code, run) in [
                ("flaky", 3, "PROVIDER_BUSY", "SUCCEEDED"),
                ("permanent", 1, "INSUFFICIENT_FUNDS", "FAILED"),
                ("throws", 2, "ACTIVITY_ERROR", "FAILED"),
                ("unknown", 1, "OUTCOME_UNKNOWN", "FAILED"),
            ] {
                let node = node(doc, id);
                assert_eq!(node["attempt"], json!(attempts), "{language}/{id}: {node}");
                assert_eq!(node["failureCode"], code, "{language}/{id}");
                assert_eq!(harness::run_status(doc, id), run, "{language}/{id}");
            }
        }
    });
}
