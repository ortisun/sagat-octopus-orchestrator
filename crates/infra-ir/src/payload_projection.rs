//! Bounded JSON structure projection for ADR-0028.
//!
//! The output type cannot represent an object key or scalar value. This adapter therefore drops
//! sensitive material while traversing rather than attempting to redact a copy of the document.

use sagat_application::payloads::{
    MAX_INSPECTION_CHILDREN, MAX_INSPECTION_DEPTH, MAX_INSPECTION_INPUT_BYTES,
    MAX_INSPECTION_NODES, PayloadProjection, PayloadProjector, PayloadStructure,
    UnsupportedPayloadFormat,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default)]
pub struct JsonPayloadProjector;

impl PayloadProjector for JsonPayloadProjector {
    fn project(&self, bytes: &[u8]) -> Result<PayloadProjection, UnsupportedPayloadFormat> {
        if bytes.len() > MAX_INSPECTION_INPUT_BYTES {
            return Err(UnsupportedPayloadFormat);
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| UnsupportedPayloadFormat)?;
        let mut budget = Budget {
            remaining: MAX_INSPECTION_NODES,
            truncated: false,
        };
        let tree = project_node(&value, 0, &mut budget).ok_or(UnsupportedPayloadFormat)?;
        Ok(PayloadProjection {
            tree,
            truncated: budget.truncated,
        })
    }
}

struct Budget {
    remaining: usize,
    truncated: bool,
}

fn project_node(value: &Value, depth: usize, budget: &mut Budget) -> Option<PayloadStructure> {
    if budget.remaining == 0 {
        budget.truncated = true;
        return None;
    }
    budget.remaining -= 1;
    Some(match value {
        Value::Object(entries) => {
            let (children, omitted_children) =
                project_children(entries.values(), entries.len(), depth, budget);
            PayloadStructure::Object {
                child_count: bounded_count(entries.len()),
                children,
                omitted_children,
            }
        }
        Value::Array(entries) => {
            let (children, omitted_children) =
                project_children(entries.iter(), entries.len(), depth, budget);
            PayloadStructure::Array {
                child_count: bounded_count(entries.len()),
                children,
                omitted_children,
            }
        }
        Value::String(_) => PayloadStructure::String,
        Value::Number(_) => PayloadStructure::Number,
        Value::Bool(_) => PayloadStructure::Boolean,
        Value::Null => PayloadStructure::Null,
    })
}

fn project_children<'a>(
    values: impl Iterator<Item = &'a Value>,
    count: usize,
    depth: usize,
    budget: &mut Budget,
) -> (Vec<PayloadStructure>, u32) {
    if depth >= MAX_INSPECTION_DEPTH {
        if count > 0 {
            budget.truncated = true;
        }
        return (Vec::new(), bounded_count(count));
    }

    let mut children = Vec::with_capacity(count.min(MAX_INSPECTION_CHILDREN));
    for value in values.take(MAX_INSPECTION_CHILDREN) {
        let Some(child) = project_node(value, depth + 1, budget) else {
            break;
        };
        children.push(child);
    }
    let omitted = count.saturating_sub(children.len());
    if omitted > 0 {
        budget.truncated = true;
    }
    (children, bounded_count(omitted))
}

fn bounded_count(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}
