//! Bounded, value-free publication analysis for classification annotations.

use std::collections::{BTreeMap, HashSet};

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationCarrierFailure,
    ClassificationCarrierStatus, ClassificationFinding, ClassificationFindingCode,
    ClassificationLocation, ClassificationPointer, ClassificationReportEntry,
    ClassificationResolution, ClassificationSegment, ClassificationWalkStatus, FindingSeverity,
    MAX_CARRIER_BYTES, MAX_CLASSIFICATION_FINDINGS, MAX_CLASSIFICATION_REPORT_ENTRIES,
    MAX_REF_EXPANSIONS, MAX_SCHEMA_DEPTH,
};
use sagat_domain::classification::Level;
use sagat_domain::workflow::NodeId;
use serde_json::Value;

const ANNOTATION: &str = "x-sagat-classification";
const UNSUPPORTED_APPLICATORS: [&str; 11] = [
    "patternProperties",
    "if",
    "then",
    "else",
    "dependentSchemas",
    "contains",
    "propertyNames",
    "not",
    "unevaluatedProperties",
    "unevaluatedItems",
    "$dynamicRef",
];

#[must_use]
pub fn analyze(document: &[u8]) -> ClassificationAnalysis {
    serde_json::from_slice(document).map_or_else(
        |_| ClassificationAnalysis::default(),
        |value| analyze_value(&value),
    )
}

#[must_use]
pub fn analyze_value(document: &Value) -> ClassificationAnalysis {
    let mut visitor = None;
    analyze_document(document, true, &mut visitor).0
}

/// Visits every effective location without applying the human-facing 500-row report cap.
/// A failed carrier emits no locations, so callers never compare a valid prefix as a whole carrier.
#[must_use]
pub fn visit_classification_locations(
    document: &Value,
    visitor: &mut dyn FnMut(&ClassificationLocation),
) -> ClassificationWalkStatus {
    let mut visitor = Some(visitor);
    analyze_document(document, false, &mut visitor).1
}

/// Resolves one semantic location using the same walk and fallback rules as publication analysis.
#[must_use]
pub fn resolve_classification_location(
    document: &Value,
    carrier: &CarrierPointer,
    location: &[ClassificationSegment],
) -> ClassificationResolution {
    let (status, root) = location_at(document, carrier, &[]);
    if let Some(failure) = status.failure(carrier) {
        return ClassificationResolution::Incomplete(failure.clone());
    }
    let Some(mut current) = root else {
        return ClassificationResolution::AbsentCarrier;
    };
    let mut selected = Vec::with_capacity(location.len());
    let mut location_present = true;
    for requested in location {
        let mut exact = selected.clone();
        exact.push(requested.clone());
        if let (_, Some(found)) = location_at(document, carrier, &exact) {
            selected = exact;
            current = found;
            continue;
        }

        let fallback = match requested {
            ClassificationSegment::Property(_) => ClassificationSegment::AdditionalProperty,
            ClassificationSegment::Index(_) => ClassificationSegment::RemainingItem,
            ClassificationSegment::AdditionalProperty | ClassificationSegment::RemainingItem => {
                return ClassificationResolution::Resolved {
                    level: current.level.stricter(Level::UNMARKED),
                    source_path: current.source_path,
                    location_present: false,
                };
            }
        };
        let mut fallback_path = selected.clone();
        fallback_path.push(fallback);
        if let (_, Some(found)) = location_at(document, carrier, &fallback_path) {
            selected = fallback_path;
            current = found;
            location_present = false;
        } else {
            return ClassificationResolution::Resolved {
                level: current.level.stricter(Level::UNMARKED),
                source_path: current.source_path,
                location_present: false,
            };
        }
    }
    ClassificationResolution::Resolved {
        level: current.level,
        source_path: current.source_path,
        location_present,
    }
}

fn location_at(
    document: &Value,
    carrier: &CarrierPointer,
    location: &[ClassificationSegment],
) -> (ClassificationWalkStatus, Option<ClassificationLocation>) {
    let mut found = None;
    let mut take = |candidate: &ClassificationLocation| {
        if candidate.location == location {
            found = Some(candidate.clone());
        }
    };
    let status = visit_one_carrier(document, carrier, &mut take);
    (status, found)
}

