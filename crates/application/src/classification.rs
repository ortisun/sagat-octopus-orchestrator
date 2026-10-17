//! Stable application boundary for classification adapters.

use std::fmt;

use sagat_domain::classification::{Audience, ClassificationRules, Level, WithheldReason};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::workflow::NodeId;

pub const MAX_CARRIER_BYTES: usize = 64 * 1024;
pub const MAX_SCHEMA_DEPTH: usize = 32;
pub const MAX_REF_EXPANSIONS: usize = 1_000;
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
pub const MAX_PROJECTION_DEPTH: usize = 16;
pub const MAX_PROJECTION_NODES: usize = 1_024;
pub const MAX_CONTAINER_CHILDREN: usize = 128;
pub const MAX_DISCLOSED_STRING_BYTES: usize = 128 * 1024;
pub const MAX_STRING_BYTES: usize = 4 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_CLASSIFICATION_FINDINGS: usize = 20;
pub const MAX_CLASSIFICATION_REPORT_ENTRIES: usize = 500;
pub const MAX_CLASSIFICATION_CACHE_ENTRIES: usize = 256;
pub const MAX_CLASSIFICATION_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// A contract value failed closed validation. It never carries the rejected text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClassificationContractError {
    #[error("invalid JSON number")]
    InvalidNumber,
    #[error("invalid classification pointer")]
    InvalidPointer,
}

/// JSON number spelling preserved exactly, without a lossy float conversion.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct JsonNumber(String);

impl JsonNumber {
    /// Parses the JSON number grammar and retains the original bytes.
    pub fn parse(value: &str) -> Result<Self, ClassificationContractError> {
        let bytes = value.as_bytes();
        let mut at = usize::from(bytes.first() == Some(&b'-'));
        if at >= bytes.len() {
            return Err(ClassificationContractError::InvalidNumber);
        }
        match bytes[at] {
            b'0' => at += 1,
            b'1'..=b'9' => {
                at += 1;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
            _ => return Err(ClassificationContractError::InvalidNumber),
        }
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            let start = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if at == start {
                return Err(ClassificationContractError::InvalidNumber);
            }
        }
        if matches!(bytes.get(at), Some(b'e' | b'E')) {
            at += 1;
            if matches!(bytes.get(at), Some(b'+' | b'-')) {
                at += 1;
            }
            let start = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if at == start {
                return Err(ClassificationContractError::InvalidNumber);
            }
        }
        if at != bytes.len() {
            return Err(ClassificationContractError::InvalidNumber);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for JsonNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JsonNumber(<redacted>)")
    }
}

/// Closed identity of a classification carrier in an immutable workflow document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CarrierPointer {
    Input,
    Result,
    /// Output of a node in the root graph.
    NodeOutput(NodeId),
    /// Output of a body-local node. The application resolves both names from the protected runtime
    /// instance path and pinned typed definition; neither name is accepted as a client pointer.
    /// Iterations share this immutable carrier and therefore are deliberately absent from the key.
    BodyNodeOutput {
        body: NodeId,
        node: NodeId,
    },
}

/// Validator cache identity. Tenant is unnecessary because workflow version ids are globally unique.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClassificationCacheKey {
    pub version: WorkflowVersionId,
    pub carrier: CarrierPointer,
}

/// Grant and request facts retained separately from the effective [`Audience`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PiiRevealFacts {
    pub granted: bool,
    pub requested: bool,
}

impl PiiRevealFacts {
    /// The reason for an inspector when PII is not in the effective audience.
    #[must_use]
    pub const fn withheld_reason(self) -> WithheldReason {
        if self.granted && !self.requested {
            WithheldReason::RevealNotRequested
        } else {
            WithheldReason::LevelNotPermitted
        }
    }
}

/// Runtime classifier input. Its debug representation exposes sizes and policy facts only.
pub struct ProjectionRequest<'a> {
    pub cache_key: ClassificationCacheKey,
    pub rules: ClassificationRules,
    pub document: &'a [u8],
    pub payload: &'a [u8],
    pub audience: Audience,
    pub reveal: PiiRevealFacts,
}

impl fmt::Debug for ProjectionRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionRequest")
            .field("cache_key", &self.cache_key)
            .field("rules", &self.rules)
            .field("document_bytes", &self.document.len())
            .field("payload_bytes", &self.payload.len())
            .field("audience", &self.audience)
            .field("reveal", &self.reveal)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

/// A disclosed scalar. Debug output reveals its kind only.
#[derive(Clone, PartialEq, Eq)]
pub enum ScalarValue {
    String(String),
    Number(JsonNumber),
    Boolean(bool),
    Null,
}

