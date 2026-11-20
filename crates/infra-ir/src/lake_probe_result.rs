//! The result of the probe of an Iceberg connection, read into typed fields and written again (feature 035, D035-150 slice S4a).
//!
//! What a runner reports is not stored. It is read into types that accept only what the contract names: the kind, the connection's own name
//! and revision (which must be the probe's), and each check with an outcome and, for a failed one, a code of the closed vocabulary. Nothing in
//! them can hold text the server did not supply, so nothing a host says (an address, a provider's message, a location, a value) can reach the
//! journal, the API or an audit record through a result. The canonical form is written from the typed fields, so two runners that report the
//! same result store the same bytes.

use sagat_application::database::{ExpectedProbe, InvalidLakeProbe, LakeProbeCodec, RebuiltProbe};
use sagat_domain::database::{
    DatabaseFailure, ProbeCheck, ProbeCheckResult, ProbeOutcome, ProbeResult,
};
use serde::{Deserialize, Serialize};

/// The most a result can be: three small checks and a name. A report over this is not a host's.
const MAX_RESULT_BYTES: usize = 4096;
/// The kind of the report, which is the only one a probe produces.
const KIND: &str = "probe";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResult {
    kind: String,
    connection: WireConnection,
    checks: Vec<WireCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireConnection {
    name: String,
    revision: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCheck {
    check: String,
    outcome: String,
    #[serde(default)]
    code: Option<String>,
}

/// The canonical form, written from the typed fields in this order.
#[derive(Serialize)]
struct Canonical<'a> {
    kind: &'static str,
    connection: CanonicalConnection<'a>,
    checks: Vec<CanonicalCheck>,
}

#[derive(Serialize)]
struct CanonicalConnection<'a> {
    name: &'a str,
    revision: u32,
}

#[derive(Serialize)]
struct CanonicalCheck {
    check: &'static str,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
}

/// The adapter of the port: JSON in, JSON out, and nothing in between that a runner chose.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonLakeProbeCodec;

impl LakeProbeCodec for JsonLakeProbeCodec {
    fn rebuild(
        &self,
        result: &[u8],
        expected: &ExpectedProbe<'_>,
    ) -> Result<RebuiltProbe, InvalidLakeProbe> {
        if result.len() > MAX_RESULT_BYTES {
            return Err(InvalidLakeProbe::TooLarge);
        }
        let wire: WireResult =
            serde_json::from_slice(result).map_err(|_| InvalidLakeProbe::Malformed)?;
        if wire.kind != KIND {
            return Err(InvalidLakeProbe::Kind);
        }
        if wire.connection.name != expected.connection
            || wire.connection.revision != expected.revision
        {
            return Err(InvalidLakeProbe::Mismatch);
        }
        let mut checks = Vec::with_capacity(wire.checks.len());
        for check in &wire.checks {
            let named = ProbeCheck::parse(&check.check).ok_or(InvalidLakeProbe::Malformed)?;
            let outcome = ProbeOutcome::parse(&check.outcome).ok_or(InvalidLakeProbe::Malformed)?;
            let code = match &check.code {
                None => None,
                Some(text) => {
                    Some(DatabaseFailure::parse(text).ok_or(InvalidLakeProbe::Malformed)?)
                }
            };
            checks.push(ProbeCheckResult::new(named, outcome, code));
        }
        let typed = ProbeResult::new(checks).map_err(|_| InvalidLakeProbe::Inconsistent)?;
        let canonical = serde_json::to_vec(&Canonical {
            kind: KIND,
            connection: CanonicalConnection {
                name: expected.connection,
                revision: expected.revision,
            },
            checks: typed
                .checks()
                .iter()
                .map(|check| CanonicalCheck {
                    check: check.check().as_str(),
                    outcome: check.outcome().as_str(),
                    code: check.code().map(DatabaseFailure::code),
                })
                .collect(),
        })
        .map_err(|_| InvalidLakeProbe::Malformed)?;
        Ok(RebuiltProbe {
            canonical,
            result: typed,
        })
    }
}
