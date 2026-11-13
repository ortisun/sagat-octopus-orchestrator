//! Which activities a worker may advertise (feature 035, D035-135). The reserved prefix `sagat.database.` belongs to the
//! dedicated runner role, and a runner is of one profile: the SQL profile (`sagat.database.postgresql`) or the lake
//! profile (`sagat.database.iceberg.write`). The profile is the set of activities it registers, authorised by the
//! patterns of its binding; it is never mixed, and never an ordinary activity beside it.
#![allow(clippy::expect_used)]

use sagat_application::worker::{DATABASE_PROTOCOL_MINOR, managed_capabilities_allowed};
use sagat_domain::{
    database::{
        ICEBERG_PROBE_ACTIVITY, ICEBERG_REVERT_ACTIVITY, ICEBERG_WRITE_ACTIVITY,
        MANAGED_DATABASE_ACTIVITY,
    },
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
        vec![ICEBERG_PROBE_ACTIVITY, MANAGED_DATABASE_ACTIVITY],
        vec![MANAGED_DATABASE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
        vec![ICEBERG_PROBE_ACTIVITY, "ordinary"],
        vec![
            ICEBERG_WRITE_ACTIVITY,
            ICEBERG_PROBE_ACTIVITY,
            MANAGED_DATABASE_ACTIVITY,
        ],
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

#[test]
fn the_profile_of_a_registration_is_the_profile_of_the_activities_it_offers() {
    use sagat_application::worker::managed_profile;
    use sagat_domain::binding::BindingProfile;
    assert_eq!(
        managed_profile(&names(&[MANAGED_DATABASE_ACTIVITY])),
        Some(BindingProfile::Database)
    );
    assert_eq!(
        managed_profile(&names(&[ICEBERG_WRITE_ACTIVITY, ICEBERG_WRITE_ACTIVITY])),
        Some(BindingProfile::Lake)
    );
    // the lake profile is the write, the probe, or both: a lake runner that has a probe to run offers it beside its write
    for lake in [
        vec![ICEBERG_PROBE_ACTIVITY],
        vec![ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
        vec![
            ICEBERG_PROBE_ACTIVITY,
            ICEBERG_WRITE_ACTIVITY,
            ICEBERG_PROBE_ACTIVITY,
        ],
    ] {
        assert_eq!(
            managed_profile(&names(&lake)),
            Some(BindingProfile::Lake),
            "{lake:?}"
        );
        assert!(
            managed_capabilities_allowed(
                Role::DatabaseRunner,
                &names(&lake),
                DATABASE_PROTOCOL_MINOR
            ),
            "{lake:?}"
        );
    }
    for neither in [
        vec![],
        vec![MANAGED_DATABASE_ACTIVITY, ICEBERG_WRITE_ACTIVITY],
        vec![MANAGED_DATABASE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
        vec![ICEBERG_WRITE_ACTIVITY, "ordinary"],
        vec!["ordinary"],
        vec![ICEBERG_REVERT_ACTIVITY],
    ] {
        assert_eq!(managed_profile(&names(&neither)), None, "{neither:?}");
    }
}

#[test]
fn credentials_are_handed_only_to_a_session_and_a_binding_of_the_profile_of_the_step() {
    use sagat_application::worker::profile_fence;
    use sagat_domain::{binding::BindingProfile, database::ManagedKind};
    let sql = names(&[MANAGED_DATABASE_ACTIVITY]);
    let lake = names(&[ICEBERG_WRITE_ACTIVITY]);
    let (database, lake_profile) = (BindingProfile::Database, BindingProfile::Lake);
    // the matching session and binding
    assert!(profile_fence(ManagedKind::Sql, &sql, &[database]));
    assert!(profile_fence(ManagedKind::Lake, &lake, &[lake_profile]));
    // the other profile's session, the other profile's binding, or both
    assert!(!profile_fence(ManagedKind::Sql, &lake, &[database]));
    assert!(!profile_fence(ManagedKind::Lake, &sql, &[lake_profile]));
    assert!(!profile_fence(ManagedKind::Sql, &sql, &[lake_profile]));
    assert!(!profile_fence(ManagedKind::Lake, &lake, &[database]));
    // a binding with no row, or with rows that disagree, is not a binding of the profile
    assert!(!profile_fence(ManagedKind::Sql, &sql, &[]));
    assert!(!profile_fence(
        ManagedKind::Sql,
        &sql,
        &[database, lake_profile]
    ));
    assert!(!profile_fence(
        ManagedKind::Lake,
        &lake,
        &[lake_profile, database]
    ));
    // a session that offers anything else, mixed or empty, is not of a profile at all
    assert!(!profile_fence(ManagedKind::Sql, &[], &[database]));
    assert!(!profile_fence(
        ManagedKind::Sql,
        &names(&[MANAGED_DATABASE_ACTIVITY, "ordinary"]),
        &[database]
    ));
    assert!(!profile_fence(
        ManagedKind::Lake,
        &names(&[ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY]),
        &[lake_profile]
    ));
}

#[test]
fn a_step_is_handed_only_to_a_session_that_offers_its_own_activity_and_not_merely_its_profile() {
    use sagat_application::worker::profile_fence;
    use sagat_domain::{binding::BindingProfile, database::ManagedKind};
    let lake_profile = BindingProfile::Lake;
    let write_only = names(&[ICEBERG_WRITE_ACTIVITY]);
    let probe_only = names(&[ICEBERG_PROBE_ACTIVITY]);
    let both = names(&[ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY]);
    // a probe goes to a session that offers the probe, a write to one that offers the write
    assert!(profile_fence(
        ManagedKind::LakeProbe,
        &probe_only,
        &[lake_profile]
    ));
    assert!(profile_fence(
        ManagedKind::LakeProbe,
        &both,
        &[lake_profile]
    ));
    assert!(profile_fence(ManagedKind::Lake, &both, &[lake_profile]));
    // the same profile is not enough: a runner that has no probe to run is never handed a probe, nor one that has only the probe a write
    assert!(!profile_fence(
        ManagedKind::LakeProbe,
        &write_only,
        &[lake_profile]
    ));
    assert!(!profile_fence(
        ManagedKind::Lake,
        &probe_only,
        &[lake_profile]
    ));
    // and the binding rules are the lake's as for a write
    assert!(!profile_fence(
        ManagedKind::LakeProbe,
        &both,
        &[BindingProfile::Database]
    ));
    assert!(!profile_fence(ManagedKind::LakeProbe, &both, &[]));
}

#[test]
fn every_managed_kind_names_the_activity_a_runner_offers_for_it() {
    use sagat_domain::database::ManagedKind;
    for (kind, activity) in [
        (ManagedKind::Sql, MANAGED_DATABASE_ACTIVITY),
        (ManagedKind::Lake, ICEBERG_WRITE_ACTIVITY),
        (ManagedKind::LakeProbe, ICEBERG_PROBE_ACTIVITY),
    ] {
        assert_eq!(kind.activity(), activity, "{kind:?}");
    }
}
