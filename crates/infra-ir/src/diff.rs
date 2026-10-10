//! The comparison of two workflow documents (FR-007-004, D140).
//!
//! The documents are compared as written, not as validated: an author who is half way through an
//! edit can see what changed against the version they started from. Nodes are matched by `id`;
//! everything outside the nodes is compared path by path. Two lists are derived from the
//! differences: what **breaks the contract** with callers and emitters (a trigger gone, an input
//! property gone, retyped or newly required, an output property gone or retyped, an awaited event
//! type nobody awaits any more), and what **somebody must prepare** (a new activity name, a new
//! secret, a script artifact the previous document did not run). Everything is sorted, so the
//! same two documents always give the same answer.

use std::collections::{BTreeMap, BTreeSet};

use sagat_application::definitions::{IrError, MAX_IR_BYTES};
use sagat_application::diff::{
    Change, DiffEdge, DiffFinding, DocumentChange, DocumentDiff, NodeChange,
};
use serde_json::{Map, Value};

/// Longest name repeated in a message.
const MAX_NAME: usize = 64;

pub(crate) fn diff(from: &[u8], to: &[u8]) -> Result<DocumentDiff, IrError> {
    let from = object(from)?;
    let to = object(to)?;

    let from_nodes = nodes_of(&from);
    let to_nodes = nodes_of(&to);

    let nodes_added: Vec<String> = to_nodes
        .keys()
        .filter(|id| !from_nodes.contains_key(*id))
        .map(|id| (*id).to_owned())
        .collect();
    let nodes_removed: Vec<String> = from_nodes
        .keys()
        .filter(|id| !to_nodes.contains_key(*id))
        .map(|id| (*id).to_owned())
        .collect();
    let nodes_changed: Vec<NodeChange> = from_nodes
        .iter()
        .filter_map(|(id, before)| {
            let after = to_nodes.get(id)?;
            let fields = differing_fields(before, after);
            (!fields.is_empty()).then(|| NodeChange {
                id: (*id).to_owned(),
                fields,
            })
        })
        .collect();

    let edges_from = edges(&from_nodes);
    let edges_to = edges(&to_nodes);
    let edges_added: Vec<DiffEdge> = edges_to.difference(&edges_from).cloned().collect();
    let edges_removed: Vec<DiffEdge> = edges_from.difference(&edges_to).cloned().collect();

    let mut changes = Vec::new();
    let keys: BTreeSet<&String> = from.keys().chain(to.keys()).collect();
    for key in keys.into_iter().filter(|k| k.as_str() != "nodes") {
        walk(
            &format!("/{}", escape(key)),
            from.get(key),
            to.get(key),
            &mut changes,
        );
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    let mut breaking = Vec::new();
    triggers(&from, &to, &mut breaking);
    schema(&from, &to, "inputSchema", Side::Input, &mut breaking);
    schema(&from, &to, "outputSchema", Side::Output, &mut breaking);
    removed_event_types(&from_nodes, &to_nodes, &mut breaking);

    let mut attention = Vec::new();
    needs_preparing(&from_nodes, &to_nodes, &mut attention);

    sort_findings(&mut breaking);
    sort_findings(&mut attention);

    let identical = nodes_added.is_empty()
        && nodes_removed.is_empty()
        && nodes_changed.is_empty()
        && edges_added.is_empty()
        && edges_removed.is_empty()
        && changes.is_empty();
    Ok(DocumentDiff {
        identical,
        nodes_added,
        nodes_removed,
        nodes_changed,
        edges_added,
        edges_removed,
        changes,
        breaking,
        attention,
    })
}

fn object(bytes: &[u8]) -> Result<Map<String, Value>, IrError> {
    if bytes.len() > MAX_IR_BYTES {
        return Err(IrError::TooLarge);
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(IrError::Malformed),
    }
}

/// The nodes that have a string `id`, by id (the first of a repeated id wins: a document with a
/// repeated id does not validate anyway).
fn nodes_of(document: &Map<String, Value>) -> BTreeMap<&str, &Map<String, Value>> {
    let mut nodes = BTreeMap::new();
    for node in document
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(map) = node.as_object()
            && let Some(id) = map.get("id").and_then(Value::as_str)
        {
            nodes.entry(id).or_insert(map);
        }
    }
    nodes
}

