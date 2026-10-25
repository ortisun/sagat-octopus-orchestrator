//! What a person is told when a document is wrong: where, what it means, what to do and where it is
//! explained (US04).
//!
//! The codes are the ones the platform's codec and validation return; the help is the tool's own and
//! is checked against both ends: a test reads the codes out of the codec's source and fails when one
//! has no help (a new rule nobody explained), or when the help names a code the codec no longer has,
//! and another test fails when `docs/quickstarts/errors.md` lacks a section for a code.

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationReportEntry, FindingSeverity,
};
use sagat_application::definitions::{IrCodec, problems_of};
use sagat_infra_ir::JsonIrCodec;

/// The help for each code: one or two sentences that say what to do.
pub const CATALOG: &[(&str, &str)] = &[
    (
        "DOCUMENT_TOO_LARGE",
        "A workflow document is at most 1 MiB. Move what repeats into child workflows and keep large constants out of the document.",
    ),
    (
        "DOCUMENT_MALFORMED",
        "The file is not one JSON or YAML object. Check the syntax at the line the parser names; a workflow is an object with apiVersion, kind, metadata, entrypoint and nodes.",
    ),
    (
        "SCHEMA",
        "The document does not match the workflow schema at the path shown (a JSON pointer). The schema is specs/001-platform-foundation/contracts/workflow.schema.json; `sagat new --list` has documents that match it.",
    ),
    (
        "UNSUPPORTED_NODE",
        "This node, or this combination of nodes, is in the schema but cannot be published yet: for example an approval inside a forEach or while body (the error's dependency names what it waits for), or a shape the enabled subset does not allow. Move the part that is not allowed into a child workflow.",
    ),
    (
        "EXPRESSION",
        "An expression does not compile in the bounded expression language. Fix the syntax; there are no calls, no loops and no assignments.",
    ),
    (
        "NAME_MISMATCH",
        "metadata.name must be the name the workflow is published as. Rename the document or the workflow so they agree.",
    ),
    (
        "UNKNOWN_SCRIPT",
        "A script node pins an artifactDigest that no ready script of this tenant has. Publish the script (`sagat script publish`) and use the digest it prints.",
    ),
    (
        "SECRET_UNAVAILABLE",
        "A node names a secret that does not exist in the namespace, or that you may not use. Create it there, or ask an administrator for access to it.",
    ),
    (
        "INVALID_DATABASE_NODE",
        "Check the connection reference, query or execute operation, typed parameters and finite limits. Managed database steps cannot declare automatic retries or impersonate an ordinary activity. A managed database step belongs to the root graph of a definition without loops, collections or a continuation policy.",
    ),
    (
        "INVALID_NODE_ID",
        "A node id is up to 64 characters of letters, digits, '_', '.' and '-', starting with a letter or a digit.",
    ),
    (
        "INVALID_ACTIVITY_NAME",
        "An activity name is up to 128 characters of letters, digits, '_', '.', ':' and '-', starting with a letter or a digit, like payment.validate.",
    ),
    (
        "INVALID_EVENT_TYPE",
        "An event type is up to 128 characters of letters, digits, '_', '.', ':' and '-', starting with a letter or a digit, like payment.confirmed.",
    ),
    (
        "INVALID_EXPRESSION",
        "An expression is not empty, is at most 2048 bytes and has no control characters.",
    ),
    (
        "NO_NODES",
        "A workflow needs at least one node, and the entrypoint names one of them.",
    ),
    (
        "TOO_MANY_NODES",
        "A workflow has at most 10000 nodes. Split it into child workflows.",
    ),
    (
        "TOO_MANY_PARALLELS",
        "A workflow has at most 32 parallel nodes. Move some branches into child workflows.",
    ),
    (
        "TOO_MANY_SECRETS",
        "A workflow declares at most 128 distinct secrets. Share one secret between the nodes that need the same credential.",
    ),
    (
        "DUPLICATE_NODE",
        "Two nodes have the same id. Ids are unique in a document.",
    ),
    (
        "UNKNOWN_ENTRYPOINT",
        "entrypoint must be the id of a node of this document.",
    ),
    (
        "UNKNOWN_TARGET",
        "A node points (next, onError, onTimeout, begin, a branch) at an id that is not a node. Fix the spelling or add the node.",
    ),
    (
        "CYCLE",
        "The graph loops back through the node shown. A workflow has no cycles: express a retry with a retry policy and a repetition with a child workflow.",
    ),
    (
        "UNREACHABLE",
        "No path from the entrypoint reaches this node. Connect it to the graph or remove it.",
    ),
    (
        "INVALID_RETRY",
        "A retry policy has 1 to 100 attempts, an initial delay from 1 ms to 24 h and a maximum delay that is not below the initial one.",
    ),
    (
        "DURATION_OUT_OF_RANGE",
        "A timer or a timeout is from 1 ms up to the maximum shown in the message for its kind.",
    ),
    (
        "INVALID_BRANCH_COUNT",
        "A parallel node has 2 to 256 distinct branches.",
    ),
    (
        "BRANCH_OVERLAP",
        "Two branches of a parallel node run through the same node. Give each branch nodes of its own; they meet at the join.",
    ),
    (
        "BRANCH_BOUNDARY",
        "A node inside a parallel branch is also reached from outside it. A branch is entered only through its parallel node.",
    ),
    (
        "END_INSIDE_BRANCH",
        "An end node cannot sit inside a branch of a parallel node: a branch finishes at the join.",
    ),
    (
        "SAGA_BOUNDARY",
        "A node inside a saga is also reached from outside it. A saga is entered only through its begin node.",
    ),
    (
        "END_INSIDE_SAGA",
        "An end node cannot sit inside a saga: let the saga's next carry on.",
    ),
    (
        "TOO_MANY_SAGAS",
        "A workflow has at most 32 saga nodes. Move some into child workflows.",
    ),
    (
        "IRREVERSIBLE_WITH_COMPENSATION",
        "A node cannot be irreversible and have a compensation too. Remove one of the two.",
    ),
    (
        "EFFECT_ON_NON_ACTIVITY",
        "irreversible, highRisk, the compensation retry and the compensation timeout only apply to an activity node.",
    ),
    (
        "INVALID_TRIGGER",
        "Fix the trigger as the message says; examples of every kind are in docs/runbooks/events.md and in `sagat new event-trigger` and `sagat new cron`.",
    ),
    ("TOO_MANY_TRIGGERS", "A workflow has at most 16 triggers."),
    (
        "DUPLICATE_TRIGGER",
        "The same trigger is declared twice. Remove one.",
    ),
    (
        "COMPENSATION_OUTSIDE_SAGA",
        "compensateWith only runs inside a saga. Put the node in a saga, or remove the compensation.",
    ),
    (
        "RECURSIVE_CHILD",
        "This child node starts the workflow it belongs to. It is legal, and the depth limit ends it at the latest: make sure a condition stops it before.",
    ),
    (
        "INVALID_SCOPE",
        "A structured node has an invalid body, target or boundary. Keep each loop or approval inside its named body and follow the location named by the message.",
    ),
    (
        "DUPLICATE_BODY",
        "Two structured bodies have the same name. Body names are unique in a workflow.",
    ),
    (
        "UNKNOWN_BODY",
        "A structured node refers to a body that the workflow does not declare. Fix the body name or add that body.",
    ),
    (
        "UNREACHABLE_BODY",
        "The workflow declares a body that no structured node uses. Reference it from a node or remove it.",
    ),
    (
        "RECURSIVE_BODY",
        "Structured bodies cannot refer back to themselves through a chain of body references. Break the cycle into a finite structured flow.",
    ),
    (
        "BODY_DEPTH_EXCEEDED",
        "Body references exceed the workflow's maximum nesting depth. Flatten the structured flow or raise maxDepth within the supported limit.",
    ),
    (
        "INVALID_WORKFLOW_LIMIT",
        "Set maxInstances from 1 to 100000 and maxDepth from 1 to 8.",
    ),
    (
        "INVALID_CONTINUATION_POLICY",
        "Set maxSegments from 1 to 10000 and a timeout from 1 millisecond to 366 days; a chain policy also needs a continueAsNew node.",
    ),
    (
        "INVALID_SWITCH",
        "A switch has 1 to 32 uniquely named cases with bounded predicates, and it does not declare next, onError or timeout.",
    ),
    (
        "CLASSIFICATION_INVALID_LEVEL",
        "Use one of the four classification levels: public, internal, pii or secret.",
    ),
    (
        "CLASSIFICATION_LOOSENED",
        "A child cannot be less restrictive than its effective parent. Raise the child level or lower the parent only after reviewing every child.",
    ),
    (
        "CLASSIFICATION_UNSUPPORTED_KEYWORD",
        "Move the classification annotation to a supported schema location such as a carrier root, property, item or local reference target.",
    ),
    (
        "CLASSIFICATION_UNRESOLVED_REF",
        "Use a local JSON Pointer that resolves inside this carrier. Remote and cross-carrier references are not accepted.",
    ),
    (
        "CLASSIFICATION_NOT_APPLICABLE",
        "Classification annotations apply only to inputSchema, outputSchema and supported node outputSchema carriers. Remove the annotation from this location.",
    ),
    (
        "CLASSIFICATION_TOO_COMPLEX",
        "Keep the classification carrier within the fixed schema size, depth and reference-expansion limits.",
    ),
    (
        "CLASSIFICATION_INHERITED_OPEN",
        "A declared child inherits public or internal access. Declare a stricter level when that child should not stay open.",
    ),
];

