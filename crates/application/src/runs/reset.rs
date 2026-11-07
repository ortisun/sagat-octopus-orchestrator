//! Reset to a point (feature 018, US06): the journal of a source run as a reset reads it, and the prefix a reset
//! re-emits under the new run. The rules of which points exist are the domain's
//! (`sagat_domain::execution::reset`); this module is what the store and the service exchange, and the use
//! cases of the points, the preview and the reset.

use sagat_domain::execution::{
    AddressedRunEvent, ResetId, ResetPlan, ResetPoint, ResetReapply, ResetRefusal, RunId,
    reset_plan, reset_points,
};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;

use super::pending_reset::{Creation, request_receipt};
use super::reapply::{
    Delivery, Material, PlannedReapply, ReapplyEntry, ReapplyKind, ReapplyOutcome, ReapplyReason,
    ReportEntry, TooMany, candidates, end_signals_at_first_skip, reapply_key, update_answer,
    update_refusal,
};
use super::*;

/// The events a reset re-emits at most, by default (`FR-018-045`): a longer prefix is refused
/// (`RESET_PREFIX_TOO_LONG`), because the re-emission is one transaction. The effective value will be the
/// minimum of the request, the namespace and the deployment, like every ceiling of the feature (G1980).
pub const RESET_PREFIX_EVENTS: usize = 5_000;

/// What a reset is asked for (`ResetRequest` of the contract).
#[derive(Debug, Clone, Copy)]
pub struct ResetRequest<'a> {
    /// The sequence of the last event of the source that the new run keeps.
    pub sequence: u64,
    /// Why, in 8 to 2000 characters of plain text; it goes to the audit trail.
    pub reason: &'a str,
    /// The caller knows that steps that completed in the source run again.
    pub acknowledge_reexecution: bool,
    /// What becomes of the business identifier of the source.
    pub business: RerunBusinessId,
    /// What is delivered again after the point.
    pub reapply: ResetReapply,
    /// What to do when the source has not ended.
    pub source_policy: ResetSourcePolicy,
}

/// What a preview says: the plan of the reset, and the signals and Updates it would try to deliver again
/// (empty for `reapply: NONE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPreview {
    pub plan: ResetPlan,
    /// Each candidate as `WILL_TRY`, or `NOT_REAPPLIED` with the reason it already has.
    pub reapply: Vec<ReapplyEntry>,
}

fn refused(reason: &'static str) -> AppError {
    AppError::ResetRefused {
        reason,
        event_type: None,
    }
}

/// Where a reset is (`state` of the contract). `Failed` and `PendingSource` are written by the reset that waits
/// for its source to end (`CANCEL_SOURCE`); a reset of an ended source is `Created` when it is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetState {
    PendingSource,
    Created,
    Failed,
}

impl ResetState {
    /// The closed name the register stores and the API answers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingSource => "PENDING_SOURCE",
            Self::Created => "CREATED",
            Self::Failed => "FAILED",
        }
    }

    /// The state a stored name stands for; `None` for a name this version does not know.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "PENDING_SOURCE" => Some(Self::PendingSource),
            "CREATED" => Some(Self::Created),
            "FAILED" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// What a reset point is, by the event whose completion it is (`ResetPoint.kind` of the contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetPointKind {
    RunStarted,
    StepCompleted,
    ChildFinished,
    CollectionFinished,
    LoopFinished,
}

impl ResetPointKind {
    /// The closed name the API answers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunStarted => "RUN_STARTED",
            Self::StepCompleted => "STEP_COMPLETED",
            Self::ChildFinished => "CHILD_FINISHED",
            Self::CollectionFinished => "COLLECTION_FINISHED",
            Self::LoopFinished => "LOOP_FINISHED",
        }
    }

    /// The kind of the point that ends at an event of this type; `None` for a type that never ends one.
    #[must_use]
    pub fn of_event_type(event_type: &str) -> Option<Self> {
        match event_type {
            "run.started" => Some(Self::RunStarted),
            "step.completed" => Some(Self::StepCompleted),
            "child.finished" => Some(Self::ChildFinished),
            "collection.finished" => Some(Self::CollectionFinished),
            "loop.finished" => Some(Self::LoopFinished),
            _ => None,
        }
    }
}

