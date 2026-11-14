//! `CANCEL_SOURCE`, the reset of a run that has not ended (`T018-057`, `FR-018-038`, `FR-018-040`, `SC-018-012`):
//! the request cancels the source in the ordinary way and leaves an entry `PENDING_SOURCE`; the pass of the engine
//! creates the run once the source has ended, if the point is still eligible, and fails the entry with a closed
//! reason when it is not. Real PostgreSQL, the real run service and the real completer; the races and the crashes
//! are in the fault suite (`tests/fault/schedules-bulk-operations/cancel_source.rs`).
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::identity::RequestContext;
use sagat_application::operations::MonitorStore as _;
use sagat_application::reset_completion::{
    Completed, Completion, DueReset, PendingResetStore as _, ResetCompleter, ResetCompletions,
};
use sagat_application::runs::{
    PendingReset, RESET_ALREADY_COMPLETED, RerunBusinessId, ResetMade, ResetRequest,
    ResetSourcePolicy, ResetState, RunService, RunStore as _,
};
use sagat_application::schedules::{AUTHORITY_REVOKED, SHARD_LEASE_LOST, ShardFence};
use sagat_application::worker::ClaimedTask;
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::execution::{ResetId, ResetReapply, RunId, RunStatus};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, ResetPass, ResetSettings};
use serde_json::json;
use uuid::Uuid;

use crate::business_ids::{business, holder_of, key, start};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{Session, World, activity, ctx, document, with};

pub(crate) const WHY: &str = "the third step used a wrong price list";

pub(crate) struct Completions(pub(crate) Arc<World>);

impl ResetCompletions for Completions {
    async fn complete(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        self.0
            .runs
            .complete_reset(principal, entry, fence, ctx)
            .await
    }
}

pub(crate) type Completer = ResetCompleter<PgRunStore, Completions, PgIdentityStore, SystemClock>;

pub(crate) fn completer(f: &Fixture) -> Completer {
    ResetCompleter::new(
        f.w.store.clone(),
        Completions(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        SystemClock,
    )
}

/// A source that has not ended: `a` is done, `b` is leased to a worker and `c` waits.
pub(crate) struct Open {
    pub(crate) run: RunId,
    pub(crate) session: Session,
    pub(crate) b: ClaimedTask,
    /// The sequence of the completion of `a`: the point the tests reset to.
    pub(crate) after_a: u64,
}

pub(crate) async fn open_chain(f: &Fixture) -> Open {
    open_chain_tagged(f, "").await
}

/// As [`open_chain`], with activities of their own so that several sources can be open at once.
pub(crate) async fn open_chain_tagged(f: &Fixture, tag: &str) -> Open {
    let (a, b, c) = (format!("a{tag}"), format!("b{tag}"), format!("c{tag}"));
    let workflow =
        f.w.publish(document(
            &format!("cancel-source-{}", Uuid::now_v7()),
            &a,
            vec![
                with(activity(&a), "next", json!(b)),
                with(activity(&b), "next", json!(c)),
                activity(&c),
            ],
        ))
        .await
        .0;
    let run = f.w.start(workflow, &json!({"k": 1})).await;
    let names = [format!("act.{a}"), format!("act.{b}"), format!("act.{c}")];
    let session =
        f.w.session(&names.iter().map(String::as_str).collect::<Vec<_>>())
            .await;
    let first = f.w.one_task_of(&session, &names[0]).await;
    f.w.complete(&session, &first, &json!({"done": "a"}))
        .await
        .expect("a completes");
    let second = f.w.one_task_of(&session, &names[1]).await;
    let after_a = completed_steps(f, run).await[0];
    assert!(!f.w.run_status(run).await.is_terminal());
    Open {
        run,
        session,
        b: second,
        after_a,
    }
}

/// The worker finishes the step that was in flight when the source was cancelled: the source ends.
pub(crate) async fn end(f: &Fixture, open: &Open) {
    f.w.complete(&open.session, &open.b, &json!({"done": "b"}))
        .await
        .expect("the attempt in flight completes");
    assert!(f.w.run_status(open.run).await.is_terminal());
}

pub(crate) async fn completed_steps(f: &Fixture, run: RunId) -> Vec<u64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT seq FROM run_events WHERE run_id = $1 AND event_type = 'step.completed' ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the completions")
    .into_iter()
    .map(|seq| u64::try_from(seq).expect("small"))
    .collect()
}

pub(crate) fn asking(sequence: u64, acknowledge: bool) -> ResetRequest<'static> {
    ResetRequest {
        sequence,
        reason: WHY,
        acknowledge_reexecution: acknowledge,
        business: RerunBusinessId::Keep,
        reapply: ResetReapply::None,
        source_policy: ResetSourcePolicy::CancelSource,
    }
}

pub(crate) async fn request(
    f: &Fixture,
    run: RunId,
    request: ResetRequest<'_>,
    key: &str,
) -> Result<ResetMade, AppError> {
    f.w.runs.reset(&f.w.admin, run, request, key, &ctx()).await
}

pub(crate) fn due_of(f: &Fixture, id: ResetId) -> DueReset {
    DueReset {
        tenant_id: f.w.admin.tenant_id(),
        id,
    }
}

/// The resets that are due now, as a replica that holds every shard sees them.
pub(crate) async fn due(f: &Fixture) -> Vec<DueReset> {
    f.w.store
        .due_resets(&Shard::all().collect::<Vec<_>>(), 100, 100, &[])
        .await
        .expect("the due resets")
}

pub(crate) async fn made_from(f: &Fixture, source: RunId) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM runs WHERE rerun_of = $1 ORDER BY id")
        .bind(source.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the runs")
}

pub(crate) async fn entries(f: &Fixture, source: RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_resets WHERE source_run_id = $1")
        .bind(source.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the register")
}

pub(crate) async fn stops(f: &Fixture, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'run.stop_requested'",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the stop requests")
}

/// `(state, failure reason, new run, completed)` of an entry of the register.
pub(crate) async fn outcome(
    f: &Fixture,
    id: ResetId,
) -> (String, Option<String>, Option<Uuid>, bool) {
    sqlx::query_as(
        "SELECT state, failure_reason, new_run_id, completed_at IS NOT NULL \
         FROM run_resets WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the entry")
}

/// `(reason)` of the audit records of `action` about `resource`, oldest first.
pub(crate) async fn audited(f: &Fixture, action: &str, resource: RunId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT coalesce(reason, '') FROM audit_records WHERE action = $1 AND resource_id = $2 \
         ORDER BY created_at, id",
    )
    .bind(action)
    .bind(resource.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the audit trail")
}

/// A service as the control plane builds it with `SAGAT_RESET_CANCEL_SOURCE_ENABLED` unset.
fn without_cancel_source(
    f: &Fixture,
) -> RunService<PgRunStore, SystemClock, sagat_infra_ir::JsonEvaluator> {
    RunService::new(
        f.w.store.clone(),
        SystemClock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_reset_reapply(true)
}

#[tokio::test]
async fn a_request_cancels_an_open_source_in_the_ordinary_way_and_leaves_an_entry_that_waits() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let reset_key = key();

    let made = request(&f, open.run, asking(open.after_a, true), &reset_key)
        .await
        .expect("the request");

    // the answer: an entry that waits, no run yet
    assert!(
        made.run.is_none(),
        "no run is made while the source is open"
    );
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert_eq!(made.reset.source, open.run);
    assert_eq!(made.reset.sequence, open.after_a);
    assert_eq!(made.reset.new_run, None);
    assert_eq!(made.reset.failure_reason, None);
    assert_eq!(
        made.reset.source_policy,
        Some(ResetSourcePolicy::CancelSource)
    );

    // the source was cancelled as `cancelRun` does: one stop request, the attempt in flight told to stop, no run
    // finished by force
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Cancelling);
    assert_eq!(stops(&f, open.run).await, 1);
    let journal = f.w.journal(open.run).await;
    assert!(
        journal.contains(&"step.cancel_requested".to_owned()),
        "{journal:?}"
    );
    assert!(!journal.contains(&"run.finished".to_owned()), "{journal:?}");

    // what the entry keeps of what was asked
    let row: (String, String, bool, String, String, i64, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT source_policy, state, acknowledged_reexecution, actor, reason, source_sequence, prefix_digest \
         FROM run_resets WHERE id = $1",
    )
    .bind(made.reset.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the entry");
    assert_eq!(row.0, "CANCEL_SOURCE");
    assert_eq!(row.1, "PENDING_SOURCE");
    assert!(row.2, "the acknowledgement is kept");
    assert_eq!(row.3, f.w.admin.subject().as_str());
    assert_eq!(row.4, WHY);
    assert_eq!(row.5, i64::try_from(open.after_a).expect("small"));
    assert_eq!(row.6.expect("a digest").len(), 32);

    // the audit trail: the cancellation says it is for the reset and quotes nobody; the request carries the
    // justification, as the contract says
    let cancels = audited(&f, "run.cancel_requested", open.run).await;
    assert_eq!(
        cancels,
        [format!("cancelled for the reset {}", made.reset.id)]
    );
    let requested = audited(&f, "run.reset", open.run).await;
    assert_eq!(requested.len(), 1, "{requested:?}");
    assert_eq!(
        requested[0],
        format!(
            "reset {} requested at {}: {WHY}",
            made.reset.id, open.after_a
        )
    );
    assert!(
        !cancels[0].contains(WHY),
        "the justification is audited once"
    );

    // the receipt of the request is the entry
    let receipt: String = sqlx::query_scalar(
        "SELECT resource_id FROM command_dedupe WHERE scope = $1 AND idempotency_key = $2",
    )
    .bind(format!("run.reset:{}", f.w.namespace.id()))
    .bind(&reset_key)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the receipt");
    assert_eq!(receipt, made.reset.id.to_string());

    // while the source is open the entry is not due, and a step on it creates nothing
    assert!(due(&f).await.is_empty(), "an open source is not due");
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("a step"),
        Completed::Idle
    );
    assert!(made_from(&f, open.run).await.is_empty());
    assert_eq!(entries(&f, open.run).await, 1);
}

