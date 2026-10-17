//! Database IR wire boundary. Failures never echo SQL, parameters or expressions.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sagat_application::definitions::{IrError, IrIssue};
use sagat_domain::{
    database::{
        ConnectionRef, DatabaseError, DatabaseLimits, DatabaseOperation, DatabaseType,
        OperationKind, Parameter, TypedValue,
    },
    identity::Slug,
    workflow::Expression,
};
use serde_json::{Map, Value};

fn invalid() -> DatabaseError {
    DatabaseError::InvalidValue
}

fn field<'a>(node: &'a Map<String, Value>, name: &str) -> Result<&'a str, DatabaseError> {
    node.get(name).and_then(Value::as_str).ok_or_else(invalid)
}

fn value_type(kind: &str) -> Result<DatabaseType, DatabaseError> {
    Ok(match kind {
        "null" => DatabaseType::Null,
        "boolean" => DatabaseType::Boolean,
        "integer" => DatabaseType::Integer,
        "decimal" => DatabaseType::Decimal,
        "float" => DatabaseType::Float,
        "text" => DatabaseType::Text,
        "timestamp" => DatabaseType::Timestamp,
        "date" => DatabaseType::Date,
        "binary" => DatabaseType::Binary,
        "json" => DatabaseType::Json,
        _ => return Err(invalid()),
    })
}

fn literal(node: &Map<String, Value>) -> Result<TypedValue, DatabaseError> {
    let value = node.get("value").ok_or_else(invalid)?;
    typed_value(value_type(field(node, "type")?)?, value)
}

pub(crate) fn typed_value(kind: DatabaseType, value: &Value) -> Result<TypedValue, DatabaseError> {
    let string = || value.as_str().ok_or_else(invalid);
    let result = match kind {
        DatabaseType::Null if value.is_null() => TypedValue::Null,
        DatabaseType::Null => return Err(invalid()),
        DatabaseType::Boolean => TypedValue::Boolean(value.as_bool().ok_or_else(invalid)?),
        DatabaseType::Integer => TypedValue::Integer(string()?.to_owned()),
        DatabaseType::Decimal => TypedValue::Decimal(string()?.to_owned()),
        DatabaseType::Float => TypedValue::float(value.as_f64().ok_or_else(invalid)?)?,
        DatabaseType::Text => TypedValue::Text(string()?.to_owned()),
        DatabaseType::Timestamp => TypedValue::Timestamp(
            time::OffsetDateTime::parse(string()?, &time::format_description::well_known::Rfc3339)
                .map_err(|_| invalid())?,
        ),
        DatabaseType::Date => TypedValue::Date(
            time::Date::parse(
                string()?,
                time::macros::format_description!("[year]-[month]-[day]"),
            )
            .map_err(|_| invalid())?,
        ),
        DatabaseType::Binary => {
            TypedValue::Binary(STANDARD.decode(string()?).map_err(|_| invalid())?)
        }
        DatabaseType::Json => {
            TypedValue::Json(serde_json::to_string(value).map_err(|_| invalid())?)
        }
    };
    result.validate()?;
    Ok(result)
}

fn parameter(value: &Value) -> Result<Parameter, DatabaseError> {
    let node = value.as_object().ok_or_else(invalid)?;
    if let Some(value) = node.get("literal") {
        return Ok(Parameter::Literal(literal(
            value.as_object().ok_or_else(invalid)?,
        )?));
    }
    Ok(Parameter::Expression {
        value_type: value_type(field(node, "type")?)?,
        expression: Expression::parse(field(node, "expression")?).map_err(|_| invalid())?,
    })
}

fn limits(value: Option<&Value>) -> Result<DatabaseLimits, DatabaseError> {
    let defaults = DatabaseLimits::default();
    let Some(value) = value else {
        return Ok(defaults);
    };
    let node = value.as_object().ok_or_else(invalid)?;
    let number = |field: &str, default: u32| -> Result<u32, DatabaseError> {
        node.get(field).map_or(Ok(default), |v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(invalid)
        })
    };
    let limits = DatabaseLimits {
        max_statement_bytes: number("maxStatementBytes", defaults.max_statement_bytes)?,
        max_parameters: number("maxParameters", defaults.max_parameters)?,
        max_parameter_bytes: number("maxParameterBytes", defaults.max_parameter_bytes)?,
        max_rows: number("maxRows", defaults.max_rows)?,
        max_result_bytes: number("maxResultBytes", defaults.max_result_bytes)?,
        timeout_ms: number("timeoutMs", defaults.timeout_ms)?,
        connect_timeout_ms: number("connectTimeoutMs", defaults.connect_timeout_ms)?,
        pool_size: number("poolSize", defaults.pool_size)?,
        idle_timeout_ms: number("idleTimeoutMs", defaults.idle_timeout_ms)?,
    };
    limits.validate()?;
    Ok(limits)
}

fn parse(node: &Map<String, Value>) -> Result<DatabaseOperation, DatabaseError> {
    let connection = node
        .get("connectionRef")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let name = Slug::parse(field(connection, "name")?).map_err(|_| invalid())?;
    let revision = connection
        .get("revision")
        .map(|v| {
            v.as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|n| *n <= i32::MAX as u32)
                .ok_or(DatabaseError::InvalidRevision)
        })
        .transpose()?;
    let connection = ConnectionRef::new(name, revision)?;
    let kind = match field(node, "operation")? {
        "query" => OperationKind::Query,
        "execute" => OperationKind::Execute,
        _ => return Err(invalid()),
    };
    let parameters = node
        .get("parameters")
        .map(|v| {
            v.as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(parameter)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    DatabaseOperation::new(
        connection,
        kind,
        field(node, "statement")?,
        parameters,
        limits(node.get("limits"))?,
    )
}

pub(crate) fn operation(node: &Map<String, Value>, at: &str) -> Result<DatabaseOperation, IrError> {
    parse(node).map_err(|error| {
        IrError::Schema(vec![IrIssue {
            path: at.to_owned(),
            message: error.to_string(),
        }])
    })
}
