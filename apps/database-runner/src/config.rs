//! Operator-owned configuration. Policy is read again before every attempt; values are bounded.
use crate::RunnerError;
use sagat_domain::{
    database::DatabaseLimits,
    identity::{NamespaceId, Slug},
};
use sagat_infra_database::{DestinationScope, NetworkPolicy};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncReadExt;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    pub grpc_url: String,
    pub token_file: PathBuf,
    pub ca_file: Option<PathBuf>,
    pub name: String,
    pub max_concurrent: u16,
    pub poll_interval_ms: u32,
    pub rpc_timeout_ms: u32,
    #[serde(default)]
    pub development_loopback: bool,
    pub deployment: DeploymentPolicy,
    pub namespaces: Vec<NamespacePolicy>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeploymentPolicy {
    pub allow_cidrs: Vec<String>,
    pub allow_ports: Vec<u16>,
    pub deny_cidrs: Vec<String>,
    pub limits: Limits,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NamespacePolicy {
    pub id: String,
    pub slug: String,
    pub allow_cidrs: Vec<String>,
    pub allow_ports: Vec<u16>,
    pub limits: Limits,
}
/// Deployment policy ceilings have independent semantics from per-operation requests.
#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Limits {
    pub max_statement_bytes: u32,
    pub max_parameters: u32,
    pub max_parameter_bytes: u32,
    pub max_rows: u32,
    pub max_result_bytes: u32,
    pub timeout_ms: u32,
    pub connect_timeout_ms: u32,
    pub pool_size: u32,
    pub idle_timeout_ms: u32,
}
impl Limits {
    pub fn validated(self) -> Result<DatabaseLimits, RunnerError> {
        let limits = DatabaseLimits {
            max_statement_bytes: self.max_statement_bytes,
            max_parameters: self.max_parameters,
            max_parameter_bytes: self.max_parameter_bytes,
            max_rows: self.max_rows,
            max_result_bytes: self.max_result_bytes,
            timeout_ms: self.timeout_ms,
            connect_timeout_ms: self.connect_timeout_ms,
            pool_size: self.pool_size,
            idle_timeout_ms: self.idle_timeout_ms,
        };
        limits.validate().map_err(|_| RunnerError::Configuration)?;
        Ok(limits)
    }
}
impl Config {
    pub async fn read(path: &Path) -> Result<Self, RunnerError> {
        let bytes = bounded_file(path, 256 * 1024).await?;
        Self::parse(&bytes)
    }
    pub(crate) fn with_name(mut self, name: String) -> Result<Self, RunnerError> {
        self.name = name;
        self.validate()?;
        Ok(self)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, RunnerError> {
        if bytes.len() > 256 * 1024 {
            return Err(RunnerError::Configuration);
        }
        let config: Self = serde_json::from_slice(bytes).map_err(|_| RunnerError::Configuration)?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), RunnerError> {
        let invalid = || RunnerError::Configuration;
        let endpoint = url::Url::parse(&self.grpc_url).map_err(|_| invalid())?;
        let loopback = endpoint.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !matches!(endpoint.path(), "" | "/")
            || endpoint.host_str().is_none()
            || !(endpoint.scheme() == "https"
                || (self.development_loopback && loopback && endpoint.scheme() == "http"))
            || self.name.is_empty()
            || self.name.len() > 64
            || self.name.chars().any(char::is_control)
            || self.token_file.as_os_str().is_empty()
            || !(1..=64).contains(&self.max_concurrent)
            || !(100..=5000).contains(&self.poll_interval_ms)
            || !(100..=10000).contains(&self.rpc_timeout_ms)
            || self.namespaces.is_empty()
            || self.namespaces.len() > 32
            || self.deployment.deny_cidrs.len() > 256
            || (!self.development_loopback && self.deployment.deny_cidrs.is_empty())
        {
            return Err(invalid());
        }
        self.network()?;
        self.deployment.limits.validated()?;
        let mut ids = std::collections::BTreeSet::new();
        let mut slugs = std::collections::BTreeSet::new();
        for namespace in &self.namespaces {
            let id = namespace.id.parse::<NamespaceId>().map_err(|_| invalid())?;
            Slug::parse(&namespace.slug).map_err(|_| invalid())?;
            if self.name.len() + 1 + namespace.slug.len() > 120 {
                return Err(invalid());
            }
            if !ids.insert(id) || !slugs.insert(&namespace.slug) {
                return Err(invalid());
            }
            namespace.scope()?;
            namespace.limits.validated()?;
        }
        Ok(())
    }
    pub fn network(&self) -> Result<NetworkPolicy, RunnerError> {
        NetworkPolicy::new(
            scope(&self.deployment.allow_cidrs, &self.deployment.allow_ports)?,
            networks(&self.deployment.deny_cidrs)?,
            self.development_loopback,
        )
        .map_err(|_| RunnerError::Configuration)
    }
    pub fn rpc_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.rpc_timeout_ms))
    }
    pub fn policy(&self, id: NamespaceId, slug: &str) -> Result<&NamespacePolicy, RunnerError> {
        self.namespaces
            .iter()
            .find(|entry| {
                entry
                    .id
                    .parse::<NamespaceId>()
                    .is_ok_and(|stored| stored == id)
                    && entry.slug == slug
            })
            .ok_or(RunnerError::Configuration)
    }
}
impl NamespacePolicy {
    pub fn scope(&self) -> Result<DestinationScope, RunnerError> {
        scope(&self.allow_cidrs, &self.allow_ports)
    }
}
fn networks(values: &[String]) -> Result<Vec<ipnet::IpNet>, RunnerError> {
    if values.len() > 256 {
        return Err(RunnerError::Configuration);
    }
    values
        .iter()
        .map(|entry| entry.parse().map_err(|_| RunnerError::Configuration))
        .collect()
}
fn scope(cidrs: &[String], ports: &[u16]) -> Result<DestinationScope, RunnerError> {
    DestinationScope::new(networks(cidrs)?, ports.to_vec()).map_err(|_| RunnerError::Configuration)
}
pub(crate) async fn bounded_file(
    path: &Path,
    limit: usize,
) -> Result<zeroize::Zeroizing<Vec<u8>>, RunnerError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| RunnerError::Configuration)?;
    if !file
        .metadata()
        .await
        .map_err(|_| RunnerError::Configuration)?
        .is_file()
    {
        return Err(RunnerError::Configuration);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| RunnerError::Configuration)?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(RunnerError::Configuration);
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    fn document() -> serde_json::Value {
        let limits = json!({"maxStatementBytes":16384,"maxParameters":64,"maxParameterBytes":65536,
            "maxRows":1000,"maxResultBytes":1048576,"timeoutMs":10000,"connectTimeoutMs":5000,"poolSize":4,"idleTimeoutMs":30000});
        json!({"grpcUrl":"https://engine.example.com:8443","tokenFile":"/run/identity/token","name":"database",
            "maxConcurrent":4,"pollIntervalMs":250,"rpcTimeoutMs":5000,
            "deployment":{"allowCidrs":["10.0.0.0/8"],"allowPorts":[5432],"denyCidrs":["10.0.0.1/32"],"limits":limits},
            "namespaces":[{"id":"00000000-0000-0000-0000-000000000001","slug":"payments","allowCidrs":["10.2.0.0/16"],"allowPorts":[5432],"limits":limits}]})
    }
    #[test]
    fn configuration_requires_verified_transport_and_intersected_bounded_policy() {
        let config = Config::parse(&serde_json::to_vec(&document()).unwrap()).unwrap();
        assert!(config.clone().with_name("database-runner-0".into()).is_ok());
        for name in [String::new(), "x".repeat(65), "pod\nname".into()] {
            assert!(config.clone().with_name(name).is_err());
        }
        let network = config.network().unwrap();
        let ns = config.namespaces[0].scope().unwrap();
        assert!(
            network
                .validate_destination(&["10.2.1.2".parse().unwrap()], 5432, &ns)
                .is_ok()
        );
        for ip in ["10.3.0.1", "10.0.0.1", "169.254.169.254", "127.0.0.1"] {
            assert!(
                network
                    .validate_destination(&[ip.parse().unwrap()], 5432, &ns)
                    .is_err()
            );
        }
        assert!(
            network
                .validate_destination(&["10.2.1.2".parse().unwrap()], 5433, &ns)
                .is_err()
        );
        for endpoint in [
            "http://engine.example.com",
            "http://127.0.0.1",
            "https://user:credential-canary@host",
            "https://host/path",
            "https://host?token=credential-canary",
        ] {
            let mut value = document();
            value["grpcUrl"] = json!(endpoint);
            assert!(Config::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for mutate in [
            |v: &mut serde_json::Value| v["maxConcurrent"] = json!(0),
            |v: &mut serde_json::Value| v["rpcTimeoutMs"] = json!(10001),
            |v: &mut serde_json::Value| v["namespaces"] = json!([]),
            |v: &mut serde_json::Value| v["deployment"]["denyCidrs"] = json!([]),
            |v: &mut serde_json::Value| v["deployment"]["limits"]["timeoutMs"] = json!(30001),
            |v: &mut serde_json::Value| v["platformDatabasePassword"] = json!("credential-canary"),
        ] {
            let mut value = document();
            mutate(&mut value);
            let error = Config::parse(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap();
            assert!(!error.to_string().contains("credential-canary"));
        }
        let mut value = document();
        value["developmentLoopback"] = json!(true);
        value["grpcUrl"] = json!("http://127.0.0.1:8080");
        assert!(Config::parse(&serde_json::to_vec(&value).unwrap()).is_ok());
        value["grpcUrl"] = json!("http://engine.example.com:8080");
        assert!(Config::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[tokio::test]
    async fn configuration_is_bounded_and_policy_revocation_is_visible_on_reread() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runner.json");
        let mut value = document();
        tokio::fs::write(&path, serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
        let old = Config::read(&path).await.unwrap();
        let id = old.namespaces[0].id.parse().unwrap();
        value["namespaces"][0]["id"] = json!("00000000-0000-0000-0000-000000000002");
        tokio::fs::write(&path, serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
        assert!(
            Config::read(&path)
                .await
                .unwrap()
                .policy(id, "payments")
                .is_err()
        );
        tokio::fs::write(&path, vec![b' '; 256 * 1024 + 1])
            .await
            .unwrap();
        assert!(Config::read(&path).await.is_err());
        assert!(bounded_file(directory.path(), 10).await.is_err());
    }
}
