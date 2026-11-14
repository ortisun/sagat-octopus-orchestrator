//! Signals and Updates delivered again by a reset (`T018-056`, `FR-018-039`): the signals the source received
//! after the point and the Updates whose declaration kept their arguments are delivered to the new run in the
//! transaction that creates it, in journal order, and the report says what came of each. Real PostgreSQL, the
//! real run service.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::execution::Engine;
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::runs::RunStore as _;
use sagat_application::runs::{
    ReapplyEntry, ReportEntry, RerunBusinessId, ResetMade, ResetRequest, RunService,
};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{ResetReapply, RunId, RunStatus, StepStatus};
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::business_ids::key;
use crate::firing::{Fixture, fixture};
use crate::reset::{finish, request};
use crate::retained_arguments::{apply, arguments};
use crate::support::engine::{
    DECLARED_OPERATION as OPERATION, activity, ctx, declared_waiter, document, wait_node, with,
};

const ONE: &str = "reapply.one";
const TWO: &str = "reapply.two";
const THREE: &str = "reapply.three";

/// Three activities with a wait for a signal in front of each but the first: `a`, `one`, `b`, `two`, `c`,
/// `three`, `d`, with a declared state and one Update that opts in to the retention of its arguments (or not).
fn flow(name: &str, opt_in: bool) -> Value {
    let mut definition = document(
        name,
        "a",
        vec![
            with(activity("a"), "next", json!("w1")),
            with(
                wait_node("w1", ONE, "input.k", 300_000, None),
                "next",
                json!("b"),
            ),
            with(activity("b"), "next", json!("w2")),
            with(
                wait_node("w2", TWO, "input.k", 300_000, None),
                "next",
                json!("c"),
            ),
            with(activity("c"), "next", json!("w3")),
            with(
                wait_node("w3", THREE, "input.k", 300_000, None),
                "next",
                json!("d"),
            ),
            activity("d"),
        ],
    );
    definition["state"] = declared_waiter(name)["state"].clone();
    if opt_in {
        definition["state"]["updates"][OPERATION]["reapplyOnReset"] = json!(true);
    }
    definition
}

async fn publish(f: &Fixture, opt_in: bool) -> WorkflowId {
    sagat_infra_ir::set_reset_reapply(true);
    f.w.publish(flow(&format!("reapply-flow-{}", Uuid::now_v7()), opt_in))
        .await
        .0
}

/// What the run received, so the tests can look for it: a payload that no other row of the tenant has.
fn payload(name: &str) -> Vec<u8> {
    json!({"signal": name, "id": Uuid::now_v7().to_string()})
        .to_string()
        .into_bytes()
}

/// A source run that ended, and where to reset it.
struct Source {
    run: RunId,
    /// The sequence of the completion of `a`.
    point: u64,
    /// The payloads of the three signals, in the order they were sent.
    payloads: [Vec<u8>; 3],
    /// The sequence of the creation Update, which is in every prefix after the start.
    creation_update: u64,
}

/// The source: an Update when it is created (inside every prefix), then after `a` completes the Update 2, the
/// signal one, `b`, the signal two, the Update 3, `c`, the signal three and `d`. So after the point the journal
/// holds, in this order: Update 2, one, two, Update 3, three.
async fn source(f: &Fixture, opt_in: bool) -> Source {
    let workflow = publish(f, opt_in).await;
    let business = format!("reapply-{}", Uuid::now_v7());
    let run = apply(
        f,
        workflow,
        &business,
        "update-key-0001",
        &arguments(true, 11),
    )
    .await;
    finish(f, run, &["a"]).await;
    let point = sequence_of(f, run, "step.completed", Some("a")).await;
    let payloads = [payload("one"), payload("two"), payload("three")];
    apply(
        f,
        workflow,
        &business,
        "update-key-0002",
        &arguments(true, 12),
    )
    .await;
    signal(f, run, ONE, &payloads[0]).await;
    finish(f, run, &["b"]).await;
    signal(f, run, TWO, &payloads[1]).await;
    apply(
        f,
        workflow,
        &business,
        "update-key-0003",
        &arguments(true, 13),
    )
    .await;
    finish(f, run, &["c"]).await;
    signal(f, run, THREE, &payloads[2]).await;
    finish(f, run, &["d"]).await;
    assert_eq!(f.w.run_status(run).await, RunStatus::Succeeded);
    Source {
        creation_update: sequence_of(f, run, "state.updated", None).await,
        run,
        point,
        payloads,
    }
}

async fn signal(f: &Fixture, run: RunId, name: &str, payload: &[u8]) {
    f.w.runs
        .signal(&f.w.admin, run, name, payload, &key(), &ctx())
        .await
        .expect("the signal is delivered");
}

/// The first sequence of an event type (of a node, when given).
async fn sequence_of(f: &Fixture, run: RunId, event_type: &str, node: Option<&str>) -> u64 {
    let seq: i64 = sqlx::query_scalar(
        "SELECT min(seq) FROM run_events \
         WHERE run_id = $1 AND event_type = $2 AND ($3::text IS NULL OR node = $3)",
    )
    .bind(run.as_uuid())
    .bind(event_type)
    .bind(node)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the sequence");
    u64::try_from(seq).expect("small")
}

/// The rows of the source after `after` that a reset may deliver again, as the journal says they are:
/// `(sequence, kind)`.
async fn rows_after(f: &Fixture, run: RunId, after: u64) -> Vec<(u64, &'static str)> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT seq, event_type FROM run_events WHERE run_id = $1 AND seq > $2 AND ( \
             (event_type = 'wait.satisfied' AND detail->>'via' = 'signal') \
             OR (event_type = 'state.updated' AND detail->>'retained' = 'true')) ORDER BY seq",
    )
    .bind(run.as_uuid())
    .bind(i64::try_from(after).expect("small"))
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the rows");
    rows.into_iter()
        .map(|(seq, event_type)| {
            (
                u64::try_from(seq).expect("small"),
                if event_type == "wait.satisfied" {
                    "SIGNAL"
                } else {
                    "UPDATE"
                },
            )
        })
        .collect()
}

/// A request to reset to `point`, delivering again what `reapply` says. The new run does not take over the business
/// identifier: a test makes several resets of one source, and the first would hold it.
fn asked(point: u64, reapply: ResetReapply) -> ResetRequest<'static> {
    ResetRequest {
        reapply,
        business: RerunBusinessId::Drop,
        ..request(point, true)
    }
}

async fn reset(f: &Fixture, source: &Source, reapply: ResetReapply) -> ResetMade {
    f.w.runs
        .reset(
            &f.w.admin,
            source.run,
            asked(source.point, reapply),
            &key(),
            &ctx(),
        )
        .await
        .expect("the reset")
}

/// An entry of a preview or of a stored report, to be read in words.
trait Wordy {
    fn kind(&self) -> String;
    fn subject(&self) -> String;
    fn outcome(&self) -> String;
    fn reason(&self) -> Option<String>;
}

impl Wordy for ReapplyEntry {
    fn kind(&self) -> String {
        self.kind.as_str().to_owned()
    }
    fn subject(&self) -> String {
        self.name.clone()
    }
    fn outcome(&self) -> String {
        self.outcome.as_str().to_owned()
    }
    fn reason(&self) -> Option<String> {
        self.reason.map(|reason| reason.as_str().to_owned())
    }
}

impl Wordy for ReportEntry {
    fn kind(&self) -> String {
        self.kind.clone()
    }
    fn subject(&self) -> String {
        self.subject().unwrap_or_default().to_owned()
    }
    fn outcome(&self) -> String {
        self.outcome.clone()
    }
    fn reason(&self) -> Option<String> {
        self.reason.clone()
    }
}

type Words = Vec<(String, String, String, Option<String>)>;

/// `(kind, name, outcome, reason)` of every entry, in order.
fn words<T: Wordy>(report: &[T]) -> Words {
    report
        .iter()
        .map(|entry| {
            (
                entry.kind(),
                Wordy::subject(entry),
                entry.outcome(),
                entry.reason(),
            )
        })
        .collect()
}

fn entry(
    kind: &'static str,
    name: &str,
    outcome: &'static str,
    reason: Option<&'static str>,
) -> (String, String, String, Option<String>) {
    (
        kind.to_owned(),
        name.to_owned(),
        outcome.to_owned(),
        reason.map(str::to_owned),
    )
}

