//! US01 — an application starts a run from an SDK (`FR-006-002`, `FR-006-005`, `US01-AC01`): the run is
//! created with the idempotency key, the correlation id, the deadline and the trace context the caller
//! gave, a repeat of the call is the same run, and a different request under the same key is refused.

use serde_json::json;

use crate::harness::{self, Selection, observation, worker_log};

#[test]
fn every_sdk_creates_the_run_with_its_idempotency_key_and_the_trace_context_it_was_given() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["run-lifecycle"])).await;
        suite.assert_conforms();
        let languages = suite.languages_of("run-lifecycle");
        assert!(!languages.is_empty());
        for language in languages {
            let doc = suite.doc("run-lifecycle", language);
            let start = &observation(doc, "start")["result"];
            // Given a caller with a W3C traceparent, a correlation id and an idempotency key, the run
            // carries the caller's trace and correlation, and so does the worker that serves it.
            assert_eq!(start["traceId"], "<given-trace>", "{language}: {start}");
            assert_eq!(start["correlationId"], "corr-{{unique}}", "{language}");
            assert_eq!(start["status"], "RUNNING", "{language}");
            let first = &worker_log(doc, "w1")[0];
            assert_eq!(
                first["traceId"], "<given-trace>",
                "{language}: the trace reaches the activity"
            );
            assert_eq!(first["traceparentPresent"], json!(true), "{language}");
            // A repeat of the call under the same key is the same run, not a second one.
            let repeat = &observation(doc, "duplicate")["result"];
            assert_eq!(repeat["runId"], start["runId"], "{language}");
            // The same key for another request is refused with the canonical, non-retryable conflict.
            let changed = &observation(doc, "changed")["error"];
            assert_eq!(changed["code"], "CONFLICT", "{language}: {changed}");
            assert_eq!(changed["reason"], "IDEMPOTENCY_KEY_REUSED", "{language}");
            assert_eq!(changed["retryable"], json!(false), "{language}");
            // The deadline the caller gave is the run's.
            assert_eq!(
                observation(doc, "with-deadline")["result"]["deadline"],
                "<time>",
                "{language}"
            );
            // The platform holds one run of the key: the run that finished is the one that was started.
            assert_eq!(harness::run_status(doc, "start"), "SUCCEEDED", "{language}");
        }
    });
}
