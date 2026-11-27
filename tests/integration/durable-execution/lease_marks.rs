//! Feature 006, Task 006-047 (`stream-completion.md`, "The marks"; decisions D006-106 to D006-108): the claim writes the marks of
//! protocol 1.8 in the statement that writes the lease, in each of the three lease statements (`upsert_step` for a root step,
//! `upsert_path_instance` for a scoped instance claimed in its path's transaction, `upsert_scoped_instance` for one claimed in a
//! full-run transaction), conditioned in SQL on a new lease, and returns what it marked; the claim counts it once its transaction
//! commits.
//!
//! This build negotiates at most its own minor, so a session is raised to its minor by SQL and the service under test judges it
//! at the ceiling the `test-support` feature raises, as the tests of the hand-back do. Nothing waits on time: every race is
//! ordered by the test (a transaction held open, a barrier at the commit).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::entropy::{SystemEntropy, Token};
use sagat_application::execution::{
    Actor, Effects, Engine, LeaseGrant, LeaseMarking, LeaseRenewal, MarkOutcome, MarkSkip,
    PathExecutionStore, PathExecutionTx, RunTx,
};
use sagat_application::instances::{Requirement, Requirements, ServingGate};
use sagat_application::runs::{Lock, RunStore};
use sagat_application::worker::{
    Capabilities, ClaimWatch, ClaimedTask, MAX_POLL_BYTES, WorkerService, WorkerSettings,
};
use sagat_domain::execution::{
    AddressedProgress, ExecutionInstanceId, InstancePath, Progress, RunId,
};
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::api::Options;
use crate::support::blob_collections::{Sched, scheduler};
use crate::support::worker::{Platform, platform_with};

type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

const SUBJECT: &str = "wendy";

/// How a requirement is recorded in the gate the engine keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recorded {
    BothKinds,
    EnginesOnly,
    Not,
}

impl Recorded {
    fn kinds(self) -> Option<Vec<String>> {
        match self {
            Self::BothKinds => Some(vec![
                "CONTROL_PLANE".to_owned(),
                "EXECUTION_ENGINE".to_owned(),
            ]),
            Self::EnginesOnly => Some(vec!["EXECUTION_ENGINE".to_owned()]),
            Self::Not => None,
        }
    }
}

/// A gate that keeps `lease-withdrawal-v1` for both kinds, and the two requirements of 1.8 as given.
fn gate(acknowledgement: Recorded, floor: Recorded) -> Arc<ServingGate> {
    let gate = ServingGate::opened();
    let mut required = vec![Requirement {
        capability: "lease-withdrawal-v1".to_owned(),
        kinds: Recorded::BothKinds.kinds().expect("kinds"),
    }];
    for (capability, recorded) in [
        ("lease-withdrawal-v2", floor),
        ("delivery-acknowledgement-v1", acknowledgement),
    ] {
        if let Some(kinds) = recorded.kinds() {
            required.push(Requirement {
                capability: capability.to_owned(),
                kinds,
            });
        }
    }
    gate.keep(&Requirements { epoch: 1, required });
    Arc::new(gate)
}

/// The attempt flag written before the claim (a fixture: only the three lease statements write it in the adapter).
#[derive(Debug, Clone, Copy)]
enum Flag {
    Null,
    /// The attempt the claim takes.
    Claimed,
    /// The attempt before it.
    Previous,
}

/// Which of the three lease statements writes the lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Statement {
    /// `upsert_step`: a root step, through the claim.
    Root,
    /// `upsert_path_instance`: a scoped instance, through the claim of its path.
    Path,
    /// `upsert_scoped_instance`: a scoped instance leased in a full-run transaction, driven at the store.
    FullRun,
}

#[derive(Debug, Clone, Copy)]
struct Case {
    name: &'static str,
    minor: u32,
    declines: bool,
    acknowledgement: Recorded,
    floor: Recorded,
    switch_on: bool,
    flag: Flag,
    ttl_seconds: u64,
    timeout_seconds: Option<u64>,
    expected: MarkOutcome,
}

/// The case that marks: a session at 1.8 that does not decline, both requirements recorded as required, the switch on, no
/// flag, a 30 s time-to-live and no `timeoutMs`.
const MARKS: Case = Case {
    name: "marks",
    minor: 8,
    declines: false,
    acknowledgement: Recorded::EnginesOnly,
    floor: Recorded::BothKinds,
    switch_on: true,
    flag: Flag::Null,
    ttl_seconds: 30,
    timeout_seconds: None,
    expected: MarkOutcome::Provisional,
};

fn skipped(reason: MarkSkip) -> MarkOutcome {
    MarkOutcome::Skipped(reason)
}

