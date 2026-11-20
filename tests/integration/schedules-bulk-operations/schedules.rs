//! The scenarios of the first story (`T018-019`, US01 scenarios 1 to 5, `FR-018-001..005`,
//! `FR-018-016`) as a reader of the spec would walk through them. Each scenario is one test here
//! or names the tests that prove it, where the proof needs a harness this file does not have.
//!
//! - **Scenario 1** (a spec with calendar, cron and interval entries, a zone, bounds and jitter):
//!   `a_spec_with_every_kind_of_entry_...` below.
//! - **Scenario 2** (two replicas, one with a stale lease): `two_replicas_consider_a_due_time_...`
//!   below, and in the engine `pass::two_replicas_split_the_schedules_by_shard_and_every_time_starts_one_run`.
//! - **Scenario 3** (a crash after admission and before commit): the kill after each of the 28
//!   writes of a start, in `tests/fault/schedules-bulk-operations/firing.rs`
//!   (`a_process_killed_after_any_write_of_a_start_...`).
//! - **Scenario 4** (a spec that never fires or fires too often is refused and nothing is stored):
//!   `a_spec_that_never_fires_...` below.
//! - **Scenario 5** (no active version or a full quota: the time stays owed and is retried under the
//!   same key until the window ends): `catchup::a_time_that_cannot_start_stays_owed_...` and
//!   `catchup::a_workflow_without_a_published_version_...`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::schedule_firing::Fired;
use sagat_application::schedules::ShardFence;
use sagat_application::shards::Lease;
use sagat_domain::sharding::Shard;
use serde_json::json;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::firing::{Fixture, fixture, t0};

fn at(text: &str) -> OffsetDateTime {
    OffsetDateTime::parse(text, &Rfc3339).expect("instant")
}

// --- scenario 1 ---------------------------------------------------------------------------------

#[tokio::test]
async fn a_spec_with_every_kind_of_entry_a_zone_bounds_and_jitter_is_revision_one_and_runs_nothing_before_it_starts()
 {
    let f = fixture().await;
    // 09:00 every day in Lisbon, the first of each month, and a weekly interval; from 2 November to
    // the end of the year; up to ten minutes of jitter
    let spec = json!({
        "calendars": [{"hour": [{"start": 9}]}],
        "cron": ["0 0 1 * *"],
        "intervals": [{"everyMs": 604_800_000}],
        "timezone": "Europe/Lisbon",
        "startAt": "2026-11-02T00:00:00Z",
        "endAt": "2026-12-31T00:00:00Z",
        "jitterMs": 600_000
    });

    let schedule = f.create("every-kind", &spec, &json!({}), None).await;

    assert_eq!((schedule.revision, schedule.state_version), (1, 1));
    assert_eq!(
        schedule.next_action_at,
        Some(at("2026-11-02T09:00:00Z")),
        "the earliest of the three kinds, on or after the start"
    );
    let jitter =
        schedule.next_actual_at.expect("actual") - schedule.next_action_at.expect("nominal");
    assert!(
        jitter >= Duration::ZERO && jitter < Duration::minutes(10),
        "the actual time moves by less than the setting: {jitter}"
    );
    let described = f
        .service
        .describe(&f.w.admin, schedule.id, 10, 5)
        .await
        .expect("describe");
    let nominals: Vec<OffsetDateTime> =
        described.next.iter().map(|(nominal, _)| *nominal).collect();
    assert_eq!(nominals.len(), 10);
    assert!(
        nominals.windows(2).all(|w| w[0] < w[1]),
        "in order, none twice"
    );
    assert!(
        nominals
            .iter()
            .all(|t| *t >= at("2026-11-02T00:00:00Z") && *t < at("2026-12-31T00:00:00Z")),
        "inside the bounds: {nominals:?}"
    );

    // nothing runs before it starts
    f.clock.advance(Duration::days(4)); // 1 November, noon
    assert_eq!(f.fire(&schedule).await, Fired::Idle);
    assert!(f.runs(&schedule).await.is_empty());
    // and when the first time comes, one run starts for it
    f.clock.advance(Duration::hours(21)); // 2 November, 09:00
    f.clock.advance(Duration::minutes(11));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        Some(at("2026-11-03T09:00:00Z"))
    );
}

// --- scenario 2 ---------------------------------------------------------------------------------