/// The report the register stores for a reset, as the database has it.
async fn stored_report(f: &Fixture, made: &ResetMade) -> Value {
    sqlx::query_scalar("SELECT reapply_report FROM run_resets WHERE id = $1")
        .bind(made.reset.id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the register")
}

async fn revision(f: &Fixture, run: RunId) -> i64 {
    sqlx::query_scalar("SELECT state_revision FROM workflow_operation_states WHERE run_id = $1")
        .bind(run.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the declared state")
}

async fn events_of(f: &Fixture, run: RunId, event_type: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = $2")
        .bind(run.as_uuid())
        .bind(event_type)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn a_reset_delivers_again_in_journal_order_what_the_source_received_after_the_point() {
    let f = fixture().await;
    let source = source(&f, true).await;

    // the journal after the point holds Update 2, one, two, Update 3, three, in this order
    let rows = rows_after(&f, source.run, source.point).await;
    assert_eq!(
        rows.iter().map(|row| row.1).collect::<Vec<_>>(),
        ["UPDATE", "SIGNAL", "SIGNAL", "UPDATE", "SIGNAL"]
    );

    let made = reset(&f, &source, ResetReapply::All).await;
    let new = made.run.as_ref().expect("the reset made a run").info.run_id;

    // Update 2 and one are delivered; two finds no step waiting (`b` has not run in the new run) and so ends the
    // signals, but not the Updates: Update 3 still goes, and three is not tried
    assert_eq!(
        words(&made.reset.reapply_report),
        [
            entry("UPDATE", OPERATION, "REAPPLIED", None),
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", TWO, "REJECTED", Some("NOT_WAITING")),
            entry("UPDATE", OPERATION, "REAPPLIED", None),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );
    // each entry names the row of the source it is, in the order of the journal
    assert_eq!(
        made.reset
            .reapply_report
            .iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        rows.iter().map(|row| row.0).collect::<Vec<_>>()
    );

    // what the report says happened: the signal woke its wait, the Updates changed the declared state
    assert_eq!(
        f.w.step_status(new, "w1").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(f.w.step_status(new, "b").await, Some(StepStatus::Ready));
    assert_eq!(
        f.w.step_status(new, "w2").await,
        None,
        "two did not reach it"
    );
    assert_eq!(
        revision(&f, new).await,
        3,
        "the creation Update and the two reapplied"
    );
    f.w.assert_consistent(new).await;

    // the signal is a signal of the new run (so a reset of this run may deliver it again), the Updates kept
    // their arguments for the same reason
    let vias: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT detail->>'via' FROM run_events \
         WHERE run_id = $1 AND event_type = 'wait.satisfied' AND detail->>'restored' IS NULL",
    )
    .bind(new.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the journal");
    assert_eq!(vias, [Some("signal".to_owned())]);
    assert_eq!(
        rows_after(&f, new, source.point).await.len(),
        3,
        "the new run's own journal holds Update 2, one and Update 3 as retained candidates"
    );

    // the register keeps what the reset returned, and the reads answer the same
    let stored = stored_report(&f, &made).await;
    assert_eq!(
        sagat_application::runs::report_from_json(&stored),
        made.reset.reapply_report
    );
    assert_eq!(
        stored.as_array().map(Vec::len),
        Some(made.reset.reapply_report.len())
    );
    assert_eq!(
        f.w.runs
            .reset_record(&f.w.admin, source.run, made.reset.id, &ctx())
            .await
            .expect("one reset"),
        made.reset
    );
    assert_eq!(
        f.w.runs
            .resets(&f.w.admin, source.run, &ctx())
            .await
            .expect("the list"),
        vec![made.reset.clone()]
    );

    // a report names the signal or the operation and never the content of either, nor a digest of it
    let text = stored.to_string();
    for payload in &source.payloads {
        let payload = String::from_utf8_lossy(payload);
        assert!(!text.contains(payload.as_ref()), "{text}");
    }
    assert!(
        !text.contains("nextState") && !text.contains("allowed"),
        "{text}"
    );
    assert!(!text.to_lowercase().contains("digest"), "{text}");

    // the new run goes on from where the reapply left it: `b` runs, and two is for the caller to send again
    finish(&f, new, &["b"]).await;
    assert_eq!(f.w.step_status(new, "w2").await, Some(StepStatus::Waiting));
    f.w.assert_consistent(new).await;
}

#[tokio::test]
async fn each_delivery_is_audited_by_the_caller_and_keyed_by_the_reset() {
    let f = fixture().await;
    let source = source(&f, true).await;
    let made = reset(&f, &source, ResetReapply::All).await;
    let new = made.run.as_ref().expect("the reset made a run").info.run_id;

    // the reset, the signal that was delivered and the two Updates that were applied, each once, by the caller
    let audit: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, actor FROM audit_records \
         WHERE tenant_id = $1 AND resource_id = $2 \
           AND action IN ('run.reset', 'run.signalled', 'run.updated') ORDER BY action",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(new.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit");
    let caller = f.w.admin.subject().as_str().to_owned();
    assert_eq!(
        audit,
        [
            ("run.reset".to_owned(), caller.clone()),
            ("run.signalled".to_owned(), caller.clone()),
            ("run.updated".to_owned(), caller.clone()),
            ("run.updated".to_owned(), caller),
        ]
    );
    // The reset is audited with the justification the caller gave (that is the contract) and the closed word of the
    // policy; the deliveries are audited by what they are and share the trace of the request, which is the exact
    // join between them and the reset (they do not name the reset or the policy: G1985). Never the content.
    let records: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT action, reason, trace_id FROM audit_records \
         WHERE tenant_id = $1 AND resource_id = $2 \
           AND action IN ('run.reset', 'run.signalled', 'run.updated') ORDER BY action, id",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(new.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit");
    let reset_reason = records[0].1.clone().unwrap_or_default();
    assert!(
        reset_reason.contains(" reapply=ALL: ") && reset_reason.ends_with(request(0, false).reason),
        "{reset_reason}"
    );
    assert!(
        records
            .iter()
            .all(|(_, _, trace)| trace.as_deref() == ctx().trace_id.as_deref()),
        "every record of the request carries its trace: {records:?}"
    );
    for (_, reason, _) in &records[1..] {
        let reason = reason.clone().unwrap_or_default();
        assert!(
            !reason.contains("nextState")
                && !reason.contains("paid")
                && !reason.contains("allowed"),
            "{reason}"
        );
    }
    // a reset that asks for nothing says nothing of a policy
    let plain = reset(&f, &source, ResetReapply::None).await;
    let plain_reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE resource_id = $1 AND action = 'run.reset'",
    )
    .bind(
        plain
            .run
            .as_ref()
            .expect("the reset made a run")
            .info
            .run_id
            .to_string(),
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert!(
        !plain_reason.unwrap_or_default().contains("reapply="),
        "a reset with no reapply carries no policy"
    );
    // the signal that nobody waited for left no audit record: it was not delivered
    let reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE resource_id = $1 AND action = 'run.signalled'",
    )
    .bind(new.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(reasons, [Some(format!("signal '{ONE}'"))]);

    // the keys are the reset's and the candidate's sequence, in the prefix callers cannot use
    let sequences: Vec<u64> = made
        .reset
        .reapply_report
        .iter()
        .map(|entry| entry.sequence)
        .collect();
    let signal_keys: Vec<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM command_dedupe WHERE scope = $1 ORDER BY idempotency_key",
    )
    .bind(format!("run.signal:{new}"))
    .fetch_all(&f.w.db.pool)
    .await
    .expect("keys");
    assert_eq!(
        signal_keys,
        [format!("reset:{}:{}", made.reset.id, sequences[1])]
    );
    let mut update_keys: Vec<String> = sqlx::query_scalar(
        "SELECT update_key FROM operation_update_receipts WHERE resolved_run_id = $1",
    )
    .bind(new.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("keys");
    update_keys.sort();
    let mut expected = vec![
        format!("reset:{}:{}", made.reset.id, sequences[0]),
        format!("reset:{}:{}", made.reset.id, sequences[3]),
    ];
    expected.sort();
    assert_eq!(update_keys, expected);
}

#[tokio::test]
async fn the_preview_lists_the_candidates_before_the_reset_and_the_report_lists_those_after() {
    let f = fixture().await;
    let source = source(&f, true).await;

    let preview =
        f.w.runs
            .reset_preview(
                &f.w.admin,
                source.run,
                source.point,
                ResetReapply::All,
                &ctx(),
            )
            .await
            .expect("the preview");
    // nothing has been tried: every candidate that can be is `WILL_TRY`
    assert_eq!(
        words(&preview.reapply),
        [
            entry("UPDATE", OPERATION, "WILL_TRY", None),
            entry("SIGNAL", ONE, "WILL_TRY", None),
            entry("SIGNAL", TWO, "WILL_TRY", None),
            entry("UPDATE", OPERATION, "WILL_TRY", None),
            entry("SIGNAL", THREE, "WILL_TRY", None),
        ]
    );
    // and a preview changes nothing
    assert!(
        f.w.runs
            .resets(&f.w.admin, source.run, &ctx())
            .await
            .expect("the list")
            .is_empty()
    );

    let made = reset(&f, &source, ResetReapply::All).await;
    let report = &made.reset.reapply_report;
    assert_eq!(report.len(), preview.reapply.len());
    for (planned, done) in preview.reapply.iter().zip(report) {
        assert_eq!(
            (
                planned.sequence,
                planned.kind.as_str(),
                Some(planned.name.as_str())
            ),
            (done.sequence, done.kind.as_str(), done.subject()),
            "the same candidates, in the same order"
        );
        // nothing was held back here, so everything the preview said it would try was tried
        assert_eq!(planned.outcome.as_str(), "WILL_TRY");
    }
}

#[tokio::test]
async fn the_policy_chooses_what_is_delivered_again() {
    let f = fixture().await;
    let source = source(&f, true).await;

    let none = reset(&f, &source, ResetReapply::None).await;
    assert!(none.reset.reapply_report.is_empty());
    assert_eq!(
        revision(
            &f,
            none.run.as_ref().expect("the reset made a run").info.run_id
        )
        .await,
        1,
        "nothing delivered"
    );
    assert_eq!(
        events_of(
            &f,
            none.run.as_ref().expect("the reset made a run").info.run_id,
            "state.updated"
        )
        .await,
        1
    );
    assert_eq!(
        f.w.step_status(
            none.run.as_ref().expect("the reset made a run").info.run_id,
            "w1"
        )
        .await,
        Some(StepStatus::Waiting)
    );

    // signals only: the Updates are not even listed, and the declared state is as the prefix left it
    let signals = reset(&f, &source, ResetReapply::Signals).await;
    assert_eq!(
        words(&signals.reset.reapply_report),
        [
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", TWO, "REJECTED", Some("NOT_WAITING")),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );
    assert_eq!(
        revision(
            &f,
            signals
                .run
                .as_ref()
                .expect("the reset made a run")
                .info
                .run_id
        )
        .await,
        1
    );

    // Updates only: both are applied, one after the other, to the state each was applied to in the source; the
    // signals are not listed, and the run waits where the prefix left it
    let updates = reset(&f, &source, ResetReapply::Updates).await;
    assert_eq!(
        words(&updates.reset.reapply_report),
        [
            entry("UPDATE", OPERATION, "REAPPLIED", None),
            entry("UPDATE", OPERATION, "REAPPLIED", None),
        ]
    );
    assert_eq!(
        revision(
            &f,
            updates
                .run
                .as_ref()
                .expect("the reset made a run")
                .info
                .run_id
        )
        .await,
        3
    );
    assert_eq!(
        f.w.step_status(
            updates
                .run
                .as_ref()
                .expect("the reset made a run")
                .info
                .run_id,
            "w1"
        )
        .await,
        Some(StepStatus::Waiting)
    );

    // three resets of one source are three entries of its register
    assert_eq!(
        f.w.runs
            .resets(&f.w.admin, source.run, &ctx())
            .await
            .expect("the list")
            .len(),
        3
    );
    for made in [&none, &signals, &updates] {
        f.w.assert_consistent(made.run.as_ref().expect("the reset made a run").info.run_id)
            .await;
    }
}

#[tokio::test]
async fn an_update_whose_declaration_does_not_opt_in_is_not_a_candidate() {
    let f = fixture().await;
    let source = source(&f, false).await;

    // the journal says the arguments were not kept, and so nothing is listed or applied
    assert_eq!(
        rows_after(&f, source.run, source.point)
            .await
            .iter()
            .map(|row| row.1)
            .collect::<Vec<_>>(),
        ["SIGNAL", "SIGNAL", "SIGNAL"]
    );
    let updates = reset(&f, &source, ResetReapply::Updates).await;
    assert!(updates.reset.reapply_report.is_empty());
    assert_eq!(
        revision(
            &f,
            updates
                .run
                .as_ref()
                .expect("the reset made a run")
                .info
                .run_id
        )
        .await,
        1
    );

    // all of it: only the signals are candidates
    let all = reset(&f, &source, ResetReapply::All).await;
    assert_eq!(
        words(&all.reset.reapply_report),
        [
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", TWO, "REJECTED", Some("NOT_WAITING")),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );
}

/// Makes the rows of the source after the point say what an older version wrote: no `via` on the wait that the
/// second signal satisfied, no `retained` on the third Update. The journal is append-only; the test lifts that
/// for its own database and puts it back.
async fn age_rows(f: &Fixture, source: &Source) {
    let mut conn = f.w.db.pool.acquire().await.expect("a connection");
    sqlx::query("ALTER TABLE run_events DISABLE TRIGGER run_events_append_only")
        .execute(&mut *conn)
        .await
        .expect("lift the guard");
    let aged = sqlx::query(
        "UPDATE run_events SET detail = detail - 'via' \
         WHERE run_id = $1 AND event_type = 'wait.satisfied' AND node = 'w2'",
    )
    .bind(source.run.as_uuid())
    .execute(&mut *conn)
    .await
    .expect("age the wait")
    .rows_affected();
    assert_eq!(aged, 1);
    let aged = sqlx::query(
        "UPDATE run_events SET detail = detail - 'retained' \
         WHERE run_id = $1 AND event_type = 'state.updated' AND seq = ( \
             SELECT max(seq) FROM run_events WHERE run_id = $1 AND event_type = 'state.updated')",
    )
    .bind(source.run.as_uuid())
    .execute(&mut *conn)
    .await
    .expect("age the Update")
    .rows_affected();
    assert_eq!(aged, 1);
    sqlx::query("ALTER TABLE run_events ENABLE ALWAYS TRIGGER run_events_append_only")
        .execute(&mut *conn)
        .await
        .expect("put the guard back");
}

#[tokio::test]
async fn a_row_that_does_not_say_what_it_was_is_a_legacy_event_and_ends_the_signals() {
    let f = fixture().await;
    let source = source(&f, true).await;
    age_rows(&f, &source).await;

    let preview =
        f.w.runs
            .reset_preview(
                &f.w.admin,
                source.run,
                source.point,
                ResetReapply::All,
                &ctx(),
            )
            .await
            .expect("the preview");
    // the preview already knows what the journal does not say
    assert_eq!(
        words(&preview.reapply),
        [
            entry("UPDATE", OPERATION, "WILL_TRY", None),
            entry("SIGNAL", ONE, "WILL_TRY", None),
            entry("SIGNAL", TWO, "NOT_REAPPLIED", Some("LEGACY_EVENT")),
            entry("UPDATE", OPERATION, "NOT_REAPPLIED", Some("LEGACY_EVENT")),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );

    let made = reset(&f, &source, ResetReapply::All).await;
    assert_eq!(
        words(&made.reset.reapply_report),
        [
            entry("UPDATE", OPERATION, "REAPPLIED", None),
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", TWO, "NOT_REAPPLIED", Some("LEGACY_EVENT")),
            entry("UPDATE", OPERATION, "NOT_REAPPLIED", Some("LEGACY_EVENT")),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );
    assert_eq!(
        revision(
            &f,
            made.run.as_ref().expect("the reset made a run").info.run_id
        )
        .await,
        2
    );
    // what the preview already knew is what the report says, line for line: the entries it held back are the same
    // lines in both (this is the case where some are held back; the case where none is, is the preview test's)
    let held_back: Vec<ReportEntry> = preview
        .reapply
        .iter()
        .filter(|planned| planned.outcome.as_str() == "NOT_REAPPLIED")
        .map(ReportEntry::from)
        .collect();
    assert_eq!(held_back.len(), 3, "{held_back:?}");
    for line in &held_back {
        assert!(
            made.reset.reapply_report.contains(line),
            "{line:?} is in the preview and not in the report"
        );
    }
    f.w.assert_consistent(made.run.as_ref().expect("the reset made a run").info.run_id)
        .await;
}

/// Removes, from the tenant, the payload of the first signal and the arguments of the first Update after the
/// point: what a retention that purged content would leave. The foreign keys that keep content from being
/// removed are lifted for this transaction alone.
async fn forget_content(f: &Fixture, source: &Source) {
    let mut tx = f.w.db.pool.begin().await.expect("a transaction");
    lift_guards(&mut tx).await;
    let gone = sqlx::query("DELETE FROM payloads WHERE tenant_id = $1 AND content = $2")
        .bind(f.w.admin.tenant_id().as_uuid())
        .bind(&source.payloads[0])
        .execute(&mut *tx)
        .await
        .expect("forget the signal")
        .rows_affected();
    assert_eq!(gone, 1, "the payload of the first signal");
    let cleared = sqlx::query(
        "UPDATE operation_update_receipts SET arguments_digest = NULL \
         WHERE resolved_run_id = $1 AND run_version = ( \
             SELECT min(seq) FROM run_events WHERE run_id = $1 AND event_type = 'state.updated' \
                                               AND seq > $2)",
    )
    .bind(source.run.as_uuid())
    .bind(i64::try_from(source.point).expect("small"))
    .execute(&mut *tx)
    .await
    .expect("forget the arguments")
    .rows_affected();
    assert_eq!(cleared, 1, "the receipt of the second Update");
    tx.commit().await.expect("commit");
    put_guards_back(f).await;
}

/// Lifts, in the transaction, what keeps the content and the receipts of a run from being changed, and the
/// foreign keys that keep content from being removed: what a retention that purged content would do. Each test
/// has a database of its own; the guards are put back once the transaction has committed.
async fn lift_guards(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) {
    for statement in [
        "ALTER TABLE payloads DISABLE TRIGGER payloads_append_only",
        "ALTER TABLE operation_update_receipts DISABLE TRIGGER operation_update_receipts_immutable",
        "SET LOCAL session_replication_role = replica",
    ] {
        sqlx::query(statement)
            .execute(&mut **tx)
            .await
            .expect("lift a guard");
    }
}

async fn put_guards_back(f: &Fixture) {
    for statement in [
        "ALTER TABLE payloads ENABLE ALWAYS TRIGGER payloads_append_only",
        "ALTER TABLE operation_update_receipts ENABLE ALWAYS TRIGGER operation_update_receipts_immutable",
    ] {
        sqlx::query(statement)
            .execute(&f.w.db.pool)
            .await
            .expect("put a guard back");
    }
}

#[tokio::test]
async fn content_that_is_gone_is_not_retained_and_the_update_that_followed_it_finds_another_state()
{
    let f = fixture().await;
    let source = source(&f, true).await;
    forget_content(&f, &source).await;

    let made = reset(&f, &source, ResetReapply::All).await;
    // Update 2 and signal one cannot be delivered; two must not reach the wait one was meant for; Update 3 is
    // tried, and is refused because the state is not at the revision it was applied to
    assert_eq!(
        words(&made.reset.reapply_report),
        [
            entry("UPDATE", OPERATION, "NOT_REAPPLIED", Some("NOT_RETAINED")),
            entry("SIGNAL", ONE, "NOT_REAPPLIED", Some("NOT_RETAINED")),
            entry(
                "SIGNAL",
                TWO,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
            entry("UPDATE", OPERATION, "REJECTED", Some("REVISION_CONFLICT")),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );
    let new = made.run.as_ref().expect("the reset made a run").info.run_id;
    assert_eq!(
        revision(&f, new).await,
        1,
        "nothing changed the declared state"
    );
    assert_eq!(f.w.step_status(new, "w1").await, Some(StepStatus::Waiting));
    // a rejected Update is no state change
    assert_eq!(events_of(&f, new, "state.updated").await, 1);
    f.w.assert_consistent(new).await;

    // the preview sees the same content
    let preview =
        f.w.runs
            .reset_preview(
                &f.w.admin,
                source.run,
                source.point,
                ResetReapply::All,
                &ctx(),
            )
            .await
            .expect("the preview");
    assert_eq!(
        words(&preview.reapply)[..2],
        [
            entry("UPDATE", OPERATION, "NOT_REAPPLIED", Some("NOT_RETAINED")),
            entry("SIGNAL", ONE, "NOT_REAPPLIED", Some("NOT_RETAINED")),
        ]
    );
}

#[tokio::test]
async fn an_update_the_validator_declines_and_one_the_declaration_does_not_accept_are_rejected_and_the_rest_still_goes()
 {
    let f = fixture().await;
    let source = source(&f, true).await;

    // Neither could have been kept by the source: a validator that reads more than the arguments, or a declaration
    // that changed, would answer so in the new run. Arguments the validator declines are kept in the place of those
    // of Update 2, and arguments the schema of the declaration does not accept (the new state is missing) in the
    // place of those of Update 3.
    let declined = arguments(false, 99);
    let not_accepted =
        DeclaredOperationArguments::parse(br#"{"allowed": true}"#).expect("arguments");
    let mut tx = f.w.db.pool.begin().await.expect("a transaction");
    lift_guards(&mut tx).await;
    for (replacement, which) in [(&declined, "min"), (&not_accepted, "max")] {
        let digest = {
            use sha2::Digest as _;
            sha2::Sha256::digest(replacement.canonical()).to_vec()
        };
        sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3)")
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(&digest)
            .bind(replacement.canonical())
            .execute(&mut *tx)
            .await
            .expect("the arguments");
        // the first Update after the point, then the last one
        let aggregate = if which == "min" {
            "min(seq) FROM run_events WHERE run_id = $1 AND event_type = 'state.updated' AND seq > $2"
        } else {
            "max(seq) FROM run_events WHERE run_id = $1 AND event_type = 'state.updated' AND seq > $2"
        };
        let swapped = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE operation_update_receipts SET arguments_digest = $3 \
             WHERE resolved_run_id = $1 AND run_version = (SELECT {aggregate})"
        )))
        .bind(source.run.as_uuid())
        .bind(i64::try_from(source.point).expect("small"))
        .bind(&digest)
        .execute(&mut *tx)
        .await
        .expect("swap")
        .rows_affected();
        assert_eq!(swapped, 1);
    }
    tx.commit().await.expect("commit");
    put_guards_back(&f).await;

    let made = reset(&f, &source, ResetReapply::All).await;
    // Update 2 is declined, and the signal that follows it is delivered all the same; Update 3 is not accepted by
    // the declaration (which is read before the revision is)
    assert_eq!(
        words(&made.reset.reapply_report),
        [
            entry("UPDATE", OPERATION, "REJECTED", Some("UPDATE_REJECTED")),
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", TWO, "REJECTED", Some("NOT_WAITING")),
            entry("UPDATE", OPERATION, "REJECTED", Some("UPDATE_NOT_ACCEPTED")),
            entry(
                "SIGNAL",
                THREE,
                "NOT_REAPPLIED",
                Some("PREVIOUS_SIGNAL_NOT_DELIVERED")
            ),
        ]
    );
    let new = made.run.as_ref().expect("the reset made a run").info.run_id;
    assert_eq!(
        f.w.step_status(new, "w1").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(revision(&f, new).await, 1);
    // the validator's refusal left a receipt (so a retry of the Update is the same answer); the declaration's left
    // none: it was refused before it was decided
    let rejected: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operation_update_receipts WHERE resolved_run_id = $1 AND status = 'REJECTED'",
    )
    .bind(new.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count");
    assert_eq!(rejected, 1);
    f.w.assert_consistent(new).await;
}

#[tokio::test]
async fn what_a_signal_with_start_and_a_published_event_satisfied_is_not_delivered_again() {
    use crate::with_start::{SIGNAL, signal_with_start, waiter};
    let f = fixture().await;
    let workflow = waiter(&f).await;

    // a run created by a request that also signalled it: the wait it satisfied is `signal_with_start`
    let started = signal_with_start(&f, workflow, "reapply-cart", br#"{"item":"book"}"#, &key())
        .await
        .expect("started and signalled");
    assert!(started.started);
    let run = started.run.info.run_id;
    finish(&f, run, &["notify"]).await;
    assert_eq!(f.w.run_status(run).await, RunStatus::Succeeded);

    let made =
        f.w.runs
            .reset(
                &f.w.admin,
                run,
                asked(1, ResetReapply::Signals),
                &key(),
                &ctx(),
            )
            .await
            .expect("the reset");
    assert!(
        made.reset.reapply_report.is_empty(),
        "{:?}",
        made.reset.reapply_report
    );
    assert_eq!(
        f.w.step_status(
            made.run.as_ref().expect("the reset made a run").info.run_id,
            "w"
        )
        .await,
        Some(StepStatus::Waiting),
        "the signal of a request that also started the run is not sent to the run that replaces it"
    );

    // a wait that a published event satisfied is the event's, not a signal
    let second = f.w.start(workflow, &json!({"k": "e"})).await;
    f.w.events
        .publish(
            &f.w.admin,
            sagat_application::events::PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(
                    sagat_domain::events::EventId::parse("reapply-event-1").expect("id"),
                ),
                event_type: sagat_domain::workflow::EventType::parse(SIGNAL).expect("type"),
                subject: None,
                correlation_key: Some(
                    sagat_domain::execution::CorrelationKey::parse("e").expect("key"),
                ),
                occurred_at: None,
                data: b"{}".to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
    f.w.dispatcher.tick().await;
    finish(&f, second, &["notify"]).await;
    assert_eq!(f.w.run_status(second).await, RunStatus::Succeeded);
    let again =
        f.w.runs
            .reset(
                &f.w.admin,
                second,
                asked(1, ResetReapply::All),
                &key(),
                &ctx(),
            )
            .await
            .expect("the reset");
    assert!(again.reset.reapply_report.is_empty());
}

#[tokio::test]
async fn the_same_request_is_the_same_reset_and_nothing_is_delivered_twice() {
    let f = fixture().await;
    let source = source(&f, true).await;
    let retry_key = key();
    let first =
        f.w.runs
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &retry_key,
                &ctx(),
            )
            .await
            .expect("the reset");
    let new = first
        .run
        .as_ref()
        .expect("the reset made a run")
        .info
        .run_id;
    let counts = |f: &Fixture| {
        let pool = f.w.db.pool.clone();
        async move {
            let journal: i64 =
                sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id = $1")
                    .bind(new.as_uuid())
                    .fetch_one(&pool)
                    .await
                    .expect("count");
            let receipts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM operation_update_receipts WHERE resolved_run_id = $1",
            )
            .bind(new.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count");
            let audited: i64 =
                sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE resource_id = $1")
                    .bind(new.to_string())
                    .fetch_one(&pool)
                    .await
                    .expect("count");
            let runs: i64 =
                sqlx::query_scalar("SELECT count(*) FROM runs WHERE rerun_of IS NOT NULL")
                    .fetch_one(&pool)
                    .await
                    .expect("count");
            (journal, receipts, audited, runs)
        }
    };
    let before = counts(&f).await;

    let again =
        f.w.runs
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &retry_key,
                &ctx(),
            )
            .await
            .expect("the retry");
    assert_eq!(again.reset, first.reset, "the same entry, the same report");
    assert_eq!(
        again
            .run
            .as_ref()
            .expect("the reset made a run")
            .info
            .run_id,
        new
    );
    assert_eq!(
        counts(&f).await,
        before,
        "nothing was delivered a second time"
    );

    // the same key for another policy is another request
    let other =
        f.w.runs
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::Signals),
                &retry_key,
                &ctx(),
            )
            .await;
    assert!(
        matches!(other, Err(AppError::IdempotencyKeyReused)),
        "{other:?}"
    );
    assert_eq!(counts(&f).await, before);
}

#[tokio::test]
async fn a_lost_hint_leaves_the_state_right_and_the_work_findable() {
    let f = fixture().await;
    let source = source(&f, true).await;
    let made = reset(&f, &source, ResetReapply::All).await;
    let new = made.run.as_ref().expect("the reset made a run").info.run_id;
    let expected = words(&made.reset.reapply_report);

    // the broker never got the announcements of the new run (they are only hints, and nothing here dispatches them);
    // the ones that were written are still waiting to go, and they are lost
    let hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE aggregate_id = $1 AND event_type = 'task.ready'",
    )
    .bind(new.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count");
    assert_eq!(hints, 1, "the step the first signal released was announced");
    sqlx::query("ALTER TABLE event_outbox DISABLE TRIGGER event_outbox_keep_undelivered")
        .execute(&f.w.db.pool)
        .await
        .expect("lift the guard of the outbox");
    sqlx::query("DELETE FROM event_outbox WHERE aggregate_id = $1")
        .bind(new.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("lose the hints");
    sqlx::query("ALTER TABLE event_outbox ENABLE ALWAYS TRIGGER event_outbox_keep_undelivered")
        .execute(&f.w.db.pool)
        .await
        .expect("put the guard back");

    // the journal, the projections and the report are what they were, and a worker polling finds the step
    f.w.assert_consistent(new).await;
    assert_eq!(words(&stored_entries(&f, &made).await), expected);
    finish(&f, new, &["b"]).await;
    assert_eq!(f.w.step_status(new, "b").await, Some(StepStatus::Succeeded));
    assert_eq!(f.w.step_status(new, "w2").await, Some(StepStatus::Waiting));
    f.w.assert_consistent(new).await;
}

/// The report of a reset as the register stores it, read back.
async fn stored_entries(f: &Fixture, made: &ResetMade) -> Vec<ReportEntry> {
    sagat_application::runs::report_from_json(&stored_report(f, made).await)
}

#[tokio::test]
async fn a_reset_of_a_reset_follows_the_retained_arguments_to_the_run_that_kept_them() {
    let f = fixture().await;
    let source = source(&f, true).await;
    let first = reset(&f, &source, ResetReapply::All).await;
    let middle = first
        .run
        .as_ref()
        .expect("the reset made a run")
        .info
        .run_id;
    // the middle run goes on to the end: the caller sends the signals the reapply could not deliver
    finish(&f, middle, &["b"]).await;
    signal(&f, middle, TWO, &payload("two")).await;
    finish(&f, middle, &["c"]).await;
    signal(&f, middle, THREE, &payload("three")).await;
    finish(&f, middle, &["d"]).await;
    assert_eq!(f.w.run_status(middle).await, RunStatus::Succeeded);

    // reset it to its start: the Update of the creation is a row of the middle run that its source wrote, and the
    // arguments are on the receipt of the source; the other Updates were applied by the middle run itself
    let last =
        f.w.runs
            .reset(
                &f.w.admin,
                middle,
                asked(1, ResetReapply::Updates),
                &key(),
                &ctx(),
            )
            .await
            .expect("a reset of a reset");
    assert_eq!(
        words(&last.reset.reapply_report),
        [
            entry("UPDATE", OPERATION, "REAPPLIED", None),
            entry("UPDATE", OPERATION, "REAPPLIED", None),
            entry("UPDATE", OPERATION, "REAPPLIED", None),
        ]
    );
    assert_eq!(
        revision(
            &f,
            last.run.as_ref().expect("the reset made a run").info.run_id
        )
        .await,
        3
    );
    // the first of them is a row the middle run restored from the source
    let creation = last.reset.reapply_report[0].sequence;
    assert_eq!(creation, source.creation_update);
    f.w.assert_consistent(last.run.as_ref().expect("the reset made a run").info.run_id)
        .await;
}

#[tokio::test]
async fn the_ceiling_is_the_most_a_reapply_holds_and_one_more_refuses_the_preview_and_the_reset() {
    let f = fixture().await;
    sagat_infra_ir::set_reset_reapply(true);
    let workflow = {
        let mut definition = document(
            &format!("reapply-ceiling-{}", Uuid::now_v7()),
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(activity("b"), "next", json!("w")),
                with(
                    wait_node("w", ONE, "input.k", 300_000, None),
                    "next",
                    json!("d"),
                ),
                activity("d"),
            ],
        );
        definition["state"] = declared_waiter("ceiling")["state"].clone();
        definition["state"]["updates"][OPERATION]["reapplyOnReset"] = json!(true);
        f.w.publish(definition).await.0
    };
    let business = format!("reapply-ceiling-{}", Uuid::now_v7());
    // the creation Update, `a`, one Update, `b`: the point after `b` has 256 Updates after it, the one after `a` 257
    let run = apply(
        &f,
        workflow,
        &business,
        "update-key-0000",
        &arguments(true, 1),
    )
    .await;
    finish(&f, run, &["a"]).await;
    let early = sequence_of(&f, run, "step.completed", Some("a")).await;
    apply(
        &f,
        workflow,
        &business,
        "update-key-0001",
        &arguments(true, 2),
    )
    .await;
    finish(&f, run, &["b"]).await;
    let late = sequence_of(&f, run, "step.completed", Some("b")).await;
    for n in 0..256_i64 {
        apply(
            &f,
            workflow,
            &business,
            &format!("update-key-{:04}", n + 2),
            &arguments(true, n + 3),
        )
        .await;
    }
    signal(&f, run, ONE, &payload("one")).await;
    finish(&f, run, &["d"]).await;
    assert_eq!(f.w.run_status(run).await, RunStatus::Succeeded);

    // 256 Updates after the late point: at the ceiling, served, and every one of them applied
    let preview =
        f.w.runs
            .reset_preview(&f.w.admin, run, late, ResetReapply::Updates, &ctx())
            .await
            .expect("at the ceiling");
    assert_eq!(preview.reapply.len(), 256);
    let made =
        f.w.runs
            .reset(
                &f.w.admin,
                run,
                asked(late, ResetReapply::Updates),
                &key(),
                &ctx(),
            )
            .await
            .expect("at the ceiling");
    assert_eq!(made.reset.reapply_report.len(), 256);
    assert!(
        made.reset
            .reapply_report
            .iter()
            .all(|entry| entry.outcome == "REAPPLIED")
    );
    assert_eq!(
        revision(
            &f,
            made.run.as_ref().expect("the reset made a run").info.run_id
        )
        .await,
        258
    );
    assert_eq!(
        stored_entries(&f, &made).await,
        made.reset.reapply_report,
        "the register holds all of them"
    );

    // one more is refused, for the preview and for the reset, and nothing is created or dropped silently
    let runs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    for answer in [
        f.w.runs
            .reset_preview(&f.w.admin, run, early, ResetReapply::Updates, &ctx())
            .await
            .map(|_| ()),
        f.w.runs
            .reset(
                &f.w.admin,
                run,
                asked(early, ResetReapply::Updates),
                &key(),
                &ctx(),
            )
            .await
            .map(|_| ()),
    ] {
        assert!(
            matches!(
                answer,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_TOO_MANY",
                    event_type: None
                })
            ),
            "{answer:?}"
        );
    }
    let runs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    assert_eq!(runs_after, runs_before);
    // the same point with the signals included is 257 candidates at the late point
    let all =
        f.w.runs
            .reset(
                &f.w.admin,
                run,
                asked(late, ResetReapply::All),
                &key(),
                &ctx(),
            )
            .await;
    assert!(
        matches!(
            all,
            Err(AppError::ResetRefused {
                reason: "RESET_REAPPLY_TOO_MANY",
                ..
            })
        ),
        "{all:?}"
    );
}

#[tokio::test]
async fn reapply_is_served_only_where_the_deployment_turned_it_on_and_to_the_tenant_that_owns_the_run()
 {
    let f = fixture().await;
    let source = source(&f, true).await;

    // a deployment that did not turn it on refuses the preview and the reset, and creates nothing
    let off = without_reapply(&f);
    let (runs_before, resets_before) = made_counts(&f).await;
    for policy in [
        ResetReapply::Signals,
        ResetReapply::Updates,
        ResetReapply::All,
    ] {
        let refused = off
            .reset_preview(&f.w.admin, source.run, source.point, policy, &ctx())
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_NOT_AVAILABLE",
                    ..
                })
            ),
            "{refused:?}"
        );
        let refused = off
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, policy),
                &key(),
                &ctx(),
            )
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_NOT_AVAILABLE",
                    ..
                })
            ),
            "{refused:?}"
        );
    }
    assert_eq!(
        made_counts(&f).await,
        (runs_before, resets_before),
        "the refusals created no run and no entry"
    );
    // and one that did not ask for a reapply is served as it always was
    off.reset_preview(
        &f.w.admin,
        source.run,
        source.point,
        ResetReapply::None,
        &ctx(),
    )
    .await
    .expect("a preview without a reapply");
    off.reset(
        &f.w.admin,
        source.run,
        asked(source.point, ResetReapply::None),
        &key(),
        &ctx(),
    )
    .await
    .expect("a reset without a reapply");

    // another tenant finds neither the run nor anything of it, and a viewer may not reset
    assert!(matches!(
        f.w.runs
            .reset_preview(
                &f.w.other_admin,
                source.run,
                source.point,
                ResetReapply::All,
                &ctx()
            )
            .await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        f.w.runs
            .reset(
                &f.w.other_admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &key(),
                &ctx()
            )
            .await,
        Err(AppError::NotFound)
    ));
    let stranger = f.w.other_admin.tenant_id();
    let digest: Vec<u8> = sqlx::query_scalar(
        "SELECT payload_digest FROM run_events WHERE run_id = $1 AND event_type = 'wait.satisfied' \
         ORDER BY seq LIMIT 1",
    )
    .bind(source.run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("a payload");
    let ask_digests = [digest.clone()];
    assert!(
        f.w.store
            .payload_contents(f.w.admin.tenant_id(), &ask_digests, true)
            .await
            .expect("read")[0]
            .as_ref()
            .is_some_and(|kept| kept.content.is_some())
    );
    assert_eq!(
        f.w.store
            .payload_contents(stranger, &ask_digests, true)
            .await,
        Ok(vec![None])
    );
    assert_eq!(
        f.w.store
            .retained_update_arguments(stranger, source.run, &[source.creation_update], true)
            .await,
        Ok(vec![None])
    );
    let kept =
        f.w.store
            .retained_update_arguments(
                f.w.admin.tenant_id(),
                source.run,
                &[source.creation_update],
                true,
            )
            .await
            .expect("read");
    assert!(kept[0].as_ref().is_some_and(|kept| kept.content.is_some()));
    // without `read` the length says it is kept and the bytes are not read, for a payload as for arguments
    let measured_payload =
        f.w.store
            .payload_contents(f.w.admin.tenant_id(), &ask_digests, false)
            .await
            .expect("read");
    assert!(
        measured_payload[0]
            .as_ref()
            .is_some_and(|kept| kept.size > 0 && kept.content.is_none()),
        "{measured_payload:?}"
    );
    // without `read` the length says it is kept and the bytes are not read
    let measured =
        f.w.store
            .retained_update_arguments(
                f.w.admin.tenant_id(),
                source.run,
                &[source.creation_update],
                false,
            )
            .await
            .expect("read");
    assert_eq!(
        measured[0]
            .as_ref()
            .map(|kept| (kept.size, kept.content.is_none())),
        kept[0].as_ref().map(|kept| (kept.size, true))
    );
    let viewer = f.w.member("viewer-of-reapply", Role::Viewer).await;
    assert!(matches!(
        f.w.runs
            .reset(
                &viewer,
                source.run,
                asked(source.point, ResetReapply::All),
                &key(),
                &ctx()
            )
            .await,
        Err(AppError::Forbidden { .. })
    ));
}

