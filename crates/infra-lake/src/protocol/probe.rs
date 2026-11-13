//! The probe of an Iceberg connection on the wire (feature 035, D035-150, slice S6a): the lease, the request line's body and
//! the result.
//!
//! A probe is the one operation of the lake host that writes nothing and has no table, so it has no effect key, no instant
//! of a first attempt, no records and no commit. What it has is a connection, which the server names by its name and its
//! revision (the connection and its two secrets come from `ResolveDatabase`, as a write's do) and the time one check may
//! take.
//!
//! - [`ProbeInput`] reads the **lease** (`sagat.lake.probe-input.v1`) as strictly as the write's reader reads its own:
//!   one object of at most [`MAX_PROBE_INPUT_BYTES`], no name twice at any depth, every member present and of its own type
//!   and bounds, no member the contract does not name. The server's builder writes it byte for byte
//!   (`fixtures/probe-input/probe.json`) and `fixtures/probe-input/refused.json` is every way a document is not that.
//! - [`ProbeBody`] is the **body of the request line**: the pinned connection with its resolved secrets and, in a member of
//!   its own because the lease's `connection` is a name and the request's is a document, the lease's members (`probe`).
//!   The Python host reads exactly this and refuses any other member (`parse_probe`).
//! - [`ProbeReport`] is the **result** the host gives, read into typed fields by the rules of the server's
//!   `ProbeResult::new` and written again as the server writes it, so that nothing a host says (an address, a provider's
//!   message, a value of the request) reaches the journal through it. The server reads the same bytes strictly once more
//!   and compares the connection and the revision with the probe's own; [`ProbeReport::answers`] is the same comparison
//!   made here, so a host that answers for another connection never costs the server a refusal.

use std::fmt;

use sagat_domain::database::{
    DatabaseFailure, ProbeCheck, ProbeCheckResult, ProbeOutcome, ProbeResult,
};
use sagat_domain::identity::Slug;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::attempt_input::{InputError, Members, Strict};
use super::{Body, BodyError, Connection};

/// The longest lease the reader takes. The document is a name, a number, a list of three words and a limit.
pub const MAX_PROBE_INPUT_BYTES: usize = 4096;

/// The schema this reader serves; another version is a different contract and not a variation of this one.
const SCHEMA: &str = "sagat.lake.probe-input.v1";
const OPERATION: &str = "probe";
/// What one check may take, in milliseconds: the same bounds the Python host reads (`request.py`).
const CHECK_TIMEOUT_MS: (u64, u64) = (100, 60_000);
/// The longest result the server reads (`MAX_RESULT_BYTES` of `crates/infra-ir/src/lake_probe_result.rs`).
const MAX_RESULT_BYTES: usize = 4096;
/// The kind of the result, which is the only one a probe produces.
const KIND: &str = "probe";

/// The probe the server leased: whose it is and how long one check may take. Nothing else, and no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeInput {
    connection: String,
    revision: u32,
    check_timeout_ms: u32,
}

impl ProbeInput {
    /// The name of the connection the probe was admitted with.
    #[must_use]
    pub fn connection(&self) -> &str {
        &self.connection
    }

    /// The revision of the connection the probe was admitted with, which the result echoes.
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// The time one check may take, in milliseconds.
    #[must_use]
    pub const fn check_timeout_ms(&self) -> u32 {
        self.check_timeout_ms
    }

    /// Reads the `input_json` of a probe lease.
    ///
    /// # Errors
    /// [`InputError`] for the first part that is not what the contract says. The error names a part and never a value.
    pub fn read(bytes: &[u8]) -> Result<Self, InputError> {
        if bytes.len() > MAX_PROBE_INPUT_BYTES {
            return Err(InputError::Document);
        }
        let Strict(document) = serde_json::from_slice(bytes).map_err(|_| InputError::Document)?;
        let Value::Object(_) = document else {
            return Err(InputError::Document);
        };
        let mut members = Members::of(document, "document")?;

        if members.text("schema")? != SCHEMA {
            return Err(InputError::Member("schema"));
        }
        if members.text("operation")? != OPERATION {
            return Err(InputError::Member("operation"));
        }
        let check_timeout_ms = members.number("checkTimeoutMs", CHECK_TIMEOUT_MS)?;
        // the three checks, once each, in the order they run: nothing else is served
        let checks_are_the_three = matches!(
            members.take("checks")?,
            Value::Array(checks)
                if checks.len() == ProbeCheck::ALL.len()
                    && checks
                        .iter()
                        .zip(ProbeCheck::ALL)
                        .all(|(item, check)| item.as_str() == Some(check.as_str()))
        );
        if !checks_are_the_three {
            return Err(InputError::Member("checks"));
        }
        let mut connection = Members::of(members.take("connection")?, "connection")?;
        let name = connection.text("name")?;
        Slug::parse(&name).map_err(|_| InputError::Member("name"))?;
        let revision = connection.number("revision", (1, u64::from(u32::MAX)))?;
        connection.finish()?;
        members.finish()?;

        Ok(Self {
            connection: name,
            revision,
            check_timeout_ms,
        })
    }
}

