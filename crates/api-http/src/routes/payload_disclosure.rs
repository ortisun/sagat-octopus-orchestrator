//! HTTP representation of classified run payload disclosure (FR-016-006..009).

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sagat_application::classification::{
    ClassifiedNode, JsonNumber, MAX_RESPONSE_BYTES, ScalarValue, ValueKind,
};
use sagat_application::error::AppError;
use sagat_application::payload_disclosure::{
    DisclosureSchema, DisclosureSchemaCarrier, DisclosureSelector, PayloadDisclosure,
    PayloadDisclosureError, PayloadDisclosureResult, StructureOnlyReason,
    UnavailableDisclosureReason,
};
use sagat_application::payloads::{ExternalContentKind, PayloadStructure};
use sagat_domain::classification::{Level, WithheldReason};
use sagat_domain::execution::RunId;
use sagat_domain::identity::Permission;
use serde::Deserialize;

use crate::admission::admit_resolved_namespace;
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};

/// Hard request envelope ceiling from FR-016-024. The router applies this before JSON extraction.
pub const DISCLOSURE_MAX_BODY_BYTES: usize = 1024;

#[derive(Debug, Deserialize)]
pub struct DisclosureRequestBody {
    source: String,
    sequence: Option<serde_json::Value>,
    reveal: Option<Vec<String>>,
    #[serde(flatten)]
    unexpected: BTreeMap<String, serde::de::IgnoredAny>,
}

impl DisclosureRequestBody {
    fn parse(self) -> Result<(DisclosureSelector, bool), ApiError> {
        if !self.unexpected.is_empty() {
            return Err(ApiError::bad_request(
                "request body contains an unknown field",
            ));
        }
        let sequence = self.sequence.map(|value| {
            value
                .as_u64()
                .and_then(NonZeroU64::new)
                .filter(|sequence| i64::try_from(sequence.get()).is_ok())
                .ok_or_else(|| {
                    ApiError::validation(
                        "journal sequence must be a positive signed 64-bit integer",
                    )
                })
        });
        let selector = match (self.source.as_str(), sequence) {
            ("input", None) => DisclosureSelector::Input,
            ("result", None) => DisclosureSelector::Result,
            ("journal", Some(Ok(sequence))) => DisclosureSelector::Journal {
                sequence: sequence.get(),
            },
            ("input" | "result", Some(_)) => {
                return Err(ApiError::validation(
                    "input and result selections do not take a sequence",
                ));
            }
            ("journal", None) => {
                return Err(ApiError::validation(
                    "journal selection requires a sequence",
                ));
            }
            ("journal", Some(Err(error))) => return Err(error),
            _ => {
                return Err(ApiError::validation(
                    "source must be input, result or journal",
                ));
            }
        };
        let reveal_pii = match self.reveal.as_deref() {
            None | Some([]) => false,
            Some([level]) if level == "pii" => true,
            Some(_) => {
                return Err(ApiError::validation(
                    "reveal may contain only the single value pii",
                ));
            }
        };
        Ok((selector, reveal_pii))
    }
}

/// Resolves the trusted run, charges its namespace once, then performs the transactional disclosure.
pub async fn disclose_payload<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<DisclosureRequestBody>,
) -> Result<Response, ApiError> {
    let run = id.parse::<RunId>().map_err(|_| ApiError::not_found())?;
    let disclosures = state
        .payload_disclosures()
        .ok_or_else(ApiError::unavailable)?;
    let (selector, reveal_pii) = body.parse()?;
    let prepared = disclosures
        .prepare(
            &caller.session.principal,
            run,
            selector,
            reveal_pii,
            &caller.ctx,
        )
        .await?;
    admit_resolved_namespace(&state, &caller, prepared.namespace_id()).await?;
    let disclosure = disclosures
        .disclose(prepared)
        .await
        .map_err(disclosure_error)?;
    let bytes = encode_disclosure(&disclosure)?;
    Ok(disclosure_response(bytes))
}

