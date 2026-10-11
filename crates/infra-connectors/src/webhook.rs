//! Outbound webhooks through the guarded egress client (`FR-004-006`, `FR-009-008`).
//!
//! The only way a subscription reaches the network. The policy of the namespace is read for every
//! request and the client is rebuilt when it changed, so widening or narrowing a policy takes effect
//! on the next attempt. A refusal is audited (once per destination in a window, with the category and
//! never the address), and is reported as a failure the worker retries: an administrator may widen
//! the policy meanwhile.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use reqwest::Method;
use sagat_application::clock::SystemClock;
use sagat_application::egress::{EgressService, EgressStore};
use sagat_application::outbound::{OutboundRequest, OutboundResponse, SendFailure, WebhookSender};
use sagat_domain::egress::EgressPolicy;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_infra_egress::{EgressClient, EgressError, EgressLimits, EgressRequest, ResponseBody};

/// Clients kept at once (one per namespace that delivers); beyond it the cache starts over.
const MAX_CLIENTS: usize = 256;

/// Longest a subscription's timeout can be (the client's own limit sits above it, so the timeout of
/// the request is what decides).
const CLIENT_CEILING: Duration = Duration::from_secs(35);

/// How a client is made for a policy. The production factory builds the guarded client; a test
/// substitutes one that can reach a local server.
pub trait ClientFactory: Send + Sync {
    /// # Errors
    /// [`EgressError::Transport`] if the TLS stack cannot be built.
    fn client(&self, policy: &EgressPolicy) -> Result<EgressClient, EgressError>;
}

/// The guarded client, exactly as the egress crate builds it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Guarded;

impl ClientFactory for Guarded {
    fn client(&self, policy: &EgressPolicy) -> Result<EgressClient, EgressError> {
        EgressClient::new(
            policy.clone(),
            EgressLimits {
                total_timeout: CLIENT_CEILING,
                // What a receiver answers is read to learn its status, not to be kept.
                max_response_bytes: 64 * 1024,
                ..EgressLimits::default()
            },
        )
    }
}

struct Cached {
    policy: EgressPolicy,
    client: Arc<EgressClient>,
}

/// The guarded clients of the namespaces, one per policy in force: asked for a client, it reads the
/// policy of the namespace, and builds a new client only when the policy changed.
pub struct Clients<S, F = Guarded> {
    egress: EgressService<S, SystemClock>,
    factory: F,
    clients: Mutex<HashMap<(TenantId, NamespaceId), Cached>>,
}

impl<S, F> std::fmt::Debug for Clients<S, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clients").finish_non_exhaustive()
    }
}

impl<S> Clients<S, Guarded>
where
    S: EgressStore,
{
    #[must_use]
    pub fn new(store: S) -> Self {
        Self::with_factory(store, Guarded)
    }
}

impl<S, F> Clients<S, F>
where
    S: EgressStore,
    F: ClientFactory,
{
    #[must_use]
    pub fn with_factory(store: S, factory: F) -> Self {
        Self {
            egress: EgressService::new(store, SystemClock),
            factory,
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// The egress service these clients answer to (it records the refusals).
    #[must_use]
    pub const fn egress(&self) -> &EgressService<S, SystemClock> {
        &self.egress
    }

    /// The client for the policy of `namespace`: the one built for it already, or a new one.
    ///
    /// # Errors
    /// [`SendFailure::Network`] when the policy cannot be read or the client cannot be built.
    pub async fn client(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Arc<EgressClient>, SendFailure> {
        let policy = self
            .egress
            .policy_for(tenant, namespace)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "could not read the egress policy");
                SendFailure::Network
            })?;
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = clients.get(&(tenant, namespace))
            && cached.policy == policy
        {
            return Ok(Arc::clone(&cached.client));
        }
        let client = Arc::new(self.factory.client(&policy).map_err(|error| {
            tracing::error!(%error, "could not build the egress client");
            SendFailure::Network
        })?);
        if clients.len() >= MAX_CLIENTS {
            clients.clear();
        }
        clients.insert(
            (tenant, namespace),
            Cached {
                policy,
                client: Arc::clone(&client),
            },
        );
        Ok(client)
    }
}

/// Sends the requests of the delivery worker.
pub struct GuardedSender<S, F = Guarded> {
    clients: Clients<S, F>,
}

impl<S, F> std::fmt::Debug for GuardedSender<S, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedSender").finish_non_exhaustive()
    }
}

impl<S> GuardedSender<S, Guarded>
where
    S: EgressStore,
{
    #[must_use]
    pub fn new(store: S) -> Self {
        Self::with_factory(store, Guarded)
    }
}

impl<S, F> GuardedSender<S, F>
where
    S: EgressStore,
    F: ClientFactory,
{
    #[must_use]
    pub fn with_factory(store: S, factory: F) -> Self {
        Self {
            clients: Clients::with_factory(store, factory),
        }
    }
}

/// `Retry-After` in delta-seconds. The HTTP-date form is not honoured (the schedule applies).
fn retry_after(headers: &[(String, String)]) -> Option<Duration> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

impl<S, F> WebhookSender for GuardedSender<S, F>
where
    S: EgressStore,
    F: ClientFactory,
{
    async fn send(&self, request: &OutboundRequest) -> Result<OutboundResponse, SendFailure> {
        let client = self
            .clients
            .client(request.tenant_id, request.namespace_id)
            .await?;
        let outgoing = EgressRequest {
            method: Method::POST,
            url: request.url.as_str().to_owned(),
            headers: request.headers.clone(),
            body: request.body.clone(),
        };
        // The status and the headers are what is learned from a receiver; its body is never read,
        // so a receiver that answers with a page cannot turn a delivery into a failure.
        match tokio::time::timeout(
            request.timeout,
            client.send_with(outgoing, ResponseBody::Skip),
        )
        .await
        {
            Err(_) => Err(SendFailure::Timeout),
            Ok(Ok(response)) => Ok(OutboundResponse {
                status: response.status.as_u16(),
                retry_after: retry_after(&response.headers),
            }),
            Ok(Err(EgressError::Denied(denial))) => {
                self.clients
                    .egress()
                    .record_denial(
                        request.tenant_id,
                        request.namespace_id,
                        &request.url.destination(),
                        denial,
                    )
                    .await;
                Err(SendFailure::Denied)
            }
            Ok(Err(EgressError::InvalidUrl)) => Err(SendFailure::Invalid),
            Ok(Err(EgressError::Timeout)) => Err(SendFailure::Timeout),
            // A redirect is never followed for a POST, and the body is not read, so what is left is
            // a failure to talk to the receiver.
            Ok(Err(_)) => Err(SendFailure::Network),
        }
    }
}
