//! The SDK conformance suite (`FR-006-006`): the same scenarios, played by the SDK of each language
//! against a real platform, must give the same normalized results, equal to the golden files.
//! `README.md` in this directory is the contract with the SDKs; `harness.rs` is the machinery.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

#[path = "harness.rs"]
mod harness;

#[test]
#[ignore = "needs the six SDK toolchains and Docker: the sdks job of CI runs it with --ignored"]
fn every_sdk_conforms_to_every_scenario() {
    harness::block_on(async {
        harness::conform(&harness::Selection::from_env())
            .await
            .assert_conforms()
    });
}

/// Catch incomplete scenario registration without first booting five language/platform stacks.
/// Builder-only contracts have a canonical document; every runtime scenario also needs its
/// separately reviewed result golden. A synthetic authoring artifact is never silently executed.
#[test]
fn every_scenario_declares_phases_with_reviewed_expected_documents() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contract/sdk-conformance");
    let mut count = 0;
    for entry in std::fs::read_dir(root.join("scenarios")).expect("scenario directory") {
        let path = entry.expect("scenario entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let scenario: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("scenario bytes"))
                .expect("scenario JSON");
        let name = scenario["name"].as_str().expect("scenario name");
        assert_eq!(path.file_stem().and_then(|stem| stem.to_str()), Some(name));
        let build_only = scenario
            .get("buildOnly")
            .map(|value| value.as_bool().expect("buildOnly must be a boolean"))
            .unwrap_or(false);
        if let Some(rejections) = scenario.get("publicationRejections") {
            assert!(
                !build_only,
                "{name}: publication refusals require the server phase"
            );
            for (workflow, reason) in rejections
                .as_object()
                .expect("publicationRejections object")
            {
                assert!(
                    scenario["workflows"].get(workflow).is_some(),
                    "{name}: unknown rejected workflow {workflow}"
                );
                assert!(
                    reason.as_str().is_some_and(|reason| !reason.is_empty()),
                    "{name}: invalid publication reason"
                );
            }
        }
        // A runtime scenario written before the server can play it names why and the item that will let it run;
        // it has its builder golden and no result golden, and the harness reports it as a failure, never a pass.
        let pending = scenario.get("pending").map(|reason| {
            let reason = reason.as_str().expect("pending must be a string");
            assert!(
                !build_only
                    && (reason.contains("T0") || reason.contains("G0") || reason.contains("G1")),
                "{name}: a pending scenario is a runtime one and names the task or gap it waits for"
            );
        });
        // A check of the contract the scenario cannot make yet is named with the task or gap it waits for (D022-097).
        for (check, reason) in scenario
            .get("deferred")
            .map(|deferred| deferred.as_object().expect("deferred is an object"))
            .into_iter()
            .flatten()
        {
            let reason = reason
                .as_str()
                .expect("a deferred check's reason is a string");
            assert!(
                !check.is_empty()
                    && (reason.contains("T0") || reason.contains("G0") || reason.contains("G1")),
                "{name}: the deferred check {check:?} names the task or gap it waits for"
            );
        }
        let mut expected = vec![format!("{name}.workflows.json")];
        if !build_only && pending.is_none() {
            expected.push(format!("{name}.json"));
        }
        for filename in expected {
            let bytes =
                std::fs::read(root.join("expected").join(&filename)).unwrap_or_else(|error| {
                    panic!("{name}: missing reviewed phase evidence {filename}: {error}")
                });
            let expected: serde_json::Value =
                serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                    panic!("{name}: invalid reviewed phase evidence {filename}: {error}")
                });
            if filename.ends_with(".workflows.json") {
                for (workflow, source) in scenario["workflows"].as_object().into_iter().flatten() {
                    if let Some(limits) = source.get("limits") {
                        assert_eq!(
                            &expected["workflows"][workflow]["limits"], limits,
                            "{name}/{workflow}: reviewed document must preserve the authored limits"
                        );
                        let canonical: serde_json::Value = serde_json::from_str(
                            expected["canonical"][workflow]
                                .as_str()
                                .expect("reviewed canonical text"),
                        )
                        .expect("reviewed canonical JSON");
                        assert_eq!(
                            &canonical["limits"], limits,
                            "{name}/{workflow}: reviewed canonical text must preserve the authored limits"
                        );
                    }
                }
            }
        }
        count += 1;
    }
    assert!(count > 0, "the registry must not be empty");
}

