//! Validates the server wire response before it can become a target connection or statement.
use crate::RunnerError;
use sagat_api_grpc::pb;
use sagat_application::database::{DatabaseCredentials, DatabaseCredentialsCodec};
use sagat_domain::{
    database::{
        ConnectionConfig, ConnectionRef, DatabaseLimits, DatabaseOperation, OperationKind,
        Parameter, TlsPolicy, TypedValue,
    },
    identity::{NamespaceId, Slug},
    secrets::{MAX_SECRET_BYTES, SecretName, SecretValue},
};
use sagat_infra_secrets::database::JsonDatabaseCredentials;
use tokio::time::{Duration, Instant};
use zeroize::Zeroize;

pub(crate) struct Capability {
    pub tenant_id: sagat_domain::identity::TenantId,
    pub connection_id: uuid::Uuid,
    pub config: ConnectionConfig,
    pub operation: DatabaseOperation,
    pub credentials: DatabaseCredentials,
    pub deadline: Instant,
}
/// On all early failures, clear the credential allocation received from protobuf.
struct Guard(pb::ResolvedDatabase);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(credential) = &mut self.0.credential {
            credential.value.zeroize();
        }
    }
}
pub(crate) fn decode(
    response: pb::ResolvedDatabase,
    namespace: NamespaceId,
    ceiling: DatabaseLimits,
) -> Result<Capability, RunnerError> {
    let mut guarded = Guard(response);
    let response = &mut guarded.0;
    let invalid = || RunnerError::Capability;
    if response.namespace_id.parse::<NamespaceId>().ok() != Some(namespace)
        || response.connection_id.parse::<uuid::Uuid>().is_err()
        || response.connection_revision == 0
    {
        return Err(invalid());
    }
    let credential = response.credential.as_mut().ok_or_else(invalid)?;
    if credential.version == 0
        || credential.value.is_empty()
        || credential.value.len() > MAX_SECRET_BYTES
    {
        return Err(invalid());
    }
    let secret_name = SecretName::parse(&credential.name).map_err(|_| invalid())?;
    let secret = SecretValue::new(std::mem::take(&mut credential.value)).map_err(|_| invalid())?;
    let credentials = JsonDatabaseCredentials
        .decode(&secret)
        .map_err(|_| invalid())?;
    let limits = response.limits.take().ok_or_else(invalid)?;
    let limits = DatabaseLimits {
        max_statement_bytes: limits.max_statement_bytes,
        max_parameters: limits.max_parameters,
        max_parameter_bytes: limits.max_parameter_bytes,
        max_rows: limits.max_rows,
        max_result_bytes: limits.max_result_bytes,
        timeout_ms: limits.timeout_ms,
        connect_timeout_ms: limits.connect_timeout_ms,
        pool_size: limits.pool_size,
        idle_timeout_ms: limits.idle_timeout_ms,
    };
    limits.validate().map_err(|_| invalid())?;
    ceiling.validate().map_err(|_| invalid())?;
    let limits = limits.intersect(ceiling);
    let postgres = response.postgres.take().ok_or_else(invalid)?;
    let tls = match pb::DatabaseTlsMode::try_from(postgres.tls) {
        Ok(pb::DatabaseTlsMode::DatabaseTlsVerifyFull) => TlsPolicy::VerifyFull {
            ca_pem: (!postgres.ca_pem.is_empty()).then_some(postgres.ca_pem),
        },
        Ok(pb::DatabaseTlsMode::DatabaseTlsDevelopmentPlaintext) if postgres.ca_pem.is_empty() => {
            TlsPolicy::DevelopmentPlaintext
        }
        _ => return Err(invalid()),
    };
    let config = ConnectionConfig::postgres(
        &postgres.host,
        u16::try_from(postgres.port).map_err(|_| invalid())?,
        &postgres.database,
        tls,
        secret_name,
        limits,
    )
    .map_err(|_| invalid())?;
    let kind = match pb::DatabaseOperationKind::try_from(response.operation) {
        Ok(pb::DatabaseOperationKind::DatabaseQuery) => OperationKind::Query,
        Ok(pb::DatabaseOperationKind::DatabaseExecute) => OperationKind::Execute,
        _ => return Err(invalid()),
    };
    if response.parameters.len() > limits.max_parameters as usize {
        return Err(invalid());
    }
    let parameters = std::mem::take(&mut response.parameters)
        .into_iter()
        .map(value)
        .collect::<Result<Vec<_>, _>>()?;
    let reference = ConnectionRef::new(
        Slug::parse(&response.connection_name).map_err(|_| invalid())?,
        Some(response.connection_revision),
    )
    .map_err(|_| invalid())?;
    let operation =
        DatabaseOperation::new(reference, kind, &response.statement, parameters, limits)
            .map_err(|_| invalid())?;
    let deadline = deadline(response.lease_expires_at_epoch_ms).ok_or_else(invalid)?;
    Ok(Capability {
        tenant_id: response.tenant_id.parse().map_err(|_| invalid())?,
        connection_id: response.connection_id.parse().map_err(|_| invalid())?,
        config,
        operation,
        credentials,
        deadline,
    })
}
fn value(value: pb::DatabaseValue) -> Result<Parameter, RunnerError> {
    use pb::database_value::Value as Wire;
    let invalid = || RunnerError::Capability;
    let value = match value.value.ok_or_else(invalid)? {
        Wire::NullValue(true) => TypedValue::Null,
        Wire::NullValue(false) => return Err(invalid()),
        Wire::BooleanValue(v) => TypedValue::Boolean(v),
        Wire::IntegerValue(v) => TypedValue::Integer(v),
        Wire::DecimalValue(v) => TypedValue::Decimal(v),
        Wire::FloatValue(v) => TypedValue::float(v).map_err(|_| invalid())?,
        Wire::TextValue(v) => TypedValue::Text(v),
        Wire::TimestampValue(v) => TypedValue::Timestamp(
            time::OffsetDateTime::parse(&v, &time::format_description::well_known::Rfc3339)
                .map_err(|_| invalid())?,
        ),
        Wire::DateValue(v) => TypedValue::Date(
            time::Date::parse(
                &v,
                time::macros::format_description!("[year]-[month]-[day]"),
            )
            .map_err(|_| invalid())?,
        ),
        Wire::BinaryValue(v) => TypedValue::Binary(v),
        Wire::JsonValue(v) => {
            serde_json::from_str::<serde_json::Value>(&v).map_err(|_| invalid())?;
            TypedValue::Json(v)
        }
    };
    value.validate().map_err(|_| invalid())?;
    Ok(Parameter::Literal(value))
}
pub(crate) fn deadline(epoch_ms: i64) -> Option<Instant> {
    let expires =
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(epoch_ms) * 1_000_000).ok()?;
    let remaining = Duration::try_from(expires - time::OffsetDateTime::now_utc()).ok()?;
    // A malformed server cannot grant this client an unbounded lease lifetime.
    if remaining.is_zero() || remaining > Duration::from_secs(3600) {
        return None;
    }
    Some(Instant::now() + remaining)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn response(namespace: NamespaceId) -> pb::ResolvedDatabase {
        pb::ResolvedDatabase {
            tenant_id: uuid::Uuid::new_v4().to_string(),
            namespace_id: namespace.to_string(),
            connection_id: uuid::Uuid::new_v4().to_string(),
            connection_name: "warehouse".into(),
            connection_revision: 1,
            postgres: Some(pb::PostgresConnection {
                host: "db.example.com".into(),
                port: 5432,
                database: "orders".into(),
                tls: 1,
                ca_pem: String::new(),
            }),
            operation: 1,
            statement: "SELECT $1".into(),
            parameters: vec![pb::DatabaseValue {
                value: Some(pb::database_value::Value::IntegerValue(
                    "9223372036854775807".into(),
                )),
            }],
            credential: Some(pb::ResolvedSecret {
                name: "db".into(),
                version: 1,
                value: br#"{"username":"reader","password":"capability-canary"}"#.to_vec(),
            }),
            limits: Some(pb::DatabaseLimits {
                max_statement_bytes: 16384,
                max_parameters: 64,
                max_parameter_bytes: 65536,
                max_rows: 1000,
                max_result_bytes: 1048576,
                timeout_ms: 10000,
                connect_timeout_ms: 5000,
                pool_size: 4,
                idle_timeout_ms: 30000,
            }),
            lease_expires_at_epoch_ms: ((time::OffsetDateTime::now_utc()
                + time::Duration::seconds(30))
            .unix_timestamp_nanos()
                / 1_000_000) as i64,
        }
    }
    #[test]
    fn capability_requires_exact_scope_closed_wire_types_and_finite_limits() {
        let namespace = NamespaceId::new(uuid::Uuid::new_v4());
        let ceiling = DatabaseLimits {
            max_rows: 1,
            ..Default::default()
        };
        let capability = decode(response(namespace), namespace, ceiling).unwrap();
        assert_eq!(capability.operation.limits.max_rows, 1);
        assert_eq!(
            capability.operation.parameters(),
            &[Parameter::Literal(TypedValue::Integer(
                "9223372036854775807".into()
            ))]
        );
        assert!(!format!("{:?}", capability.credentials).contains("capability-canary"));
        assert!(
            decode(
                response(namespace),
                NamespaceId::new(uuid::Uuid::new_v4()),
                ceiling
            )
            .is_err()
        );
        for mutate in [
            |r: &mut pb::ResolvedDatabase| r.operation = 0,
            |r: &mut pb::ResolvedDatabase| r.connection_revision = 0,
            |r: &mut pb::ResolvedDatabase| r.lease_expires_at_epoch_ms = 0,
            |r: &mut pb::ResolvedDatabase| r.limits.as_mut().unwrap().max_result_bytes = 0,
            |r: &mut pb::ResolvedDatabase| {
                r.parameters[0].value = Some(pb::database_value::Value::FloatValue(f64::NAN))
            },
            |r: &mut pb::ResolvedDatabase| {
                r.parameters[0].value =
                    Some(pb::database_value::Value::JsonValue("bad json".into()))
            },
            |r: &mut pb::ResolvedDatabase| {
                r.parameters[0].value = Some(pb::database_value::Value::NullValue(false))
            },
            |r: &mut pb::ResolvedDatabase| {
                r.credential.as_mut().unwrap().value = b"not a credential".to_vec()
            },
        ] {
            let mut response = response(namespace);
            mutate(&mut response);
            assert!(decode(response, namespace, ceiling).is_err());
        }
    }
}
