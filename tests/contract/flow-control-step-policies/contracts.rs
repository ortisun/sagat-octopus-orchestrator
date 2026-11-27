//! The canonical contracts and the fragments of feature 022 say the same thing for the 0.3.0 slice (T022-060 narrowed
//! to the concurrency parts, T022-087 the rate-limit parts; decision D022-110): the shapes of the two key listings, of
//! `Run.concurrency`, `Run.rateLimit` and of the two IR policies are the same in the fragment and in the canonical
//! document, and every code the server gives for them is named where a client reads the canonical contract. A shape
//! changed in one document and not the other, or a code the server gives that no canonical document names, fails here.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sagat_application::flow_control::{FlowControlCapability, FlowControlRefusal};
use serde_json::{Map, Value};

const CANONICAL_OPENAPI: &str = "specs/001-platform-foundation/contracts/openapi.yaml";
const FRAGMENT_OPENAPI: &str =
    "specs/022-flow-control-step-policies/contracts/openapi-flow-control.yaml";
const CANONICAL_IR: &str = "specs/001-platform-foundation/contracts/workflow.schema.json";
const FRAGMENT_IR: &str =
    "specs/022-flow-control-step-policies/contracts/workflow-ir-fragment.schema.json";
/// How the fragment points into the canonical document; the canonical document writes the same reference as `#`.
const CANONICAL_REFERENCE: &str = "../../001-platform-foundation/contracts/openapi.yaml#";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn text(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn yaml(path: &str) -> Value {
    serde_saphyr::from_str(&text(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn json(path: &str) -> Value {
    serde_json::from_str(&text(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The shape alone: the prose (`description`, `summary`), the tags and the markers of origin (`x-...`) removed, and a
/// reference into the canonical document written as the canonical document writes it.
fn shape(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, value)| {
                    let prose =
                        matches!(key.as_str(), "description" | "summary") && value.is_string();
                    let tags = key.as_str() == "tags" && value.is_array();
                    !prose && !tags && !key.starts_with("x-")
                })
                .map(|(key, value)| {
                    let value = match (key.as_str(), value.as_str()) {
                        ("$ref", Some(reference)) => {
                            Value::String(reference.replacen(CANONICAL_REFERENCE, "#", 1))
                        }
                        _ => shape(value),
                    };
                    (key.clone(), value)
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(shape).collect()),
        other => other.clone(),
    }
}

/// Where two shapes differ, each difference as a pointer and the two values.
fn differences(fragment: &Value, canonical: &Value, at: &str, out: &mut Vec<String>) {
    match (fragment, canonical) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let here = format!("{at}/{key}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => differences(left, right, &here, out),
                    (Some(left), None) => out.push(format!("{here}: only in the fragment: {left}")),
                    (None, Some(right)) => {
                        out.push(format!("{here}: only in the canonical document: {right}"))
                    }
                    (None, None) => {}
                }
            }
        }
        (left, right) if left != right => {
            out.push(format!("{at}: fragment {left}, canonical {right}"))
        }
        _ => {}
    }
}

/// The upper snake case codes a text names (`CONCURRENCY_SELF_CHILD`), not the words around them.
fn codes(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .filter(|word| {
            word.contains('_')
                && word.len() > 3
                && word.starts_with(|c: char| c.is_ascii_uppercase())
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_fragment_and_the_canonical_openapi_agree_on_every_shape_of_the_slice() {
    let fragment = yaml(FRAGMENT_OPENAPI);
    let canonical = yaml(CANONICAL_OPENAPI);
    let mut out = Vec::new();
    for path in [
        "/namespaces/{namespaceId}/concurrency-keys",
        "/namespaces/{namespaceId}/rate-limit-keys",
    ] {
        differences(
            &shape(&fragment["paths"][path]),
            &shape(&canonical["paths"][path]),
            &format!("paths{path}"),
            &mut out,
        );
    }
    for name in [
        "ConcurrencyKey",
        "ConcurrencyKeyPage",
        "ConcurrencyKeyGroup",
        "RateLimitKey",
        "RateLimitKeyPage",
        "RateLimitKeyGroup",
    ] {
        differences(
            &shape(&fragment["components"]["schemas"][name]),
            &shape(&canonical["components"]["schemas"][name]),
            name,
            &mut out,
        );
    }
    // the fragment writes the two `Run` properties inline; the canonical document names their schemas
    let run = &fragment["components"]["schemas"]["RunFlowControlFields"]["properties"];
    for (property, schema) in [
        ("concurrency", "RunConcurrency"),
        ("rateLimit", "RunRateLimit"),
    ] {
        assert_eq!(
            canonical["components"]["schemas"]["Run"]["properties"][property]["$ref"],
            Value::String(format!("#/components/schemas/{schema}")),
            "Run.{property} is the schema {schema}"
        );
        differences(
            &shape(&run[property]),
            &shape(&canonical["components"]["schemas"][schema]),
            &format!("Run.{property}"),
            &mut out,
        );
    }
    let waiting: Vec<&Value> =
        canonical["components"]["schemas"]["RuntimeNode"]["properties"]["waitingOn"]["enum"]
            .as_array()
            .expect("the waitingOn enum")
            .iter()
            .collect();
    for value in fragment["components"]["schemas"]["RuntimeNodeWaitingOnAdditions"]["enum"]
        .as_array()
        .expect("the additions")
    {
        if !waiting.contains(&value) {
            out.push(format!(
                "RuntimeNode.waitingOn: {value} is not in the canonical enum"
            ));
        }
    }
    assert!(
        out.is_empty(),
        "the fragment and the canonical OpenAPI differ:\n{}",
        out.join("\n")
    );
}

#[test]
fn the_fragment_and_the_canonical_ir_agree_on_the_two_policies_and_their_rules() {
    let fragment = json(FRAGMENT_IR);
    let canonical = json(CANONICAL_IR);
    let mut out = Vec::new();
    for name in ["concurrencyPolicy", "rateLimitPolicy"] {
        let (draft, merged) = (&fragment["$defs"][name], &canonical["$defs"][name]);
        differences(&shape(draft), &shape(merged), name, &mut out);
        // the canonical schema has no `x-semantic-rules`: what the fragment lists there is said in its descriptions
        let said = merged.to_string();
        let rules = draft["x-semantic-rules"]
            .as_array()
            .expect("the rules of the fragment");
        for code in rules.iter().filter_map(Value::as_str).flat_map(codes) {
            if !codes(&said).contains(&code) {
                out.push(format!("{name}: the canonical schema does not name {code}"));
            }
        }
    }
    assert!(
        out.is_empty(),
        "the fragment and the canonical IR differ:\n{}",
        out.join("\n")
    );
}

/// Every refusal the server gives for flow control, the compiler holding the list whole.
fn every_refusal() -> Vec<FlowControlRefusal> {
    let all = [
        FlowControlRefusal::ConcurrencyQueueFull,
        FlowControlRefusal::RateLimitQueueFull,
        FlowControlRefusal::Limit {
            limit: "queued_runs",
        },
        FlowControlRefusal::ActivationPending {
            capability: FlowControlCapability::ConcurrencyKeys,
        },
        FlowControlRefusal::CapabilityDisabled {
            capability: FlowControlCapability::RateLimits,
        },
        FlowControlRefusal::BinaryCapabilityMissing,
        FlowControlRefusal::PermitUnavailable,
        FlowControlRefusal::SlotUnavailable,
    ];
    for refusal in all {
        // a variant added to the enumeration fails to compile here until it is in the list above
        match refusal {
            FlowControlRefusal::ConcurrencyQueueFull
            | FlowControlRefusal::RateLimitQueueFull
            | FlowControlRefusal::Limit { .. }
            | FlowControlRefusal::ActivationPending { .. }
            | FlowControlRefusal::CapabilityDisabled { .. }
            | FlowControlRefusal::BinaryCapabilityMissing
            | FlowControlRefusal::PermitUnavailable
            | FlowControlRefusal::SlotUnavailable => {}
        }
    }
    all.to_vec()
}

#[test]
fn the_canonical_openapi_names_every_refusal_reason_of_flow_control() {
    let canonical = yaml(CANONICAL_OPENAPI);
    let said = codes(
        canonical["components"]["schemas"]["Error"]["properties"]["details"]["description"]
            .as_str()
            .expect("the description of Error.details"),
    );
    let missing: Vec<&str> = every_refusal()
        .into_iter()
        .map(FlowControlRefusal::reason)
        .filter(|reason| !said.contains(*reason))
        .collect();
    assert!(
        missing.is_empty(),
        "Error.details does not name these reasons: {missing:?}"
    );
}

#[test]
fn the_canonical_openapi_names_every_publication_code_of_the_two_gates() {
    let errors = text("specs/022-flow-control-step-policies/contracts/errors.md");
    let table = errors
        .split("## Publication and validation")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the publication table");
    // the rows of the two gates; a row owned by feature 031 and the `UNSUPPORTED_NODE` rows (a dependency, said in the
    // IR schema) are not this listing's
    let rows: BTreeSet<String> = table
        .lines()
        .filter(|line| line.starts_with("| `") && !line.contains("owned by 031"))
        .filter_map(|line| line.split('`').nth(1).map(str::to_owned))
        .filter(|code| {
            code != "UNSUPPORTED_NODE"
                && (code.contains("CONCURRENCY")
                    || code.contains("RATE_LIMIT")
                    || code.contains("FLOW_CONTROL"))
        })
        .collect();
    assert!(
        rows.len() >= 9,
        "the publication rows of the two gates: {rows:?}"
    );
    let server = text("crates/application/src/definitions.rs");
    let canonical = yaml(CANONICAL_OPENAPI);
    let said = codes(
        canonical["components"]["schemas"]["DefinitionProblem"]["properties"]["code"]["description"]
            .as_str()
            .expect("the description of DefinitionProblem.code"),
    );
    for code in &rows {
        assert!(
            server.contains(&format!("\"{code}\"")),
            "the server reports {code}"
        );
    }
    let missing: Vec<&String> = rows.iter().filter(|code| !said.contains(*code)).collect();
    assert!(
        missing.is_empty(),
        "DefinitionProblem.code does not name these codes: {missing:?}"
    );
}

#[test]
fn the_canonical_openapi_names_every_step_failure_code_of_flow_control() {
    let errors = text("specs/022-flow-control-step-policies/contracts/errors.md");
    let table = errors
        .split("## Durable failure codes")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the failure code table");
    // the codes a step or a child node fails with; the run causes are on `Run.failureCode`, the existing codes
    // (`EXPRESSION_ERROR`, ...) are not this feature's
    let step: BTreeSet<String> = table
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter(|line| {
            let place = line.split('|').nth(2).unwrap_or_default();
            place.contains("step") || place.contains("child node")
        })
        .flat_map(|line| codes(line.split('|').nth(1).unwrap_or_default()))
        .collect();
    assert!(step.len() >= 8, "the step and child node codes: {step:?}");
    let canonical = yaml(CANONICAL_OPENAPI);
    let said = codes(
        canonical["components"]["schemas"]["RuntimeNode"]["properties"]["failureCode"]["description"]
            .as_str()
            .unwrap_or_default(),
    );
    let missing: Vec<&String> = step.iter().filter(|code| !said.contains(*code)).collect();
    assert!(
        missing.is_empty(),
        "RuntimeNode.failureCode does not name these codes: {missing:?}"
    );
}