fn visit_one_carrier(
    document: &Value,
    wanted: &CarrierPointer,
    visitor: &mut dyn FnMut(&ClassificationLocation),
) -> ClassificationWalkStatus {
    let selected = match wanted {
        CarrierPointer::Input => document
            .get("inputSchema")
            .map(|schema| (schema, "/inputSchema".to_owned(), false)),
        CarrierPointer::Result => document
            .get("outputSchema")
            .map(|schema| (schema, "/outputSchema".to_owned(), false)),
        CarrierPointer::NodeOutput(wanted_node) => document
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|nodes| select_node_carrier(nodes, "/nodes", wanted_node)),
        CarrierPointer::BodyNodeOutput { body, node } => document
            .get("bodies")
            .and_then(Value::as_object)
            .and_then(|bodies| bodies.get(body.as_str()))
            .and_then(|body| body.get("nodes"))
            .and_then(Value::as_array)
            .and_then(|nodes| {
                select_node_carrier(
                    nodes,
                    &format!("/bodies/{}/nodes", escape(body.as_str())),
                    node,
                )
            }),
    };
    let Some((schema, path, always_check)) = selected else {
        return ClassificationWalkStatus::default();
    };
    let mut analysis = Analysis::default();
    let mut status = ClassificationWalkStatus::default();
    let mut visitor = Some(visitor);
    process_carrier(
        schema,
        wanted.clone(),
        &path,
        always_check,
        false,
        &mut analysis,
        &mut status,
        &mut visitor,
    );
    status
}

fn select_node_carrier<'a>(
    nodes: &'a [Value],
    nodes_path: &str,
    wanted: &NodeId,
) -> Option<(&'a Value, String, bool)> {
    nodes.iter().enumerate().find_map(|(index, candidate)| {
        if !matches!(
            candidate.get("type").and_then(Value::as_str),
            Some("activity" | "script" | "waitEvent" | "child")
        ) || candidate.get("id").and_then(Value::as_str) != Some(wanted.as_str())
        {
            return None;
        }
        candidate
            .get("outputSchema")
            .map(|schema| (schema, format!("{nodes_path}/{index}/outputSchema"), true))
    })
}

type Visitor<'a> = Option<&'a mut dyn FnMut(&ClassificationLocation)>;

fn analyze_document(
    document: &Value,
    capture_analysis: bool,
    visitor: &mut Visitor<'_>,
) -> (ClassificationAnalysis, ClassificationWalkStatus) {
    let mut analysis = Analysis::default();
    let mut status = ClassificationWalkStatus::default();
    let mut carrier_prefixes = HashSet::new();

    for (field, carrier) in [
        ("inputSchema", CarrierPointer::Input),
        ("outputSchema", CarrierPointer::Result),
    ] {
        if let Some(schema) = document.get(field) {
            let path = format!("/{field}");
            carrier_prefixes.insert(path.clone());
            process_carrier(
                schema,
                carrier,
                &path,
                false,
                capture_analysis,
                &mut analysis,
                &mut status,
                visitor,
            );
        }
    }

    if let Some(nodes) = document.get("nodes").and_then(Value::as_array) {
        discover_node_carriers(
            nodes,
            "/nodes",
            None,
            capture_analysis,
            &mut analysis,
            &mut status,
            visitor,
            &mut carrier_prefixes,
        );
    }
    if let Some(bodies) = document.get("bodies").and_then(Value::as_object) {
        for (name, body) in bodies {
            let Some(nodes) = body.get("nodes").and_then(Value::as_array) else {
                continue;
            };
            let Ok(body_name) = NodeId::parse(name) else {
                continue;
            };
            discover_node_carriers(
                nodes,
                &format!("/bodies/{}/nodes", escape(name)),
                Some(&body_name),
                capture_analysis,
                &mut analysis,
                &mut status,
                visitor,
                &mut carrier_prefixes,
            );
        }
    }

    if capture_analysis {
        scan_outside_carriers(document, "", &carrier_prefixes, &mut analysis);
    }
    (analysis.finish(), status)
}