/// The body of a probe's request line: the pinned connection, with its resolved secrets, and the probe the host is asked to
/// run. Nothing of the connection is shown by [`fmt::Debug`].
#[derive(Clone)]
pub struct ProbeBody {
    connection: Connection,
    name: String,
    revision: u32,
    check_timeout_ms: u32,
}

impl ProbeBody {
    #[must_use]
    pub fn new(connection: Connection, input: &ProbeInput) -> Self {
        Self {
            connection,
            name: input.connection.clone(),
            revision: input.revision,
            check_timeout_ms: input.check_timeout_ms,
        }
    }

    /// Asks the host for less time per check than the lease names. The attempt may have less time than three of the lease's
    /// checks (a connection that allows less, or a connection that took long to resolve), and a host that is given a time
    /// that does not fit answers with the attempt's end and not with a typed timeout of the check. Nothing raises the time.
    #[must_use]
    pub const fn with_check_timeout_ms(mut self, milliseconds: u32) -> Self {
        self.check_timeout_ms = milliseconds;
        self
    }

    /// The members of the request that are not the envelope.
    ///
    /// # Errors
    /// [`BodyError::Connection`] for a location or a secret the host would refuse, and [`BodyError::Limits`] for a time of a
    /// check outside what the host reads (100 to 60 000 milliseconds).
    pub fn into_body(self) -> Result<Body, BodyError> {
        let (low, high) = CHECK_TIMEOUT_MS;
        if !(low..=high).contains(&u64::from(self.check_timeout_ms)) {
            return Err(BodyError::Limits);
        }
        let connection = self.connection.to_value()?;
        let probe = json!({
            "connection": {"name": self.name, "revision": self.revision},
            "checks": ProbeCheck::ALL.map(ProbeCheck::as_str),
            "checkTimeoutMs": self.check_timeout_ms,
        });
        let mut members = Map::new();
        members.insert("connection".to_owned(), connection);
        members.insert("probe".to_owned(), probe);
        Ok(Body::from(members))
    }
}

/// The probe carries a connection with secrets, so only whose probe it is is shown.
impl fmt::Debug for ProbeBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProbeBody")
            .field("connection", &self.name)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireReport {
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

/// The canonical form, written from the typed fields in this order: the one the server stores.
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

/// What a host says of a connection it probed, as typed fields: the connection's own name and revision, and each check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    connection: String,
    revision: u32,
    result: ProbeResult,
}

impl ProbeReport {
    /// The receipt member of a result line whose `kind` is `probe`, or `None` when it does not hold as typed fields.
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        // a result is three small checks and a name: a report over what the server reads is not a host's
        if serde_json::to_vec(value).ok()?.len() > MAX_RESULT_BYTES {
            return None;
        }
        let wire: WireReport = serde_json::from_value(value.clone()).ok()?;
        if wire.kind != KIND {
            return None;
        }
        let mut checks = Vec::with_capacity(wire.checks.len());
        for check in &wire.checks {
            let named = ProbeCheck::parse(&check.check)?;
            let outcome = ProbeOutcome::parse(&check.outcome)?;
            let code = match &check.code {
                None => None,
                Some(text) => Some(DatabaseFailure::parse(text)?),
            };
            checks.push(ProbeCheckResult::new(named, outcome, code));
        }
        Some(Self {
            connection: wire.connection.name,
            revision: wire.connection.revision,
            result: ProbeResult::new(checks).ok()?,
        })
    }

    /// The name of the connection the host says it probed.
    #[must_use]
    pub fn connection(&self) -> &str {
        &self.connection
    }

    /// The revision of the connection the host says it probed.
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// The checks, which satisfy the rules of the domain's [`ProbeResult`].
    #[must_use]
    pub const fn result(&self) -> &ProbeResult {
        &self.result
    }

    /// Whether this is the result of the probe that was leased: the connection and the revision are its own.
    #[must_use]
    pub fn answers(&self, input: &ProbeInput) -> bool {
        self.connection == input.connection && self.revision == input.revision
    }

    /// The result as the server stores it, written from the typed fields and never from what the host sent. This is what
    /// `CompleteStep` carries.
    #[must_use]
    pub fn output_json(&self) -> Vec<u8> {
        let canonical = Canonical {
            kind: KIND,
            connection: CanonicalConnection {
                name: &self.connection,
                revision: self.revision,
            },
            checks: self
                .result
                .checks()
                .iter()
                .map(|check| CanonicalCheck {
                    check: check.check().as_str(),
                    outcome: check.outcome().as_str(),
                    code: check.code().map(DatabaseFailure::code),
                })
                .collect(),
        };
        // three words and a number: this cannot fail, and an empty report would be refused by the server, never stored
        serde_json::to_vec(&canonical).unwrap_or_default()
    }
}