/// A later version wrote an event this one does not know, after everything the run did.
async fn write_an_event_of_a_newer_kind(f: &Fixture, source: &Source) {
    let last: i64 = sqlx::query_scalar("SELECT max(seq) FROM run_events WHERE run_id = $1")
        .bind(source.run.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the last row");
    sqlx::query(
        "INSERT INTO run_events (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,$3,'future.reapply.observed','{}','system:reapply-test',clock_timestamp())",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(source.run.as_uuid())
    .bind(last + 1)
    .execute(&f.w.db.pool)
    .await
    .expect("an event of a newer kind");
}

#[tokio::test]
async fn rows_this_version_cannot_read_are_not_guessed_at_when_something_is_to_be_delivered_again()
{
    let f = fixture().await;
    let source = source(&f, true).await;
    write_an_event_of_a_newer_kind(&f, &source).await;

    // without a reapply it does not matter; with one, the rows after the point may hold what is to be delivered
    reset(&f, &source, ResetReapply::None).await;
    for policy in [
        ResetReapply::Signals,
        ResetReapply::Updates,
        ResetReapply::All,
    ] {
        let refused =
            f.w.runs
                .reset(
                    &f.w.admin,
                    source.run,
                    asked(source.point, policy),
                    &key(),
                    &ctx(),
                )
                .await;
        assert!(
            matches!(
                &refused,
                Err(AppError::ResetRefused { reason: "RESET_RULE_MISSING", event_type: Some(kind) })
                    if kind == "future.reapply.observed"
            ),
            "{refused:?}"
        );
    }
}
#[tokio::test]
async fn a_reset_of_a_reset_keeps_two_equal_signals_that_the_first_reset_delivered_in_one_transaction()
 {
    // `a`, then two waits in a row for the same signal: the source received it twice, with the same payload
    let f = fixture().await;
    let workflow =
        f.w.publish(document(
            &format!("reapply-twice-{}", Uuid::now_v7()),
            "a",
            vec![
                with(activity("a"), "next", json!("w1")),
                with(
                    wait_node("w1", ONE, "input.k", 300_000, None),
                    "next",
                    json!("w2"),
                ),
                with(
                    wait_node("w2", ONE, "input.k", 300_000, None),
                    "next",
                    json!("c"),
                ),
                activity("c"),
            ],
        ))
        .await
        .0;
    let source = f.w.start(workflow, &json!({"k": "s"})).await;
    finish(&f, source, &["a"]).await;
    let point = sequence_of(&f, source, "step.completed", Some("a")).await;
    let same = payload("twice");
    signal(&f, source, ONE, &same).await;
    signal(&f, source, ONE, &same).await;
    finish(&f, source, &["c"]).await;
    assert_eq!(f.w.run_status(source).await, RunStatus::Succeeded);

    // the first reset delivers both in the transaction that creates its run: two rows of one instant, one payload
    let first =
        f.w.runs
            .reset(
                &f.w.admin,
                source,
                asked(point, ResetReapply::Signals),
                &key(),
                &ctx(),
            )
            .await
            .expect("the first reset");
    assert_eq!(
        words(&first.reset.reapply_report),
        [
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", ONE, "REAPPLIED", None)
        ]
    );
    let middle = first
        .run
        .as_ref()
        .expect("the reset made a run")
        .info
        .run_id;
    finish(&f, middle, &["c"]).await;
    assert_eq!(f.w.run_status(middle).await, RunStatus::Succeeded);
    let instants: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT occurred_at) FROM run_events \
         WHERE run_id = $1 AND event_type = 'wait.satisfied' AND detail->>'restored' IS NULL",
    )
    .bind(middle.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        instants, 1,
        "the two rows of the middle run share one instant"
    );

    // the reset of that reset delivers both again: the second is not taken for the first
    let last =
        f.w.runs
            .reset(
                &f.w.admin,
                middle,
                asked(point, ResetReapply::Signals),
                &key(),
                &ctx(),
            )
            .await
            .expect("the reset of the reset");
    assert_eq!(
        words(&last.reset.reapply_report),
        [
            entry("SIGNAL", ONE, "REAPPLIED", None),
            entry("SIGNAL", ONE, "REAPPLIED", None)
        ]
    );
    let new = last.run.as_ref().expect("the reset made a run").info.run_id;
    assert_eq!(
        f.w.step_status(new, "w1").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(
        f.w.step_status(new, "w2").await,
        Some(StepStatus::Succeeded)
    );
    f.w.assert_consistent(new).await;
}
/// The run service of a deployment whose operator did not enable the reapply of a reset.
fn without_reapply(f: &Fixture) -> crate::support::engine::Runs {
    RunService::new(
        f.w.store.clone(),
        SystemClock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_declared_updates(std::sync::Arc::new(
        sagat_infra_ir::JsonDeclaredOperationEvaluator,
    ))
}

/// How many runs and entries of the register the database holds.
async fn made_counts(f: &Fixture) -> (i64, i64) {
    let runs = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    let resets = sqlx::query_scalar("SELECT count(*) FROM run_resets")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count");
    (runs, resets)
}

#[tokio::test]
async fn a_retry_is_answered_from_the_register_where_the_reapply_is_off_now_and_a_new_request_is_refused()
 {
    let f = fixture().await;
    let source = source(&f, true).await;
    let retry_key = key();
    let first =
        f.w.runs
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &retry_key,
                &ctx(),
            )
            .await
            .expect("the reset, where the reapply is on");

    // the operator turned it off, or the retry reaches a replica where it is: the request that succeeded is answered
    // with what it made, not refused
    let off = without_reapply(&f);
    let counts = made_counts(&f).await;
    let again = off
        .reset(
            &f.w.admin,
            source.run,
            asked(source.point, ResetReapply::All),
            &retry_key,
            &ctx(),
        )
        .await
        .expect("the retry is answered from the register");
    assert_eq!(again.reset, first.reset, "the stored report, as it was");
    assert_eq!(
        again
            .run
            .as_ref()
            .expect("the reset made a run")
            .info
            .run_id,
        first
            .run
            .as_ref()
            .expect("the reset made a run")
            .info
            .run_id
    );
    assert_eq!(made_counts(&f).await, counts);

    // a request that was never made is refused, and makes nothing: no run and no entry
    for policy in [
        ResetReapply::Signals,
        ResetReapply::Updates,
        ResetReapply::All,
    ] {
        let refused = off
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, policy),
                &key(),
                &ctx(),
            )
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_NOT_AVAILABLE",
                    ..
                })
            ),
            "{refused:?}"
        );
    }
    assert_eq!(made_counts(&f).await, counts);

    // the same key for another request is what it always was: a key reused
    let other = off
        .reset(
            &f.w.admin,
            source.run,
            asked(source.point, ResetReapply::None),
            &retry_key,
            &ctx(),
        )
        .await;
    assert!(
        matches!(other, Err(AppError::IdempotencyKeyReused)),
        "{other:?}"
    );
}
#[tokio::test]
async fn a_deployment_that_did_not_turn_the_reapply_on_says_so_before_it_looks_at_the_source() {
    let f = fixture().await;
    let source = source(&f, true).await;
    write_an_event_of_a_newer_kind(&f, &source).await;
    let off = without_reapply(&f);
    // the source holds a row this version cannot read, which is the answer of a deployment that serves the reapply
    // (rows_this_version_cannot_read...): where the setting is off the request is refused for that, nothing is
    // planned, and the source is not read for it
    for policy in [
        ResetReapply::Signals,
        ResetReapply::Updates,
        ResetReapply::All,
    ] {
        let refused = off
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, policy),
                &key(),
                &ctx(),
            )
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_NOT_AVAILABLE",
                    ..
                })
            ),
            "{policy:?}: {refused:?}"
        );
    }
}