#[allow(clippy::too_many_arguments)]
fn discover_node_carriers(
    nodes: &[Value],
    nodes_path: &str,
    body: Option<&NodeId>,
    capture_analysis: bool,
    analysis: &mut Analysis,
    status: &mut ClassificationWalkStatus,
    visitor: &mut Visitor<'_>,
    carrier_prefixes: &mut HashSet<String>,
) {
    for (index, candidate) in nodes.iter().enumerate() {
        let Some(schema) = candidate.get("outputSchema") else {
            continue;
        };
        if !matches!(
            candidate.get("type").and_then(Value::as_str),
            Some("activity" | "script" | "waitEvent" | "child")
        ) {
            continue;
        }
        let Some(node) = candidate
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| NodeId::parse(id).ok())
        else {
            continue;
        };
        let carrier = match body {
            Some(body) => CarrierPointer::BodyNodeOutput {
                body: body.clone(),
                node,
            },
            None => CarrierPointer::NodeOutput(node),
        };
        let path = format!("{nodes_path}/{index}/outputSchema");
        carrier_prefixes.insert(path.clone());
        process_carrier(
            schema,
            carrier,
            &path,
            true,
            capture_analysis,
            analysis,
            status,
            visitor,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn process_carrier(
    schema: &Value,
    carrier: CarrierPointer,
    document_path: &str,
    always_check: bool,
    capture_analysis: bool,
    analysis: &mut Analysis,
    status: &mut ClassificationWalkStatus,
    visitor: &mut Visitor<'_>,
) {
    let bytes = serde_json::to_vec(schema).map_or(usize::MAX, |bytes| bytes.len());
    if bytes > MAX_CARRIER_BYTES || value_depth(schema) > MAX_SCHEMA_DEPTH {
        if capture_analysis {
            analysis.finding(ClassificationFindingCode::TooComplex, document_path);
        }
        status.carriers.push(ClassificationCarrierStatus::Failed(
            ClassificationCarrierFailure {
                carrier,
                code: ClassificationFindingCode::TooComplex,
                source_path: document_path.to_owned(),
            },
        ));
        return;
    }

    let report_findings = capture_analysis && (contains_annotation(schema) || always_check);
    let mut dry = CarrierWalk::new(
        schema,
        document_path,
        carrier.clone(),
        analysis,
        report_findings,
        false,
        false,
        visitor,
    );
    let _ = dry.scan_schema(schema, document_path, &mut HashSet::new());
    dry.expansions = 0;
    dry.walk_location(
        vec![SchemaNode::root(schema, document_path)],
        &mut Vec::new(),
        None,
        true,
        0,
    );
    if let Some(failure) = dry.failure.take() {
        status
            .carriers
            .push(ClassificationCarrierStatus::Failed(failure));
        return;
    }

    let mut emit = CarrierWalk::new(
        schema,
        document_path,
        carrier.clone(),
        analysis,
        false,
        true,
        capture_analysis,
        visitor,
    );
    emit.walk_location(
        vec![SchemaNode::root(schema, document_path)],
        &mut Vec::new(),
        None,
        true,
        0,
    );
    debug_assert!(emit.failure.is_none());
    status
        .carriers
        .push(ClassificationCarrierStatus::Complete(carrier));
}

#[derive(Default)]
struct Analysis {
    result: ClassificationAnalysis,
}

impl Analysis {
    fn finding(&mut self, code: ClassificationFindingCode, path: &str) {
        let is_error = code.severity() == FindingSeverity::Error;
        self.result.has_errors |= is_error;
        let path = match ClassificationPointer::parse(path) {
            Ok(path) => path,
            Err(_) if is_error => {
                let Ok(path) = ClassificationPointer::parse(safe_ancestor(path)) else {
                    self.result.findings_truncated = true;
                    return;
                };
                path
            }
            Err(_) => {
                self.result.findings_truncated = true;
                return;
            }
        };
        if self
            .result
            .findings
            .iter()
            .any(|finding| finding.code == code && finding.path == path)
        {
            return;
        }
        let finding = ClassificationFinding { code, path };
        if self.result.findings.len() < MAX_CLASSIFICATION_FINDINGS {
            self.result.findings.push(finding);
        } else {
            self.result.findings_truncated = true;
            if is_error
                && let Some(index) = self
                    .result
                    .findings
                    .iter()
                    .rposition(|existing| existing.code.severity() == FindingSeverity::Warning)
            {
                self.result.findings[index] = finding;
            }
        }
    }

    fn report(
        &mut self,
        carrier: &CarrierPointer,
        segments: &[ClassificationSegment],
        level: Level,
        declared: bool,
    ) {
        if self.result.report.len() >= MAX_CLASSIFICATION_REPORT_ENTRIES {
            self.result.report_truncated = true;
            return;
        }
        let Ok(path) = ClassificationPointer::parse(instance_path(segments)) else {
            self.result.report_truncated = true;
            return;
        };
        self.result.report.push(ClassificationReportEntry {
            carrier: carrier.clone(),
            path,
            level,
            declared,
        });
    }

    fn finish(mut self) -> ClassificationAnalysis {
        self.result.findings.sort_by(|left, right| {
            left.path
                .as_str()
                .cmp(right.path.as_str())
                .then_with(|| left.code.as_str().cmp(right.code.as_str()))
        });
        self.result.report.sort_by(|left, right| {
            carrier_sort_key(&left.carrier)
                .cmp(&carrier_sort_key(&right.carrier))
                .then_with(|| left.path.as_str().cmp(right.path.as_str()))
        });
        self.result
    }
}

#[derive(Clone)]
struct SchemaNode<'a> {
    value: &'a Value,
    document_path: String,
}

impl<'a> SchemaNode<'a> {
    fn root(value: &'a Value, document_path: &str) -> Self {
        Self {
            value,
            document_path: document_path.to_owned(),
        }
    }
}

struct CarrierWalk<'a, 'b, 'v> {
    root: &'a Value,
    root_path: &'a str,
    carrier: CarrierPointer,
    analysis: &'b mut Analysis,
    record_findings: bool,
    emit: bool,
    capture_report: bool,
    visitor: &'b mut Visitor<'v>,
    expansions: usize,
    failure: Option<ClassificationCarrierFailure>,
}

