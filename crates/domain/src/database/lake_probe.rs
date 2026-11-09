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

/// What a probe checks, a closed vocabulary shared by its lease document and its result (`contracts/connections.md`,
/// `ConnectionProbeResult.checks[]`). The order of [`Self::ALL`] is the order the checks run, and it is part of what a result means: a
/// check that cannot run because an earlier one failed is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProbeCheck {
    /// The catalog accepts the credentials and answers its configuration.
    CatalogAuth,
    /// The warehouse the connection names is there and its locations are the connection's.
    Warehouse,
    /// The storage credentials can list under a warehouse location.
    StorageAccess,
}

impl ProbeCheck {
    /// Every check of an Iceberg probe, in the order they run.
    pub const ALL: [Self; 3] = [Self::CatalogAuth, Self::Warehouse, Self::StorageAccess];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CatalogAuth => "CATALOG_AUTH",
            Self::Warehouse => "WAREHOUSE",
            Self::StorageAccess => "STORAGE_ACCESS",
        }
    }

    /// `None` for anything but the exact name of a check of an Iceberg probe.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|check| check.as_str() == text)
    }
}