#[tokio::test]
async fn the_run_is_created_once_the_source_has_ended_from_the_prefix_the_request_named() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    let at_the_request = f.w.journal(open.run).await;

    end(&f, &open).await;
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Cancelled);
    let waiting = due(&f).await;
    assert_eq!(
        waiting.len(),
        1,
        "the entry is due once the source has ended"
    );
    assert_eq!(waiting[0].id, made.reset.id);

    let before = f.w.journal(open.run).await;
    let source_rows = crate::reset::rows(&f, open.run).await;
    assert_eq!(
        completer(&f)
            .step(&waiting[0], None)
            .await
            .expect("the step"),
        Completed::Created
    );

    let (state, failure, new_run, completed) = outcome(&f, made.reset.id).await;
    assert_eq!(state, "CREATED");
    assert_eq!(failure, None);
    assert!(completed, "completed_at is set");
    let new = RunId::new(new_run.expect("the run it made"));
    assert_eq!(made_from(&f, open.run).await, [new.as_uuid()]);
    let record =
        f.w.runs
            .get(&f.w.admin, new, &ctx())
            .await
            .expect("the run");
    assert_eq!(record.info.rerun_of, Some(open.run));
    assert_eq!(record.info.started_by, f.w.admin.subject().as_str());

    // its history is the source's up to the point, marked restored, then the reset, and it replays to its projection
    let history = f.w.journal(new).await;
    let kept = usize::try_from(open.after_a).expect("small");
    assert_eq!(history[..kept], before[..kept]);
    assert_eq!(history[kept], "run.reset");
    // row by row, not by name: the type, who did it, the payload, when, the trace, and the mark that it was restored
    let new_rows = crate::reset::rows(&f, new).await;
    assert!(new_rows.len() > kept, "the reset follows the prefix");
    for (restored, original) in new_rows.iter().zip(&source_rows).take(kept) {
        assert_eq!(restored.0, original.0, "the sequence");
        assert_eq!(restored.1, original.1, "the type of {}", original.0);
        assert_eq!(restored.2, original.2, "who did {}", original.0);
        assert_eq!(restored.3, original.3, "the payload of {}", original.0);
        assert_eq!(restored.5, original.5, "when {} happened", original.0);
        assert_eq!(restored.6, original.6, "the trace of {}", original.0);
        assert_eq!(
            restored.4.as_deref(),
            Some("true"),
            "{} is marked restored",
            original.0
        );
    }
    f.w.store
        .verify_run(f.w.admin.tenant_id(), new)
        .await
        .expect("the projection equals a replay of the journal");
    let reset_sequence: Option<i64> =
        sqlx::query_scalar("SELECT reset_sequence FROM runs WHERE id = $1")
            .bind(new.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the run");
    assert_eq!(
        reset_sequence,
        Some(i64::try_from(open.after_a).expect("small"))
    );

    // the source never reopens, and the run is audited as any reset
    assert_eq!(f.w.journal(open.run).await, before);
    assert_eq!(
        crate::reset::rows(&f, open.run).await,
        source_rows,
        "the source's rows are the ones it ended with, every column of them"
    );
    assert!(
        before.len() > at_the_request.len(),
        "the source went on to its end"
    );
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Cancelled);
    assert_eq!(
        audited(&f, "run.reset", new).await,
        [format!(
            "reset from {} at {} completes reset {}: {WHY}",
            open.run, open.after_a, made.reset.id
        )]
    );

    // the entry is completed once: nothing is due, a second step finds nothing to do
    assert!(due(&f).await.is_empty());
    assert_eq!(
        completer(&f)
            .step(&waiting[0], None)
            .await
            .expect("a second step"),
        Completed::Idle
    );
    assert_eq!(made_from(&f, open.run).await.len(), 1);
    let listed =
        f.w.runs
            .resets(&f.w.admin, open.run, &ctx())
            .await
            .expect("resets");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].new_run, Some(new));
}

#[tokio::test]
async fn a_retry_is_answered_from_the_register_whatever_the_source_and_the_setting_are_now() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let reset_key = key();
    let first = request(&f, open.run, asking(open.after_a, true), &reset_key)
        .await
        .expect("the request");

    // the lost answer is sent again: the same entry, nothing cancelled twice, nothing written twice
    let again = request(&f, open.run, asking(open.after_a, true), &reset_key)
        .await
        .expect("the retry");
    assert_eq!(again.reset.id, first.reset.id);
    assert_eq!(again.reset.state, ResetState::PendingSource);
    assert!(again.run.is_none());
    assert_eq!(stops(&f, open.run).await, 1);
    assert_eq!(entries(&f, open.run).await, 1);

    // where the setting is off a retry of an accepted request is still answered, and a new one is refused in words
    let off = without_cancel_source(&f);
    let answered = off
        .reset(
            &f.w.admin,
            open.run,
            asking(open.after_a, true),
            &reset_key,
            &ctx(),
        )
        .await
        .expect("answered from the register");
    assert_eq!(answered.reset.id, first.reset.id);
    let refused = off
        .reset(
            &f.w.admin,
            open.run,
            asking(open.after_a, true),
            &key(),
            &ctx(),
        )
        .await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_CANCEL_SOURCE_NOT_AVAILABLE",
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(
        entries(&f, open.run).await,
        1,
        "the refused request wrote nothing"
    );
    assert_eq!(stops(&f, open.run).await, 1, "and cancelled nothing");

    // the source ends and the run is made; the retry is the entry as it is now, and makes no second run
    end(&f, &open).await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, first.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
    for service in [&f.w.runs, &off] {
        let late = service
            .reset(
                &f.w.admin,
                open.run,
                asking(open.after_a, true),
                &reset_key,
                &ctx(),
            )
            .await
            .expect("the retry after the run was made");
        assert_eq!(late.reset.id, first.reset.id);
        assert_eq!(late.reset.state, ResetState::Created);
        let run = late.run.expect("the run it made");
        assert_eq!(late.reset.new_run, Some(run.info.run_id));
    }
    assert_eq!(made_from(&f, open.run).await.len(), 1);
    assert_eq!(entries(&f, open.run).await, 1);
}

#[tokio::test]
async fn the_same_key_with_another_request_is_not_the_same_request() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let reset_key = key();
    request(&f, open.run, asking(open.after_a, true), &reset_key)
        .await
        .expect("the request");

    // another point, another acknowledgement, another policy: all under the key of the first
    let mut others = vec![asking(1, true), asking(open.after_a, false)];
    others.push(ResetRequest {
        source_policy: ResetSourcePolicy::RequireTerminal,
        ..asking(open.after_a, true)
    });
    others.push(ResetRequest {
        business: RerunBusinessId::Drop,
        ..asking(open.after_a, true)
    });
    for other in others {
        let refused = request(&f, open.run, other, &reset_key).await;
        assert!(
            matches!(refused, Err(AppError::IdempotencyKeyReused)),
            "{refused:?}"
        );
    }
    assert_eq!(entries(&f, open.run).await, 1);
    assert_eq!(stops(&f, open.run).await, 1);
}

#[tokio::test]
async fn a_source_that_has_ended_is_reset_at_once_whatever_the_policy_and_the_setting() {
    let f = fixture().await;
    let ended = crate::reset::ended_chain(&f).await;
    let point = completed_steps(&f, ended).await[0];

    let off = without_cancel_source(&f);
    let reset_key = key();
    let made = off
        .reset(&f.w.admin, ended, asking(point, true), &reset_key, &ctx())
        .await
        .expect("an ended source needs no cancelling");
    assert_eq!(made.reset.state, ResetState::Created);
    let run = made.run.expect("the run is made at once");
    assert_eq!(made.reset.new_run, Some(run.info.run_id));
    assert_eq!(
        made.reset.source_policy,
        Some(ResetSourcePolicy::CancelSource),
        "the register says what was asked, though a source that ended is reset at once"
    );

    // and it is the same reset as a request that did not ask to cancel anything
    let same = request(
        &f,
        ended,
        ResetRequest {
            source_policy: ResetSourcePolicy::RequireTerminal,
            ..asking(point, true)
        },
        &reset_key,
    )
    .await
    .expect("the same request");
    assert_eq!(same.reset.id, made.reset.id);
    assert_eq!(made_from(&f, ended).await.len(), 1);
    assert_eq!(stops(&f, ended).await, 0, "nothing was cancelled");
}

#[tokio::test]
async fn with_the_setting_off_an_open_source_is_refused_in_words_and_left_alone() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let off = without_cancel_source(&f);

    let refused = off
        .reset(
            &f.w.admin,
            open.run,
            asking(open.after_a, true),
            &key(),
            &ctx(),
        )
        .await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_CANCEL_SOURCE_NOT_AVAILABLE",
                ..
            })
        ),
        "{refused:?}"
    );
    // a request that does not ask to cancel is refused as it always was
    let plain = off
        .reset(
            &f.w.admin,
            open.run,
            ResetRequest {
                source_policy: ResetSourcePolicy::RequireTerminal,
                ..asking(open.after_a, true)
            },
            &key(),
            &ctx(),
        )
        .await;
    assert!(
        matches!(
            plain,
            Err(AppError::ResetRefused {
                reason: "RESET_SOURCE_NOT_TERMINAL",
                ..
            })
        ),
        "{plain:?}"
    );
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Running);
    assert_eq!(stops(&f, open.run).await, 0);
    assert_eq!(entries(&f, open.run).await, 0);
}

#[tokio::test]
async fn the_requests_refusals_are_those_of_a_fresh_reset_and_cancel_nothing() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    // a point that is not one: the lease of `b`, the event after `a`'s completion
    let not_a_point =
        f.w.journal(open.run)
            .await
            .iter()
            .position(|t| t == "step.leased")
            .expect("a lease");
    let refused = request(
        &f,
        open.run,
        asking(u64::try_from(not_a_point + 1).expect("small"), true),
        &key(),
    )
    .await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_POINT_NOT_ELIGIBLE",
                ..
            })
        ),
        "{refused:?}"
    );
    // steps that completed after the point need the acknowledgement, and `b`'s completion is still to come, so a
    // request at the start of the run, where `a` has completed after the point, needs it now
    let refused = request(&f, open.run, asking(1, false), &key()).await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
                ..
            })
        ),
        "{refused:?}"
    );
    // a sequence past the journal
    let refused = request(&f, open.run, asking(10_000, true), &key()).await;
    assert!(refused.is_err(), "{refused:?}");
    // who may not reset may not cancel by resetting
    let viewer = f.w.member("vera", Role::Viewer).await;
    let refused =
        f.w.runs
            .reset(
                &viewer,
                open.run,
                asking(open.after_a, true),
                &key(),
                &ctx(),
            )
            .await;
    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "{refused:?}"
    );
    // a justification is required
    let refused =
        f.w.runs
            .reset(
                &f.w.admin,
                open.run,
                ResetRequest {
                    reason: "short",
                    ..asking(open.after_a, true)
                },
                &key(),
                &ctx(),
            )
            .await;
    assert!(
        matches!(refused, Err(AppError::NotAllowed(_))),
        "{refused:?}"
    );

    assert_eq!(
        f.w.run_status(open.run).await,
        RunStatus::Running,
        "nothing was cancelled"
    );
    assert_eq!(stops(&f, open.run).await, 0);
    assert_eq!(entries(&f, open.run).await, 0);
}

