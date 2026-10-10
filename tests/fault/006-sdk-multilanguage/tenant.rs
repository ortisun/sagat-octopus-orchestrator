//! The tenant boundary and the credentials, seen through the SDKs (`NFR-006-SEC`, `SC-006-004`): another
//! tenant's run does not exist, a token the platform refuses is the canonical error, an identity with no
//! binding cannot register a worker, and none of the errors repeats a token or a payload (the harness
//! fails the scenario when one does).

use serde_json::json;

use crate::harness::{self, Selection, observation};

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn another_tenants_run_does_not_exist_and_a_bad_token_is_refused_with_the_canonical_errors() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["run-lifecycle", "errors"])).await;
        suite.assert_conforms();
        for scenario in ["run-lifecycle", "errors"] {
            for language in suite.languages_of(scenario) {
                let doc = suite.doc(scenario, language);
                let other = &observation(doc, "other-tenant")["error"];
                // Not FORBIDDEN: the run of another tenant is not there to be forbidden.
                assert_eq!(other["code"], "NOT_FOUND", "{scenario}/{language}: {other}");
                assert_eq!(other["status"], json!(404), "{scenario}/{language}");
                let bad = &observation(doc, "no-token")["error"];
                assert_eq!(
                    bad["code"], "UNAUTHENTICATED",
                    "{scenario}/{language}: {bad}"
                );
                assert_eq!(bad["status"], json!(401), "{scenario}/{language}");
                for error in [other, bad] {
                    assert!(
                        !error["remediation"].as_str().unwrap_or_default().is_empty(),
                        "{scenario}/{language}: an error without remediation"
                    );
                }
            }
        }
    });
}

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn a_worker_identity_with_no_binding_cannot_register_its_activities() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["worker-registration"])).await;
        suite.assert_conforms();
        for language in suite.languages_of("worker-registration") {
            let doc = suite.doc("worker-registration", language);
            let refused = &observation(doc, "unbound-refused")["error"];
            assert_eq!(refused["code"], "NOT_BOUND", "{language}: {refused}");
            assert_eq!(refused["retryable"], json!(false), "{language}");
        }
    });
}
