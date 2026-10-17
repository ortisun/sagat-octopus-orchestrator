//! Quotas: how much of the platform one tenant or one namespace may use (FR-009-006, US04).
//!
//! Quotas cover exact live counts, retained bytes, admission rates, retention and operation ceilings. Count and
//! storage admission is decided against authoritative usage in the transaction that creates the
//! counted data. Rate admission uses durable database-time token buckets. Retention quotas bound new
//! policy choices rather than reporting a scalar current usage. Database operation ceilings likewise
//! have no scalar usage; they narrow each managed attempt.
//!
//! The defaults below are **provisional**: the specification leaves "capacity and quota values per
//! tier" to be defined after benchmark and security testing (spec.md, open design questions). They
//! are generous enough not to get in the way of a single tenant and small enough to bound what a
//! hostile one can make the database hold.
//!
//! Who may change a limit matters as much as its value. A tenant's administrator chooses the limits
//! of the tenant's own namespaces, **but only up to a ceiling the platform grants that tenant**: the
//! default below, or what an operator has granted since (`control-plane quota-ceiling`). Without
//! the ceiling the quota would be controlled by the party it constrains, and a noisy tenant could
//! simply remove it. Nothing in the API can raise a ceiling.

use std::fmt;
use std::str::FromStr;

/// Largest value a limit can be set to; also what "effectively unlimited" means. Keeps every count
/// and every comparison well inside `i64`.
pub const MAX_LIMIT: u64 = 1_000_000_000;

/// Largest logical-byte quota PostgreSQL can safely aggregate with ample headroom for deltas.
pub const MAX_BYTE_LIMIT: u64 = 1_u64 << 50;

/// What is limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QuotaResource {
    /// Runs that have not finished, in one namespace.
    ActiveRuns,
    /// Runs that have not finished, in all the namespaces of the tenant.
    TenantActiveRuns,
    /// Workflows (not versions) in one namespace.
    Workflows,
    /// Live worker sessions of the tenant.
    WorkerSessions,
    /// Tasks of one activity that may hold an unexpired worker lease in one namespace.
    ActivityConcurrency,
    /// Secrets in one namespace.
    Secrets,
    /// Webhook endpoints in one namespace (feature 004).
    EventEndpoints,
    /// Subscriptions to outbound events in one namespace (feature 004).
    WebhookSubscriptions,
    /// Outbound subscriptions to tenant-operated brokers in one namespace (feature 004).
    BrokerSubscriptions,
    /// Connectors to message brokers in one namespace (feature 004).
    Connectors,
    /// Scripts (not versions) in one namespace (feature 005).
    Scripts,
    /// Builds and test runs of one namespace that have not finished: the concurrent work the sandbox
    /// is asked for on behalf of authors (feature 005).
    ScriptJobs,
    /// Tenant-owned retained content and outstanding upload reservations.
    TenantContentBytes,
    /// Tenant-owned retained redacted application logs.
    TenantLogBytes,
    /// Authenticated namespace API requests admitted each second.
    ApiRequestsPerSecond,
    /// New namespace events admitted each second.
    EventsPerSecond,
    /// Longest event retention newly selected in a namespace.
    EventRetentionSeconds,
    /// Longest script retention newly selected in a namespace.
    ScriptRetentionDays,
    /// Sql bytes per database operation (feature 017).
    DatabaseStatementBytes,
    /// Parameters per database operation (feature 017).
    DatabaseParameters,
    /// Parameter bytes per database operation (feature 017).
    DatabaseParameterBytes,
    /// Rows per database result (feature 017).
    DatabaseRows,
    /// Bytes per database result (feature 017).
    DatabaseResultBytes,
    /// Database operation deadline (feature 017).
    DatabaseTimeoutMs,
    /// Database connect deadline (feature 017).
    DatabaseConnectTimeoutMs,
    /// Concurrent connections per database target (feature 017).
    DatabasePoolSize,
    /// Database idle lifetime (feature 017).
    DatabaseIdleTimeoutMs,
    /// Database connections in the namespace (feature 017).
    DatabaseConnections,
    /// Concurrent database operations in the namespace (feature 017).
    DatabaseConcurrency,
    /// Concurrent database operations of the tenant (feature 017).
    TenantDatabaseConcurrency,
    /// Pending secret provider operations in the namespace (feature 017).
    SecretProviderOperations,
}

