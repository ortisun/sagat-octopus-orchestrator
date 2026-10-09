//! The egress policy of a namespace: where its workflows' outbound requests may go (FR-009-008,
//! US03).
//!
//! The rule is default-deny: a namespace with no policy may reach nothing. The domain refuses what no
//! policy can allow (loopback, link-local and the cloud metadata address, multicast, reserved
//! space), so an administrator cannot open those by mistake; private space needs an explicit network
//! rule. This module stores the policy, lets an administrator change it, and gives the features that
//! make outbound requests (webhooks, connectors, the script sandbox) one place to ask, which also
//! records every refusal.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::egress::{Denial, Destination, EgressError, EgressPolicy, HostRule};
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use time::OffsetDateTime;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;

/// Most ports one policy names.
pub const MAX_PORTS: usize = 32;

/// A policy as stored: canonical rule text, so what is shown back is what was understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPolicy {
    pub rules: Vec<String>,
    pub ports: Vec<u16>,
    pub allow_http: bool,
    pub updated_by: String,
    pub updated_at: OffsetDateTime,
}

/// Storage port of the egress policies. Implemented by `sagat-infra-postgres`.
pub trait EgressStore: NamespaceDirectory {
    /// The policy of a namespace; `None` when none was set (everything is denied).
    fn egress_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Option<StoredPolicy>, StoreError>> + Send;

    /// Replaces the policy, with its audit record, in one transaction.
    fn put_egress_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        policy: &StoredPolicy,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Removes the policy. `false` when there was none.
    fn delete_egress_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// Reads and changes egress policies, and answers the features that make outbound requests.
pub struct EgressService<S, C> {
    store: S,
    clock: C,
}

impl<S, C> EgressService<S, C>
where
    S: EgressStore,
    C: Clock,
{
    #[must_use]
    pub const fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }

    /// The policy in force, as text. A namespace that has none shows an empty policy.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn get(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<StoredPolicy, AppError> {
        principal.authorize(Permission::EgressManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .egress_policy(principal.tenant_id(), namespace)
            .await?
            .unwrap_or_else(|| StoredPolicy {
                rules: Vec::new(),
                ports: Vec::new(),
                allow_http: false,
                updated_by: String::new(),
                updated_at: OffsetDateTime::UNIX_EPOCH,
            }))
    }

    /// Replaces the policy of a namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`]; [`AppError::NotAllowed`] for a rule that is
    /// not an exact host, a `*.` domain or a CIDR network, too many rules or ports, or port 0.
    pub async fn set(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        rules: &[String],
        ports: &[u16],
        allow_http: bool,
        ctx: &RequestContext,
    ) -> Result<StoredPolicy, AppError> {
        principal.authorize(Permission::EgressManage)?;
        if ports.len() > MAX_PORTS {
            return Err(AppError::NotAllowed("a policy names at most 32 ports"));
        }
        let parsed = rules
            .iter()
            .map(|rule| HostRule::parse(rule))
            .collect::<Result<Vec<_>, _>>()
            .map_err(egress_error)?;
        // Validates the count and the ports.
        EgressPolicy::new(parsed.clone(), ports.to_vec(), allow_http).map_err(egress_error)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;

        let mut canonical: Vec<String> = parsed.iter().map(ToString::to_string).collect();
        canonical.sort();
        canonical.dedup();
        let mut ports = ports.to_vec();
        ports.sort_unstable();
        ports.dedup();
        let now = self.clock.now();
        let policy = StoredPolicy {
            rules: canonical,
            ports,
            allow_http,
            updated_by: principal.subject().as_str().to_owned(),
            updated_at: now,
        };
        // The audit trail says how much changed, not where traffic may now go: the rules can name
        // internal networks and the trail is readable by more people than the policy.
        let audit = AuditRecord::by(
            principal,
            AuditAction::EgressPolicyChanged,
            ResourceType::EgressPolicy,
            namespace.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "{} rules, {} ports, http {}",
            policy.rules.len(),
            policy.ports.len(),
            if allow_http { "allowed" } else { "refused" }
        ));
        self.store
            .put_egress_policy(principal.tenant_id(), namespace, &policy, &audit)
            .await?;
        Ok(policy)
    }

    /// Removes the policy: the namespace may reach nothing again.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`] for a namespace the tenant does not have.
    pub async fn reset(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::EgressManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::EgressPolicyChanged,
            ResourceType::EgressPolicy,
            namespace.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason("policy removed: everything is denied");
        self.store
            .delete_egress_policy(principal.tenant_id(), namespace, &audit)
            .await?;
        Ok(())
    }

    /// The policy a feature must enforce for outbound requests of `namespace`. Default-deny.
    ///
    /// # Errors
    /// [`AppError::Internal`] when the stored policy no longer parses (it is then enforced by
    /// nobody, so the caller must refuse rather than guess).
    pub async fn policy_for(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<EgressPolicy, AppError> {
        let Some(stored) = self.store.egress_policy(tenant, namespace).await? else {
            return Ok(EgressPolicy::deny_all());
        };
        let rules = stored
            .rules
            .iter()
            .map(|rule| HostRule::parse(rule))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::Internal)?;
        EgressPolicy::new(rules, stored.ports, stored.allow_http).map_err(|_| AppError::Internal)
    }

    /// Whether `namespace` may make a request to `destination` (the first stage, before any name is
    /// resolved). A refusal is recorded in the audit trail, once per destination in a window, with
    /// the category and never the path or the query.
    ///
    /// # Errors
    /// The [`Denial`], wrapped in the error of the policy lookup failing.
    pub async fn check(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        destination: &Destination<'_>,
    ) -> Result<Result<(), Denial>, AppError> {
        let policy = self.policy_for(tenant, namespace).await?;
        let verdict = policy.check_destination(destination);
        if let Err(denial) = verdict {
            self.record_denial(tenant, namespace, destination, denial)
                .await;
        }
        Ok(verdict)
    }

    /// Records that a request was refused, for a caller that enforces the policy itself (the
    /// second stage of the guarded client refuses on the addresses a name resolved to).
    pub async fn record_denial(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        destination: &Destination<'_>,
        denial: Denial,
    ) {
        let host: String = destination.host.chars().take(128).collect();
        let record = AuditRecord::engine(
            tenant,
            AuditAction::EgressDenied,
            ResourceType::EgressPolicy,
            format!("{namespace}/{host}:{}", destination.port),
            denial.code().to_owned(),
            None,
            self.clock.now(),
        );
        if let Err(error) = self.store.record_denial(&record).await {
            tracing::error!(%error, action = %record.action(), "failed to record audit entry");
        }
    }
}

fn egress_error(error: EgressError) -> AppError {
    AppError::NotAllowed(match error {
        EgressError::TooManyRules => "a policy holds at most 256 rules",
        EgressError::InvalidRule(_) => {
            "a rule is an exact host name, '*.' and a domain, or a CIDR network"
        }
        EgressError::InvalidPort => "port 0 is not a port",
    })
}