/// The file that explains each code, one section for each.
pub const DOCS_FILE: &str = "docs/quickstarts/errors.md";

/// What to do about a code, if the tool knows.
#[must_use]
pub fn help(code: &str) -> Option<&'static str> {
    CATALOG.iter().find(|(c, _)| *c == code).map(|(_, h)| *h)
}

/// Where the code is explained: the base is `SAGAT_DOCS_URL` when it is set (the address the
/// documentation is published at), and the path in the repository otherwise.
#[must_use]
pub fn docs_link(code: &str, base: Option<&str>) -> String {
    let base = base
        .map(|b| b.trim_end_matches('/').to_owned())
        .map_or_else(|| DOCS_FILE.to_owned(), |b| format!("{b}/{DOCS_FILE}"));
    format!("{base}#{}", code.to_ascii_lowercase())
}

/// One thing wrong (or worth knowing) about a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub warning: bool,
    pub code: String,
    pub message: String,
    /// The node, or the JSON pointer, it is about.
    pub at: Option<String>,
    pub help: Option<&'static str>,
}

impl Diagnostic {
    /// The lines that tell it: what, where, what to do and where it is explained.
    #[must_use]
    pub fn lines(&self, docs_base: Option<&str>) -> Vec<String> {
        let kind = if self.warning { "warning" } else { "error" };
        let mut lines = vec![format!(
            "{kind}[{}]: {}",
            self.code,
            one_line(&self.message)
        )];
        if let Some(at) = &self.at {
            lines.push(format!("  --> {}", one_line(at)));
        }
        if let Some(help) = self.help {
            lines.push(format!("  help: {help}"));
        }
        lines.push(format!("  docs: {}", docs_link(&self.code, docs_base)));
        lines
    }
}