/// At which level a resource is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaScope {
    /// Counted per namespace; a tenant-wide setting is the default of its namespaces.
    Namespace,
    /// Counted across the whole tenant; only a tenant-wide setting applies.
    Tenant,
}

/// How a quota participates in admission and usage reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaKind {
    Count,
    Storage,
    Rate,
    Retention,
    /// A positive per-operation ceiling, without scalar usage.
    Policy,
}

/// The scalar unit carried by a quota value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaUnit {
    Items,
    Bytes,
    RequestsPerSecond,
    Seconds,
    Days,
    Milliseconds,
}

impl QuotaResource {
    pub const ALL: [Self; 31] = [
        Self::ActiveRuns,
        Self::TenantActiveRuns,
        Self::Workflows,
        Self::WorkerSessions,
        Self::ActivityConcurrency,
        Self::Secrets,
        Self::EventEndpoints,
        Self::WebhookSubscriptions,
        Self::BrokerSubscriptions,
        Self::Connectors,
        Self::Scripts,
        Self::ScriptJobs,
        Self::TenantContentBytes,
        Self::TenantLogBytes,
        Self::ApiRequestsPerSecond,
        Self::EventsPerSecond,
        Self::EventRetentionSeconds,
        Self::ScriptRetentionDays,
        Self::DatabaseStatementBytes,
        Self::DatabaseParameters,
        Self::DatabaseParameterBytes,
        Self::DatabaseRows,
        Self::DatabaseResultBytes,
        Self::DatabaseTimeoutMs,
        Self::DatabaseConnectTimeoutMs,
        Self::DatabasePoolSize,
        Self::DatabaseIdleTimeoutMs,
        Self::DatabaseConnections,
        Self::DatabaseConcurrency,
        Self::TenantDatabaseConcurrency,
        Self::SecretProviderOperations,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActiveRuns => "active_runs",
            Self::TenantActiveRuns => "tenant_active_runs",
            Self::Workflows => "workflows",
            Self::WorkerSessions => "worker_sessions",
            Self::ActivityConcurrency => "activity_concurrency",
            Self::Secrets => "secrets",
            Self::EventEndpoints => "event_endpoints",
            Self::WebhookSubscriptions => "webhook_subscriptions",
            Self::BrokerSubscriptions => "broker_subscriptions",
            Self::Connectors => "event_connectors",
            Self::Scripts => "scripts",
            Self::ScriptJobs => "script_jobs",
            Self::TenantContentBytes => "tenant_content_bytes",
            Self::TenantLogBytes => "tenant_log_bytes",
            Self::ApiRequestsPerSecond => "api_requests_per_second",
            Self::EventsPerSecond => "events_per_second",
            Self::EventRetentionSeconds => "event_retention_seconds",
            Self::ScriptRetentionDays => "script_retention_days",
            Self::DatabaseStatementBytes => "database_statement_bytes",
            Self::DatabaseParameters => "database_parameters",
            Self::DatabaseParameterBytes => "database_parameter_bytes",
            Self::DatabaseRows => "database_rows",
            Self::DatabaseResultBytes => "database_result_bytes",
            Self::DatabaseTimeoutMs => "database_timeout_ms",
            Self::DatabaseConnectTimeoutMs => "database_connect_timeout_ms",
            Self::DatabasePoolSize => "database_pool_size",
            Self::DatabaseIdleTimeoutMs => "database_idle_timeout_ms",
            Self::DatabaseConnections => "database_connections",
            Self::DatabaseConcurrency => "database_concurrency",
            Self::TenantDatabaseConcurrency => "tenant_database_concurrency",
            Self::SecretProviderOperations => "secret_provider_operations",
        }
    }

    #[must_use]
    pub const fn scope(self) -> QuotaScope {
        match self {
            Self::ActiveRuns
            | Self::ActivityConcurrency
            | Self::Workflows
            | Self::Secrets
            | Self::EventEndpoints
            | Self::WebhookSubscriptions
            | Self::BrokerSubscriptions
            | Self::Connectors
            | Self::Scripts
            | Self::ScriptJobs
            | Self::ApiRequestsPerSecond
            | Self::EventsPerSecond
            | Self::EventRetentionSeconds
            | Self::ScriptRetentionDays
            | Self::DatabaseStatementBytes
            | Self::DatabaseParameters
            | Self::DatabaseParameterBytes
            | Self::DatabaseRows
            | Self::DatabaseResultBytes
            | Self::DatabaseTimeoutMs
            | Self::DatabaseConnectTimeoutMs
            | Self::DatabasePoolSize
            | Self::DatabaseIdleTimeoutMs
            | Self::DatabaseConnections
            | Self::DatabaseConcurrency
            | Self::SecretProviderOperations => QuotaScope::Namespace,
            Self::TenantActiveRuns
            | Self::WorkerSessions
            | Self::TenantContentBytes
            | Self::TenantLogBytes
            | Self::TenantDatabaseConcurrency => QuotaScope::Tenant,
        }
    }

    #[must_use]
    pub const fn kind(self) -> QuotaKind {
        match self {
            Self::TenantContentBytes | Self::TenantLogBytes => QuotaKind::Storage,
            Self::ApiRequestsPerSecond | Self::EventsPerSecond => QuotaKind::Rate,
            Self::EventRetentionSeconds | Self::ScriptRetentionDays => QuotaKind::Retention,
            Self::DatabaseStatementBytes
            | Self::DatabaseParameters
            | Self::DatabaseParameterBytes
            | Self::DatabaseRows
            | Self::DatabaseResultBytes
            | Self::DatabaseTimeoutMs
            | Self::DatabaseConnectTimeoutMs
            | Self::DatabasePoolSize
            | Self::DatabaseIdleTimeoutMs => QuotaKind::Policy,
            _ => QuotaKind::Count,
        }
    }

    #[must_use]
    pub const fn unit(self) -> QuotaUnit {
        match self {
            Self::TenantContentBytes | Self::TenantLogBytes => QuotaUnit::Bytes,
            Self::ApiRequestsPerSecond | Self::EventsPerSecond => QuotaUnit::RequestsPerSecond,
            Self::EventRetentionSeconds => QuotaUnit::Seconds,
            Self::ScriptRetentionDays => QuotaUnit::Days,
            Self::DatabaseStatementBytes => QuotaUnit::Bytes,
            Self::DatabaseParameterBytes => QuotaUnit::Bytes,
            Self::DatabaseResultBytes => QuotaUnit::Bytes,
            Self::DatabaseTimeoutMs => QuotaUnit::Milliseconds,
            Self::DatabaseConnectTimeoutMs => QuotaUnit::Milliseconds,
            Self::DatabaseIdleTimeoutMs => QuotaUnit::Milliseconds,
            _ => QuotaUnit::Items,
        }
    }

    /// Database policies share one tenant/namespace revocation fence.
    #[must_use]
    pub const fn is_database(self) -> bool {
        matches!(
            self,
            Self::DatabaseStatementBytes
                | Self::DatabaseParameters
                | Self::DatabaseParameterBytes
                | Self::DatabaseRows
                | Self::DatabaseResultBytes
                | Self::DatabaseTimeoutMs
                | Self::DatabaseConnectTimeoutMs
                | Self::DatabasePoolSize
                | Self::DatabaseIdleTimeoutMs
                | Self::DatabaseConnections
                | Self::DatabaseConcurrency
                | Self::TenantDatabaseConcurrency
                | Self::SecretProviderOperations
        )
    }

    /// Whether the quota view can report one exact retained/live usage value.
    #[must_use]
    pub const fn has_usage(self) -> bool {
        !matches!(
            self.kind(),
            QuotaKind::Rate | QuotaKind::Retention | QuotaKind::Policy
        )
    }

    /// Largest value accepted for this resource without expanding legacy count limits.
    #[must_use]
    pub const fn maximum(self) -> u64 {
        match self {
            Self::TenantContentBytes | Self::TenantLogBytes => MAX_BYTE_LIMIT,
            Self::ApiRequestsPerSecond | Self::EventsPerSecond => 1_000_000,
            Self::EventRetentionSeconds => 2_592_000,
            Self::ScriptRetentionDays => 365,
            Self::DatabaseStatementBytes => 65_536,
            Self::DatabaseParameters => 256,
            Self::DatabaseParameterBytes => 262_144,
            Self::DatabaseRows => 10_000,
            Self::DatabaseResultBytes => 4_194_304,
            Self::DatabaseTimeoutMs => 30_000,
            Self::DatabaseConnectTimeoutMs => 10_000,
            Self::DatabasePoolSize => 16,
            Self::DatabaseIdleTimeoutMs => 60_000,
            Self::DatabaseConnections => 1_000,
            Self::DatabaseConcurrency => 64,
            Self::TenantDatabaseConcurrency => 256,
            Self::SecretProviderOperations => 128,
            _ => MAX_LIMIT,
        }
    }

    /// What applies when nobody has set anything, and the ceiling of a tenant nobody has granted
    /// more. Provisional (see the module documentation).
    #[must_use]
    pub const fn default_limit(self) -> u64 {
        match self {
            Self::ActiveRuns => 10_000,
            Self::TenantActiveRuns => 50_000,
            Self::Workflows => 1_000,
            Self::WorkerSessions => 1_000,
            // No activity is throttled until a tenant administrator sets a default or override.
            Self::ActivityConcurrency => MAX_LIMIT,
            Self::Secrets => 500,
            Self::EventEndpoints => 100,
            Self::WebhookSubscriptions => 50,
            Self::BrokerSubscriptions => 50,
            Self::Connectors => 20,
            Self::Scripts => 500,
            Self::ScriptJobs => 8,
            Self::TenantContentBytes => 10 * 1024 * 1024 * 1024,
            Self::TenantLogBytes => 1024 * 1024 * 1024,
            Self::ApiRequestsPerSecond => 100,
            Self::EventsPerSecond => 1_000,
            Self::EventRetentionSeconds => 2_592_000,
            Self::ScriptRetentionDays => 365,
            Self::DatabaseStatementBytes => 65_536,
            Self::DatabaseParameters => 256,
            Self::DatabaseParameterBytes => 262_144,
            Self::DatabaseRows => 10_000,
            Self::DatabaseResultBytes => 4_194_304,
            Self::DatabaseTimeoutMs => 30_000,
            Self::DatabaseConnectTimeoutMs => 10_000,
            Self::DatabasePoolSize => 16,
            Self::DatabaseIdleTimeoutMs => 60_000,
            Self::DatabaseConnections => 100,
            Self::DatabaseConcurrency => 16,
            Self::TenantDatabaseConcurrency => 64,
            Self::SecretProviderOperations => 32,
        }
    }

    /// The name used in the error a refused request carries. Never a value.
    #[must_use]
    pub const fn limit_name(self) -> &'static str {
        match self {
            Self::ActiveRuns => "active runs in the namespace",
            Self::TenantActiveRuns => "active runs of the tenant",
            Self::Workflows => "workflows in the namespace",
            Self::WorkerSessions => "worker sessions of the tenant",
            Self::ActivityConcurrency => "concurrent tasks of the activity",
            Self::Secrets => "secrets in the namespace",
            Self::EventEndpoints => "webhook endpoints in the namespace",
            Self::WebhookSubscriptions => "outbound webhook subscriptions in the namespace",
            Self::BrokerSubscriptions => "outbound broker subscriptions in the namespace",
            Self::Connectors => "message connectors in the namespace",
            Self::Scripts => "scripts in the namespace",
            Self::ScriptJobs => "builds and test runs in progress in the namespace",
            Self::TenantContentBytes => "retained content bytes of the tenant",
            Self::TenantLogBytes => "retained log bytes of the tenant",
            Self::ApiRequestsPerSecond => "API requests per second in the namespace",
            Self::EventsPerSecond => "events per second in the namespace",
            Self::EventRetentionSeconds => "event retention in the namespace",
            Self::ScriptRetentionDays => "script retention in the namespace",
            Self::DatabaseStatementBytes => "SQL bytes per database operation",
            Self::DatabaseParameters => "parameters per database operation",
            Self::DatabaseParameterBytes => "parameter bytes per database operation",
            Self::DatabaseRows => "rows per database result",
            Self::DatabaseResultBytes => "bytes per database result",
            Self::DatabaseTimeoutMs => "database operation deadline",
            Self::DatabaseConnectTimeoutMs => "database connect deadline",
            Self::DatabasePoolSize => "concurrent connections per database target",
            Self::DatabaseIdleTimeoutMs => "database idle lifetime",
            Self::DatabaseConnections => "database connections in the namespace",
            Self::DatabaseConcurrency => "concurrent database operations in the namespace",
            Self::TenantDatabaseConcurrency => "concurrent database operations of the tenant",
            Self::SecretProviderOperations => "pending secret provider operations in the namespace",
        }
    }
}