impl<'a, 'b, 'v> CarrierWalk<'a, 'b, 'v> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        root: &'a Value,
        root_path: &'a str,
        carrier: CarrierPointer,
        analysis: &'b mut Analysis,
        record_findings: bool,
        emit: bool,
        capture_report: bool,
        visitor: &'b mut Visitor<'v>,
    ) -> Self {
        Self {
            root,
            root_path,
            carrier,
            analysis,
            record_findings,
            emit,
            capture_report,
            visitor,
            expansions: 0,
            failure: None,
        }
    }

    fn fail(&mut self, code: ClassificationFindingCode, path: &str) {
        if self.record_findings {
            self.analysis.finding(code, path);
        }
        if matches!(
            code,
            ClassificationFindingCode::TooComplex
                | ClassificationFindingCode::UnresolvedRef
                | ClassificationFindingCode::UnsupportedKeyword
        ) && self.failure.is_none()
        {
            self.failure = Some(ClassificationCarrierFailure {
                carrier: self.carrier.clone(),
                code,
                source_path: path.to_owned(),
            });
        }
    }

    fn emit_location(
        &mut self,
        segments: &[ClassificationSegment],
        source_path: &str,
        level: Level,
        declared: bool,
    ) {
        if !self.emit {
            return;
        }
        if self.capture_report {
            self.analysis
                .report(&self.carrier, segments, level, declared);
        }
        if let Some(visitor) = self.visitor.as_deref_mut() {
            visitor(&ClassificationLocation {
                carrier: self.carrier.clone(),
                location: segments.to_vec(),
                source_path: source_path.to_owned(),
                level,
                declared,
            });
        }
    }

    fn walk_location(
        &mut self,
        schemas: Vec<SchemaNode<'a>>,
        segments: &mut Vec<ClassificationSegment>,
        inherited: Option<Level>,
        root: bool,
        depth: usize,
    ) {
        if depth > MAX_SCHEMA_DEPTH {
            self.fail(
                ClassificationFindingCode::TooComplex,
                &schemas[0].document_path,
            );
            return;
        }
        let expanded = self.expand(schemas, &mut HashSet::new());
        if expanded.is_empty() || self.failure.is_some() {
            return;
        }
        let declared = expanded
            .iter()
            .any(|schema| annotation(schema.value).is_some());
        let mut effective = if root {
            expanded
                .iter()
                .filter_map(|schema| valid_annotation(schema.value))
                .max()
                .unwrap_or(Level::UNMARKED)
        } else {
            inherited.unwrap_or(Level::UNMARKED)
        };
        for schema in &expanded {
            if let Some(level) = valid_annotation(schema.value) {
                if !root && Some(level) < inherited {
                    self.fail(ClassificationFindingCode::Loosened, &schema.document_path);
                }
                effective = effective.stricter(level);
            }
        }
        let source_path = expanded[0].document_path.clone();
        self.emit_location(segments, &source_path, effective, declared);

        let mut property_sources: BTreeMap<String, String> = BTreeMap::new();
        for schema in &expanded {
            if let Some(entries) = schema.value.get("properties").and_then(Value::as_object) {
                for name in entries.keys() {
                    property_sources.entry(name.clone()).or_insert_with(|| {
                        format!("{}/properties/{}", schema.document_path, escape(name))
                    });
                }
            }
        }
        for (name, first_source) in property_sources {
            let mut children = Vec::new();
            let mut unsupported = false;
            for schema in &expanded {
                if let Some(child) = schema
                    .value
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get(&name))
                {
                    children.push(SchemaNode {
                        value: child,
                        document_path: format!(
                            "{}/properties/{}",
                            schema.document_path,
                            escape(&name)
                        ),
                    });
                } else if schema
                    .value
                    .get("patternProperties")
                    .and_then(Value::as_object)
                    .is_some_and(|patterns| !patterns.is_empty())
                {
                    unsupported = true;
                } else if let Some(child) = schema
                    .value
                    .get("additionalProperties")
                    .filter(|value| value.is_object())
                {
                    children.push(SchemaNode {
                        value: child,
                        document_path: format!("{}/additionalProperties", schema.document_path),
                    });
                }
            }
            let child_declared = children
                .iter()
                .any(|schema| annotation(schema.value).is_some());
            let inherited = if unsupported || children.is_empty() {
                effective.stricter(Level::UNMARKED)
            } else {
                effective
            };
            if !child_declared && matches!(inherited, Level::Public | Level::Internal) {
                self.fail(ClassificationFindingCode::InheritedOpen, &first_source);
            }
            segments.push(ClassificationSegment::Property(name));
            self.walk_location(children, segments, Some(inherited), false, depth + 1);
            segments.pop();
        }

        let maximum_prefix = expanded
            .iter()
            .filter_map(|schema| schema.value.get("prefixItems").and_then(Value::as_array))
            .map(Vec::len)
            .max()
            .unwrap_or(0);
        for index in 0..maximum_prefix {
            let mut children = Vec::new();
            let mut source = None;
            for schema in &expanded {
                if let Some(child) = schema
                    .value
                    .get("prefixItems")
                    .and_then(Value::as_array)
                    .and_then(|items| items.get(index))
                {
                    let path = format!("{}/prefixItems/{index}", schema.document_path);
                    source.get_or_insert_with(|| path.clone());
                    children.push(SchemaNode {
                        value: child,
                        document_path: path,
                    });
                } else if let Some(child) =
                    schema.value.get("items").filter(|value| value.is_object())
                {
                    let path = format!("{}/items", schema.document_path);
                    source.get_or_insert_with(|| path.clone());
                    children.push(SchemaNode {
                        value: child,
                        document_path: path,
                    });
                }
            }
            let child_declared = children
                .iter()
                .any(|schema| annotation(schema.value).is_some());
            let inherited = if children.is_empty() {
                effective.stricter(Level::UNMARKED)
            } else {
                effective
            };
            if !child_declared && matches!(inherited, Level::Public | Level::Internal) {
                self.fail(
                    ClassificationFindingCode::InheritedOpen,
                    source.as_deref().unwrap_or(&source_path),
                );
            }
            segments.push(ClassificationSegment::Index(index));
            self.walk_location(children, segments, Some(inherited), false, depth + 1);
            segments.pop();
        }

        let mut additional = Vec::new();
        let mut pattern_floor = None;
        for schema in &expanded {
            if schema
                .value
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|patterns| !patterns.is_empty())
            {
                pattern_floor
                    .get_or_insert_with(|| format!("{}/patternProperties", schema.document_path));
            } else if let Some(value) = schema
                .value
                .get("additionalProperties")
                .filter(|value| value.is_object())
            {
                additional.push(SchemaNode {
                    value,
                    document_path: format!("{}/additionalProperties", schema.document_path),
                });
            }
        }
        if !additional.is_empty() {
            let inherited = if pattern_floor.is_some() {
                effective.stricter(Level::UNMARKED)
            } else {
                effective
            };
            segments.push(ClassificationSegment::AdditionalProperty);
            self.walk_location(additional, segments, Some(inherited), false, depth + 1);
            segments.pop();
        } else if let Some(path) = pattern_floor {
            segments.push(ClassificationSegment::AdditionalProperty);
            self.emit_location(segments, &path, effective.stricter(Level::UNMARKED), false);
            segments.pop();
        }

        let items: Vec<_> = expanded
            .iter()
            .filter_map(|schema| {
                schema
                    .value
                    .get("items")
                    .filter(|value| value.is_object())
                    .map(|value| SchemaNode {
                        value,
                        document_path: format!("{}/items", schema.document_path),
                    })
            })
            .collect();
        if !items.is_empty() {
            segments.push(ClassificationSegment::RemainingItem);
            self.walk_location(items, segments, Some(effective), false, depth + 1);
            segments.pop();
        }
    }

    fn expand(
        &mut self,
        schemas: Vec<SchemaNode<'a>>,
        active: &mut HashSet<String>,
    ) -> Vec<SchemaNode<'a>> {
        let mut expanded = Vec::new();
        for schema in schemas {
            expanded.push(schema.clone());
            let Some(object) = schema.value.as_object() else {
                continue;
            };
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                self.expansions += 1;
                if self.expansions > MAX_REF_EXPANSIONS {
                    self.fail(ClassificationFindingCode::TooComplex, &schema.document_path);
                } else if !reference.starts_with("#/") {
                    self.fail(
                        ClassificationFindingCode::UnresolvedRef,
                        &format!("{}/$ref", schema.document_path),
                    );
                } else if !active.insert(reference.to_owned()) {
                    self.fail(ClassificationFindingCode::TooComplex, &schema.document_path);
                } else if let Some(target) = self.root.pointer(&reference[1..]) {
                    let target_path = format!("{}{}", self.root_path, &reference[1..]);
                    expanded.extend(self.expand(
                        vec![SchemaNode {
                            value: target,
                            document_path: target_path,
                        }],
                        active,
                    ));
                    active.remove(reference);
                } else {
                    self.fail(
                        ClassificationFindingCode::UnresolvedRef,
                        &format!("{}/$ref", schema.document_path),
                    );
                    active.remove(reference);
                }
            }
            for keyword in ["allOf", "anyOf", "oneOf"] {
                if let Some(branches) = object.get(keyword).and_then(Value::as_array) {
                    for (index, branch) in branches.iter().enumerate() {
                        expanded.extend(self.expand(
                            vec![SchemaNode {
                                value: branch,
                                document_path: format!(
                                    "{}/{keyword}/{index}",
                                    schema.document_path
                                ),
                            }],
                            active,
                        ));
                    }
                }
            }
        }
        expanded
    }

    fn scan_schema(&mut self, schema: &Value, path: &str, active: &mut HashSet<String>) -> bool {
        let Some(object) = schema.as_object() else {
            return false;
        };
        let mut annotation_reachable = object.contains_key(ANNOTATION);
        if let Some(annotation) = object.get(ANNOTATION)
            && annotation
                .as_str()
                .and_then(|level| Level::parse(level).ok())
                .is_none()
        {
            self.fail(
                ClassificationFindingCode::InvalidLevel,
                &format!("{path}/{ANNOTATION}"),
            );
        }
        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            self.expansions += 1;
            if self.expansions > MAX_REF_EXPANSIONS {
                self.fail(ClassificationFindingCode::TooComplex, path);
            } else if !reference.starts_with("#/") {
                self.fail(
                    ClassificationFindingCode::UnresolvedRef,
                    &format!("{path}/$ref"),
                );
            } else if !active.insert(reference.to_owned()) {
                self.fail(ClassificationFindingCode::TooComplex, path);
            } else if let Some(target) = self.root.pointer(&reference[1..]) {
                let target_path = format!("{}{}", self.root_path, &reference[1..]);
                annotation_reachable |= self.scan_schema(target, &target_path, active);
                active.remove(reference);
            } else {
                self.fail(
                    ClassificationFindingCode::UnresolvedRef,
                    &format!("{path}/$ref"),
                );
                active.remove(reference);
            }
        }
        for (keyword, value) in object {
            if keyword == "$ref" || keyword == ANNOTATION {
                continue;
            }
            let child_reachable = match value {
                Value::Object(_) => {
                    self.scan_schema(value, &format!("{path}/{}", escape(keyword)), active)
                }
                Value::Array(values) => {
                    let mut reachable = false;
                    for (index, value) in values.iter().enumerate() {
                        reachable |= self.scan_schema(
                            value,
                            &format!("{path}/{}/{index}", escape(keyword)),
                            active,
                        );
                    }
                    reachable
                }
                _ => false,
            };
            if UNSUPPORTED_APPLICATORS.contains(&keyword.as_str()) && child_reachable {
                self.fail(
                    ClassificationFindingCode::UnsupportedKeyword,
                    &format!("{path}/{keyword}"),
                );
            }
            annotation_reachable |= child_reachable;
        }
        annotation_reachable
    }
}