impl fmt::Debug for ScalarValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::String(_) => "String(<redacted>)",
            Self::Number(_) => "Number(<redacted>)",
            Self::Boolean(_) => "Boolean(<redacted>)",
            Self::Null => "Null",
        })
    }
}

/// One returned child. A name is present only for a declared property or a disclosed value.
#[derive(Clone, PartialEq, Eq)]
pub struct ClassifiedChild {
    pub ordinal: u32,
    pub name: Option<String>,
    pub node: Box<ClassifiedNode>,
}

impl fmt::Debug for ClassifiedChild {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClassifiedChild")
            .field("ordinal", &self.ordinal)
            .field("has_name", &self.name.is_some())
            .field("node", &self.node)
            .finish()
    }
}

/// Bounded disclosure tree. Its custom Debug implementation never emits a name or scalar.
#[derive(Clone, PartialEq, Eq)]
pub enum ClassifiedNode {
    Value {
        level: Level,
        kind: ValueKind,
        value: ScalarValue,
        value_truncated: bool,
        original_length: Option<u32>,
    },
    Container {
        level: Level,
        kind: ValueKind,
        child_count: u32,
        omitted_children: u32,
        children: Vec<ClassifiedChild>,
    },
    Withheld {
        level: Level,
        kind: ValueKind,
        reason: WithheldReason,
    },
}

impl fmt::Debug for ClassifiedNode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value {
                level,
                kind,
                value_truncated,
                original_length,
                ..
            } => formatter
                .debug_struct("Value")
                .field("level", level)
                .field("kind", kind)
                .field("value", &"<redacted>")
                .field("value_truncated", value_truncated)
                .field("original_length", original_length)
                .finish(),
            Self::Container {
                level,
                kind,
                child_count,
                omitted_children,
                children,
            } => formatter
                .debug_struct("Container")
                .field("level", level)
                .field("kind", kind)
                .field("child_count", child_count)
                .field("omitted_children", omitted_children)
                .field("children", children)
                .finish(),
            Self::Withheld {
                level,
                kind,
                reason,
            } => formatter
                .debug_struct("Withheld")
                .field("level", level)
                .field("kind", kind)
                .field("reason", reason)
                .finish(),
        }
    }
}

/// Bit set of levels actually disclosed, used to choose the required audit action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisclosedLevels(u8);

impl DisclosedLevels {
    pub const NONE: Self = Self(0);
    pub const PUBLIC: Self = Self(1);
    pub const INTERNAL: Self = Self(2);
    pub const PII: Self = Self(4);

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn contains(self, level: Level) -> bool {
        let bit = match level {
            Level::Public => Self::PUBLIC.0,
            Level::Internal => Self::INTERNAL.0,
            Level::Pii => Self::PII.0,
            Level::Secret => 0,
        };
        bit != 0 && self.0 & bit != 0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClassifiedPayload {
    pub tree: ClassifiedNode,
    pub truncated: bool,
    pub disclosed: DisclosedLevels,
}

impl fmt::Debug for ClassifiedPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClassifiedPayload")
            .field("tree", &self.tree)
            .field("truncated", &self.truncated)
            .field("disclosed", &self.disclosed)
            .finish()
    }
}

/// Closed runtime failures. No parser, validator, schema or payload text crosses the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionFailure {
    #[error("classification rules are not active for this workflow version")]
    LegacyVersion,
    #[error("the selected payload has no classification carrier")]
    NoCarrier,
    #[error("the classification carrier cannot be used")]
    SchemaUnusable,
    #[error("the retained payload is not supported JSON")]
    UnsupportedPayload,
    #[error("the retained payload does not satisfy its classification carrier")]
    SchemaMismatch,
    #[error("the retained payload exceeds the classification bound")]
    PayloadTooLarge,
    #[error("the audience cannot receive the classification carrier root")]
    NotDisclosable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassificationFindingCode {
    InvalidLevel,
    Loosened,
    UnsupportedKeyword,
    UnresolvedRef,
    NotApplicable,
    TooComplex,
    InheritedOpen,
}