/// A place a run can be reset to, as the API shows it: where, of what kind, at which node and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPointInfo {
    /// `N`: the events `1..=N` of the source are re-emitted.
    pub sequence: u64,
    pub kind: ResetPointKind,
    /// The root node whose completion this is; none for the start of the run.
    pub node: Option<String>,
    /// When the event that ends the prefix happened.
    pub occurred_at: OffsetDateTime,
}

fn point_info(source: &ResetSource, point: ResetPoint) -> Result<ResetPointInfo, AppError> {
    let row = usize::try_from(point.sequence)
        .ok()
        .and_then(|sequence| sequence.checked_sub(1))
        .and_then(|index| source.events.get(index));
    let Some(row) = row else {
        tracing::error!(
            sequence = point.sequence,
            "a reset point is not a row of the journal"
        );
        return Err(AppError::Internal);
    };
    let Some(kind) = ResetPointKind::of_event_type(&row.event_type) else {
        tracing::error!(sequence = point.sequence, event_type = %row.event_type, "a reset point ends at an event that cannot end one");
        return Err(AppError::Internal);
    };
    Ok(ResetPointInfo {
        sequence: point.sequence,
        kind,
        node: point.completes.map(|node| node.as_str().to_owned()),
        occurred_at: row.occurred_at,
    })
}

/// An entry of the register of resets (`run_resets`, `Reset` of the contract): that a run was created from the
/// prefix of another, from which point, by whom and with what outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetRecord {
    pub id: ResetId,
    pub source: RunId,
    /// The sequence of the last event of the source that the new run keeps.
    pub sequence: u64,
    pub state: ResetState,
    /// Why a reset that failed did (`FAILED` only).
    pub failure_reason: Option<String>,
    /// The run the reset created (`CREATED` only).
    pub new_run: Option<RunId>,
    pub reapply: ResetReapply,
    /// What the reapply did, in journal order (empty for `NONE`): never a payload or an argument. Read as the register
    /// holds it, because a later version may have written names this one does not know.
    pub reapply_report: Vec<ReportEntry>,
    /// What the request said to do with a source that had not ended; `None` for a policy a later version wrote.
    pub source_policy: Option<ResetSourcePolicy>,
    pub created_at: OffsetDateTime,
}

/// What a reset writes to the register in the transaction that creates its run.
#[derive(Clone, Copy)]
pub struct ResetRegistration<'a> {
    /// The id of the entry. It is chosen before the reapply, whose keys are derived from it.
    pub id: ResetId,
    pub source: RunId,
    pub sequence: u64,
    /// The digest of the prefix re-emitted, which `run.reset` carries too.
    pub prefix_digest: sagat_domain::workflow::Digest,
    pub reapply: ResetReapply,
    pub business: RerunBusinessId,
    /// What the request said to do with a source that had not ended: the register says what was asked, though a
    /// source that has ended is reset at once.
    pub source_policy: ResetSourcePolicy,
    /// Whether the request acknowledged that steps run again.
    pub acknowledged: bool,
    /// The subject that asked.
    pub actor: &'a str,
    /// The justification, in plain text; the register keeps it as content of the tenant.
    pub reason: &'a str,
    /// What the reapply did, written with the entry in the transaction that creates the run.
    pub report: &'a [ReapplyEntry],
    /// The entry already exists, `PENDING_SOURCE`, and this completes it (`CANCEL_SOURCE`); otherwise it is
    /// written, `CREATED`, with the run.
    pub completes_pending: bool,
}

impl std::fmt::Debug for ResetRegistration<'_> {
    /// The subject that asked, the justification and the report are not printed.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResetRegistration")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("sequence", &self.sequence)
            .field("reapply", &self.reapply)
            .field("source_policy", &self.source_policy)
            .field("completes_pending", &self.completes_pending)
            .finish_non_exhaustive()
    }
}

/// What a reset answers: the entry of the register and the run it created, if it has: a request that waits for
/// its source to end (`CANCEL_SOURCE`) has none yet.
#[derive(Debug, Clone)]
pub struct ResetMade {
    pub reset: ResetRecord,
    pub run: Option<RunRecord>,
}

/// One row of the journal of a source run, decoded, with what a reset keeps of it: who did it, when, which
/// trace, which payload it names (a step output, a signal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvent {
    /// The place of the row in the journal of the source, from 1.
    pub sequence: u64,
    pub event: AddressedRunEvent,
    /// The row as the source stored it, which is what the new run stores: a reset copies, it does not encode
    /// again, so what a version added to a detail that its event does not carry is kept.
    pub event_type: String,
    pub node: Option<String>,
    pub attempt: Option<i32>,
    pub detail: serde_json::Value,
    pub actor: String,
    pub trace_id: Option<String>,
    pub occurred_at: OffsetDateTime,
    /// The digest of the payload the row refers to, if it does.
    pub payload_digest: Option<Vec<u8>>,
    /// The canonical encoding of the row (sequence, type, node, attempt, detail, payload digest), which the
    /// digest of a prefix is taken over.
    pub canonical: Vec<u8>,
}

