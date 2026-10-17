//! Strict retained-output wire boundary. Values and parser diagnostics never appear in errors.
use sagat_application::database::{DatabaseResultValidator, InvalidDatabaseResult};
use sagat_domain::database::{DatabaseLimits, DatabaseType};
use serde::Deserialize;
use serde_json::Value;

pub struct JsonDatabaseResultValidator;
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ResultWire {
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>,
    affected_rows: String,
    truncated: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Column {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cell {
    #[serde(rename = "type")]
    kind: String,
    value: Value,
}
impl DatabaseResultValidator for JsonDatabaseResultValidator {
    fn validate(
        &self,
        content: &[u8],
        limits: DatabaseLimits,
    ) -> Result<(), InvalidDatabaseResult> {
        limits.validate().map_err(|_| InvalidDatabaseResult)?;
        if content.len() > limits.max_result_bytes as usize {
            return Err(InvalidDatabaseResult);
        }
        let result: ResultWire =
            serde_json::from_slice(content).map_err(|_| InvalidDatabaseResult)?;
        if result.truncated
            || result.rows.len() > limits.max_rows as usize
            || result
                .affected_rows
                .parse::<u64>()
                .ok()
                .is_none_or(|v| v.to_string() != result.affected_rows)
        {
            return Err(InvalidDatabaseResult);
        }
        let column_types = result
            .columns
            .iter()
            .map(|c| {
                // PostgreSQL identifiers/aliases are at most 63 bytes, excluding NUL.
                if c.name.len() > 63 || c.name.contains('\0') {
                    return Err(InvalidDatabaseResult);
                }
                crate::database::value_type(&c.kind).map_err(|_| InvalidDatabaseResult)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for row in &result.rows {
            if row.len() != column_types.len() {
                return Err(InvalidDatabaseResult);
            }
            for (cell, column) in row.iter().zip(&column_types) {
                let kind =
                    crate::database::value_type(&cell.kind).map_err(|_| InvalidDatabaseResult)?;
                if kind != DatabaseType::Null && kind != *column {
                    return Err(InvalidDatabaseResult);
                }
                crate::database::typed_value(kind, &cell.value)
                    .map_err(|_| InvalidDatabaseResult)?;
            }
        }
        Ok(())
    }
}
