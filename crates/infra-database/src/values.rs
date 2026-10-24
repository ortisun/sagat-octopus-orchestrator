use crate::ExecutionError;
use bytes::BytesMut;
use sagat_domain::database::{Parameter, TypedValue};
use serde_json::{Value, json};
use std::{error::Error, fmt::Write as _};
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};
use tokio_postgres::{
    Row,
    types::{Format, FromSql, IsNull, ToSql, Type, to_sql_checked},
};

// Text-format bind values preserve arbitrary precision. PostgreSQL still parses each value as its
// explicit type; parameters never become SQL syntax. Debug on the wrapped domain value is redacted.
#[derive(Debug)]
pub(crate) struct Bind<'a>(pub(crate) &'a TypedValue);
impl Bind<'_> {
    pub(crate) fn pg_type(&self) -> Type {
        match self.0 {
            TypedValue::Null => Type::UNKNOWN,
            TypedValue::Boolean(_) => Type::BOOL,
            TypedValue::Integer(s) if s.parse::<i64>().is_ok() => Type::INT8,
            TypedValue::Integer(_) | TypedValue::Decimal(_) => Type::NUMERIC,
            TypedValue::Float(_) => Type::FLOAT8,
            TypedValue::Text(_) => Type::TEXT,
            TypedValue::Timestamp(_) => Type::TIMESTAMPTZ,
            TypedValue::Date(_) => Type::DATE,
            TypedValue::Binary(_) => Type::BYTEA,
            TypedValue::Json(_) => Type::JSONB,
        }
    }
}
pub(crate) fn bindings(parameters: &[Parameter]) -> Result<Vec<Bind<'_>>, ExecutionError> {
    parameters
        .iter()
        .map(|p| match p {
            Parameter::Literal(value) => {
                value.validate().map_err(|_| ExecutionError::ValueType)?;
                if let TypedValue::Json(s) = value {
                    serde_json::from_str::<Value>(s).map_err(|_| ExecutionError::ValueType)?;
                }
                Ok(Bind(value))
            }
            _ => Err(ExecutionError::ValueType),
        })
        .collect()
}
impl ToSql for Bind<'_> {
    fn to_sql(&self, _: &Type, out: &mut BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        match self.0 {
            TypedValue::Null => return Ok(IsNull::Yes),
            TypedValue::Boolean(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            TypedValue::Integer(s)
            | TypedValue::Decimal(s)
            | TypedValue::Text(s)
            | TypedValue::Json(s) => out.extend_from_slice(s.as_bytes()),
            TypedValue::Float(f) => out.extend_from_slice(f.get().to_string().as_bytes()),
            TypedValue::Timestamp(t) => out.extend_from_slice(t.format(&Rfc3339)?.as_bytes()),
            TypedValue::Date(d) => out.extend_from_slice(d.to_string().as_bytes()),
            TypedValue::Binary(b) => {
                out.extend_from_slice(b"\\x");
                const HEX: &[u8] = b"0123456789abcdef";
                for byte in b {
                    out.extend_from_slice(&[
                        HEX[usize::from(byte >> 4)],
                        HEX[usize::from(byte & 15)],
                    ]);
                }
            }
        }
        Ok(IsNull::No)
    }
    fn accepts(_: &Type) -> bool {
        true
    }
    fn encode_format(&self, _: &Type) -> Format {
        Format::Text
    }
    to_sql_checked!();
}

