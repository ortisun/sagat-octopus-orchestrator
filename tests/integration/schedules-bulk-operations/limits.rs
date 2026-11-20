//! The finite ceilings of a schedule (`T018-022`, `FR-018-003`, `FR-018-045`, `NFR-018-003`): one
//! test per row of the plan's table that a schedule of the first story meets. Every refusal comes
//! before work (nothing is stored, charged or audited), and a ceiling lowered after the fact refuses
//! new admissions while what exists keeps firing.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::error::AppError;
use sagat_application::schedule_firing::{Fired, ScheduleFiring};
use sagat_application::schedules::{ScheduleDocument, ScheduleWrite, decode_document};
use sagat_domain::schedules::Ceilings;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgIdentityStore, PgScheduleStore};
use sagat_scheduler::ScheduleSettings;
use serde_json::{Value, json};
use time::Duration;

use crate::firing::{Fixture, Starts, fixture, fixture_from, hourly};
use crate::pass::replica_with;
use crate::support::schedules::{outcomes_of, some_hours_ago};

/// What a refused creation must not have changed: schedules, their revisions, stored payloads, audit.
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

fn document(f: &Fixture, name: &str, spec: Value) -> ScheduleDocument {
    ScheduleDocument {
        name: name.to_owned(),
        spec,
        policies: None,
        action: json!({"workflowId": f.workflow.to_string(), "input": {"region": "eu"}}),
        paused: false,
        note: None,
        remaining_actions: None,
    }
}

/// The way a request reaches the service: decoded under the ceilings of the namespace.
fn decode(
    f: &Fixture,
    name: &str,
    spec: Value,
    ceilings: &Ceilings,
) -> Result<ScheduleWrite, AppError> {
    decode_document(document(f, name, spec), ceilings)
}

fn refusal(error: &AppError) -> (&'static str, String) {
    match error {
        AppError::InvalidSchedule { reason, field } => (reason, field.clone().unwrap_or_default()),
        other => panic!("not a schedule validation error: {other:?}"),
    }
}

/// `n` intervals whose times are two minutes apart in turn, so no two are closer than the spacing.
fn intervals(n: u64) -> Value {
    let items: Vec<Value> = (0..n)
        .map(|i| json!({"everyMs": n * 120_000, "offsetMs": i * 120_000}))
        .collect();
    json!({ "intervals": items })
}

// --- Schedules per namespace (active + paused): 200 / 10,000 ------------------------------------

#[tokio::test]
async fn a_namespace_limited_to_zero_refuses_new_schedules_and_the_existing_ones_still_fire() {
    let f = fixture().await;
    let existing = f.create("old", &hourly(), &json!({}), None).await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'schedules', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("limit");
    let before = footprint(&f).await;

    let refused = f
        .try_create(f.write_of("new", &hourly(), &json!({}), None))
        .await
        .expect_err("zero admits nothing");
    assert!(matches!(refused, AppError::QuotaExceeded(_)), "{refused:?}");
    assert_eq!(
        footprint(&f).await,
        before,
        "the refusal came before any work: nothing was stored, charged or audited"
    );

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&existing).await, Fired::Acted, "what exists drains");
}

// --- Spec entries / exclusions per schedule: 10 / 10, hard 32 / 32 ------------------------------

#[tokio::test]
async fn ten_spec_entries_are_accepted_and_the_eleventh_is_refused_with_nothing_stored() {
    let f = fixture().await;
    let ceilings = Ceilings::default();
    let write = decode(&f, "ten", intervals(10), &ceilings).expect("ten entries");
    f.try_create(write).await.expect("created");
    let before = footprint(&f).await;

    let error = decode(&f, "eleven", intervals(11), &ceilings).expect_err("eleven entries");
    let (reason, field) = refusal(&error);
    assert_eq!(reason, "SCHEDULE_SPEC_INVALID");
    assert!(
        field.starts_with("/spec"),
        "the refusal points into the spec: {field}"
    );
    assert_eq!(footprint(&f).await, before);
}