impl fmt::Display for QuotaResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resource name that does not exist.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown quota resource")]
pub struct UnknownResource;

impl FromStr for QuotaResource {
    type Err = UnknownResource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == s)
            .ok_or(UnknownResource)
    }
}

/// The highest limit a tenant may set for `resource`: what an operator granted it, or the platform
/// default.
#[must_use]
pub fn ceiling(resource: QuotaResource, granted: Option<u64>) -> u64 {
    granted
        .unwrap_or_else(|| resource.default_limit())
        .min(resource.maximum())
}

/// Where the limit in force comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaSource {
    /// The tenant set nothing: the ceiling applies (the platform default unless an operator
    /// granted more).
    Default,
    /// Set for every namespace of the tenant (or, for a tenant-scoped resource, for the tenant).
    Tenant,
    /// Set for this namespace.
    Namespace,
}

impl QuotaSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Tenant => "tenant",
            Self::Namespace => "namespace",
        }
    }
}

/// A value that cannot be a limit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
    #[error("the limit is above the maximum for this resource")]
    TooLarge,
    #[error("the limit is outside the valid range for this resource")]
    OutsideRange,
    /// More than the platform grants this tenant. The ceiling is shown in the quota view.
    #[error("the value is above the ceiling the platform grants this tenant")]
    AboveCeiling,
    #[error("this quota is counted for the whole tenant and cannot be set for one namespace")]
    NotPerNamespace,
}