#[test]
fn compensation_safety_scenario_covers_the_shared_delivery_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contract/sdk-conformance");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(
            &std::fs::read(root.join("scenarios").join(name)).expect("scenario bytes"),
        )
        .expect("scenario JSON")
    };
    let scenario = read("compensation-safety.json");
    assert_eq!(scenario["reviewEvidence"], true);
    let steps = scenario["steps"].as_array().expect("steps");
    for (action, expected) in [
        ("RETRY_STEP", 2),
        ("RETRY_COMPENSATION", 1),
        ("RESOLVE_STEP", 3),
    ] {
        assert_eq!(
            steps.iter().filter(|step| step["action"] == action).count(),
            expected,
            "{action} coverage"
        );
    }
    assert!(steps.iter().any(|step| step["op"] == "cancel"));
    assert_eq!(
        scenario["workflows"]["known-compensation-failure"]["program"][0]["options"]["onCompensationFailure"],
        "fail"
    );

    let legacy = read("saga-compensation.json");
    assert!(
        legacy["workers"]["w1"]["activities"]["saga.pay2"]["behavior"]["failsUntilAttempt"]
            .is_null()
    );
    assert!(legacy["steps"].as_array().is_some_and(|steps| {
        steps.iter().any(|step| {
            step["id"] == "not-applied-resolved"
                && step["action"] == "RESOLVE_STEP"
                && step["outcome"] == "SUCCEEDED"
        })
    }));
}

/// Feature 022 (FR-022-035, FR-022-050, T022-048..052, T022-067, T022-083): the reviewed golden of the six SDKs'
/// flow-control builders is the shared fixture corpus where it names a fixture, and every document in it is what the
/// server's codec publishes: its canonical text is the server's own and it decodes to the gates the scenario declares.
#[test]
fn flow_control_builder_golden_is_the_shared_fixtures_and_the_server_canonical_definition() {
    use sagat_application::definitions::IrCodec;
    use sagat_infra_ir::JsonIrCodec;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: std::path::PathBuf| -> serde_json::Value {
        serde_json::from_slice(
            &std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
        )
        .expect("JSON")
    };
    let suite = root.join("contract/sdk-conformance");
    let golden = read(suite.join("expected/flow-control-builders.workflows.json"));
    let scenario = read(suite.join("scenarios/flow-control-builders.json"));
    let workflows = golden["workflows"].as_object().expect("workflow documents");
    assert_eq!(
        workflows.keys().collect::<Vec<_>>(),
        scenario["workflows"]
            .as_object()
            .expect("scenario workflows")
            .keys()
            .collect::<Vec<_>>()
    );
    for fixture in ["provider-sync", "account-serial"] {
        let shared = read(root.join(format!(
            "contract/flow-control-step-policies/fixtures/{fixture}.workflow.json"
        )));
        assert_eq!(
            golden["workflows"][fixture], shared,
            "{fixture}: the SDKs do not write the shared fixture"
        );
    }
    for (name, document) in workflows {
        let parsed = JsonIrCodec
            .parse(&serde_json::to_vec(document).expect("workflow JSON"))
            .unwrap_or_else(|error| {
                panic!("{name}: the server refuses the SDK document: {error:?}")
            });
        assert_eq!(
            String::from_utf8(parsed.canonical).expect("canonical UTF-8"),
            golden["canonical"][name]
                .as_str()
                .expect("reviewed canonical text"),
            "{name}: SDK and server canonical forms differ"
        );
        let declared = &scenario["workflows"][name];
        let definition = &parsed.definition;
        let run = definition.run_flow_control();
        assert_eq!(
            run.concurrency().map(|policy| u64::from(policy.limit())),
            declared["concurrency"]["limit"].as_u64(),
            "{name}: run-level concurrency"
        );
        assert_eq!(
            run.rate_limit().map(|policy| u64::from(policy.limit())),
            declared["rateLimit"]["limit"].as_u64(),
            "{name}: run-level rate limit"
        );
        let mut gated: Vec<(String, Option<u64>, Option<u64>)> = definition
            .gated_nodes()
            .map(|node| {
                let gates = node.flow_control();
                (
                    node.id().as_str().to_owned(),
                    gates.concurrency().map(|policy| u64::from(policy.limit())),
                    gates.rate_limit().map(|policy| u64::from(policy.limit())),
                )
            })
            .collect();
        gated.sort();
        let mut expected: Vec<(String, Option<u64>, Option<u64>)> = declared["program"]
            .as_array()
            .expect("program")
            .iter()
            .filter(|op| {
                op["options"].get("concurrency").is_some()
                    || op["options"].get("rateLimit").is_some()
            })
            .map(|op| {
                (
                    op["id"].as_str().expect("id").to_owned(),
                    op["options"]["concurrency"]["limit"].as_u64(),
                    op["options"]["rateLimit"]["limit"].as_u64(),
                )
            })
            .collect();
        expected.sort();
        assert_eq!(gated, expected, "{name}: gated nodes");
    }
}