/// Each dimension of the contract varied from [`MARKS`], and the reasons that hold together, judged in order.
fn matrix() -> Vec<Case> {
    vec![
        MARKS,
        Case {
            name: "1.7",
            minor: 7,
            expected: MarkOutcome::Unreserved,
            ..MARKS
        },
        Case {
            name: "declining",
            declines: true,
            expected: skipped(MarkSkip::Declined),
            ..MARKS
        },
        Case {
            name: "acknowledgement recorded for both kinds",
            acknowledgement: Recorded::BothKinds,
            ..MARKS
        },
        Case {
            name: "acknowledgement not recorded",
            acknowledgement: Recorded::Not,
            expected: skipped(MarkSkip::NotActivated),
            ..MARKS
        },
        Case {
            name: "floor recorded for engines only",
            floor: Recorded::EnginesOnly,
            expected: skipped(MarkSkip::RequirementMissing),
            ..MARKS
        },
        Case {
            name: "floor not recorded",
            floor: Recorded::Not,
            expected: skipped(MarkSkip::RequirementMissing),
            ..MARKS
        },
        Case {
            name: "switch off",
            switch_on: false,
            expected: skipped(MarkSkip::SwitchOff),
            ..MARKS
        },
        Case {
            name: "flag at the claimed attempt",
            flag: Flag::Claimed,
            expected: skipped(MarkSkip::FlaggedBefore),
            ..MARKS
        },
        Case {
            name: "flag at the attempt before",
            flag: Flag::Previous,
            ..MARKS
        },
        Case {
            name: "time-to-live 17 s",
            ttl_seconds: 17,
            expected: skipped(MarkSkip::ShortDeadline),
            ..MARKS
        },
        Case {
            name: "time-to-live 18 s",
            ttl_seconds: 18,
            ..MARKS
        },
        Case {
            name: "timeoutMs 17 s",
            timeout_seconds: Some(17),
            expected: skipped(MarkSkip::ShortDeadline),
            ..MARKS
        },
        Case {
            name: "timeoutMs 18 s",
            timeout_seconds: Some(18),
            ..MARKS
        },
        // the order of the reasons
        Case {
            name: "not activated before declining",
            acknowledgement: Recorded::Not,
            declines: true,
            expected: skipped(MarkSkip::NotActivated),
            ..MARKS
        },
        Case {
            name: "the switch before declining",
            switch_on: false,
            declines: true,
            expected: skipped(MarkSkip::SwitchOff),
            ..MARKS
        },
        Case {
            name: "declining before the flag",
            declines: true,
            flag: Flag::Claimed,
            expected: skipped(MarkSkip::Declined),
            ..MARKS
        },
        Case {
            name: "the flag before a short deadline",
            flag: Flag::Claimed,
            ttl_seconds: 17,
            expected: skipped(MarkSkip::FlaggedBefore),
            ..MARKS
        },
        Case {
            name: "below 1.8 nothing is judged",
            minor: 7,
            declines: true,
            switch_on: false,
            ttl_seconds: 17,
            expected: MarkOutcome::Unreserved,
            ..MARKS
        },
    ]
}

struct Lab {
    p: Platform,
    principal: Principal,
    scheduler: Sched,
}

async fn lab() -> Lab {
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(50);
    let p = platform_with(options).await;
    // the worker's membership and binding (`*`), as an operator gives them
    drop(p.worker(SUBJECT).await);
    let (status, body) = p
        .rest(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 64})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let scheduler = scheduler(&p, None).await;
    let principal = Principal::new(
        p.tenant,
        Subject::parse(SUBJECT).expect("subject"),
        Role::Worker,
    );
    Lab {
        p,
        principal,
        scheduler,
    }
}

/// One activity `a` of `activity`, three attempts, at the root or in the body of a `forEach` of one item.
fn workflow(name: &str, activity: &str, scoped: bool, timeout_seconds: Option<u64>) -> Value {
    let mut node = json!({
        "id": "a", "type": "activity", "activity": activity,
        "retry": {"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}
    });
    if let Some(seconds) = timeout_seconds {
        node["timeoutMs"] = json!(seconds * 1000);
    }
    if !scoped {
        return json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": name}, "entrypoint": "a", "nodes": [node]
        });
    }
    node["next"] = json!("returned");
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "a", "nodes": [node, {"id": "returned", "type": "return", "result": "1"}]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 1, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// The scoped instance of `a` in the one item of `lines`.
fn item() -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("lines").expect("node"), 0)
            .expect("path"),
        NodeId::parse("a").expect("node"),
    )
}

/// The marks of step `a` of `run`, and its generation and attempt.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    generation: i64,
    attempt: i32,
    hand_back_generation: Option<i64>,
    offered_generation: Option<i64>,
    claimed: bool,
    on_this_server: Option<bool>,
    acknowledged: bool,
    capability_generation: Option<i64>,
}