fn scan_outside_carriers(
    value: &Value,
    path: &str,
    carrier_prefixes: &HashSet<String>,
    analysis: &mut Analysis,
) {
    if carrier_prefixes.contains(path) {
        return;
    }
    match value {
        Value::Object(object) => {
            for (name, value) in object {
                let child = format!("{path}/{}", escape(name));
                if name == ANNOTATION {
                    analysis.finding(ClassificationFindingCode::NotApplicable, &child);
                } else {
                    scan_outside_carriers(value, &child, carrier_prefixes, analysis);
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                scan_outside_carriers(
                    value,
                    &format!("{path}/{index}"),
                    carrier_prefixes,
                    analysis,
                );
            }
        }
        _ => {}
    }
}

fn annotation(value: &Value) -> Option<&Value> {
    value.as_object()?.get(ANNOTATION)
}

fn valid_annotation(value: &Value) -> Option<Level> {
    Level::parse(annotation(value)?.as_str()?).ok()
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

fn value_depth(root: &Value) -> usize {
    let mut maximum = 0;
    let mut pending = vec![(root, 1usize)];
    while let Some((value, depth)) = pending.pop() {
        maximum = maximum.max(depth);
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => {
                pending.extend(values.values().map(|value| (value, depth + 1)));
            }
            _ => {}
        }
    }
    maximum
}

fn instance_path(segments: &[ClassificationSegment]) -> String {
    let mut path = String::new();
    for segment in segments {
        path.push('/');
        match segment {
            ClassificationSegment::Property(name) => path.push_str(&escape(name)),
            ClassificationSegment::Index(index) => path.push_str(&index.to_string()),
            ClassificationSegment::AdditionalProperty | ClassificationSegment::RemainingItem => {
                path.push('*');
            }
        }
    }
    path
}

fn carrier_sort_key(carrier: &CarrierPointer) -> (u8, &str, &str) {
    match carrier {
        CarrierPointer::Input => (0, "", ""),
        CarrierPointer::Result => (1, "", ""),
        CarrierPointer::NodeOutput(id) => (2, "", id.as_str()),
        CarrierPointer::BodyNodeOutput { body, node } => (3, body.as_str(), node.as_str()),
    }
}

fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn safe_ancestor(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut safe = String::new();
    for segment in path.strip_prefix('/').unwrap_or(path).split('/') {
        if segment.chars().any(char::is_control)
            || safe.len().saturating_add(segment.len()).saturating_add(1) > 2_048
        {
            break;
        }
        safe.push('/');
        safe.push_str(segment);
    }
    safe
}
