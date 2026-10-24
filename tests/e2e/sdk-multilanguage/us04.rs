//! US04 — migrating the SDK version (`FR-006-001`, `NFR-006-COMP`, `US04-AC01`): a worker of a previous
//! minor version of the protocol connects to the updated server and is served at its own; one of a newer
//! minor version is served at the server's; another major version is refused with a canonical error.

use serde_json::json;

use crate::harness::{self, Selection, observation, worker_log};

#[test]
#[ignore = "needs Node, Python, Go, .NET and a JDK: the sdks job of CI runs it with --ignored"]
fn workers_of_an_older_and_of_a_newer_minor_version_are_both_served_and_another_major_is_refused() {
    harness::block_on(async {
        let suite = harness::conform(&Selection::scenarios(&["protocol-negotiation"])).await;
        suite.assert_conforms();
        let languages = suite.languages_of("protocol-negotiation");
        assert!(!languages.is_empty());
        for language in languages {
            let doc = suite.doc("protocol-negotiation", language);
            // Given a worker that speaks an older minor version, it is served at that version; one that
            // speaks a newer minor than the server's is served at the server's.
            for (offer, accepted) in [
                ("1", "1"),
                ("1.0", "1"),
                ("1.1", "1.1"),
                ("1.2", "1.2"),
                ("1.3", "1.3"),
                ("1.4", "1.4"),
                ("1.5", "1.5"),
                ("1.9", "1.6"),
            ] {
                let step = observation(doc, &format!("offer-{offer}"));
                assert_eq!(
                    step["result"]["accepted"], accepted,
                    "{language}: offered {offer}"
                );
            }
            // Another major version, or something that is not a version, is a canonical refusal.
            for offer in ["2", "0.9", "junk"] {
                let id = if offer == "junk" {
                    "offer-junk".to_owned()
                } else {
                    format!("offer-{offer}")
                };
                let error = &observation(doc, &id)["error"];
                assert_eq!(
                    error["code"], "UNSUPPORTED_PROTOCOL",
                    "{language}: offered {offer}"
                );
                assert_eq!(
                    error["retryable"],
                    json!(false),
                    "{language}: offered {offer}"
                );
            }
            // A worker at the old version serves a task that outlives the lease: its heartbeats, which
            // carry no step identity at that version, are accepted, and the task is never taken twice.
            assert_eq!(
                observation(doc, "start-old")["result"]["protocolVersion"],
                "1",
                "{language}"
            );
            assert_eq!(
                observation(doc, "done-old")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
            let old = worker_log(doc, "old");
            assert_eq!(old.len(), 1, "{language}: {old:?}");
            assert_eq!(old[0]["attempt"], json!(1), "{language}: the lease lapsed");
            // The same SDK upgraded (offering the newest minor) serves the next run at the server's.
            assert_eq!(
                observation(doc, "start-new")["result"]["protocolVersion"],
                "1.5",
                "{language}"
            );
            assert_eq!(
                observation(doc, "done-new")["result"]["status"],
                "SUCCEEDED",
                "{language}"
            );
        }
    });
}
