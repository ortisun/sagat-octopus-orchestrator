//! Wire types of the script routes: what an author writes and reads, and what a sandbox runner is
//! handed and hands back.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sagat_application::scripts::{
    ArtifactRecord, ArtifactSummary, BuildAssignment, ExecutionRecord, ExecutionSummary,
    PolicyRequest, ScriptRecord, TestAssignment, TestRunRecord, TestRunSummary, VersionRecord,
};
use sagat_domain::scripts::{Egress, Failure, Policy, activity_of};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::ApiError;

const MIB: u64 = 1024 * 1024;

/// `null` and an absent field mean different things in a change: absent leaves the value, `null`
/// removes it.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Limits as an author writes them. Every field is optional; what is left out is the default.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_millis: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratch_mib: Option<u64>,
    /// `none` (the default) or `namespace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<String>,
}

impl PolicyBody {
    /// The request the body is. Out-of-range values are refused later, by the limits themselves.
    pub fn request(&self) -> Result<PolicyRequest, ApiError> {
        Ok(PolicyRequest {
            memory_bytes: self.memory_mib.map(|v| v.saturating_mul(MIB)),
            cpu_millis: self.cpu_millis,
            wall: self.timeout_ms.map(Duration::from_millis),
            processes: self.processes,
            output_bytes: self.output_bytes,
            scratch_bytes: self.scratch_mib.map(|v| v.saturating_mul(MIB)),
            egress: self
                .egress
                .as_deref()
                .map(|text| {
                    text.parse::<Egress>()
                        .map_err(|_| ApiError::validation("egress is `none` or `namespace`"))
                })
                .transpose()?,
        })
    }
}

/// A policy as stored, in the units an author writes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyView {
    pub memory_mib: u64,
    pub cpu_millis: u32,
    pub timeout_ms: u64,
    pub processes: u32,
    pub output_bytes: usize,
    pub scratch_mib: u64,
    pub egress: &'static str,
}

impl From<&Policy> for PolicyView {
    fn from(policy: &Policy) -> Self {
        let l = &policy.limits;
        Self {
            memory_mib: l.memory_bytes() / MIB,
            cpu_millis: l.cpu_millis(),
            timeout_ms: u64::try_from(l.wall().as_millis()).unwrap_or(u64::MAX),
            processes: l.processes(),
            output_bytes: l.output_bytes(),
            scratch_mib: l.scratch_bytes() / MIB,
            egress: policy.egress.as_str(),
        }
    }
}

/// A schema in a request: a JSON object, kept as its serialized bytes.
fn schema_bytes(value: &Value) -> Result<Vec<u8>, ApiError> {
    if !value.is_object() {
        return Err(ApiError::validation("a schema is a JSON Schema object"));
    }
    serde_json::to_vec(value).map_err(|_| ApiError::validation("a schema is a JSON Schema object"))
}

fn schema_value(bytes: Option<&Vec<u8>>) -> Option<Value> {
    bytes.and_then(|b| serde_json::from_slice(b).ok())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateScriptBody {
    pub name: String,
    pub language: String,
    pub source: String,
    #[serde(default)]
    pub lockfile: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub policy: Option<PolicyBody>,
    #[serde(default)]
    pub retention_days: Option<u16>,
}

impl CreateScriptBody {
    pub fn input_schema_bytes(&self) -> Result<Option<Vec<u8>>, ApiError> {
        self.input_schema.as_ref().map(schema_bytes).transpose()
    }

    pub fn output_schema_bytes(&self) -> Result<Option<Vec<u8>>, ApiError> {
        self.output_schema.as_ref().map(schema_bytes).transpose()
    }
}

/// A change of the draft, against the revision the editor read.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateScriptBody {
    pub revision: u64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub lockfile: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub input_schema: Option<Option<Value>>,
    #[serde(default, deserialize_with = "present")]
    pub output_schema: Option<Option<Value>>,
    #[serde(default)]
    pub policy: Option<PolicyBody>,
    #[serde(default)]
    pub retention_days: Option<u16>,
}

impl UpdateScriptBody {
    pub fn input_schema_change(&self) -> Result<Option<Option<Vec<u8>>>, ApiError> {
        self.input_schema
            .as_ref()
            .map(|inner| inner.as_ref().map(schema_bytes).transpose())
            .transpose()
    }

