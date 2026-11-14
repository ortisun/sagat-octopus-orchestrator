//! The reset of a run that has not ended (feature 018, US06, `FR-018-038`, `T018-057`): `CANCEL_SOURCE`.
//!
//! The request cancels the source in the ordinary way (no terminate) and writes an entry of the register in state
//! `PENDING_SOURCE`, with the receipt of the request and the audit record, in one transaction that holds the row of
//! the source. A fenced pass of the engine ([`crate::reset_completion`]) creates the run once the source has ended,
//! if the point is still eligible, in one transaction that also completes the entry: `CREATED` with the new run,
//! or, when something a fresh reset checks no longer holds, `FAILED` with a closed reason. The source never reopens.

use sagat_domain::execution::{ResetId, ResetPlan, ResetReapply};
use sagat_domain::quota::QuotaResource;

use super::*;
use crate::reset_completion::{Completion, ResetCompletions};
use crate::schedules::ShardFence;

/// Why the pass that completes a reset finds it completed already: another engine did it first, or it failed.
pub const RESET_ALREADY_COMPLETED: &str = "the reset has already been completed";

/// What a reset does with a source that has not ended (`sourcePolicy` of the contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResetSourcePolicy {
    /// An open source is refused: `RESET_SOURCE_NOT_TERMINAL`.
    #[default]
    RequireTerminal,
    /// The source is cancelled in the ordinary way and the run is created once it has ended.
    CancelSource,
}

impl ResetSourcePolicy {
    /// The closed name the contract, the register and the fingerprint of a request use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequireTerminal => "REQUIRE_TERMINAL",
            Self::CancelSource => "CANCEL_SOURCE",
        }
    }

    /// The policy a stored name stands for; `None` for a name this version does not know.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "REQUIRE_TERMINAL" => Some(Self::RequireTerminal),
            "CANCEL_SOURCE" => Some(Self::CancelSource),
            _ => None,
        }
    }
}

/// What the request of a reset that waits for its source writes, in the transaction that cancels the source.
#[derive(Clone, Copy)]
pub struct PendingRegistration<'a> {
    pub id: ResetId,
    pub source: RunId,
    pub sequence: u64,
    /// The digest of the prefix `1..=sequence` as it stands now, which the completion must find again.
    pub prefix_digest: sagat_domain::workflow::Digest,
    pub reapply: ResetReapply,
    pub business: RerunBusinessId,
    pub acknowledged: bool,
    /// The subject that asked.
    pub actor: &'a str,
    /// The justification, in plain text; the register keeps it as content of the tenant.
    pub reason: &'a str,
    /// The most resets the namespace may have waiting, judged where the entry is written.
    pub limit: u64,
    /// The audit record of the request (`run.reset`), committed with the entry.
    pub audit: &'a AuditRecord,
    /// The receipt of the request: its result is the id of the entry.
    pub receipt: &'a Idempotency,
}

impl std::fmt::Debug for PendingRegistration<'_> {
    /// The subject that asked, the justification, the audit record and the key of the receipt are not printed.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingRegistration")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("sequence", &self.sequence)
            .field("reapply", &self.reapply)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

/// A reset that waits for its source to end, as the pass that completes it reads it. Its `Debug` hides the
/// justification of the caller.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingReset {
    pub id: ResetId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub source: RunId,
    pub sequence: u64,
    /// The digest of the prefix when the request was made.
    pub prefix_digest: Vec<u8>,
    pub reapply: ResetReapply,
    pub business: RerunBusinessId,
    pub acknowledged: bool,
    /// The subject that asked, whose authority is read again.
    pub actor: String,
    pub reason: String,
    pub created_at: OffsetDateTime,
}

impl std::fmt::Debug for PendingReset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingReset")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("sequence", &self.sequence)
            .field("reapply", &self.reapply)
            .finish_non_exhaustive()
    }
}

impl PendingReset {
    /// What the requester must hold for the reset to be made: what a fresh reset asks, and the right to deliver
    /// again what the reapply delivers.
    #[must_use]
    pub fn permissions(&self) -> Vec<Permission> {
        let mut needed = vec![Permission::RunOperate, Permission::RunStart];
        if self.reapply.signals() {
            needed.push(Permission::RunSignal);
        }
        if self.reapply.updates() {
            needed.push(Permission::RunUpdate);
        }
        needed
    }
}