#[tokio::test]
async fn ten_exclusions_are_accepted_and_the_eleventh_is_refused_with_nothing_stored() {
    let f = fixture().await;
    let ceilings = Ceilings::default();
    let with = |n: usize| {
        json!({
            "intervals": [{"everyMs": 3_600_000}],
            "exclusions": vec![json!({"hour": [{"start": 9}]}); n]
        })
    };
    let write = decode(&f, "ten-exclusions", with(10), &ceilings).expect("ten exclusions");
    f.try_create(write).await.expect("created");
    let before = footprint(&f).await;

    let error = decode(&f, "eleven-exclusions", with(11), &ceilings).expect_err("eleven");
    assert_eq!(refusal(&error).0, "SCHEDULE_SPEC_INVALID");
    assert_eq!(footprint(&f).await, before);
}

#[tokio::test]
async fn the_hard_ceiling_of_entries_holds_whatever_a_namespace_asks_for() {
    let f = fixture().await;
    let generous = Ceilings {
        spec_entries: 1_000,
        exclusions: 1_000,
        ..Ceilings::default()
    }
    .clamped();
    assert_eq!(
        generous.spec_entries, 32,
        "a namespace cannot raise the hard ceiling"
    );
    assert!(decode(&f, "thirty-two", intervals(32), &generous).is_ok());
    assert!(decode(&f, "thirty-three", intervals(33), &generous).is_err());
}

// --- Minimum spacing between nominal times: 60 s, floor 10 s ------------------------------------

#[tokio::test]
async fn two_times_closer_than_the_spacing_are_refused_and_the_spacing_itself_is_accepted() {
    let f = fixture().await;
    let every = |ms: u64| json!({"intervals": [{"everyMs": ms}]});
    f.create("at-the-spacing", &every(60_000), &json!({}), None)
        .await;
    let before = footprint(&f).await;

    let error = f
        .try_create(f.write_of("too-frequent", &every(30_000), &json!({}), None))
        .await
        .expect_err("30 s is closer than 60 s");
    assert_eq!(
        refusal(&error),
        ("SCHEDULE_TOO_FREQUENT", "/spec".to_owned())
    );
    assert_eq!(footprint(&f).await, before);
}

#[tokio::test]
async fn a_namespace_may_raise_the_spacing_but_never_take_it_below_the_floor() {
    let lowered = Ceilings {
        min_spacing_ms: 1,
        ..Ceilings::default()
    }
    .clamped();
    assert_eq!(lowered.min_spacing_ms, 10_000, "the floor is 10 s");
    let raised = Ceilings {
        min_spacing_ms: 600_000,
        ..Ceilings::default()
    };
    assert_eq!(raised.clamped().min_spacing_ms, 600_000);
}

// --- Schedule input: 64 KiB ---------------------------------------------------------------------

#[tokio::test]
async fn an_input_of_exactly_64_kib_is_accepted_and_one_byte_more_is_refused_before_work() {
    let f = fixture().await;
    // {"b":"…"} is 8 bytes of structure around the text
    let input = |n: usize| json!({"b": "x".repeat(n)});
    assert_eq!(
        serde_json::to_vec(&input(65_528)).expect("json").len(),
        65_536
    );

    let mut at = f.write_of("at-the-ceiling", &hourly(), &json!({}), None);
    at.input = input(65_528);
    f.try_create(at).await.expect("exactly 64 KiB");
    let before = footprint(&f).await;

    let mut over = f.write_of("over-the-ceiling", &hourly(), &json!({}), None);
    over.input = input(65_529);
    let error = f.try_create(over).await.expect_err("one byte over");
    assert!(matches!(error, AppError::PayloadTooLarge(_)), "{error:?}");
    assert_eq!(footprint(&f).await, before, "nothing was stored or charged");
}

// --- Note: 512 characters; Remaining actions: 1,000,000 -----------------------------------------