#[tokio::test]
async fn the_namespace_holds_at_most_the_pending_resets_its_limit_allows() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'pending_resets', 2, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of two");

    let one = open_chain_tagged(&f, "1").await;
    let two = open_chain_tagged(&f, "2").await;
    let three = open_chain_tagged(&f, "3").await;
    let first = request(&f, one.run, asking(one.after_a, true), &key())
        .await
        .expect("one");
    request(&f, two.run, asking(two.after_a, true), &key())
        .await
        .expect("two");

    let refused = request(&f, three.run, asking(three.after_a, true), &key()).await;
    assert!(
        matches!(&refused, Err(AppError::QuotaExceeded(what)) if *what == sagat_domain::quota::QuotaResource::PendingResets.limit_name()),
        "{refused:?}"
    );
    // refused before work: the third source was not cancelled and has no entry
    assert_eq!(f.w.run_status(three.run).await, RunStatus::Running);
    assert_eq!(stops(&f, three.run).await, 0);
    assert_eq!(entries(&f, three.run).await, 0);

    // room comes back when an entry is completed
    end(&f, &one).await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, first.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
    request(&f, three.run, asking(three.after_a, true), &key())
        .await
        .expect("room for a third");
    assert_eq!(entries(&f, three.run).await, 1);
}

#[tokio::test]
async fn an_acknowledgement_given_with_the_request_covers_the_steps_the_source_completes_afterwards()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
}

#[tokio::test]
async fn a_requester_who_lost_the_right_to_start_runs_before_the_completion_creates_nothing() {
    let f = fixture().await;
    let operator = f.w.member("opal", Role::Operator).await;
    let open = open_chain(&f).await;
    let made =
        f.w.runs
            .reset(
                &operator,
                open.run,
                asking(open.after_a, true),
                &key(),
                &ctx(),
            )
            .await
            .expect("an operator may reset");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    end(&f, &open).await;

    sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2")
        .bind(operator.tenant_id().as_uuid())
        .bind(operator.subject().as_str())
        .execute(&f.w.db.pool)
        .await
        .expect("demote");
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Failed("AUTHORITY_REVOKED")
    );
    let (state, failure, new_run, _) = outcome(&f, made.reset.id).await;
    assert_eq!(
        (state.as_str(), failure.as_deref(), new_run),
        ("FAILED", Some("AUTHORITY_REVOKED"), None)
    );
    assert!(made_from(&f, open.run).await.is_empty());
}

#[tokio::test]
async fn a_requester_who_is_no_longer_a_member_fails_the_entry_the_same_way() {
    let f = fixture().await;
    let operator = f.w.member("olga", Role::Operator).await;
    let open = open_chain(&f).await;
    let removed_key = key();
    let made =
        f.w.runs
            .reset(
                &operator,
                open.run,
                asking(open.after_a, true),
                &removed_key,
                &ctx(),
            )
            .await
            .expect("an operator may reset");
    end(&f, &open).await;
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = $2")
        .bind(operator.tenant_id().as_uuid())
        .bind(operator.subject().as_str())
        .execute(&f.w.db.pool)
        .await
        .expect("remove");
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Failed("AUTHORITY_REVOKED")
    );
    assert!(made_from(&f, open.run).await.is_empty());
    // the lost answer, sent again, is the entry as it ended: a cancelled run with no replacement, said as it is
    let retried =
        f.w.runs
            .reset(
                &operator,
                open.run,
                asking(open.after_a, true),
                &removed_key,
                &ctx(),
            )
            .await
            .expect("answered from the register");
    assert_eq!(retried.reset.id, made.reset.id);
    assert_eq!(retried.reset.state, ResetState::Failed);
    assert_eq!(
        retried.reset.failure_reason.as_deref(),
        Some("AUTHORITY_REVOKED")
    );
    assert!(retried.run.is_none());
}

#[tokio::test]
async fn a_requester_who_may_start_but_no_longer_operate_is_revoked_too() {
    // `run:operate` is part of what a fresh reset asks, so a role that keeps `run:start` only is not enough
    let f = fixture().await;
    let operator = f.w.member("odin", Role::Operator).await;
    let open = open_chain(&f).await;
    let made =
        f.w.runs
            .reset(
                &operator,
                open.run,
                asking(open.after_a, true),
                &key(),
                &ctx(),
            )
            .await
            .expect("the request");
    end(&f, &open).await;
    sqlx::query("UPDATE memberships SET role = 'developer' WHERE tenant_id = $1 AND subject = $2")
        .bind(operator.tenant_id().as_uuid())
        .bind(operator.subject().as_str())
        .execute(&f.w.db.pool)
        .await
        .expect("a role that starts runs and does not operate them");
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Failed("AUTHORITY_REVOKED")
    );
}

#[tokio::test]
async fn keeping_the_business_identifier_fails_the_entry_when_another_open_run_holds_it_by_then() {
    let f = fixture().await;
    let id = format!("order-{}", Uuid::now_v7());
    let source = start(
        &f,
        business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the source")
    .run
    .info
    .run_id;
    let made = request(&f, source, asking(1, true), &key())
        .await
        .expect("the request");
    assert!(
        f.w.run_status(source).await.is_terminal(),
        "nothing was in flight: the cancel ended it"
    );

    // the identifier moved on while the entry waited for its turn
    let holder = start(
        &f,
        business(&id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a later start takes the identifier of the ended run")
    .run
    .info
    .run_id;
    assert_eq!(holder_of(&f, &id).await, Some(holder.as_uuid()));

    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Failed("BUSINESS_ID_IN_USE")
    );
    assert!(
        made_from(&f, source).await.is_empty(),
        "nothing was created"
    );
    assert_eq!(
        holder_of(&f, &id).await,
        Some(holder.as_uuid()),
        "and the holder keeps it"
    );
}

#[tokio::test]
async fn keeping_the_identifier_binds_it_when_it_is_free_and_dropping_it_starts_without_one() {
    let f = fixture().await;
    let kept = format!("order-{}", Uuid::now_v7());
    let dropped = format!("order-{}", Uuid::now_v7());
    for (id, policy) in [
        (&kept, RerunBusinessId::Keep),
        (&dropped, RerunBusinessId::Drop),
    ] {
        let source = start(
            &f,
            business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
            &key(),
        )
        .await
        .expect("the source")
        .run
        .info
        .run_id;
        let made = request(
            &f,
            source,
            ResetRequest {
                business: policy,
                ..asking(1, true)
            },
            &key(),
        )
        .await
        .expect("the request");
        assert_eq!(
            completer(&f)
                .step(&due_of(&f, made.reset.id), None)
                .await
                .expect("step"),
            Completed::Created
        );
        let new = made_from(&f, source).await[0];
        let business_id: Option<String> =
            sqlx::query_scalar("SELECT business_id FROM runs WHERE id = $1")
                .bind(new)
                .fetch_one(&f.w.db.pool)
                .await
                .expect("the run");
        match policy {
            RerunBusinessId::Keep => {
                assert_eq!(business_id.as_deref(), Some(id.as_str()));
                assert_eq!(holder_of(&f, id).await, Some(new));
            }
            RerunBusinessId::Drop => assert_eq!(business_id, None),
        }
    }
}

/// An entry as a request would have written it, for a source that has ended, with the digest and the point given.
pub(crate) async fn plant(
    f: &Fixture,
    source: RunId,
    sequence: i64,
    digest: &[u8],
    acknowledged: bool,
) -> ResetId {
    plant_in(
        f,
        f.w.namespace.id(),
        source,
        sequence,
        digest,
        acknowledged,
    )
    .await
}

/// As [`plant`], in the namespace given.
async fn plant_in(
    f: &Fixture,
    namespace: sagat_domain::identity::NamespaceId,
    source: RunId,
    sequence: i64,
    digest: &[u8],
    acknowledged: bool,
) -> ResetId {
    plant_with(
        f,
        ResetId::generate(),
        namespace,
        source,
        sequence,
        digest,
        acknowledged,
    )
    .await
}

/// As [`plant_in`], with the id given (the shard of an entry is a function of its id).
pub(crate) async fn plant_with(
    f: &Fixture,
    id: ResetId,
    namespace: sagat_domain::identity::NamespaceId,
    source: RunId,
    sequence: i64,
    digest: &[u8],
    acknowledged: bool,
) -> ResetId {
    sqlx::query(
        "INSERT INTO run_resets \
         (tenant_id, namespace_id, id, source_run_id, source_sequence, prefix_digest, reapply, business_id_policy, \
          source_policy, state, actor, reason, created_at, reapply_report, acknowledged_reexecution) \
         VALUES ($1, $2, $3, $4, $5, $6, 'NONE', 'KEEP', 'CANCEL_SOURCE', 'PENDING_SOURCE', $7, $8, now(), '[]', $9)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(namespace.as_uuid())
    .bind(id.as_uuid())
    .bind(source.as_uuid())
    .bind(sequence)
    .bind(digest)
    .bind(f.w.admin.subject().as_str())
    .bind(WHY)
    .bind(acknowledged)
    .execute(&f.w.db.pool)
    .await
    .expect("plant an entry");
    id
}

pub(crate) async fn digest_of(f: &Fixture, source: RunId, sequence: usize) -> Vec<u8> {
    let rows =
        f.w.store
            .reset_source(f.w.admin.tenant_id(), source)
            .await
            .expect("the journal")
            .expect("the source");
    sagat_application::runs::prefix_digest(&rows.events[..sequence])
        .as_bytes()
        .to_vec()
}

#[tokio::test]
async fn a_point_that_is_no_longer_eligible_or_a_prefix_that_is_not_the_one_asked_for_fails_the_entry()
 {
    let f = fixture().await;
    let ended = crate::reset::ended_chain(&f).await;
    let point = completed_steps(&f, ended).await[0];
    let kept = usize::try_from(point).expect("small");
    let good = digest_of(&f, ended, kept).await;

    // the digest of the prefix is not the one the request saw
    let wrong = plant(
        &f,
        ended,
        i64::try_from(point).expect("small"),
        &[7u8; 32],
        true,
    )
    .await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, wrong), None)
            .await
            .expect("step"),
        Completed::Failed("BOUNDARY_NO_LONGER_ELIGIBLE")
    );
    // a sequence that is not a point of the journal as it ended
    let leased =
        f.w.journal(ended)
            .await
            .iter()
            .position(|t| t == "step.leased")
            .expect("a lease")
            + 1;
    let not_a_point = plant(
        &f,
        ended,
        i64::try_from(leased).expect("small"),
        &digest_of(&f, ended, leased).await,
        true,
    )
    .await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, not_a_point), None)
            .await
            .expect("step"),
        Completed::Failed("BOUNDARY_NO_LONGER_ELIGIBLE")
    );
    // and the same entry with the right digest and a real point is made
    let right = plant(&f, ended, i64::try_from(point).expect("small"), &good, true).await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, right), None)
            .await
            .expect("step"),
        Completed::Created
    );
    for id in [wrong, not_a_point] {
        let (state, failure, new_run, _) = outcome(&f, id).await;
        assert_eq!(
            (state.as_str(), failure.as_deref(), new_run),
            ("FAILED", Some("BOUNDARY_NO_LONGER_ELIGIBLE"), None)
        );
    }
    assert_eq!(made_from(&f, ended).await.len(), 1);
}