/// The completion of a pending reset inside the transaction that creates its run: the entry, the fence of the shard
/// that owns it and the authority of the requester, all looked at where the run is admitted.
#[derive(Clone)]
pub struct PendingAdmission {
    pub reset: ResetId,
    /// The lease the engine acts under; `None` for a caller that holds none (a test).
    pub fence: Option<ShardFence>,
    /// The subject that asked, whose membership is read again under its lock.
    pub actor: String,
    /// What the member must hold.
    pub permissions: Vec<Permission>,
}

impl std::fmt::Debug for PendingAdmission {
    /// The subject that asked and the owner of the lease are not printed.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingAdmission")
            .field("reset", &self.reset)
            .field("fenced", &self.fence.is_some())
            .field("permissions", &self.permissions.len())
            .finish_non_exhaustive()
    }
}

/// What makes the run of a reset from the journal of a source that has ended: the same facts whether the run is made
/// at once or by the pass that completes a reset that waited (`RunService::create_from_prefix`, D2012).
pub(super) struct Creation<'a> {
    /// The point as the journal of the source says it is: what is restored and what runs again.
    pub plan: &'a ResetPlan,
    /// The key of the creation's receipt: the caller's for a reset made at once, `reset:{id}` for a completion.
    pub key: &'a str,
    pub sequence: u64,
    pub reapply: ResetReapply,
    pub business: RerunBusinessId,
    pub acknowledged: bool,
    pub source_policy: ResetSourcePolicy,
    /// The justification, trimmed.
    pub reason: &'a str,
    /// Whether the setting of the deployment for the reapply is asked: it is for a reset made at once and not for a
    /// completion, which was accepted at the request and runs in a process that has no such setting (D1990).
    pub gate_reapply: bool,
    pub pending: Option<&'a PendingAdmission>,
}

/// The receipt of a reset request: scope per namespace, the key of the caller, and a fingerprint of what was asked.
/// The policy is part of it, so that a key used for a request that waits is not another policy's.
pub(super) fn request_receipt(
    record: &RunRecord,
    source: &ResetSource,
    request: &ResetRequest<'_>,
    key: &str,
) -> Idempotency {
    // the justification as it is judged, audited and kept: trimmed (D2010)
    let reason = request.reason.trim();
    // the prefix as it stands; a sequence outside the journal has none, and no accepted request ever had that
    let digest = usize::try_from(request.sequence)
        .ok()
        .filter(|kept| (1..=source.events.len()).contains(kept))
        .map(|kept| prefix_digest(&source.events[..kept]));
    // one byte: the choice of the business policy (no word of the statements is written in Rust, the audit of the
    // privileges of the runtime role reads every literal as a possible statement)
    let drops = [u8::from(matches!(request.business, RerunBusinessId::Drop))];
    Idempotency {
        scope: format!("run.reset:{}", record.info.namespace_id),
        key: key.to_owned(),
        fingerprint: fingerprint(&[
            b"run.reset.request",
            request.source_policy.as_str().as_bytes(),
            record.info.run_id.to_string().as_bytes(),
            &request.sequence.to_be_bytes(),
            digest.as_ref().map_or(&[][..], |d| d.as_bytes().as_slice()),
            request.reapply.as_str().as_bytes(),
            &drops,
            &[u8::from(request.acknowledge_reexecution)],
            reason.as_bytes(),
        ]),
    }
}

