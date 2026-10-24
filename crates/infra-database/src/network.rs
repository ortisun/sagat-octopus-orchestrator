use crate::ExecutionError;
use ipnet::IpNet;
use std::net::{IpAddr, SocketAddr};

/// A bounded destination scope shared by deployment and namespace policy. Empty sets deny access.
#[derive(Debug, Clone)]
pub struct DestinationScope {
    networks: Vec<IpNet>,
    ports: Vec<u16>,
}
impl DestinationScope {
    pub fn new(networks: Vec<IpNet>, ports: Vec<u16>) -> Result<Self, ExecutionError> {
        if networks.len() > 256 || ports.len() > 256 || ports.contains(&0) {
            return Err(ExecutionError::InvalidConfiguration);
        }
        Ok(Self { networks, ports })
    }
}

/// Operator allowlist, namespace narrowing and platform/metadata exclusions intersect.
/// An empty allowlist denies everything. Entries are IP networks, never unresolved host patterns.
#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    allow: DestinationScope,
    deny: Vec<IpNet>,
    development: bool,
}
impl NetworkPolicy {
    pub fn new(
        allow: DestinationScope,
        deny: Vec<IpNet>,
        development: bool,
    ) -> Result<Self, ExecutionError> {
        if deny.len() > 256 {
            return Err(ExecutionError::InvalidConfiguration);
        }
        Ok(Self {
            allow,
            deny,
            development,
        })
    }
    pub fn allowed(&self) -> &DestinationScope {
        &self.allow
    }
    pub fn development(&self) -> bool {
        self.development
    }
    fn validate_addresses(
        &self,
        addresses: &[IpAddr],
        namespace: &DestinationScope,
    ) -> Result<(), ExecutionError> {
        if addresses.is_empty() || addresses.len() > 16 || namespace.networks.len() > 256 {
            return Err(ExecutionError::DestinationDenied);
        }
        for &address in addresses {
            let address = address.to_canonical();
            if prohibited(address, self.development)
                || self.deny.iter().any(|net| net.contains(&address))
                || !self.allow.networks.iter().any(|net| net.contains(&address))
                || !namespace.networks.iter().any(|net| net.contains(&address))
            {
                return Err(ExecutionError::DestinationDenied);
            }
        }
        Ok(())
    }
    fn validate_port(&self, port: u16, namespace: &DestinationScope) -> Result<(), ExecutionError> {
        if !self.allow.ports.contains(&port) || !namespace.ports.contains(&port) {
            return Err(ExecutionError::DestinationDenied);
        }
        Ok(())
    }
    pub fn validate_destination(
        &self,
        addresses: &[IpAddr],
        port: u16,
        namespace: &DestinationScope,
    ) -> Result<(), ExecutionError> {
        self.validate_port(port, namespace)?;
        self.validate_addresses(addresses, namespace)
    }
    pub(crate) async fn resolve(
        &self,
        host: &str,
        port: u16,
        namespace: &DestinationScope,
    ) -> Result<SocketAddr, ExecutionError> {
        self.validate_port(port, namespace)?;
        // One resolution only. Reject mixed safe/unsafe answers; connect to the validated IP.
        let addresses: Vec<_> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| ExecutionError::Connection)?
            .take(17)
            .map(|a| a.ip())
            .collect();
        self.validate_addresses(&addresses, namespace)?;
        let pinned = addresses
            .iter()
            .copied()
            .find(IpAddr::is_ipv4)
            .unwrap_or(addresses[0]);
        Ok(SocketAddr::new(pinned, port))
    }
}
fn prohibited(ip: IpAddr, development: bool) -> bool {
    if ip.is_loopback() {
        return !development;
    }
    match ip {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            o[0] == 0
                || o[0] >= 224
                || ip.is_link_local()
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || o == [100, 100, 100, 200]
                || o == [168, 63, 129, 16]
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Global unicast or explicitly allowed ULA only. Exclude 6to4 and Teredo translation.
            !((s[0] & 0xe000 == 0x2000) || (s[0] & 0xfe00 == 0xfc00))
                || s[0] == 0x2002
                || (s[0] == 0x2001 && s[1] == 0)
        }
    }
}