mod flow_control_assertions {
    //! The independent assertions of `flow-concurrency` and `flow-rate-limit` (feature 022) on journals made up here:
    //! each check must refuse the journal that breaks it, so that a pass of the scenario means what it says.
    use super::harness::flow_control::{GateEvent, check_concurrency, check_rate, check_window};
    use std::collections::BTreeMap;

    fn event(run: &str, seq: i64, kind: &str, key: &str, at_us: i64) -> GateEvent {
        GateEvent {
            run: run.into(),
            seq,
            kind: kind.into(),
            node: None,
            scope: Some("RUN".into()),
            group: Some("flow-serial".into()),
            key: Some(key.into()),
            at_us,
            granted_at_us: None,
            from_queue: None,
        }
    }

    fn grant(run: &str, key: &str, at_us: i64, from_queue: bool) -> GateEvent {
        GateEvent {
            granted_at_us: Some(at_us),
            from_queue: Some(from_queue),
            ..event(run, 2, "rate_limit.granted", key, at_us)
        }
    }

    fn lease(run: &str, at_us: i64) -> GateEvent {
        GateEvent {
            node: Some("fetch".into()),
            scope: None,
            group: None,
            key: None,
            ..event(run, 3, "step.leased", "", at_us)
        }
    }

    /// One holder of key A at a time, granted in the order queued; key B alongside.
    fn serial() -> Vec<GateEvent> {
        vec![
            event("a1", 1, "concurrency.acquired", "A", 10),
            event("a2", 1, "concurrency.queued", "A", 20),
            event("a4", 1, "concurrency.queued", "A", 30),
            event("b1", 1, "concurrency.acquired", "B", 40),
            event("a1", 2, "concurrency.released", "A", 100),
            event("a2", 2, "concurrency.acquired", "A", 100),
            event("b1", 2, "concurrency.released", "B", 110),
            event("a2", 3, "concurrency.released", "A", 200),
            event("a4", 2, "concurrency.acquired", "A", 250),
            event("a4", 3, "concurrency.released", "A", 300),
        ]
    }

    #[test]
    fn a_journal_that_keeps_the_limit_and_the_order_passes() {
        assert_eq!(check_concurrency(&serial(), 1), Vec::<String>::new());
    }