#[tokio::test]
async fn a_run_the_namespace_has_no_room_for_fails_the_entry_as_refused_by_admission() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 1, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of one");
    // one run holds the only place
    f.w.start(f.workflow, &json!({})).await;

    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Failed("ADMISSION_REFUSED")
    );
    assert!(made_from(&f, open.run).await.is_empty());
}

#[tokio::test]
async fn a_source_whose_compensation_is_stuck_is_open_and_the_entry_waits_for_an_operator() {
    let f = fixture().await;
    let source = crate::batch_actions::run_with_a_stuck_compensation(&f.w).await;
    assert_eq!(f.w.run_status(source).await, RunStatus::ManualReview);
    let stopped = stops(&f, source).await;

    // it is open: the request is accepted, the run is cancelled as `cancelRun` would (the compensation that cannot be
    // undone keeps it in manual review) and the entry waits
    let made = request(&f, source, asking(1, true), &key())
        .await
        .expect("the request");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert_eq!(
        stops(&f, source).await,
        stopped + 1,
        "cancelled once, in the ordinary way"
    );
    assert_eq!(
        f.w.run_status(source).await,
        RunStatus::ManualReview,
        "and still waiting for an operator"
    );
    assert!(due(&f).await.is_empty());
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Idle,
        "manual review is open"
    );
    assert!(made_from(&f, source).await.is_empty());

    // the operator gives up on the compensation: the run ends and the entry is made
    f.w.review(
        source,
        "reserve~c",
        sagat_application::runs::ReviewAction::Skip,
        "the provider is gone for good",
    )
    .await
    .expect("skip the compensation");
    assert!(f.w.run_status(source).await.is_terminal());
    assert_eq!(due(&f).await.len(), 1);
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
}

#[tokio::test]
async fn a_source_that_is_being_cancelled_already_is_not_cancelled_twice_and_the_entry_waits_for_its_end()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    f.w.operate_run(open.run, sagat_application::runs::RunOperation::Cancel)
        .await;
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Cancelling);
    assert_eq!(stops(&f, open.run).await, 1);

    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("a source on its way out is waited for");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert_eq!(
        stops(&f, open.run).await,
        1,
        "the second cancel is not made"
    );
    assert!(
        audited(&f, "run.cancel_requested", open.run).await.len() == 1,
        "and not audited"
    );

    end(&f, &open).await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
}

#[tokio::test]
async fn another_tenant_sees_neither_the_request_nor_the_entry() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let refused =
        f.w.runs
            .reset(
                &f.w.other_admin,
                open.run,
                asking(open.after_a, true),
                &key(),
                &ctx(),
            )
            .await;
    assert!(matches!(refused, Err(AppError::NotFound)), "{refused:?}");
    assert_eq!(stops(&f, open.run).await, 0, "a stranger cancels nothing");
    assert_eq!(entries(&f, open.run).await, 0);

    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    let seen = f.w.runs.resets(&f.w.other_admin, open.run, &ctx()).await;
    assert!(matches!(seen, Err(AppError::NotFound)), "{seen:?}");
    let seen =
        f.w.runs
            .reset_record(&f.w.other_admin, open.run, made.reset.id, &ctx())
            .await;
    assert!(matches!(seen, Err(AppError::NotFound)), "{seen:?}");
    // a key of the first tenant means nothing to the second
    let again =
        f.w.runs
            .reset(
                &f.w.other_admin,
                open.run,
                asking(open.after_a, true),
                &key(),
                &ctx(),
            )
            .await;
    assert!(matches!(again, Err(AppError::NotFound)), "{again:?}");
}

// --------------------------------------------------------------------------------------- the pass of the engine

struct Replica {
    keeper: Keeper<PgRunStore>,
    pass: ResetPass<PgRunStore, Completions, PgIdentityStore, SystemClock>,
    placement: Placement,
}

fn replica(f: &Fixture, name: &str) -> Replica {
    let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    Replica {
        keeper,
        pass: ResetPass::new(completer(f), placement.clone(), ResetSettings::default()),
        placement,
    }
}

#[tokio::test]
async fn the_pass_creates_the_runs_of_the_entries_whose_source_has_ended_and_only_those() {
    let f = fixture().await;
    let ended = open_chain_tagged(&f, "1").await;
    let waiting = open_chain_tagged(&f, "2").await;
    let first = request(&f, ended.run, asking(ended.after_a, true), &key())
        .await
        .expect("one");
    let second = request(&f, waiting.run, asking(waiting.after_a, true), &key())
        .await
        .expect("two");
    end(&f, &ended).await;

    let one = replica(&f, "engine-one");
    one.keeper.round().await.expect("keeper");
    let round = one.pass.round().await;
    assert_eq!(
        (
            round.due,
            round.created,
            round.failed_entries,
            round.refused,
            round.errors
        ),
        (1, 1, 0, 0, 0)
    );
    assert!(round.did_work());
    assert_eq!(made_from(&f, ended.run).await.len(), 1);
    assert!(
        made_from(&f, waiting.run).await.is_empty(),
        "its source is still open"
    );
    assert_eq!(outcome(&f, first.reset.id).await.0, "CREATED");
    assert_eq!(outcome(&f, second.reset.id).await.0, "PENDING_SOURCE");

    // nothing is left to do until the second source ends
    let idle = one.pass.round().await;
    assert_eq!((idle.due, idle.created), (0, 0));
    assert!(!idle.did_work());
    end(&f, &waiting).await;
    assert_eq!(one.pass.round().await.created, 1);
    assert_eq!(made_from(&f, waiting.run).await.len(), 1);
}

#[tokio::test]
async fn a_replica_that_holds_no_shard_completes_nothing() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;

    // it never kept: it holds no shard, so nothing is its to complete
    let lone = replica(&f, "engine-without-shards");
    let round = lone.pass.round().await;
    assert_eq!((round.due, round.created), (0, 0));
    assert_eq!(outcome(&f, made.reset.id).await.0, "PENDING_SOURCE");

    lone.keeper.round().await.expect("keeper");
    assert_eq!(lone.pass.round().await.created, 1);
}

#[tokio::test]
async fn a_lease_that_moved_refuses_the_completion_and_nothing_is_created() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;

    let slow = replica(&f, "slow-engine");
    slow.keeper.round().await.expect("keeper");
    let shard = Shard::of(made.reset.id.as_uuid());
    let lease = slow
        .placement
        .snapshot()
        .lease_of(shard, std::time::Instant::now())
        .expect("it holds the shard");
    let fence = ShardFence {
        owner: slow.placement.owner().to_owned(),
        lease,
    };
    // another replica took the shard over while this one was working
    sqlx::query("UPDATE scheduler_shards SET owner = 'the-thief', generation = generation + 1 WHERE shard = $1")
        .bind(i16::from(shard.index()))
        .execute(&f.w.db.pool)
        .await
        .expect("take the shard");

    let refused = completer(&f)
        .step(&due_of(&f, made.reset.id), Some(fence.clone()))
        .await
        .expect("a step");
    assert!(
        matches!(refused, Completed::Refused(reason) if reason.contains("lease")),
        "{refused:?}"
    );
    assert_eq!(outcome(&f, made.reset.id).await.0, "PENDING_SOURCE");
    assert!(made_from(&f, open.run).await.is_empty());

    // the failure of an entry is fenced the same way: a replica that lost its lease cannot end an entry either
    // an entry of the same shard (the fence is the lease of one shard; a fence for another one is a defect of the
    // caller, which the pass reports as an error and does not take for a race)
    let mut id = ResetId::generate();
    while Shard::of(id.as_uuid()) != shard {
        id = ResetId::generate();
    }
    let planted = plant_with(&f, id, f.w.namespace.id(), open.run, 1, &[1u8; 32], true).await;
    let refused = completer(&f)
        .step(&due_of(&f, planted), Some(fence))
        .await
        .expect("a step");
    assert!(
        matches!(refused, Completed::Refused(reason) if reason == SHARD_LEASE_LOST),
        "{refused:?}"
    );
    assert_eq!(outcome(&f, planted).await.0, "PENDING_SOURCE");
}

// ------------------------------------------------------------------------------------------- the telemetry

#[test]
fn what_the_pass_completes_is_counted_once_it_is_committed_with_closed_labels() {
    use crate::counted::{recorded, series};
    const NAME: &str = "workflow_reset_completions_total";
    let page = recorded(|| async {
        let f = fixture().await;
        let made = open_chain_tagged(&f, "1").await;
        let ended_for_failure = crate::reset::ended_chain(&f).await;
        let waiting = open_chain_tagged(&f, "3").await;
        let created = request(&f, made.run, asking(made.after_a, true), &key())
            .await
            .expect("one");
        let first_point = completed_steps(&f, ended_for_failure).await[0];
        let failed_id = plant(
            &f,
            ended_for_failure,
            i64::try_from(first_point).expect("small"),
            &digest_of(
                &f,
                ended_for_failure,
                usize::try_from(first_point).expect("small"),
            )
            .await,
            false,
        )
        .await;
        let pending = request(&f, waiting.run, asking(waiting.after_a, true), &key())
            .await
            .expect("three");
        end(&f, &made).await;
        assert_eq!(
            completer(&f)
                .step(&due_of(&f, failed_id), None)
                .await
                .expect("the failure"),
            Completed::Failed("REEXECUTION_NOT_ACKNOWLEDGED")
        );
        assert_eq!(
            completer(&f)
                .step(&due_of(&f, created.reset.id), None)
                .await
                .expect("the run"),
            Completed::Created
        );
        // a step on an entry that is completed already counts nothing
        completer(&f)
            .step(&due_of(&f, created.reset.id), None)
            .await
            .expect("again");
        // nor does one whose source is open
        completer(&f)
            .step(&due_of(&f, pending.reset.id), None)
            .await
            .expect("idle");
    });
    for (outcome, reason, times) in [
        ("created", "none", 1.0),
        ("failed", "REEXECUTION_NOT_ACKNOWLEDGED", 1.0),
    ] {
        assert_eq!(
            series(&page, NAME, &[("outcome", outcome), ("reason", reason)]),
            Some(times),
            "{outcome} {reason}\n{page}"
        );
    }
    // and nothing else: no label that is not one of the two closed ones
    let lines: Vec<&str> = page.lines().filter(|line| line.starts_with(NAME)).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in lines {
        let inside = &line[line.find('{').expect("labels") + 1..line.rfind('}').expect("labels")];
        let mut keys: Vec<&str> = inside
            .split(',')
            .filter_map(|pair| pair.split('=').next())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["outcome", "reason"], "{line}");
    }
}

