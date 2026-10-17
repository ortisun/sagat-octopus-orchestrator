//! Bounded, value-free publication analysis for classification annotations.

use std::collections::{BTreeMap, HashSet};

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationFinding, ClassificationFindingCode,
    ClassificationPointer, ClassificationReportEntry, MAX_CARRIER_BYTES,
    MAX_CLASSIFICATION_FINDINGS, MAX_CLASSIFICATION_REPORT_ENTRIES, MAX_REF_EXPANSIONS,
    MAX_SCHEMA_DEPTH,
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
    let mut analysis = Analysis::default();
    let mut carrier_prefixes = Vec::new();

    for (field, carrier) in [
        ("inputSchema", CarrierPointer::Input),
        ("outputSchema", CarrierPointer::Result),
    ] {
        if let Some(schema) = document.get(field) {
            let path = format!("/{field}");
            carrier_prefixes.push(path.clone());
            analysis.carrier(schema, carrier, &path, false);
        }
    }

    if let Some(nodes) = document.get("nodes").and_then(Value::as_array) {
        for (index, node) in nodes.iter().enumerate() {
            let Some(schema) = node.get("outputSchema") else {
                continue;
            };
            let path = format!("/nodes/{index}/outputSchema");
            let supported = matches!(
                node.get("type").and_then(Value::as_str),
                Some("activity" | "script" | "waitEvent" | "child")
            );
            let carrier = node
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| NodeId::parse(id).ok())
                .map(CarrierPointer::NodeOutput);
            if supported {
                carrier_prefixes.push(path.clone());
                if let Some(carrier) = carrier {
                    analysis.carrier(schema, carrier, &path, true);
                }
            }
        }
    }

    scan_outside_carriers(document, "", &carrier_prefixes, &mut analysis);
    analysis.finish()
}

#[derive(Default)]
struct Analysis {
    result: ClassificationAnalysis,
}

impl Analysis {
    fn finding(&mut self, code: ClassificationFindingCode, path: &str) {
        if self.result.findings.len() == MAX_CLASSIFICATION_FINDINGS {
            self.result.findings_truncated = true;
            return;
        }
        if self.result.findings.len() > MAX_CLASSIFICATION_FINDINGS {
            return;
        }
        let Ok(path) = ClassificationPointer::parse(path) else {
            self.result.findings_truncated = true;
            return;
        };
        if !self
            .result
            .findings
            .iter()
            .any(|finding| finding.code == code && finding.path == path)
        {
            self.result
                .findings
                .push(ClassificationFinding { code, path });
        }
    }

