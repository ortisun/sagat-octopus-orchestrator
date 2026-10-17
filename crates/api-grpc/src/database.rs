//! Typed managed capability wire boundary. Never format SQL, parameter or credential values.
use crate::pb;
use sagat_application::{database::ResolvedDatabase, error::AppError};
use sagat_domain::database::{ConnectionConfig, OperationKind, Parameter, TlsPolicy, TypedValue};

pub(crate) fn response(resolved: ResolvedDatabase) -> Result<pb::ResolvedDatabase, AppError> {
    let ConnectionConfig::Postgres {
        host,
        port,
        database,
        tls,
        ..
    } = resolved.connection.config;
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
    let l = resolved.operation.limits;
    Ok(pb::ResolvedDatabase {
        tenant_id: resolved.connection.tenant_id.to_string(),
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
        operation: match resolved.operation.kind {
            OperationKind::Query => pb::DatabaseOperationKind::DatabaseQuery,
            OperationKind::Execute => pb::DatabaseOperationKind::DatabaseExecute,
        } as i32,
        statement: resolved.operation.statement().to_owned(),
        parameters: resolved
            .operation
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
            name: resolved.credential.name.to_string(),
            version: resolved.credential.version,
            value: resolved.credential.value.expose().to_vec(),
        }),
        lease_expires_at_epoch_ms: i64::try_from(
            resolved.expires_at.unix_timestamp_nanos() / 1_000_000,
        )
        .map_err(|_| AppError::Internal)?,
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
