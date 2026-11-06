//! Business identifiers: the lookup of a binding and the two calls that start a run under one
//! (`signal_with_start`, `update_with_start`). The identifier is sent as given, with no trimming
//! and no case folding; the server judges it and the policies.
use super::wire::{Fields, summary_debug, wire_enum};
use super::*;
use crate::UpdateReceipt;
use serde::{Deserialize, Serialize};

wire_enum! {
    /// What a start does when an open execution holds the business identifier.
    BusinessIdConflictPolicy { Fail => "FAIL", UseExisting => "USE_EXISTING" }
}
wire_enum! {
    /// What a start does when a closed execution still remembers the business identifier.
    BusinessIdReusePolicy {
        AllowDuplicate => "ALLOW_DUPLICATE",
        AllowDuplicateFailedOnly => "ALLOW_DUPLICATE_FAILED_ONLY",
        RejectDuplicate => "REJECT_DUPLICATE",
    }
}

/// How a business identifier stands in a namespace.
#[derive(Clone)]
pub struct BusinessIdBinding {
    pub business_id: String,
    /// The run that holds the identifier, or the initial run of the continuation chain that does.
    pub execution_id: Uuid,
    /// The run that carries the work now.
    pub current_run_id: Uuid,
    pub open: bool,
    pub last_status: Option<String>,
    pub workflow_id: Uuid,
    pub bound_at: OffsetDateTime,
    pub reusable_after: Option<OffsetDateTime>,
    pub raw: Value,
}
summary_debug!(BusinessIdBinding { execution_id, open });
impl BusinessIdBinding {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        let f = Fields::of(&raw, "a business id binding")?;
        Ok(Self {
            business_id: f.text("businessId")?,
            execution_id: f.uuid("executionId")?,
            current_run_id: f.uuid("currentRunId")?,
            open: f.flag("open"),
            last_status: f.opt_text("lastStatus"),
            workflow_id: f.uuid("workflowId")?,
            bound_at: f.instant("boundAt")?,
            reusable_after: f.opt_instant("reusableAfter")?,
            raw,
        })
    }
}

/// The lookup of business identifiers, reached from [`Client::business_ids`].
#[derive(Clone, Copy)]
pub struct BusinessIds<'a> {
    client: &'a Client,
}
impl std::fmt::Debug for BusinessIds<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusinessIds").finish_non_exhaustive()
    }
}
impl Client {
    /// How a business identifier stands in a namespace.
    pub fn business_ids(&self) -> BusinessIds<'_> {
        BusinessIds { client: self }
    }
}
impl BusinessIds<'_> {
    /// How an identifier stands in a namespace. An identifier nobody holds is `NOT_FOUND`. It is
    /// percent-encoded as given; only a value that cannot be one path segment (not 1 to 200 visible
    /// ASCII characters, or exactly `.` or `..`, which every resolver reads as a step in the path)
    /// is refused here, and nothing is sent.
    pub async fn get(
        &self,
        namespace_id: &str,
        business_id: &str,
        call: CallOptions,
    ) -> Result<BusinessIdBinding, SagatError> {
        if !body_key(business_id, 200) || matches!(business_id, "." | "..") {
            return Err(SagatError::validation(
                "a business identifier is 1 to 200 visible ASCII characters, and not . or ..",
            ));
        }
        let mut path = wire::namespace_path(namespace_id)?;
        path.extend(["business-ids".to_owned(), business_id.to_owned()]);
        self.client
            .execute(wire::request(Method::GET, &path, 200, Vec::new()), call)
            .await?
            .parse(BusinessIdBinding::parse)
    }
}

/// The options the with-start calls share.
#[derive(Clone, Debug, Default)]
pub struct WithStartOptions {
    pub call: CallOptions,
    pub version_id: Option<String>,
    pub correlation_id: Option<String>,
    pub deadline: Option<RunDeadline>,
    pub business_id_reuse_policy: Option<BusinessIdReusePolicy>,
}
impl From<CallOptions> for WithStartOptions {
    fn from(call: CallOptions) -> Self {
        Self {
            call,
            ..Self::default()
        }
    }
}
/// The options of `update_with_start`: the shared ones, the policy for an open holder and the
/// start delay (a `Duration`, converted once to integer milliseconds before the first attempt).
#[derive(Clone, Debug, Default)]
pub struct UpdateWithStartOptions {
    pub with_start: WithStartOptions,
    pub business_id_conflict_policy: Option<BusinessIdConflictPolicy>,
    pub start_delay: Option<Duration>,
    /// The revision of the state the update expects; zero or more.
    pub expected_state_revision: Option<u64>,
}
impl From<CallOptions> for UpdateWithStartOptions {
    fn from(call: CallOptions) -> Self {
        Self {
            with_start: call.into(),
            ..Self::default()
        }
    }
}
impl From<WithStartOptions> for UpdateWithStartOptions {
    fn from(with_start: WithStartOptions) -> Self {
        Self {
            with_start,
            ..Self::default()
        }
    }
}

/// What a signal-with-start did.
#[derive(Clone)]
pub struct SignalWithStartResult {
    pub run_id: Uuid,
    /// Whether this request created the run.
    pub started: bool,
    /// The nodes the signal woke.
    pub delivered_to: Vec<String>,
    pub raw: Value,
}
summary_debug!(SignalWithStartResult { run_id, started });
impl SignalWithStartResult {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        let f = Fields::of(&raw, "a signal-with-start result")?;
        Ok(Self {
            run_id: f.uuid("runId")?,
            started: f.flag("started"),
            delivered_to: f.strings("deliveredTo"),
            raw,
        })
    }
}