/// A run service over a store of its own whose pool counts the connections it hands out: one for each statement, and
/// one for each transaction, that the service runs. The count is the pool's, so it is exact and does not depend on a
/// log; the service reads what a reset needs outside the transaction that creates the run, so the number of reads is
/// what is counted. (It connects as the owner of the database, in both roles of the suites: it counts, it does not
/// test a privilege.)
async fn counting_runs(
    f: &Fixture,
) -> (
    crate::support::engine::Runs,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let acquired = std::sync::Arc::new(AtomicUsize::new(0));
    let seen = acquired.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .before_acquire(move |_connection, _meta| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(true) })
        })
        .connect(&f.w.db.url)
        .await
        .expect("a pool that counts");
    let store = sagat_infra_postgres::PgRunStore::new(pool, f.w.cache.clone());
    let runs = RunService::new(
        store,
        SystemClock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_declared_updates(std::sync::Arc::new(
        sagat_infra_ir::JsonDeclaredOperationEvaluator,
    ))
    .with_reset_reapply(true);
    (runs, acquired)
}

#[tokio::test]
async fn a_chain_of_resets_of_resets_is_walked_once_and_the_walk_stops_at_its_bound() {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let f = fixture().await;
    sagat_infra_ir::set_reset_reapply(true);
    let workflow = {
        let mut definition = document(
            &format!("reapply-chain-{}", Uuid::now_v7()),
            "a",
            vec![
                with(activity("a"), "next", json!("w")),
                with(
                    wait_node("w", ONE, "input.k", 300_000, None),
                    "next",
                    json!("d"),
                ),
                activity("d"),
            ],
        );
        definition["state"] = declared_waiter("chain")["state"].clone();
        definition["state"]["updates"][OPERATION]["reapplyOnReset"] = json!(true);
        f.w.publish(definition).await.0
    };

    // a source that retained 200 Updates before its first step completed: every run that is reset to a point after
    // them restores their rows, and their arguments stay on the receipts of the source
    const UPDATES: i64 = 200;
    let business = format!("reapply-chain-{}", Uuid::now_v7());
    let source = {
        let mut run = None;
        for n in 0..UPDATES {
            run = Some(
                apply(
                    &f,
                    workflow,
                    &business,
                    &format!("update-key-{n:04}"),
                    &arguments(true, n + 1),
                )
                .await,
            );
        }
        run.expect("a run")
    };
    finish(&f, source, &["a"]).await;
    let point = sequence_of(&f, source, "step.completed", Some("a")).await;
    signal(&f, source, ONE, &payload("chain")).await;
    finish(&f, source, &["d"]).await;
    assert_eq!(f.w.run_status(source).await, RunStatus::Succeeded);

    // a chain of 65 resets, each of the one before, each ended (cancelled) so that it can be reset in its turn
    let mut chain = vec![source];
    for _ in 0..65 {
        let made =
            f.w.runs
                .reset(
                    &f.w.admin,
                    *chain.last().expect("a run"),
                    asked(point, ResetReapply::None),
                    &key(),
                    &ctx(),
                )
                .await
                .expect("a link of the chain");
        let run = made.run.as_ref().expect("the reset made a run").info.run_id;
        f.w.operate_run(run, sagat_application::runs::RunOperation::Cancel)
            .await;
        assert_eq!(f.w.run_status(run).await, RunStatus::Cancelled);
        chain.push(run);
    }

    // 64 hops from the source: every Update is found, and the preview and the reset read what they need in a number of
    // statements that does not grow with the 200 candidates (a walk of the chain for each of them was 200 statements
    // more, and one for each hop 12,800 and seconds)
    // measured: 6 for the preview and 16 for the reset (its reads, the transaction, the register, the announcement);
    // the bounds leave room for a statement more, not for one for each candidate
    const PREVIEW_READS: usize = 12;
    const RESET_READS: usize = 32;
    let (runs, acquired) = counting_runs(&f).await;
    let near = chain[64];
    let started = Instant::now();
    let preview = runs
        .reset_preview(&f.w.admin, near, 1, ResetReapply::Updates, &ctx())
        .await
        .expect("the preview");
    let took = started.elapsed();
    let reads = acquired.swap(0, Ordering::SeqCst);
    assert!(
        reads <= PREVIEW_READS,
        "the preview of 200 candidates took {reads} connections: it reads for each of them"
    );
    assert_eq!(preview.reapply.len(), 200);
    assert!(
        preview
            .reapply
            .iter()
            .all(|entry| entry.outcome.as_str() == "WILL_TRY"),
        "every one is found at 64 hops"
    );
    assert!(
        took < Duration::from_secs(2),
        "the preview walked the chain for every candidate: {took:?}"
    );
    let made = runs
        .reset(
            &f.w.admin,
            near,
            asked(1, ResetReapply::Updates),
            &key(),
            &ctx(),
        )
        .await
        .expect("the reset at the bound");
    let reads = acquired.swap(0, Ordering::SeqCst);
    assert!(
        reads <= RESET_READS,
        "the reset of 200 candidates took {reads} connections: it reads for each of them"
    );
    assert!(
        made.reset
            .reapply_report
            .iter()
            .all(|entry| entry.outcome == "REAPPLIED"),
        "{:?}",
        made.reset.reapply_report.first()
    );
    assert_eq!(made.reset.reapply_report.len(), 200);

    // 65 hops: past the bound nothing is walked, and each Update says so instead of being dropped
    let far = chain[65];
    let preview =
        f.w.runs
            .reset_preview(&f.w.admin, far, 1, ResetReapply::Updates, &ctx())
            .await
            .expect("the preview");
    assert_eq!(preview.reapply.len(), 200);
    assert!(
        preview.reapply.iter().all(|entry| {
            entry.outcome.as_str() == "NOT_REAPPLIED"
                && entry.reason.map(|reason| reason.as_str()) == Some("NOT_RETAINED")
        }),
        "{:?}",
        preview.reapply.first()
    );
}
/// The configuration of the control plane as the operator would give it, with `extra` on top.
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
async fn the_service_the_control_plane_builds_refuses_the_reapply_unless_the_operator_set_it() {
    // the default of the test support is that the reapply is on, so that the suites can exercise it; what the
    // production default is, is decided here, by the function the control plane builds its service with
    let f = fixture().await;
    let source = source(&f, true).await;

    let unset =
        sagat_control_plane::run_service(f.w.store.clone(), f.w.cache.clone(), &served(&[]));
    for policy in [
        ResetReapply::Signals,
        ResetReapply::Updates,
        ResetReapply::All,
    ] {
        let refused = unset
            .reset_preview(&f.w.admin, source.run, source.point, policy, &ctx())
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_NOT_AVAILABLE",
                    ..
                })
            ),
            "{refused:?}"
        );
    }
    for off in ["false", "0", "no", ""] {
        let service = sagat_control_plane::run_service(
            f.w.store.clone(),
            f.w.cache.clone(),
            &served(&[("SAGAT_RESET_REAPPLY_ENABLED", off)]),
        );
        let refused = service
            .reset_preview(
                &f.w.admin,
                source.run,
                source.point,
                ResetReapply::All,
                &ctx(),
            )
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::ResetRefused {
                    reason: "RESET_REAPPLY_NOT_AVAILABLE",
                    ..
                })
            ),
            "{off:?}: {refused:?}"
        );
    }

    // set, it is served, and a reset made through it delivers
    for on in ["true", "1", "yes"] {
        let service = sagat_control_plane::run_service(
            f.w.store.clone(),
            f.w.cache.clone(),
            &served(&[("SAGAT_RESET_REAPPLY_ENABLED", on)]),
        );
        let preview = service
            .reset_preview(
                &f.w.admin,
                source.run,
                source.point,
                ResetReapply::All,
                &ctx(),
            )
            .await
            .unwrap_or_else(|error| panic!("{on:?}: {error:?}"));
        assert_eq!(preview.reapply.len(), 5, "{on:?}");
    }
    let service = sagat_control_plane::run_service(
        f.w.store.clone(),
        f.w.cache.clone(),
        &served(&[("SAGAT_RESET_REAPPLY_ENABLED", "true")]),
    );
    let made = service
        .reset(
            &f.w.admin,
            source.run,
            asked(source.point, ResetReapply::All),
            &key(),
            &ctx(),
        )
        .await
        .expect("a reset through the service the configuration built");
    assert_eq!(made.reset.reapply_report.len(), 5);

    // and what it does not touch is served as it always was
    unset
        .reset_preview(
            &f.w.admin,
            source.run,
            source.point,
            ResetReapply::None,
            &ctx(),
        )
        .await
        .expect("a preview with no reapply");
}
/// An Update-with-Start request that finds the open run of `business`, as the answer instead of a panic.
async fn try_apply(
    f: &Fixture,
    workflow: WorkflowId,
    business: &str,
    update_key: &str,
    balance: i64,
) -> Result<sagat_application::runs::UpdateStarted, AppError> {
    use sagat_application::runs::{BusinessStart, StartOptions, StartRun, UpdateAtStart};
    use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy, ReuseWindow};
    let operation = sagat_domain::workflow::ActivityName::parse(OPERATION).expect("an operation");
    let update_key = sagat_domain::execution::UpdateKey::parse(update_key).expect("an update key");
    let args = arguments(true, balance);
    f.w.runs
        .update_with_start(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: br#"{"account":{"balance":10},"k":"s"}"#,
                correlation_id: None,
                deadline: None,
                key: &key(),
            },
            StartOptions {
                business: Some(BusinessStart {
                    id: sagat_domain::business_ids::BusinessId::new(business)
                        .expect("an identifier"),
                    conflict: ConflictPolicy::UseExisting,
                    reuse: ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                }),
                start_delay: None,
            },
            UpdateAtStart {
                operation: &operation,
                update_key: &update_key,
                arguments: &args,
                expected_state_revision: None,
                batch_job: None,
            },
            &ctx(),
        )
        .await
}

