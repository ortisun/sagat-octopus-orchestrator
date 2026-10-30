//! What a schedule starts: the workflow, its version policy and the options of the run it makes
//! (`FR-018-016`, `T018-016`).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::audit::ResourceType;
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::schedules::{
    ScheduleError, ScheduleId, ScheduleTarget, TargetParts, VersionPolicy,
};

fn parts() -> TargetParts {
    TargetParts {
        workflow: WorkflowId::generate(),
        version: VersionPolicy::Active,
        nominal_time_field: None,
        run_timeout_ms: None,
        business_id_prefix: None,
        business_id_conflict: ConflictPolicy::default(),
        business_id_reuse: ReusePolicy::default(),
    }
}

#[test]
fn a_schedule_id_is_an_id_of_its_own_that_round_trips() {
    let id = ScheduleId::generate();
    assert_eq!(id.to_string().parse::<ScheduleId>(), Ok(id));
    assert_eq!(ScheduleId::new(id.as_uuid()), id);
    assert!("not-a-uuid".parse::<ScheduleId>().is_err());
}

#[test]
fn a_name_is_one_to_a_hundred_and_twenty_eight_lowercase_characters_digits_and_dashes() {
    use sagat_domain::schedules::ScheduleName;
    for ok in ["a", "0", "nightly-report", "a-b-c", &"x".repeat(128)] {
        assert_eq!(ScheduleName::new(ok).expect(ok).as_str(), ok);
    }
    for bad in [
        "",
        "-leading",
        "Upper",
        "has space",
        "under_score",
        "caf\u{e9}",
        &"x".repeat(129),
    ] {
        assert_eq!(
            ScheduleName::new(bad).expect_err(bad),
            ScheduleError::BadName,
            "{bad:?}"
        );
    }
}

#[test]
fn the_audit_trail_names_schedules_and_batch_jobs() {
    assert_eq!(ResourceType::Schedule.as_str(), "schedule");
    assert_eq!(ResourceType::BatchJob.as_str(), "batch_job");
}

#[test]
fn a_target_with_nothing_but_a_workflow_follows_the_active_version() {
    let p = parts();
    let workflow = p.workflow;
    let target = ScheduleTarget::new(p).expect("target");
    assert_eq!(target.workflow(), workflow);
    assert_eq!(target.version(), VersionPolicy::Active);
    assert_eq!(target.nominal_time_field(), None);
    assert_eq!(target.run_timeout_ms(), None);
    assert!(target.business_id().is_none());
}

#[test]
fn a_pinned_target_names_its_version() {
    let version = WorkflowVersionId::generate();
    let target = ScheduleTarget::new(TargetParts {
        version: VersionPolicy::Pinned(version),
        ..parts()
    })
    .expect("target");
    assert_eq!(target.version(), VersionPolicy::Pinned(version));
}

#[test]
fn the_nominal_time_field_is_a_top_level_identifier() {
    for ok in ["scheduledFor", "_t", "a", &"x".repeat(64)] {
        let target = ScheduleTarget::new(TargetParts {
            nominal_time_field: Some(ok.to_owned()),
            ..parts()
        })
        .expect(ok);
        assert_eq!(target.nominal_time_field(), Some(ok));
    }
    for bad in [
        "",
        "1st",
        "has space",
        "a-b",
        "a.b",
        "caf\u{e9}",
        &"x".repeat(65),
    ] {
        let err = ScheduleTarget::new(TargetParts {
            nominal_time_field: Some(bad.to_owned()),
            ..parts()
        })
        .expect_err(bad);
        assert_eq!(err, ScheduleError::BadNominalTimeField, "{bad:?}");
    }
}

#[test]
fn the_run_timeout_is_a_second_to_a_leap_year() {
    for ok in [1_000, 60_000, 31_622_400_000] {
        assert!(
            ScheduleTarget::new(TargetParts {
                run_timeout_ms: Some(ok),
                ..parts()
            })
            .is_ok(),
            "{ok}"
        );
    }
    for bad in [0, 999, 31_622_400_001] {
        assert_eq!(
            ScheduleTarget::new(TargetParts {
                run_timeout_ms: Some(bad),
                ..parts()
            })
            .expect_err("timeout"),
            ScheduleError::BadRunTimeout,
            "{bad}"
        );
    }
}

#[test]
fn a_business_id_prefix_leaves_room_for_the_nominal_time() {
    // prefix + "-" + 20 characters of RFC 3339 UTC seconds must fit the 200 of a business id
    let ok = ScheduleTarget::new(TargetParts {
        business_id_prefix: Some("o".repeat(160)),
        business_id_conflict: ConflictPolicy::UseExisting,
        business_id_reuse: ReusePolicy::RejectDuplicate,
        ..parts()
    })
    .expect("160");
    let id = ok.business_id().expect("business id");
    assert_eq!(id.prefix().len(), 160);
    assert_eq!(id.conflict(), ConflictPolicy::UseExisting);
    assert_eq!(id.reuse(), ReusePolicy::RejectDuplicate);

    for bad in ["", "has space", "caf\u{e9}", &"o".repeat(161)] {
        let err = ScheduleTarget::new(TargetParts {
            business_id_prefix: Some(bad.to_owned()),
            ..parts()
        })
        .expect_err(bad);
        assert_eq!(err, ScheduleError::BadBusinessIdPrefix, "{bad:?}");
    }
}

#[test]
fn the_business_id_of_an_action_is_the_prefix_a_dash_and_the_nominal_time() {
    let target = ScheduleTarget::new(TargetParts {
        business_id_prefix: Some("order".to_owned()),
        ..parts()
    })
    .expect("target");
    let id = target
        .business_id()
        .expect("business id")
        .for_nominal_time("2026-10-29T09:00:00Z")
        .expect("id");
    assert_eq!(id.as_str(), "order-2026-10-29T09:00:00Z");
    // the longest prefix and a full-width time still make a legal business id
    let widest = ScheduleTarget::new(TargetParts {
        business_id_prefix: Some("o".repeat(160)),
        ..parts()
    })
    .expect("target");
    assert!(
        widest
            .business_id()
            .expect("business id")
            .for_nominal_time("2026-10-29T09:00:00Z")
            .is_ok()
    );
    assert_eq!(
        widest
            .business_id()
            .expect("business id")
            .for_nominal_time(&"9".repeat(60))
            .expect_err("too long"),
        ScheduleError::BadBusinessIdPrefix,
        "a nominal time that would push the id past 200 characters is refused, not truncated"
    );
}
