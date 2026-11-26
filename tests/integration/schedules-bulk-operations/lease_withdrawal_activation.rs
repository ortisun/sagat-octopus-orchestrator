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
    // the floor of protocol 1.8 is one the command activates since the follow-up of pull request #260 (decision D006-096)
    assert_eq!(
        parse(&[
            "capability-activate",
            "lease-withdrawal-v2",
            "--operator",
            "ana"
        ]),
        Ok(Command::CapabilityActivate(ActivationArgs {
            capability: "lease-withdrawal-v2".to_owned(),
            operator: Some("ana".to_owned()),
            expect_live: None,
            dry_run: false,
        }))
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
            "lease-withdrawal-v3",
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

/// Decision D006-082: the census says which of the kinds asked have a live process, so an entry that requires one of each
/// (feature 022's rule, `each_kind_live`) can refuse a fleet of one kind only. A silent process does not count.
#[tokio::test]
async fn the_census_names_the_kinds_that_have_a_live_process() {
    let r = registry().await;
    let census = |r: &Registry| {
        let pool = r.db.pool.clone();
        async move {
            let mut connection = pool.acquire().await.expect("a connection");
            sagat_infra_postgres::capability_census(
                &mut connection,
                Capability::LeaseWithdrawalV1,
                &InstanceKind::ALL,
            )
            .await
            .expect("census")
        }
    };
    assert!(census(&r).await.kinds_live.is_empty(), "nothing is live");
    r.serving(
        InstanceKind::ExecutionEngine,
        &[Capability::LeaseWithdrawalV1],
    )
    .await;
    r.serving(
        InstanceKind::ExecutionEngine,
        &[Capability::LeaseWithdrawalV1],
    )
    .await;
    let engines = census(&r).await;
    assert_eq!(engines.live, 2);
    assert_eq!(engines.kinds_live, vec![InstanceKind::ExecutionEngine]);
    let control_plane = r
        .serving(InstanceKind::ControlPlane, &[Capability::LeaseWithdrawalV1])
        .await;
    assert_eq!(census(&r).await.kinds_live, InstanceKind::ALL.to_vec());
    r.age(control_plane, 40).await;
    assert_eq!(
        census(&r).await.kinds_live,
        vec![InstanceKind::ExecutionEngine],
        "a silent process is not live"
    );
}

fn v2_args(dry_run: bool) -> ActivationArgs {
    ActivationArgs {
        capability: "lease-withdrawal-v2".to_owned(),
        operator: Some("ana".to_owned()),
        expect_live: Some(2),
        dry_run,
    }
}

async fn recorded_v2(r: &Registry) -> Option<Vec<String>> {
    sqlx::query_scalar(
        "SELECT kinds FROM platform_requirements WHERE capability = 'lease-withdrawal-v2'",
    )
    .fetch_optional(&r.db.pool)
    .await
    .expect("requirements")
}

/// A control plane and an engine of the follow-up's build: both list the two capabilities.
async fn fleet_of_the_follow_up(r: &Registry) {
    for kind in InstanceKind::ALL {
        r.serving(
            kind,
            &[Capability::LeaseWithdrawalV1, Capability::LeaseWithdrawalV2],
        )
        .await;
    }
}

/// Request (9) of decision D006-059 (Task 006-071, decision D006-096): `lease-withdrawal-v2` is activated only after
/// `lease-withdrawal-v1` is recorded for both kinds. Without it, and with it recorded for the engines only, the command
/// refuses, a dry run too, naming both; with it, the requirement is recorded for both kinds, once.
#[tokio::test]
async fn lease_withdrawal_v2_is_refused_until_lease_withdrawal_v1_is_recorded_for_both_kinds() {
    let r = registry().await;
    fleet_of_the_follow_up(&r).await;
    for dry_run in [true, false] {
        let message = declined(activate(&r, &v2_args(dry_run)).await);
        assert!(
            message.contains("lease-withdrawal-v2 follows lease-withdrawal-v1"),
            "dry_run={dry_run}: {message}"
        );
    }
    assert_eq!(recorded_v2(&r).await, None);
    // recorded by hand for one kind: not the requirement it follows
    r.require(
        Capability::LeaseWithdrawalV1,
        &[InstanceKind::ExecutionEngine],
    )
    .await
    .expect("recorded for the engines");
    let message = declined(activate(&r, &v2_args(false)).await);
    assert!(message.contains("follows lease-withdrawal-v1"), "{message}");
    assert_eq!(recorded_v2(&r).await, None);
    sqlx::query_scalar::<_, String>(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'ana')",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("removed");
    activate(&r, &args(Some("ana"), Some(2), false))
        .await
        .expect("lease-withdrawal-v1 for both kinds");
    match activate(&r, &v2_args(false)).await.expect("activated") {
        ActivationOutcome::Activated { entry, live, .. } => {
            assert_eq!(entry.capability, Capability::LeaseWithdrawalV2);
            assert_eq!(live, 2);
        }
        other => panic!("{other:?}"),
    }
    let mut kinds = recorded_v2(&r).await.expect("recorded");
    kinds.sort();
    assert_eq!(kinds, ["CONTROL_PLANE", "EXECUTION_ENGINE"]);
    assert!(matches!(
        activate(&r, &v2_args(false)).await,
        Ok(ActivationOutcome::AlreadyActive { .. })
    ));
}

/// Decision D006-096: the requirement it follows is read again under the epoch lock. A removal of `lease-withdrawal-v1`
/// that commits while the activation of `lease-withdrawal-v2` waits for the lock it holds makes the activation refuse,
/// though its first check found the requirement.
#[tokio::test]
async fn the_requirement_it_follows_is_read_again_under_the_epoch_lock() {
    let r = registry().await;
    fleet_of_the_follow_up(&r).await;
    activate(&r, &args(Some("ana"), Some(2), false))
        .await
        .expect("lease-withdrawal-v1");
    let mut removal = r.db.pool.begin().await.expect("a transaction");
    let removed: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v1', 'ana')",
    )
    .fetch_one(&mut *removal)
    .await
    .expect("removed, not committed");
    assert_eq!(removed, "REMOVED");
    // the first check reads the committed row, so it passes; the activation then waits for the epoch lock
    let url = r.db.url.clone();
    let activation =
        tokio::spawn(async move { capability_activate(&database(url), &v2_args(false)).await });
    r.wait_for_lock_waiters(1, "the activation waits for the epoch lock")
        .await;
    removal.commit().await.expect("the removal commits");
    let message = declined(activation.await.expect("the activation ended"));
    assert!(
        message.contains("lease-withdrawal-v2 follows lease-withdrawal-v1"),
        "{message}"
    );
    assert_eq!(recorded_v2(&r).await, None);
}