pub(crate) fn type_name(ty: &Type) -> Result<&'static str, ExecutionError> {
    Ok(match *ty {
        Type::BOOL => "boolean",
        Type::INT2 | Type::INT4 | Type::INT8 => "integer",
        Type::NUMERIC => "decimal",
        Type::FLOAT4 | Type::FLOAT8 => "float",
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => "text",
        Type::TIMESTAMPTZ => "timestamp",
        Type::DATE => "date",
        Type::BYTEA => "binary",
        Type::JSON | Type::JSONB => "json",
        _ => return Err(ExecutionError::ValueType),
    })
}
pub(crate) fn cell(row: &Row, i: usize) -> Result<Value, ExecutionError> {
    use base64::Engine as _;
    macro_rules! get { ($ty:ty, $convert:expr) => {{
        let value: Option<$ty> = row.try_get(i).map_err(|_| ExecutionError::ValueType)?;
        match value { Some(value) => ($convert)(value)?, None => return Ok(json!({"type":"null","value":null})) }
    }} }
    let ty = row.columns()[i].type_();
    let name = type_name(ty)?;
    let value = match *ty {
        Type::BOOL => get!(bool, |v| Ok::<_, ExecutionError>(Value::Bool(v))),
        Type::INT2 => get!(i16, |v: i16| Ok::<_, ExecutionError>(json!(v.to_string()))),
        Type::INT4 => get!(i32, |v: i32| Ok::<_, ExecutionError>(json!(v.to_string()))),
        Type::INT8 => get!(i64, |v: i64| Ok::<_, ExecutionError>(json!(v.to_string()))),
        Type::NUMERIC => get!(Decimal, |v: Decimal| Ok::<_, ExecutionError>(
            Value::String(v.0)
        )),
        Type::FLOAT4 => get!(f32, |v: f32| float(f64::from(v))),
        Type::FLOAT8 => get!(f64, float),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
            get!(String, |v| Ok::<_, ExecutionError>(Value::String(v)))
        }
        Type::TIMESTAMPTZ => get!(OffsetDateTime, |v: OffsetDateTime| v
            .format(&Rfc3339)
            .map(Value::String)
            .map_err(|_| ExecutionError::ValueType)),
        Type::DATE => get!(Date, |v: Date| Ok::<_, ExecutionError>(json!(
            v.to_string()
        ))),
        Type::BYTEA => get!(Vec<u8>, |v: Vec<u8>| Ok::<_, ExecutionError>(json!(
            base64::engine::general_purpose::STANDARD.encode(v)
        ))),
        Type::JSON | Type::JSONB => get!(Value, Ok::<_, ExecutionError>),
        _ => return Err(ExecutionError::ValueType),
    };
    Ok(json!({"type":name,"value":value}))
}
fn float(v: f64) -> Result<Value, ExecutionError> {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .ok_or(ExecutionError::ValueType)
}
struct Decimal(String);
impl<'a> FromSql<'a> for Decimal {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        numeric(raw).map(Self).map_err(Into::into)
    }
    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}
// PostgreSQL NUMERIC is base-10000 digits, signed weight and a decimal display scale.
fn numeric(raw: &[u8]) -> Result<String, ExecutionError> {
    if raw.len() < 8 {
        return Err(ExecutionError::ValueType);
    }
    let word = |i| u16::from_be_bytes([raw[i], raw[i + 1]]);
    let count = usize::from(word(0));
    let weight = i32::from(word(2) as i16);
    let sign = word(4);
    let scale = usize::from(word(6));
    let whole_groups = (weight + 1).max(1) as usize;
    if raw.len() != 8 + count * 2
        || !matches!(sign, 0 | 0x4000)
        || scale > 16383
        || whole_groups * 4 + scale + 2 > 16384
    {
        return Err(ExecutionError::ValueType);
    }
    let digits: Vec<_> = (0..count).map(|i| word(8 + i * 2)).collect();
    if digits.iter().any(|d| *d > 9999) {
        return Err(ExecutionError::ValueType);
    }
    let digit = |power: i32| -> u16 {
        usize::try_from(weight - power)
            .ok()
            .and_then(|i| digits.get(i))
            .copied()
            .unwrap_or(0)
    };
    let mut s = String::with_capacity(whole_groups * 4 + scale + 2);
    if sign == 0x4000 && digits.iter().any(|d| *d != 0) {
        s.push('-');
    }
    if weight < 0 {
        s.push('0');
    } else {
        for power in (0..=weight).rev() {
            if power == weight {
                write!(s, "{}", digit(power)).map_err(|_| ExecutionError::ValueType)?;
            } else {
                write!(s, "{:04}", digit(power)).map_err(|_| ExecutionError::ValueType)?;
            }
        }
    }
    if scale > 0 {
        s.push('.');
        let begin = s.len();
        for i in 1..=scale.div_ceil(4) {
            write!(s, "{:04}", digit(-(i as i32))).map_err(|_| ExecutionError::ValueType)?;
        }
        s.truncate(begin + scale);
    }
    Ok(s)
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn numeric_preserves_precision_scale_sign_and_rejects_non_finite_values() {
        let wire = |n: i16, w: i16, s: i16, d: i16, groups: &[i16]| -> Vec<u8> {
            [n, w, s, d]
                .into_iter()
                .chain(groups.iter().copied())
                .flat_map(i16::to_be_bytes)
                .collect()
        };
        assert_eq!(
            numeric(&wire(3, 1, 0, 4, &[1234, 5678, 9000])).unwrap(),
            "12345678.9000"
        );
        assert_eq!(
            numeric(&wire(1, -2, 0x4000, 9, &[1234])).unwrap(),
            "-0.000012340"
        );
        assert_eq!(numeric(&wire(0, 0, 0, 2, &[])).unwrap(), "0.00");
        assert!(numeric(&wire(0, 0, -16384, 0, &[])).is_err());
        assert!(numeric(&wire(1, 0, 0, 0, &[10000])).is_err());
    }
}