fn one_line(text: &str) -> String {
    crate::one_line(text)
}

/// The lines for a set of diagnostics: the help and the link are told with the first diagnostic of
/// each code and not repeated for the next ones of the same code (a document with twenty schema
/// problems is told once what a schema problem is).
#[must_use]
pub fn render(diagnostics: &[Diagnostic], docs_base: Option<&str>) -> Vec<String> {
    let mut told: Vec<&str> = Vec::new();
    let mut lines = Vec::new();
    for diagnostic in diagnostics {
        let mut own = diagnostic.lines(docs_base);
        if told.contains(&diagnostic.code.as_str()) {
            own.retain(|line| !line.starts_with("  help:") && !line.starts_with("  docs:"));
        } else {
            told.push(&diagnostic.code);
        }
        lines.extend(own);
    }
    lines
}

/// What the platform's codec says about a document, with the help for each thing it says.
#[must_use]
pub fn of_document(document: &[u8]) -> Vec<Diagnostic> {
    analyze_document(document).0
}

/// Local validation diagnostics and the same bounded classification report used by publication.
#[must_use]
pub fn analyze_document(document: &[u8]) -> (Vec<Diagnostic>, Option<ClassificationAnalysis>) {
    let mut diagnostics: Vec<Diagnostic> = match JsonIrCodec.parse(document) {
        Ok(parsed) => parsed
            .warnings
            .iter()
            .map(|w| {
                let (code, node) = match w {
                    sagat_domain::workflow::DefinitionWarning::CompensationOutsideSaga { node } => {
                        ("COMPENSATION_OUTSIDE_SAGA", node.to_string())
                    }
                    sagat_domain::workflow::DefinitionWarning::RecursiveChild { node } => {
                        ("RECURSIVE_CHILD", node.to_string())
                    }
                };
                Diagnostic {
                    warning: true,
                    code: code.to_owned(),
                    message: w.to_string(),
                    at: Some(format!("node '{node}'")),
                    help: help(code),
                }
            })
            .collect(),
        Err(error) => problems_of(&error)
            .into_iter()
            .map(|p| Diagnostic {
                warning: false,
                code: p.code.to_owned(),
                message: p.message,
                at: p
                    .node
                    .map(|n| format!("node '{n}'"))
                    .or_else(|| p.path.map(|path| format!("at {}", pointer(&path)))),
                help: help(p.code),
            })
            .collect(),
    };
    let analysis = JsonIrCodec.classification_analysis(document);
    if let Some(analysis) = &analysis {
        for finding in &analysis.findings {
            let code = finding.code.as_str();
            let at = format!("at {}", pointer(finding.path.as_str()));
            if diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code && diagnostic.at.as_deref() == Some(&at))
            {
                continue;
            }
            diagnostics.push(Diagnostic {
                warning: finding.code.severity() == FindingSeverity::Warning,
                code: code.to_owned(),
                message: finding.code.message().to_owned(),
                at: Some(at),
                help: help(code),
            });
        }
    }
    (diagnostics, analysis)
}