impl<S, C, E> RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    /// Whether this deployment lets a reset cancel an open source (`SAGAT_RESET_CANCEL_SOURCE_ENABLED`). The
    /// request asks it where the entry would be written, after the receipt has been looked at (D1996).
    #[must_use]
    pub const fn reset_cancel_source_enabled(&self) -> bool {
        self.reset_cancel_source
    }

    /// The answer to a request that was made already: the entry of the register, and the run when it has one.
    pub(super) async fn answer_from_register(
        &self,
        principal: &Principal,
        source: RunId,
        receipt: &Idempotency,
        ctx: &RequestContext,
    ) -> Result<Option<ResetMade>, AppError> {
        let tenant = principal.tenant_id();
        let Some(stored) = self.store.reset_receipt(tenant, receipt).await? else {
            return Ok(None);
        };
        self.made_from_receipt(principal, source, &stored, ctx)
            .await
            .map(Some)
    }

    async fn made_from_receipt(
        &self,
        principal: &Principal,
        source: RunId,
        stored: &str,
        ctx: &RequestContext,
    ) -> Result<ResetMade, AppError> {
        let tenant = principal.tenant_id();
        let Ok(id) = stored.parse::<uuid::Uuid>().map(ResetId::new) else {
            tracing::error!("the receipt of a reset request does not name an entry");
            return Err(AppError::Internal);
        };
        let Some(reset) = self.store.run_reset(tenant, source, id).await? else {
            tracing::error!("the receipt of a reset request names an entry that is not there");
            return Err(AppError::Internal);
        };
        let run = match reset.new_run {
            Some(run) => Some(self.get(principal, run, ctx).await?),
            None => None,
        };
        Ok(ResetMade { reset, run })
    }

    /// The request of a reset of a source that has not ended and is to be cancelled (`CANCEL_SOURCE`): one
    /// transaction that holds the source's row writes the entry and the receipt, cancels the source in the ordinary
    /// way and audits the request. A source that ended while the request waited for the row is an ordinary reset.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn reset_cancelling(
        &self,
        principal: &Principal,
        id: RunId,
        record: RunRecord,
        source: ResetSource,
        request: ResetRequest<'_>,
        key: &str,
        receipt: Idempotency,
        ctx: &RequestContext,
    ) -> Result<ResetMade, AppError> {
        // Asked here, where an entry would be written, and not before the receipt was looked at: a retry of a
        // request that was accepted is answered whatever the setting is now (D1996).
        if !self.reset_cancel_source {
            return Err(AppError::ResetRefused {
                reason: "RESET_CANCEL_SOURCE_NOT_AVAILABLE",
                event_type: None,
            });
        }
        self.reapply_available(request.reapply)?;
        // what a fresh reset checks, on the journal as it is now; the completion checks it again on the final one
        let plan = self
            .plan_reset(principal, &record, &source, request.sequence)
            .await?;
        if plan.needs_acknowledgement() && !request.acknowledge_reexecution {
            return Err(AppError::ResetRefused {
                reason: "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
                event_type: None,
            });
        }
        self.plan_reapply(
            principal,
            &record,
            &source,
            request.sequence,
            request.reapply,
            false,
        )
        .await?;
        let kept = usize::try_from(request.sequence).map_err(|_| AppError::Internal)?;
        let digest = prefix_digest(&source.events[..kept]);
        let tenant = principal.tenant_id();
        let limit = self
            .store
            .effective_limit(
                tenant,
                Some(record.info.namespace_id),
                QuotaResource::PendingResets,
            )
            .await?;
        let reason = request.reason.trim();
        let reset = ResetId::generate();
        let now = self.clock.now();
        // The justification is audited with the request, as the contract says; nothing else of the caller's content.
        let requested = AuditRecord::by(
            principal,
            AuditAction::RunReset,
            ResourceType::Run,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "reset {reset} requested at {}{}: {reason}",
            request.sequence,
            if request.reapply == ResetReapply::None {
                String::new()
            } else {
                format!(" reapply={}", request.reapply.as_str())
            }
        ));
        let Some(mut tx) = self.store.begin(tenant, id, Lock::Wait).await? else {
            return Err(self.missing(principal, id, ctx).await);
        };
        // Two equal requests meet at the row of the source: the second finds the receipt of the first.
        if let Some(stored) = tx.replay(&receipt).await? {
            drop(tx);
            return self.made_from_receipt(principal, id, &stored, ctx).await;
        }
        // The source ended while the request waited for its row: it is a reset of an ended run, made at once. The
        // journal read before is not the final one, so the candidate is read again.
        if tx.parts().1.status().is_terminal() {
            drop(tx);
            let (record, source) = self.reset_candidate(principal, id, ctx).await?;
            return self
                .reset_ended(principal, id, record, source, request, key, ctx)
                .await;
        }
        tx.register_pending_reset(&PendingRegistration {
            id: reset,
            source: id,
            sequence: request.sequence,
            prefix_digest: digest,
            reapply: request.reapply,
            business: request.business,
            acknowledged: request.acknowledge_reexecution,
            actor: principal.subject().as_str(),
            reason,
            limit,
            audit: &requested,
            receipt: &receipt,
        })
        .await?;
        // The ordinary cancellation, as `cancelRun` does it: no terminate. A source that is stopping already (it was
        // cancelled, or a saga is undoing it) needs no second request; the entry waits for it to end all the same.
        if tx.parts().1.stop_reason().is_none() {
            let mut effects = Effects::by(
                Actor::Principal(principal.subject().as_str().to_owned()),
                ctx.trace_id.clone(),
            );
            effects.audit = Some(
                AuditRecord::by(
                    principal,
                    AuditAction::RunCancelRequested,
                    ResourceType::Run,
                    id.to_string(),
                    ctx.trace_id.clone(),
                    now,
                )
                .with_reason(format!("cancelled for the reset {reset}")),
            );
            tx.latch_chain_cancel().await?;
            self.engine
                .run(&mut tx, effects, |def, state, now| {
                    let progress =
                        execution::stop_run(def, state, StopReason::Cancelled, None, now)?;
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error)?;
        }
        tx.commit().await?;
        let Some(entry) = self.store.run_reset(tenant, id, reset).await? else {
            tracing::error!("a reset request that committed has no entry in the register");
            return Err(AppError::Internal);
        };
        Ok(ResetMade {
            reset: entry,
            run: None,
        })
    }

    /// Creates the run of a pending reset whose source has ended, in the transaction that completes the entry
    /// (`FR-018-038`): the point is looked at again on the final journal of the source, and everything a fresh reset
    /// checks is checked with the authority the requester holds now. Nothing is created while the source is open.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] when the requester no longer holds what the reset needs;
    /// [`AppError::ResetRefused`] when the point is no longer eligible (`RESET_POINT_NOT_ELIGIBLE`), steps run again
    /// that were not acknowledged, or there is more to deliver again than a reset may; the refusals of a start (a
    /// quota, a business identifier another execution holds);
    /// [`AppError::StateConflict`] when the lease is not held or the entry was completed first.
    pub async fn complete_reset(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        let permissions = entry.permissions();
        for permission in &permissions {
            principal.authorize(*permission)?;
        }
        // The source and the things it names were read at the request, and nothing deletes them: a `NotFound` here is
        // not an answer of the reset but a defect to look at, and it leaves the entry pending (D2007). It is not a
        // lost authority either: no reason is invented for it.
        let (record, source) = self
            .reset_candidate(principal, entry.source, ctx)
            .await
            .map_err(|error| anomaly(entry, "the source", error))?;
        if !record.status.is_terminal() {
            return Ok(Completion::SourceOpen);
        }
        let plan = self
            .plan_reset(principal, &record, &source, entry.sequence)
            .await
            .map_err(|error| anomaly(entry, "the plan of the point", error))?;
        let kept = usize::try_from(entry.sequence).map_err(|_| AppError::Internal)?;
        // the prefix is immutable: another one means this is not the point that was asked for
        if prefix_digest(&source.events[..kept]).as_bytes().as_slice()
            != entry.prefix_digest.as_slice()
        {
            return Err(AppError::ResetRefused {
                reason: "RESET_POINT_NOT_ELIGIBLE",
                event_type: None,
            });
        }
        let Some(origin) = self
            .store
            .rerun_source(principal.tenant_id(), entry.source)
            .await
            .map_err(|error| store_failure(true, error))?
        else {
            return Err(anomaly(
                entry,
                "the input of the source",
                AppError::NotFound,
            ));
        };
        let admission = PendingAdmission {
            reset: entry.id,
            fence,
            actor: entry.actor.clone(),
            permissions,
        };
        let key = format!("reset:{}", entry.id);
        let started = self
            .create_from_prefix(
                principal,
                &record,
                &source,
                &origin,
                &Creation {
                    plan: &plan,
                    key: &key,
                    sequence: entry.sequence,
                    reapply: entry.reapply,
                    business: entry.business,
                    acknowledged: entry.acknowledged,
                    source_policy: ResetSourcePolicy::CancelSource,
                    reason: &entry.reason,
                    gate_reapply: false,
                    pending: Some(&admission),
                },
                ctx,
            )
            .await?;
        Ok(Completion::Created(started.run.info.run_id))
    }
}