/// Keep every response private, including body-limit rejections produced before the handler.
pub async fn no_store(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let mut response = if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::from(AppError::PayloadTooLarge("payload disclosure request")).into_response()
    } else {
        response
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

fn disclosure_response(bytes: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

fn disclosure_error(error: PayloadDisclosureError) -> ApiError {
    match error {
        PayloadDisclosureError::Application(error) => error.into(),
        PayloadDisclosureError::PiiRevealNotGranted => ApiError::from(AppError::Forbidden {
            permission: Permission::PayloadRevealPii,
        })
        .with_reason("PII_REVEAL_NOT_GRANTED"),
    }
}

fn encode_disclosure(disclosure: &PayloadDisclosure) -> Result<Vec<u8>, ApiError> {
    let mut out = Vec::new();
    out.push(b'{');
    field_name(&mut out, "runId", true)?;
    string(&mut out, &disclosure.run_id.to_string())?;
    field_name(&mut out, "runVersion", false)?;
    integer(&mut out, disclosure.run_version);
    field_name(&mut out, "selection", false)?;
    selection(&mut out, disclosure.selection)?;
    if let Some(node_id) = &disclosure.node_id {
        field_name(&mut out, "nodeId", false)?;
        string(&mut out, node_id)?;
    }
    if let Some(attempt) = disclosure.attempt {
        field_name(&mut out, "attempt", false)?;
        integer(&mut out, u64::from(attempt));
    }
    field_name(&mut out, "policy", false)?;
    string(
        &mut out,
        sagat_application::payload_disclosure::PAYLOAD_DISCLOSURE_POLICY,
    )?;
    field_name(&mut out, "caller", false)?;
    caller(&mut out, disclosure)?;
    result(&mut out, &disclosure.result)?;
    out.push(b'}');
    if out.len() > MAX_RESPONSE_BYTES {
        tracing::error!(
            size_bytes = out.len(),
            "payload disclosure response exceeded its bound"
        );
        return Err(ApiError::unavailable());
    }
    Ok(out)
}

fn selection(out: &mut Vec<u8>, selector: DisclosureSelector) -> Result<(), ApiError> {
    out.push(b'{');
    field_name(out, "source", true)?;
    string(out, selector.source())?;
    if let DisclosureSelector::Journal { sequence } = selector {
        field_name(out, "sequence", false)?;
        integer(out, sequence);
    }
    out.push(b'}');
    Ok(())
}

fn caller(out: &mut Vec<u8>, disclosure: &PayloadDisclosure) -> Result<(), ApiError> {
    out.extend_from_slice(br#"{"disclosedLevels":["#);
    let mut first = true;
    for (level, name) in [
        (Level::Public, "public"),
        (Level::Internal, "internal"),
        (Level::Pii, "pii"),
    ] {
        if disclosure.caller.disclosed_levels.contains(level) {
            if !first {
                out.push(b',');
            }
            string(out, name)?;
            first = false;
        }
    }
    out.extend_from_slice(br#"],"revealPii":"#);
    string(
        out,
        if disclosure.caller.reveal_granted {
            "granted"
        } else {
            "not_granted"
        },
    )?;
    out.extend_from_slice(br#","revealed":"#);
    boolean(out, disclosure.caller.revealed);
    out.push(b'}');
    Ok(())
}

fn result(out: &mut Vec<u8>, result: &PayloadDisclosureResult) -> Result<(), ApiError> {
    match result {
        PayloadDisclosureResult::Disclosed {
            schema,
            size_bytes,
            projection,
        } => {
            field_name(out, "kind", false)?;
            string(out, "disclosed")?;
            field_name(out, "schema", false)?;
            schema_body(out, schema)?;
            if let Some(size_bytes) = size_bytes {
                field_name(out, "sizeBytes", false)?;
                integer(out, *size_bytes);
            }
            field_name(out, "truncated", false)?;
            boolean(out, projection.truncated);
            field_name(out, "tree", false)?;
            classified_node(out, &projection.tree)?;
        }
        PayloadDisclosureResult::StructureOnly {
            schema,
            size_bytes,
            reason,
            projection,
        } => {
            field_name(out, "kind", false)?;
            string(out, "structure-only")?;
            if let Some(schema) = schema {
                field_name(out, "schema", false)?;
                schema_body(out, schema)?;
            }
            field_name(out, "sizeBytes", false)?;
            integer(out, *size_bytes);
            field_name(out, "truncated", false)?;
            boolean(out, projection.truncated);
            field_name(out, "reason", false)?;
            string(
                out,
                match reason {
                    StructureOnlyReason::NoSchema => "NO_SCHEMA",
                    StructureOnlyReason::SchemaMismatch => "SCHEMA_MISMATCH",
                    StructureOnlyReason::LegacyVersion => "LEGACY_VERSION",
                    StructureOnlyReason::SchemaUnusable => "SCHEMA_UNUSABLE",
                },
            )?;
            field_name(out, "tree", false)?;
            structure(out, &projection.tree)?;
        }
        PayloadDisclosureResult::External {
            size_bytes,
            content_kind,
        } => {
            field_name(out, "kind", false)?;
            string(out, "external")?;
            field_name(out, "sizeBytes", false)?;
            integer(out, *size_bytes);
            field_name(out, "contentKind", false)?;
            string(
                out,
                match content_kind {
                    ExternalContentKind::Json => "json",
                    ExternalContentKind::Text => "text",
                    ExternalContentKind::Binary => "binary",
                    ExternalContentKind::Other => "other",
                },
            )?;
            field_name(out, "reason", false)?;
            string(out, "EXTERNAL_CONTENT_NOT_INSPECTED")?;
        }
        PayloadDisclosureResult::Unavailable { reason, size_bytes } => {
            field_name(out, "kind", false)?;
            string(out, "unavailable")?;
            field_name(out, "reason", false)?;
            string(
                out,
                match reason {
                    UnavailableDisclosureReason::NoRetainedPayload => "NO_RETAINED_PAYLOAD",
                    UnavailableDisclosureReason::UnsupportedFormat => "UNSUPPORTED_FORMAT",
                    UnavailableDisclosureReason::NotDisclosable => "NOT_DISCLOSABLE",
                },
            )?;
            if let Some(size_bytes) = size_bytes {
                field_name(out, "sizeBytes", false)?;
                integer(out, *size_bytes);
            }
        }
    }
    Ok(())
}

fn schema_body(out: &mut Vec<u8>, schema: &DisclosureSchema) -> Result<(), ApiError> {
    out.push(b'{');
    field_name(out, "carrier", true)?;
    string(
        out,
        match schema.carrier {
            DisclosureSchemaCarrier::InputSchema => "inputSchema",
            DisclosureSchemaCarrier::OutputSchema => "outputSchema",
            DisclosureSchemaCarrier::NodeOutputSchema => "nodeOutputSchema",
        },
    )?;
    field_name(out, "versionNumber", false)?;
    integer(out, u64::from(schema.version_number));
    field_name(out, "rules", false)?;
    integer(out, u64::from(schema.rules.as_u8()));
    out.push(b'}');
    Ok(())
}

fn classified_node(out: &mut Vec<u8>, node: &ClassifiedNode) -> Result<(), ApiError> {
    out.push(b'{');
    match node {
        ClassifiedNode::Value {
            level,
            kind,
            value,
            value_truncated,
            original_length,
        } => {
            field_name(out, "kind", true)?;
            string(out, "value")?;
            node_type_and_level(out, *kind, *level)?;
            field_name(out, "value", false)?;
            scalar(out, value)?;
            if *value_truncated {
                field_name(out, "valueTruncated", false)?;
                boolean(out, true);
                if let Some(length) = original_length {
                    field_name(out, "length", false)?;
                    integer(out, u64::from(*length));
                }
            }
        }
        ClassifiedNode::Container {
            level,
            kind,
            child_count,
            omitted_children,
            children,
        } => {
            field_name(out, "kind", true)?;
            string(out, "container")?;
            node_type_and_level(out, *kind, *level)?;
            field_name(out, "childCount", false)?;
            integer(out, u64::from(*child_count));
            field_name(out, "omittedChildren", false)?;
            integer(out, u64::from(*omitted_children));
            field_name(out, "children", false)?;
            out.push(b'[');
            for (index, child) in children.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                out.push(b'{');
                field_name(out, "ordinal", true)?;
                integer(out, u64::from(child.ordinal));
                if let Some(name) = &child.name {
                    field_name(out, "name", false)?;
                    string(out, name)?;
                }
                field_name(out, "node", false)?;
                classified_node(out, &child.node)?;
                out.push(b'}');
            }
            out.push(b']');
        }
        ClassifiedNode::Withheld {
            level,
            kind,
            reason,
        } => {
            field_name(out, "kind", true)?;
            string(out, "withheld")?;
            node_type_and_level(out, *kind, *level)?;
            field_name(out, "reason", false)?;
            string(
                out,
                match reason {
                    WithheldReason::LevelNotPermitted => "LEVEL_NOT_PERMITTED",
                    WithheldReason::RevealNotRequested => "REVEAL_NOT_REQUESTED",
                    WithheldReason::Secret => "SECRET",
                },
            )?;
        }
    }
    out.push(b'}');
    Ok(())
}

fn node_type_and_level(out: &mut Vec<u8>, kind: ValueKind, level: Level) -> Result<(), ApiError> {
    field_name(out, "type", false)?;
    string(out, value_kind(kind))?;
    field_name(out, "level", false)?;
    string(out, level.as_str())
}

fn scalar(out: &mut Vec<u8>, value: &ScalarValue) -> Result<(), ApiError> {
    match value {
        ScalarValue::String(value) => string(out, value),
        ScalarValue::Number(value) => {
            exact_number(out, value);
            Ok(())
        }
        ScalarValue::Boolean(value) => {
            boolean(out, *value);
            Ok(())
        }
        ScalarValue::Null => {
            out.extend_from_slice(b"null");
            Ok(())
        }
    }
}

fn structure(out: &mut Vec<u8>, node: &PayloadStructure) -> Result<(), ApiError> {
    out.push(b'{');
    field_name(out, "type", true)?;
    match node {
        PayloadStructure::Object {
            child_count,
            children,
            omitted_children,
        }
        | PayloadStructure::Array {
            child_count,
            children,
            omitted_children,
        } => {
            string(
                out,
                if matches!(node, PayloadStructure::Object { .. }) {
                    "object"
                } else {
                    "array"
                },
            )?;
            field_name(out, "childCount", false)?;
            integer(out, u64::from(*child_count));
            field_name(out, "children", false)?;
            out.push(b'[');
            for (index, child) in children.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                structure(out, child)?;
            }
            out.push(b']');
            field_name(out, "omittedChildren", false)?;
            integer(out, u64::from(*omitted_children));
        }
        PayloadStructure::String
        | PayloadStructure::Number
        | PayloadStructure::Boolean
        | PayloadStructure::Null => {
            string(out, structure_kind(node))?;
            field_name(out, "redacted", false)?;
            boolean(out, true);
        }
    }
    out.push(b'}');
    Ok(())
}

const fn value_kind(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Object => "object",
        ValueKind::Array => "array",
        ValueKind::String => "string",
        ValueKind::Number => "number",
        ValueKind::Boolean => "boolean",
        ValueKind::Null => "null",
    }
}

const fn structure_kind(node: &PayloadStructure) -> &'static str {
    match node {
        PayloadStructure::Object { .. } => "object",
        PayloadStructure::Array { .. } => "array",
        PayloadStructure::String => "string",
        PayloadStructure::Number => "number",
        PayloadStructure::Boolean => "boolean",
        PayloadStructure::Null => "null",
    }
}

fn field_name(out: &mut Vec<u8>, name: &'static str, first: bool) -> Result<(), ApiError> {
    if !first {
        out.push(b',');
    }
    string(out, name)?;
    out.push(b':');
    Ok(())
}

fn string(out: &mut Vec<u8>, value: &str) -> Result<(), ApiError> {
    serde_json::to_writer(out, value).map_err(|_| ApiError::unavailable())
}

fn integer(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(value.to_string().as_bytes());
}

fn exact_number(out: &mut Vec<u8>, value: &JsonNumber) {
    out.extend_from_slice(value.as_str().as_bytes());
}

fn boolean(out: &mut Vec<u8>, value: bool) {
    out.extend_from_slice(if value { b"true" } else { b"false" });
}

#[cfg(test)]
mod tests {
    use sagat_application::classification::{ClassifiedPayload, DisclosedLevels, ScalarValue};
    use sagat_application::payload_disclosure::{DisclosureCaller, DisclosureSchema};
    use sagat_domain::classification::ClassificationRules;

    use super::*;

    fn scalar_disclosure(value: String) -> PayloadDisclosure {
        PayloadDisclosure {
            run_id: RunId::generate(),
            run_version: 1,
            selection: DisclosureSelector::Input,
            node_id: None,
            attempt: None,
            caller: DisclosureCaller {
                disclosed_levels: DisclosedLevels::PUBLIC,
                reveal_granted: false,
                revealed: false,
            },
            result: PayloadDisclosureResult::Disclosed {
                schema: DisclosureSchema {
                    carrier: DisclosureSchemaCarrier::InputSchema,
                    version_number: 1,
                    rules: ClassificationRules::CURRENT,
                },
                size_bytes: Some(u64::try_from(value.len()).expect("test size")),
                projection: ClassifiedPayload {
                    tree: ClassifiedNode::Value {
                        level: Level::Public,
                        kind: ValueKind::String,
                        value: ScalarValue::String(value),
                        value_truncated: false,
                        original_length: None,
                    },
                    truncated: false,
                    disclosed: DisclosedLevels::PUBLIC,
                },
            },
        }
    }

    #[test]
    fn exact_numbers_and_escaped_names_are_encoded_without_rounding() {
        let number = "-12345678901234567890.001e+400";
        let disclosure = PayloadDisclosure {
            run_id: RunId::generate(),
            run_version: 7,
            selection: DisclosureSelector::Input,
            node_id: None,
            attempt: None,
            caller: DisclosureCaller {
                disclosed_levels: DisclosedLevels::PUBLIC,
                reveal_granted: false,
                revealed: false,
            },
            result: PayloadDisclosureResult::Disclosed {
                schema: DisclosureSchema {
                    carrier: DisclosureSchemaCarrier::InputSchema,
                    version_number: 3,
                    rules: ClassificationRules::CURRENT,
                },
                size_bytes: Some(32),
                projection: ClassifiedPayload {
                    tree: ClassifiedNode::Container {
                        level: Level::Public,
                        kind: ValueKind::Object,
                        child_count: 1,
                        omitted_children: 0,
                        children: vec![sagat_application::classification::ClassifiedChild {
                            ordinal: 0,
                            name: Some("quote\"line\n".into()),
                            node: Box::new(ClassifiedNode::Value {
                                level: Level::Public,
                                kind: ValueKind::Number,
                                value: ScalarValue::Number(
                                    JsonNumber::parse(number).expect("number"),
                                ),
                                value_truncated: false,
                                original_length: None,
                            }),
                        }],
                    },
                    truncated: false,
                    disclosed: DisclosedLevels::PUBLIC,
                },
            },
        };

        let bytes = encode_disclosure(&disclosure).expect("encoded");
        let text = String::from_utf8(bytes).expect("UTF-8 JSON");
        assert!(text.contains(number));
        assert!(text.contains(r#""name":"quote\"line\n""#));
        let parsed: serde_json::Value =
            serde_json::from_str(&text.replace(number, "0")).expect("valid JSON shape");
        assert_eq!(parsed["kind"], "disclosed");
    }

    #[test]
    fn the_stored_size_is_in_the_answer_only_when_the_use_case_gave_it() {
        let mut disclosure = scalar_disclosure("x".into());
        let body = |disclosure: &PayloadDisclosure| -> serde_json::Value {
            serde_json::from_slice(&encode_disclosure(disclosure).expect("envelope"))
                .expect("JSON envelope")
        };
        assert_eq!(body(&disclosure)["sizeBytes"], 1);
        if let PayloadDisclosureResult::Disclosed { size_bytes, .. } = &mut disclosure.result {
            *size_bytes = None;
        }
        let without = body(&disclosure);
        assert_eq!(without["kind"], "disclosed");
        assert!(
            without.get("sizeBytes").is_none(),
            "a caller who may not inspect must not learn the size of the stored payload"
        );
    }

    #[test]
    fn request_validation_keeps_selectors_closed() {
        let parsed = DisclosureRequestBody {
            source: "journal".into(),
            unexpected: BTreeMap::new(),
            sequence: Some(serde_json::json!(42)),
            reveal: Some(vec!["pii".into()]),
        }
        .parse()
        .expect("valid selector");
        assert_eq!(parsed, (DisclosureSelector::Journal { sequence: 42 }, true));

        for body in [
            DisclosureRequestBody {
                source: "journal".into(),
                unexpected: BTreeMap::new(),
                sequence: Some(serde_json::json!(0)),
                reveal: None,
            },
            DisclosureRequestBody {
                source: "input".into(),
                unexpected: BTreeMap::new(),
                sequence: Some(serde_json::json!(1)),
                reveal: None,
            },
            DisclosureRequestBody {
                source: "input".into(),
                unexpected: BTreeMap::new(),
                sequence: None,
                reveal: Some(vec!["secret".into()]),
            },
        ] {
            assert!(body.parse().is_err());
        }
    }

    #[test]
    fn unknown_fields_are_bad_requests_without_echoing_their_names() {
        use axum::response::IntoResponse;
        let body: DisclosureRequestBody =
            serde_json::from_str(r#"{"source":"input","ERROR-REFLECTION-CANARY":"ignored"}"#)
                .expect("syntactically valid request");
        assert_eq!(
            body.parse()
                .expect_err("closed fields")
                .into_response()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(
            serde_json::from_str::<DisclosureRequestBody>(
                r#"{"source":"input","bad":"ERROR-REFLECTION-CANARY"#,
            )
            .expect_err("truncated string")
            .is_eof()
        );
    }

    #[test]
    fn final_envelope_is_bounded_and_successes_are_private_no_store() {
        // Use a six-digit probe so `sizeBytes` has the same width as the final bounded value.
        let probe_len = 100_000;
        let probe =
            encode_disclosure(&scalar_disclosure("x".repeat(probe_len))).expect("probe envelope");
        let envelope_bytes = probe.len() - probe_len;
        let fill = MAX_RESPONSE_BYTES
            .checked_sub(envelope_bytes)
            .expect("envelope fits the response ceiling");
        let exact = encode_disclosure(&scalar_disclosure("x".repeat(fill))).expect("exact ceiling");
        assert_eq!(exact.len(), MAX_RESPONSE_BYTES);
        assert!(encode_disclosure(&scalar_disclosure("x".repeat(fill + 1))).is_err());

        let response = disclosure_response(exact);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("private, no-store"))
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json"))
        );
    }
}