/// Decision D006-096: `lease-withdrawal-v2` is a replay floor of one cause. It can be removed while the journals hold only
/// causes a build without it replays (`NOT_SENT` here), and is refused once one holds `NOT_ACKNOWLEDGED` (written by SQL
/// fixture: nothing of this build writes it), and outside READ COMMITTED.
#[tokio::test]
async fn lease_withdrawal_v2_cannot_be_removed_while_a_journal_holds_a_not_acknowledged_withdrawal()
{
    let r = registry().await;
    fleet_of_the_follow_up(&r).await;
    activate(&r, &args(Some("ana"), Some(2), false))
        .await
        .expect("lease-withdrawal-v1");
    let withdrawal = |cause: &'static str| {
        let pool = r.db.pool.clone();
        async move {
            let mut tx = pool.begin().await.expect("a transaction");
            sqlx::query("SET LOCAL session_replication_role = replica")
                .execute(&mut *tx)
                .await
                .expect("foreign keys off for this transaction");
            sqlx::query(
                "INSERT INTO run_events (tenant_id, run_id, seq, event_type, node, attempt, detail, actor, occurred_at) \
                 VALUES (gen_random_uuid(), gen_random_uuid(), 1, 'step.lease_withdrawn', 'a', 1, \
                         jsonb_build_object('attempt', 1, 'generation', 1, 'cause', $1::text), 'lease-withdrawal', now())",
            )
            .bind(cause)
            .execute(&mut *tx)
            .await
            .expect("a withdrawal in a journal");
            tx.commit().await.expect("committed");
        }
    };
    let remove = || {
        let pool = r.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v2', 'ana')",
            )
            .fetch_one(&pool)
            .await
            .map_err(|error| error.to_string())
        }
    };
    withdrawal("NOT_SENT").await;
    activate(&r, &v2_args(false))
        .await
        .expect("lease-withdrawal-v2");
    assert_eq!(
        remove().await.as_deref(),
        Ok("REMOVED"),
        "only a NOT_SENT withdrawal"
    );
    activate(&r, &v2_args(false)).await.expect("recorded again");
    let mut strict = r.db.pool.begin().await.expect("a transaction");
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *strict)
        .await
        .expect("isolation level");
    let refused = sqlx::query_scalar::<_, String>(
        "SELECT out_outcome FROM platform_requirement_remove('lease-withdrawal-v2', 'ana')",
    )
    .fetch_one(&mut *strict)
    .await
    .expect_err("refused outside READ COMMITTED")
    .to_string();
    assert!(
        refused.contains("lease-withdrawal-v2 is removed only under READ COMMITTED"),
        "{refused}"
    );
    drop(strict);
    withdrawal("NOT_ACKNOWLEDGED").await;
    let refused = remove().await.expect_err("the floor holds");
    assert!(
        refused.contains(
            "lease-withdrawal-v2 cannot be removed while a journal holds step.lease_withdrawn with the cause NOT_ACKNOWLEDGED"
        ),
        "{refused}"
    );
    assert!(recorded_v2(&r).await.is_some(), "the requirement stays");
}

