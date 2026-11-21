//! `control-plane capability-activate lease-withdrawal-v1` (feature 006, decisions D006-004 and D006-034; ADR-0098 decision 4;
//! the pattern of ADR-0082) against the real registry: it records `lease-withdrawal-v1` of control planes and engines only
//! when every live process lists it and the count is the one expected, once, with the owner role. The command is the one
//! every fleet requirement an operator activates goes through.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::instances::{Capability, InstanceKind};
use sagat_control_plane::RunError;
use sagat_control_plane::capability_activation::{
    ACTIVATABLE, Activatable, ActivationArgs, ActivationOutcome, capability_activate,
};
use sagat_control_plane::cli::{self, Command};
use sagat_control_plane::config::DatabaseConfig;

const LEASE: &Activatable = &ACTIVATABLE[0];

use crate::support::instances::{Registry, registry};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

fn database(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    }
}

fn args(operator: Option<&str>, expect_live: Option<u32>, dry_run: bool) -> ActivationArgs {
    ActivationArgs {
        capability: "lease-withdrawal-v1".to_owned(),
        operator: operator.map(str::to_owned),
        expect_live,
        dry_run,
    }
}

async fn activate(r: &Registry, a: &ActivationArgs) -> Result<ActivationOutcome, RunError> {
    capability_activate(&database(r.db.url.clone()), a).await
}

async fn recorded(r: &Registry) -> Option<Vec<String>> {
    sqlx::query_scalar(
        "SELECT kinds FROM platform_requirements WHERE capability = 'lease-withdrawal-v1'",
    )
    .fetch_optional(&r.db.pool)
    .await
    .expect("requirements")
}

fn declined(result: Result<ActivationOutcome, RunError>) -> String {
    match result {
        Err(RunError::Declined(message)) => message,
        other => panic!("expected a refusal in words, got {other:?}"),
    }
}

async fn fleet(r: &Registry, lacking: bool) -> uuid::Uuid {
    r.serving(InstanceKind::ControlPlane, &[Capability::LeaseWithdrawalV1])
        .await;
    r.serving(
        InstanceKind::ExecutionEngine,
        &[Capability::LeaseWithdrawalV1],
    )
    .await;
    let capabilities: &[Capability] = if lacking {
        &[Capability::RunInputValidationV1]
    } else {
        &[Capability::LeaseWithdrawalV1]
    };
    r.serving(InstanceKind::ExecutionEngine, capabilities).await
}

#[tokio::test]
async fn it_refuses_while_a_live_process_lacks_the_capability_and_names_it() {
    let r = registry().await;
    let lacking = fleet(&r, true).await;
    let message = declined(activate(&r, &args(Some("ana"), Some(3), false)).await);
    assert!(
        message.contains("1 live process(es) do not list lease-withdrawal-v1"),
        "{message}"
    );
    assert!(message.contains(&lacking.to_string()), "{message}");
    assert_eq!(recorded(&r).await, None);
}

#[tokio::test]
async fn it_refuses_a_count_that_is_not_the_one_expected_and_an_empty_fleet_without_one() {
    let r = registry().await;
    let message = declined(activate(&r, &args(Some("ana"), None, false)).await);
    assert!(
        message.contains("no control plane or execution engine is live"),
        "{message}"
    );
    fleet(&r, false).await;
    for expected in [2, 4] {
        let message = declined(activate(&r, &args(Some("ana"), Some(expected), false)).await);
        assert!(
            message.contains("3 control plane(s) and execution engine(s) are live"),
            "{message}"
        );
    }
    assert_eq!(recorded(&r).await, None);
}

#[tokio::test]
async fn a_dry_run_changes_nothing_and_the_activation_records_both_kinds_once() {
    let r = registry().await;
    fleet(&r, false).await;
    assert_eq!(
        activate(&r, &args(None, Some(3), true))
            .await
            .expect("dry run"),
        ActivationOutcome::WouldActivate {
            entry: LEASE,
            live: 3
        }
    );
    assert_eq!(recorded(&r).await, None);
    let epoch = r.epoch_now().await;
    match activate(&r, &args(Some("ana"), Some(3), false))
        .await
        .expect("activate")
    {
        ActivationOutcome::Activated {
            live, epoch: now, ..
        } => {
            assert_eq!(live, 3);
            assert_eq!(now, epoch + 1, "the epoch moves once");
        }
        other => panic!("{other:?}"),
    }
    let mut kinds = recorded(&r).await.expect("recorded");
    kinds.sort();
    assert_eq!(kinds, ["CONTROL_PLANE", "EXECUTION_ENGINE"]);
    assert_eq!(
        activate(&r, &args(Some("ana"), Some(3), false))
            .await
            .expect("again"),
        ActivationOutcome::AlreadyActive { entry: LEASE }
    );
    assert_eq!(r.history().await, 1, "the history has one activation");
}

#[tokio::test]
async fn the_runtime_role_cannot_run_it_and_is_told_to_use_the_owner_role() {
    let r = registry().await;
    fleet(&r, false).await;
    let runtime = database(r.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));
    for dry_run in [false, true] {
        match capability_activate(&runtime, &args(Some("ana"), Some(3), dry_run)).await {
            Err(RunError::Declined(message)) => {
                assert!(message.contains("schema-owning database role"), "{message}");
            }
            other => panic!("dry_run={dry_run}: {other:?}"),
        }
    }
    assert_eq!(recorded(&r).await, None);
}

#[test]
fn the_command_line_takes_a_capability_and_the_three_flags_and_nothing_else() {
    let parse =
        |line: &[&str]| cli::parse(&line.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    assert_eq!(
        parse(&[
            "capability-activate",
            "lease-withdrawal-v1",
            "--operator",
            "ana",
            "--expect-live=3",
            "--dry-run"
        ]),
        Ok(Command::CapabilityActivate(args(
            Some("ana"),
            Some(3),
            true
        )))
    );
    for wrong in [
        &[
            "capability-activate",
            "lease-withdrawal-v1",
            "--operator",
            "--dry-run",
        ][..],
        &[
            "capability-activate",
            "lease-withdrawal-v1",
            "--expect-live",
            "many",
        ][..],
        &["capability-activate", "lease-withdrawal-v1", "--apply"][..],
        // the capability is named first, it is one the command may activate, and only once
        &["capability-activate", "--operator", "ana"][..],
        &["capability-activate"][..],
        &[
            "capability-activate",
            "lease-withdrawal-v2",
            "--operator",
            "ana",
        ][..],
        &["capability-activate", "schedules-v1", "--operator", "ana"][..],
        &[
            "capability-activate",
            "lease-withdrawal-v1",
            "lease-withdrawal-v1",
        ][..],
        // never released, so it has no alias (decision D006-034)
        &["lease-withdrawal-activate", "--operator", "ana"][..],
    ] {
        assert!(parse(wrong).is_err(), "{wrong:?}");
    }
}

/// While no journal holds `step.lease_withdrawn`, the requirement can be removed again (the way back before any withdrawal,
/// decision D006-032); the refusal once one does is proved over the real engine (`contract_grpc_durable_execution`).
#[tokio::test]
async fn before_any_withdrawal_the_requirement_can_be_removed() {
    let r = registry().await;
    fleet(&r, false).await;
    activate(&r, &args(Some("ana"), Some(3), false))
        .await
        .expect("activated");
    let outcome: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'ana')",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("removed");
    assert_eq!(outcome, "REMOVED");
    assert_eq!(recorded(&r).await, None);
}
