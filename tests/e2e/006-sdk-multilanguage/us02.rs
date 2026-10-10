//! US02 — a worker is registered from any SDK (`FR-006-003`, `US02-AC01`): the server validates the
//! capabilities the worker offers, hands it the task and completes the step through the common protocol.

use serde_json::json;

use crate::harness::{self, Selection, node, observation, worker_log};

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_worker_of_every_sdk_registers_receives_the_task_and_completes_the_step() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["worker-registration"])).await;
        suite.assert_conforms();
        let languages = suite.languages_of("worker-registration");
        assert!(!languages.is_empty());
        for language in languages {
            let doc = suite.doc("worker-registration", language);
            // Given an identity with no binding for the activity, the server refuses its capabilities
            // with the canonical error, which is not retried, and the worker takes no task.
            let refused = &observation(doc, "unbound-refused")["error"];
            assert_eq!(refused["code"], "NOT_BOUND", "{language}: {refused}");
            assert_eq!(refused["retryable"], json!(false), "{language}");
            assert!(
                worker_log(doc, "unbound").is_empty(),
                "{language}: the unbound worker took a task"
            );
            // Given a bound identity, the worker registers at the protocol the server agrees to, receives
            // the task once and the step is completed.
            assert_eq!(
                observation(doc, "bound-registers")["result"]["protocolVersion"],
                "1.2",
                "{language}"
            );
            // Two attempts in its log: the run it served, and (after the restart below) a second one.
            let log = worker_log(doc, "bound");
            assert_eq!(log.len(), 2, "{language}: {log:?}");
            assert_eq!(log[0]["taskKind"], "FORWARD", "{language}");
            assert_eq!(log[0]["attempt"], json!(1), "{language}");
            assert_eq!(log[0]["outcome"], "returned", "{language}");
            assert_eq!(
                observation(doc, "done")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
            // The host is restartable: stopped and started again, it serves a second run.
            assert_eq!(
                observation(doc, "bound-restarts")["result"]["protocolVersion"],
                "1.2",
                "{language}"
            );
            assert_eq!(
                observation(doc, "done-again")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
            let work = node(doc, "work");
            assert_eq!(work["status"], "SUCCEEDED", "{language}");
            assert_eq!(work["attempt"], json!(1), "{language}");
        }
    });
}
