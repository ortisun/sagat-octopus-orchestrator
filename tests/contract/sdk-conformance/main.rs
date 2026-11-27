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
        let mut expected = vec![format!("{name}.workflows.json")];
        if !build_only {
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

/// Design 041, task T041-14: the search behind `mustNotStore` finds a canary planted in any one of the sources it
/// reads, names that source alone, and never repeats the canary whole.
#[test]
fn must_not_store_finds_a_planted_canary_in_every_source() {
    let canary = "canary-planted-4Rk9".to_owned();
    let names = [
        "the stored lines",
        "the bodies of the step-log routes",
        "the runner's output",
        "the platform's state",
        "the audit trail",
        "the server's output",
    ];
    let clean = |n: &str| (n.to_owned(), format!("{n}: [REDACTED] and nothing else"));
    let none: Vec<(String, String)> = names.iter().map(|n| clean(n)).collect();
    assert!(
        harness::step_logs::stored_appearances(&none, std::slice::from_ref(&canary)).is_empty()
    );
    for planted in names {
        let sources: Vec<(String, String)> = names
            .iter()
            .map(|n| {
                if *n == planted {
                    ((*n).to_owned(), format!("before {canary} after"))
                } else {
                    clean(n)
                }
            })
            .collect();
        let found = harness::step_logs::stored_appearances(&sources, std::slice::from_ref(&canary));
        assert_eq!(found.len(), 1, "{planted}: {found:?}");
        assert!(found[0].starts_with(planted), "{found:?}");
        assert!(
            !found[0].contains(&canary),
            "the report repeats the canary: {found:?}"
        );
    }
}

/// Design 041, decision D041-162: a pending language passes only by refusing the first step-log operation as its
/// runner refuses an unknown operation today, and fails once it stops refusing it.
#[test]
fn a_pending_language_passes_only_by_its_runners_refusal() {
    use harness::step_logs::pending_refusal;
    use serde_json::json;
    let scenario =
        json!({"steps": [{"id": "run", "op": "start"}, {"id": "logs", "op": "stepLogs"}]});
    let recorded = |class: &str| {
        json!({"observations": [
            {"id": "run", "op": "start", "ok": true},
            {"id": "logs", "op": "stepLogs", "ok": false, "error": {"code": "UNEXPECTED", "message": class}},
        ]})
    };
    let served = json!({"observations": [{"id": "logs", "op": "stepLogs", "ok": true, "result": {"lines": []}}]});
    let ok = |stderr: &str| Ok((String::new(), stderr.to_owned()));
    for (lang, class, stderr) in [
        ("typescript", "Error", ""),
        (
            "python",
            "ValueError",
            "ValueError: unknown step op stepLogs",
        ),
        ("dotnet", "InvalidOperationException", ""),
        ("java", "IllegalArgumentException", ""),
    ] {
        assert_eq!(
            pending_refusal(lang, &scenario, &ok(stderr), Some(&recorded(class))),
            Ok(()),
            "{lang}"
        );
        assert!(
            pending_refusal(lang, &scenario, &ok(stderr), Some(&served)).is_err(),
            "{lang} served the op"
        );
        assert!(
            pending_refusal(lang, &scenario, &ok(stderr), Some(&recorded("Other"))).is_err(),
            "{lang}"
        );
        assert!(
            pending_refusal(lang, &scenario, &Err("crashed".into()), None).is_err(),
            "{lang} crashed"
        );
    }
    assert!(pending_refusal("python", &scenario, &ok(""), Some(&recorded("ValueError"))).is_err());
    let failed = |text: &str| Err(format!("run.sh exited with 1:\n{text}"));
    assert_eq!(
        pending_refusal(
            "go",
            &scenario,
            &failed("step logs: unknown step op \"stepLogs\""),
            None
        ),
        Ok(())
    );
    assert!(
        pending_refusal(
            "go",
            &scenario,
            &failed("step run: unknown step op \"other\""),
            None
        )
        .is_err()
    );
    assert!(pending_refusal("go", &scenario, &ok(""), Some(&served)).is_err());
    assert_eq!(
        pending_refusal(
            "rust",
            &scenario,
            &failed("Rust runtime scenario operation stepLogs is not implemented yet"),
            None
        ),
        Ok(())
    );
    assert!(pending_refusal("rust", &scenario, &ok(""), Some(&served)).is_err());
    // The first new operation decides: here a release of a held answer comes before the read.
    let release_first = json!({"steps": [{"id": "release", "op": "releaseAnswer"}, {"id": "logs", "op": "stepLogs"}]});
    assert_eq!(
        pending_refusal(
            "go",
            &release_first,
            &failed("unknown step op \"releaseAnswer\""),
            None
        ),
        Ok(())
    );
    assert!(
        harness::step_logs::pending_languages(&json!({"pendingLanguages": ["csharp"]})).is_err()
    );
}

/// Design 041, task T041-14: the wait after a held answer is compared by the side of the 2-second wait before a
/// report it fell on, and a line's exception by the scenario's texts it holds.
#[test]
fn held_answers_and_exceptions_are_compared_by_what_every_language_shares() {
    use serde_json::json;
    let mut normalizer = harness::normalize::Normalizer::new("", Vec::new());
    let held = harness::normalize::compared(
        &json!({"heldAnswers": [
            {"reportSent": "afterRelease", "afterHoldMs": 1999},
            {"reportSent": "beforeRelease", "afterHoldMs": 2000},
        ]}),
        &mut normalizer,
    );
    assert_eq!(
        held["heldAnswers"],
        json!([
            {"reportSent": "afterRelease", "afterHoldMs": "under2000"},
            {"reportSent": "beforeRelease", "afterHoldMs": "atLeast2000"},
        ])
    );
    let mut out = json!({"observations": [{"op": "stepLogs", "result": {"lines": [
        {"exception": "java.lang.RuntimeException: failure [REDACTED]\n\tat Runner.main(Runner.java:1)"},
        {"exception": null},
    ]}}]});
    harness::step_logs::compare_exceptions_by_texts(
        &mut out,
        &["failure [REDACTED]".to_owned(), "absent".to_owned()],
    );
    assert_eq!(
        out["observations"][0]["result"]["lines"],
        json!([{"exception": ["failure [REDACTED]"]}, {"exception": null}])
    );
}
