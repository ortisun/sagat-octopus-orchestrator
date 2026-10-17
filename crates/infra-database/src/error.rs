/// Public failure categories never contain PostgreSQL diagnostics, SQL or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    #[error("invalid database execution configuration")]
    InvalidConfiguration,
    #[error("database statement is not supported")]
    StatementDenied,
    #[error("database destination is denied")]
    DestinationDenied,
    #[error("database connection failed")]
    Connection,
    #[error("database TLS verification failed")]
    Tls,
    #[error("database authentication failed")]
    Authentication,
    #[error("database role must have restricted privileges")]
    PrivilegedRole,
    #[error("database capacity is exhausted")]
    Capacity,
    #[error("database execution deadline expired")]
    Timeout,
    #[error("database execution was cancelled")]
    Cancelled,
    #[error("database statement failed")]
    Statement,
    #[error("database value type is unsupported")]
    ValueType,
    #[error("database result exceeds its limit")]
    ResultLimit,
    #[error("database write outcome is unknown; reconciliation is required")]
    OutcomeUnknown,
}
