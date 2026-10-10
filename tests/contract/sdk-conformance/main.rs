//! The SDK conformance suite (`FR-006-006`): the same scenarios, played by the SDK of each language
//! against a real platform, must give the same normalized results, equal to the golden files.
//! `README.md` in this directory is the contract with the SDKs; `harness.rs` is the machinery.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

#[path = "harness.rs"]
mod harness;

#[test]
fn every_sdk_conforms_to_every_scenario() {
    harness::block_on(async {
        harness::conform(&harness::Selection::from_env())
            .await
            .assert_conforms()
    });
}