#[tokio::test]
async fn a_pending_reset_is_counted_and_is_late_only_when_its_source_has_ended_and_nothing_has_made_the_run()
 {
    let f = fixture().await;
    let none = f.w.store.sample().await.expect("sample");
    assert_eq!((none.resets_pending, none.overdue.resets), (0, 0.0));

    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    let waiting = f.w.store.sample().await.expect("sample");
    assert_eq!(waiting.resets_pending, 1, "it waits");
    assert_eq!(
        waiting.overdue.resets, 0.0,
        "a source that is open is not the engine's lag: it waits for the run"
    );

    end(&f, &open).await;
    let late = f.w.store.sample().await.expect("sample");
    assert_eq!(late.resets_pending, 1);
    assert!(
        late.overdue.resets > 0.0 && late.overdue.resets < 120.0,
        "the source has ended and nothing has made the run: {}",
        late.overdue.resets
    );
    assert_eq!(late.overdue.batches, 0.0, "the other passes are not late");

    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
    let done = f.w.store.sample().await.expect("sample");
    assert_eq!((done.resets_pending, done.overdue.resets), (0, 0.0));
}

// ------------------------------------------------------------------------------------------- the reapply

#[tokio::test]
async fn a_pending_reset_delivers_again_what_the_source_received_though_the_engine_has_no_such_setting()
 {
    use crate::support::engine::wait_node;
    use sagat_application::runs::ReapplyOutcome;
    const SIGNAL: &str = "cancel-source.confirmed";
    let f = fixture().await;
    let workflow =
        f.w.publish(document(
            &format!("cancel-source-signal-{}", Uuid::now_v7()),
            "a",
            vec![
                with(activity("a"), "next", json!("w")),
                with(
                    wait_node("w", SIGNAL, "input.k", 300_000, None),
                    "next",
                    json!("b"),
                ),
                activity("b"),
            ],
        ))
        .await
        .0;
    let run = f.w.start(workflow, &json!({"k": "s"})).await;
    let session = f.w.session(&["act.a", "act.b"]).await;
    let a = f.w.one_task_of(&session, "act.a").await;
    f.w.complete(&session, &a, &json!({"done": "a"}))
        .await
        .expect("a completes");
    let point = completed_steps(&f, run).await[0];
    // the signal wakes the wait and `b` is leased: the source is open, with a signal after the point
    f.w.runs
        .signal(&f.w.admin, run, SIGNAL, br#"{"paid":true}"#, &key(), &ctx())
        .await
        .expect("the signal");
    let b = f.w.one_task_of(&session, "act.b").await;

    let made = request(
        &f,
        run,
        ResetRequest {
            reapply: ResetReapply::Signals,
            ..asking(point, true)
        },
        &key(),
    )
    .await
    .expect("the request");
    assert_eq!(made.reset.reapply, ResetReapply::Signals);
    f.w.complete(&session, &b, &json!({"done": "b"}))
        .await
        .expect("b completes after the cancellation");
    assert!(f.w.run_status(run).await.is_terminal());

    // the engine's service has no setting for the reapply (D1990) and does not ask for one at the completion: what was
    // accepted at the request stands (D1997)
    let engine = RunService::new(
        f.w.store.clone(),
        SystemClock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_declared_updates(Arc::new(sagat_infra_ir::JsonDeclaredOperationEvaluator));
    let by_the_engine = ResetCompleter::new(
        f.w.store.clone(),
        engine,
        PgIdentityStore::new(f.w.db.pool.clone()),
        SystemClock,
    );
    assert_eq!(
        by_the_engine
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
    let entry =
        f.w.runs
            .reset_record(&f.w.admin, run, made.reset.id, &ctx())
            .await
            .expect("the entry");
    assert_eq!(entry.state, ResetState::Created);
    assert_eq!(entry.reapply_report.len(), 1, "{:?}", entry.reapply_report);
    assert_eq!(entry.reapply_report[0].kind, "SIGNAL");
    assert_eq!(
        entry.reapply_report[0].outcome,
        ReapplyOutcome::Reapplied.as_str()
    );
    let new = entry.new_run.expect("the run");
    let signalled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'wait.satisfied' \
           AND coalesce(detail->>'restored', 'false') <> 'true'",
    )
    .bind(new.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the deliveries");
    assert_eq!(signalled, 1, "the new run was signalled once, by the reset");
}

// --------------------------------------------------------------------------------- what a second look found

#[tokio::test]
async fn a_replica_is_offered_the_entries_of_the_shards_it_holds_and_no_others() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;

    let own = Shard::of(made.reset.id.as_uuid());
    let others: Vec<Shard> = Shard::all()
        .filter(|shard| shard.index() != own.index())
        .collect();
    let seen =
        f.w.store
            .due_resets(&[own], 10, 10, &[])
            .await
            .expect("due");
    assert_eq!(seen, [due_of(&f, made.reset.id)], "the shard of the entry");
    let none =
        f.w.store
            .due_resets(&others, 10, 10, &[])
            .await
            .expect("due");
    assert!(
        none.is_empty(),
        "an entry is not offered to a replica that holds other shards: {none:?}"
    );
    assert!(
        f.w.store
            .due_resets(&[], 10, 10, &[])
            .await
            .expect("due")
            .is_empty(),
        "nor to one that holds none"
    );
}

#[tokio::test]
async fn the_limit_of_pending_resets_counts_the_entries_of_its_namespace_and_no_others() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'pending_resets', 2, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of two");
    let elsewhere = f.w.add_namespace("elsewhere").await;
    let ended = crate::reset::ended_chain(&f).await;
    // two entries wait in another namespace of the tenant (the register's keys allow it; a request cannot make them
    // here because the workflows of the fixture live in `payments`)
    for _ in 0..2 {
        plant_in(&f, elsewhere.id(), ended, 1, &[9u8; 32], true).await;
    }

    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key()).await;
    assert!(
        made.is_ok(),
        "the entries of another namespace do not count: {made:?}"
    );
    // and the limit is still a limit of this one
    let second = open_chain_tagged(&f, "2").await;
    request(&f, second.run, asking(second.after_a, true), &key())
        .await
        .expect("the second place");
    let third = open_chain_tagged(&f, "3").await;
    let refused = request(&f, third.run, asking(third.after_a, true), &key()).await;
    assert!(
        matches!(refused, Err(AppError::QuotaExceeded(_))),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_step_that_read_an_entry_before_another_engine_made_its_run_is_refused_and_makes_none() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;
    // the engine that is slow read the entry while it was pending
    let stale =
        f.w.store
            .pending_reset(f.w.admin.tenant_id(), made.reset.id)
            .await
            .expect("read")
            .expect("it is pending");
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("the other engine"),
        Completed::Created
    );

    let refused =
        f.w.runs
            .complete_reset(&f.w.admin, &stale, None, &ctx())
            .await;
    assert!(
        matches!(
            refused,
            Err(AppError::StateConflict(reason)) if reason == RESET_ALREADY_COMPLETED
        ),
        "{refused:?}"
    );
    assert_eq!(made_from(&f, open.run).await.len(), 1, "one run");
    assert_eq!(outcome(&f, made.reset.id).await.0, "CREATED");
}

#[tokio::test]
async fn a_step_that_read_an_entry_before_another_engine_failed_it_is_refused_and_makes_no_run() {
    let f = fixture().await;
    let ended = crate::reset::ended_chain(&f).await;
    let point = completed_steps(&f, ended).await[0];
    let kept = usize::try_from(point).expect("small");
    let good = digest_of(&f, ended, kept).await;
    // an entry the service would make, as the engine that is slow read it
    let id = plant(&f, ended, i64::try_from(point).expect("small"), &good, true).await;
    let stale =
        f.w.store
            .pending_reset(f.w.admin.tenant_id(), id)
            .await
            .expect("read")
            .expect("it is pending");
    // the other engine ended it: nothing was made, so no receipt of a run exists
    sqlx::query(
        "UPDATE run_resets SET state = 'FAILED', failure_reason = 'ADMISSION_REFUSED', completed_at = now() \
         WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("fail it");

    let refused =
        f.w.runs
            .complete_reset(&f.w.admin, &stale, None, &ctx())
            .await;
    assert!(
        matches!(
            refused,
            Err(AppError::StateConflict(reason)) if reason == RESET_ALREADY_COMPLETED
        ),
        "{refused:?}"
    );
    assert!(made_from(&f, ended).await.is_empty(), "no run");
    assert_eq!(
        outcome(&f, id).await.0,
        "FAILED",
        "as the other engine left it"
    );
}

#[tokio::test]
async fn the_run_of_a_completion_is_recorded_under_the_key_of_the_platform_and_not_under_a_callers()
{
    let f = fixture().await;
    let open = open_chain(&f).await;
    let caller_key = key();
    let made = request(&f, open.run, asking(open.after_a, true), &caller_key)
        .await
        .expect("the request");
    end(&f, &open).await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, made.reset.id), None)
            .await
            .expect("step"),
        Completed::Created
    );
    let new = made_from(&f, open.run).await[0];
    // the receipts of starts: the source's own, which the fixture made, and the one of the run the completion made
    let receipts: Vec<(String, String)> = sqlx::query_as(
        "SELECT idempotency_key, resource_id FROM command_dedupe WHERE scope = $1 ORDER BY idempotency_key",
    )
    .bind(format!("run.start:{}", f.w.namespace.id()))
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the receipts of starts");
    assert!(
        receipts.contains(&(format!("reset:{}", made.reset.id), new.to_string())),
        "the run is recorded under the key of the platform (the prefix `reset:` is reserved): {receipts:?}"
    );
    assert!(
        !receipts.iter().any(|(key, _)| key == &caller_key),
        "the key of the caller's request made no start: {receipts:?}"
    );
}

