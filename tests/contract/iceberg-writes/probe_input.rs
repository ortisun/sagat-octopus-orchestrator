//! The lease document of the probe of an Iceberg connection (feature 035, D035-150 slice S3): `sagat.lake.probe-input.v1`.
//!
//! The document is a fixture the server's builder must produce byte for byte and the runner's reader (slice S6) will be held to. It is the
//! whole of what a probe's lease says, and it says little: which connection (by the name and the revision the probe was admitted with, so
//! that the result can echo them), which checks to run in which order, and how long one check may take. No secret, no address, no table:
//! the connection's configuration and its two secrets come from `ResolveDatabase`, as they do for a write.
use sagat_application::worker::lake_probe_input::{
    PROBE_CHECK_TIMEOUT_MS, PROBE_INPUT_SCHEMA, ProbeFacts, lake_probe_input,
};
use sagat_domain::database::ProbeCheck;
use serde_json::{Value, json};

fn fixture() -> Vec<u8> {
    std::fs::read(super::fixtures().join("probe-input/probe.json")).expect("the fixture")
}

#[test]
fn the_server_builds_the_fixture_byte_for_byte() {
    let built = lake_probe_input(&ProbeFacts {
        connection: "lake-main",
        revision: 3,
    });
    assert_eq!(
        String::from_utf8(built).expect("UTF-8"),
        String::from_utf8(fixture()).expect("UTF-8").trim_end(),
        "the builder and the fixture are the one contract"
    );
}

#[test]
fn the_document_names_the_checks_of_the_vocabulary_in_their_order_and_nothing_the_runner_could_misuse()
 {
    let built: Value = serde_json::from_slice(&lake_probe_input(&ProbeFacts {
        connection: "lake-main",
        revision: 1,
    }))
    .expect("JSON");
    let names: Vec<&str> = ProbeCheck::ALL.iter().map(|check| check.as_str()).collect();
    assert_eq!(built["checks"], json!(names));
    assert_eq!(built["schema"], PROBE_INPUT_SCHEMA);
    assert_eq!(built["operation"], "probe");
    assert_eq!(built["checkTimeoutMs"], PROBE_CHECK_TIMEOUT_MS);
    // the members are exactly these: a member added later is a change of the schema, and this test says so
    let mut members: Vec<&str> = built
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    members.sort_unstable();
    assert_eq!(
        members,
        [
            "checkTimeoutMs",
            "checks",
            "connection",
            "operation",
            "schema"
        ]
    );
    assert_eq!(
        built["connection"],
        json!({"name": "lake-main", "revision": 1})
    );
    let text = built.to_string();
    for never in ["secret", "password", "token", "s3://", "http", "uri", "key"] {
        assert!(
            !text.contains(never),
            "{never} is not in a probe's lease: {text}"
        );
    }
    // the checks together fit inside the attempt's own budget of the fixed probe document (25 s), so a typed answer arrives before the deadline
    let total = u64::from(PROBE_CHECK_TIMEOUT_MS) * ProbeCheck::ALL.len() as u64;
    assert!(
        total < 25_000,
        "{total} ms of checks do not fit in the attempt's 25 s"
    );
}
