//! Quotas: how much of the platform one tenant or one namespace may use (FR-009-006, US04).
//!
//! A quota is a *hard* limit, not a rate: a request that would exceed it is refused, and the count
//! it is judged against must be exact at the moment of the decision. The stores therefore count under
//! a lock, in the same transaction that creates the thing counted; nothing here is an estimate.
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
    /// Connectors to message brokers in one namespace (feature 004).
    Connectors,
    /// Scripts (not versions) in one namespace (feature 005).
    Scripts,
    /// Builds and test runs of one namespace that have not finished: the concurrent work the sandbox
    /// is asked for on behalf of authors (feature 005).
    ScriptJobs,
}

/// At which level a resource is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaScope {
    /// Counted per namespace; a tenant-wide setting is the default of its namespaces.
    Namespace,
    /// Counted across the whole tenant; only a tenant-wide setting applies.
    Tenant,
}

impl QuotaResource {
    pub const ALL: [Self; 11] = [
        Self::ActiveRuns,
        Self::TenantActiveRuns,
        Self::Workflows,
        Self::WorkerSessions,
        Self::ActivityConcurrency,
        Self::Secrets,
        Self::EventEndpoints,
        Self::WebhookSubscriptions,
        Self::Connectors,
        Self::Scripts,
        Self::ScriptJobs,
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
            Self::Connectors => "event_connectors",
            Self::Scripts => "scripts",
            Self::ScriptJobs => "script_jobs",
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
            | Self::Connectors
            | Self::Scripts
            | Self::ScriptJobs => QuotaScope::Namespace,
            Self::TenantActiveRuns | Self::WorkerSessions => QuotaScope::Tenant,
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
            Self::Connectors => 20,
            Self::Scripts => 500,
            Self::ScriptJobs => 8,
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
            Self::Connectors => "message connectors in the namespace",
            Self::Scripts => "scripts in the namespace",
            Self::ScriptJobs => "builds and test runs in progress in the namespace",
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
        .min(MAX_LIMIT)
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
    #[error("a limit is at most {MAX_LIMIT}")]
    TooLarge,
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
    if value > MAX_LIMIT {
        return Err(QuotaError::TooLarge);
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
pub const fn validate_ceiling(value: u64) -> Result<u64, QuotaError> {
    if value > MAX_LIMIT {
        Err(QuotaError::TooLarge)
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
                MAX_LIMIT,
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
        assert_eq!(validate_ceiling(big), Ok(big));
        assert_eq!(validate_ceiling(big + 1), Err(QuotaError::TooLarge));
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
            assert!(resource.default_limit() <= MAX_LIMIT, "{resource}");
            assert!(resource.default_limit() > 0, "{resource}");
        }
    }
}