// --- delivery acknowledgement, the registry's part (Task 006-046, decision D006-104) --------------------------------------

/// Records `capability` for `kinds` by hand SQL, as the owner; the outcome or the error's text.
async fn record_by_hand(r: &Registry, capability: &str, kinds: &[&str]) -> Result<String, String> {
    sqlx::query_scalar::<_, String>(
        "SELECT out_outcome FROM platform_requirement_activate($1, $2::text[], 'ops')",
    )
    .bind(capability)
    .bind(kinds)
    .fetch_one(&r.db.pool)
    .await
    .map_err(|error| error.to_string())
}

async fn remove_by_hand(r: &Registry, capability: &str) -> Result<String, String> {
    sqlx::query_scalar::<_, String>(
        "SELECT out_outcome FROM platform_requirement_remove($1, 'ops')",
    )
    .bind(capability)
    .fetch_one(&r.db.pool)
    .await
    .map_err(|error| error.to_string())
}

/// What every withdrawal of a marked lease reads, as the runtime role would read it.
async fn requirements_read(r: &Registry) -> (bool, bool, Option<time::OffsetDateTime>) {
    sqlx::query_as(
        "SELECT out_lease_withdrawal_v2, out_delivery_acknowledgement, out_required_since \
           FROM delivery_acknowledgement_required()",
    )
    .fetch_one(&r.process_pool)
    .await
    .expect("the requirements read")
}

const BOTH: [&str; 2] = ["CONTROL_PLANE", "EXECUTION_ENGINE"];