async fn row(p: &Platform, run: RunId) -> Row {
    use sqlx::Row as _;
    let row = sqlx::query(
        "SELECT generation, attempt, lease_hand_back_generation, lease_offered_generation, \
                lease_claimed_at IS NOT NULL AS claimed, \
                lease_claim_incarnation = pg_postmaster_start_time() AS on_this_server, \
                lease_acknowledged_at IS NOT NULL AS acknowledged, lease_capability_generation \
           FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the step row");
    Row {
        generation: row.get("generation"),
        attempt: row.get("attempt"),
        hand_back_generation: row.get("lease_hand_back_generation"),
        offered_generation: row.get("lease_offered_generation"),
        claimed: row.get("claimed"),
        on_this_server: row.get("on_this_server"),
        acknowledged: row.get("acknowledged"),
        capability_generation: row.get("lease_capability_generation"),
    }
}

impl Lab {
    /// A worker service of this platform's store, with the engine's gate, switch and time-to-live of `case`, judging sessions
    /// at the minor of 1.8.
    fn workers(&self, case: &Case) -> Workers {
        WorkerService::new(
            self.p.api.executions.clone(),
            SystemClock,
            Engine::new(JsonEvaluator),
            SystemEntropy,
            WorkerSettings {
                lease_ttl: Duration::from_secs(case.ttl_seconds),
                ..WorkerSettings::default()
            },
        )
        .with_lease_withdrawal(gate(case.acknowledgement, case.floor))
        .with_new_withdrawals(case.switch_on)
        .with_hand_back_ceiling(8)
    }

    /// A session for `activity` stored at `minor`, declining or not. Registered at a minor this build negotiates, then raised
    /// by SQL as `as_protocol_1_8` does.
    async fn session(
        &self,
        workers: &Workers,
        activity: &str,
        minor: u32,
        declines: bool,
    ) -> (Uuid, String) {
        let offered = if minor >= 6 { "1.6" } else { "1.4" };
        // the cases run one after another and each is done with its session: the earlier ones end, so the identity stays
        // under its limit of live sessions
        sqlx::query(
            "UPDATE worker_sessions SET expires_at = clock_timestamp() - interval '1 second' \
              WHERE tenant_id = $1 AND subject = $2",
        )
        .bind(self.p.tenant.as_uuid())
        .bind(SUBJECT)
        .execute(&self.p.api.db.pool)
        .await
        .expect("end the earlier sessions");
        let registered = workers
            .register(
                &self.principal,
                "payments",
                &format!("marks-{}", Uuid::now_v7()),
                &Capabilities {
                    activities: vec![activity.to_owned()],
                    languages: vec![],
                    pools: vec![],
                    max_inflight: 8,
                    protocol_version: offered.to_owned(),
                    declines_acknowledgement: declines,
                },
            )
            .await
            .expect("register");
        sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = $2 WHERE id = $1")
            .bind(registered.worker_id)
            .bind(i32::try_from(minor).expect("minor"))
            .execute(&self.p.api.db.pool)
            .await
            .expect("raise the minor");
        (
            registered.worker_id,
            registered.session_token.expose().to_owned(),
        )
    }

