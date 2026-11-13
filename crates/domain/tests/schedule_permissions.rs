//! Who may do what with schedules and batch jobs, and what the audit trail calls it
//! (`FR-018-041`, `T018-007`).
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_domain::audit::AuditAction;
use sagat_domain::identity::{Permission, Role};

#[test]
fn the_four_permissions_have_their_names() {
    assert_eq!(Permission::ScheduleRead.as_str(), "schedule:read");
    assert_eq!(Permission::ScheduleWrite.as_str(), "schedule:write");
    assert_eq!(Permission::ScheduleOperate.as_str(), "schedule:operate");
    assert_eq!(Permission::RunBatch.as_str(), "run:batch");
}

#[test]
fn the_catalog_lists_each_permission_once_under_its_own_name() {
    let names: BTreeSet<_> = Permission::ALL.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        names.len(),
        Permission::ALL.len(),
        "two permissions share a name"
    );
    for added in [
        Permission::ScheduleRead,
        Permission::ScheduleWrite,
        Permission::ScheduleOperate,
        Permission::RunBatch,
    ] {
        assert_eq!(
            Permission::ALL.iter().filter(|p| **p == added).count(),
            1,
            "{added:?}"
        );
    }
}

/// The authorization matrix of the plan: read is for everyone who looks at runs; creating,
/// changing and deleting is for the people who write workflows; pause, resume, trigger and backfill
/// for the people who operate them; a batch multiplies blast radius, so it is for operators only
/// (O1002).
#[test]
fn the_roles_hold_what_the_authorization_matrix_gives_them() {
    use Permission::{RunBatch, ScheduleOperate, ScheduleRead, ScheduleWrite};
    let matrix: [(Role, [bool; 4]); 7] = [
        // read, write, operate, batch
        (Role::Admin, [true, true, true, true]),
        (Role::Developer, [true, true, true, false]),
        (Role::Operator, [true, false, true, true]),
        (Role::Viewer, [true, false, false, false]),
        (Role::Worker, [false, false, false, false]),
        (Role::Runner, [false, false, false, false]),
        (Role::DatabaseRunner, [false, false, false, false]),
    ];
    for (role, expected) in matrix {
        let actual = [
            role.grants(ScheduleRead),
            role.grants(ScheduleWrite),
            role.grants(ScheduleOperate),
            role.grants(RunBatch),
        ];
        assert_eq!(actual, expected, "{role}: read, write, operate, batch");
    }
}

#[test]
fn an_administrator_holds_every_permission_but_the_pii_reveal() {
    for permission in Permission::ALL {
        if permission == Permission::PayloadRevealPii {
            assert!(
                !Role::Admin.grants(permission),
                "reveal is a grant, never a role"
            );
        } else {
            assert!(Role::Admin.grants(permission), "{permission:?}");
        }
    }
}

#[test]
fn the_audit_actions_have_the_names_of_the_plan() {
    let named = [
        (AuditAction::ScheduleCreated, "schedule.created"),
        (AuditAction::ScheduleUpdated, "schedule.updated"),
        (AuditAction::ScheduleDeleted, "schedule.deleted"),
        (AuditAction::SchedulePaused, "schedule.paused"),
        (AuditAction::ScheduleResumed, "schedule.resumed"),
        (AuditAction::ScheduleAutoPaused, "schedule.auto_paused"),
        (AuditAction::ScheduleTriggered, "schedule.triggered"),
        (
            AuditAction::ScheduleBackfillRequested,
            "schedule.backfill_requested",
        ),
        (
            AuditAction::ScheduleBackfillCancelled,
            "schedule.backfill_cancelled",
        ),
        (
            AuditAction::ScheduleInputDisclosed,
            "schedule.input_disclosed",
        ),
        (
            AuditAction::ScheduleTriggerHandover,
            "schedule.trigger_handover",
        ),
        (AuditAction::BatchCreated, "batch.created"),
        (AuditAction::BatchCancelled, "batch.cancelled"),
        (AuditAction::BatchFinished, "batch.finished"),
        (AuditAction::RunReset, "run.reset"),
        (AuditAction::RunResetRequested, "run.reset_requested"),
    ];
    let unique: BTreeSet<_> = named.iter().map(|(_, name)| *name).collect();
    assert_eq!(unique.len(), named.len());
    for (action, name) in named {
        assert_eq!(action.as_str(), name);
    }
}

#[test]
fn reading_a_schedules_input_collapses_repeats_like_every_other_disclosure() {
    // an auditor who opens the same schedule five times a minute leaves one record per window
    assert!(AuditAction::ScheduleInputDisclosed.collapses_repeats());
    assert!(
        AuditAction::ScheduleInputDisclosed
            .repeat_window()
            .is_some()
    );
    for action in [
        AuditAction::ScheduleCreated,
        AuditAction::BatchCreated,
        AuditAction::RunReset,
    ] {
        assert!(
            !action.collapses_repeats(),
            "{action:?}: a change is always a new event"
        );
    }
}
