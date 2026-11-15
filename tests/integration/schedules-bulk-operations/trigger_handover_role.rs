//! The role of the automatic handover through its life (`T018-084`, `T018-085`, ADR-0091 decision 1, D2264, D2265, D2269, D2272,
//! D2274, D2275): what every `migrate` does to `sagat_handover`, its password, its retirement and a role wider than the design.
//!
//! A role is a role of the cluster, so these run on a PostgreSQL of their own (with `log_statement = all`, to read what the server
//! logged), never on the shared one the other tests log in to as `sagat_handover`.
#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::str::FromStr as _;
use std::time::Duration;

use sagat_control_plane::config::DatabaseConfig;
use sagat_control_plane::handover_auto::{
    AutoOutcome, AutoRequest, Interrupt, Sleep, auto_handover,
};
use sagat_infra_postgres::{LoginSet, set_handover_login};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

use crate::support::bounded;

/// A password a URL cannot carry unescaped, at least 16 characters.
const PASSWORD: &str = "Zx9@hand/over#role-2026";

fn database(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: url.to_owned(),
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    }
}

async fn as_handover(base: &str, password: &str) -> Result<PgPool, sqlx::Error> {
    let options = PgConnectOptions::from_str(base)
        .expect("url")
        .username("sagat_handover")
        .password(password);
    PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
}

async fn can_log_in(owner: &PgPool) -> bool {
    sqlx::query_scalar("SELECT rolcanlogin FROM pg_roles WHERE rolname = 'sagat_handover'")
        .fetch_one(owner)
        .await
        .expect("role")
}

async fn granted(owner: &PgPool) -> bool {
    sqlx::query_scalar("SELECT has_function_privilege('sagat_handover', 'schedule_trigger_handover_auto(uuid)', 'EXECUTE')")
        .fetch_one(owner)
        .await
        .expect("privilege")
}

async fn sql(owner: &PgPool, statement: &str) {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
        .execute(owner)
        .await
        .unwrap_or_else(|error| panic!("{statement}: {error}"));
}

fn request(run: Uuid, retire_role: bool) -> AutoRequest {
    AutoRequest {
        run,
        stable: 3,
        horizon: Duration::from_secs(600),
        min_live: 1,
        wait: Duration::from_secs(7_200),
        retire_role,
        dry_run: false,
        operator: None,
    }
}

