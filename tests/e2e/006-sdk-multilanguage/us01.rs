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
            // (the deadline is compared as "so long after the run was created": an hour, not a unit mistake)
            assert_eq!(
                observation(doc, "with-deadline")["result"]["deadline"],
                "<createdAt+3600s>",
                "{language}"
            );
            // The list of the runs that wait is the same when it is read a page at a time.
            assert_eq!(
                observation(doc, "waiting-paged")["result"],
                observation(doc, "waiting")["result"],
                "{language}"
            );
            // The platform holds one run of the key: the run that finished is the one that was started.
            assert_eq!(harness::run_status(doc, "start"), "SUCCEEDED", "{language}");
        }
    });
}

/// US01-AC01 as it is worded: "Given an ASP.NET Core service; When it registers a client via DI and calls
/// StartAsync; Then Run is created with idempotencyKey and trace context propagated." The service is the C# SDK
/// runner's `aspnet` mode (see the conformance README); the platform is the real one.
#[test]
fn an_aspnet_core_service_registers_the_client_through_di_and_starts_a_run_with_its_key_and_the_trace()
 {
    if std::env::var("SAGAT_SDK_LANGS")
        .is_ok_and(|langs| !langs.split(',').any(|l| l.trim() == "dotnet"))
    {
        eprintln!("skipped: SAGAT_SDK_LANGS leaves out dotnet");
        return;
    }
    let scenario = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("e2e/006-sdk-multilanguage/scenarios/us01-aspnet.json");
    harness::block_on(async {
        let failures = harness::ensure_prepared(&["dotnet"]).await;
        assert!(failures.is_empty(), "{failures:?}");
        let out = harness::run_mode(&scenario, "dotnet", "aspnet")
            .await
            .expect("the aspnet mode of the C# runner");
        let sent = &out["sent"];
        let (first, second) = (&out["first"], &out["second"]);
        assert_eq!(first["status"], json!(202), "{out}");
        assert_eq!(second["status"], json!(202), "{out}");
        // The idempotency key makes the second call the run of the first, not another run.
        assert_eq!(first["run"]["runId"], second["run"]["runId"], "{out}");
        assert!(
            first["run"]["runId"]
                .as_str()
                .is_some_and(|id| id.len() == 36),
            "{out}"
        );
        assert_eq!(first["run"]["status"], "RUNNING", "{out}");
        // The trace the request came with is the trace of the run: the SDK took it from the ambient Activity.
        let trace = sent["traceId"].as_str().expect("trace id");
        assert_eq!(trace.len(), 32, "{out}");
        assert_eq!(first["run"]["traceId"], trace, "{out}");
        assert_eq!(second["run"]["traceId"], trace, "{out}");
        assert!(
            sent["traceparent"]
                .as_str()
                .expect("traceparent")
                .contains(trace),
            "{out}"
        );
    });
}