    /// A run of `activity` (scoped: its one item admitted), its step `a` `READY`.
    async fn run(&self, activity: &str, scoped: bool, timeout_seconds: Option<u64>) -> RunId {
        let name = format!("marks-{}", Uuid::now_v7());
        let id = self
            .p
            .publish(&name, &workflow(&name, activity, scoped, timeout_seconds))
            .await;
        let input = if scoped {
            json!({"lines": [1]})
        } else {
            json!({})
        };
        let run: RunId = self.p.start(&id, &input).await.parse().expect("run id");
        for _ in 0..20 {
            if scoped {
                self.scheduler.tick().await;
            }
            let ready: Option<String> = sqlx::query_scalar(
                "SELECT status FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
            )
            .bind(self.p.tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_optional(&self.p.api.db.pool)
            .await
            .expect("the step");
            if ready.as_deref() == Some("READY") {
                return run;
            }
        }
        panic!("scoped={scoped}: step a never became READY");
    }

    /// Writes the attempt flag of the `READY` step before its claim (the claim takes attempt 1).
    async fn flag(&self, run: RunId, flag: Flag) {
        let value = match flag {
            Flag::Null => return,
            Flag::Claimed => 1,
            Flag::Previous => 0,
        };
        sqlx::query(
            "UPDATE steps SET provisional_withdrawn_attempt = $3 WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
        )
        .bind(self.p.tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(value)
        .execute(&self.p.api.db.pool)
        .await
        .expect("the attempt flag");
    }

    /// The claim of the session's one task (its activity is the case's own, so no other run is a candidate).
    async fn claim(&self, workers: &Workers, session: &(Uuid, String)) -> ClaimedTask {
        let mut tasks = workers
            .poll(&self.principal, session.0, &session.1, 1, None)
            .await
            .expect("poll");
        assert_eq!(tasks.len(), 1, "one task");
        tasks.remove(0)
    }

    /// Leases the scoped item of `run` in a full-run transaction (`upsert_scoped_instance`), with `marking`, as the scoped
    /// branch of the claim of a full-run transaction does.
    async fn lease_in_a_full_run_transaction(
        &self,
        run: RunId,
        session: Uuid,
        ttl: Duration,
        timeout: Option<Duration>,
        marking: LeaseMarking,
    ) -> Option<sagat_application::execution::ClaimMark> {
        let store = &self.p.api.executions;
        let mut tx = store
            .begin(self.p.tenant, run, Lock::Wait)
            .await
            .expect("begin")
            .expect("the run");
        let instance = item();
        let mut effects = Effects::by(Actor::Worker(session.to_string()), None);
        effects.lease = Some(LeaseGrant {
            instance: instance.clone(),
            token_hash: Token::from_bytes([9; 32]).expect("token").hash(),
            worker_id: session.to_string(),
            ttl,
            attempt_timeout: timeout,
            marking,
        });
        let leased = instance.clone();
        Engine::new(JsonEvaluator)
            .run_addressed(&mut tx, Vec::new(), effects, move |_, state, _| {
                let (_, event) = sagat_domain::execution::lease_instance(state, &leased)?;
                Ok((
                    (),
                    AddressedProgress {
                        events: vec![event],
                    },
                ))
            })
            .await
            .expect("lease in a full-run transaction");
        let mark = tx.claim_mark(&instance);
        tx.commit().await.expect("commit");
        mark
    }
}

/// What the engine would ask the statement to mark for `case` (the root and path rows check the service computes it).
fn marking_of(case: &Case) -> LeaseMarking {
    if case.minor < 8 {
        return LeaseMarking::Unreserved;
    }
    let acknowledgement = case.acknowledgement != Recorded::Not;
    let floor = case.floor == Recorded::BothKinds;
    LeaseMarking::Reserved {
        skipped: MarkSkip::of_engine(acknowledgement, floor, case.switch_on, case.declines),
    }
}

/// `worker_lease_marks_total` and each `worker_lease_marks_skipped_total{reason}`, zero before their first increment.
fn counters(metrics: &sagat_telemetry::Metrics) -> (u64, Vec<(MarkSkip, u64)>) {
    let page = metrics.render();
    let value = |prefix: &str| {
        page.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map_or(0, |value| value as u64)
    };
    (
        value("worker_lease_marks_total "),
        MarkSkip::ALL
            .into_iter()
            .map(|reason| {
                (
                    reason,
                    value(&format!(
                        "worker_lease_marks_skipped_total{{reason=\"{}\"}} ",
                        reason.as_str()
                    )),
                )
            })
            .collect(),
    )
}

/// What `outcome` adds to the counters.
fn counted(
    before: &(u64, Vec<(MarkSkip, u64)>),
    outcome: MarkOutcome,
) -> (u64, Vec<(MarkSkip, u64)>) {
    let mut after = before.clone();
    match outcome {
        MarkOutcome::Provisional => after.0 += 1,
        MarkOutcome::Skipped(reason) => {
            for (each, n) in &mut after.1 {
                if *each == reason {
                    *n += 1;
                }
            }
        }
        MarkOutcome::Unreserved => {}
    }
    after
}

/// What the row must hold after the claim of `expected`.
fn assert_marks(row: &Row, expected: MarkOutcome, case: &str) {
    assert_eq!(row.attempt, 1, "{case}: the claimed attempt");
    if expected == MarkOutcome::Unreserved {
        assert_eq!(
            (
                row.hand_back_generation,
                row.offered_generation,
                row.claimed,
                row.on_this_server
            ),
            (None, None, false, None),
            "{case}: a session below 1.8 is not marked"
        );
    } else {
        assert_eq!(
            row.hand_back_generation,
            Some(row.generation),
            "{case}: the reservation"
        );
        assert!(row.claimed, "{case}: the claim's time");
        assert_eq!(
            row.on_this_server,
            Some(true),
            "{case}: the claim's incarnation"
        );
        let offered = (expected == MarkOutcome::Provisional).then_some(row.generation);
        assert_eq!(
            row.offered_generation, offered,
            "{case}: the provisional mark"
        );
    }
    assert!(!row.acknowledged, "{case}: a new lease is not acknowledged");
    assert_eq!(
        row.capability_generation, None,
        "{case}: no capability served"
    );
}

/// The matrix of the claim over the two statements a claim writes a lease through: each case's minor, word, requirements,
/// switch, attempt flag, time-to-live and `timeoutMs`, root and scoped, with what the claim returned, what the row holds and
/// what was counted after the commit. A session of 1.4 claims only at the root (a scoped instance needs 1.5).
#[tokio::test]
async fn the_claim_writes_the_marks_of_each_case_and_counts_them_after_its_commit() {
    let (metrics, recorder) = sagat_telemetry::Metrics::detached(Duration::from_secs(60));
    let _recording = metrics::set_default_local_recorder(&recorder);
    let lab = lab().await;
    let mut cases: Vec<(Case, Statement)> = Vec::new();
    for case in matrix() {
        cases.push((case, Statement::Root));
        cases.push((case, Statement::Path));
    }
    let root_only = Case {
        name: "1.4",
        minor: 4,
        expected: MarkOutcome::Unreserved,
        ..MARKS
    };
    cases.push((root_only, Statement::Root));
    for (index, (case, statement)) in cases.into_iter().enumerate() {
        let label = format!("{} ({statement:?})", case.name);
        let activity = format!("mark.case{index}");
        let workers = lab.workers(&case);
        let session = lab
            .session(&workers, &activity, case.minor, case.declines)
            .await;
        let run = lab
            .run(
                &activity,
                statement == Statement::Path,
                case.timeout_seconds,
            )
            .await;
        lab.flag(run, case.flag).await;
        let before = counters(&metrics);
        let task = lab.claim(&workers, &session).await;
        assert_eq!(
            task.instance.path().is_root(),
            statement == Statement::Root,
            "{label}"
        );
        assert_eq!(
            task.mark,
            Some(case.expected),
            "{label}: what the claim returned"
        );
        assert_eq!(
            task.acknowledge_before_start(),
            case.expected == MarkOutcome::Provisional,
            "{label}: the flag on the lease"
        );
        assert_marks(&row(&lab.p, run).await, case.expected, &label);
        assert_eq!(
            counters(&metrics),
            counted(&before, case.expected),
            "{label}: counted once, after the commit"
        );
    }
}

/// The third lease statement, `upsert_scoped_instance`: a scoped instance leased in a full-run transaction writes the same
/// marks, from the marking the claim asks for and the statement's own reasons.
#[tokio::test]
async fn a_scoped_instance_leased_in_a_full_run_transaction_is_marked_by_the_same_rules() {
    let lab = lab().await;
    let cases = [
        MARKS,
        Case {
            name: "1.7",
            minor: 7,
            expected: MarkOutcome::Unreserved,
            ..MARKS
        },
        Case {
            name: "declining",
            declines: true,
            expected: skipped(MarkSkip::Declined),
            ..MARKS
        },
        Case {
            name: "switch off",
            switch_on: false,
            expected: skipped(MarkSkip::SwitchOff),
            ..MARKS
        },
        Case {
            name: "flag at the claimed attempt",
            flag: Flag::Claimed,
            expected: skipped(MarkSkip::FlaggedBefore),
            ..MARKS
        },
        Case {
            name: "flag at the attempt before",
            flag: Flag::Previous,
            ..MARKS
        },
        Case {
            name: "time-to-live 17 s",
            ttl_seconds: 17,
            expected: skipped(MarkSkip::ShortDeadline),
            ..MARKS
        },
        Case {
            name: "time-to-live 18 s",
            ttl_seconds: 18,
            ..MARKS
        },
        Case {
            name: "timeoutMs 17 s",
            timeout_seconds: Some(17),
            expected: skipped(MarkSkip::ShortDeadline),
            ..MARKS
        },
        Case {
            name: "timeoutMs 18 s",
            timeout_seconds: Some(18),
            ..MARKS
        },
    ];
    for (index, case) in cases.into_iter().enumerate() {
        let activity = format!("mark.full{index}");
        let run = lab.run(&activity, true, case.timeout_seconds).await;
        lab.flag(run, case.flag).await;
        let mark = lab
            .lease_in_a_full_run_transaction(
                run,
                Uuid::now_v7(),
                Duration::from_secs(case.ttl_seconds),
                case.timeout_seconds.map(Duration::from_secs),
                marking_of(&case),
            )
            .await
            .expect("the statement returned its marks");
        assert_eq!(mark.outcome, case.expected, "{}", case.name);
        assert_marks(&row(&lab.p, run).await, case.expected, case.name);
    }
}

/// A floor binary, which does not know the columns, writes the next lease with the old column list: the marks of the earlier
/// lease stay on the row and no longer count, because the new lease has the next generation.
#[tokio::test]
async fn a_lease_written_afterwards_with_the_old_column_list_is_not_marked() {
    let lab = lab().await;
    for (index, scoped) in [false, true].into_iter().enumerate() {
        let activity = format!("mark.old{index}");
        let workers = lab.workers(&MARKS);
        let session = lab.session(&workers, &activity, 8, false).await;
        let run = lab.run(&activity, scoped, None).await;
        let task = lab.claim(&workers, &session).await;
        assert_eq!(task.mark, Some(MarkOutcome::Provisional));
        assert!(
            counts(&lab.p, run).await,
            "scoped={scoped}: the claim's mark counts"
        );
        // the statement of the floor build: status, attempt, generation and the lease, and nothing of 1.8
        sqlx::query(
            "UPDATE steps SET status = 'LEASED', generation = generation + 1, \
                    lease_token_hash = $3, lease_expires_at = now() + interval '30 seconds', leased_by = 'floor' \
              WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
        )
        .bind(lab.p.tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(Token::from_bytes([3; 32]).expect("token").hash())
        .execute(&lab.p.api.db.pool)
        .await
        .expect("the floor's lease");
        let after = row(&lab.p, run).await;
        assert_eq!(
            after.hand_back_generation,
            Some(after.generation - 1),
            "scoped={scoped}: the old mark is left"
        );
        assert!(
            !counts(&lab.p, run).await,
            "scoped={scoped}: and does not count"
        );
    }
}

/// Whether the reservation of step `a` of `run` counts, by the adapter's macro without the recording.
async fn counts(p: &Platform, run: RunId) -> bool {
    // the SQL is the adapter's constant, nothing a caller gives
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM steps s WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = 'a'",
        sagat_infra_postgres::lease_marks::COUNTS
    )))
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the condition")
}

