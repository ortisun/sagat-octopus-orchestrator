//! Closed managed-executor failures safe to retain in a payload-free journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseFailure {
    Configuration,
    StatementDenied,
    DestinationDenied,
    Connection,
    Tls,
    Authentication,
    PrivilegedRole,
    Capacity,
    Timeout,
    Cancelled,
    Statement,
    ValueType,
    ResultLimit,
    Capability,
    Credential,
    ResultUnavailable,
    OutcomeUnknown,
    WorkerError,
}
impl DatabaseFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Configuration => "DATABASE_CONFIGURATION_INVALID",
            Self::StatementDenied => "DATABASE_STATEMENT_DENIED",
            Self::DestinationDenied => "DATABASE_DESTINATION_DENIED",
            Self::Connection => "DATABASE_CONNECTION_FAILED",
            Self::Tls => "DATABASE_TLS_FAILED",
            Self::Authentication => "DATABASE_AUTHENTICATION_FAILED",
            Self::PrivilegedRole => "DATABASE_PRIVILEGED_ROLE",
            Self::Capacity => "DATABASE_CAPACITY_EXHAUSTED",
            Self::Timeout => "DATABASE_TIMEOUT",
            Self::Cancelled => "DATABASE_CANCELLED",
            Self::Statement => "DATABASE_STATEMENT_FAILED",
            Self::ValueType => "DATABASE_TYPE_UNSUPPORTED",
            Self::ResultLimit => "DATABASE_RESULT_LIMIT",
            Self::Capability => "DATABASE_CAPABILITY_UNAVAILABLE",
            Self::Credential => "DATABASE_CREDENTIAL_UNAVAILABLE",
            Self::ResultUnavailable => "DATABASE_RESULT_UNAVAILABLE",
            Self::OutcomeUnknown => "OUTCOME_UNKNOWN",
            Self::WorkerError => "WORKER_ERROR",
        }
    }
    pub fn parse(code: &str) -> Option<Self> {
        const ALL: &[DatabaseFailure] = &[
            DatabaseFailure::Configuration,
            DatabaseFailure::StatementDenied,
            DatabaseFailure::DestinationDenied,
            DatabaseFailure::Connection,
            DatabaseFailure::Tls,
            DatabaseFailure::Authentication,
            DatabaseFailure::PrivilegedRole,
            DatabaseFailure::Capacity,
            DatabaseFailure::Timeout,
            DatabaseFailure::Cancelled,
            DatabaseFailure::Statement,
            DatabaseFailure::ValueType,
            DatabaseFailure::ResultLimit,
            DatabaseFailure::Capability,
            DatabaseFailure::Credential,
            DatabaseFailure::ResultUnavailable,
            DatabaseFailure::OutcomeUnknown,
            DatabaseFailure::WorkerError,
        ];
        ALL.iter().copied().find(|failure| failure.code() == code)
    }
}
