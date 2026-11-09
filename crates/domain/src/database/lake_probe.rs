//! The probe of an Iceberg connection, as the payload of a node (feature 035, T035-045, D035-150 slice S1, ADR-0080).
//!
//! A probe tests one connection and writes nothing: it has no table, mapping or records, so it is not a variant of the write. It is the
//! third kind of managed step, and only the platform creates one (a private definition, ADR-0080); no author's definition carries it.

use super::ConnectionRef;

/// The reserved activity of an Iceberg probe. It is on the managed prefix, so an ordinary definition cannot name it without the payload.
pub const ICEBERG_PROBE_ACTIVITY: &str = "sagat.database.lake.probe";

/// What a probe tests: the connection, by name and, when the probe was asked for one revision, by revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LakeProbe {
    connection: ConnectionRef,
}

impl LakeProbe {
    #[must_use]
    pub const fn new(connection: ConnectionRef) -> Self {
        Self { connection }
    }

    #[must_use]
    pub const fn connection(&self) -> &ConnectionRef {
        &self.connection
    }
}
