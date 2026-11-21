//! The lease document of the probe of an Iceberg connection (feature 035, D035-150 slice S3): `sagat.lake.probe-input.v1`.
//!
//! A probe's lease says little, and every member of it is a fact of the server's: which connection (by the name and the revision the probe
//! was admitted with, so that the result echoes them and the server can compare), which checks to run and in which order, and how long one
//! check may take. It carries no secret, no address and no table; the connection's configuration and its two secrets come from
//! `ResolveDatabase`, as they do for a write. The document is pinned by a fixture
//! (`tests/contract/iceberg-writes/fixtures/probe-input/probe.json`) that this builder must produce byte for byte and the runner's reader
//! is held to.

use sagat_domain::database::ProbeCheck;
use serde_json::json;

/// The schema of the document. A change of any member is a new schema.
pub const PROBE_INPUT_SCHEMA: &str = "sagat.lake.probe-input.v1";

/// How long one check may take. The checks together fit inside the 25 s the fixed probe document gives the attempt, so that a typed
/// answer arrives before the run's own deadline.
pub const PROBE_CHECK_TIMEOUT_MS: u32 = 7_000;

/// What the lease of a probe says about its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeFacts<'a> {
    pub connection: &'a str,
    pub revision: u32,
}

/// The document, as the bytes the lease carries. The members are written in the order of the fixture (sorted), so two builds of the same
/// facts are the same bytes.
#[must_use]
pub fn lake_probe_input(facts: &ProbeFacts<'_>) -> Vec<u8> {
    let checks: Vec<&str> = ProbeCheck::ALL.iter().map(|check| check.as_str()).collect();
    json!({
        "schema": PROBE_INPUT_SCHEMA,
        "operation": "probe",
        "connection": {"name": facts.connection, "revision": facts.revision},
        "checks": checks,
        "checkTimeoutMs": PROBE_CHECK_TIMEOUT_MS,
    })
    .to_string()
    .into_bytes()
}
