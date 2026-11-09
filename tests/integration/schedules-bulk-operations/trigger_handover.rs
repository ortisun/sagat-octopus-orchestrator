//! The handover of the cron triggers (`T018-061`, `FR-018-017`, `FR-018-052`, `G2002`): `control-plane
//! schedule-trigger-handover` is an activation of the instance-capability registry that requires `schedules-v1` of every
//! live execution engine and, in the same transaction, moves the handover row, as an audited operator, with the owner
//! role only. Real PostgreSQL, the command's own function, engines that are registered rows.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::instances::{Capability, InstanceKind};
use sagat_control_plane::RunError;
use sagat_control_plane::cli::{self, Command, HandoverArgs};
use sagat_control_plane::config::DatabaseConfig;
use sagat_control_plane::handover::{HandoverOutcome, HandoverRequest, schedule_trigger_handover};

use crate::migration::undo_to;
use crate::support::instances::{Registry, registry};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

fn database(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    }
}

fn request(operator: Option<&str>, expect_live: Option<u32>, dry_run: bool) -> HandoverRequest {
    HandoverRequest {
        operator: operator.map(str::to_owned),
        expect_live,
        dry_run,
    }
}

async fn hand_over(r: &Registry, req: &HandoverRequest) -> Result<HandoverOutcome, RunError> {
    schedule_trigger_handover(&database(r.db.url.clone()), req).await
}

async fn state(r: &Registry) -> (String, Option<String>, Option<String>) {
    sqlx::query_as("SELECT state, changed_by, changed_db_role FROM schedule_trigger_handover")
        .fetch_one(&r.db.pool)
        .await
        .expect("handover row")
}

async fn requirements(r: &Registry) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM platform_requirements WHERE capability = 'schedules-v1'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("requirements")
}

fn declined(result: Result<HandoverOutcome, RunError>) -> String {
    match result {
        Err(RunError::Declined(message)) => message,
        other => panic!("expected a refusal in words, got {other:?}"),
    }
}

async fn engine(r: &Registry, with_the_capability: bool) -> uuid::Uuid {
    let capabilities: &[Capability] = if with_the_capability {
        &[Capability::SchedulesV1]
    } else {
        &[]
    };
    r.serving(InstanceKind::ExecutionEngine, capabilities).await
}

#[tokio::test]
async fn the_handover_refuses_while_an_engine_lacks_the_capability_and_names_it() {
    let r = registry().await;
    let a = engine(&r, true).await;
    let b = engine(&r, true).await;
    let lacking = engine(&r, false).await;
    let _ = (a, b);

    let message = declined(hand_over(&r, &request(Some("ana"), Some(3), false)).await);
    assert!(
        message.contains("1 live execution engine(s) do not list schedules-v1"),
        "{message}"
    );
    assert!(
        message.contains(&lacking.to_string()),
        "it names the engine: {message}"
    );
    assert!(
        message.contains("SAGAT_SCHEDULES_ENABLED"),
        "and says why an engine lacks it: {message}"
    );
    assert_eq!(state(&r).await.0, "LEGACY", "nothing was changed");
    assert_eq!(requirements(&r).await, 0);
    assert_eq!(r.history().await, 0);

    // with the engine stopped (silent for 30 s) the same command passes with the two that are left
    r.age(lacking, 45).await;
    let done = hand_over(&r, &request(Some("ana"), Some(2), false))
        .await
        .expect("handed over");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { live: 2, .. }),
        "{done:?}"
    );
}

