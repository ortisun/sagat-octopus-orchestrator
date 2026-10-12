//! Authorized structural payload inspection (ADR-0028).

use std::num::NonZeroU64;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use sagat_application::payloads::{
    ExternalContentKind, InspectionResult, MAX_INSPECTION_RESPONSE_BYTES, PayloadInspection,
    PayloadSelector, PayloadStructure, UnavailablePayloadReason,
};
use sagat_domain::execution::RunId;
use serde::{Deserialize, Serialize};

use crate::admission::admit_resolved_namespace;
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum InspectionSource {
    Input,
    Journal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionSelectorBody {
    source: InspectionSource,
    sequence: Option<NonZeroU64>,
}

impl TryFrom<InspectionSelectorBody> for PayloadSelector {
    type Error = ApiError;

    fn try_from(body: InspectionSelectorBody) -> Result<Self, Self::Error> {
        match (body.source, body.sequence) {
            (InspectionSource::Input, None) => Ok(Self::Input),
            (InspectionSource::Journal, Some(sequence)) => {
                let sequence = sequence.get();
                i64::try_from(sequence)
                    .map(|_| Self::Journal { sequence })
                    .map_err(|_| {
                        ApiError::validation("journal sequence is outside the supported range")
                    })
            }
            (InspectionSource::Input, Some(_)) => Err(ApiError::validation(
                "input selection does not take a sequence",
            )),
            (InspectionSource::Journal, None) => Err(ApiError::validation(
                "journal selection requires a sequence",
            )),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionBody {
    run_id: String,
    run_version: u64,
    selection: SelectionBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attempt: Option<u32>,
    policy: &'static str,
    #[serde(flatten)]
    result: InspectionResultBody,
}

#[derive(Debug, Serialize)]
#[serde(tag = "source", rename_all = "lowercase")]
enum SelectionBody {
    Input,
    Journal { sequence: u64 },
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum InspectionResultBody {
    InlineJson {
        #[serde(rename = "sizeBytes")]
        size_bytes: u64,
        truncated: bool,
        tree: StructureBody,
    },
    External {
        #[serde(rename = "sizeBytes")]
        size_bytes: u64,
        #[serde(rename = "contentKind")]
        content_kind: ContentKindBody,
        reason: &'static str,
    },
    Unavailable {
        reason: &'static str,
        #[serde(rename = "sizeBytes", skip_serializing_if = "Option::is_none")]
        size_bytes: Option<u64>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum ContentKindBody {
    Json,
    Text,
    Binary,
    Other,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum StructureBody {
    Object {
        #[serde(rename = "childCount")]
        child_count: u32,
        children: Vec<Self>,
        #[serde(rename = "omittedChildren")]
        omitted_children: u32,
    },
    Array {
        #[serde(rename = "childCount")]
        child_count: u32,
        children: Vec<Self>,
        #[serde(rename = "omittedChildren")]
        omitted_children: u32,
    },
    String {
        redacted: bool,
    },
    Number {
        redacted: bool,
    },
    Boolean {
        redacted: bool,
    },
    Null {
        redacted: bool,
    },
}

impl From<&PayloadInspection> for InspectionBody {
    fn from(inspection: &PayloadInspection) -> Self {
        let provenance = &inspection.provenance;
        Self {
            run_id: provenance.run_id.to_string(),
            run_version: provenance.run_version,
            selection: match provenance.selection {
                PayloadSelector::Input => SelectionBody::Input,
                PayloadSelector::Journal { sequence } => SelectionBody::Journal { sequence },
            },
            node_id: provenance.node_id.clone(),
            attempt: provenance.attempt,
            policy: sagat_application::payloads::PAYLOAD_INSPECTION_POLICY,
            result: InspectionResultBody::from(&inspection.result),
        }
    }
}

impl From<&InspectionResult> for InspectionResultBody {
    fn from(result: &InspectionResult) -> Self {
        match result {
            InspectionResult::InlineJson {
                size_bytes,
                projection,
            } => Self::InlineJson {
                size_bytes: *size_bytes,
                truncated: projection.truncated,
                tree: StructureBody::from(&projection.tree),
            },
            InspectionResult::External {
                size_bytes,
                content_kind,
            } => Self::External {
                size_bytes: *size_bytes,
                content_kind: match content_kind {
                    ExternalContentKind::Json => ContentKindBody::Json,
                    ExternalContentKind::Text => ContentKindBody::Text,
                    ExternalContentKind::Binary => ContentKindBody::Binary,
                    ExternalContentKind::Other => ContentKindBody::Other,
                },
                reason: "EXTERNAL_CONTENT_NOT_INSPECTED",
            },
            InspectionResult::Unavailable { reason, size_bytes } => Self::Unavailable {
                reason: match reason {
                    UnavailablePayloadReason::NoRetainedPayload => "NO_RETAINED_PAYLOAD",
                    UnavailablePayloadReason::UnsupportedFormat => "UNSUPPORTED_FORMAT",
                },
                size_bytes: *size_bytes,
            },
        }
    }
}

impl From<&PayloadStructure> for StructureBody {
    fn from(structure: &PayloadStructure) -> Self {
        match structure {
            PayloadStructure::Object {
                child_count,
                children,
                omitted_children,
            } => Self::Object {
                child_count: *child_count,
                children: children.iter().map(Self::from).collect(),
                omitted_children: *omitted_children,
            },
            PayloadStructure::Array {
                child_count,
                children,
                omitted_children,
            } => Self::Array {
                child_count: *child_count,
                children: children.iter().map(Self::from).collect(),
                omitted_children: *omitted_children,
            },
            PayloadStructure::String => Self::String { redacted: true },
            PayloadStructure::Number => Self::Number { redacted: true },
            PayloadStructure::Boolean => Self::Boolean { redacted: true },
            PayloadStructure::Null => Self::Null { redacted: true },
        }
    }
}

/// Prepares the tenant-bound run first, charges its canonical namespace once, then performs the
/// audited read and projection. No database transaction is held while admission is charged.
pub async fn inspect_payload<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<InspectionSelectorBody>,
) -> Result<Response, ApiError> {
    let run = id.parse::<RunId>().map_err(|_| ApiError::not_found())?;
    let inspections = state
        .payload_inspections()
        .ok_or_else(ApiError::unavailable)?;
    let selector = PayloadSelector::try_from(body)?;
    let prepared = inspections
        .prepare(&caller.session.principal, run, selector, &caller.ctx)
        .await?;
    admit_resolved_namespace(&state, &caller, prepared.namespace_id()).await?;
    let inspection = inspections.inspect(prepared).await?;
    let bytes = serde_json::to_vec(&InspectionBody::from(&inspection))
        .map_err(|_| ApiError::unavailable())?;
    if bytes.len() > MAX_INSPECTION_RESPONSE_BYTES {
        tracing::error!(
            size_bytes = bytes.len(),
            "payload inspection response exceeded its bound"
        );
        return Err(ApiError::unavailable());
    }
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
    Ok(response)
}