fn differing_fields(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<String> {
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    keys.into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

/// Every link a node makes to another, with the field it comes from.
fn edges(nodes: &BTreeMap<&str, &Map<String, Value>>) -> BTreeSet<DiffEdge> {
    let mut out = BTreeSet::new();
    for (id, node) in nodes {
        let mut link = |kind: &'static str, to: &Value| {
            if let Some(to) = to.as_str() {
                out.insert(DiffEdge {
                    from: (*id).to_owned(),
                    to: to.to_owned(),
                    kind,
                });
            }
        };
        for (field, kind) in [
            ("next", "next"),
            ("onError", "error"),
            ("onTimeout", "timeout"),
            ("ifTrue", "true"),
            ("ifFalse", "false"),
            ("begin", "begin"),
        ] {
            if let Some(value) = node.get(field) {
                link(kind, value);
            }
        }
        for branch in node
            .get("branches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            link("branch", branch);
        }
    }
    out
}

/// A JSON pointer segment.
fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Records every leaf that differs between two values at `path`.
fn walk(path: &str, before: Option<&Value>, after: Option<&Value>, out: &mut Vec<DocumentChange>) {
    match (before, after) {
        (None, None) => {}
        (None, Some(_)) => out.push(DocumentChange {
            path: path.to_owned(),
            change: Change::Added,
        }),
        (Some(_), None) => out.push(DocumentChange {
            path: path.to_owned(),
            change: Change::Removed,
        }),
        (Some(a), Some(b)) if a == b => {}
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                walk(
                    &format!("{path}/{}", escape(key)),
                    a.get(key),
                    b.get(key),
                    out,
                );
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) => {
            for index in 0..a.len().max(b.len()) {
                walk(&format!("{path}/{index}"), a.get(index), b.get(index), out);
            }
        }
        (Some(_), Some(_)) => out.push(DocumentChange {
            path: path.to_owned(),
            change: Change::Changed,
        }),
    }
}

/// A name from the document as it may appear in a message: clipped, with control characters
/// replaced, so a hostile document cannot put a log line or a terminal escape in an answer.
fn name(text: &str) -> String {
    let mut clean: String = text
        .chars()
        .take(MAX_NAME)
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect();
    if text.chars().count() > MAX_NAME {
        clean.push('…');
    }
    clean
}

fn finding(
    code: &'static str,
    message: String,
    node: Option<&str>,
    path: Option<String>,
) -> DiffFinding {
    DiffFinding {
        code,
        message,
        node: node.map(str::to_owned),
        path,
    }
}

fn sort_findings(findings: &mut [DiffFinding]) {
    findings.sort_by(|a, b| {
        (&a.node, &a.path, a.code, &a.message).cmp(&(&b.node, &b.path, b.code, &b.message))
    });
}

// ------------------------------------------------------------------------------ the contract