impl ClassificationFindingCode {
    pub const ALL: [Self; 7] = [
        Self::InvalidLevel,
        Self::Loosened,
        Self::UnsupportedKeyword,
        Self::UnresolvedRef,
        Self::NotApplicable,
        Self::TooComplex,
        Self::InheritedOpen,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidLevel => "CLASSIFICATION_INVALID_LEVEL",
            Self::Loosened => "CLASSIFICATION_LOOSENED",
            Self::UnsupportedKeyword => "CLASSIFICATION_UNSUPPORTED_KEYWORD",
            Self::UnresolvedRef => "CLASSIFICATION_UNRESOLVED_REF",
            Self::NotApplicable => "CLASSIFICATION_NOT_APPLICABLE",
            Self::TooComplex => "CLASSIFICATION_TOO_COMPLEX",
            Self::InheritedOpen => "CLASSIFICATION_INHERITED_OPEN",
        }
    }

    #[must_use]
    pub const fn severity(self) -> FindingSeverity {
        match self {
            Self::InheritedOpen => FindingSeverity::Warning,
            _ => FindingSeverity::Error,
        }
    }

    /// Closed, value-free message suitable for publication errors and validation reports.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidLevel => "classification level must be public, internal, pii or secret",
            Self::Loosened => "classification cannot be less strict than its effective parent",
            Self::UnsupportedKeyword => {
                "classification appears under an unsupported schema keyword"
            }
            Self::UnresolvedRef => "classification reference must resolve inside its carrier",
            Self::NotApplicable => "classification appears outside a supported carrier",
            Self::TooComplex => "classification carrier exceeds a fixed complexity bound",
            Self::InheritedOpen => "declared location inherits an open classification level",
        }
    }
}

/// Bounded JSON pointer. Publication paths may name schema properties but never payload data.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ClassificationPointer(String);

