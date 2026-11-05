//! Closed managed-executor failures safe to retain in a payload-free journal.
//!
//! One table builds the enum, [`DatabaseFailure::ALL`], `code()`, `parse()` and `retryable()`, so a code cannot be
//! added to one of them only (FR-035-029, D035-042). The rows of feature 035 are the table of
//! `specs/035-iceberg-write-steps/contracts/errors.md`, and a test pins the two together.

macro_rules! failures {
    ($($(#[$doc:meta])* $variant:ident => $code:literal, $retryable:literal;)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum DatabaseFailure {
            $($(#[$doc])* $variant,)+
        }
        impl DatabaseFailure {
            /// Every failure, in the order of the table.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            #[must_use]
            pub const fn code(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// Whether the step may be retried automatically after this failure. The codes of feature 017 keep
            /// being reported as not retryable, as the runner does today; those of feature 035 follow the table of
            /// `contracts/errors.md`. An unknown outcome is never repeated blindly.
            #[must_use]
            pub const fn retryable(self) -> bool {
                match self {
                    $(Self::$variant => $retryable,)+
                }
            }

            #[must_use]
            pub fn parse(code: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|failure| failure.code() == code)
            }
        }
    };
}

failures! {
    // Feature 017: the managed PostgreSQL executor.
    Configuration => "DATABASE_CONFIGURATION_INVALID", false;
    StatementDenied => "DATABASE_STATEMENT_DENIED", false;
    DestinationDenied => "DATABASE_DESTINATION_DENIED", false;
    Connection => "DATABASE_CONNECTION_FAILED", false;
    Tls => "DATABASE_TLS_FAILED", false;
    Authentication => "DATABASE_AUTHENTICATION_FAILED", false;
    PrivilegedRole => "DATABASE_PRIVILEGED_ROLE", false;
    Capacity => "DATABASE_CAPACITY_EXHAUSTED", false;
    Timeout => "DATABASE_TIMEOUT", false;
    Cancelled => "DATABASE_CANCELLED", false;
    Statement => "DATABASE_STATEMENT_FAILED", false;
    ValueType => "DATABASE_TYPE_UNSUPPORTED", false;
    ResultLimit => "DATABASE_RESULT_LIMIT", false;
    Capability => "DATABASE_CAPABILITY_UNAVAILABLE", false;
    Credential => "DATABASE_CREDENTIAL_UNAVAILABLE", false;
    ResultUnavailable => "DATABASE_RESULT_UNAVAILABLE", false;
    OutcomeUnknown => "OUTCOME_UNKNOWN", false;
    WorkerError => "WORKER_ERROR", false;
    // Feature 035: Iceberg writes and step-output destinations (contracts/errors.md).
    IcebergTableNotFound => "ICEBERG_TABLE_NOT_FOUND", false;
    IcebergUnsupportedTable => "ICEBERG_UNSUPPORTED_TABLE", false;
    IcebergLocationNotAllowed => "ICEBERG_LOCATION_NOT_ALLOWED", false;
    IcebergSchemaMismatch => "ICEBERG_SCHEMA_MISMATCH", false;
    IcebergDuplicateKey => "ICEBERG_DUPLICATE_KEY", false;
    IcebergScopeViolation => "ICEBERG_SCOPE_VIOLATION", false;
    IcebergCommitConflict => "ICEBERG_COMMIT_CONFLICT", true;
    IcebergRevertBlocked => "ICEBERG_REVERT_BLOCKED", false;
    IcebergLimitExceeded => "ICEBERG_LIMIT_EXCEEDED", false;
    CatalogUnavailable => "CATALOG_UNAVAILABLE", true;
    StorageUnavailable => "STORAGE_UNAVAILABLE", true;
    DestinationKeyConstraintMissing => "DESTINATION_KEY_CONSTRAINT_MISSING", false;
    DestinationSchemaMismatch => "DESTINATION_SCHEMA_MISMATCH", false;
    DestinationDeduplicationNotConfirmed => "DESTINATION_DEDUPLICATION_NOT_CONFIRMED", false;
    DestinationObjectConflict => "DESTINATION_OBJECT_CONFLICT", false;
    DestinationDeliveryBoundExceeded => "DESTINATION_DELIVERY_BOUND_EXCEEDED", false;
    DestinationSubscriptionShared => "DESTINATION_SUBSCRIPTION_SHARED", false;
    DestinationAuthFailed => "DESTINATION_AUTH_FAILED", false;
    DestinationUnavailable => "DESTINATION_UNAVAILABLE", true;
    DestinationSubscriptionUnusable => "DESTINATION_SUBSCRIPTION_UNUSABLE", false;
    DestinationFailed => "DESTINATION_FAILED", false;
    LakeHostProtocol => "LAKE_HOST_PROTOCOL", true;
    LakeSandboxUnavailable => "LAKE_SANDBOX_UNAVAILABLE", true;
    ClassificationEgressRefused => "CLASSIFICATION_EGRESS_REFUSED", false;
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::DatabaseFailure;
    use std::collections::BTreeSet;

    /// The failure codes that existed before feature 035. The runner reports every one of them as not retryable.
    fn belongs_to_017(code: &str) -> bool {
        code.starts_with("DATABASE_") || code == "WORKER_ERROR"
    }

    /// (code, retryable) of each row of the table of `contracts/errors.md`; `OUTCOME_UNKNOWN` has none ("—"), which
    /// is read as not retryable because an unknown outcome is never repeated blindly.
    fn documented() -> Vec<(String, bool)> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../specs/035-iceberg-write-steps/contracts/errors.md");
        let text =
            std::fs::read_to_string(&path).expect("contracts/errors.md is in the repository");
        let rows: Vec<(String, bool)> = text
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                (cells[1].trim_matches('`').to_owned(), cells[2] == "yes")
            })
            .collect();
        assert!(
            rows.len() >= 24,
            "the table of errors.md has {} rows",
            rows.len()
        );
        rows
    }

    /// FR-035-029, D035-042: no code can be added to one list only. The enum, `code()`, `parse()` and `retryable()` are
    /// built from one table, and this pins that table to the document.
    #[test]
    fn every_documented_code_is_known_with_its_documented_retry_flag() {
        for (code, retryable) in documented() {
            let failure = DatabaseFailure::parse(&code)
                .unwrap_or_else(|| panic!("{code} is in errors.md and unknown to DatabaseFailure"));
            assert_eq!(failure.code(), code);
            assert_eq!(failure.retryable(), retryable, "{code}");
        }
    }

    #[test]
    fn every_known_code_is_in_errors_md_or_belongs_to_017() {
        let documented: BTreeSet<String> = documented().into_iter().map(|(code, _)| code).collect();
        for failure in DatabaseFailure::ALL {
            let code = failure.code();
            assert!(
                documented.contains(code) || belongs_to_017(code),
                "{code} is a DatabaseFailure code that errors.md does not list"
            );
        }
    }

    #[test]
    fn code_and_parse_agree_for_every_failure_and_no_code_is_twice() {
        let mut seen = BTreeSet::new();
        for failure in DatabaseFailure::ALL {
            assert_eq!(DatabaseFailure::parse(failure.code()), Some(*failure));
            assert!(
                seen.insert(failure.code()),
                "{} is listed twice",
                failure.code()
            );
        }
        assert_eq!(seen.len(), DatabaseFailure::ALL.len());
    }

    #[test]
    fn the_017_codes_keep_being_reported_as_not_retryable() {
        for failure in DatabaseFailure::ALL {
            if belongs_to_017(failure.code()) {
                assert!(!failure.retryable(), "{}", failure.code());
            }
        }
    }

    #[test]
    fn a_code_that_is_not_in_the_set_is_refused() {
        for code in [
            "",
            "ICEBERG_NOPE",
            "iceberg_table_not_found",
            "ICEBERG_TABLE_NOT_FOUND ",
            "DATABASE_",
        ] {
            assert_eq!(DatabaseFailure::parse(code), None, "{code:?}");
        }
    }
}
