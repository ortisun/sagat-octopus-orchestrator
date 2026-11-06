//! Reset to a point (feature 018, US06): the journal of a source run as a reset reads it, and the prefix a reset
//! re-emits under the new run. The rules of which points exist are the domain's
//! (`sagat_domain::execution::reset`); this module is what the store and the service exchange, and the use
//! cases of the points, the preview and the reset.

use sagat_domain::execution::{
    AddressedRunEvent, ResetPlan, ResetPoint, ResetReapply, ResetRefusal, RunId, reset_plan,
    reset_points,
};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;

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
}

fn refused(reason: &'static str) -> AppError {
    AppError::ResetRefused {
        reason,
        event_type: None,
    }
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
    async fn reset_candidate(
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
    ) -> Result<Vec<ResetPoint>, AppError> {
        let (_, source) = self.reset_candidate(principal, id, ctx).await?;
        let points = reset_points(id, &Self::reset_journal(&source)).map_err(|error| {
            tracing::error!(%error, run = %id, "the journal of a run does not replay");
            AppError::Internal
        })?;
        Ok(points.eligible)
    }

    /// What a reset to `sequence` would restore, run again and reapply, changing nothing (`FR-018-036`).
    ///
    /// # Errors
    /// As [`Self::reset_points`]; [`AppError::ResetRefused`] `RESET_POINT_NOT_ELIGIBLE`,
    /// `RESET_RULE_MISSING` (with the type) or `RESET_PREFIX_TOO_LONG`.
    pub async fn reset_preview(
        &self,
        principal: &Principal,
        id: RunId,
        sequence: u64,
        ctx: &RequestContext,
    ) -> Result<ResetPlan, AppError> {
        let (record, source) = self.reset_candidate(principal, id, ctx).await?;
        self.plan_reset(principal, &record, &source, sequence).await
    }

    async fn plan_reset(
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
        let version = self
            .store
            .get_version_by_id(principal.tenant_id(), record.info.version_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let definition = self
            .cache
            .get_or_parse(version.version.id(), &version.document)
            .map_err(|error| {
                tracing::error!(%error, version = %version.version.id(), "a stored version no longer parses");
                AppError::Internal
            })?;
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
    /// # Errors
    /// [`AppError::Forbidden`] without `run:operate` and `run:start`; [`AppError::NotAllowed`] without a
    /// justification or for a reapply this version does not do; [`AppError::ResetRefused`]
    /// `RESET_SOURCE_NOT_TERMINAL` for a run that has not ended, `RESET_REEXECUTION_NOT_ACKNOWLEDGED`, and
    /// those of [`Self::reset_preview`]; the errors of a start.
    pub async fn reset(
        &self,
        principal: &Principal,
        id: RunId,
        request: ResetRequest<'_>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<RunRecord, AppError> {
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
        if request.reapply != ResetReapply::None {
            return Err(AppError::NotAllowed(
                "delivering signals and updates again after a reset is not available in this version",
            ));
        }
        let (record, source) = self.reset_candidate(principal, id, ctx).await?;
        if !record.status.is_terminal() {
            return Err(refused("RESET_SOURCE_NOT_TERMINAL"));
        }
        let plan = self
            .plan_reset(principal, &record, &source, request.sequence)
            .await?;
        if plan.needs_acknowledgement() && !request.acknowledge_reexecution {
            return Err(refused("RESET_REEXECUTION_NOT_ACKNOWLEDGED"));
        }
        let kept = usize::try_from(request.sequence).map_err(|_| AppError::Internal)?;
        let prefix = RestoredPrefix {
            source: id,
            events: source.events[..kept].to_vec(),
        };
        let Some(origin) = self.store.rerun_source(principal.tenant_id(), id).await? else {
            return Err(self.missing(principal, id, ctx).await);
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
                key,
            },
            keep_business_id(record.info.business_id.as_ref(), request.business),
            Some(Rerun {
                of: id,
                reason,
                business: request.business,
                reset: Some(ResetStart {
                    prefix: &prefix,
                    reapply: request.reapply,
                }),
            }),
            None,
            None,
            None,
            ctx,
        )
        .await
        .map(|started| started.run)
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
}