/// The two forms of the adapter's macro against the application's pure function, over every combination of a row: the status
/// (`LEASED`, `RUNNING`, `READY`), the reservation's generation (the step's, the one before, none), the incarnation (this
/// server's, another, none) and the claim's time (before, at and after the recording, none). The incarnation is the adapter's
/// to compare; the function receives it as a boolean.
#[tokio::test]
async fn the_macro_of_the_marks_and_the_pure_function_agree_on_every_row() {
    let lab = lab().await;
    let workers = lab.workers(&MARKS);
    let session = lab.session(&workers, "mark.parity", 8, false).await;
    let run = lab.run("mark.parity", false, None).await;
    lab.claim(&workers, &session).await;
    // the lease's token, put back whenever the combination holds the lease (a held step has one, a `READY` step none)
    let token: Vec<u8> = sqlx::query_scalar(
        "SELECT lease_token_hash FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(lab.p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&lab.p.api.db.pool)
    .await
    .expect("the lease's token");
    let since: time::OffsetDateTime = sqlx::query_scalar("SELECT date_trunc('second', now())")
        .fetch_one(&lab.p.api.db.pool)
        .await
        .expect("a recording time");
    let select = format!(
        "SELECT {}, {} AS counts, {} AS counts_since FROM steps s \
          WHERE s.tenant_id = $2 AND s.run_id = $3 AND s.node = 'a'",
        sagat_infra_postgres::lease_marks::COLUMNS,
        sagat_infra_postgres::lease_marks::COUNTS,
        sagat_infra_postgres::lease_marks::COUNTS_SINCE_RECORDING
    );
    let mut counted = 0;
    for status in ["LEASED", "RUNNING", "READY"] {
        for reservation in ["generation", "generation - 1", "NULL"] {
            for incarnation in [
                "pg_postmaster_start_time()",
                "pg_postmaster_start_time() - interval '1 second'",
                "NULL",
            ] {
                for claimed in [
                    "$3::timestamptz - interval '1 second'",
                    "$3::timestamptz",
                    "$3::timestamptz + interval '1 second'",
                    "NULL",
                ] {
                    // the combination is the test's own literals
                    sqlx::query(sqlx::AssertSqlSafe(format!(
                        "UPDATE steps SET status = '{status}', lease_hand_back_generation = {reservation}, \
                                lease_claim_incarnation = {incarnation}, lease_claimed_at = {claimed}, \
                                lease_token_hash = CASE WHEN '{status}' IN ('LEASED', 'RUNNING') THEN $4::bytea END \
                          WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'"
                    )))
                    .bind(lab.p.tenant.as_uuid())
                    .bind(run.as_uuid())
                    .bind(since)
                    .bind(&token)
                    .execute(&lab.p.api.db.pool)
                    .await
                    .expect("write the combination");
                    let row = sqlx::query(sqlx::AssertSqlSafe(select.clone()))
                        .bind(since)
                        .bind(lab.p.tenant.as_uuid())
                        .bind(run.as_uuid())
                        .fetch_one(&lab.p.api.db.pool)
                        .await
                        .expect("read the combination");
                    let marks = sagat_infra_postgres::lease_marks::decode(&row).expect("decode");
                    let combination = format!("{status}, {reservation}, {incarnation}, {claimed}");
                    let sql_counts: bool = sqlx::Row::get(&row, "counts");
                    let sql_since: bool = sqlx::Row::get(&row, "counts_since");
                    assert_eq!(marks.counts(), sql_counts, "{combination}");
                    assert_eq!(marks.counts_since(since), sql_since, "{combination}");
                    counted += usize::from(sql_since);
                }
            }
        }
    }
    // the case that counts with the recording: held, the step's generation, this server, claimed after
    assert_eq!(
        counted, 2,
        "LEASED and RUNNING, claimed after the recording"
    );
}

/// A claim's consumer that goes away just before the commit of the run's transaction that leased (decision D006-092): the
/// transaction is rolled back, so no mark is written and neither counter moves, at the root and in a scope.
#[tokio::test]
async fn a_claim_that_would_mark_whose_consumer_goes_before_its_commit_marks_and_counts_nothing() {
    struct GoesAtTheCommit(AtomicBool);
    impl ClaimWatch for GoesAtTheCommit {
        fn consumer_gone(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn before_commit(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            self.0.store(true, Ordering::SeqCst);
            Box::pin(std::future::ready(()))
        }
    }
    let (metrics, recorder) = sagat_telemetry::Metrics::detached(Duration::from_secs(60));
    let _recording = metrics::set_default_local_recorder(&recorder);
    let lab = lab().await;
    for (index, scoped) in [false, true].into_iter().enumerate() {
        let activity = format!("mark.gone{index}");
        let workers = lab.workers(&MARKS);
        let session = lab.session(&workers, &activity, 8, false).await;
        let run = lab.run(&activity, scoped, None).await;
        let before = counters(&metrics);
        let watch = GoesAtTheCommit(AtomicBool::new(false));
        let claimed = workers
            .poll_bounded(
                &lab.principal,
                session.0,
                &session.1,
                1,
                MAX_POLL_BYTES,
                None,
                &watch,
            )
            .await
            .expect("poll");
        assert!(claimed.is_empty(), "scoped={scoped}: rolled back");
        assert!(
            watch.0.load(Ordering::SeqCst),
            "scoped={scoped}: the barrier ran"
        );
        let after = row(&lab.p, run).await;
        assert_eq!(
            (
                after.attempt,
                after.hand_back_generation,
                after.offered_generation,
                after.claimed
            ),
            (0, None, None, false),
            "scoped={scoped}: no lease and no mark"
        );
        assert_eq!(
            counters(&metrics),
            before,
            "scoped={scoped}: nothing counted"
        );
        // the same claim with its consumer present marks and counts
        let task = lab.claim(&workers, &session).await;
        assert_eq!(task.mark, Some(MarkOutcome::Provisional), "scoped={scoped}");
        assert_eq!(
            counters(&metrics),
            counted(&before, MarkOutcome::Provisional),
            "scoped={scoped}"
        );
    }
}

/// The barrier of the marks (`stream-completion.md`, "The marks"): a run transaction that loaded the run before an
/// acknowledgement committed, and that then writes the row through each statement that can touch a `LEASED` row (a renewal
/// here), keeps the acknowledgement and the claim's marks: no statement assigns a mark from a value it read earlier. The
/// acknowledgement is written by SQL as its statement will (Task 006-049 builds it).
#[tokio::test]
async fn a_run_transaction_loaded_before_an_acknowledgement_keeps_it_through_each_lease_statement()
{
    let lab = lab().await;
    for (index, statement) in [Statement::Root, Statement::FullRun, Statement::Path]
        .into_iter()
        .enumerate()
    {
        let activity = format!("mark.barrier{index}");
        let workers = lab.workers(&MARKS);
        let session = lab.session(&workers, &activity, 8, false).await;
        let run = lab.run(&activity, statement != Statement::Root, None).await;
        let task = lab.claim(&workers, &session).await;
        assert_eq!(task.mark, Some(MarkOutcome::Provisional), "{statement:?}");
        let claimed = row(&lab.p, run).await;
        let renewal = |instance: ExecutionInstanceId| {
            let mut effects = Effects::by(Actor::Worker(session.0.to_string()), None);
            effects.renewal = Some(LeaseRenewal {
                instance,
                ttl: Duration::from_secs(30),
            });
            effects
        };
        // the run's transaction loads the run (and holds its lock) before the acknowledgement commits
        let store = &lab.p.api.executions;
        match statement {
            Statement::Root | Statement::FullRun => {
                let mut tx = store
                    .begin(lab.p.tenant, run, Lock::Wait)
                    .await
                    .expect("begin")
                    .expect("the run");
                acknowledge(&lab.p, run).await;
                if statement == Statement::Root {
                    Engine::new(JsonEvaluator)
                        .run(&mut tx, renewal(task.instance.clone()), |_, _, _| {
                            Ok((
                                (),
                                Progress {
                                    events: Vec::new(),
                                    pending: None,
                                },
                            ))
                        })
                        .await
                        .expect("a renewal through upsert_step");
                } else {
                    Engine::new(JsonEvaluator)
                        .run_addressed(
                            &mut tx,
                            Vec::new(),
                            renewal(task.instance.clone()),
                            |_, _, _| Ok(((), AddressedProgress { events: Vec::new() })),
                        )
                        .await
                        .expect("a renewal through upsert_scoped_instance");
                }
                tx.commit().await.expect("commit");
            }
            Statement::Path => {
                let mut tx = store
                    .begin_current_path(lab.p.tenant, run, item().path().clone(), Lock::Wait)
                    .await
                    .expect("begin the path")
                    .expect("the path");
                acknowledge(&lab.p, run).await;
                tx.persist_addressed(&[], &[], renewal(task.instance.clone()))
                    .await
                    .expect("a renewal through upsert_path_instance");
                tx.commit().await.expect("commit");
            }
        }
        let after = row(&lab.p, run).await;
        assert!(
            after.acknowledged,
            "{statement:?}: the acknowledgement is kept"
        );
        assert_eq!(
            Row {
                acknowledged: false,
                ..after
            },
            claimed,
            "{statement:?}: the claim's marks are kept"
        );
    }
}

/// What the acknowledgement of Task 006-049 writes, by SQL on its own connection: `lease_acknowledged_at` of a provisional
/// lease, committed.
async fn acknowledge(p: &Platform, run: RunId) {
    let done = sqlx::query(
        "UPDATE steps SET lease_acknowledged_at = clock_timestamp() \
          WHERE tenant_id = $1 AND run_id = $2 AND node = 'a' AND status = 'LEASED' \
            AND lease_offered_generation = generation",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("acknowledge");
    assert_eq!(done.rows_affected(), 1, "a provisional lease acknowledged");
}

/// The frame of a stream and the answer of `PollTasks` carry a flagged lease's `acknowledge_before_start`, and its
/// `attempt_deadline_epoch_ms` when the attempt deadline comes before the expiry: 20 s after the claim with a 20 s `timeoutMs`
/// and a 60 s time-to-live, none with a 60 s `timeoutMs`. The stored expiry is not shortened.
#[tokio::test]
async fn a_flagged_lease_carries_its_attempt_deadline_only_when_it_comes_first() {
    let mut options = Options {
        hand_back_ceiling: 8,
        ..Options::default()
    };
    options.worker.lease_ttl = Duration::from_secs(60);
    options.worker.idle_retry = Duration::from_millis(50);
    options.lease_withdrawal = Some(gate(Recorded::EnginesOnly, Recorded::BothKinds));
    let p = platform_with(options).await;
    for (index, (timeout_seconds, streamed)) in [(20, false), (60, false), (20, true), (60, true)]
        .into_iter()
        .enumerate()
    {
        let activity = format!("mark.frame{index}");
        let name = format!("marks-{}", Uuid::now_v7());
        let workflow = p
            .publish(
                &name,
                &workflow(&name, &activity, false, Some(timeout_seconds)),
            )
            .await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = p.worker(&format!("frame{index}")).await;
        worker.register_version(&[activity.as_str()], "1.6").await;
        sqlx::query("UPDATE worker_sessions SET accepted_protocol_minor = 8 WHERE id = $1::uuid")
            .bind(&worker.worker_id)
            .execute(&p.api.db.pool)
            .await
            .expect("raise the minor");
        let lease = if streamed {
            let mut stream = worker
                .client
                .stream_tasks(worker.req(worker.poll_message(1)))
                .await
                .expect("open the stream")
                .into_inner();
            let frame = tokio::time::timeout(Duration::from_secs(15), stream.message())
                .await
                .expect("a frame in time")
                .expect("a frame")
                .expect("not the end");
            frame.tasks.into_iter().next().expect("a lease")
        } else {
            worker.one_task().await
        };
        let case = format!("timeoutMs {timeout_seconds} s, streamed={streamed}");
        assert!(lease.acknowledge_before_start, "{case}: flagged");
        let (claimed_at, expires_at, attempt_deadline_at): (
            time::OffsetDateTime,
            time::OffsetDateTime,
            Option<time::OffsetDateTime>,
        ) = sqlx::query_as(
            "SELECT updated_at, lease_expires_at, attempt_deadline_at FROM steps \
              WHERE tenant_id = $1 AND run_id = $2::uuid AND node = 'a'",
        )
        .bind(p.tenant.as_uuid())
        .bind(&run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the lease's row");
        let epoch_ms = |at: time::OffsetDateTime| {
            i64::try_from(at.unix_timestamp_nanos() / 1_000_000).expect("milliseconds")
        };
        assert_eq!(
            expires_at - claimed_at,
            time::Duration::seconds(60),
            "{case}: the expiry is not shortened"
        );
        assert_eq!(lease.expires_at_epoch_ms, epoch_ms(expires_at), "{case}");
        if timeout_seconds == 20 {
            let deadline = attempt_deadline_at.expect("an attempt deadline");
            assert_eq!(
                deadline - claimed_at,
                time::Duration::seconds(20),
                "{case}: 20 s after the claim"
            );
            assert_eq!(
                lease.attempt_deadline_epoch_ms,
                epoch_ms(deadline),
                "{case}: carried"
            );
        } else {
            assert_eq!(lease.attempt_deadline_epoch_ms, 0, "{case}: none");
        }
    }
}