/// Checks that a tenant may store `value` as the limit of `resource` at the given level, given the
/// `ceiling` the platform grants it.
///
/// # Errors
/// [`QuotaError`] when the value is out of range or above the ceiling, or a namespace-level value is
/// given for a resource that is counted across the tenant.
pub fn validate(
    resource: QuotaResource,
    namespace_level: bool,
    value: u64,
    ceiling: u64,
) -> Result<u64, QuotaError> {
    if value > resource.maximum() {
        return Err(QuotaError::TooLarge);
    }
    if value == 0 && resource.kind() == QuotaKind::Policy {
        return Err(QuotaError::OutsideRange);
    }
    if resource == QuotaResource::EventRetentionSeconds && (1..60).contains(&value) {
        return Err(QuotaError::OutsideRange);
    }
    if namespace_level && resource.scope() == QuotaScope::Tenant {
        return Err(QuotaError::NotPerNamespace);
    }
    if value > ceiling {
        return Err(QuotaError::AboveCeiling);
    }
    Ok(value)
}

/// Checks a value an operator grants as a ceiling.
///
/// # Errors
/// [`QuotaError::TooLarge`] above [`MAX_LIMIT`].
pub const fn validate_ceiling(resource: QuotaResource, value: u64) -> Result<u64, QuotaError> {
    if value > resource.maximum() {
        Err(QuotaError::TooLarge)
    } else if (value == 0 && matches!(resource.kind(), QuotaKind::Policy))
        || (matches!(resource, QuotaResource::EventRetentionSeconds) && value > 0 && value < 60)
    {
        Err(QuotaError::OutsideRange)
    } else {
        Ok(value)
    }
}