#[tokio::test]
async fn an_update_to_a_run_that_is_paused_or_being_cancelled_is_a_clean_refusal_and_changes_nothing()
 {
    use sagat_application::declared_operations::DeclaredOperationStoreError as Refused;
    use sagat_application::runs::RunOperation;
    use sagat_domain::execution::OperationError;
    let f = fixture().await;
    let workflow = publish(&f, true).await;
    let journal = |run: RunId| {
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM run_events WHERE run_id = $1")
                .bind(run.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count")
        }
    };

    // a run that is paused: the Update is refused as the run it is not, and nothing of it is written
    let paused_business = format!("reapply-paused-{}", Uuid::now_v7());
    let paused = apply(
        &f,
        workflow,
        &paused_business,
        "update-key-0001",
        &arguments(true, 1),
    )
    .await;
    f.w.operate_run(paused, RunOperation::Pause).await;
    assert_eq!(f.w.run_status(paused).await, RunStatus::Paused);
    let (events, revision_before) = (journal(paused).await, revision(&f, paused).await);
    let refused = try_apply(&f, workflow, &paused_business, "update-key-0002", 2).await;
    assert!(
        matches!(
            &refused,
            Err(AppError::DeclaredUpdate(Refused::Operation(
                OperationError::RunNotMutable(RunStatus::Paused)
            )))
        ),
        "{:?}",
        refused.map(|started| started.started)
    );
    assert_eq!(journal(paused).await, events);
    assert_eq!(revision(&f, paused).await, revision_before);
    f.w.assert_consistent(paused).await;
    // resumed, it takes the same Update, and the state of the transaction is the journal's
    f.w.operate_run(paused, RunOperation::Resume).await;
    try_apply(&f, workflow, &paused_business, "update-key-0002", 2)
        .await
        .expect("a run that was resumed takes the Update");
    f.w.assert_consistent(paused).await;

    // a run that is being cancelled (its attempt is in flight): the same
    let cancelling_business = format!("reapply-cancelling-{}", Uuid::now_v7());
    let cancelling = apply(
        &f,
        workflow,
        &cancelling_business,
        "update-key-0001",
        &arguments(true, 1),
    )
    .await;
    let session = f.w.session(&["act.a"]).await;
    let _leased = f.w.one_task_of(&session, "act.a").await;
    f.w.operate_run(cancelling, RunOperation::Cancel).await;
    assert_eq!(f.w.run_status(cancelling).await, RunStatus::Cancelling);
    let (events, revision_before) = (journal(cancelling).await, revision(&f, cancelling).await);
    let refused = try_apply(&f, workflow, &cancelling_business, "update-key-0002", 2).await;
    assert!(
        matches!(
            &refused,
            Err(AppError::DeclaredUpdate(Refused::Operation(
                OperationError::RunNotMutable(RunStatus::Cancelling)
            )))
        ),
        "{:?}",
        refused.map(|started| started.started)
    );
    assert_eq!(journal(cancelling).await, events);
    assert_eq!(revision(&f, cancelling).await, revision_before);
    f.w.assert_consistent(cancelling).await;
}