impl ClassificationPointer {
    pub fn parse(value: impl Into<String>) -> Result<Self, ClassificationContractError> {
        let value = value.into();
        if value.len() > 2_048
            || (!value.is_empty() && !value.starts_with('/'))
            || value.chars().any(char::is_control)
        {
            return Err(ClassificationContractError::InvalidPointer);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ClassificationPointer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ClassificationPointer")
            .field(&self.0)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationFinding {
    pub code: ClassificationFindingCode,
    pub path: ClassificationPointer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationReportEntry {
    pub carrier: CarrierPointer,
    pub path: ClassificationPointer,
    pub level: Level,
    pub declared: bool,
}

/// One semantic step within a carrier. Variants keep literal property names distinct from JSON
/// Schema applicators that govern otherwise unmatched object properties and array items.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClassificationSegment {
    Property(String),
    Index(usize),
    AdditionalProperty,
    RemainingItem,
}

/// One effective classification location emitted by the bounded authoring walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationLocation {
    pub carrier: CarrierPointer,
    pub location: Vec<ClassificationSegment>,
    /// Exact JSON pointer to the schema location which governs this effective location.
    pub source_path: String,
    pub level: Level,
    pub declared: bool,
}

/// Why a carrier could not be walked completely. The path and message code are safe to expose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationCarrierFailure {
    pub carrier: CarrierPointer,
    pub code: ClassificationFindingCode,
    pub source_path: String,
}

/// Per-carrier outcome of a classification location walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationCarrierStatus {
    Complete(CarrierPointer),
    Failed(ClassificationCarrierFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassificationWalkStatus {
    pub carriers: Vec<ClassificationCarrierStatus>,
}

impl ClassificationWalkStatus {
    #[must_use]
    pub fn failure(&self, carrier: &CarrierPointer) -> Option<&ClassificationCarrierFailure> {
        self.carriers.iter().find_map(|status| match status {
            ClassificationCarrierStatus::Failed(failure) if &failure.carrier == carrier => {
                Some(failure)
            }
            ClassificationCarrierStatus::Complete(_) | ClassificationCarrierStatus::Failed(_) => {
                None
            }
        })
    }
}

/// Effective classification lookup used by authoring comparison without retaining a whole report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationResolution {
    AbsentCarrier,
    Incomplete(ClassificationCarrierFailure),
    Resolved {
        level: Level,
        source_path: String,
        /// True when this exact semantic location is present in the carrier walk. False means its
        /// value came from an applicable fallback or the conservative PII floor.
        location_present: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassificationAnalysis {
    pub findings: Vec<ClassificationFinding>,
    pub report: Vec<ClassificationReportEntry>,
    pub findings_truncated: bool,
    pub report_truncated: bool,
    /// Set independently of the bounded finding list so truncation can never allow publication.
    pub has_errors: bool,
}

impl ClassificationAnalysis {
    #[must_use]
    pub fn is_publishable(&self) -> bool {
        !self.has_errors
            && !self
                .findings
                .iter()
                .any(|finding| finding.code.severity() == FindingSeverity::Error)
    }
}

/// Synchronous CPU-only boundary implemented by the schema adapter.
pub trait ClassificationInterpreter: Send + Sync {
    fn analyze(&self, document: &[u8]) -> ClassificationAnalysis;

    fn project(
        &self,
        request: ProjectionRequest<'_>,
    ) -> Result<ClassifiedPayload, ProjectionFailure>;
}

#[cfg(test)]
mod tests {
    use sagat_domain::classification::{Audience, ClassificationRules, Level};
    use sagat_domain::definition::WorkflowVersionId;
    use sagat_domain::workflow::NodeId;

    use super::*;

    #[test]
    fn exact_json_number_text_is_kept_without_a_float_conversion() {
        let number = JsonNumber::parse("-12345678901234567890.001e+42").expect("JSON number");
        assert_eq!(number.as_str(), "-12345678901234567890.001e+42");

        for invalid in ["", "+1", "01", "1.", ".1", "NaN", "1e"] {
            assert!(JsonNumber::parse(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn payload_carrying_debug_output_never_contains_values_or_names() {
        let request = ProjectionRequest {
            cache_key: ClassificationCacheKey {
                version: WorkflowVersionId::generate(),
                carrier: CarrierPointer::NodeOutput(
                    NodeId::parse("charge").expect("valid node id"),
                ),
            },
            rules: ClassificationRules::CURRENT,
            document: br#"{"schema":"schema-canary"}"#,
            payload: br#"{"customer-secret-key":"value-canary"}"#,
            audience: Audience::new(true, false, true),
            reveal: PiiRevealFacts {
                granted: true,
                requested: false,
            },
        };
        let projected = ClassifiedPayload {
            tree: ClassifiedNode::Container {
                level: Level::Public,
                kind: ValueKind::Object,
                child_count: 1,
                omitted_children: 0,
                children: vec![ClassifiedChild {
                    ordinal: 0,
                    name: Some("customer-secret-key".into()),
                    node: Box::new(ClassifiedNode::Value {
                        level: Level::Public,
                        kind: ValueKind::String,
                        value: ScalarValue::String("value-canary".into()),
                        value_truncated: false,
                        original_length: None,
                    }),
                }],
            },
            truncated: false,
            disclosed: DisclosedLevels::PUBLIC,
        };

        let debug = format!("{request:?} {projected:?}");
        for forbidden in [
            "schema-canary",
            "customer-secret-key",
            "value-canary",
            "12345678901234567890",
        ] {
            assert!(
                !debug.contains(forbidden),
                "debug leaked {forbidden:?}: {debug}"
            );
        }
        assert!(debug.contains("document_bytes") && debug.contains("payload_bytes"));
    }

    #[test]
    fn finding_codes_have_closed_safe_messages_and_severities() {
        for code in ClassificationFindingCode::ALL {
            assert!(!code.message().is_empty());
            assert!(!code.message().contains('{'));
            assert_eq!(
                code.severity(),
                if code == ClassificationFindingCode::InheritedOpen {
                    FindingSeverity::Warning
                } else {
                    FindingSeverity::Error
                }
            );
        }
    }

    #[test]
    fn reveal_facts_distinguish_an_omitted_request_from_a_missing_grant() {
        assert_eq!(
            PiiRevealFacts {
                granted: true,
                requested: false,
            }
            .withheld_reason(),
            sagat_domain::classification::WithheldReason::RevealNotRequested
        );
        assert_eq!(
            PiiRevealFacts {
                granted: false,
                requested: false,
            }
            .withheld_reason(),
            sagat_domain::classification::WithheldReason::LevelNotPermitted
        );
    }

    #[test]
    fn an_omitted_root_has_an_explicit_value_free_result() {
        assert_eq!(
            ProjectionFailure::NotDisclosable.to_string(),
            "the audience cannot receive the classification carrier root"
        );
    }

    #[test]
    fn root_and_body_node_carriers_have_distinct_cache_identities() {
        use std::collections::HashSet;

        let node = NodeId::parse("shared").expect("node");
        let carriers = HashSet::from([
            CarrierPointer::NodeOutput(node.clone()),
            CarrierPointer::BodyNodeOutput {
                body: NodeId::parse("first").expect("body"),
                node: node.clone(),
            },
            CarrierPointer::BodyNodeOutput {
                body: NodeId::parse("second").expect("body"),
                node,
            },
        ]);
        assert_eq!(carriers.len(), 3);
    }

    #[test]
    fn authoring_location_identity_keeps_literals_separate_from_applicators() {
        let literal_star = vec![ClassificationSegment::Property("*".into())];
        let literal_zero = vec![ClassificationSegment::Property("0".into())];

        assert_ne!(
            literal_star,
            vec![ClassificationSegment::AdditionalProperty]
        );
        assert_ne!(literal_zero, vec![ClassificationSegment::Index(0)]);
        assert_ne!(
            vec![ClassificationSegment::AdditionalProperty],
            vec![ClassificationSegment::RemainingItem]
        );
    }
}