    #[test]
    fn two_holders_of_one_key_at_once_are_refused() {
        let mut events = serial();
        events[5].at_us = 99; // a2 acquired before a1 released
        let failures = check_concurrency(&events, 1);
        assert!(
            failures
                .iter()
                .any(|f| f.contains("2 runs held a permit at once")),
            "{failures:?}"
        );
        assert_eq!(
            check_concurrency(&events, 2),
            Vec::<String>::new(),
            "within a limit of two"
        );
    }

    #[test]
    fn a_grant_out_of_admission_order_is_refused() {
        let mut events = serial();
        events[2].at_us = 15; // a4 queued before a2, still granted after it
        let failures = check_concurrency(&events, 1);
        assert!(
            failures
                .iter()
                .any(|f| f.contains("was queued before run a4")
                    || f.contains("queued before run a2")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_sliding_window_counts_from_every_instant() {
        assert!(check_window("x", &[0, 999_999, 1_000_000, 1_999_999], 2, 1_000_000).is_empty());
        assert_eq!(
            check_window("x", &[0, 500_000, 999_999], 2, 1_000_000).len(),
            1
        );
        // three in the window that starts at 600 ms, though no aligned second holds more than two
        assert_eq!(
            check_window("x", &[1_200_000, 600_000, 900_000], 2, 1_000_000).len(),
            1
        );
    }

    fn buffered() -> Vec<GateEvent> {
        vec![
            grant("p1", "P", 0, false),
            grant("p2", "P", 10, false),
            event("p3", 1, "rate_limit.queued", "P", 20),
            event("p4", 1, "rate_limit.queued", "P", 30),
            event("p5", 1, "rate_limit.queued", "P", 40),
            grant("q1", "Q", 50, false),
            event("p5", 2, "rate_limit.dequeued", "P", 60),
            grant("p3", "P", 60_000_000, true),
            grant("p4", "P", 60_000_010, true),
            lease("p1", 100),
            lease("p2", 1_000_100),
            lease("q1", 2_000_100),
            lease("p3", 60_000_100),
            lease("p4", 61_000_100),
        ]
    }

    fn fetch() -> BTreeMap<String, (usize, i64)> {
        BTreeMap::from([("fetch".to_owned(), (1, 1_000_000))])
    }

    #[test]
    fn a_journal_within_both_windows_passes() {
        assert_eq!(
            check_rate(&buffered(), (2, 60_000_000), &fetch()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_third_grant_in_one_window_and_a_second_lease_in_one_second_are_refused() {
        let mut events = buffered();
        events[7].granted_at_us = Some(59_999_999);
        events[10].at_us = 999_999;
        let failures = check_rate(&events, (2, 60_000_000), &fetch());
        assert!(
            failures
                .iter()
                .any(|f| f.starts_with("run grants of group")),
            "{failures:?}"
        );
        assert!(
            failures.iter().any(|f| f.starts_with("leases of fetch")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_grant_that_says_it_queued_when_it_did_not_is_refused_and_the_buffer_keeps_its_order() {
        let mut events = buffered();
        events[0].from_queue = Some(true);
        events[7].granted_at_us = Some(60_000_020); // p3 granted after p4
        let failures = check_rate(&events, (2, 60_000_000), &fetch());
        assert!(
            failures
                .iter()
                .any(|f| f.contains("run p1: granted with from_queue Some(true) but never queued")),
            "{failures:?}"
        );
        assert!(
            failures
                .iter()
                .any(|f| f.contains("buffered before run p4 but granted after it")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_dequeued_run_that_was_granted_is_refused() {
        let mut events = buffered();
        events.push(grant("p5", "P", 120_000_000, true));
        let failures = check_rate(&events, (2, 60_000_000), &fetch());
        assert!(
            failures
                .iter()
                .any(|f| f.contains("run p5 was dequeued and granted")),
            "{failures:?}"
        );
    }
}
