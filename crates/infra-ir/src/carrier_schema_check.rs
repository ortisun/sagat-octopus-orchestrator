//! Generic JSON Schema compilation for classification carriers.

use sagat_application::definitions::IrIssue;
use sagat_application::scripts::{SchemaChecker, SchemaProblem};
use sagat_infra_schema::Schemas;
use serde_json::Value;

const ANNOTATION: &str = "x-sagat-classification";
const MAX_ISSUES: usize = 20;

/// Compiles only schemas which become classification carriers. Legacy root schemas without any
/// classification annotation retain their established acceptance behavior.
#[must_use]
pub(crate) fn issues(document: &Value) -> Vec<IrIssue> {
    let mut issues = Vec::new();
    for field in ["inputSchema", "outputSchema"] {
        if let Some(schema) = document.get(field)
            && contains_annotation(schema)
        {
            check(schema, &format!("/{field}"), &mut issues);
        }
    }
    if let Some(nodes) = document.get("nodes").and_then(Value::as_array) {
        check_nodes(nodes, "/nodes", &mut issues);
    }
    if let Some(bodies) = document.get("bodies").and_then(Value::as_object) {
        for (body, definition) in bodies {
            if let Some(nodes) = definition.get("nodes").and_then(Value::as_array) {
                check_nodes(
                    nodes,
                    &format!("/bodies/{}/nodes", escape(body)),
                    &mut issues,
                );
            }
        }
    }
    issues
}

fn check_nodes(nodes: &[Value], path: &str, issues: &mut Vec<IrIssue>) {
    for (index, node) in nodes.iter().enumerate() {
        if issues.len() == MAX_ISSUES {
            return;
        }
        if !matches!(
            node.get("type").and_then(Value::as_str),
            Some("activity" | "script" | "waitEvent" | "child")
        ) {
            continue;
        }
        if let Some(schema) = node.get("outputSchema") {
            check(schema, &format!("{path}/{index}/outputSchema"), issues);
        }
    }
}

fn check(schema: &Value, path: &str, issues: &mut Vec<IrIssue>) {
    if issues.len() == MAX_ISSUES {
        return;
    }
    let problem = match serde_json::to_vec(schema) {
        Ok(bytes) => Schemas.check(&bytes).err(),
        Err(_) => Some(SchemaProblem::Invalid),
    };
    let Some(problem) = problem else {
        return;
    };
    let message = match problem {
        SchemaProblem::TooLarge => "classification carrier schema exceeds the fixed size bound",
        SchemaProblem::NotJson | SchemaProblem::NotAnObject => {
            "classification carrier schema must be a JSON object"
        }
        SchemaProblem::Invalid => "classification carrier schema is not a supported JSON Schema",
    };
    issues.push(IrIssue {
        path: path.to_owned(),
        message: message.to_owned(),
    });
}

fn contains_annotation(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(ANNOTATION) || object.values().any(contains_annotation)
        }
        Value::Array(values) => values.iter().any(contains_annotation),
        _ => false,
    }
}

fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
