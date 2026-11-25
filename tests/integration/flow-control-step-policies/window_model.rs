//! The window decision of PostgreSQL against the domain's pure model (task T022-071; decision D022-082): random
//! sequences of run starts of acquirers with mixed limits and windows on one key, grant passes, ages of the counted
//! grants and a clock set back, applied through the real adapter (`PgRunStore`, the scheduler's grant pass), each
//! decision compared with what `sagat_domain::execution::window` decides from the same instants. A disagreement is a
//! defect of one of the two, never a model to bend.
//!
//! Time never decides here: before each operation every grant row of the key is written at an age the test chose (as the
//! owner, with the guard that forbids it switched off in that transaction, the method of decision D022-072), and the ages
//! lie at least five seconds from every window's edge, so the real time an operation takes cannot move a grant across
//! one. The model reads the instants back from the rows and decides an admission at the instant its row records; a pass
//! counts as of the instant it selected the key, which lies between two readings of the database clock taken around
//! it, and the model must decide the same at both ends. The sequences come from a fixed list of seeds through a small
//! generator written out here, so a failure names its seed and replays.

use sagat_domain::execution::RunId;
use sagat_domain::execution::window::{Admission, Bound, Grant, Waiter, Window};
use sagat_domain::workflow::RatePeriod;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, document};
use crate::support::flow_control::{database_now_us, gated_world, publish};

const SECOND_US: i64 = 1_000_000;

/// The ages a grant is written at, in seconds before the database clock: ten seconds ahead of it (a fresh grant, or a
/// clock set back: counts for every window), half a minute (counts for a minute and an hour), half an hour (for an hour)
/// and two hours (for none). Each lies at least five seconds from the edges of a second, a minute and an hour.
const AGES_S: [i64; 4] = [-10, 30, 1_800, 7_200];

/// The acquirers sharing the group and the key: (limit, window).
const ACQUIRERS: [(u32, &str, RatePeriod); 3] = [
    (1, "second", RatePeriod::Second),
    (2, "minute", RatePeriod::Minute),
    (3, "hour", RatePeriod::Hour),
];

/// One operation of a sequence.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// A start of the acquirer of this index.
    Start(usize),
    /// One grant pass, the key made due first.
    Pass,
    /// Every grant ages by this many of the steps of [`AGES_S`].
    Age(usize),
    /// The database clock set back: every grant counted ahead of it again (decision D022-072).
    SetBack,
}

/// A small deterministic generator (SplitMix64), so that a seed replays its sequence.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> usize {
        usize::try_from(self.next() % n).expect("small")
    }

    fn sequence(&mut self, len: usize) -> Vec<Op> {
        (0..len)
            .map(|_| match self.below(10) {
                0..=4 => Op::Start(self.below(3)),
                5..=6 => Op::Pass,
                7..=8 => Op::Age(1 + self.below(3)),
                _ => Op::SetBack,
            })
            .collect()
    }
}

fn rated_start(name: &str, limit: u32, per: &str) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] = json!({
        "group": "model", "key": "input.accountId", "limit": limit, "per": per, "queueTimeoutMs": 86_400_000
    });
    doc
}

/// A grant of the key as the test keeps it: its run, its acquirer and the index of its age.
struct Granted {
    run: RunId,
    age: usize,
}

/// A queued start as the test keeps it, in queue order.
struct Queued {
    run: RunId,
    acquirer: usize,
}

/// Writes every grant of `granted` at its age before the database clock, as the owner with the guard off in this one
/// transaction, and returns their instants as written, in the order of `granted`.
async fn anchor(w: &World, granted: &[Granted]) -> Vec<i64> {
    if granted.is_empty() {
        return Vec::new();
    }
    let runs: Vec<uuid::Uuid> = granted.iter().map(|g| g.run.as_uuid()).collect();
    let ages: Vec<i64> = granted.iter().map(|g| AGES_S[g.age] * SECOND_US).collect();
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await
        .expect("a bound on lock waits");
    sqlx::query("ALTER TABLE rate_limit_grants DISABLE TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard off in this transaction");
    sqlx::query(
        "UPDATE rate_limit_grants g \
            SET granted_at = statement_timestamp() - a.age * interval '1 microsecond', \
                leased_at = statement_timestamp() - a.age * interval '1 microsecond' \
           FROM unnest($1::uuid[], $2::bigint[]) AS a(run_id, age) \
          WHERE g.run_id = a.run_id AND g.limit_scope = 'RUN'",
    )
    .bind(&runs)
    .bind(&ages)
    .execute(&mut *tx)
    .await
    .expect("the grants written at their ages");
    sqlx::query("ALTER TABLE rate_limit_grants ENABLE ALWAYS TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard on again");
    tx.commit().await.expect("anchored");
    let mut instants = Vec::with_capacity(runs.len());
    for run in &runs {
        instants.push(grant_us(w, *run).await.expect("a grant row"));
    }
    instants
}

/// The instant of `run`'s grant row, in microseconds since the epoch.
async fn grant_us(w: &World, run: uuid::Uuid) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM leased_at) * 1000000)::bigint FROM rate_limit_grants \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run)
    .fetch_optional(&w.db.pool)
    .await
    .expect("grant")
}

/// The enqueuing instant of `run`'s waiter row.
async fn waiter_us(w: &World, run: RunId) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM enqueued_at) * 1000000)::bigint FROM rate_limit_waiters \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_optional(&w.db.pool)
    .await
    .expect("waiter")
}

fn bound(acquirer: usize) -> Bound {
    let (limit, _, per) = ACQUIRERS[acquirer];
    Bound { limit, per }
}

