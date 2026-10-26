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

use std::collections::{BTreeMap, BTreeSet, HashSet};

use sagat_application::classification::{ClassificationCarrierStatus, ClassificationResolution};
use sagat_application::classification_index::ClassificationLocationIndex;
use sagat_application::definitions::{IrError, MAX_IR_BYTES};
use sagat_application::diff::{
    Change, DiffEdge, DiffFinding, DiffRules, DocumentChange, DocumentDiff, NodeChange,
};
use sagat_domain::classification::{ClassificationRules, Level};
use serde_json::{Map, Value};

/// Longest name repeated in a message.
const MAX_NAME: usize = 64;

/// Levels of a path that are shown in full (see [`render`]).
const MAX_SEGMENTS: usize = 10;

/// A schema with nothing in it, to compare against when one side has no `properties`.
static NOTHING: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);

/// Differences outside the nodes that are listed; the rest are counted. A draft is stored after a
/// size check only (a megabyte, which can be a hundred thousand array elements), and the answer is
/// read by a person.
const MAX_CHANGES: usize = 1_000;

/// Findings of one kind that are listed; the rest are counted in a last finding.
const MAX_FINDINGS: usize = 100;

/// One step of the way to a place in a document. A path is made of these and rendered only for what is listed:
/// a document can be nested a hundred levels deep with names of kilobytes, and a path built at every one of
/// a hundred thousand places would cost gigabytes.
#[derive(Clone, Copy)]
enum Seg<'a> {
    Key(&'a str),
    Index(usize),
}

