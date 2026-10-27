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
