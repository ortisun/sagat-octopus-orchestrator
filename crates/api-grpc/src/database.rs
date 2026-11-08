//! Typed managed capability wire boundary. Never format SQL, parameter or credential values.
use crate::lake_connection::{LakeSecrets, iceberg_connection};
use crate::pb;
use sagat_application::{
    database::{ResolvedDatabase, ResolvedGrant, ResolvedLake},
    error::AppError,
};
use sagat_domain::database::{ConnectionConfig, OperationKind, Parameter, TlsPolicy, TypedValue};

/// The answer to a managed step, by what the server resolved for it: a statement with its credential for a SQL step, a connection with its
/// secrets for a lake step. A grant over a connection of the other kind is refused, never answered.
pub fn response(resolved: ResolvedDatabase) -> Result<pb::ResolvedDatabase, AppError> {
    let (operation, credential) = match resolved.grant {
        ResolvedGrant::Sql {
            operation,
            credential,
        } => (operation, credential),
        ResolvedGrant::Lake(lake) => {
            return lake_response(&resolved.connection, &lake, resolved.expires_at);
        }
    };
    let ConnectionConfig::Postgres {
        host,
        port,
        database,
        tls,
        ..
    } = resolved.connection.config
    else {
        // The lake kinds are answered by the lake profile (feature 035, T035-026); the SQL profile has no answer for them.
        return Err(AppError::NotAllowed(
            "this connection is not served by the SQL profile",
        ));
    };
    let (tls, ca_pem) = match tls {
        TlsPolicy::VerifyFull { ca_pem } => (
            pb::DatabaseTlsMode::DatabaseTlsVerifyFull,
            ca_pem.unwrap_or_default(),
        ),
        TlsPolicy::DevelopmentPlaintext => (
            pb::DatabaseTlsMode::DatabaseTlsDevelopmentPlaintext,
            String::new(),
        ),
    };
    let l = operation.limits;
    Ok(pb::ResolvedDatabase {
        tenant_id: resolved.connection.tenant_id.to_string(),
        // The lake kinds are answered by the lake profile (feature 035); a PostgreSQL connection sets none of them.
        iceberg: None,
        object_storage: None,
        clickhouse: None,
        namespace_id: resolved.connection.namespace_id.to_string(),
        connection_name: resolved.connection.name.to_string(),
        connection_id: resolved.connection.id.to_string(),
        connection_revision: resolved.connection.revision,
        postgres: Some(pb::PostgresConnection {
            host,
            port: u32::from(port),
            database,
            tls: tls as i32,
            ca_pem,
        }),
        operation: match operation.kind {
            OperationKind::Query => pb::DatabaseOperationKind::DatabaseQuery,
            OperationKind::Execute => pb::DatabaseOperationKind::DatabaseExecute,
        } as i32,
        statement: operation.statement().to_owned(),
        parameters: operation
            .parameters()
            .iter()
            .map(|p| match p {
                Parameter::Literal(v) => value(v),
                Parameter::Expression { .. } => Err(AppError::Internal),
            })
            .collect::<Result<Vec<_>, _>>()?,
        limits: Some(pb::DatabaseLimits {
            max_statement_bytes: l.max_statement_bytes,
            max_parameters: l.max_parameters,
            max_parameter_bytes: l.max_parameter_bytes,
            max_rows: l.max_rows,
            max_result_bytes: l.max_result_bytes,
            timeout_ms: l.timeout_ms,
            connect_timeout_ms: l.connect_timeout_ms,
            pool_size: l.pool_size,
            idle_timeout_ms: l.idle_timeout_ms,
        }),
        credential: Some(pb::ResolvedSecret {
            name: credential.name.to_string(),
            version: credential.version,
            value: credential.value.expose().to_vec(),
        }),
        lease_expires_at_epoch_ms: i64::try_from(
            resolved.expires_at.unix_timestamp_nanos() / 1_000_000,
        )
        .map_err(|_| AppError::Internal)?,
    })
}
/// A lake step: the connection as it is stored, the secrets the server resolved under their roles, and nothing of SQL. The secrets
/// that travel are exactly the ones the connection names: a grant that has one it does not name, or lacks one it does, is not
/// answered (the store selects them from the connection, so it cannot happen; the check is for the day it does).
fn lake_response(
    connection: &sagat_application::database::ConnectionRecord,
    lake: &ResolvedLake,
    expires_at: time::OffsetDateTime,
) -> Result<pb::ResolvedDatabase, AppError> {
    let refused = || AppError::NotAllowed("this connection is not served by the lake profile");
    let config = connection.config.as_iceberg().ok_or_else(refused)?;
    if config.catalog_secret.is_some() != lake.catalog.is_some()
        || config.storage_secret.is_some() != lake.storage.is_some()
    {
        return Err(refused());
    }
    let secrets = LakeSecrets {
        catalog: lake.catalog.as_ref().map(|secret| secret.value.expose()),
        storage: lake.storage.as_ref().map(|secret| secret.value.expose()),
    };
    Ok(pb::ResolvedDatabase {
        tenant_id: connection.tenant_id.to_string(),
        namespace_id: connection.namespace_id.to_string(),
        connection_name: connection.name.to_string(),
        connection_id: connection.id.to_string(),
        connection_revision: connection.revision,
        iceberg: Some(iceberg_connection(config, &secrets)),
        lease_expires_at_epoch_ms: i64::try_from(expires_at.unix_timestamp_nanos() / 1_000_000)
            .map_err(|_| AppError::Internal)?,
        ..Default::default()
    })
}

fn value(value: &TypedValue) -> Result<pb::DatabaseValue, AppError> {
    use pb::database_value::Value;
    Ok(pb::DatabaseValue {
        value: Some(match value {
            TypedValue::Null => Value::NullValue(true),
            TypedValue::Boolean(b) => Value::BooleanValue(*b),
            TypedValue::Integer(s) => Value::IntegerValue(s.clone()),
            TypedValue::Decimal(s) => Value::DecimalValue(s.clone()),
            TypedValue::Float(f) => Value::FloatValue(f.get()),
            TypedValue::Text(s) => Value::TextValue(s.clone()),
            TypedValue::Timestamp(t) => Value::TimestampValue(
                t.format(&time::format_description::well_known::Rfc3339)
                    .map_err(|_| AppError::Internal)?,
            ),
            TypedValue::Date(d) => Value::DateValue(
                d.format(time::macros::format_description!("[year]-[month]-[day]"))
                    .map_err(|_| AppError::Internal)?,
            ),
            TypedValue::Binary(b) => Value::BinaryValue(b.clone()),
            TypedValue::Json(s) => Value::JsonValue(s.clone()),
        }),
    })
}