/// What became of the update of an update-with-start: `outcome` is `APPLIED` with a receipt, or a
/// rejection with its `reason` (an answer, not an error).
#[derive(Clone)]
pub struct UpdateWithStartOutcome {
    pub outcome: String,
    pub receipt: Option<UpdateReceipt>,
    pub reason: Option<String>,
}
summary_debug!(UpdateWithStartOutcome { outcome, reason });

#[derive(Clone)]
pub struct UpdateWithStartResult {
    pub run_id: Uuid,
    /// Whether this request created the run.
    pub started: bool,
    pub update: UpdateWithStartOutcome,
    pub raw: Value,
}
summary_debug!(UpdateWithStartResult { run_id, started });
impl UpdateWithStartResult {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        let f = Fields::of(&raw, "an update-with-start result")?;
        let update = raw
            .get("update")
            .ok_or_else(|| SagatError::new("PROTOCOL", "update is missing from the answer"))?;
        let u = Fields::of(update, "the update of the result")?;
        let receipt = match update.get("receipt") {
            None | Some(Value::Null) => None,
            Some(receipt) => Some(UpdateReceipt::parse(receipt.clone())?),
        };
        Ok(Self {
            run_id: f.uuid("runId")?,
            started: f.flag("started"),
            update: UpdateWithStartOutcome {
                outcome: u.text("outcome")?,
                receipt,
                reason: u.opt_text("reason"),
            },
            raw,
        })
    }
}

fn shared_fields(body: &mut Value, options: &WithStartOptions) -> Result<(), SagatError> {
    if let Some(version) = &options.version_id {
        uuid(version)?;
        body["versionId"] = json!(version);
    }
    if let Some(correlation) = &options.correlation_id {
        body["correlationId"] = json!(correlation);
    }
    if let Some(deadline) = &options.deadline {
        let instant = match deadline {
            RunDeadline::At(instant) => *instant,
            RunDeadline::After(duration) => OffsetDateTime::now_utc()
                .checked_add(time::Duration::try_from(*duration).map_err(|_| validation())?)
                .ok_or_else(validation)?,
        };
        body["deadline"] = json!(
            instant
                .to_offset(UtcOffset::UTC)
                .format(&Rfc3339)
                .map_err(|_| validation())?
        );
    }
    if let Some(policy) = &options.business_id_reuse_policy {
        body["businessIdReusePolicy"] = json!(policy.as_str());
    }
    Ok(())
}

impl Client {
    /// Signals the open run of a business identifier, or starts one and delivers the signal at its
    /// creation. A run no step of which waits for the signal is refused (`CONFLICT`) and nothing is
    /// created or changed. One `Idempotency-Key` is given or generated and kept by every retry.
    pub async fn signal_with_start(
        &self,
        workflow: &str,
        business_id: &str,
        input: &Value,
        signal_name: &str,
        payload: &Value,
        options: impl Into<WithStartOptions>,
    ) -> Result<SignalWithStartResult, SagatError> {
        if !input.is_object() || !payload.is_object() {
            return Err(validation());
        }
        let options = options.into();
        let request_key = key(&options.call)?;
        let mut body = json!({"businessId":business_id,"input":input,"signal":{"name":signal_name,"payload":payload}});
        shared_fields(&mut body, &options)?;
        body["workflowId"] = self
            .resolve_workflow(workflow, options.call.clone())
            .await?
            .into();
        let request = WireRequest::new(Method::POST, &["runs", "signal-with-start"], 202)
            .json(&body)?
            .keyed(request_key);
        self.execute(request, options.call)
            .await?
            .parse(SignalWithStartResult::parse)
    }

    /// Applies a declared Update to the open run of a business identifier, starting the run first
    /// when none is open. The caller's `update_key` (1 to 128 printable ASCII characters) is required
    /// and kept by every retry. A refusal of the validator or a stale revision is an answer
    /// (`update.outcome` is `REJECTED`), not an error.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_with_start(
        &self,
        workflow: &str,
        business_id: &str,
        input: &Value,
        update_name: &str,
        update_key: &str,
        args: &Value,
        options: impl Into<UpdateWithStartOptions>,
    ) -> Result<UpdateWithStartResult, SagatError> {
        let options = options.into();
        if !input.is_object()
            || !body_key(update_key, 128)
            || options
                .expected_state_revision
                .is_some_and(|v| v > i64::MAX as u64)
        {
            return Err(validation());
        }
        operation_name(update_name)?;
        let request_key = key(&options.with_start.call)?;
        let mut update = json!({"name":update_name,"updateKey":update_key,"args":args});
        if let Some(revision) = options.expected_state_revision {
            update["expectedStateRevision"] = json!(revision);
        }
        let mut body = json!({"businessId":business_id,"input":input,"update":update});
        shared_fields(&mut body, &options.with_start)?;
        if let Some(policy) = &options.business_id_conflict_policy {
            body["businessIdConflictPolicy"] = json!(policy.as_str());
        }
        if let Some(delay) = options.start_delay {
            body["startDelayMs"] = json!(wire::delay_millis(delay));
        }
        body["workflowId"] = self
            .resolve_workflow(workflow, options.with_start.call.clone())
            .await?
            .into();
        let request = WireRequest::new(Method::POST, &["runs", "update-with-start"], 200)
            .json(&body)?
            .keyed(request_key);
        self.execute(request, options.with_start.call)
            .await?
            .parse(UpdateWithStartResult::parse)
    }
}