    pub fn output_schema_change(&self) -> Result<Option<Option<Vec<u8>>>, ApiError> {
        self.output_schema
            .as_ref()
            .map(|inner| inner.as_ref().map(schema_bytes).transpose())
            .transpose()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptBody {
    pub id: String,
    pub namespace_id: String,
    pub name: String,
    pub language: &'static str,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub policy: PolicyView,
    pub retention_days: u16,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<u32>,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

impl From<&ScriptRecord> for ScriptBody {
    fn from(script: &ScriptRecord) -> Self {
        let draft = &script.draft;
        Self {
            id: script.id.to_string(),
            namespace_id: script.namespace_id.to_string(),
            name: script.name.to_string(),
            language: script.language.as_str(),
            source: draft.source.as_str().to_owned(),
            lockfile: draft.lockfile.as_ref().map(|l| l.as_str().to_owned()),
            input_schema: schema_value(draft.input_schema.as_ref()),
            output_schema: schema_value(draft.output_schema.as_ref()),
            policy: PolicyView::from(&draft.policy),
            retention_days: draft.retention.get(),
            revision: script.revision,
            latest_version: script.latest_version.map(|n| n.get()),
            created_by: script.created_by.clone(),
            created_at: script.created_at,
            updated_at: script.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ScriptListBody {
    pub items: Vec<ScriptBody>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionBody {
    pub id: String,
    pub script_id: String,
    pub number: u32,
    pub language: &'static str,
    pub status: &'static str,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub policy: PolicyView,
    pub retention_days: u16,
    pub content_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_key: Option<String>,
    /// What a workflow pins: the digest of the artifact the build made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
    /// The activity a `script` node becomes, which a worker binding can name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_log: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub built_at: Option<time::OffsetDateTime>,
}

impl From<&VersionRecord> for VersionBody {
    fn from(version: &VersionRecord) -> Self {
        let draft = &version.draft;
        Self {
            id: version.id.to_string(),
            script_id: version.script_id.to_string(),
            number: version.number.get(),
            language: version.language.as_str(),
            status: version.status.as_str(),
            source: draft.source.as_str().to_owned(),
            lockfile: draft.lockfile.as_ref().map(|l| l.as_str().to_owned()),
            input_schema: schema_value(draft.input_schema.as_ref()),
            output_schema: schema_value(draft.output_schema.as_ref()),
            policy: PolicyView::from(&draft.policy),
            retention_days: draft.retention.get(),
            content_digest: version.content_digest.to_string(),
            build_key: version.build_key.as_ref().map(ToString::to_string),
            artifact_digest: version.artifact.as_ref().map(ToString::to_string),
            activity: version.artifact.as_ref().map(activity_of),
            key_id: version.key_id.clone(),
            toolchain: version.toolchain.clone(),
            build_log: version.build_log.clone(),
            failure: version.failure.clone(),
            created_by: version.created_by.clone(),
            created_at: version.created_at,
            built_at: version.built_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct VersionListBody {
    pub items: Vec<VersionBody>,
}

/// Published, or already the latest version.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedBody {
    pub version: VersionBody,
    /// `false` when the draft was the latest version already.
    pub created: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSummaryBody {
    pub digest: String,
    pub language: &'static str,
    pub size: u64,
    pub key_id: String,
    pub signature: String,
    pub toolchain: String,
    pub sbom: Option<Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
}

impl From<&ArtifactSummary> for ArtifactSummaryBody {
    fn from(artifact: &ArtifactSummary) -> Self {
        Self {
            digest: artifact.digest.to_string(),
            language: artifact.language.as_str(),
            size: artifact.size,
            key_id: artifact.key_id.clone(),
            signature: STANDARD.encode(&artifact.signature),
            toolchain: artifact.toolchain.clone(),
            sbom: serde_json::from_slice(&artifact.sbom).ok(),
            created_at: artifact.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestRunRequestBody {
    /// The input of the script, any JSON value.
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRunBody {
    pub id: String,
    pub script_id: String,
    pub language: &'static str,
    pub status: &'static str,
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Why there is no result, when there is none: a stable code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    pub log: String,
    pub log_truncated: bool,
    /// What the script was refused: `fetch https://host` or `secret name`.
    pub denied: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
    /// From asking for a container to the first instruction of the script (a cold or warm start).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_peak_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_output_schema: Option<Value>,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<time::OffsetDateTime>,
}

impl From<&TestRunRecord> for TestRunBody {
    fn from(run: &TestRunRecord) -> Self {
        Self {
            id: run.id.to_string(),
            script_id: run.script_id.to_string(),
            language: run.language.as_str(),
            status: run.status.as_str(),
            input: serde_json::from_slice(&run.input).ok(),
            result: run
                .result
                .as_deref()
                .and_then(|r| serde_json::from_slice(r).ok()),
            failure: run.failure.map(Failure::code),
            retryable: run.failure.map(Failure::retryable),
            log: run.log.clone(),
            log_truncated: run.log_truncated,
            denied: run.denied.clone(),
            exit_code: run.exit_code,
            wall_ms: run.wall_ms,
            start_ms: run.start_ms,
            memory_peak_bytes: run.memory_peak_bytes,
            inferred_input_schema: schema_value(run.inferred_input_schema.as_ref()),
            inferred_output_schema: schema_value(run.inferred_output_schema.as_ref()),
            created_by: run.created_by.clone(),
            created_at: run.created_at,
            finished_at: run.finished_at,
        }
    }
}

/// A test run as a listing shows it: how it went; the run itself has what it said and returned.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRunSummaryBody {
    pub id: String,
    pub script_id: String,
    pub language: &'static str,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<time::OffsetDateTime>,
}

impl From<&TestRunSummary> for TestRunSummaryBody {
    fn from(run: &TestRunSummary) -> Self {
        Self {
            id: run.id.to_string(),
            script_id: run.script_id.to_string(),
            language: run.language.as_str(),
            status: run.status.as_str(),
            failure: run.failure.map(Failure::code),
            wall_ms: run.wall_ms,
            start_ms: run.start_ms,
            created_by: run.created_by.clone(),
            created_at: run.created_at,
            finished_at: run.finished_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TestRunListBody {
    pub items: Vec<TestRunSummaryBody>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionBody {
    pub id: String,
    pub artifact_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub log: String,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub wall_ms: u64,
    pub start_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_peak_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: time::OffsetDateTime,
}

impl From<&ExecutionRecord> for ExecutionBody {
    fn from(record: &ExecutionRecord) -> Self {
        let e = &record.execution;
        Self {
            id: record.id.to_string(),
            artifact_digest: e.digest.to_string(),
            run_id: e.run_id.map(|id| id.to_string()),
            node: e.node.clone(),
            attempt: e.attempt,
            status: e.status.as_str(),
            failure: e.failure.map(Failure::code),
            exit_code: e.exit_code,
            log: e.log.clone(),
            log_truncated: e.log_truncated,
            denied: e.denied.clone(),
            wall_ms: e.wall_ms,
            start_ms: e.start_ms,
            memory_peak_bytes: e.memory_peak_bytes,
            trace_id: e.trace_id.clone(),
            created_at: record.created_at,
            expires_at: record.expires_at,
        }
    }
}

/// An execution as a listing shows it: without its log.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummaryBody {
    pub id: String,
    pub artifact_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub wall_ms: u64,
    pub start_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_peak_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: time::OffsetDateTime,
}

impl From<&ExecutionSummary> for ExecutionSummaryBody {
    fn from(e: &ExecutionSummary) -> Self {
        Self {
            id: e.id.to_string(),
            artifact_digest: e.digest.to_string(),
            run_id: e.run_id.map(|id| id.to_string()),
            node: e.node.clone(),
            attempt: e.attempt,
            status: e.status.as_str(),
            failure: e.failure.map(Failure::code),
            exit_code: e.exit_code,
            log_truncated: e.log_truncated,
            denied: e.denied.clone(),
            wall_ms: e.wall_ms,
            start_ms: e.start_ms,
            memory_peak_bytes: e.memory_peak_bytes,
            trace_id: e.trace_id.clone(),
            created_at: e.created_at,
            expires_at: e.expires_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExecutionListBody {
    pub items: Vec<ExecutionSummaryBody>,
}

// ----------------------------------------------------------------------------------- the runner

/// What a runner can build or run: one toolchain identity per language.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimBuildBody {
    /// Language to the identity of the toolchain that builds it (the digest of its image and the
    /// version of the runner's shim).
    pub toolchains: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimTestBody {
    pub languages: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildAssignmentBody {
    pub job_id: String,
    pub lease_token: String,
    pub lease_seconds: u64,
    pub version_id: String,
    pub script_id: String,
    pub language: &'static str,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile: Option<String>,
    /// The policy as the platform writes it, to be built into the artifact.
    pub policy: String,
    /// The schemas the input and the result must satisfy, to be built into the artifact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub build_key: String,
}

impl From<&BuildAssignment> for BuildAssignmentBody {
    fn from(job: &BuildAssignment) -> Self {
        Self {
            job_id: job.job.to_string(),
            lease_token: job.lease_token.clone(),
            lease_seconds: job.lease.as_secs(),
            version_id: job.version.to_string(),
            script_id: job.script.to_string(),
            language: job.language.as_str(),
            source: job.source.as_str().to_owned(),
            lockfile: job.lockfile.as_ref().map(|l| l.as_str().to_owned()),
            policy: job.policy.canonical(),
            input_schema: schema_value(job.input_schema.as_ref()),
            output_schema: schema_value(job.output_schema.as_ref()),
            build_key: job.build_key.to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseBody {
    pub lease_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseExtendedBody {
    pub lease_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompleteBuildBody {
    pub lease_token: String,
    /// `built` or `failed`.
    pub outcome: String,
    /// The artifact, base64.
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub sbom: Option<Value>,
    #[serde(default)]
    pub toolchain: Option<String>,
    #[serde(default)]
    pub log: Option<String>,
    /// For `failed`: `rejected`, `lockfile`, `limit`, `unsupported` or `unavailable`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestAssignmentBody {
    pub run_id: String,
    pub lease_token: String,
    pub lease_seconds: u64,
    pub script_id: String,
    pub namespace_id: String,
    pub language: &'static str,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub policy: String,
    pub input: Value,
}

impl From<&TestAssignment> for TestAssignmentBody {
    fn from(assignment: &TestAssignment) -> Self {
        let run = &assignment.run;
        let draft = &run.draft;
        Self {
            run_id: run.id.to_string(),
            lease_token: assignment.lease_token.clone(),
            lease_seconds: assignment.lease.as_secs(),
            script_id: run.script_id.to_string(),
            namespace_id: run.namespace_id.to_string(),
            language: run.language.as_str(),
            source: draft.source.as_str().to_owned(),
            lockfile: draft.lockfile.as_ref().map(|l| l.as_str().to_owned()),
            input_schema: schema_value(draft.input_schema.as_ref()),
            output_schema: schema_value(draft.output_schema.as_ref()),
            policy: draft.policy.canonical(),
            input: serde_json::from_slice(&run.input).unwrap_or(Value::Null),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompleteTestBody {
    pub lease_token: String,
    /// The result, when there is one.
    #[serde(default)]
    pub result: Option<Value>,
    /// Why there is none: a `SCRIPT_*` code.
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub log_truncated: bool,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub wall_ms: u64,
    #[serde(default)]
    pub start_ms: u64,
    #[serde(default)]
    pub memory_peak_bytes: Option<u64>,
    #[serde(default)]
    pub inferred_input_schema: Option<Value>,
    #[serde(default)]
    pub inferred_output_schema: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerArtifactBody {
    /// The tenant the signature was made for.
    pub tenant_id: String,
    pub digest: String,
    pub language: &'static str,
    /// The artifact, base64.
    pub artifact: String,
    pub signature: String,
    pub key_id: String,
    pub toolchain: String,
}

impl From<&ArtifactRecord> for RunnerArtifactBody {
    fn from(artifact: &ArtifactRecord) -> Self {
        Self {
            tenant_id: artifact.tenant_id.to_string(),
            digest: artifact.digest.to_string(),
            language: artifact.language.as_str(),
            artifact: STANDARD.encode(&artifact.content),
            signature: STANDARD.encode(&artifact.signature),
            key_id: artifact.key_id.clone(),
            toolchain: artifact.toolchain.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SigningKeyBody {
    pub key_id: String,
    /// The public key, base64.
    pub public_key: String,
}

impl SigningKeyBody {
    #[must_use]
    pub fn new(key_id: &str, public_key: &[u8]) -> Self {
        Self {
            key_id: key_id.to_owned(),
            public_key: STANDARD.encode(public_key),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerEgressBody {
    pub rules: Vec<String>,
    pub ports: Vec<u16>,
    pub allow_http: bool,
}

#[derive(Debug, Serialize)]
pub struct PendingBody {
    /// Activities (`script.sha256-...`) with a step ready to run.
    pub activities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordExecutionBody {
    pub namespace_id: String,
    pub digest: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub attempt: Option<u32>,
    /// `SUCCEEDED` or `FAILED`.
    pub status: String,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub log_truncated: bool,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub wall_ms: u64,
    #[serde(default)]
    pub start_ms: u64,
    #[serde(default)]
    pub memory_peak_bytes: Option<u64>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RecordedBody {
    /// `false` when this attempt had been recorded before.
    pub recorded: bool,
}

/// The bytes a runner sent as an artifact.
pub fn decode_artifact(text: &str) -> Result<Vec<u8>, ApiError> {
    STANDARD
        .decode(text)
        .map_err(|_| ApiError::validation("the artifact is not base64"))
}
