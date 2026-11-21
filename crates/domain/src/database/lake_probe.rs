//! The probe of an Iceberg connection, as the payload of a node (feature 035, T035-045, D035-150 slice S1, ADR-0080).
//!
//! A probe tests one connection and writes nothing: it has no table, mapping or records, so it is not a variant of the write. It is the
//! third kind of managed step, and only the platform creates one (a private definition, ADR-0080); no author's definition carries it.

use super::{ConnectionRef, DatabaseFailure};

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

    /// Whether the check may fail with `code`. The codes are the ones that already exist (the result invents none, D035-150): which side failed
    /// is said by the check, and the code says why, so a code that names the other side is not one this check can fail with. A check that
    /// exceeded its time says `DATABASE_TIMEOUT`.
    #[must_use]
    pub const fn allows(self, code: DatabaseFailure) -> bool {
        use DatabaseFailure::{
            Authentication, CatalogUnavailable, DestinationDenied, IcebergLocationNotAllowed,
            StorageUnavailable, Timeout, Tls,
        };
        match self {
            Self::CatalogAuth => matches!(
                code,
                Authentication | Tls | DestinationDenied | CatalogUnavailable | Timeout
            ),
            Self::Warehouse => matches!(
                code,
                Authentication | CatalogUnavailable | IcebergLocationNotAllowed | Timeout
            ),
            Self::StorageAccess => matches!(
                code,
                Authentication | Tls | DestinationDenied | StorageUnavailable | Timeout
            ),
        }
    }
}

/// How one check ended. A check is the answer, not a fault of the platform: `Failed` is a result that says the connection does not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeOutcome {
    Passed,
    Failed,
    /// The check did not run: an earlier one failed, or (the storage check) the connection vends its credentials, which nothing does
    /// without a table. A skipped check is **not confirmed**, and never passed.
    Skipped,
}

impl ProbeOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "PASSED",
            Self::Failed => "FAILED",
            Self::Skipped => "SKIPPED",
        }
    }

    /// `None` for anything but the exact name of an outcome.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Passed, Self::Failed, Self::Skipped]
            .into_iter()
            .find(|outcome| outcome.as_str() == text)
    }
}

/// One check and how it ended. Built only through [`ProbeResult::new`], which holds what makes a set of them a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeCheckResult {
    check: ProbeCheck,
    outcome: ProbeOutcome,
    code: Option<DatabaseFailure>,
}

impl ProbeCheckResult {
    #[must_use]
    pub const fn new(
        check: ProbeCheck,
        outcome: ProbeOutcome,
        code: Option<DatabaseFailure>,
    ) -> Self {
        Self {
            check,
            outcome,
            code,
        }
    }

    #[must_use]
    pub const fn check(&self) -> ProbeCheck {
        self.check
    }

    #[must_use]
    pub const fn outcome(&self) -> ProbeOutcome {
        self.outcome
    }

    /// Why a failed check failed; `None` for any other.
    #[must_use]
    pub const fn code(&self) -> Option<DatabaseFailure> {
        self.code
    }
}

/// A result that is not one: the checks are not the three in order, or one of them contradicts what ended before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the checks of a probe do not make a result")]
pub struct ProbeResultError;

/// The result of the probe of an Iceberg connection: each check of [`ProbeCheck::ALL`], once, in order, ended as the ones before it allow.
///
/// - a failed check has a code that is allowed for it, and no other check has one;
/// - after a failed check every later one is skipped (it could not run);
/// - a skip has a cause: an earlier failure, except the storage check's on credentials the catalog vends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    checks: Vec<ProbeCheckResult>,
}

impl ProbeResult {
    /// # Errors
    /// [`ProbeResultError`] for anything that is not the three checks in order, each ended as the ones before it allow.
    pub fn new(checks: Vec<ProbeCheckResult>) -> Result<Self, ProbeResultError> {
        if checks.len() != ProbeCheck::ALL.len() {
            return Err(ProbeResultError);
        }
        let mut failed_before = false;
        for (result, expected) in checks.iter().zip(ProbeCheck::ALL) {
            if result.check != expected {
                return Err(ProbeResultError);
            }
            let fine = match result.outcome {
                ProbeOutcome::Failed => {
                    !failed_before && result.code.is_some_and(|code| expected.allows(code))
                }
                ProbeOutcome::Passed => !failed_before && result.code.is_none(),
                // what could not run says nothing of why it was not confirmed
                ProbeOutcome::Skipped => {
                    result.code.is_none()
                        && (failed_before || expected == ProbeCheck::StorageAccess)
                }
            };
            if !fine {
                return Err(ProbeResultError);
            }
            failed_before |= result.outcome == ProbeOutcome::Failed;
        }
        Ok(Self { checks })
    }

    #[must_use]
    pub fn checks(&self) -> &[ProbeCheckResult] {
        &self.checks
    }

    /// The code of the first check that failed: what the connection's probe says is wrong, for a reader that knows no checks.
    #[must_use]
    pub fn first_failure(&self) -> Option<DatabaseFailure> {
        self.checks.iter().find_map(|check| {
            (check.outcome == ProbeOutcome::Failed)
                .then_some(check.code)
                .flatten()
        })
    }
}
