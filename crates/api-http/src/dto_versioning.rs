//! The shapes of the versioning routes: approval policies, requests to publish, the active
//! pointer and its history, and promotion (FR-011-002..005).

use sagat_application::publishing::{ActivationRecord, PolicySource, PolicyView, Promotion};
use sagat_domain::versioning::PublicationRequest;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::dto_exec::{PublishedBody, VersionSummary};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyBody {
    pub namespace_id: String,
    pub environment: &'static str,
    pub required_approvals: u8,
    /// `default` (what the environment asks for until someone sets it) or `set`.
    pub source: &'static str,
    pub set_by: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub set_at: Option<OffsetDateTime>,
}

impl From<&PolicyView> for PolicyBody {
    fn from(view: &PolicyView) -> Self {
        let (source, set_by, set_at) = match &view.source {
            PolicySource::Default(_) => ("default", None, None),
            PolicySource::Set { by, at } => ("set", Some(by.clone()), Some(*at)),
        };
        Self {
            namespace_id: view.namespace_id.to_string(),
            environment: view.environment.as_str(),
            required_approvals: view.policy.required(),
            source,
            set_by,
            set_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetPolicyBody {
    pub required_approvals: u8,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenRequestBody {
    pub draft_revision: u64,
    #[serde(default)]
    pub release_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionBody {
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionOut {
    pub by: String,
    pub verdict: &'static str,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedBy {
    pub version_id: String,
    pub by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestBody {
    pub id: String,
    pub workflow_id: String,
    pub namespace_id: String,
    pub kind: &'static str,
    pub state: &'static str,
    pub draft_revision: u64,
    pub digest: String,
    pub release_notes: Option<String>,
    /// For a promotion: the version whose document this publishes.
    pub promoted_from_version_id: Option<String>,
    pub requested_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub requested_at: OffsetDateTime,
    /// What the policy asked for when the request was opened.
    pub required_approvals: u8,
    pub approvals: u8,
    pub decisions: Vec<DecisionOut>,
    pub published: Option<PublishedBy>,
}

impl From<&PublicationRequest> for RequestBody {
    fn from(request: &PublicationRequest) -> Self {
        Self {
            id: request.id().to_string(),
            workflow_id: request.workflow_id().to_string(),
            namespace_id: request.namespace_id().to_string(),
            kind: request.kind().as_str(),
            state: request.state().as_str(),
            draft_revision: request.draft_revision(),
            digest: request.digest().to_hex(),
            release_notes: request.release_notes().map(str::to_owned),
            promoted_from_version_id: request.promoted_from().map(|v| v.to_string()),
            requested_by: request.requested_by().to_string(),
            requested_at: request.requested_at(),
            required_approvals: request.required(),
            approvals: request.approvals(),
            decisions: request
                .decisions()
                .iter()
                .map(|d| DecisionOut {
                    by: d.by.to_string(),
                    verdict: d.verdict.as_str(),
                    reason: d.reason.clone(),
                    at: d.at,
                })
                .collect(),
            published: request.published().map(|(version, by, at)| PublishedBy {
                version_id: version.to_string(),
                by: by.to_string(),
                at: *at,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivateBody {
    pub version: u32,
    pub reason: String,
    /// The version the caller saw active; the change is refused if it is not any more.
    #[serde(default)]
    pub expected_version: Option<u32>,
    /// The caller accepts that callers and emitters of the workflow may fail.
    #[serde(default)]
    pub accept_breaking: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotedFromBody {
    pub namespace_id: String,
    pub workflow_id: String,
    pub version: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationBody {
    pub id: String,
    pub workflow_id: String,
    pub version: u32,
    pub version_id: String,
    pub previous_version: Option<u32>,
    pub kind: &'static str,
    pub reason: Option<String>,
    pub actor: String,
    pub request_id: Option<String>,
    pub promoted_from: Option<PromotedFromBody>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

impl From<&ActivationRecord> for ActivationBody {
    fn from(a: &ActivationRecord) -> Self {
        Self {
            id: a.id.to_string(),
            workflow_id: a.workflow_id.to_string(),
            version: a.version,
            version_id: a.version_id.to_string(),
            previous_version: a.previous,
            kind: a.kind.as_str(),
            reason: a.reason.clone(),
            actor: a.actor.clone(),
            request_id: a.request.map(|r| r.to_string()),
            promoted_from: a.promoted_from.as_ref().map(|p| PromotedFromBody {
                namespace_id: p.namespace_id.to_string(),
                workflow_id: p.workflow_id.to_string(),
                version: p.version,
            }),
            at: a.at,
        }
    }
}

/// What moving the pointer did: the change, and whether anything changed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationResultBody {
    #[serde(flatten)]
    pub activation: ActivationBody,
    /// False when the version was the active one already.
    pub changed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromoteBody {
    pub namespace_id: String,
    pub reason: String,
    #[serde(default)]
    pub accept_breaking: bool,
}

/// What a promotion came to.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionBody {
    /// `PUBLISHED`, `REQUESTED` (the target namespace asks for approvals) or `ALREADY_ACTIVE`.
    pub outcome: &'static str,
    pub version: Option<PublishedBody>,
    pub request: Option<RequestBody>,
    pub active_version: Option<u32>,
}

impl PromotionBody {
    #[must_use]
    pub fn of(promotion: &Promotion) -> Self {
        match promotion {
            Promotion::Published(published) => Self {
                outcome: "PUBLISHED",
                version: Some(PublishedBody {
                    version: VersionSummary::from(&published.version),
                    created: published.created,
                    warnings: sagat_application::definitions::warnings_text(&published.warnings),
                }),
                request: None,
                active_version: Some(published.version.number()),
            },
            Promotion::Requested(request) => Self {
                outcome: "REQUESTED",
                version: None,
                request: Some(RequestBody::from(request.as_ref())),
                active_version: None,
            },
            Promotion::AlreadyActive { version } => Self {
                outcome: "ALREADY_ACTIVE",
                version: None,
                request: None,
                active_version: Some(*version),
            },
        }
    }
}