/// Task 006-046: `delivery_acknowledgement_required()` answers `lease-withdrawal-v2` only when it is recorded for both kinds,
/// `delivery-acknowledgement-v1` when it is recorded for engines, and the latter's `required_since`.
#[tokio::test]
async fn the_requirements_read_answers_each_recording() {
    let r = registry().await;
    assert_eq!(requirements_read(&r).await, (false, false, None), "none");
    record_by_hand(&r, "lease-withdrawal-v1", &BOTH)
        .await
        .expect("v1");
    record_by_hand(&r, "lease-withdrawal-v2", &["EXECUTION_ENGINE"])
        .await
        .expect("v2 for the engines only");
    assert_eq!(
        requirements_read(&r).await,
        (false, false, None),
        "engines only"
    );
    remove_by_hand(&r, "lease-withdrawal-v2")
        .await
        .expect("removed");
    record_by_hand(&r, "lease-withdrawal-v2", &BOTH)
        .await
        .expect("v2");
    assert_eq!(
        requirements_read(&r).await,
        (true, false, None),
        "both kinds"
    );
    record_by_hand(&r, "delivery-acknowledgement-v1", &["EXECUTION_ENGINE"])
        .await
        .expect("recorded for the engines");
    let (floor, acknowledged, since) = requirements_read(&r).await;
    assert!(floor && acknowledged);
    let recorded: time::OffsetDateTime = sqlx::query_scalar(
        "SELECT required_since FROM platform_requirements WHERE capability = 'delivery-acknowledgement-v1'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("required_since");
    assert_eq!(since, Some(recorded));
}

/// Task 006-046: the trigger keeps the order of the two requirements, by hand SQL as by any command: no
/// `delivery-acknowledgement-v1` before `lease-withdrawal-v2` for both kinds, nor while a registered process, live or lapsed,
/// lacks `lease-withdrawal-v2`; no removal of `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded; only
/// under READ COMMITTED. Each accepted once its condition holds.
#[tokio::test]
async fn the_trigger_keeps_the_order_of_the_two_requirements() {
    let r = registry().await;
    let refused = record_by_hand(&r, "delivery-acknowledgement-v1", &["EXECUTION_ENGINE"])
        .await
        .expect_err("before lease-withdrawal-v2");
    assert!(
        refused.contains("only after lease-withdrawal-v2"),
        "{refused}"
    );
    record_by_hand(&r, "lease-withdrawal-v1", &BOTH)
        .await
        .expect("v1");
    // a process of the build of pull request #260 alone, registered before `lease-withdrawal-v2` and silent for a minute
    // since: not live, so the activation's census does not see it, but still registered
    let older = r
        .serving(
            InstanceKind::ExecutionEngine,
            &[Capability::LeaseWithdrawalV1],
        )
        .await;
    r.age(older, 60).await;
    record_by_hand(&r, "lease-withdrawal-v2", &BOTH)
        .await
        .expect("v2");
    let refused = record_by_hand(&r, "delivery-acknowledgement-v1", &["EXECUTION_ENGINE"])
        .await
        .expect_err("a registered process lacks lease-withdrawal-v2");
    assert!(refused.contains(&older.to_string()), "{refused}");
    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(older)
        .execute(&r.db.pool)
        .await
        .expect("its process confirmed stopped, its row removed");
    // outside READ COMMITTED
    let mut strict = r.db.pool.begin().await.expect("a transaction");
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *strict)
        .await
        .expect("isolation");
    let refused = sqlx::query_scalar::<_, String>(
        "SELECT out_outcome FROM platform_requirement_activate('delivery-acknowledgement-v1', ARRAY['EXECUTION_ENGINE'], 'ops')",
    )
    .fetch_one(&mut *strict)
    .await
    .expect_err("only under READ COMMITTED")
    .to_string();
    assert!(refused.contains("only under READ COMMITTED"), "{refused}");
    drop(strict);
    assert_eq!(
        record_by_hand(&r, "delivery-acknowledgement-v1", &["EXECUTION_ENGINE"]).await,
        Ok("RECORDED".to_owned())
    );
    let refused = remove_by_hand(&r, "lease-withdrawal-v2")
        .await
        .expect_err("while delivery-acknowledgement-v1 is recorded");
    assert!(
        refused.contains("remove delivery-acknowledgement-v1 first"),
        "{refused}"
    );
    assert_eq!(
        remove_by_hand(&r, "delivery-acknowledgement-v1").await,
        Ok("REMOVED".to_owned())
    );
    assert_eq!(
        remove_by_hand(&r, "lease-withdrawal-v2").await,
        Ok("REMOVED".to_owned())
    );
}

/// Task 006-046: the capability is known to the registry and to the code, but no process of this build lists it and the
/// activation command has no entry for it yet (Task 006-057 adds it).
#[test]
fn delivery_acknowledgement_is_known_and_not_activatable() {
    assert_eq!(
        Capability::parse("delivery-acknowledgement-v1"),
        Some(Capability::DeliveryAcknowledgementV1)
    );
    assert!(
        sagat_control_plane::capability_activation::activatable("delivery-acknowledgement-v1")
            .is_none()
    );
}