/// The limit in force, from what is stored: the namespace's own setting wins over the tenant-wide
/// one, which wins over the ceiling. Never more than the `ceiling`, even when an operator lowered it
/// below what the tenant had set.
#[must_use]
pub fn resolve(
    resource: QuotaResource,
    namespace_setting: Option<u64>,
    tenant_setting: Option<u64>,
    ceiling: u64,
) -> (u64, QuotaSource) {
    // A tenant-scoped resource has no namespace level, whatever is stored.
    let namespace_setting = match resource.scope() {
        QuotaScope::Namespace => namespace_setting,
        QuotaScope::Tenant => None,
    };
    match (namespace_setting, tenant_setting) {
        (Some(value), _) => (value.min(ceiling), QuotaSource::Namespace),
        (None, Some(value)) => (value.min(ceiling), QuotaSource::Tenant),
        (None, None) => (ceiling, QuotaSource::Default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CEILING: u64 = 1_000;

    #[test]
    fn the_namespace_wins_over_the_tenant_which_wins_over_the_ceiling() {
        let r = QuotaResource::ActiveRuns;
        assert_eq!(
            resolve(r, Some(5), Some(50), CEILING),
            (5, QuotaSource::Namespace),
            "the namespace's own setting wins, even over a larger tenant value"
        );
        assert_eq!(
            resolve(r, None, Some(50), CEILING),
            (50, QuotaSource::Tenant)
        );
        assert_eq!(
            resolve(r, None, None, CEILING),
            (CEILING, QuotaSource::Default),
            "with nothing set the whole ceiling is available"
        );
        assert_eq!(
            resolve(r, Some(0), None, CEILING),
            (0, QuotaSource::Namespace),
            "zero is a limit"
        );
    }

    #[test]
    fn a_setting_above_a_lowered_ceiling_is_clamped_to_it() {
        let r = QuotaResource::ActiveRuns;
        // The operator lowered the ceiling after the tenant had set larger values.
        assert_eq!(
            resolve(r, Some(900), None, 100),
            (100, QuotaSource::Namespace)
        );
        assert_eq!(resolve(r, None, Some(900), 100), (100, QuotaSource::Tenant));
        assert_eq!(
            resolve(r, Some(100), None, 100).0,
            100,
            "at the ceiling is fine"
        );
        assert_eq!(resolve(r, Some(7), None, 100).0, 7, "below it is untouched");
    }

    #[test]
    fn a_tenant_wide_resource_ignores_a_namespace_setting() {
        let r = QuotaResource::WorkerSessions;
        assert_eq!(
            resolve(r, Some(1), Some(7), CEILING),
            (7, QuotaSource::Tenant)
        );
        assert_eq!(resolve(r, Some(1), None, CEILING).1, QuotaSource::Default);
    }

    #[test]
    fn the_ceiling_is_what_an_operator_granted_or_the_default() {
        for resource in QuotaResource::ALL {
            assert_eq!(ceiling(resource, None), resource.default_limit());
            assert_eq!(ceiling(resource, Some(3)), 3);
            assert_eq!(ceiling(resource, Some(0)), 0, "zero is a ceiling");
            assert_eq!(
                ceiling(resource, Some(u64::MAX)),
                resource.maximum(),
                "a stored value out of range cannot lift the limit"
            );
        }
    }

    #[test]
    fn values_are_bounded_by_the_range_the_level_and_the_ceiling() {
        let big = MAX_LIMIT;
        assert_eq!(
            validate(QuotaResource::Workflows, true, 10, CEILING),
            Ok(10)
        );
        assert_eq!(validate(QuotaResource::Workflows, true, 0, CEILING), Ok(0));
        assert_eq!(
            validate(QuotaResource::Workflows, true, CEILING, CEILING),
            Ok(CEILING),
            "the ceiling itself is allowed"
        );
        assert_eq!(
            validate(QuotaResource::Workflows, true, CEILING + 1, CEILING),
            Err(QuotaError::AboveCeiling)
        );
        assert_eq!(
            validate(QuotaResource::Workflows, false, big + 1, big),
            Err(QuotaError::TooLarge)
        );
        assert_eq!(
            validate(QuotaResource::WorkerSessions, true, 10, CEILING),
            Err(QuotaError::NotPerNamespace)
        );
        assert_eq!(
            validate(QuotaResource::WorkerSessions, false, 10, CEILING),
            Ok(10)
        );
        assert_eq!(validate_ceiling(QuotaResource::Workflows, big), Ok(big));
        assert_eq!(
            validate_ceiling(QuotaResource::Workflows, big + 1),
            Err(QuotaError::TooLarge)
        );
    }

    #[test]
    fn names_round_trip_and_unknown_names_are_refused() {
        for resource in QuotaResource::ALL {
            assert_eq!(resource.as_str().parse(), Ok(resource));
        }
        assert_eq!("builds".parse::<QuotaResource>(), Err(UnknownResource));
        assert_eq!("".parse::<QuotaResource>(), Err(UnknownResource));
    }

    #[test]
    fn every_default_is_inside_the_range_a_limit_may_take() {
        for resource in QuotaResource::ALL {
            assert!(resource.default_limit() <= resource.maximum(), "{resource}");
            assert!(resource.default_limit() > 0, "{resource}");
        }
    }

    #[test]
    fn durable_resources_have_explicit_scope_kind_unit_range_and_usage_semantics() {
        assert_eq!(QuotaResource::ALL.len(), 31);
        for (resource, scope, kind, unit, maximum, default, has_usage) in [
            (
                QuotaResource::TenantContentBytes,
                QuotaScope::Tenant,
                QuotaKind::Storage,
                QuotaUnit::Bytes,
                1_u64 << 50,
                10 * 1024 * 1024 * 1024,
                true,
            ),
            (
                QuotaResource::TenantLogBytes,
                QuotaScope::Tenant,
                QuotaKind::Storage,
                QuotaUnit::Bytes,
                1_u64 << 50,
                1024 * 1024 * 1024,
                true,
            ),
            (
                QuotaResource::ApiRequestsPerSecond,
                QuotaScope::Namespace,
                QuotaKind::Rate,
                QuotaUnit::RequestsPerSecond,
                1_000_000,
                100,
                false,
            ),
            (
                QuotaResource::EventsPerSecond,
                QuotaScope::Namespace,
                QuotaKind::Rate,
                QuotaUnit::RequestsPerSecond,
                1_000_000,
                1_000,
                false,
            ),
            (
                QuotaResource::EventRetentionSeconds,
                QuotaScope::Namespace,
                QuotaKind::Retention,
                QuotaUnit::Seconds,
                2_592_000,
                2_592_000,
                false,
            ),
            (
                QuotaResource::ScriptRetentionDays,
                QuotaScope::Namespace,
                QuotaKind::Retention,
                QuotaUnit::Days,
                365,
                365,
                false,
            ),
        ] {
            assert_eq!(resource.scope(), scope, "{resource}");
            assert_eq!(resource.kind(), kind, "{resource}");
            assert_eq!(resource.unit(), unit, "{resource}");
            assert_eq!(resource.maximum(), maximum, "{resource}");
            assert_eq!(resource.default_limit(), default, "{resource}");
            assert_eq!(resource.has_usage(), has_usage, "{resource}");
        }
    }

    #[test]
    fn resource_specific_ranges_do_not_expand_legacy_count_limits() {
        assert_eq!(QuotaResource::Workflows.maximum(), MAX_LIMIT);
        assert_eq!(
            validate(
                QuotaResource::TenantContentBytes,
                false,
                1_u64 << 50,
                1_u64 << 50
            ),
            Ok(1_u64 << 50)
        );
        assert_eq!(
            validate(
                QuotaResource::TenantContentBytes,
                false,
                (1_u64 << 50) + 1,
                1_u64 << 50
            ),
            Err(QuotaError::TooLarge)
        );
        assert_eq!(
            validate(QuotaResource::Workflows, false, MAX_LIMIT + 1, 1_u64 << 50),
            Err(QuotaError::TooLarge)
        );
        assert_eq!(
            validate(QuotaResource::EventRetentionSeconds, true, 59, 2_592_000),
            Err(QuotaError::OutsideRange)
        );
        assert_eq!(
            validate(QuotaResource::EventRetentionSeconds, true, 60, 2_592_000),
            Ok(60)
        );
        assert_eq!(
            validate(QuotaResource::EventRetentionSeconds, true, 0, 2_592_000),
            Ok(0)
        );
    }
    #[test]
    fn database_policy_bounds_are_positive_but_zero_counts_disable_admission() {
        for resource in QuotaResource::ALL.into_iter().filter(|r| r.is_database()) {
            let expected = if resource.kind() == QuotaKind::Policy {
                Err(QuotaError::OutsideRange)
            } else {
                Ok(0)
            };
            assert_eq!(validate(resource, false, 0, resource.maximum()), expected);
            assert_eq!(validate_ceiling(resource, 0), expected);
            assert_eq!(
                validate_ceiling(resource, resource.maximum() + 1),
                Err(QuotaError::TooLarge)
            );
            assert_eq!(resource.has_usage(), resource.kind() == QuotaKind::Count);
        }
        assert_eq!(
            QuotaResource::DatabaseTimeoutMs.unit(),
            QuotaUnit::Milliseconds
        );
        assert_eq!(QuotaResource::DatabaseRows.default_limit(), 10_000);
        assert_eq!(QuotaResource::DatabaseConcurrency.default_limit(), 16);
    }
}