/// A `NotFound` met while completing a reset that waited, outside the creation of the run, is an anomaly: it leaves
/// the entry pending to be tried again, with a line that names the entry (ids are not content), and it is not turned
/// into a reason of the register.
fn anomaly(entry: &PendingReset, stage: &'static str, error: AppError) -> AppError {
    if matches!(error, AppError::NotFound) {
        tracing::warn!(
            tenant_id = %entry.tenant_id,
            reset_id = %entry.id,
            stage,
            "a pending reset names something that is not there: it stays pending"
        );
        return AppError::Unavailable;
    }
    error
}

impl<S, C, E> ResetCompletions for RunService<S, C, E>
where
    S: RunStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
{
    async fn complete(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        self.complete_reset(principal, entry, fence, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(reapply: ResetReapply) -> PendingReset {
        PendingReset {
            id: ResetId::generate(),
            tenant_id: TenantId::new(uuid::Uuid::nil()),
            namespace_id: NamespaceId::new(uuid::Uuid::nil()),
            source: RunId::generate(),
            sequence: 1,
            prefix_digest: vec![0; 32],
            reapply,
            business: RerunBusinessId::Keep,
            acknowledged: true,
            actor: "alice".to_owned(),
            reason: "a reason of the requester".to_owned(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// What the member who asked must hold when the run is made: what a reset asks, and the right to deliver again what
    /// its `reapply` delivers (the same checks `reset()` makes at the request).
    #[test]
    fn the_requester_must_hold_what_a_fresh_reset_asks_and_what_its_reapply_delivers() {
        let base = [Permission::RunOperate, Permission::RunStart];
        for (reapply, extra) in [
            (ResetReapply::None, vec![]),
            (ResetReapply::Signals, vec![Permission::RunSignal]),
            (ResetReapply::Updates, vec![Permission::RunUpdate]),
            (
                ResetReapply::All,
                vec![Permission::RunSignal, Permission::RunUpdate],
            ),
        ] {
            let mut expected = base.to_vec();
            expected.extend(extra);
            assert_eq!(
                entry(reapply).permissions(),
                expected,
                "{}",
                reapply.as_str()
            );
        }
    }

    #[test]
    fn the_policy_of_a_source_has_one_name_each_and_an_unknown_name_is_none() {
        for (policy, name) in [
            (ResetSourcePolicy::RequireTerminal, "REQUIRE_TERMINAL"),
            (ResetSourcePolicy::CancelSource, "CANCEL_SOURCE"),
        ] {
            assert_eq!(policy.as_str(), name);
            assert_eq!(ResetSourcePolicy::parse(name), Some(policy));
        }
        assert_eq!(ResetSourcePolicy::parse("cancel_source"), None);
        assert_eq!(ResetSourcePolicy::parse("SOMETIMES"), None);
        assert_eq!(
            ResetSourcePolicy::default(),
            ResetSourcePolicy::RequireTerminal
        );
    }

    #[test]
    fn the_debug_of_a_pending_reset_never_prints_what_the_requester_wrote() {
        let printed = format!("{:?}", entry(ResetReapply::None));
        assert!(!printed.contains("a reason of the requester"), "{printed}");
        assert!(!printed.contains("alice"), "{printed}");
    }

    // ---------------------------------------------------------------- what the store failed with (D2007)

    #[test]
    fn a_failure_that_is_not_an_answer_of_the_admission_is_unavailable_for_the_pass_and_converted_for_a_client()
     {
        let not_answers: [fn() -> StoreError; 4] = [
            || StoreError::DatabaseUnavailable,
            || StoreError::DatabaseLimit,
            || StoreError::MissingReference,
            || StoreError::RevealGrantRequired,
        ];
        for make in not_answers {
            // the pass: a failure to try again, which never ends the entry
            assert!(
                matches!(store_failure(true, make()), AppError::Unavailable),
                "{:?}",
                make()
            );
        }
        // every other caller is told what it always was
        assert!(matches!(
            store_failure(false, StoreError::DatabaseUnavailable),
            AppError::NotFound
        ));
        assert!(matches!(
            store_failure(false, StoreError::MissingReference),
            AppError::NotFound
        ));
        assert!(matches!(
            store_failure(false, StoreError::DatabaseLimit),
            AppError::NotAllowed(_)
        ));
        assert!(matches!(
            store_failure(false, StoreError::RevealGrantRequired),
            AppError::Forbidden { .. }
        ));
    }

    #[test]
    fn the_answers_of_the_admission_are_converted_as_ever_even_for_the_pass() {
        assert!(matches!(
            store_failure(
                true,
                StoreError::QuotaExceeded {
                    what: "active runs"
                }
            ),
            AppError::QuotaExceeded("active runs")
        ));
        assert!(matches!(
            store_failure(
                true,
                StoreError::BusinessIdHeld {
                    reason: "BUSINESS_ID_IN_USE",
                    holder: uuid::Uuid::nil(),
                    reusable_after: None
                }
            ),
            AppError::BusinessIdConflict { .. }
        ));
        assert!(matches!(
            store_failure(true, StoreError::StateConflict("a conflict")),
            AppError::StateConflict("a conflict")
        ));
        assert!(matches!(
            store_failure(
                true,
                StoreError::Unavailable {
                    detail: String::new()
                }
            ),
            AppError::Unavailable
        ));
        assert!(matches!(
            store_failure(
                true,
                StoreError::Internal {
                    detail: String::new()
                }
            ),
            AppError::Internal
        ));
    }

    // ------------------------------------------------------------------------ nothing of the caller is printed

    #[test]
    fn the_debug_of_what_a_request_and_a_completion_carry_prints_none_of_it() {
        use sagat_domain::identity::{Role, Subject};
        let secrets = [
            "the justification of the caller",
            "alice-secret",
            "secret-key-0001",
            "SECRET-SCOPE",
            "owner-secret",
        ];
        let principal = Principal::new(
            TenantId::new(uuid::Uuid::nil()),
            Subject::parse("alice-secret").expect("a subject"),
            Role::Operator,
        );
        let audit = AuditRecord::by(
            &principal,
            AuditAction::RunReset,
            ResourceType::Run,
            "the-run".to_owned(),
            None,
            OffsetDateTime::UNIX_EPOCH,
        )
        .with_reason("the justification of the caller");
        let receipt = Idempotency {
            scope: "run.reset:SECRET-SCOPE".to_owned(),
            key: "secret-key-0001".to_owned(),
            fingerprint: vec![1, 2, 3],
        };
        let registration = PendingRegistration {
            id: ResetId::generate(),
            source: RunId::generate(),
            sequence: 4,
            prefix_digest: sagat_domain::workflow::Digest::new([7; 32]),
            reapply: ResetReapply::All,
            business: RerunBusinessId::Keep,
            acknowledged: true,
            actor: "alice-secret",
            reason: "the justification of the caller",
            limit: 16,
            audit: &audit,
            receipt: &receipt,
        };
        let admission = PendingAdmission {
            reset: ResetId::generate(),
            fence: Some(ShardFence {
                owner: "owner-secret".to_owned(),
                lease: crate::shards::Lease {
                    shard: sagat_domain::sharding::Shard::of(uuid::Uuid::nil()),
                    generation: 1,
                },
            }),
            actor: "alice-secret".to_owned(),
            permissions: vec![Permission::RunOperate],
        };
        let prefix = RestoredPrefix {
            source: RunId::generate(),
            events: Vec::new(),
        };
        let start = ResetStart {
            prefix: &prefix,
            reapply: ResetReapply::None,
            plan: &[],
            reason: "the justification of the caller",
            business: RerunBusinessId::Keep,
            source_policy: ResetSourcePolicy::CancelSource,
            acknowledged: true,
            pending: Some(&admission),
        };
        let registered = ResetRegistration {
            id: ResetId::generate(),
            source: RunId::generate(),
            sequence: 4,
            prefix_digest: sagat_domain::workflow::Digest::new([7; 32]),
            reapply: ResetReapply::None,
            business: RerunBusinessId::Keep,
            source_policy: ResetSourcePolicy::CancelSource,
            acknowledged: true,
            actor: "alice-secret",
            reason: "the justification of the caller",
            report: &[],
            completes_pending: false,
        };
        for (name, printed) in [
            ("PendingRegistration", format!("{registration:?}")),
            ("PendingAdmission", format!("{admission:?}")),
            ("ResetStart", format!("{start:?}")),
            ("ResetRegistration", format!("{registered:?}")),
        ] {
            assert!(printed.starts_with(name), "{printed}");
            for secret in secrets {
                assert!(
                    !printed.contains(secret),
                    "{name} prints {secret}: {printed}"
                );
            }
        }
    }
}