/// The journal of a source run, as far as this version can read it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResetSource {
    /// The rows that decode, in order, up to the first one that does not.
    pub events: Vec<SourceEvent>,
    /// The type of the first row that does not decode: a type written by a version this one does not know has
    /// no rule, so no point at or after it exists (`RESET_RULE_MISSING`).
    pub unknown_event_type: Option<String>,
}

/// What a reset re-emits under the new run: the events `1..=N` of the source, with what the rows say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredPrefix {
    pub source: RunId,
    pub events: Vec<SourceEvent>,
}

impl RestoredPrefix {
    /// `N`: the sequence of the last event it re-emits.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.events.last().map_or(0, |event| event.sequence)
    }

    /// The digest of the canonical encoding of the rows `1..=N`: what `run.reset` carries, so a reader can
    /// verify the copy against a retained source.
    #[must_use]
    pub fn digest(&self) -> sagat_domain::workflow::Digest {
        prefix_digest(&self.events)
    }
}

/// The digest of the canonical encoding of `events`, in order, each framed by its length so that no two
/// different prefixes encode alike.
#[must_use]
pub fn prefix_digest(events: &[SourceEvent]) -> sagat_domain::workflow::Digest {
    let mut hasher = Sha256::new();
    for event in events {
        hasher.update((event.canonical.len() as u64).to_be_bytes());
        hasher.update(&event.canonical);
    }
    sagat_domain::workflow::Digest::new(hasher.finalize().into())
}

