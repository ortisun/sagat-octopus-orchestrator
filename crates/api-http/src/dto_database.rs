//! Public connection contract: settings and secret references, never credentials/provider routing.
use sagat_application::{database::ConnectionRecord, error::AppError};
use sagat_domain::{
    database::{ConnectionConfig, ConnectionStatus, DatabaseLimits, TlsPolicy},
    secrets::SecretName,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LimitsBody {
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
impl From<DatabaseLimits> for LimitsBody {
    fn from(v: DatabaseLimits) -> Self {
        Self {
            max_statement_bytes: v.max_statement_bytes,
            max_parameters: v.max_parameters,
            max_parameter_bytes: v.max_parameter_bytes,
            max_rows: v.max_rows,
            max_result_bytes: v.max_result_bytes,
            timeout_ms: v.timeout_ms,
            connect_timeout_ms: v.connect_timeout_ms,
            pool_size: v.pool_size,
            idle_timeout_ms: v.idle_timeout_ms,
        }
    }
}
impl Default for LimitsBody {
    fn default() -> Self {
        DatabaseLimits::default().into()
    }
}
impl From<LimitsBody> for DatabaseLimits {
    fn from(v: LimitsBody) -> Self {
        Self {
            max_statement_bytes: v.max_statement_bytes,
            max_parameters: v.max_parameters,
            max_parameter_bytes: v.max_parameter_bytes,
            max_rows: v.max_rows,
            max_result_bytes: v.max_result_bytes,
            timeout_ms: v.timeout_ms,
            connect_timeout_ms: v.connect_timeout_ms,
            pool_size: v.pool_size,
            idle_timeout_ms: v.idle_timeout_ms,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum TlsBody {
    #[serde(rename = "verify-full")]
    VerifyFull {
        #[serde(rename = "caPem", skip_serializing_if = "Option::is_none")]
        ca_pem: Option<String>,
    },
    #[serde(rename = "development-plaintext")]
    DevelopmentPlaintext,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "driver", deny_unknown_fields)]
pub enum ConfigBody {
    #[serde(rename = "postgresql")]
    Postgres {
        host: String,
        port: u16,
        database: String,
        tls: TlsBody,
        #[serde(rename = "secretName")]
        secret_name: String,
        #[serde(default)]
        limits: LimitsBody,
    },
}
impl ConfigBody {
    pub fn into_domain(self) -> Result<ConnectionConfig, AppError> {
        let Self::Postgres {
            host,
            port,
            database,
            tls,
            secret_name,
            limits,
        } = self;
        let tls = match tls {
            TlsBody::VerifyFull { ca_pem } => TlsPolicy::VerifyFull { ca_pem },
            TlsBody::DevelopmentPlaintext => TlsPolicy::DevelopmentPlaintext,
        };
        let secret = SecretName::parse(&secret_name)
            .map_err(|_| AppError::NotAllowed("invalid database secret reference"))?;
        ConnectionConfig::postgres(&host, port, &database, tls, secret, limits.into())
            .map_err(|_| AppError::NotAllowed("invalid database connection configuration"))
    }
}
impl From<&ConnectionConfig> for ConfigBody {
    fn from(v: &ConnectionConfig) -> Self {
        let ConnectionConfig::Postgres {
            host,
            port,
            database,
            tls,
            secret,
            limits,
            // The API does not carry the egress ceiling yet (feature 035, US01): the request body refuses it, so a
            // connection written through the API is always `internal`, which a response without it also says.
            egress_ceiling: _,
        } = v;
        let tls = match tls {
            TlsPolicy::VerifyFull { ca_pem } => TlsBody::VerifyFull {
                ca_pem: ca_pem.clone(),
            },
            TlsPolicy::DevelopmentPlaintext => TlsBody::DevelopmentPlaintext,
        };
        Self::Postgres {
            host: host.clone(),
            port: *port,
            database: database.clone(),
            tls,
            secret_name: secret.to_string(),
            limits: (*limits).into(),
        }
    }
}
#[derive(Deserialize, Serialize)]
pub enum StatusBody {
    #[serde(rename = "ACTIVE")]
    Active,
    #[serde(rename = "PAUSED")]
    Paused,
}
impl From<StatusBody> for ConnectionStatus {
    fn from(v: StatusBody) -> Self {
        match v {
            StatusBody::Active => Self::Active,
            StatusBody::Paused => Self::Paused,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PutConnectionBody {
    pub request_id: String,
    pub expected_revision: u32,
    pub config: ConfigBody,
    pub status: StatusBody,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteConnectionBody {
    pub request_id: String,
    pub expected_revision: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionBody {
    pub id: String,
    pub namespace_id: String,
    pub name: String,
    pub revision: u32,
    pub status: &'static str,
    pub config: ConfigBody,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl From<&ConnectionRecord> for ConnectionBody {
    fn from(v: &ConnectionRecord) -> Self {
        Self {
            id: v.id.to_string(),
            namespace_id: v.namespace_id.to_string(),
            name: v.name.to_string(),
            revision: v.revision,
            status: v.status.as_str(),
            config: (&v.config).into(),
            created_at: v.created_at,
            updated_at: v.updated_at,
        }
    }
}
#[derive(Serialize)]
pub struct ConnectionListBody {
    pub items: Vec<ConnectionBody>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionQuery {
    pub revision: Option<u32>,
}

/// Implemented server capabilities; a healthy runner and target access still require a probe.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDriverBody {
    pub id: &'static str,
    pub operations: [&'static str; 2],
    pub parameter_types: [&'static str; 10],
    pub default_limits: LimitsBody,
    pub maximum_limits: LimitsBody,
}
#[derive(Serialize)]
pub struct DatabaseDriverListBody {
    pub items: [DatabaseDriverBody; 1],
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use sagat_domain::classification::EgressCeiling;
    use serde_json::json;

    fn body() -> serde_json::Value {
        json!({
            "driver": "postgresql", "host": "db.example.com", "port": 5432, "database": "orders",
            "tls": {"mode": "verify-full"}, "secretName": "orders-access"
        })
    }

    /// G035-027 item 7: the store holds the ceiling, the endpoint that sets it is the connections work of US01. Until
    /// then a request that names it is refused and not silently stored as `internal`.
    #[test]
    fn a_request_that_names_the_egress_ceiling_is_refused() {
        assert!(serde_json::from_value::<ConfigBody>(body()).is_ok());
        for ceiling in ["public", "internal", "pii"] {
            let mut with_ceiling = body();
            with_ceiling["egressCeiling"] = json!(ceiling);
            assert!(
                serde_json::from_value::<ConfigBody>(with_ceiling).is_err(),
                "{ceiling}"
            );
        }
    }

    /// The response says nothing about the ceiling, so what a connection with a `pii` ceiling returns is what one with the
    /// default returns, and the field cannot leak through the response before the contract carries it.
    #[test]
    fn the_response_does_not_carry_the_egress_ceiling() {
        let config = ConnectionConfig::postgres(
            "db.example.com",
            5432,
            "orders",
            TlsPolicy::VerifyFull { ca_pem: None },
            SecretName::parse("orders-access").expect("a secret name"),
            DatabaseLimits::default(),
        )
        .expect("a valid connection");
        let default = serde_json::to_value(ConfigBody::from(&config)).expect("serializes");
        let pii = serde_json::to_value(ConfigBody::from(
            &config.with_egress_ceiling(EgressCeiling::Pii),
        ))
        .expect("serializes");
        assert_eq!(default, pii);
        assert!(default.get("egressCeiling").is_none());
    }
}
