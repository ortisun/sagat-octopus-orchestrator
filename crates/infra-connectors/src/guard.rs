//! The egress policy for brokers a library connects to by itself.
//!
//! An HTTP request goes through the guarded client, which checks every address it connects to. A
//! NATS or Kafka client library opens its own sockets, so the policy is applied before it does: the
//! broker's name must be allowed, and every address it resolves to must be one the policy allows
//! (loopback, link-local, metadata and the like never are). What a broker then tells the client
//! about *other* brokers (Kafka's advertised listeners) the library connects to without asking; a
//! limit of this design, recorded as a gap, and the reason the default policy of a namespace, which
//! allows nothing, matters.

use std::future::Future;

use sagat_application::clock::SystemClock;
use sagat_application::egress::{EgressService, EgressStore};
use sagat_domain::connectors::Server;
use sagat_domain::identity::{NamespaceId, TenantId};

/// Resolution preserves the difference between a policy refusal and unavailable infrastructure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveFailure {
    Denied,
    Network,
}

/// Decides whether a connector may connect to its brokers.
pub trait BrokerGuard: Send + Sync {
    /// Returns the exact checked addresses for a client that supports DNS pinning.
    /// Custom guards fail closed unless they explicitly implement this capability.
    fn resolve(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
        _server: &Server,
    ) -> impl Future<Output = Result<Vec<std::net::SocketAddr>, ResolveFailure>> + Send {
        async { Err(ResolveFailure::Denied) }
    }

    /// `true` when every one of `servers` may be connected to by `namespace`.
    fn permits(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        servers: &[Server],
    ) -> impl Future<Output = bool> + Send;
}

/// The guard of the platform: the namespace's egress policy, on the name and on every address.
pub struct PolicyGuard<S> {
    egress: EgressService<S, SystemClock>,
}

impl<S: EgressStore> PolicyGuard<S> {
    #[must_use]
    pub fn new(store: S) -> Self {
        Self {
            egress: EgressService::new(store, SystemClock),
        }
    }
}

impl<S: EgressStore> BrokerGuard for PolicyGuard<S> {
    async fn permits(&self, tenant: TenantId, namespace: NamespaceId, servers: &[Server]) -> bool {
        for server in servers {
            if self.resolve(tenant, namespace, server).await.is_err() {
                return false;
            }
        }
        true
    }

    async fn resolve(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        server: &Server,
    ) -> Result<Vec<std::net::SocketAddr>, ResolveFailure> {
        let policy = self
            .egress
            .policy_for(tenant, namespace)
            .await
            .map_err(|_| ResolveFailure::Network)?;
        if let Err(denial) = policy.check_destination(&server.destination()) {
            self.egress
                .record_denial(tenant, namespace, &server.destination(), denial)
                .await;
            return Err(ResolveFailure::Denied);
        }
        let addresses: Vec<_> = tokio::net::lookup_host((server.host(), server.port()))
            .await
            .map_err(|_| ResolveFailure::Network)?
            .collect();
        if addresses.is_empty() {
            return Err(ResolveFailure::Network);
        }
        for address in &addresses {
            if let Err(denial) = policy.check_address(address.ip()) {
                self.egress
                    .record_denial(tenant, namespace, &server.destination(), denial)
                    .await;
                return Err(ResolveFailure::Denied);
            }
        }
        Ok(addresses)
    }
}

/// A guard that permits everything: for tests that reach brokers on this machine, which no policy
/// can allow. It exists only when the crate is built with the `unguarded-for-tests` feature, which
/// no binary of the platform enables.
#[cfg(feature = "unguarded-for-tests")]
#[derive(Debug, Clone, Copy, Default)]
pub struct Unguarded;

#[cfg(feature = "unguarded-for-tests")]
impl BrokerGuard for Unguarded {
    async fn resolve(
        &self,
        _: TenantId,
        _: NamespaceId,
        server: &Server,
    ) -> Result<Vec<std::net::SocketAddr>, ResolveFailure> {
        tokio::net::lookup_host((server.host(), server.port()))
            .await
            .map(Iterator::collect)
            .map_err(|_| ResolveFailure::Network)
    }

    async fn permits(&self, _: TenantId, _: NamespaceId, _: &[Server]) -> bool {
        true
    }
}
