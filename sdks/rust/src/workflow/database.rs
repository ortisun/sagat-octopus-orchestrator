use super::*;
/// A logical namespace connection, never a DSN or credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseOperation {
    Query,
    Execute,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    Null,
    Boolean,
    Integer,
    Decimal,
    Float,
    Text,
    Timestamp,
    Date,
    Binary,
    Json,
}
/// Integer and decimal bindings use strings; binary uses base64.
#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseValue {
    #[serde(rename = "type")]
    pub kind: DatabaseType,
    pub value: Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum DatabaseParameter {
    Literal {
        literal: DatabaseValue,
    },
    Expression {
        #[serde(rename = "type")]
        kind: DatabaseType,
        expression: String,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseOptions {
    pub connection_ref: ConnectionRef,
    pub operation: DatabaseOperation,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<DatabaseParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<BTreeMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
}
macro_rules! opaque_debug {($($name:ty),*)=>{$(impl std::fmt::Debug for $name{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("[managed database operation]")}})*};}
opaque_debug!(DatabaseOptions, DatabaseParameter, DatabaseValue);
fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit())
}
fn parameter(parameter: &DatabaseParameter) -> bool {
    let DatabaseParameter::Literal { literal } = parameter else {
        return matches!(parameter,DatabaseParameter::Expression{expression,..} if !expression.is_empty());
    };
    let value = &literal.value;
    match literal.kind {
        DatabaseType::Null => value.is_null(),
        DatabaseType::Boolean => value.is_boolean(),
        DatabaseType::Integer => value
            .as_str()
            .is_some_and(|s| s.len() <= 40 && digits(s.strip_prefix('-').unwrap_or(s))),
        DatabaseType::Decimal => value.as_str().is_some_and(|s| {
            if s.len() > 16384 {
                return false;
            }
            let unsigned = s.strip_prefix('-').unwrap_or(s);
            if let Some((whole, fraction)) = unsigned.split_once('.') {
                digits(whole) && digits(fraction)
            } else {
                digits(unsigned)
            }
        }),
        DatabaseType::Float => value.as_f64().is_some_and(f64::is_finite),
        DatabaseType::Text => value.as_str().is_some_and(|s| !s.contains('\0')),
        DatabaseType::Timestamp | DatabaseType::Date | DatabaseType::Binary => value.is_string(),
        DatabaseType::Json => true,
    }
}
fn ceiling(key: &str) -> u64 {
    match key {
        "maxStatementBytes" => 65536,
        "maxParameters" => 256,
        "maxParameterBytes" => 262144,
        "maxRows" => 10000,
        "maxResultBytes" => 4194304,
        "timeoutMs" => 30000,
        "connectTimeoutMs" => 10000,
        "poolSize" => 16,
        "idleTimeoutMs" => 60000,
        _ => 0,
    }
}
impl Builder {
    /// Declare a managed operation. Only the server resolves credentials and executes SQL.
    pub fn database(&self, id: &str, options: DatabaseOptions) -> &Self {
        let name = &options.connection_ref.name;
        let valid_name = (3..=63).contains(&name.len())
            && name.as_bytes()[0].is_ascii_lowercase()
            && name
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            && name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            && !name.contains("--");
        let parameters = options.parameters.as_deref().unwrap_or_default();
        if !valid_name
            || options
                .connection_ref
                .revision
                .is_some_and(|v| v == 0 || v > i32::MAX as u32)
            || options.statement.trim().is_empty()
            || options.statement.len() > 65536
            || options.statement.contains('\0')
            || parameters.len() > 256
            || parameters.iter().any(|p| !parameter(p))
            || options.limits.as_ref().is_some_and(|limits| {
                limits
                    .iter()
                    .any(|(key, value)| *value == 0 || *value > ceiling(key))
            })
            || options.timeout_ms.is_some_and(|v| v == 0 || v > 60000)
            || options.on_error.as_ref().is_some_and(String::is_empty)
        {
            return self.fail("invalid managed database operation");
        }
        let size = serde_json::to_value(parameters)
            .ok()
            .and_then(|v| canonical_json(&v).ok())
            .map(|s| s.len());
        if size.is_none_or(|size| size > 262144) {
            return self.fail("invalid managed database operation");
        }
        self.options(json!({"id":id,"type":"database"}), options)
    }
}