impl<S, C, E> RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    /// The source of a reset: the run, its journal and the refusals that do not depend on the point (a child
    /// run, a continuation chain, a database execution run on a private definition).
    pub(super) async fn reset_candidate(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<(RunRecord, ResetSource), AppError> {
        principal.authorize(Permission::RunOperate)?;
        principal.authorize(Permission::RunRead)?;
        let record = self.get(principal, id, ctx).await?;
        if record.info.parent.is_some() || record.info.origin.is_some() {
            return Err(refused("RESET_NOT_SUPPORTED_FOR_SOURCE"));
        }
        let Some(source) = self.store.reset_source(principal.tenant_id(), id).await? else {
            return Err(self.missing(principal, id, ctx).await);
        };
        if source.events.iter().any(|row| {
            matches!(
                row.event_type.as_str(),
                "execution_chain.started" | "execution_chain.successor_linked"
            )
        }) {
            return Err(refused("RESET_NOT_SUPPORTED_FOR_SOURCE"));
        }
        Ok((record, source))
    }

    fn reset_journal(source: &ResetSource) -> Vec<AddressedRunEvent> {
        source.events.iter().map(|row| row.event.clone()).collect()
    }

    /// The places a run can be reset to, by ascending sequence (`FR-018-035`): a point whose prefix holds an
    /// event type this version has no rule for is not listed.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:operate` and `run:read`; [`AppError::NotFound`] for a run the
    /// caller cannot see; [`AppError::ResetRefused`] `RESET_NOT_SUPPORTED_FOR_SOURCE` for a child run or a
    /// member of a continuation chain.
    pub async fn reset_points(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<Vec<ResetPointInfo>, AppError> {
        let (_, source) = self.reset_candidate(principal, id, ctx).await?;
        let points = reset_points(id, &Self::reset_journal(&source)).map_err(|error| {
            tracing::error!(%error, run = %id, "the journal of a run does not replay");
            AppError::Internal
        })?;
        points
            .eligible
            .into_iter()
            .map(|point| point_info(&source, point))
            .collect()
    }

    /// What a reset to `sequence` would restore, run again and reapply, changing nothing (`FR-018-036`). With a
    /// `reapply` policy it lists the signals and Updates after the point that the reset would try to deliver
    /// again, in journal order: `WILL_TRY`, or `NOT_REAPPLIED` with the reason they already have. Whether the new
    /// run will accept them (a step that waits, a validator that agrees) is decided by the reset, not here.
    ///
    /// # Errors
    /// As [`Self::reset_points`]; [`AppError::ResetRefused`] `RESET_POINT_NOT_ELIGIBLE`,
    /// `RESET_RULE_MISSING` (with the type), `RESET_PREFIX_TOO_LONG`, `RESET_REAPPLY_NOT_AVAILABLE` for a
    /// `reapply` this deployment does not serve and `RESET_REAPPLY_TOO_MANY`.
    pub async fn reset_preview(
        &self,
        principal: &Principal,
        id: RunId,
        sequence: u64,
        reapply: ResetReapply,
        ctx: &RequestContext,
    ) -> Result<ResetPreview, AppError> {
        self.reapply_available(reapply)?;
        let (record, source) = self.reset_candidate(principal, id, ctx).await?;
        let plan = self
            .plan_reset(principal, &record, &source, sequence)
            .await?;
        let planned = self
            .plan_reapply(principal, &record, &source, sequence, reapply, false)
            .await?;
        Ok(ResetPreview {
            plan,
            reapply: planned.into_iter().map(|item| item.entry).collect(),
        })
    }

    /// Whether this deployment lets a reset deliver signals and Updates again. A preview asks it at once; a reset asks it
    /// where it would create the run (`finish_start`), so that a retry of a request that succeeded is answered from
    /// the register whatever the setting is now.
    pub(super) fn reapply_available(&self, reapply: ResetReapply) -> Result<(), AppError> {
        if reapply != ResetReapply::None && !self.reset_reapply {
            return Err(refused("RESET_REAPPLY_NOT_AVAILABLE"));
        }
        Ok(())
    }

    /// The signals and Updates of the source after `sequence` that `reapply` delivers again to the new run, in
    /// journal order, each with what it needs to be tried (`keep`: the content, for a reset) or the reason it
    /// will not be. The content is read here, outside the transaction that creates the run: it is immutable and
    /// addressed by its digest.
    pub(super) async fn plan_reapply(
        &self,
        principal: &Principal,
        record: &RunRecord,
        source: &ResetSource,
        sequence: u64,
        reapply: ResetReapply,
        keep: bool,
    ) -> Result<Vec<PlannedReapply>, AppError> {
        if reapply == ResetReapply::None {
            return Ok(Vec::new());
        }
        // rows the source holds after the point that this version cannot read may be signals or Updates
        if let Some(event_type) = &source.unknown_event_type {
            return Err(AppError::ResetRefused {
                reason: "RESET_RULE_MISSING",
                event_type: Some(event_type.clone()),
            });
        }
        let definition = self.definition_of(principal, record).await?;
        let found = candidates(&definition, &source.events, sequence, reapply)
            .map_err(|TooMany| refused("RESET_REAPPLY_TOO_MANY"))?;
        // what every candidate needs is read in one statement for the signals and one for the Updates, whatever
        // their number: the chain of resets that made the source is read once
        let tenant = principal.tenant_id();
        let digests: Vec<Vec<u8>> = found
            .iter()
            .filter_map(|candidate| match &candidate.material {
                Material::Signal { payload_digest } => Some(payload_digest.clone()),
                Material::Skip(_) | Material::Update { .. } => None,
            })
            .collect();
        let sequences: Vec<u64> = found
            .iter()
            .filter(|candidate| matches!(candidate.material, Material::Update { .. }))
            .map(|candidate| candidate.sequence)
            .collect();
        let mut signals = if digests.is_empty() {
            Vec::new()
        } else {
            self.store.payload_contents(tenant, &digests, keep).await?
        }
        .into_iter();
        let mut updates = if sequences.is_empty() {
            Vec::new()
        } else {
            self.store
                .retained_update_arguments(tenant, record.info.run_id, &sequences, keep)
                .await?
        }
        .into_iter();
        let mut planned: Vec<PlannedReapply> = found
            .iter()
            .map(|candidate| match &candidate.material {
                Material::Skip(reason) => PlannedReapply::skipped(candidate, *reason),
                Material::Signal { .. } => {
                    PlannedReapply::of_signal(candidate, signals.next().flatten(), keep)
                }
                Material::Update { before_revision } => PlannedReapply::of_update(
                    candidate,
                    *before_revision,
                    updates.next().flatten(),
                    keep,
                ),
            })
            .collect();
        end_signals_at_first_skip(&mut planned);
        Ok(planned)
    }

    /// The definition a run was started with.
    async fn definition_of(
        &self,
        principal: &Principal,
        record: &RunRecord,
    ) -> Result<Arc<WorkflowDefinition>, AppError> {
        let version = self
            .store
            .get_version_by_id(principal.tenant_id(), record.info.version_id)
            .await?
            .ok_or(AppError::NotFound)?;
        self.cache
            .get_or_parse(version.version.id(), &version.document)
            .map_err(|error| {
                tracing::error!(%error, version = %version.version.id(), "a stored version no longer parses");
                AppError::Internal
            })
    }

    pub(super) async fn plan_reset(
        &self,
        principal: &Principal,
        record: &RunRecord,
        source: &ResetSource,
        sequence: u64,
    ) -> Result<ResetPlan, AppError> {
        // a point whose prefix holds a type this version cannot decode has no rule to apply
        if let Some(event_type) = &source.unknown_event_type
            && usize::try_from(sequence).map_or(true, |n| n > source.events.len())
        {
            return Err(AppError::ResetRefused {
                reason: "RESET_RULE_MISSING",
                event_type: Some(event_type.clone()),
            });
        }
        if usize::try_from(sequence).map_or(true, |n| n > RESET_PREFIX_EVENTS) {
            return Err(refused("RESET_PREFIX_TOO_LONG"));
        }
        let definition = self.definition_of(principal, record).await?;
        reset_plan(
            &definition,
            record.info.run_id,
            &Self::reset_journal(source),
            sequence,
        )
        .map_err(|refusal| match refusal {
            ResetRefusal::NotAPoint | ResetRefusal::Blocked(_) => {
                refused("RESET_POINT_NOT_ELIGIBLE")
            }
        })
    }

    /// Creates a run from the prefix of an ended one (`FR-018-037`): a run of the same version and input
    /// whose journal re-emits the events `1..=sequence` of the source, marked `restored`, then records the
    /// reset and goes on from there. The steps that completed in the source and run again need the caller's
    /// acknowledgement.
    ///
    /// A source that has not ended is refused (`RESET_SOURCE_NOT_TERMINAL`), unless the request says
    /// `CANCEL_SOURCE`: then the source is cancelled in the ordinary way and the reset waits, `PENDING_SOURCE`, for
    /// the engine to create the run once the source has ended (`FR-018-038`). The answer to such a request has no
    /// run yet. A key that already made a request is answered from the register whatever the settings are now.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:operate` and `run:start` (and `run:signal` to reapply signals,
    /// `run:update` to reapply Updates); [`AppError::NotAllowed`] without a justification;
    /// [`AppError::ResetRefused`] `RESET_SOURCE_NOT_TERMINAL` for a run that has not ended,
    /// `RESET_CANCEL_SOURCE_NOT_AVAILABLE` for a deployment that does not serve `CANCEL_SOURCE`,
    /// `RESET_REEXECUTION_NOT_ACKNOWLEDGED`, and those of [`Self::reset_preview`];
    /// [`AppError::QuotaExceeded`] when the namespace has the most pending resets it may; the errors of a start.
    pub async fn reset(
        &self,
        principal: &Principal,
        id: RunId,
        request: ResetRequest<'_>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<ResetMade, AppError> {
        principal.authorize(Permission::RunOperate)?;
        principal.authorize(Permission::RunStart)?;
        let reason = request.reason.trim();
        let length = reason.chars().count();
        if !(MIN_JUSTIFICATION..=MAX_JUSTIFICATION).contains(&length)
            || reason.chars().any(char::is_control)
        {
            return Err(AppError::NotAllowed(
                "a justification of 8 to 2000 characters, in plain text, is required",
            ));
        }
        // what the reset delivers again it delivers as the caller: with the right to do it
        if request.reapply.signals() {
            principal.authorize(Permission::RunSignal)?;
        }
        if request.reapply.updates() {
            principal.authorize(Permission::RunUpdate)?;
        }
        let (record, source) = self.reset_candidate(principal, id, ctx).await?;
        // A key that made a request that waits for its source is answered from the register before anything is
        // judged: the source may have ended since, and the point may no longer be eligible, and the first answer
        // stands (D1996). The same key with another request is `IDEMPOTENCY_KEY_REUSED`.
        let receipt = request_receipt(&record, &source, &request, key);
        if let Some(made) = self
            .answer_from_register(principal, id, &receipt, ctx)
            .await?
        {
            return Ok(made);
        }
        if record.status.is_terminal() {
            return self
                .reset_ended(principal, id, record, source, request, key, ctx)
                .await;
        }
        match request.source_policy {
            ResetSourcePolicy::RequireTerminal => Err(refused("RESET_SOURCE_NOT_TERMINAL")),
            ResetSourcePolicy::CancelSource => {
                self.reset_cancelling(principal, id, record, source, request, key, receipt, ctx)
                    .await
            }
        }
    }

    /// The reset of a source that has ended: the run is created at once, `CREATED`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn reset_ended(
        &self,
        principal: &Principal,
        id: RunId,
        record: RunRecord,
        source: ResetSource,
        request: ResetRequest<'_>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<ResetMade, AppError> {
        let plan = self
            .plan_reset(principal, &record, &source, request.sequence)
            .await?;
        let Some(origin) = self.store.rerun_source(principal.tenant_id(), id).await? else {
            return Err(self.missing(principal, id, ctx).await);
        };
        let started = self
            .create_from_prefix(
                principal,
                &record,
                &source,
                &origin,
                &Creation {
                    plan: &plan,
                    key,
                    sequence: request.sequence,
                    reapply: request.reapply,
                    business: request.business,
                    acknowledged: request.acknowledge_reexecution,
                    source_policy: request.source_policy,
                    reason: request.reason.trim(),
                    // a request that was never made is refused where the reapply is off; a retry of one that was
                    // made is answered from the register before it gets here
                    gate_reapply: true,
                    pending: None,
                },
                ctx,
            )
            .await?;
        let run = started.run;
        // the entry of the register was written with the run, in the same transaction, so a retry finds the
        // entry of the first request
        let Some(reset) = self
            .store
            .reset_made(principal.tenant_id(), run.info.run_id)
            .await?
        else {
            tracing::error!(run = %run.info.run_id, "a run created by a reset has no entry in the register");
            return Err(AppError::Internal);
        };
        Ok(ResetMade {
            reset,
            run: Some(run),
        })
    }

    /// The one sequence that makes the run of a reset from the journal of a source that has ended, whether it is made at
    /// once ([`Self::reset_ended`]) or by the pass that completes a reset that waited (`complete_reset`, D2012): the
    /// acknowledgement of the steps that run again, then what is delivered again, then the run. Two copies of it
    /// had drifted in the order of the first two, so the same defect gave different reasons.
    pub(super) async fn create_from_prefix(
        &self,
        principal: &Principal,
        record: &RunRecord,
        source: &ResetSource,
        origin: &RerunSource,
        creation: &Creation<'_>,
        ctx: &RequestContext,
    ) -> Result<Started, AppError> {
        if creation.plan.needs_acknowledgement() && !creation.acknowledged {
            return Err(refused("RESET_REEXECUTION_NOT_ACKNOWLEDGED"));
        }
        // Where the reapply is off nothing is planned: a retry of a request that succeeded is answered from the
        // register, and a request that was never made is refused where it would create the run. What the pass of the
        // engine completes was accepted at the request, and the engine has no setting for it (D1990).
        let planned = if !creation.gate_reapply || self.reset_reapply {
            self.plan_reapply(
                principal,
                record,
                source,
                creation.sequence,
                creation.reapply,
                true,
            )
            .await?
        } else {
            Vec::new()
        };
        let kept = usize::try_from(creation.sequence).map_err(|_| AppError::Internal)?;
        let prefix = RestoredPrefix {
            source: record.info.run_id,
            events: source.events[..kept].to_vec(),
        };
        self.start_with(
            principal,
            &StartRun {
                namespace: &origin.namespace,
                workflow: record.info.workflow_id,
                version: Some(record.info.version_id),
                input: &origin.input,
                correlation_id: record.info.correlation_id.as_deref(),
                deadline: None,
                key: creation.key,
            },
            keep_business_id(record.info.business_id.as_ref(), creation.business),
            Some(Rerun {
                of: record.info.run_id,
                reason: creation.reason,
                business: creation.business,
                reset: Some(ResetStart {
                    prefix: &prefix,
                    reapply: creation.reapply,
                    plan: &planned,
                    reason: creation.reason,
                    business: creation.business,
                    source_policy: creation.source_policy,
                    acknowledged: creation.acknowledged,
                    pending: creation.pending,
                }),
            }),
            None,
            None,
            None,
            ctx,
        )
        .await
    }

    /// Delivers to the run `run_id`, which the reset `reset` is creating in `tx`, what `plan` says, in journal
    /// order (`FR-018-039`), and returns the report the register keeps. Each signal and Update goes in as its
    /// first delivery did (`deliver_signal`, `apply_declared_update`) under a key derived from the reset, so
    /// they are audited, journaled and announced as any other.
    ///
    /// A signal nobody waits for is `REJECTED` and ends the reapply of signals; an Update the declaration
    /// declines, or whose state is no longer at the revision it was applied to, is `REJECTED` and the next
    /// candidate still goes. Anything else (storage, wiring, a refusal that is not the new run's answer) is an
    /// error: the caller drops `tx` and nothing of the reset exists.
    pub(super) async fn reapply(
        &self,
        tx: &mut S::Tx,
        principal: &Principal,
        run_id: RunId,
        reset: ResetId,
        plan: &[PlannedReapply],
        ctx: &RequestContext,
    ) -> Result<Vec<ReapplyEntry>, AppError> {
        let mut report = Vec::with_capacity(plan.len());
        let mut signals_ended = false;
        for item in plan {
            let mut entry = item.entry.clone();
            let key = reapply_key(reset, entry.sequence);
            match (&item.delivery, entry.kind) {
                // settled before the transaction: it will not be tried, and says why
                (None, _) if entry.outcome == ReapplyOutcome::NotReapplied => {}
                (Some(Delivery::Signal { payload }), ReapplyKind::Signal) => {
                    if signals_ended {
                        entry.outcome = ReapplyOutcome::NotReapplied;
                        entry.reason = Some(ReapplyReason::PreviousSignalNotDelivered);
                    } else {
                        let idempotency = Idempotency {
                            scope: format!("run.signal:{run_id}"),
                            key,
                            fingerprint: fingerprint(&[entry.name.as_bytes(), payload]),
                        };
                        match self
                            .deliver_signal(
                                tx,
                                principal,
                                run_id,
                                &entry.name,
                                payload,
                                idempotency,
                                WaitVia::Signal,
                                ctx,
                            )
                            .await
                        {
                            Ok(_) => entry.outcome = ReapplyOutcome::Reapplied,
                            Err(AppError::StateConflict(NOT_WAITING_FOR_SIGNAL)) => {
                                entry.outcome = ReapplyOutcome::Rejected;
                                entry.reason = Some(ReapplyReason::NotWaiting);
                                signals_ended = true;
                            }
                            Err(other) => return Err(other),
                        }
                    }
                }
                (
                    Some(Delivery::Update {
                        before_revision,
                        arguments,
                    }),
                    ReapplyKind::Update,
                ) => {
                    let (Ok(operation), Ok(update_key)) =
                        (ActivityName::parse(&entry.name), UpdateKey::parse(&key))
                    else {
                        tracing::error!("a reapplied Update has a name or a key that is not valid");
                        return Err(AppError::Internal);
                    };
                    let evaluator = self.declared_evaluator()?;
                    let command = self.declared_update(
                        principal,
                        tx.info().namespace_id,
                        run_id,
                        UpdateAtStart {
                            operation: &operation,
                            update_key: &update_key,
                            arguments,
                            // only to the state it was applied to in the source
                            expected_state_revision: Some(*before_revision),
                            batch_job: None,
                        },
                        tx.now(),
                        ctx,
                    );
                    match tx.apply_declared_update(&command, evaluator).await {
                        Ok(applied) => {
                            let receipt = applied.resolution.receipt();
                            let Some((outcome, reason)) =
                                update_answer(receipt.status(), receipt.rejection())
                            else {
                                tracing::error!("a rejected Update has no rejection");
                                return Err(AppError::Internal);
                            };
                            entry.outcome = outcome;
                            entry.reason = reason;
                        }
                        Err(error) => match update_refusal(&error) {
                            Some(reason) => {
                                entry.outcome = ReapplyOutcome::Rejected;
                                entry.reason = Some(reason);
                            }
                            None => return Err(AppError::DeclaredUpdate(error)),
                        },
                    }
                }
                _ => {
                    tracing::error!("a reapply entry does not match what it is delivered with");
                    return Err(AppError::Internal);
                }
            }
            report.push(entry);
        }
        Ok(report)
    }

    /// The resets made from a run, newest first (at most 200).
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `run:read`; [`AppError::NotFound`] for a run the caller cannot see.
    pub async fn resets(
        &self,
        principal: &Principal,
        id: RunId,
        ctx: &RequestContext,
    ) -> Result<Vec<ResetRecord>, AppError> {
        principal.authorize(Permission::RunRead)?;
        let record = self.get(principal, id, ctx).await?;
        Ok(self
            .store
            .run_resets(principal.tenant_id(), record.info.run_id)
            .await?)
    }

    /// One reset made from a run.
    ///
    /// # Errors
    /// As [`Self::resets`]; [`AppError::NotFound`] for a reset that does not exist or that another run made.
    pub async fn reset_record(
        &self,
        principal: &Principal,
        id: RunId,
        reset: ResetId,
        ctx: &RequestContext,
    ) -> Result<ResetRecord, AppError> {
        principal.authorize(Permission::RunRead)?;
        let record = self.get(principal, id, ctx).await?;
        self.store
            .run_reset(principal.tenant_id(), record.info.run_id, reset)
            .await?
            .ok_or(AppError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::execution::RunEvent;
    use sagat_domain::workflow::NodeId;

    fn row(sequence: u64, canonical: &[u8]) -> SourceEvent {
        SourceEvent {
            sequence,
            event: AddressedRunEvent::root(RunEvent::RunStarted {
                entrypoint: NodeId::parse("a").expect("a node"),
            }),
            event_type: "run.started".into(),
            node: Some("a".into()),
            attempt: None,
            detail: serde_json::json!({}),
            actor: "system".into(),
            trace_id: None,
            occurred_at: OffsetDateTime::UNIX_EPOCH,
            payload_digest: None,
            canonical: canonical.to_vec(),
        }
    }

    #[test]
    fn the_digest_depends_on_every_row_in_order_and_on_where_a_row_ends() {
        let ab = prefix_digest(&[row(1, b"a"), row(2, b"b")]);
        // the same rows, the same digest
        assert_eq!(ab, prefix_digest(&[row(1, b"a"), row(2, b"b")]));
        // another order, another digest
        assert_ne!(ab, prefix_digest(&[row(1, b"b"), row(2, b"a")]));
        // a shorter prefix is another prefix
        assert_ne!(ab, prefix_digest(&[row(1, b"a")]));
        // where a row ends counts: "ab" then nothing is not "a" then "b"
        assert_ne!(ab, prefix_digest(&[row(1, b"ab")]));
        assert_ne!(
            prefix_digest(&[row(1, b"a"), row(2, b"")]),
            prefix_digest(&[row(1, b"a")])
        );
    }

    #[test]
    fn a_prefix_knows_its_last_sequence_and_digest() {
        let prefix = RestoredPrefix {
            source: RunId::generate(),
            events: vec![row(1, b"a"), row(2, b"b")],
        };
        assert_eq!(prefix.sequence(), 2);
        assert_eq!(prefix.digest(), prefix_digest(&prefix.events));
        assert_eq!(
            RestoredPrefix {
                source: RunId::generate(),
                events: Vec::new()
            }
            .sequence(),
            0
        );
    }

    #[test]
    fn a_point_is_named_by_the_event_that_ends_its_prefix() {
        for (event_type, kind, name) in [
            ("run.started", ResetPointKind::RunStarted, "RUN_STARTED"),
            (
                "step.completed",
                ResetPointKind::StepCompleted,
                "STEP_COMPLETED",
            ),
            (
                "child.finished",
                ResetPointKind::ChildFinished,
                "CHILD_FINISHED",
            ),
            (
                "collection.finished",
                ResetPointKind::CollectionFinished,
                "COLLECTION_FINISHED",
            ),
            (
                "loop.finished",
                ResetPointKind::LoopFinished,
                "LOOP_FINISHED",
            ),
        ] {
            assert_eq!(ResetPointKind::of_event_type(event_type), Some(kind));
            assert_eq!(kind.as_str(), name);
        }
        // an event that cannot end a prefix names no point
        for other in ["step.started", "run.succeeded", "timer.fired", ""] {
            assert_eq!(ResetPointKind::of_event_type(other), None, "{other}");
        }
    }

    #[test]
    fn a_state_of_the_register_has_one_name_each_and_an_unknown_name_is_none() {
        for (state, name) in [
            (ResetState::PendingSource, "PENDING_SOURCE"),
            (ResetState::Created, "CREATED"),
            (ResetState::Failed, "FAILED"),
        ] {
            assert_eq!(state.as_str(), name);
            assert_eq!(ResetState::parse(name), Some(state));
        }
        assert_eq!(ResetState::parse("created"), None);
        assert_eq!(ResetState::parse("CANCELLED"), None);
    }
}