#[tokio::test]
async fn a_note_of_512_characters_is_accepted_and_513_is_refused() {
    let f = fixture().await;
    let ceilings = Ceilings::default();
    let noted = |n: usize| {
        let mut doc = document(&f, "noted", hourly());
        doc.note = Some("n".repeat(n));
        doc
    };
    assert!(decode_document(noted(512), &ceilings).is_ok());
    let error = decode_document(noted(513), &ceilings).expect_err("513");
    assert_eq!(
        refusal(&error),
        ("SCHEDULE_SPEC_INVALID", "/note".to_owned())
    );
}

#[tokio::test]
async fn remaining_actions_run_from_one_to_a_million() {
    let f = fixture().await;
    let ceilings = Ceilings::default();
    let counted = |n: u32| {
        let mut doc = document(&f, "counted", hourly());
        doc.remaining_actions = Some(n);
        doc
    };
    assert!(decode_document(counted(1), &ceilings).is_ok());
    assert!(decode_document(counted(1_000_000), &ceilings).is_ok());
    for refused in [0, 1_000_001] {
        let error =
            decode_document(counted(refused), &ceilings).expect_err("outside 1..=1,000,000");
        assert_eq!(
            refusal(&error),
            ("SCHEDULE_SPEC_INVALID", "/remainingActions".to_owned())
        );
    }
}

// --- A ceiling lowered afterwards (NFR-018-003) --------------------------------------------------

#[tokio::test]
async fn a_lowered_ceiling_refuses_new_specs_while_a_schedule_made_before_it_keeps_firing() {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let wide = f.create("wide", &intervals(5), &json!({}), None).await;

    // The namespace now allows two entries: a new spec of five is refused ...
    let lowered = Ceilings {
        spec_entries: 2,
        ..Ceilings::default()
    };
    let error = decode(&f, "wide-again", intervals(5), &lowered).expect_err("over the new ceiling");
    assert_eq!(refusal(&error).0, "SCHEDULE_SPEC_INVALID");

    // ... and the five-entry schedule is still read back and still fires, under those ceilings.
    let firing = ScheduleFiring::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SpecCalendar,
        f.clock.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        Starts(Arc::clone(&f.w)),
        lowered,
    );
    f.clock
        .set(start + Duration::hours(1) + Duration::minutes(6));
    let due = f.due(&wide);
    let mut steps = 0;
    for _ in 0..10 {
        match firing.fire(&due, None).await.expect("fire") {
            Fired::Acted => steps += 1,
            Fired::Idle | Fired::Refused(_) => break,
        }
    }
    assert!(
        steps > 0,
        "the schedule made before the ceiling kept firing"
    );
    assert!(!f.runs(&wide).await.is_empty());
}

// --- Schedule actions per pass round per replica: 100 / 1,000 -----------------------------------

#[tokio::test]
async fn a_round_stops_at_its_action_ceiling_and_the_next_round_carries_on() {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let mut schedules = Vec::new();
    for i in 0..5 {
        schedules.push(
            f.create(&format!("s{i}"), &hourly(), &json!({}), None)
                .await,
        );
    }
    let a = replica_with(
        &f,
        "capped",
        ScheduleSettings {
            round_actions: 3,
            ..ScheduleSettings::default()
        },
    );
    a.keep().await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));

    let first = a.pass.round().await;
    assert_eq!((first.steps, first.failed), (3, 0), "{first:?}");
    let second = a.pass.round().await;
    assert_eq!((second.steps, second.failed), (2, 0), "{second:?}");
    assert_eq!(a.pass.round().await.steps, 0, "nothing is left");

    for schedule in &schedules {
        assert_eq!(
            f.runs(schedule).await.len(),
            1,
            "{} started once",
            schedule.name.as_str()
        );
        assert_eq!(f.outcomes(schedule).await, outcomes_of(&[("STARTED", 1)]));
    }
}