#[test]
fn a_shard_stolen_between_the_scan_and_the_step_refuses_the_step_and_counts_the_fencing() {
    use crate::counted::{recorded, series};
    let page = recorded(|| async {
        let f = fixture().await;
        let open = open_chain(&f).await;
        let made = request(&f, open.run, asking(open.after_a, true), &key())
            .await
            .expect("the request");
        end(&f, &open).await;

        let slow = replica(&f, "slow-engine");
        slow.keeper.round().await.expect("keeper");
        // another replica takes the shard over after this one scanned for its work (its own snapshot does not know)
        sqlx::query(
            "UPDATE scheduler_shards SET owner = 'the-thief', generation = generation + 1 WHERE shard = $1",
        )
        .bind(i16::from(Shard::of(made.reset.id.as_uuid()).index()))
        .execute(&f.w.db.pool)
        .await
        .expect("take the shard");

        let round = slow.pass.round().await;
        assert_eq!(
            (round.due, round.created, round.refused, round.errors),
            (1, 0, 1, 0),
            "{round:?}"
        );
        assert!(!round.did_work());
        assert_eq!(outcome(&f, made.reset.id).await.0, "PENDING_SOURCE");
        assert!(made_from(&f, open.run).await.is_empty());
    });
    assert_eq!(
        series(&page, "scheduler_fenced_total", &[("kind", "reset")]),
        Some(1.0),
        "{page}"
    );
}

// ------------------------------------------------------------------- the review of 2026-11-06 (D2006..D2012)

/// An answer the service of the engine is made to give for an entry, in place of the real completion: a way to put
/// a failure of the platform where the test needs one.
type Answers = Arc<
    std::sync::Mutex<std::collections::HashMap<ResetId, Box<dyn Fn() -> AppError + Send + Sync>>>,
>;