/// The model's window of the key, from the instants as written and the queue in order.
fn model(instants: &[i64], queued: &[Queued]) -> Window {
    Window {
        grants: instants.iter().map(|at| Grant::leased(*at)).collect(),
        waiters: queued
            .iter()
            .map(|q| Waiter {
                bound: bound(q.acquirer),
                deadline_us: i64::MAX,
                live: true,
            })
            .collect(),
    }
}

/// What a sequence decided: (granted at admission, queued, granted by a pass).
#[derive(Debug, Default, Clone, Copy)]
struct Decided {
    granted: u32,
    queued: u32,
    from_queue: u32,
}

async fn run_sequence(
    w: &World,
    workflows: &[sagat_domain::definition::WorkflowId],
    seed: u64,
    decided: &mut Decided,
) {
    let mut rng = Seeded(seed);
    let ops = rng.sequence(14);
    let account = format!("acct-{seed}");
    let input = json!({"accountId": account});
    let mut granted: Vec<Granted> = Vec::new();
    let mut queued: Vec<Queued> = Vec::new();
    for (step, op) in ops.iter().enumerate() {
        let context = format!("seed {seed}, step {step} ({op:?}) of {ops:?}");
        match *op {
            Op::Start(acquirer) => {
                let instants = anchor(w, &granted).await;
                let run = w.start(workflows[acquirer], &input).await;
                let theirs = match (grant_us(w, run.as_uuid()).await, waiter_us(w, run).await) {
                    (Some(at), None) => (Admission::Granted, at),
                    (None, Some(at)) => (Admission::Queued, at),
                    other => panic!("a start neither granted nor queued: {other:?}; {context}"),
                };
                let ours = model(&instants, &queued).admit(bound(acquirer), theirs.1, true);
                assert_eq!(theirs.0, ours, "the admission at {}: {context}", theirs.1);
                match theirs.0 {
                    Admission::Granted => {
                        decided.granted += 1;
                        granted.push(Granted { run, age: 0 });
                    }
                    _ => {
                        decided.queued += 1;
                        queued.push(Queued { run, acquirer });
                    }
                }
            }
            Op::Pass => {
                let instants = anchor(w, &granted).await;
                sqlx::query(
                    "UPDATE rate_limit_windows SET next_eligible_at = now() - interval '1 second' WHERE waiters > 0",
                )
                .execute(&w.db.pool)
                .await
                .expect("due");
                let before = database_now_us(&w.db.pool).await;
                let report = w.scheduler.grant_gates().await;
                let after = database_now_us(&w.db.pool).await;
                assert_eq!(report.failed, 0, "{report:?}; {context}");
                // the pass counted as of its selection, somewhere in [before, after]: the model decides the same at both ends
                let ends: Vec<Vec<usize>> = [before, after]
                    .into_iter()
                    .map(|at| {
                        let mut window = model(&instants, &queued);
                        window
                            .pass(at, at, true, 256)
                            .into_iter()
                            .map(|(position, _)| position)
                            .collect()
                    })
                    .collect();
                assert_eq!(
                    ends[0], ends[1],
                    "the test's ages left a grant at a window's edge during the pass: {context}"
                );
                let mut theirs = Vec::new();
                for (position, q) in queued.iter().enumerate() {
                    if grant_us(w, q.run.as_uuid()).await.is_some() {
                        assert!(waiter_us(w, q.run).await.is_none(), "{context}");
                        theirs.push(position);
                    }
                }
                assert_eq!(theirs, ends[0], "the waiters a pass grants: {context}");
                decided.from_queue += u32::try_from(theirs.len()).expect("small");
                for position in theirs.iter().rev() {
                    let q = queued.remove(*position);
                    granted.push(Granted { run: q.run, age: 0 });
                }
            }
            Op::Age(steps) => {
                for g in &mut granted {
                    g.age = (g.age + steps).min(AGES_S.len() - 1);
                }
            }
            Op::SetBack => {
                for g in &mut granted {
                    g.age = 0;
                }
            }
        }
    }
    // the rows agree with what the test kept: every grant and every waiter of the key, nothing else
    let (grants, waiters): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants WHERE limit_scope = 'RUN' AND run_id = ANY($1)), \
                (SELECT count(*) FROM rate_limit_waiters WHERE limit_scope = 'RUN' AND run_id = ANY($2))",
    )
    .bind(granted.iter().map(|g| g.run.as_uuid()).collect::<Vec<_>>())
    .bind(queued.iter().map(|q| q.run.as_uuid()).collect::<Vec<_>>())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows");
    assert_eq!(
        (grants, waiters),
        (
            i64::try_from(granted.len()).expect("small"),
            i64::try_from(queued.len()).expect("small")
        ),
        "seed {seed}"
    );
}

/// SC-022-017 and FR-022-040 through the real adapter: each admission and each pass of twelve seeded sequences, of three
/// acquirers sharing one key (`limit` 1 a second, 2 a minute, 3 an hour), decides what the domain's model decides from
/// the same instants.
#[tokio::test]
async fn the_sql_window_decides_what_the_domain_model_decides() {
    let w = gated_world().await;
    let workflows: Vec<_> = {
        let mut ids = Vec::new();
        for (n, (limit, per, _)) in ACQUIRERS.iter().enumerate() {
            ids.push(publish(&w, rated_start(&format!("window-model-{n}"), *limit, per)).await);
        }
        ids
    };
    let mut decided = Decided::default();
    for seed in 1..=12 {
        run_sequence(&w, &workflows, seed, &mut decided).await;
    }
    // the sequences reach every kind of decision, so an agreement is not a vacuous one
    assert!(
        decided.granted > 0 && decided.queued > 0 && decided.from_queue > 0,
        "{decided:?}"
    );
    eprintln!("decisions compared: {decided:?}");
}