#[tokio::test]
async fn an_update_addressed_to_another_run_than_the_transaction_s_is_refused_before_it_writes() {
    use sagat_application::declared_operations::{
        DeclaredOperationStoreError as Refused, DeclaredUpdate,
    };
    use sagat_application::execution::RunTx as _;
    use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
    use sagat_domain::execution::{OperationAddress, OperationScope, UpdateRequest};
    let f = fixture().await;
    let workflow = publish(&f, true).await;
    let mine = apply(
        &f,
        workflow,
        &format!("reapply-mine-{}", Uuid::now_v7()),
        "update-key-0001",
        &arguments(true, 1),
    )
    .await;
    let other = apply(
        &f,
        workflow,
        &format!("reapply-other-{}", Uuid::now_v7()),
        "update-key-0001",
        &arguments(true, 1),
    )
    .await;
    let (events, revision_before) = (
        events_of(&f, other, "state.updated").await,
        revision(&f, other).await,
    );

    let mut tx =
        f.w.store
            .begin(
                f.w.admin.tenant_id(),
                mine,
                sagat_application::runs::Lock::Wait,
            )
            .await
            .expect("begin")
            .expect("the run");
    let namespace = tx.info().namespace_id;
    let now = tx.now();
    let command = DeclaredUpdate::new(
        UpdateRequest::new(
            OperationScope::new(
                f.w.admin.tenant_id(),
                namespace,
                OperationAddress::Run(other),
            ),
            other,
            sagat_domain::workflow::ActivityName::parse(OPERATION).expect("operation"),
            sagat_domain::execution::UpdateKey::parse("update-key-0002").expect("key"),
            sagat_domain::workflow::Digest::new([0; 32]),
            None,
            f.w.admin.subject().clone(),
        ),
        arguments(true, 99),
        AuditRecord::by(
            &f.w.admin,
            AuditAction::RunUpdated,
            ResourceType::Run,
            other.to_string(),
            None,
            now,
        ),
        Uuid::now_v7(),
        now,
    );
    let answer = tx
        .apply_declared_update(&command, &sagat_infra_ir::JsonDeclaredOperationEvaluator)
        .await;
    assert!(
        matches!(
            &answer,
            Err(Refused::Storage(sagat_application::error::StoreError::Internal { detail }))
                if detail.contains("another run")
        ),
        "an Update for another run is a defect of the caller, not an answer: {:?}",
        answer.map(|applied| applied.result.is_some())
    );
    // the other run is as it was, whether or not this transaction commits
    drop(tx);
    assert_eq!(events_of(&f, other, "state.updated").await, events);
    assert_eq!(revision(&f, other).await, revision_before);
    f.w.assert_consistent(mine).await;
    f.w.assert_consistent(other).await;
}
#[test]
fn what_a_reset_delivers_again_is_counted_once_it_is_committed_with_closed_labels() {
    use crate::counted::{recorded, series};
    const NAME: &str = "workflow_reset_reapply_total";
    let page = recorded(|| async {
        let f = fixture().await;
        let source = source(&f, true).await;
        let retry_key = key();
        f.w.runs
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &retry_key,
                &ctx(),
            )
            .await
            .expect("the reset");
        // a retry is answered from the register and delivers nothing, so it counts nothing
        f.w.runs
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &retry_key,
                &ctx(),
            )
            .await
            .expect("the retry");
        // a reset that is refused creates nothing and counts nothing
        let refused = without_reapply(&f)
            .reset(
                &f.w.admin,
                source.run,
                asked(source.point, ResetReapply::All),
                &key(),
                &ctx(),
            )
            .await;
        assert!(refused.is_err());
        // a reset that asks for nothing counts nothing
        reset(&f, &source, ResetReapply::None).await;
    });
    // Update 2 and Update 3 and the first signal were delivered; the second found nothing waiting; the third was not tried
    for (kind, outcome, reason, times) in [
        ("UPDATE", "REAPPLIED", "none", 2.0),
        ("SIGNAL", "REAPPLIED", "none", 1.0),
        ("SIGNAL", "REJECTED", "NOT_WAITING", 1.0),
        (
            "SIGNAL",
            "NOT_REAPPLIED",
            "PREVIOUS_SIGNAL_NOT_DELIVERED",
            1.0,
        ),
    ] {
        assert_eq!(
            series(
                &page,
                NAME,
                &[("kind", kind), ("outcome", outcome), ("reason", reason)]
            ),
            Some(times),
            "{kind} {outcome} {reason}\n{page}"
        );
    }
    // and nothing else: one series for each of the four, and no label that is not one of the three closed ones
    let lines: Vec<&str> = page.lines().filter(|line| line.starts_with(NAME)).collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    for line in lines {
        let inside = &line[line.find('{').expect("labels") + 1..line.rfind('}').expect("labels")];
        let mut keys: Vec<&str> = inside
            .split(',')
            .filter_map(|pair| pair.split('=').next())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["kind", "outcome", "reason"], "{line}");
    }
}
/// D1993: a reset reads the payload of a signal and the arguments of an Update before the transaction that creates the
/// run, and the delivery writes the content again (`INSERT … ON CONFLICT DO NOTHING`). That is safe only while nothing
/// can delete content in between. Nothing can today: the table is append-only for everyone, the guard cannot be
/// skipped by a session, and no function of the schema deletes from it. If feature 020 (erasure) or any other change
/// lets content go, this test fails, and the read must move inside the transaction (or the digest be checked there)
/// before it is changed.
#[tokio::test]
async fn nothing_can_delete_the_content_a_reset_reads_before_the_transaction_that_delivers_it() {
    let f = fixture().await;
    let guard: String = sqlx::query_scalar(
        "SELECT tgenabled::text FROM pg_trigger WHERE tgname = 'payloads_append_only'",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the guard");
    assert_eq!(guard, "A", "the guard of `payloads` is enabled ALWAYS");
    let deleting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = 'public' AND p.prosrc ~* 'delete\\s+from\\s+payloads'",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the functions");
    assert_eq!(deleting, 0, "a function of the schema deletes content");
    for privilege in ["DELETE", "UPDATE", "TRUNCATE"] {
        let held: bool = sqlx::query_scalar("SELECT has_table_privilege($1, 'payloads', $2)")
            .bind(crate::support::RUNTIME_ROLE)
            .bind(privilege)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the privilege");
        assert!(!held, "the runtime role holds {privilege} on `payloads`");
    }
}
