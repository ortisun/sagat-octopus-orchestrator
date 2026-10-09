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

/// Decides whether a connector may connect to its brokers.
pub trait BrokerGuard: Send + Sync {
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
        let Ok(policy) = self.egress.policy_for(tenant, namespace).await else {
            return false;
        };
        for server in servers {
            // The first stage: the name and the port.
            if let Err(denial) = policy.check_destination(&server.destination()) {
                self.egress
                    .record_denial(tenant, namespace, &server.destination(), denial)
                    .await;
                return false;
            }
            // The second: every address the name resolves to. One that is out of bounds refuses the
            // whole answer, like the guarded HTTP client does.
            let Ok(addresses) = tokio::net::lookup_host((server.host(), server.port())).await
            else {
                return false;
            };
            let mut any = false;
            for address in addresses {
                any = true;
                if let Err(denial) = policy.check_address(address.ip()) {
                    self.egress
                        .record_denial(tenant, namespace, &server.destination(), denial)
                        .await;
                    return false;
                }
            }
            if !any {
                return false;
            }
        }
        true
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
    async fn permits(&self, _: TenantId, _: NamespaceId, _: &[Server]) -> bool {
        true
    }
}