/// Effective classification levels for every bounded carrier path in an authoring report.
#[must_use]
pub fn classification_report(
    workflow: &str,
    entries: &[ClassificationReportEntry],
    truncated: bool,
) -> Vec<String> {
    if entries.is_empty() && !truncated {
        return Vec::new();
    }
    let mut lines = vec![format!("classification for {}:", one_line(workflow))];
    for entry in entries {
        let carrier = match &entry.carrier {
            CarrierPointer::Input => "inputSchema".to_owned(),
            CarrierPointer::Result => "outputSchema".to_owned(),
            CarrierPointer::NodeOutput(node) => {
                format!("nodeOutputSchema {}", one_line(node.as_str()))
            }
            CarrierPointer::BodyNodeOutput { body, node } => format!(
                "body {} nodeOutputSchema {}",
                one_line(body.as_str()),
                one_line(node.as_str())
            ),
        };
        let path = if entry.path.as_str().is_empty() {
            "/"
        } else {
            entry.path.as_str()
        };
        lines.push(format!(
            "  {carrier} {} {} {}",
            one_line(path),
            entry.level.as_str(),
            if entry.declared {
                "declared"
            } else {
                "inherited"
            }
        ));
    }
    if truncated {
        lines.push("  report truncated at the fixed classification limit".to_owned());
    }
    lines
}

/// A JSON pointer that is empty names the document.
fn pointer(path: &str) -> String {
    if path.is_empty() {
        "the document".to_owned()
    } else {
        path.to_owned()
    }
}

/// A problem the platform's validation returned (`POST /workflows/{id}/validate`).
#[must_use]
pub fn of_server_problem(problem: &serde_json::Value) -> Diagnostic {
    let text = |key: &str| problem.get(key).and_then(serde_json::Value::as_str);
    let code = text("code").unwrap_or("PROBLEM");
    Diagnostic {
        warning: text("severity") == Some("warning"),
        code: code.to_owned(),
        message: text("message").unwrap_or("").to_owned(),
        at: text("nodeId")
            .map(|n| format!("node '{n}'"))
            .or_else(|| text("path").map(|p| format!("at {}", pointer(p)))),
        help: help(code),
    }
}