#[tokio::test]
async fn the_handover_passes_with_every_engine_listing_the_capability_and_records_who_did_it() {
    let r = registry().await;
    for _ in 0..3 {
        engine(&r, true).await;
    }
    // a control plane lists nothing and is not asked
    r.serving(InstanceKind::ControlPlane, &[]).await;

    let done = hand_over(&r, &request(Some("ana.operator"), Some(3), false))
        .await
        .expect("handed over");
    let HandoverOutcome::HandedOver { live, epoch } = done else {
        panic!("{done:?}")
    };
    assert_eq!(live, 3);
    assert_eq!(epoch, r.epoch_now().await);

    let (state, by, role) = state(&r).await;
    assert_eq!(state, "HANDED_OVER");
    assert_eq!(
        by.as_deref(),
        Some("ana.operator"),
        "the operator the command line named"
    );
    let owner: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&r.db.pool)
        .await
        .expect("owner");
    assert_eq!(
        role.as_deref(),
        Some(owner.as_str()),
        "and the database role of the session"
    );
    let at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT changed_at FROM schedule_trigger_handover")
            .fetch_one(&r.db.pool)
            .await
            .expect("changed_at");
    assert!(at.is_some());

    // the requirement and its history are the registry's record of the same act, in the same transaction
    assert_eq!(requirements(&r).await, 1);
    let (action, actor, database_user): (String, String, String) = sqlx::query_as(
        "SELECT action, actor, database_user FROM platform_requirement_history WHERE capability = 'schedules-v1'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("history");
    assert_eq!(
        (action.as_str(), actor.as_str(), database_user.as_str()),
        ("ACTIVATED", "ana.operator", owner.as_str())
    );
}

#[tokio::test]
async fn an_expectation_that_does_not_match_refuses_and_none_live_is_never_a_vacuous_pass() {
    let r = registry().await;
    // nothing is live: without --expect-live the activation would verify nothing
    let none = declined(hand_over(&r, &request(Some("ana"), None, false)).await);
    assert!(
        none.contains("no execution engine is live") && none.contains("--expect-live 0"),
        "{none}"
    );
    assert_eq!(
        (state(&r).await.0.as_str(), requirements(&r).await),
        ("LEGACY", 0)
    );

    engine(&r, true).await;
    engine(&r, true).await;
    let fewer = declined(hand_over(&r, &request(Some("ana"), Some(3), false)).await);
    assert!(
        fewer.contains("2 execution engine(s) are live") && fewer.contains("--expect-live says 3"),
        "{fewer}"
    );
    let more = declined(hand_over(&r, &request(Some("ana"), Some(1), false)).await);
    assert!(
        more.contains("2 execution engine(s) are live") && more.contains("--expect-live says 1"),
        "{more}"
    );
    assert_eq!(state(&r).await.0, "LEGACY");
    assert_eq!(r.history().await, 0, "nothing was recorded by any refusal");

    // an operator that says zero, when none is live, means it
    let r2 = registry().await;
    let done = hand_over(&r2, &request(Some("ana"), Some(0), false))
        .await
        .expect("zero is allowed when said");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { live: 0, .. }),
        "{done:?}"
    );
}

#[tokio::test]
async fn a_dry_run_verifies_everything_and_changes_nothing() {
    let r = registry().await;
    engine(&r, true).await;
    let would = hand_over(&r, &request(None, Some(1), true))
        .await
        .expect("dry run");
    assert_eq!(would, HandoverOutcome::WouldHandOver { live: 1 });
    assert_eq!(state(&r).await, ("LEGACY".to_owned(), None, None));
    assert_eq!(
        (
            requirements(&r).await,
            r.history().await,
            r.epoch_now().await
        ),
        (0, 0, 0)
    );

    // and it refuses what the real run would refuse
    let lacking = engine(&r, false).await;
    let message = declined(hand_over(&r, &request(None, Some(2), true)).await);
    assert!(message.contains(&lacking.to_string()), "{message}");
    let message = declined(hand_over(&r, &request(None, Some(5), true)).await);
    assert!(message.contains("--expect-live says 5"), "{message}");
    assert_eq!(state(&r).await.0, "LEGACY");
}