#[tokio::test]
async fn the_role_is_ensured_granted_logged_in_refused_when_wider_and_retired_for_good() {
    let container = bounded::within(
        "start a PostgreSQL of its own",
        bounded::START_LIMIT,
        Postgres::default()
            .with_tag("16-alpine")
            .with_cmd(["-c", "log_statement=all"])
            .start(),
    )
    .await
    .expect("start PostgreSQL (is Docker running?)");
    let port = bounded::mapped_port(&container, 5432).await;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let base = format!("postgres://127.0.0.1:{port}/postgres");
    let owner = loop {
        if let Ok(pool) = PgPoolOptions::new().max_connections(4).connect(&url).await
            && sqlx::query("SELECT 1").execute(&pool).await.is_ok()
        {
            break pool;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    // a database migrated from 0.2 has no role: migrate makes it NOLOGIN, with every grant
    sagat_control_plane::run_migrations(&database(&url))
        .await
        .expect("migrate");
    assert!(!can_log_in(&owner).await, "created without a login");
    assert!(granted(&owner).await, "and granted");

    // created by hand after the migration that introduced it: the next migrate grants it
    sql(&owner, "DROP OWNED BY sagat_handover").await;
    sql(&owner, "DROP ROLE sagat_handover").await;
    sql(&owner, "CREATE ROLE sagat_handover NOLOGIN").await;
    assert!(!granted(&owner).await);
    sagat_control_plane::run_migrations(&database(&url))
        .await
        .expect("migrate");
    assert!(granted(&owner).await, "granted again by the next migrate");

    // a role wider than the design is given nothing, and migrate goes on
    for (widen, narrow) in [
        (
            "ALTER ROLE sagat_handover BYPASSRLS",
            "ALTER ROLE sagat_handover NOBYPASSRLS",
        ),
        (
            "ALTER ROLE sagat_handover CREATEDB",
            "ALTER ROLE sagat_handover NOCREATEDB",
        ),
        (
            "CREATE ROLE wider_group NOLOGIN; GRANT wider_group TO sagat_handover",
            "DROP ROLE wider_group",
        ),
    ] {
        for statement in widen.split("; ") {
            sql(&owner, statement).await;
        }
        sagat_control_plane::run_migrations(&database(&url))
            .await
            .expect("migrate goes on");
        assert!(
            !granted(&owner).await,
            "a wider role is refused its grants: {widen}"
        );
        sql(&owner, narrow).await;
        sagat_control_plane::run_migrations(&database(&url))
            .await
            .expect("migrate");
        assert!(
            granted(&owner).await,
            "and granted again once narrow: {narrow}"
        );
    }

    // PostgreSQL 16: a role another CREATEROLE role made can be altered only with ADMIN OPTION; migrate warns and goes on
    sql(&owner, "DROP OWNED BY sagat_handover").await;
    sql(&owner, "DROP ROLE sagat_handover").await;
    sql(
        &owner,
        "CREATE ROLE creator LOGIN CREATEROLE PASSWORD 'creator-password'",
    )
    .await;
    let creator = PgPoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "postgres://creator:creator-password@127.0.0.1:{port}/postgres"
        ))
        .await
        .expect("creator");
    sql(&creator, "CREATE ROLE sagat_handover NOLOGIN").await;
    creator.close().await;
    sql(
        &owner,
        "CREATE ROLE migrator LOGIN CREATEROLE PASSWORD 'migrator-password'",
    )
    .await;
    sql(
        &owner,
        "GRANT SELECT, UPDATE ON schedule_trigger_handover TO migrator",
    )
    .await;
    sql(
        &owner,
        "GRANT SELECT ON schedule_trigger_handover_retirement TO migrator",
    )
    .await;
    let migrator = PgPoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "postgres://migrator:migrator-password@127.0.0.1:{port}/postgres"
        ))
        .await
        .expect("migrator");
    let verifier = sagat_control_plane::handover_role::scram_verifier(PASSWORD, &[3; 16], 4096);
    assert_eq!(
        set_handover_login(&migrator, &verifier)
            .await
            .expect("no failure"),
        LoginSet::NotPermitted,
        "42501, said and gone on"
    );
    assert!(!can_log_in(&owner).await);
    migrator.close().await;
    sql(&owner, "DROP OWNED BY migrator").await;
    sql(&owner, "DROP ROLE migrator").await;
    sql(&owner, "DROP OWNED BY sagat_handover").await;
    sql(&owner, "DROP ROLE sagat_handover").await;
    sql(&owner, "DROP ROLE creator").await;

    // the password: a verifier, never the clear text, and a password with @, / and # connects apart from the URL
    sagat_control_plane::run_migrations_with(&database(&url), Some(PASSWORD))
        .await
        .expect("migrate");
    assert!(
        can_log_in(&owner).await,
        "SAGAT_HANDOVER_PASSWORD lets it log in"
    );
    let job = as_handover(&base, PASSWORD)
        .await
        .expect("the password connects");
    job.close().await;
    let logged = String::from_utf8_lossy(&container.stderr_to_vec().await.expect("logs"))
        .into_owned()
        + &String::from_utf8_lossy(&container.stdout_to_vec().await.expect("logs"));
    assert!(
        logged.contains("ALTER ROLE sagat_handover LOGIN PASSWORD 'SCRAM-SHA-256$4096:"),
        "log_statement shows the verifier"
    );
    assert!(!logged.contains(PASSWORD), "and never the clear password");

    // the fleet, and the handover made by the job's functions
    sql(&owner, "SELECT * FROM instance_register(gen_random_uuid(), decode(repeat('01', 32), 'hex'), 'EXECUTION_ENGINE', NULL, '0.3.0', ARRAY['schedules-v1'])").await;
    sql(&owner, "SELECT * FROM instance_register(gen_random_uuid(), decode(repeat('02', 32), 'hex'), 'CONTROL_PLANE', NULL, '0.3.0', '{}')").await;
    let job = as_handover(&base, PASSWORD).await.expect("connect");
    let run = Uuid::new_v4();
    for _ in 0..4 {
        sqlx::query("SELECT * FROM schedule_trigger_handover_round($1, 3, interval '10 minutes', 1, interval '2 hours', NULL)")
            .bind(run)
            .execute(&job)
            .await
            .expect("round");
        sqlx::query(
            "UPDATE schedule_trigger_handover_attempts SET first_round_at = clock_timestamp() - interval '11 minutes', \
                    last_stable_at = last_stable_at - interval '16 seconds' WHERE run_id = $1",
        )
        .bind(run)
        .execute(&owner)
        .await
        .expect("age");
    }
    sqlx::query("SELECT * FROM schedule_trigger_handover_round($1, 3, interval '10 minutes', 1, interval '2 hours', NULL)")
        .bind(run)
        .execute(&job)
        .await
        .expect("round");
    let (outcome,): (String,) =
        sqlx::query_as("SELECT out_outcome FROM schedule_trigger_handover_auto($1)")
            .bind(run)
            .fetch_one(&job)
            .await
            .expect("wrapper");
    assert_eq!(outcome, "HANDED_OVER");
    job.close().await;

    // after HANDED_OVER and without a retirement, migrate with the password sets nothing
    sql(&owner, "ALTER ROLE sagat_handover NOLOGIN").await;
    sagat_control_plane::run_migrations_with(&database(&url), Some(PASSWORD))
        .await
        .expect("migrate");
    assert!(
        !can_log_in(&owner).await,
        "handed over: the role is left as it is"
    );
    sql(
        &owner,
        &format!("ALTER ROLE sagat_handover LOGIN PASSWORD '{verifier}'"),
    )
    .await;

    // the job, with --retire-role: the audit, then the role cannot log in, and the retirement is recorded
    let job_url = format!("postgres://sagat_handover@127.0.0.1:{port}/postgres");
    let done = auto_handover(
        &database(&job_url),
        Some(PASSWORD),
        &request(Uuid::new_v4(), true),
        &Sleep,
        &Interrupt::default(),
    )
    .await
    .expect("exit 0");
    assert!(
        matches!(done, AutoOutcome::AlreadyHandedOver { .. }),
        "{done:?}"
    );
    assert!(!can_log_in(&owner).await, "retired");
    let retired_by: String =
        sqlx::query_scalar("SELECT retired_by FROM schedule_trigger_handover_retirement")
            .fetch_one(&owner)
            .await
            .expect("recorded");
    assert_eq!(retired_by, "sagat_handover");
    // a later job that cannot log in: exit 0 with --retire-role, exit 2 without
    let later = auto_handover(
        &database(&job_url),
        Some(PASSWORD),
        &request(Uuid::new_v4(), true),
        &Sleep,
        &Interrupt::default(),
    )
    .await;
    assert!(matches!(later, Ok(AutoOutcome::RoleRetired)), "{later:?}");
    let later = auto_handover(
        &database(&job_url),
        Some(PASSWORD),
        &request(Uuid::new_v4(), false),
        &Sleep,
        &Interrupt::default(),
    )
    .await;
    assert_eq!(
        later
            .as_ref()
            .map_or_else(sagat_control_plane::RunError::exit_code, |_| 0),
        2,
        "{later:?}"
    );

    // retirement survives a later migrate with the password still given
    sagat_control_plane::run_migrations_with(&database(&url), Some(PASSWORD))
        .await
        .expect("migrate");
    assert!(
        !can_log_in(&owner).await,
        "a later migrate does not log the role in again"
    );
    assert!(
        granted(&owner).await,
        "its grants stay, harmless without a login"
    );
}