fn answers() -> Answers {
    Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn script(answers: &Answers, id: ResetId, answer: impl Fn() -> AppError + Send + Sync + 'static) {
    answers
        .lock()
        .expect("the answers")
        .insert(id, Box::new(answer));
}

pub(crate) struct Scripted {
    inner: Completions,
    answers: Answers,
}

impl ResetCompletions for Scripted {
    async fn complete(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> Result<Completion, AppError> {
        let answer = self
            .answers
            .lock()
            .expect("the answers")
            .get(&entry.id)
            .map(|answer| answer());
        match answer {
            Some(error) => Err(error),
            None => self.inner.complete(principal, entry, fence, ctx).await,
        }
    }
}

type ScriptedCompleter = ResetCompleter<PgRunStore, Scripted, PgIdentityStore, SystemClock>;

fn scripted(f: &Fixture, answers: &Answers) -> ScriptedCompleter {
    ResetCompleter::new(
        f.w.store.clone(),
        Scripted {
            inner: Completions(Arc::clone(&f.w)),
            answers: Arc::clone(answers),
        },
        PgIdentityStore::new(f.w.db.pool.clone()),
        SystemClock,
    )
}

/// What a scripted completer answers with, for the table below.
type Answer = Box<dyn Fn() -> AppError + Send + Sync>;

/// What a step came to, for the table of answers below.
#[derive(Debug)]
enum Came {
    /// The entry was failed with this reason.
    Failed(&'static str),
    /// A race that changed nothing: `Completed::Refused`.
    Raced,
    /// Not an outcome: the step failed and the entry stays pending to be tried again.
    Retry,
}

#[tokio::test]
async fn only_an_answer_of_the_platform_ends_an_entry_and_anything_else_leaves_it_pending() {
    use sagat_domain::identity::Permission;
    let f = fixture().await;
    let ended = crate::reset::ended_chain(&f).await;
    let point = completed_steps(&f, ended).await[0];
    let digest = digest_of(&f, ended, usize::try_from(point).expect("small")).await;
    let cases: Vec<(&str, Answer, Came)> = vec![
        (
            "unavailable",
            Box::new(|| AppError::Unavailable),
            Came::Retry,
        ),
        ("internal", Box::new(|| AppError::Internal), Came::Retry),
        (
            "a conflict of state the pass does not know",
            Box::new(|| {
                AppError::StateConflict("the schedule changed since the decision was made")
            }),
            Came::Retry,
        ),
        (
            "the key of the receipt reused",
            Box::new(|| AppError::IdempotencyKeyReused),
            Came::Retry,
        ),
        (
            "completed first",
            Box::new(|| AppError::StateConflict(RESET_ALREADY_COMPLETED)),
            Came::Raced,
        ),
        (
            "the lease lost",
            Box::new(|| AppError::StateConflict(SHARD_LEASE_LOST)),
            Came::Raced,
        ),
        (
            "the authority revoked, as the store says it",
            Box::new(|| AppError::StateConflict(AUTHORITY_REVOKED)),
            Came::Failed("AUTHORITY_REVOKED"),
        ),
        (
            "the permission missing",
            Box::new(|| AppError::Forbidden {
                permission: Permission::RunStart,
            }),
            Came::Failed("AUTHORITY_REVOKED"),
        ),
        (
            "a quota",
            Box::new(|| AppError::QuotaExceeded("active runs in the namespace")),
            Came::Failed("ADMISSION_REFUSED"),
        ),
        (
            "a start that is not acceptable",
            Box::new(|| AppError::InvalidStart {
                reason: "START_DELAY_INVALID",
                field: None,
            }),
            Came::Failed("ADMISSION_REFUSED"),
        ),
        (
            "the workflow is gone",
            Box::new(|| AppError::NotFound),
            Came::Failed("ADMISSION_REFUSED"),
        ),
        (
            "an input the platform refuses",
            Box::new(|| AppError::NotAllowed("the run input is larger than 256 KiB")),
            Came::Failed("ADMISSION_REFUSED"),
        ),
        (
            "the business identifier held",
            Box::new(|| AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_IN_USE",
                holder: RunId::generate(),
                reusable_after: None,
            }),
            Came::Failed("BUSINESS_ID_IN_USE"),
        ),
    ];
    for (label, answer, came) in cases {
        let id = plant(
            &f,
            ended,
            i64::try_from(point).expect("small"),
            &digest,
            true,
        )
        .await;
        let scripted_answers = answers();
        script(&scripted_answers, id, answer);
        let step = scripted(&f, &scripted_answers)
            .step(&due_of(&f, id), None)
            .await;
        let state = outcome(&f, id).await;
        match came {
            Came::Failed(reason) => {
                assert_eq!(
                    step.as_ref().ok(),
                    Some(&Completed::Failed(reason)),
                    "{label}: {step:?}"
                );
                assert_eq!(state.0, "FAILED", "{label}");
                assert_eq!(state.1.as_deref(), Some(reason), "{label}");
            }
            Came::Raced => {
                assert!(
                    matches!(step, Ok(Completed::Refused(_))),
                    "{label}: {step:?}"
                );
                assert_eq!(state.0, "PENDING_SOURCE", "{label}");
            }
            Came::Retry => {
                assert!(step.is_err(), "{label}: {step:?}");
                assert_eq!(
                    state.0, "PENDING_SOURCE",
                    "{label}: not written as an outcome"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_source_the_requester_cannot_see_is_an_anomaly_that_leaves_the_entry_pending_and_not_a_lost_authority()
 {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;
    let entry =
        f.w.store
            .pending_reset(f.w.admin.tenant_id(), made.reset.id)
            .await
            .expect("read")
            .expect("it is pending");
    // a member of the other tenant cannot see the source: a source read at the request cannot disappear, so this is
    // not an outcome of the reset and no reason is invented for it
    let stranger =
        f.w.runs
            .complete_reset(&f.w.other_admin, &entry, None, &ctx())
            .await;
    assert!(
        matches!(stranger, Err(AppError::Unavailable)),
        "{stranger:?}"
    );
    // a role without the right to operate runs is refused before anything is read
    let viewer = f.w.member("vic", Role::Viewer).await;
    let refused = f.w.runs.complete_reset(&viewer, &entry, None, &ctx()).await;
    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "{refused:?}"
    );
    assert!(made_from(&f, open.run).await.is_empty());
    assert_eq!(outcome(&f, made.reset.id).await.0, "PENDING_SOURCE");
}

/// A source in the other tenant that has ended, and the entry that waits for it (a request cancels a run nothing is
/// running, so the source is over at once).
pub(crate) async fn other_tenant_entry(f: &Fixture) -> ResetId {
    let workflow =
        f.w.publish_as_other_tenant(document(
            &format!("cancel-source-other-{}", Uuid::now_v7()),
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    let run =
        f.w.start_as_other_tenant(workflow, &json!({"k": 2}))
            .await
            .expect("a run of the other tenant");
    let made =
        f.w.runs
            .reset(&f.w.other_admin, run, asking(1, true), &key(), &ctx())
            .await
            .expect("the request of the other tenant");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    made.reset.id
}

#[tokio::test]
async fn an_entry_that_keeps_failing_does_not_hold_the_head_of_the_scan_of_the_others() {
    let f = fixture().await;
    let answers = answers();
    // the older entry, of the first tenant, fails for as long as the test lasts
    let bad = open_chain_tagged(&f, "1").await;
    let stuck = request(&f, bad.run, asking(bad.after_a, true), &key())
        .await
        .expect("one");
    end(&f, &bad).await;
    script(&answers, stuck.reset.id, || AppError::Unavailable);
    // the younger one, of the other tenant, is fine
    let good = other_tenant_entry(&f).await;

    let placement = Placement::new(format!("engine-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("keeper");
    // a replica that may complete one entry in a round: the head is the one that fails
    let pass = ResetPass::new(
        scripted(&f, &answers),
        placement,
        ResetSettings {
            batch: 1,
            ..ResetSettings::default()
        },
    );
    let mut errors = 0;
    for _ in 0..4 {
        let round = pass.round().await;
        errors += round.errors;
        if outcome(&f, good).await.0 == "CREATED" {
            break;
        }
    }
    assert_eq!(
        outcome(&f, good).await.0,
        "CREATED",
        "the entry behind the one that keeps failing is completed"
    );
    assert_eq!(
        outcome(&f, stuck.reset.id).await.0,
        "PENDING_SOURCE",
        "and the one that fails is still waiting, not written as an outcome"
    );
    assert!(errors >= 1, "it was tried: {errors}");
}

#[tokio::test]
async fn an_entry_that_keeps_failing_does_not_hold_back_the_younger_entries_of_its_own_tenant() {
    let f = fixture().await;
    let answers = answers();
    let bad = open_chain_tagged(&f, "1").await;
    let good = open_chain_tagged(&f, "2").await;
    let stuck = request(&f, bad.run, asking(bad.after_a, true), &key())
        .await
        .expect("one");
    let fine = request(&f, good.run, asking(good.after_a, true), &key())
        .await
        .expect("two");
    end(&f, &bad).await;
    end(&f, &good).await;
    script(&answers, stuck.reset.id, || AppError::Internal);

    let placement = Placement::new(format!("engine-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("keeper");
    let pass = ResetPass::new(
        scripted(&f, &answers),
        placement,
        ResetSettings {
            batch: 1,
            ..ResetSettings::default()
        },
    );
    for _ in 0..4 {
        pass.round().await;
        if outcome(&f, fine.reset.id).await.0 == "CREATED" {
            break;
        }
    }
    assert_eq!(outcome(&f, fine.reset.id).await.0, "CREATED");
    assert_eq!(outcome(&f, stuck.reset.id).await.0, "PENDING_SOURCE");
}

/// What the register keeps of the acknowledgement of an entry.
async fn kept_acknowledgement(f: &Fixture, id: ResetId) -> bool {
    sqlx::query_scalar("SELECT acknowledged_reexecution FROM run_resets WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the entry")
}

// -------------------------------------------------------------------- the acknowledgement (D2008)

#[tokio::test]
async fn a_step_in_flight_when_the_request_is_made_needs_the_acknowledgement_and_nothing_is_cancelled_without_it()
 {
    let f = fixture().await;
    // `b` is leased to a worker: it will usually complete after the cancellation
    let open = open_chain(&f).await;
    let refusal_key = key();
    let refused = request(&f, open.run, asking(open.after_a, false), &refusal_key).await;
    assert!(
        matches!(
            refused,
            Err(AppError::ResetRefused {
                reason: "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
                ..
            })
        ),
        "{refused:?}"
    );
    // before anything: the source is running, no entry, no receipt, no stop request, no audit record
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Running);
    assert_eq!(stops(&f, open.run).await, 0);
    assert_eq!(entries(&f, open.run).await, 0);
    assert!(
        audited(&f, "run.cancel_requested", open.run)
            .await
            .is_empty()
    );
    assert!(audited(&f, "run.reset", open.run).await.is_empty());
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM command_dedupe WHERE idempotency_key = $1")
            .bind(&refusal_key)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("receipts");
    assert_eq!(receipts, 0);

    // the preview of the same point says the same, with no policy to ask: only a source that is open can have a step
    // in flight, and the only way to reset one is to cancel it
    let preview =
        f.w.runs
            .reset_preview(
                &f.w.admin,
                open.run,
                open.after_a,
                ResetReapply::None,
                &ctx(),
            )
            .await
            .expect("the preview");
    assert!(preview.plan.needs_acknowledgement());
    assert!(
        preview
            .plan
            .reexecuted
            .iter()
            .any(|step| step.in_flight && !step.completed_in_source),
        "{:?}",
        preview.plan.reexecuted
    );

    // with it the request is accepted
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request with the acknowledgement");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert!(
        kept_acknowledgement(&f, made.reset.id).await,
        "the entry says what the request said: acknowledged"
    );
}

#[tokio::test]
async fn nothing_in_flight_and_nothing_completed_after_the_point_asks_for_no_acknowledgement() {
    let f = fixture().await;
    // a run nobody has leased a step of: the point is its start, and a cancellation takes everything
    let run = f.w.start(f.workflow, &json!({})).await;
    let preview =
        f.w.runs
            .reset_preview(&f.w.admin, run, 1, ResetReapply::None, &ctx())
            .await
            .expect("the preview");
    assert!(!preview.plan.needs_acknowledgement(), "{:?}", preview.plan);
    let made = request(&f, run, asking(1, false), &key())
        .await
        .expect("accepted without one: no attempt can complete");
    assert_eq!(made.reset.state, ResetState::PendingSource);
    assert!(
        !kept_acknowledgement(&f, made.reset.id).await,
        "the entry says what the request said: not acknowledged"
    );
}

#[tokio::test]
async fn an_entry_whose_source_ended_with_a_step_done_after_the_point_and_no_acknowledgement_fails()
{
    // the re-check at the completion, for an entry as an older request, or a source that moved on, could leave it
    let f = fixture().await;
    let ended = crate::reset::ended_chain(&f).await;
    let point = completed_steps(&f, ended).await[0];
    let digest = digest_of(&f, ended, usize::try_from(point).expect("small")).await;
    let id = plant(
        &f,
        ended,
        i64::try_from(point).expect("small"),
        &digest,
        false,
    )
    .await;
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, id), None)
            .await
            .expect("step"),
        Completed::Failed("REEXECUTION_NOT_ACKNOWLEDGED")
    );
    let (state, failure, new_run, completed) = outcome(&f, id).await;
    assert_eq!(state, "FAILED");
    assert_eq!(failure.as_deref(), Some("REEXECUTION_NOT_ACKNOWLEDGED"));
    assert_eq!(new_run, None);
    assert!(completed);
    assert!(made_from(&f, ended).await.is_empty());

    // the failure is audited by the engine with the closed reason, the id of the reset to join it with the request,
    // and not the justification
    let failed = audited(&f, "run.reset", ended).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(
        failed[0],
        format!("completes reset {id}: failed REEXECUTION_NOT_ACKNOWLEDGED")
    );
    assert!(!failed[0].contains(WHY));
    let actor: String = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE action = 'run.reset' AND resource_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(ended.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("actor");
    assert_ne!(
        actor,
        f.w.admin.subject().as_str(),
        "the platform ends the entry"
    );
    assert_eq!(
        completer(&f)
            .step(&due_of(&f, id), None)
            .await
            .expect("again"),
        Completed::Idle
    );
}

// ----------------------------------------------------------------- the justification is the request (D2010)

#[tokio::test]
async fn the_same_key_with_another_justification_is_not_the_same_request_in_either_path() {
    let f = fixture().await;

    // a source that has not ended: the receipt of the request
    let open = open_chain(&f).await;
    let reset_key = key();
    let first = request(&f, open.run, asking(open.after_a, true), &reset_key)
        .await
        .expect("the request");
    let other = ResetRequest {
        reason: "another reason for the very same reset",
        ..asking(open.after_a, true)
    };
    let refused = request(&f, open.run, other, &reset_key).await;
    assert!(
        matches!(refused, Err(AppError::IdempotencyKeyReused)),
        "{refused:?}"
    );
    // the spaces around the justification are not a justification (it is trimmed before it is judged and audited)
    let spaced = ResetRequest {
        reason: "  the third step used a wrong price list  ",
        ..asking(open.after_a, true)
    };
    let same = request(&f, open.run, spaced, &reset_key)
        .await
        .expect("the same request");
    assert_eq!(same.reset.id, first.reset.id);
    assert_eq!(entries(&f, open.run).await, 1);
    assert_eq!(
        audited(&f, "run.reset", open.run).await.len(),
        1,
        "the second justification was never audited"
    );

    // a source that has ended: the receipt of the run
    let ended = crate::reset::ended_chain(&f).await;
    let point = completed_steps(&f, ended).await[0];
    let ended_key = key();
    let made = request(
        &f,
        ended,
        ResetRequest {
            source_policy: ResetSourcePolicy::RequireTerminal,
            ..asking(point, true)
        },
        &ended_key,
    )
    .await
    .expect("the reset");
    assert_eq!(made.reset.state, ResetState::Created);
    for policy in [
        ResetSourcePolicy::RequireTerminal,
        ResetSourcePolicy::CancelSource,
    ] {
        let refused = request(
            &f,
            ended,
            ResetRequest {
                reason: "another reason for the very same reset",
                source_policy: policy,
                ..asking(point, true)
            },
            &ended_key,
        )
        .await;
        assert!(
            matches!(refused, Err(AppError::IdempotencyKeyReused)),
            "{policy:?}: {refused:?}"
        );
    }
    assert_eq!(made_from(&f, ended).await.len(), 1);
}

// ---------------------------------------------------------- the register says what was asked (D2011)

#[tokio::test]
async fn the_register_keeps_the_policy_and_the_acknowledgement_that_were_asked_for_a_source_that_ended()
 {
    let f = fixture().await;
    let ended = crate::reset::ended_chain(&f).await;
    let steps = completed_steps(&f, ended).await;
    // (the point, the policy, the acknowledgement): the first point re-executes steps that completed, so it needs one;
    // the last re-executes nothing, so a request may do without
    for (point, policy, acknowledged, name) in [
        (
            steps[0],
            ResetSourcePolicy::CancelSource,
            true,
            "CANCEL_SOURCE",
        ),
        (
            steps[2],
            ResetSourcePolicy::RequireTerminal,
            false,
            "REQUIRE_TERMINAL",
        ),
        (
            steps[2],
            ResetSourcePolicy::CancelSource,
            false,
            "CANCEL_SOURCE",
        ),
    ] {
        let made = request(
            &f,
            ended,
            ResetRequest {
                source_policy: policy,
                business: RerunBusinessId::Drop,
                ..asking(point, acknowledged)
            },
            &key(),
        )
        .await
        .expect("an ended source is reset at once");
        let row: (String, bool) = sqlx::query_as(
            "SELECT source_policy, acknowledged_reexecution FROM run_resets WHERE id = $1",
        )
        .bind(made.reset.id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the entry");
        assert_eq!(row, (name.to_owned(), acknowledged), "{name} at {point}");
        assert_eq!(made.reset.source_policy, Some(policy));
    }
}
#[tokio::test]
async fn a_fence_for_another_shard_than_the_entrys_is_a_defect_and_not_a_race() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;
    let slow = replica(&f, "wrong-shard");
    slow.keeper.round().await.expect("keeper");
    let own = Shard::of(made.reset.id.as_uuid());
    let other = Shard::all()
        .find(|shard| shard.index() != own.index())
        .expect("another shard");
    let lease = slow
        .placement
        .snapshot()
        .lease_of(other, std::time::Instant::now())
        .expect("it holds every shard");
    let fence = ShardFence {
        owner: slow.placement.owner().to_owned(),
        lease,
    };
    let step = completer(&f)
        .step(&due_of(&f, made.reset.id), Some(fence))
        .await;
    assert!(
        matches!(step, Err(AppError::StateConflict(_))),
        "an error to look at, not a refusal that changed nothing: {step:?}"
    );
    assert_eq!(outcome(&f, made.reset.id).await.0, "PENDING_SOURCE");
}

// ------------------------------------------------------------------ the scan and the pass (D2006)

/// A replica that holds every shard and runs the pass over `answers` with `settings`.
async fn scripted_replica(
    f: &Fixture,
    answers: &Answers,
    settings: ResetSettings,
) -> ResetPass<PgRunStore, Scripted, PgIdentityStore, SystemClock> {
    let placement = Placement::new(format!("engine-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("keeper");
    ResetPass::new(scripted(f, answers), placement, settings)
}

/// Three entries of the first tenant and one of the other, all due: the oldest first.
async fn entries_of_two_tenants(f: &Fixture) -> (Vec<ResetId>, ResetId) {
    let ended = crate::reset::ended_chain(f).await;
    let point = completed_steps(f, ended).await[0];
    let digest = digest_of(f, ended, usize::try_from(point).expect("small")).await;
    let mut ours = Vec::new();
    for _ in 0..3 {
        ours.push(
            plant(
                f,
                ended,
                i64::try_from(point).expect("small"),
                &digest,
                true,
            )
            .await,
        );
    }
    let theirs = other_tenant_entry(f).await;
    (ours, theirs)
}

#[tokio::test]
async fn the_scan_takes_at_most_its_share_of_each_tenant_and_leaves_out_those_it_is_told_to() {
    let f = fixture().await;
    let (ours, theirs) = entries_of_two_tenants(&f).await;
    let all: Vec<Shard> = Shard::all().collect();
    let scan = |limit: u32, per_tenant: u32, skipping: Vec<ResetId>| {
        let store = f.w.store.clone();
        let all = all.clone();
        async move {
            store
                .due_resets(&all, limit, per_tenant, &skipping)
                .await
                .expect("the scan")
                .into_iter()
                .map(|due| due.id)
                .collect::<Vec<_>>()
        }
    };
    // the share of one tenant is one: its oldest entry, and the other tenant's
    let one_each = scan(10, 1, vec![]).await;
    assert_eq!(one_each.len(), 2, "{one_each:?}");
    assert!(one_each.contains(&ours[0]) && one_each.contains(&theirs));
    // and with the room for the whole of it, all four, oldest first
    let everything = scan(10, 10, vec![]).await;
    assert_eq!(everything.len(), 4);
    assert_eq!(everything[..3], ours[..], "the older entries come first");
    // the batch bounds the whole
    assert_eq!(scan(2, 10, vec![]).await, ours[..2]);
    // an entry named is left out, and the next of its tenant takes its place in the share
    let without_first = scan(10, 1, vec![ours[0]]).await;
    assert!(without_first.contains(&ours[1]) && !without_first.contains(&ours[0]));
    assert_eq!(without_first.len(), 2);
    // every one of them left out leaves nothing
    let mut all_ids = ours.clone();
    all_ids.push(theirs);
    assert!(scan(10, 10, all_ids).await.is_empty());
}

#[tokio::test]
async fn the_pass_leaves_an_entry_that_failed_until_its_time_and_forgets_it_once_it_is_made() {
    let f = fixture().await;
    let answers = answers();
    let open = open_chain(&f).await;
    let made = request(&f, open.run, asking(open.after_a, true), &key())
        .await
        .expect("the request");
    end(&f, &open).await;
    script(&answers, made.reset.id, || AppError::Unavailable);
    let pass = scripted_replica(&f, &answers, ResetSettings::default()).await;

    let t0 = std::time::Instant::now();
    let at = |seconds: u64| t0 + StdDuration::from_secs(seconds);
    // it fails: tried again after 2 s
    let first = pass.round_at(t0).await;
    assert_eq!((first.due, first.errors, first.backing_off), (1, 1, 0));
    let second = pass.round_at(at(1)).await;
    assert_eq!(
        (second.due, second.errors, second.backing_off),
        (0, 0, 1),
        "left out of the scan until its time: {second:?}"
    );
    // its time has come, and it fails again: now 4 s
    let third = pass.round_at(at(2)).await;
    assert_eq!((third.due, third.errors), (1, 1), "{third:?}");
    assert_eq!(pass.round_at(at(5)).await.due, 0, "4 s from the second");
    assert_eq!(pass.round_at(at(6)).await.due, 1, "its time has come");
    // the platform recovers: the entry is made at its next time, and what was remembered of it is gone
    answers.lock().expect("the answers").clear();
    let later = pass.round_at(at(1_000)).await;
    assert_eq!(
        (later.due, later.created, later.errors),
        (1, 1, 0),
        "{later:?}"
    );
    assert_eq!(outcome(&f, made.reset.id).await.0, "CREATED");
    assert_eq!(pass.round_at(at(1_001)).await.backing_off, 0, "forgotten");
}

#[tokio::test]
async fn every_line_the_pass_logs_names_the_tenant_and_the_reset() {
    let f = fixture().await;
    let answers = answers();
    let ended = crate::reset::ended_chain(&f).await;
    let point = i64::try_from(completed_steps(&f, ended).await[0]).expect("small");
    let digest = digest_of(&f, ended, usize::try_from(point).expect("small")).await;
    let failing = plant(&f, ended, point, &digest, true).await;
    let ending = plant(&f, ended, point, &digest, true).await;
    let racing = plant(&f, ended, point, &digest, true).await;
    script(&answers, failing, || AppError::Internal);
    script(&answers, ending, || AppError::Forbidden {
        permission: sagat_domain::identity::Permission::RunStart,
    });
    script(&answers, racing, || {
        AppError::StateConflict(SHARD_LEASE_LOST)
    });
    let pass = scripted_replica(&f, &answers, ResetSettings::default()).await;

    let (_guard, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);
    let round = pass.round().await;
    assert_eq!(
        (round.errors, round.failed_entries, round.refused),
        (1, 1, 1),
        "{round:?}"
    );
    let text = logs.text();
    let tenant = f.w.admin.tenant_id().to_string();
    for (id, what) in [
        (failing, "could not be completed"),
        (ending, "failed"),
        (racing, "was refused"),
    ] {
        let line = text
            .lines()
            .find(|line| line.contains(&id.to_string()))
            .unwrap_or_else(|| panic!("{what}: no line names the reset {id}\n{text}"));
        assert!(line.contains(&tenant), "{what}: {line}");
        assert!(
            line.contains("reset_id") && line.contains("tenant_id"),
            "{what}: {line}"
        );
    }
    // and no line of the pass is without them
    for line in text
        .lines()
        .filter(|line| line.contains("sagat_scheduler::resets"))
    {
        assert!(
            line.contains("tenant_id") && line.contains("reset_id"),
            "a line of the pass without the ids: {line}"
        );
    }
    // the entry that was failed went through the failure path under a valid fence: written once, and audited once
    assert_eq!(
        outcome(&f, ending).await.1.as_deref(),
        Some("AUTHORITY_REVOKED")
    );
    let failed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.reset' AND reason = $1",
    )
    .bind(format!(
        "completes reset {ending}: failed AUTHORITY_REVOKED"
    ))
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the audit trail");
    assert_eq!(failed, 1);
}

#[tokio::test]
async fn a_round_takes_no_more_of_a_tenant_than_its_share_though_the_batch_would_hold_more() {
    let f = fixture().await;
    let answers = answers();
    let (ours, theirs) = entries_of_two_tenants(&f).await;
    for id in ours.iter().copied().chain([theirs]) {
        script(&answers, id, || AppError::Unavailable);
    }
    // room for a hundred entries, a share of one per tenant: three entries of one tenant and one of the other are due
    let pass = scripted_replica(
        &f,
        &answers,
        ResetSettings {
            batch: 100,
            per_tenant: 1,
            ..ResetSettings::default()
        },
    )
    .await;
    let round = pass.round().await;
    assert_eq!((round.due, round.errors), (2, 2), "{round:?}");
    // the two the round left alone are not backed off from: they were not tried
    assert_eq!(round.backing_off, 0, "{round:?}");
}

// ---------------------------------------------------------------------------- the points of a request (N)

#[tokio::test]
async fn a_sequence_past_the_ceiling_and_one_past_the_journal_are_refused_with_their_own_reasons() {
    let f = fixture().await;
    let open = open_chain(&f).await;
    let journal = i64::try_from(f.w.journal(open.run).await.len()).expect("small");
    let past_the_ceiling = request(&f, open.run, asking(10_000, true), &key()).await;
    assert!(
        matches!(
            past_the_ceiling,
            Err(AppError::ResetRefused {
                reason: "RESET_PREFIX_TOO_LONG",
                ..
            })
        ),
        "{past_the_ceiling:?}"
    );
    // really past the journal, and under the ceiling: not a point
    let past_the_journal = request(
        &f,
        open.run,
        asking(u64::try_from(journal + 10).expect("small"), true),
        &key(),
    )
    .await;
    assert!(
        matches!(
            past_the_journal,
            Err(AppError::ResetRefused {
                reason: "RESET_POINT_NOT_ELIGIBLE",
                ..
            })
        ),
        "{past_the_journal:?}"
    );
    assert_eq!(stops(&f, open.run).await, 0);
    assert_eq!(entries(&f, open.run).await, 0);
}

// ------------------------------------------------------------------------------------------- the settings

fn served(extra: &[(&str, &str)]) -> sagat_control_plane::config::ServeConfig {
    let mut values = vec![
        ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
        ("SAGAT_OIDC_AUDIENCE", "sagat"),
        ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
    ];
    values.extend_from_slice(extra);
    let lookup = move |name: &str| {
        values
            .iter()
            .rev()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    };
    sagat_control_plane::config::ServeConfig::from_lookup(&lookup).expect("a configuration")
}

#[tokio::test]
async fn the_service_the_control_plane_builds_refuses_the_cancel_of_a_source_unless_the_operator_set_it()
 {
    // the default of the test support is that it is on, so that the suites can exercise it; what the production
    // default is, is decided here, by the function the control plane builds its service with
    let f = fixture().await;
    let open = open_chain(&f).await;
    let cache: DefinitionCache = f.w.cache.clone();
    for off in [None, Some("false"), Some("0"), Some("no"), Some("")] {
        let extra: Vec<(&str, &str)> = off
            .map(|v| ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", v))
            .into_iter()
            .collect();
        let service =
            sagat_control_plane::run_service(f.w.store.clone(), cache.clone(), &served(&extra));
        let refused = service
            .reset(
                &f.w.admin,
                open.run,
                asking(open.after_a, true),
                &key(),
                &ctx(),
            )
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_CANCEL_SOURCE_NOT_AVAILABLE",
                    ..
                })
            ),
            "{off:?}: {refused:?}"
        );
    }
    assert_eq!(f.w.run_status(open.run).await, RunStatus::Running);
    for on in ["true", "1", "yes"] {
        let service = sagat_control_plane::run_service(
            f.w.store.clone(),
            cache.clone(),
            &served(&[("SAGAT_RESET_CANCEL_SOURCE_ENABLED", on)]),
        );
        let fresh = open_chain_tagged(&f, &format!("on{}", on.len())).await;
        let made = service
            .reset(
                &f.w.admin,
                fresh.run,
                asking(fresh.after_a, true),
                &key(),
                &ctx(),
            )
            .await
            .unwrap_or_else(|error| panic!("{on}: {error:?}"));
        assert_eq!(made.reset.state, ResetState::PendingSource, "{on}");
    }
}