/// What starts a run, as far as another system can tell: the kind and the name it listens on.
fn trigger_identity(trigger: &Value) -> (String, String) {
    let text = |field: &str| {
        trigger
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let kind = text("kind");
    let key = match kind.as_str() {
        "event" => text("eventType"),
        "webhook" => text("endpoint"),
        "message" => text("connector"),
        "cron" => text("cron"),
        _ => String::new(),
    };
    (kind, key)
}

fn describe_trigger((kind, key): &(String, String)) -> String {
    if key.is_empty() {
        name(kind)
    } else {
        format!("{} '{}'", name(kind), name(key))
    }
}

fn triggers(from: &Map<String, Value>, to: &Map<String, Value>, out: &mut Vec<DiffFinding>) {
    let list = |document: &Map<String, Value>| {
        document
            .get("triggers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (before, after) = (list(from), list(to));
    for (index, trigger) in before.iter().enumerate() {
        if after.contains(trigger) {
            continue;
        }
        let identity = trigger_identity(trigger);
        let path = Some(format!("/triggers/{index}"));
        if after.iter().any(|t| trigger_identity(t) == identity) {
            out.push(finding(
                "TRIGGER_CHANGED",
                format!(
                    "the {} trigger changed: events that used to start a run may not any more",
                    describe_trigger(&identity)
                ),
                None,
                path,
            ));
        } else {
            out.push(finding(
                "TRIGGER_REMOVED",
                format!(
                    "the {} trigger is gone: nothing starts a run from it any more",
                    describe_trigger(&identity)
                ),
                None,
                path,
            ));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Input,
    Output,
}

fn schema(
    from: &Map<String, Value>,
    to: &Map<String, Value>,
    field: &str,
    side: Side,
    out: &mut Vec<DiffFinding>,
) {
    let Some(before) = from.get(field).and_then(Value::as_object) else {
        return;
    };
    let empty = Map::new();
    let after = to.get(field).and_then(Value::as_object);
    // An input schema that went away accepts anything; an output schema that went away promises
    // nothing, so every property it promised is gone.
    let after = match (after, side) {
        (Some(after), _) => after,
        (None, Side::Input) => return,
        (None, Side::Output) => &empty,
    };
    compare_properties(&format!("/{field}"), before, after, side, out);
}

fn compare_properties(
    path: &str,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    side: Side,
    out: &mut Vec<DiffFinding>,
) {
    let (removed, retyped, required) = match side {
        Side::Input => (
            "INPUT_PROPERTY_REMOVED",
            "INPUT_TYPE_CHANGED",
            "INPUT_REQUIRED_ADDED",
        ),
        Side::Output => ("OUTPUT_PROPERTY_REMOVED", "OUTPUT_TYPE_CHANGED", ""),
    };
    let empty = Map::new();
    let props_before = before
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let props_after = after
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (property, definition) in props_before {
        let here = format!("{path}/properties/{}", escape(property));
        let Some(new_definition) = props_after.get(property) else {
            out.push(finding(
                removed,
                format!(
                    "the property '{}' is no longer described: callers and consumers that use it can fail",
                    name(property)
                ),
                None,
                Some(here),
            ));
            continue;
        };
        if let (Some(old_type), Some(new_type)) =
            (definition.get("type"), new_definition.get("type"))
            && old_type != new_type
        {
            out.push(finding(
                retyped,
                format!("the type of the property '{}' changed", name(property)),
                None,
                Some(format!("{here}/type")),
            ));
        }
        if let (Some(a), Some(b)) = (definition.as_object(), new_definition.as_object()) {
            compare_properties(&here, a, b, side, out);
        }
    }
    if side == Side::Input {
        let names = |schema: &Map<String, Value>| -> BTreeSet<String> {
            schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        let known = names(before);
        for added in names(after).difference(&known) {
            out.push(finding(
                required,
                format!(
                    "'{}' is now required: callers that do not send it will be refused",
                    name(added)
                ),
                None,
                Some(format!("{path}/required")),
            ));
        }
    }
}

/// The event types that nodes await, by node.
fn awaited_events<'a>(
    nodes: &'a BTreeMap<&str, &'a Map<String, Value>>,
) -> Vec<(&'a str, &'a str)> {
    nodes
        .iter()
        .filter_map(|(id, node)| {
            (node.get("type").and_then(Value::as_str) == Some("waitEvent"))
                .then(|| node.get("eventType").and_then(Value::as_str))
                .flatten()
                .map(|event| (*id, event))
        })
        .collect()
}

fn removed_event_types(
    from: &BTreeMap<&str, &Map<String, Value>>,
    to: &BTreeMap<&str, &Map<String, Value>>,
    out: &mut Vec<DiffFinding>,
) {
    let still: BTreeSet<&str> = awaited_events(to).into_iter().map(|(_, e)| e).collect();
    for (node, event) in awaited_events(from) {
        if !still.contains(event) {
            out.push(finding(
                "EVENT_TYPE_REMOVED",
                format!(
                    "no node awaits '{}' any more: whoever publishes it is no longer heard",
                    name(event)
                ),
                Some(node),
                None,
            ));
        }
    }
}

// ------------------------------------------------------------------------------ preparation

fn strings<'a>(node: &'a Map<String, Value>, field: &str) -> Vec<&'a str> {
    match node.get(field) {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn activity_names<'a>(node: &'a Map<String, Value>) -> Vec<&'a str> {
    ["activity", "compensateWith", "reconcileWith"]
        .iter()
        .flat_map(|field| strings(node, field))
        .collect()
}

fn needs_preparing(
    from: &BTreeMap<&str, &Map<String, Value>>,
    to: &BTreeMap<&str, &Map<String, Value>>,
    out: &mut Vec<DiffFinding>,
) {
    let served: BTreeSet<&str> = from.values().flat_map(|n| activity_names(n)).collect();
    let secrets: BTreeSet<&str> = from.values().flat_map(|n| strings(n, "secrets")).collect();
    let scripts: BTreeSet<&str> = from
        .values()
        .filter(|n| n.get("type").and_then(Value::as_str) == Some("script"))
        .flat_map(|n| strings(n, "artifactDigest"))
        .collect();
    for (id, node) in to {
        let code = if from.contains_key(id) {
            "ACTIVITY_CHANGED"
        } else {
            "ACTIVITY_ADDED"
        };
        let names: BTreeSet<&str> = activity_names(node)
            .into_iter()
            .filter(|n| !served.contains(n))
            .collect();
        for activity in names {
            out.push(finding(
                code,
                format!(
                    "the activity '{}' is new: a worker must serve it and be bound to it before runs reach this node",
                    name(activity)
                ),
                Some(id),
                None,
            ));
        }
        let new_secrets: BTreeSet<&str> = strings(node, "secrets")
            .into_iter()
            .filter(|s| !secrets.contains(s))
            .collect();
        for secret in new_secrets {
            out.push(finding(
                "SECRET_ADDED",
                format!(
                    "the node reads the secret '{}': it must exist and be usable by whoever publishes",
                    name(secret)
                ),
                Some(id),
                None,
            ));
        }
        if node.get("type").and_then(Value::as_str) == Some("script") {
            for digest in strings(node, "artifactDigest") {
                if !scripts.contains(digest) {
                    out.push(finding(
                        "SCRIPT_CHANGED",
                        format!(
                            "the node runs a script artifact ({}…) that the previous document did not",
                            name(&digest.chars().take(12).collect::<String>())
                        ),
                        Some(id),
                        None,
                    ));
                }
            }
        }
    }
}
