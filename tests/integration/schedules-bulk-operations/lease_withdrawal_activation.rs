//! `control-plane lease-withdrawal-activate` (feature 006, decision D006-004; ADR-0098 decision 4; the pattern of ADR-0082)
//! against the real registry: it records `lease-withdrawal-v1` of control planes and engines only when every live process
//! lists it and the count is the one expected, once, with the owner role.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::instances::{Capability, InstanceKind};
use sagat_control_plane::RunError;
use sagat_control_plane::cli::{self, Command};
use sagat_control_plane::config::DatabaseConfig;
use sagat_control_plane::lease_withdrawal::{
    ActivationArgs, ActivationOutcome, lease_withdrawal_activate,
};

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
        operator: operator.map(str::to_owned),
        expect_live,
        dry_run,
    }
}

async fn activate(r: &Registry, a: &ActivationArgs) -> Result<ActivationOutcome, RunError> {
    lease_withdrawal_activate(&database(r.db.url.clone()), a).await
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
        ActivationOutcome::WouldActivate { live: 3 }
    );
    assert_eq!(recorded(&r).await, None);
    let epoch = r.epoch_now().await;
    match activate(&r, &args(Some("ana"), Some(3), false))
        .await
        .expect("activate")
    {
        ActivationOutcome::Activated { live, epoch: now } => {
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
        ActivationOutcome::AlreadyActive
    );
    assert_eq!(r.history().await, 1, "the history has one activation");
}

#[tokio::test]
async fn the_runtime_role_cannot_run_it_and_is_told_to_use_the_owner_role() {
    let r = registry().await;
    fleet(&r, false).await;
    let runtime = database(r.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));
    for dry_run in [false, true] {
        match lease_withdrawal_activate(&runtime, &args(Some("ana"), Some(3), dry_run)).await {
            Err(RunError::Declined(message)) => {
                assert!(message.contains("schema-owning database role"), "{message}");
            }
            other => panic!("dry_run={dry_run}: {other:?}"),
        }
    }
    assert_eq!(recorded(&r).await, None);
}

#[test]
fn the_command_line_takes_the_three_flags_and_nothing_else() {
    let parse =
        |line: &[&str]| cli::parse(&line.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    assert_eq!(
        parse(&[
            "lease-withdrawal-activate",
            "--operator",
            "ana",
            "--expect-live=3",
            "--dry-run"
        ]),
        Ok(Command::LeaseWithdrawalActivate(args(
            Some("ana"),
            Some(3),
            true
        )))
    );
    for wrong in [
        &["lease-withdrawal-activate", "--operator", "--dry-run"][..],
        &["lease-withdrawal-activate", "--expect-live", "many"][..],
        &["lease-withdrawal-activate", "--apply"][..],
    ] {
        assert!(parse(wrong).is_err(), "{wrong:?}");
    }
}