#[tokio::test]
async fn a_second_run_says_it_is_already_handed_over_and_changes_nothing() {
    let r = registry().await;
    engine(&r, true).await;
    hand_over(&r, &request(Some("ana"), Some(1), false))
        .await
        .expect("first");
    let epoch = r.epoch_now().await;
    let history = r.history().await;

    let again = hand_over(&r, &request(Some("bob"), Some(1), false))
        .await
        .expect("second");
    let HandoverOutcome::AlreadyHandedOver { by, db_role } = again else {
        panic!("{again:?}")
    };
    assert_eq!(
        by.as_deref(),
        Some("ana"),
        "it says who did it, not who asked again"
    );
    assert!(db_role.is_some());
    assert_eq!(
        (r.epoch_now().await, r.history().await),
        (epoch, history),
        "nothing was written"
    );
    // and even an expectation that no longer holds does not matter: there is nothing left to do
    let late = hand_over(&r, &request(Some("bob"), Some(9), false))
        .await
        .expect("idempotent");
    assert!(matches!(late, HandoverOutcome::AlreadyHandedOver { .. }));
    // a dry run of an accomplished handover is the same answer
    assert!(matches!(
        hand_over(&r, &request(None, None, true))
            .await
            .expect("dry"),
        HandoverOutcome::AlreadyHandedOver { .. }
    ));
}

#[tokio::test]
async fn the_runtime_role_cannot_run_it_and_is_told_to_use_the_owner_role() {
    let r = registry().await;
    engine(&r, true).await;
    let runtime = database(r.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));

    let message =
        match schedule_trigger_handover(&runtime, &request(Some("ana"), Some(1), false)).await {
            Err(RunError::Declined(message)) => message,
            other => panic!("{other:?}"),
        };
    assert!(message.contains("schema-owning database role"), "{message}");
    assert_eq!(state(&r).await.0, "LEGACY");
    assert_eq!((requirements(&r).await, r.history().await), (0, 0));
    // a dry run says so too: that role could not do it
    let dry = schedule_trigger_handover(&runtime, &request(None, Some(1), true)).await;
    assert!(
        matches!(&dry, Err(RunError::Declined(m)) if m.contains("schema-owning database role")),
        "{dry:?}"
    );
}

#[tokio::test]
async fn only_the_owner_can_write_the_handover_row_and_nothing_can_move_it_back() {
    let r = registry().await;
    let runtime = r.db.runtime_pool().await;
    // the runtime role reads the row and writes nothing, calls nothing
    let read: i64 = sqlx::query_scalar("SELECT count(*) FROM schedule_trigger_handover")
        .fetch_one(&runtime)
        .await
        .expect("it reads");
    assert_eq!(read, 1);
    for statement in [
        "UPDATE schedule_trigger_handover SET state = 'HANDED_OVER', changed_by = 'x', changed_at = now(), changed_db_role = 'y'",
        "INSERT INTO schedule_trigger_handover (only_row, state) VALUES (false, 'LEGACY')",
        "DELETE FROM schedule_trigger_handover",
        "SELECT schedule_trigger_handover_complete('x')",
        "SELECT * FROM schedule_trigger_handover_census(false)",
        "SELECT platform_requirement_activate('schedules-v1', ARRAY['EXECUTION_ENGINE'], 'x')",
    ] {
        let denied = sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&runtime)
            .await
            .expect_err(statement);
        assert!(
            denied.to_string().contains("permission denied"),
            "{statement}: {denied}"
        );
    }
    let may: bool = sqlx::query_scalar(
        "SELECT has_function_privilege($1, 'schedule_trigger_handover_complete(text)', 'EXECUTE')",
    )
    .bind(RUNTIME_ROLE)
    .fetch_one(&r.db.pool)
    .await
    .expect("privilege");
    assert!(!may, "the function is not the runtime role's");

    // the owner cannot move the row by hand either: it changes only through the function
    let by_hand = sqlx::query(
        "UPDATE schedule_trigger_handover SET state = 'HANDED_OVER', changed_by = 'x', changed_at = now(), changed_db_role = 'y'",
    )
    .execute(&r.db.pool)
    .await
    .expect_err("a hand edit");
    assert!(
        by_hand
            .to_string()
            .contains("only through schedule_trigger_handover_complete"),
        "{by_hand}"
    );

    // and the function itself refuses unless the requirement is recorded
    let early = sqlx::query_scalar::<_, String>("SELECT schedule_trigger_handover_complete('ana')")
        .fetch_one(&r.db.pool)
        .await
        .expect_err("no requirement");
    assert!(early.to_string().contains("not recorded"), "{early}");
    assert_eq!(state(&r).await.0, "LEGACY");

    // once handed over, no statement moves it back, with the guard's flag set or not
    engine(&r, true).await;
    hand_over(&r, &request(Some("ana"), Some(1), false))
        .await
        .expect("handed over");
    let mut tx = r.db.pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('sagat.handover_change', 'handover', true)")
        .execute(&mut *tx)
        .await
        .expect("flag");
    let back = sqlx::query(
        "UPDATE schedule_trigger_handover SET state = 'LEGACY', changed_by = NULL, changed_at = NULL, changed_db_role = NULL",
    )
    .execute(&mut *tx)
    .await
    .expect_err("never back");
    assert!(
        back.to_string().contains("never changed or moved back"),
        "{back}"
    );
    let outcome: String = sqlx::query_scalar("SELECT schedule_trigger_handover_complete('bob')")
        .fetch_one(&r.db.pool)
        .await
        .expect("idempotent function");
    assert_eq!(outcome, "ALREADY_HANDED_OVER");
}