    fn report(&mut self, carrier: &CarrierPointer, path: &str, level: Level, declared: bool) {
        if self.result.report.len() == MAX_CLASSIFICATION_REPORT_ENTRIES {
            self.result.report_truncated = true;
            return;
        }
        if self.result.report.len() > MAX_CLASSIFICATION_REPORT_ENTRIES {
            return;
        }
        let Ok(path) = ClassificationPointer::parse(path) else {
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

    fn carrier(
        &mut self,
        schema: &Value,
        carrier: CarrierPointer,
        document_path: &str,
        always_check: bool,
    ) {
        let bytes = serde_json::to_vec(schema).map_or(usize::MAX, |bytes| bytes.len());
        if bytes > MAX_CARRIER_BYTES || value_depth(schema) > MAX_SCHEMA_DEPTH {
            self.finding(ClassificationFindingCode::TooComplex, document_path);
            return;
        }
        let carries_annotation = contains_annotation(schema);
        let mut walk = CarrierWalk::new(schema, document_path, carrier, self);
        walk.walk_location(
            vec![SchemaNode::root(schema, document_path)],
            "",
            None,
            true,
            0,
        );
        if carries_annotation || always_check {
            walk.expansions = 0;
            walk.scan_schema(schema, document_path, &mut HashSet::new());
        }
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

struct CarrierWalk<'a, 'b> {
    root: &'a Value,
    root_path: &'a str,
    carrier: CarrierPointer,
    analysis: &'b mut Analysis,
    expansions: usize,
}

impl<'a, 'b> CarrierWalk<'a, 'b> {
    fn new(
        root: &'a Value,
        root_path: &'a str,
        carrier: CarrierPointer,
        analysis: &'b mut Analysis,
    ) -> Self {
        Self {
            root,
            root_path,
            carrier,
            analysis,
            expansions: 0,
        }
    }

    fn walk_location(
        &mut self,
        schemas: Vec<SchemaNode<'a>>,
        instance_path: &str,
        inherited: Option<Level>,
        root: bool,
        depth: usize,
    ) {
        if depth > MAX_SCHEMA_DEPTH {
            self.analysis.finding(
                ClassificationFindingCode::TooComplex,
                &schemas[0].document_path,
            );
            return;
        }
        let expanded = self.expand(schemas, &mut HashSet::new());
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
                    self.analysis
                        .finding(ClassificationFindingCode::Loosened, &schema.document_path);
                }
                effective = effective.stricter(level);
            }
        }
        self.analysis
            .report(&self.carrier, instance_path, effective, declared);

        let mut properties: BTreeMap<String, Vec<SchemaNode<'a>>> = BTreeMap::new();
        for schema in &expanded {
            if let Some(entries) = schema.value.get("properties").and_then(Value::as_object) {
                for (name, child) in entries {
                    properties
                        .entry(name.clone())
                        .or_default()
                        .push(SchemaNode {
                            value: child,
                            document_path: format!(
                                "{}/properties/{}",
                                schema.document_path,
                                escape(name)
                            ),
                        });
                }
            }
        }
        for (name, children) in properties {
            let child_path = format!("{instance_path}/{}", escape(&name));
            let child_declared = children
                .iter()
                .any(|schema| annotation(schema.value).is_some());
            if !child_declared && matches!(effective, Level::Public | Level::Internal) {
                self.analysis.finding(
                    ClassificationFindingCode::InheritedOpen,
                    &children[0].document_path,
                );
            }
            self.walk_location(children, &child_path, Some(effective), false, depth + 1);
        }

        let mut positional: BTreeMap<usize, Vec<SchemaNode<'a>>> = BTreeMap::new();
        for schema in &expanded {
            if let Some(items) = schema.value.get("prefixItems").and_then(Value::as_array) {
                for (index, item) in items.iter().enumerate() {
                    positional.entry(index).or_default().push(SchemaNode {
                        value: item,
                        document_path: format!("{}/prefixItems/{index}", schema.document_path),
                    });
                }
            }
        }
        for (index, children) in positional {
            let child_path = format!("{instance_path}/{index}");
            let child_declared = children
                .iter()
                .any(|schema| annotation(schema.value).is_some());
            if !child_declared && matches!(effective, Level::Public | Level::Internal) {
                self.analysis.finding(
                    ClassificationFindingCode::InheritedOpen,
                    &children[0].document_path,
                );
            }
            self.walk_location(children, &child_path, Some(effective), false, depth + 1);
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
                    self.analysis
                        .finding(ClassificationFindingCode::TooComplex, &schema.document_path);
                } else if reference.starts_with("#/") && active.insert(reference.to_owned()) {
                    if let Some(target) = self.root.pointer(&reference[1..]) {
                        let target_path = format!("{}{}", self.root_path, &reference[1..]);
                        expanded.extend(self.expand(
                            vec![SchemaNode {
                                value: target,
                                document_path: target_path,
                            }],
                            active,
                        ));
                    }
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

    fn scan_schema(&mut self, schema: &Value, path: &str, active: &mut HashSet<String>) {
        let Some(object) = schema.as_object() else {
            return;
        };
        if let Some(annotation) = object.get(ANNOTATION)
            && (annotation
                .as_str()
                .and_then(|level| Level::parse(level).ok()))
            .is_none()
        {
            self.analysis.finding(
                ClassificationFindingCode::InvalidLevel,
                &format!("{path}/{ANNOTATION}"),
            );
        }
        for keyword in UNSUPPORTED_APPLICATORS {
            if let Some(value) = object.get(keyword)
                && contains_annotation(value)
            {
                self.analysis.finding(
                    ClassificationFindingCode::UnsupportedKeyword,
                    &format!("{path}/{keyword}"),
                );
            }
        }
        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            self.expansions += 1;
            if self.expansions > MAX_REF_EXPANSIONS {
                self.analysis
                    .finding(ClassificationFindingCode::TooComplex, path);
            } else if !reference.starts_with("#/") {
                self.analysis.finding(
                    ClassificationFindingCode::UnresolvedRef,
                    &format!("{path}/$ref"),
                );
            } else if !active.insert(reference.to_owned()) {
                self.analysis
                    .finding(ClassificationFindingCode::TooComplex, path);
            } else if let Some(target) = self.root.pointer(&reference[1..]) {
                let target_path = format!("{}{}", self.root_path, &reference[1..]);
                self.scan_schema(target, &target_path, active);
                active.remove(reference);
            } else {
                self.analysis.finding(
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
            match value {
                Value::Object(_) => {
                    self.scan_schema(value, &format!("{path}/{}", escape(keyword)), active)
                }
                Value::Array(values) => {
                    for (index, value) in values.iter().enumerate() {
                        self.scan_schema(
                            value,
                            &format!("{path}/{}/{index}", escape(keyword)),
                            active,
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

fn scan_outside_carriers(
    value: &Value,
    path: &str,
    carrier_prefixes: &[String],
    analysis: &mut Analysis,
) {
    if carrier_prefixes.iter().any(|carrier| path == carrier) {
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

fn carrier_sort_key(carrier: &CarrierPointer) -> (u8, &str) {
    match carrier {
        CarrierPointer::Input => (0, ""),
        CarrierPointer::Result => (1, ""),
        CarrierPointer::NodeOutput(id) => (2, id.as_str()),
    }
}

fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