/// A JSON pointer to the place `trail` leads to. A name longer than [`MAX_NAME`] is clipped, and a path
/// through more than [`MAX_SEGMENTS`] levels shows its first four and its last ones with `…` between, so
/// a path is a way to find a place and not a copy of the document.
fn render(trail: &[Seg<'_>]) -> String {
    const HEAD: usize = 4;
    fn put(out: &mut String, segments: &[Seg<'_>]) {
        for seg in segments {
            out.push('/');
            match seg {
                Seg::Key(key) => out.push_str(&escape(&name(key))),
                Seg::Index(index) => out.push_str(&index.to_string()),
            }
        }
    }
    let mut out = String::new();
    if trail.len() <= MAX_SEGMENTS {
        put(&mut out, trail);
    } else {
        put(&mut out, &trail[..HEAD]);
        out.push_str("/…");
        put(&mut out, &trail[trail.len() - (MAX_SEGMENTS - HEAD)..]);
    }
    out
}

/// The differences outside the nodes: the first [`MAX_CHANGES`] and how many came after them.
#[derive(Default)]
struct Changes {
    listed: Vec<DocumentChange>,
    omitted: usize,
}

impl Changes {
    fn push(&mut self, trail: &[Seg<'_>], change: Change) {
        if self.listed.len() < MAX_CHANGES {
            self.listed.push(DocumentChange {
                path: render(trail),
                change,
            });
        } else {
            self.omitted += 1;
        }
    }
}

/// The findings of one kind: the first [`MAX_FINDINGS`] are built and listed, the rest are only counted
/// (a finding that is not listed is never built).
#[derive(Default)]
struct Found {
    listed: Vec<DiffFinding>,
    omitted: usize,
}

impl Found {
    fn push(&mut self, make: impl FnOnce() -> DiffFinding) {
        if self.listed.len() < MAX_FINDINGS {
            self.listed.push(make());
        } else {
            self.omitted += 1;
        }
    }

    /// Sorted, with a last finding that says how many were left out.
    fn into_list(mut self) -> Vec<DiffFinding> {
        sort_findings(&mut self.listed);
        if self.omitted > 0 {
            self.listed.push(finding(
                "MORE_FINDINGS",
                format!("{} more findings of this kind are not listed", self.omitted),
                None,
                None,
            ));
        }
        self.listed
    }
}

pub(crate) fn diff(from: &[u8], to: &[u8], rules: DiffRules) -> Result<DocumentDiff, IrError> {
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

    let mut changes = Changes::default();
    let keys: BTreeSet<&String> = from.keys().chain(to.keys()).collect();
    let mut trail = Vec::new();
    for key in keys.into_iter().filter(|k| k.as_str() != "nodes") {
        trail.push(Seg::Key(key.as_str()));
        walk(&mut trail, from.get(key), to.get(key), &mut changes);
        trail.pop();
    }
    let Changes {
        listed: mut changes,
        omitted: changes_omitted,
    } = changes;
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    // A workflow that was never published had no callers and no emitters: nothing to break.
    let mut breaking = Found::default();
    if !from.is_empty() {
        triggers(&from, &to, &mut breaking);
        schema(&from, &to, "inputSchema", Side::Input, &mut breaking);
        schema(&from, &to, "outputSchema", Side::Output, &mut breaking);
        removed_event_types(&from_nodes, &to_nodes, &mut breaking);
    }

    let mut attention = Found::default();
    needs_preparing(&from_nodes, &to_nodes, &mut attention);
    // As for the breaking changes: a workflow that was never published had nobody to inform of a
    // level that changed, so its first draft is not compared level by level.
    let _ = classification_changes(
        &Value::Object(from.clone()),
        &Value::Object(to.clone()),
        rules,
        !from.is_empty(),
        &mut attention,
    );
    let (breaking, attention) = (breaking.into_list(), attention.into_list());

    let identical = nodes_added.is_empty()
        && nodes_removed.is_empty()
        && nodes_changed.is_empty()
        && edges_added.is_empty()
        && edges_removed.is_empty()
        && changes.is_empty()
        && changes_omitted == 0;
    Ok(DocumentDiff {
        identical,
        nodes_added,
        nodes_removed,
        nodes_changed,
        edges_added,
        edges_removed,
        changes,
        changes_omitted,
        breaking,
        attention,
    })
}

/// What one side of a comparison discloses.
enum Disclosure {
    /// Published under the current rules: the effective level of every location of its carriers.
    Levels(ClassificationLocationIndex),
    /// Published under rules `0`: it reads no annotation and discloses no value, whatever its
    /// schemas say (FR-016-021).
    Nothing,
}

impl Disclosure {
    fn of(document: &Value, rules: ClassificationRules) -> Self {
        if rules == ClassificationRules::NONE {
            return Self::Nothing;
        }
        Self::Levels(ClassificationLocationIndex::build(|visitor| {
            crate::classification_check::visit_locations(document, visitor)
        }))
    }

    fn steps(&self) -> u64 {
        match self {
            Self::Levels(index) => index.status().steps,
            Self::Nothing => 0,
        }
    }
}

/// The level of a location as a comparison sees it: `None` is a value nobody can read, because
/// the side has no carrier for it or reads none (a payload with no schema is structure only for
/// everyone, which is stricter than `secret` for a person who may read nothing else).
type Seen = Option<Level>;

/// Compares the effective level of every location of both documents and returns the walk steps
/// spent. Each document is walked once (the locations are looked up in the other one's index); the
/// cost is the size of the carriers, not the product of locations and carrier size.
fn classification_changes(
    from: &Value,
    to: &Value,
    rules: DiffRules,
    compare: bool,
    out: &mut Found,
) -> u64 {
    let from_side = Disclosure::of(from, rules.from);
    let to_side = Disclosure::of(to, rules.to);

    let mut reported_failures = HashSet::new();
    for side in [&from_side, &to_side] {
        let Disclosure::Levels(index) = side else {
            continue;
        };
        for status in &index.status().carriers {
            let ClassificationCarrierStatus::Failed(failure) = status else {
                continue;
            };
            if reported_failures.insert((failure.carrier.clone(), failure.code)) {
                let code = failure.code;
                let path = failure.source_path.clone();
                out.push(|| finding(code.as_str(), code.message().to_owned(), None, Some(path)));
            }
        }
    }
    if !compare {
        return from_side.steps() + to_side.steps();
    }

    if let Disclosure::Levels(index) = &from_side {
        for old in index.locations() {
            let (after, path) = match &to_side {
                Disclosure::Nothing => (None, old.source_path.clone()),
                Disclosure::Levels(other) => {
                    match other.resolve(&old.carrier, &old.location) {
                        ClassificationResolution::Resolved {
                            level,
                            source_path,
                            location_present,
                        } => (
                            Some(level),
                            if location_present {
                                source_path
                            } else {
                                old.source_path.clone()
                            },
                        ),
                        ClassificationResolution::AbsentCarrier => (None, old.source_path.clone()),
                        // Not compared: a prefix of a carrier that failed is not the carrier.
                        ClassificationResolution::Incomplete(_) => continue,
                    }
                }
            };
            classification_change(Some(old.level), after, &path, out);
        }
    }
    if let Disclosure::Levels(index) = &to_side {
        for new in index.locations() {
            let before = match &from_side {
                Disclosure::Nothing => None,
                Disclosure::Levels(other) => match other.resolve(&new.carrier, &new.location) {
                    // Present on both sides: compared from the other side's walk.
                    ClassificationResolution::Resolved {
                        location_present: true,
                        ..
                    }
                    | ClassificationResolution::Incomplete(_) => continue,
                    ClassificationResolution::Resolved { level, .. } => Some(level),
                    ClassificationResolution::AbsentCarrier => None,
                },
            };
            classification_change(before, Some(new.level), &new.source_path, out);
        }
    }
    from_side.steps() + to_side.steps()
}

fn classification_change(before: Seen, after: Seen, path: &str, out: &mut Found) {
    // Nobody can read an undisclosed value, which is as strict as `secret` can be.
    let (rank_before, rank_after) = (
        before.unwrap_or(Level::Secret),
        after.unwrap_or(Level::Secret),
    );
    let code = if rank_before > rank_after {
        "CLASSIFICATION_RELAXED"
    } else if rank_before < rank_after {
        "CLASSIFICATION_TIGHTENED"
    } else {
        return;
    };
    out.push(|| {
        finding(
            code,
            format!(
                "classification changed from {} to {}",
                label(before),
                label(after)
            ),
            None,
            Some(path.to_owned()),
        )
    });
}

fn label(seen: Seen) -> String {
    seen.map_or_else(|| "undisclosed".to_owned(), |level| level.to_string())
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

/// Records every leaf that differs between two values at the place `trail` leads to.
fn walk<'a>(
    trail: &mut Vec<Seg<'a>>,
    before: Option<&'a Value>,
    after: Option<&'a Value>,
    out: &mut Changes,
) {
    match (before, after) {
        (None, None) => {}
        (None, Some(_)) => out.push(trail, Change::Added),
        (Some(_), None) => out.push(trail, Change::Removed),
        (Some(a), Some(b)) if a == b => {}
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                trail.push(Seg::Key(key.as_str()));
                walk(trail, a.get(key), b.get(key), out);
                trail.pop();
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) => {
            for index in 0..a.len().max(b.len()) {
                trail.push(Seg::Index(index));
                walk(trail, a.get(index), b.get(index), out);
                trail.pop();
            }
        }
        (Some(_), Some(_)) => out.push(trail, Change::Changed),
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

fn triggers(from: &Map<String, Value>, to: &Map<String, Value>, out: &mut Found) {
    let list = |document: &Map<String, Value>| -> Vec<Value> {
        document
            .get("triggers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (before, after) = (list(from), list(to));
    // Sets, not scans: a draft can hold a hundred thousand of them, and a comparison that is
    // quadratic in that holds a thread of the platform for minutes. Values are compared by their
    // canonical text (the keys of an object are sorted).
    let kept: HashSet<String> = after.iter().map(Value::to_string).collect();
    let identities: HashSet<(String, String)> = after.iter().map(trigger_identity).collect();
    for (index, trigger) in before.iter().enumerate() {
        if kept.contains(&trigger.to_string()) {
            continue;
        }
        let identity = trigger_identity(trigger);
        let path = Some(format!("/triggers/{index}"));
        if identities.contains(&identity) {
            out.push(|| {
                finding(
                    "TRIGGER_CHANGED",
                    format!(
                        "the {} trigger changed: events that used to start a run may not any more",
                        describe_trigger(&identity)
                    ),
                    None,
                    path,
                )
            });
        } else {
            out.push(|| {
                finding(
                    "TRIGGER_REMOVED",
                    format!(
                        "the {} trigger is gone: nothing starts a run from it any more",
                        describe_trigger(&identity)
                    ),
                    None,
                    path,
                )
            });
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
    out: &mut Found,
) {
    let empty = Map::new();
    // No input schema accepts anything, so a `required` entry added to one that had none breaks the
    // callers that did not send it; no output schema promised nothing, so there is nothing to break.
    let before = match (from.get(field).and_then(Value::as_object), side) {
        (Some(before), _) => before,
        (None, Side::Input) => &empty,
        (None, Side::Output) => return,
    };
    let after = to.get(field).and_then(Value::as_object);
    // An input schema that went away accepts anything; an output schema that went away promises
    // nothing, so every property it promised is gone.
    let after = match (after, side) {
        (Some(after), _) => after,
        (None, Side::Input) => return,
        (None, Side::Output) => &empty,
    };
    compare_properties(&mut vec![Seg::Key(field)], before, after, side, out);
}

fn compare_properties<'a>(
    trail: &mut Vec<Seg<'a>>,
    before: &'a Map<String, Value>,
    after: &'a Map<String, Value>,
    side: Side,
    out: &mut Found,
) {
    let (removed, retyped, required) = match side {
        Side::Input => (
            "INPUT_PROPERTY_REMOVED",
            "INPUT_TYPE_CHANGED",
            "INPUT_REQUIRED_ADDED",
        ),
        Side::Output => ("OUTPUT_PROPERTY_REMOVED", "OUTPUT_TYPE_CHANGED", ""),
    };
    let props_before = before
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&NOTHING);
    let props_after = after
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&NOTHING);
    trail.push(Seg::Key("properties"));
    for (property, definition) in props_before {
        trail.push(Seg::Key(property.as_str()));
        let Some(new_definition) = props_after.get(property) else {
            out.push(|| {
                finding(
                    removed,
                    format!(
                        "the property '{}' is no longer described: callers and consumers that use it can fail",
                        name(property)
                    ),
                    None,
                    Some(render(trail)),
                )
            });
            trail.pop();
            continue;
        };
        if let (Some(old_type), Some(new_type)) =
            (definition.get("type"), new_definition.get("type"))
            && old_type != new_type
        {
            out.push(|| {
                trail.push(Seg::Key("type"));
                let path = render(trail);
                trail.pop();
                finding(
                    retyped,
                    format!("the type of the property '{}' changed", name(property)),
                    None,
                    Some(path),
                )
            });
        }
        if let (Some(a), Some(b)) = (definition.as_object(), new_definition.as_object()) {
            compare_properties(trail, a, b, side, out);
        }
        trail.pop();
    }
    trail.pop();
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
            out.push(|| {
                trail.push(Seg::Key("required"));
                let path = render(trail);
                trail.pop();
                finding(
                    required,
                    format!(
                        "'{}' is now required: callers that do not send it will be refused",
                        name(added)
                    ),
                    None,
                    Some(path),
                )
            });
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
    out: &mut Found,
) {
    let still: BTreeSet<&str> = awaited_events(to).into_iter().map(|(_, e)| e).collect();
    for (node, event) in awaited_events(from) {
        if !still.contains(event) {
            out.push(|| {
                finding(
                    "EVENT_TYPE_REMOVED",
                    format!(
                        "no node awaits '{}' any more: whoever publishes it is no longer heard",
                        name(event)
                    ),
                    Some(node),
                    None,
                )
            });
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

fn activity_names(node: &Map<String, Value>) -> Vec<&str> {
    ["activity", "compensateWith", "reconcileWith"]
        .iter()
        .flat_map(|field| strings(node, field))
        .collect()
}

fn needs_preparing(
    from: &BTreeMap<&str, &Map<String, Value>>,
    to: &BTreeMap<&str, &Map<String, Value>>,
    out: &mut Found,
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
            out.push(|| {
                finding(
                    code,
                    format!(
                        "the activity '{}' is new: a worker must serve it and be bound to it before runs reach this node",
                        name(activity)
                    ),
                    Some(id),
                    None,
                )
            });
        }
        let new_secrets: BTreeSet<&str> = strings(node, "secrets")
            .into_iter()
            .filter(|s| !secrets.contains(s))
            .collect();
        for secret in new_secrets {
            out.push(|| {
                finding(
                    "SECRET_ADDED",
                    format!(
                        "the node reads the secret '{}': it must exist and be usable by whoever publishes",
                        name(secret)
                    ),
                    Some(id),
                    None,
                )
            });
        }
        if node.get("type").and_then(Value::as_str) == Some("script") {
            for digest in strings(node, "artifactDigest") {
                if !scripts.contains(digest) {
                    out.push(|| {
                        finding(
                            "SCRIPT_CHANGED",
                            format!(
                                "the node runs a script artifact ({}…) that the previous document did not",
                                name(&digest.chars().take(12).collect::<String>())
                            ),
                            Some(id),
                            None,
                        )
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)] // test fixtures fail immediately

    use serde_json::json;

    use super::*;

    const PROPERTIES: usize = 2_000;

    /// An object of `PROPERTIES` unannotated properties at `level` (about 52 KB, under the
    /// 64 KiB bound of a carrier): every location inherits the level of the root.
    fn carrier(level: &str) -> Value {
        let properties: Map<String, Value> = (0..PROPERTIES)
            .map(|index| (format!("p{index:04}"), json!({"type": "string"})))
            .collect();
        json!({"inputSchema": {
            "type": "object", "x-sagat-classification": level, "properties": properties
        }})
    }

    /// FR-016-025 / NFR-016-003: comparing two versions walks each carrier a fixed number of times
    /// whatever the number of locations. The cost is counted in walk steps, not in time, so the
    /// bound holds on a loaded machine. Before the index existed every location started a walk of
    /// its own over the whole carrier (130 s for this document in a debug build).
    #[test]
    fn comparing_a_two_thousand_property_carrier_costs_a_fixed_number_of_passes() {
        let before = carrier("pii");
        let after = carrier("public");
        let size = serde_json::to_vec(&before).expect("JSON").len();
        assert!(size < 64 * 1024, "the carrier is within its bound ({size})");

        let mut found = Found::default();
        let steps = classification_changes(&before, &after, DiffRules::CURRENT, true, &mut found);
        let locations = (PROPERTIES + 1) as u64;
        // Measured: five steps per location and side (a scan, a dry walk and an emitting walk).
        // A walk per location, as before, costs the number of locations times that.
        let budget = 2 * 8 * locations;
        assert!(
            steps <= budget,
            "{steps} steps for {locations} locations (budget {budget})"
        );
        // Every location is relaxed: the first 100 are listed and the rest counted exactly.
        let listed = found.into_list();
        assert_eq!(listed.len(), MAX_FINDINGS + 1);
        assert!(
            listed[..MAX_FINDINGS]
                .iter()
                .all(|f| f.code == "CLASSIFICATION_RELAXED")
        );
        assert_eq!(listed[MAX_FINDINGS].code, "MORE_FINDINGS");
        assert!(listed[MAX_FINDINGS].message.starts_with("1901 more"));

        // Nothing changed: the same bound, and nothing listed.
        let mut found = Found::default();
        let same = classification_changes(&before, &before, DiffRules::CURRENT, true, &mut found);
        assert!(same <= budget && found.into_list().is_empty());
    }
}