#[tokio::test]
async fn an_engine_that_starts_without_the_schedule_pass_after_the_handover_is_refused() {
    use sagat_application::instances::{InstanceStore as _, Registered};
    let r = registry().await;
    engine(&r, true).await;
    hand_over(&r, &request(Some("ana"), Some(1), false))
        .await
        .expect("handed over");

    let late = uuid::Uuid::new_v4();
    let got = r
        .process
        .register(&crate::support::instances::registration(
            late,
            InstanceKind::ExecutionEngine,
            &[],
        ))
        .await
        .expect("register");
    assert!(
        matches!(got, Registered::CapabilityMissing { ref missing, .. } if missing == &["schedules-v1".to_owned()]),
        "{got:?}"
    );
}

#[test]
fn the_command_line_is_parsed_with_its_flags_and_refuses_what_it_does_not_know() {
    let parse =
        |args: &[&str]| cli::parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
    assert_eq!(
        parse(&[
            "schedule-trigger-handover",
            "--operator",
            "ana",
            "--expect-live",
            "3",
            "--dry-run"
        ]),
        Ok(Command::ScheduleTriggerHandover(HandoverArgs {
            operator: Some("ana".to_owned()),
            expect_live: Some(3),
            dry_run: true,
            apply: false,
        }))
    );
    assert_eq!(
        parse(&["schedule-trigger-handover", "--operator", "ana", "--apply"]),
        Ok(Command::ScheduleTriggerHandover(HandoverArgs {
            operator: Some("ana".to_owned()),
            apply: true,
            ..HandoverArgs::default()
        })),
        "the irreversible run is asked for"
    );
    assert_eq!(
        parse(&["schedule-trigger-handover", "--operator=ana"]),
        Ok(Command::ScheduleTriggerHandover(HandoverArgs {
            operator: Some("ana".to_owned()),
            ..HandoverArgs::default()
        }))
    );
    for bad in [
        vec!["schedule-trigger-handover", "--expect-live", "many"],
        vec!["schedule-trigger-handover", "--expect-live"],
        vec!["schedule-trigger-handover", "--operator"],
        vec!["schedule-trigger-handover", "--force"],
        vec!["schedule-trigger-handover", "ana"],
        vec!["schedule-trigger-handover", "--dry-run=yes"],
        // an option is never a value: it must not be recorded as who made the handover
        vec![
            "schedule-trigger-handover",
            "--operator",
            "--dry-run",
            "--expect-live",
            "3",
        ],
        vec!["schedule-trigger-handover", "--operator=--apply"],
        vec![
            "schedule-trigger-handover",
            "--operator",
            "ana",
            "--expect-live",
            "-1",
        ],
        vec![
            "schedule-trigger-handover",
            "--operator",
            "ana",
            "--apply",
            "--dry-run",
        ],
        vec!["schedule-trigger-handover", "--apply=yes"],
    ] {
        assert!(parse(&bad).is_err(), "{bad:?}");
    }
    assert!(cli::USAGE.contains(
        "schedule-trigger-handover --operator NAME [--expect-live N] [--dry-run | --apply]"
    ));
    assert!(
        cli::USAGE.contains("--apply               do it. Without it the command only verifies"),
        "the usage text says what --apply is for"
    );
    assert!(
        cli::USAGE.contains(
            "owner role only for migrate,\n    provision-tenant and schedule-trigger-handover"
        ),
        "the usage text names the commands that need the owner role"
    );
}

