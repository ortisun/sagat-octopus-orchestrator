//! Which activities a worker may advertise (feature 035, D035-135). The reserved prefix `sagat.database.` belongs to the
//! dedicated runner role, and a runner is of one profile: the SQL profile (`sagat.database.postgresql`) or the lake
//! profile (`sagat.database.iceberg.write`). The profile is the set of activities it registers, authorised by the
//! patterns of its binding; it is never mixed, and never an ordinary activity beside it.
#![allow(clippy::expect_used)]

use sagat_application::worker::{DATABASE_PROTOCOL_MINOR, managed_capabilities_allowed};
use sagat_domain::{
    database::{ICEBERG_REVERT_ACTIVITY, ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY},
    identity::Role,
};

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_database_runner_registers_for_the_sql_profile_or_for_the_lake_profile() {
    for profile in [MANAGED_DATABASE_ACTIVITY, ICEBERG_WRITE_ACTIVITY] {
        assert!(
            managed_capabilities_allowed(
                Role::DatabaseRunner,
                &names(&[profile]),
                DATABASE_PROTOCOL_MINOR
            ),
            "{profile}"
        );
        assert!(
            managed_capabilities_allowed(
                Role::DatabaseRunner,
                &names(&[profile, profile]),
                DATABASE_PROTOCOL_MINOR + 1
            ),
            "a newer minor and a repeated name are the same profile: {profile}"
        );
    }
}

#[test]
fn a_runner_is_of_one_profile_and_offers_nothing_ordinary_beside_it() {
    let mixed = [
        vec![MANAGED_DATABASE_ACTIVITY, ICEBERG_WRITE_ACTIVITY],
        vec![ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY],
        vec![MANAGED_DATABASE_ACTIVITY, "ordinary"],
        vec![ICEBERG_WRITE_ACTIVITY, "ordinary"],
        vec!["ordinary"],
    ];
    for activities in mixed {
        assert!(
            !managed_capabilities_allowed(
                Role::DatabaseRunner,
                &names(&activities),
                DATABASE_PROTOCOL_MINOR
            ),
            "{activities:?}"
        );
    }
    assert!(!managed_capabilities_allowed(
        Role::DatabaseRunner,
        &[],
        DATABASE_PROTOCOL_MINOR
    ));
}

#[test]
fn only_what_the_server_serves_is_accepted_and_nothing_below_the_managed_minor() {
    // The revert is a compensation node, which is not yet a managed step (feature 035, D035-135); the others are tasks
    // of later stories. A runner that offers them is refused instead of waiting for work that cannot come.
    for not_yet in [
        ICEBERG_REVERT_ACTIVITY,
        "sagat.database.destination.iceberg",
        "sagat.database.destination.object_storage",
        "sagat.database.lake.probe",
        "sagat.database.lake.browse",
        "sagat.database.postgresql.other",
        "sagat.database.",
    ] {
        assert!(
            !managed_capabilities_allowed(
                Role::DatabaseRunner,
                &names(&[not_yet]),
                DATABASE_PROTOCOL_MINOR
            ),
            "{not_yet}"
        );
    }
    for profile in [MANAGED_DATABASE_ACTIVITY, ICEBERG_WRITE_ACTIVITY] {
        assert!(
            !managed_capabilities_allowed(
                Role::DatabaseRunner,
                &names(&[profile]),
                DATABASE_PROTOCOL_MINOR - 1
            ),
            "{profile} needs the managed minor"
        );
    }
}

#[test]
fn no_other_role_registers_under_the_reserved_prefix() {
    for role in [Role::Worker, Role::Runner] {
        for reserved in [MANAGED_DATABASE_ACTIVITY, ICEBERG_WRITE_ACTIVITY] {
            assert!(
                !managed_capabilities_allowed(
                    role,
                    &names(&["ordinary", reserved]),
                    DATABASE_PROTOCOL_MINOR
                ),
                "{role:?} {reserved}"
            );
        }
        assert!(managed_capabilities_allowed(
            role,
            &names(&["ordinary", "payments.charge"]),
            DATABASE_PROTOCOL_MINOR
        ));
    }
}