#[tokio::test]
async fn two_replicas_consider_a_due_time_and_exactly_one_run_starts_for_it() {
    let f = fixture().await;
    let schedule = f
        .create("two-replicas", &crate::firing::hourly(), &json!({}), None)
        .await;
    let shard = Shard::of(schedule.id.as_uuid());
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-b', generation = 4, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("replica b holds the shard");
    let holder = ShardFence {
        owner: "replica-b".to_owned(),
        lease: Lease {
            shard,
            generation: 4,
        },
    };
    let stale = ShardFence {
        owner: "replica-a".to_owned(),
        lease: Lease {
            shard,
            generation: 3,
        },
    };
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    let due = f.due(&schedule);

    // the replica that lost the shard considers the time first and is fenced where it acts; the
    // holder then acts; a second look by the former finds nothing left to do
    let stale_result = f
        .firing
        .fire(&due, Some(stale.clone()))
        .await
        .expect("stale");
    assert!(
        matches!(stale_result, Fired::Refused(_)),
        "the replica that lost the shard is fenced: {stale_result:?}"
    );
    assert!(
        f.runs(&schedule).await.is_empty(),
        "and nothing was started"
    );
    assert_eq!(
        f.firing.fire(&due, Some(holder)).await.expect("holder"),
        Fired::Acted
    );
    assert_eq!(
        f.firing.fire(&due, Some(stale)).await.expect("stale again"),
        Fired::Idle,
        "the time was acted on"
    );
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 1, "exactly one run");
    let (started_by, schedule_id, nominal, kind, revision): (
        String,
        Uuid,
        OffsetDateTime,
        String,
        i32,
    ) = sqlx::query_as(
        "SELECT started_by, schedule_id, schedule_nominal_at, schedule_action_kind, schedule_revision \
         FROM runs WHERE id = $1",
    )
    .bind(runs[0])
    .fetch_one(&f.w.db.pool)
    .await
    .expect("run");
    assert_eq!(started_by, format!("system:schedule:{}", schedule.id));
    assert_eq!(
        (schedule_id, nominal, kind.as_str(), revision),
        (
            schedule.id.as_uuid(),
            t0() + Duration::hours(1),
            "SCHEDULED",
            1
        ),
        "the provenance of the run"
    );
    let (scope, key): (String, String) =
        sqlx::query_as("SELECT scope, idempotency_key FROM command_dedupe WHERE resource_id = $1")
            .bind(runs[0].to_string())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the key of the time");
    assert_eq!(scope, format!("schedule:{}", schedule.id));
    assert_eq!(
        key,
        format!(
            "nominal-{}",
            (t0() + Duration::hours(1)).unix_timestamp() * 1000
        ),
        "keyed by the schedule and the nominal time"
    );
}

// --- scenario 4 ---------------------------------------------------------------------------------

/// What a refused creation must not have changed: schedules, revisions, stored payloads, audit, charges.
async fn footprint(f: &Fixture) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM schedules), (SELECT count(*) FROM schedule_revisions), \
                (SELECT count(*) FROM payloads), (SELECT count(*) FROM audit_records), \
                (SELECT count(*) FROM quota_transaction_deltas)",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("footprint")
}

#[tokio::test]
async fn a_spec_that_never_fires_or_fires_too_often_is_refused_with_a_reason_and_nothing_is_stored()
{
    let f = fixture().await;
    let before = footprint(&f).await;
    for (spec, reason) in [
        (
            json!({"cron": ["0 9 * * *"], "startAt": "2026-01-01T00:00:00Z", "endAt": "2026-01-01T06:00:00Z"}),
            "SCHEDULE_NEVER_FIRES",
        ),
        (
            json!({"calendars": [{"month": [{"start": 2}], "dayOfMonth": [{"start": 30}]}]}),
            "SCHEDULE_NEVER_FIRES",
        ),
        (
            json!({"intervals": [{"everyMs": 30_000}]}),
            "SCHEDULE_TOO_FREQUENT",
        ),
        (
            json!({"cron": ["* * * * *"], "intervals": [{"everyMs": 45_000}]}),
            "SCHEDULE_TOO_FREQUENT",
        ),
    ] {
        let write = f.write_of("refused", &spec, &json!({}), None);
        let refused = f
            .service
            .create(
                &f.w.admin,
                f.w.namespace.id(),
                write,
                &sagat_application::identity::RequestContext::default(),
            )
            .await
            .expect_err(reason);
        match refused {
            AppError::InvalidSchedule { reason: got, .. } => assert_eq!(got, reason, "{spec}"),
            other => panic!("not a validation refusal for {spec}: {other:?}"),
        }
    }
    assert_eq!(
        footprint(&f).await,
        before,
        "nothing was stored, charged or audited"
    );
}