#[tokio::test]
async fn the_migration_rolls_back_on_a_schema_that_was_not_handed_over_applies_again_and_refuses_after_the_handover()
 {
    let r = registry().await;
    let this = 20_270_114_000_000_i64;
    let previous: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE version < $1")
            .bind(this)
            .fetch_one(&r.db.pool)
            .await
            .expect("previous");
    // a handed-over schema keeps what it recorded
    engine(&r, true).await;
    hand_over(&r, &request(Some("ana"), Some(1), false))
        .await
        .expect("handed over");
    let refused = undo_to(&r.db, previous).await.expect_err("handed over");
    assert!(
        format!("{refused:?}").contains("handed over"),
        "{refused:?}"
    );
    assert_eq!(state(&r).await.0, "HANDED_OVER");

    // a clean schema goes back and forward
    let clean = registry().await;
    undo_to(&clean.db, previous).await.expect("down");
    let column: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns WHERE table_name = 'schedule_trigger_handover' AND column_name = 'changed_db_role'",
    )
    .fetch_one(&clean.db.pool)
    .await
    .expect("column");
    let function: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc WHERE proname LIKE 'schedule_trigger_handover%'",
    )
    .fetch_one(&clean.db.pool)
    .await
    .expect("functions");
    assert_eq!(
        (column, function),
        (0, 0),
        "the down removed the column, the guard and the function"
    );
    sagat_infra_postgres::migrate(&clean.db.pool)
        .await
        .expect("up again");
    let again: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc WHERE proname = 'schedule_trigger_handover_complete'",
    )
    .fetch_one(&clean.db.pool)
    .await
    .expect("function");
    assert_eq!(again, 1);
}

#[tokio::test]
async fn an_operator_name_that_could_make_the_record_read_as_somebody_else_is_refused_before_anything_is_done()
 {
    let r = registry().await;
    engine(&r, true).await;
    for name in [
        "ana\u{202E}evil",
        "ana\u{200B}",
        "ana\u{0007}",
        "ana\nroot",
        "\u{FEFF}ana",
        " ana",
        "-ana",
        "",
    ] {
        let refused = hand_over(&r, &request(Some(name), Some(1), false)).await;
        assert!(
            matches!(refused, Err(RunError::Config(_))),
            "{name:?} must be refused as a name: {refused:?}"
        );
    }
    assert_eq!(state(&r).await.0, "LEGACY", "nothing was done");
    // an ordinary name, with a space and non-ASCII letters, is fine
    let done = hand_over(&r, &request(Some("Ana Maria Sao Paulo"), Some(1), false)).await;
    assert!(
        matches!(done, Ok(HandoverOutcome::HandedOver { .. })),
        "{done:?}"
    );
}

#[test]
fn the_irreversible_run_is_made_only_when_apply_is_given() {
    use sagat_control_plane::handover::request_of;
    let args = |dry_run, apply| HandoverArgs {
        operator: Some("ana".to_owned()),
        expect_live: Some(2),
        dry_run,
        apply,
    };
    assert!(
        request_of(&args(false, false)).dry_run,
        "no --apply: it only verifies"
    );
    assert!(request_of(&args(true, false)).dry_run);
    assert!(
        !request_of(&args(false, true)).dry_run,
        "--apply: it hands over"
    );
    let made = request_of(&args(false, true));
    assert_eq!(
        (made.operator.as_deref(), made.expect_live),
        (Some("ana"), Some(2))
    );
}
