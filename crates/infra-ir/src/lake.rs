//! The `icebergWrite` IR wire boundary (feature 035, T035-053; `contracts/workflow-lake.schema.json`).
//!
//! The canonical schema has already refused unknown fields, wrong types and out-of-range numbers when a node gets here;
//! this builds the domain step and applies the rules the domain owns. Failures name the part and never echo a value.
//!
//! Only append is built. An overwrite, an upsert and a record schema are refused as unsupported and never dropped: a step
//! that wrote less than it said, or that lost a stricter classification of its fields, is worse than no step.
use sagat_application::definitions::{IrError, IrIssue};
use sagat_domain::{
    database::{
        ColumnMapping, IcebergWrite, IcebergWriteParts, LakeLimits, LakeProbe, RecordSource,
        TableIdentifier, WriteIntent,
    },
    workflow::{Expression, NodeId},
};
use serde_json::{Map, Value};

fn issue(at: &str, message: impl Into<String>) -> IrError {
    IrError::Schema(vec![IrIssue {
        path: at.to_owned(),
        message: message.into(),
    }])
}

fn unsupported(node: &Map<String, Value>, what: &str, reason: &'static str) -> IrError {
    IrError::Unsupported {
        node: node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(64)
            .collect(),
        kind: format!("icebergWrite.{what}"),
        reason,
    }
}

fn text<'a>(node: &'a Map<String, Value>, name: &str, at: &str) -> Result<&'a str, IrError> {
    node.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| issue(&format!("{at}/{name}"), "expected a string"))
}

fn object<'a>(
    node: &'a Map<String, Value>,
    name: &str,
    at: &str,
) -> Result<&'a Map<String, Value>, IrError> {
    node.get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| issue(&format!("{at}/{name}"), "expected an object"))
}

fn number(limits: &Map<String, Value>, name: &str, at: &str) -> Result<Option<u32>, IrError> {
    limits
        .get(name)
        .map(|value| {
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    issue(
                        &format!("{at}/limits/{name}"),
                        "expected a positive integer",
                    )
                })
        })
        .transpose()
}

fn table(node: &Map<String, Value>, at: &str) -> Result<TableIdentifier, IrError> {
    let table = object(node, "table", at)?;
    let namespace = table
        .get("namespace")
        .and_then(Value::as_array)
        .ok_or_else(|| issue(&format!("{at}/table/namespace"), "expected an array"))?
        .iter()
        .map(|level| level.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| issue(&format!("{at}/table/namespace"), "expected strings"))?;
    TableIdentifier::new(namespace, text(table, "name", &format!("{at}/table"))?)
        .map_err(|error| issue(&format!("{at}/table"), error.to_string()))
}

fn records(node: &Map<String, Value>, at: &str) -> Result<RecordSource, IrError> {
    let records = object(node, "records", at)?;
    let here = format!("{at}/records");
    let expression = |name: &str| {
        Expression::parse(text(records, name, &here)?)
            .map_err(|_| issue(&format!("{here}/{name}"), "invalid record expression"))
    };
    match (
        records.get("inline"),
        records.get("blobs"),
        records.get("databaseResult"),
    ) {
        (Some(_), None, None) => Ok(RecordSource::Inline(expression("inline")?)),
        (None, Some(_), None) => Ok(RecordSource::Blobs(expression("blobs")?)),
        (None, None, Some(_)) => NodeId::parse(text(records, "databaseResult", &here)?)
            .map(RecordSource::DatabaseResult)
            .map_err(|_| issue(&format!("{here}/databaseResult"), "invalid node id")),
        _ => Err(issue(&here, "a step takes exactly one record source")),
    }
}

fn mapping(node: &Map<String, Value>, at: &str) -> Result<Vec<ColumnMapping>, IrError> {
    let Some(entries) = node.get("mapping") else {
        return Ok(Vec::new());
    };
    entries
        .as_array()
        .ok_or_else(|| issue(&format!("{at}/mapping"), "expected an array"))?
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let here = format!("{at}/mapping/{index}");
            let entry = entry
                .as_object()
                .ok_or_else(|| issue(&here, "expected an object"))?;
            ColumnMapping::new(text(entry, "field", &here)?, text(entry, "column", &here)?)
                .map_err(|error| issue(&here, error.to_string()))
        })
        .collect()
}

fn limits(node: &Map<String, Value>, at: &str) -> Result<LakeLimits, IrError> {
    let Some(limits) = node.get("limits") else {
        return Ok(LakeLimits::default());
    };
    let limits = limits
        .as_object()
        .ok_or_else(|| issue(&format!("{at}/limits"), "expected an object"))?;
    Ok(LakeLimits {
        max_record_bytes: number(limits, "maxRecordBytes", at)?,
        max_records: number(limits, "maxRecords", at)?,
        timeout_ms: number(limits, "timeoutMs", at)?,
    })
}

/// The domain step of an `icebergProbe` node (a private document, ADR-0080): the connection to test.
pub(crate) fn probe(node: &Map<String, Value>, at: &str) -> Result<LakeProbe, IrError> {
    let connection = crate::database::connection_ref(object(node, "connectionRef", at)?)
        .map_err(|error| issue(&format!("{at}/connectionRef"), error.to_string()))?;
    Ok(LakeProbe::new(connection))
}

/// The domain step of an `icebergWrite` node.
pub(crate) fn write(node: &Map<String, Value>, at: &str) -> Result<IcebergWrite, IrError> {
    // What is not built yet is refused before anything else is read, so that no part of it is quietly dropped.
    if node.contains_key("overwrite") || text(node, "mode", at)? == "overwrite" {
        return Err(unsupported(
            node,
            "overwrite",
            "overwriting a table is not enabled by the runtime yet",
        ));
    }
    if node.contains_key("upsert") || text(node, "mode", at)? == "upsert" {
        return Err(unsupported(
            node,
            "upsert",
            "upserting into a table is not enabled by the runtime yet",
        ));
    }
    if node.contains_key("recordSchema") {
        return Err(unsupported(
            node,
            "recordSchema",
            "a record schema is not enabled by the runtime yet, and the step is not published without the classification it declares",
        ));
    }
    let connection = crate::database::connection_ref(object(node, "connectionRef", at)?)
        .map_err(|error| issue(&format!("{at}/connectionRef"), error.to_string()))?;
    let reference = match node.get("ref") {
        None => "main".to_owned(),
        Some(_) => text(node, "ref", at)?.to_owned(),
    };
    IcebergWrite::new(IcebergWriteParts {
        connection,
        table: table(node, at)?,
        reference,
        intent: WriteIntent::Append,
        records: records(node, at)?,
        mapping: mapping(node, at)?,
        ignore_unknown_fields: node
            .get("ignoreUnknownFields")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limits: limits(node, at)?,
        revert: node.get("compensation").and_then(Value::as_str) == Some("revert"),
    })
    .map_err(|error| issue(at, error.to_string()))
}
