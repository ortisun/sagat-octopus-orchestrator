//! The upgrade of the cron triggers from v0.2.0 to 0.3.0 with schedules on by default (`T018-060`, `T018-061`,
//! `SC-018-005`, OD-2026-11-13-8), with the genuine released legacy cron pass and not current code standing for it.
//!
//! The released process fires a tick, then stays up while the current migrations run. A current engine started with
//! the default configuration (no `SAGAT_SCHEDULES_ENABLED`) makes the trigger-owned schedules and fires the next tick
//! once, from whichever of its two passes gets there first, and leaves the tick the release fired alone. The released
//! pass and the current schedule pass then reach one tick in each order, and once with both
//! processes up at the same time: one run each time. The released process is
//! stopped and joined (the runbook's first step), the owner runs `schedule-trigger-handover --apply`, and the legacy pass
//! of the current code starts nothing while the schedule pass fires the tick. What this does not prove is in
//! `research.md` (D2260 to D2262, G2260).
use super::release_upgrade_inflight::control;
use crate::support::{
    self, RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, bounded,
    oidc::TestIdp,
    worker::{RunningEngine, engine_env},
};
use sagat_application::{
    entropy::SystemEntropy,
    execution::{DefinitionCache, Engine},
    schedule_triggers::cron_document,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_scheduler::{EventDispatchSettings, EventDispatcher};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::HashMap, os::fd::OwnedFd, path::PathBuf, process::Stdio, sync::Arc, time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::net::UnixStream;
use uuid::Uuid;

const RELEASE: &str = "bfda342073db71e84f30bdc79ccf3fbc97a7153c";
const SCENARIO: &str = "cron-trigger-upgrade";
const OLD_CASE: &str =
    "release_upgrade_phase::cron_trigger::fire_a_cron_tick_then_tick_on_request_across_the_upgrade";
const DEFAULT: &str = "cron-upgrade-default";
const OLD_FIRST: &str = "cron-upgrade-old-first";
const NEW_FIRST: &str = "cron-upgrade-new-first";
const RACE: &str = "cron-upgrade-race";
const HANDOVER: &str = "cron-upgrade-handover";

#[derive(Clone, Copy)]
struct Cron {
    trigger: Uuid,
    workflow: Uuid,
}

fn cron_of(witness: &Value, name: &str) -> Cron {
    let id = |field: &str| -> Uuid {
        witness["triggers"][name][field]
            .as_str()
            .expect("released identity")
            .parse()
            .expect("UUID")
    };
    Cron {
        trigger: id("trigger"),
        workflow: id("workflow"),
    }
}

fn instant(epoch: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(epoch).expect("instant")
}

/// Polls the database for what an engine's loops do on their own; the bound is the reconciliation's poll many times over.
async fn until<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A current engine with the default configuration of the schedules: the test settings of the support (fast loops, a
/// local identity provider) on the runtime role, and no `SAGAT_SCHEDULES_ENABLED` at all.
fn default_engine_env(db: &support::TestDb, idp: &TestIdp) -> HashMap<String, String> {
    let runtime = db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD);
    let env = engine_env(db, idp, &[("DATABASE_URL", runtime.as_str())]);
    assert!(
        !env.contains_key("SAGAT_SCHEDULES_ENABLED"),
        "the default configuration"
    );
    let lookup = |name: &str| env.get(name).cloned();
    let config = sagat_control_plane::config::Config::from_lookup(&lookup).expect("config");
    let engine = sagat_execution_engine::EngineConfig::from_lookup(&lookup, config.environment)
        .expect("engine config");
    assert!(
        engine.schedules_enabled,
        "0.3.0 runs the schedule pass unless told not to (OD-2026-11-13-8)"
    );
    env
}

async fn tick_released(stream: &mut UnixStream) -> u64 {
    control::send(stream, &json!({"tick": true})).await;
    let reply = bounded::within(
        "released legacy pass tick",
        Duration::from_secs(30),
        control::receive(stream),
    )
    .await;
    reply["runsStarted"].as_u64().expect("released round")
}

/// The trigger's next time, and whether it is backing off.
async fn trigger_state(pool: &PgPool, cron: Cron) -> (OffsetDateTime, bool) {
    sqlx::query_as("SELECT next_fire_at, retry_at IS NOT NULL FROM workflow_triggers WHERE id = $1")
        .bind(cron.trigger)
        .fetch_one(pool)
        .await
        .expect("the trigger")
}

async fn schedule_of(pool: &PgPool, cron: Cron) -> Uuid {
    sqlx::query_scalar("SELECT id FROM schedules WHERE trigger_id = $1")
        .bind(cron.trigger)
        .fetch_one(pool)
        .await
        .expect("the trigger-owned schedule")
}

/// Makes `tick` the next time of the trigger (what a legacy pass fires), of its schedule (what the schedule pass fires),
/// or both, in one transaction.
async fn due(pool: &PgPool, cron: Cron, tick: OffsetDateTime, trigger: bool, schedule: bool) {
    let mut tx = pool.begin().await.expect("tx");
    if trigger {
        sqlx::query(
            "UPDATE workflow_triggers SET next_fire_at = $2, retry_at = NULL WHERE id = $1",
        )
        .bind(cron.trigger)
        .bind(tick)
        .execute(&mut *tx)
        .await
        .expect("the trigger is due");
    }
    if schedule {
        let moved = sqlx::query(
            "UPDATE schedules SET next_action_at = $2, next_actual_at = $2, state_version = state_version + 1 \
             WHERE trigger_id = $1",
        )
        .bind(cron.trigger)
        .bind(tick)
        .execute(&mut *tx)
        .await
        .expect("the schedule is due")
        .rows_affected();
        assert_eq!(moved, 1);
    }
    tx.commit().await.expect("commit");
}

/// The decision of the trigger's schedule for the tick, once a schedule pass has made it: outcome and run.
async fn decision(pool: &PgPool, cron: Cron, tick: OffsetDateTime) -> (String, Option<Uuid>) {
    let schedule = schedule_of(pool, cron).await;
    until("the schedule pass decides the tick", || async {
        sqlx::query_as(
            "SELECT outcome, run_id FROM schedule_actions WHERE schedule_id = $1 AND nominal_at = $2 AND origin = 'SPEC'",
        )
        .bind(schedule)
        .bind(tick)
        .fetch_optional(pool)
        .await
        .expect("actions")
    })
    .await
}

/// Every run of the workflow, oldest first: id, started by, schedule provenance, input document. The provenance is read
/// through the row's JSON so that the same query reads the released schema, which has no such column.
async fn runs(pool: &PgPool, cron: Cron) -> Vec<(Uuid, String, Option<Uuid>, Vec<u8>)> {
    sqlx::query_as(
        "SELECT r.id, r.started_by, (to_jsonb(r) ->> 'schedule_id')::uuid, p.content FROM runs r \
         JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest \
         WHERE r.workflow_id = $1 ORDER BY r.created_at, r.id",
    )
    .bind(cron.workflow)
    .fetch_all(pool)
    .await
    .expect("runs")
}

async fn keys(pool: &PgPool, cron: Cron) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT idempotency_key FROM command_dedupe WHERE scope = $1 ORDER BY idempotency_key",
    )
    .bind(format!("trigger:{}", cron.trigger))
    .fetch_all(pool)
    .await
    .expect("keys")
}

fn key(tick: OffsetDateTime) -> String {
    format!("cron-{}", tick.unix_timestamp())
}

/// The one run of the tick: it carries feature 004's `cron.fired` document of that nominal time (the same JSON the
/// current `cron_document` makes for it), was started by the engine, and no other run of the workflow says that time.
/// Returns its id, its schedule provenance and whether its bytes are the current function's to the byte.
async fn one_run(pool: &PgPool, cron: Cron, tick: OffsetDateTime) -> (Uuid, Option<Uuid>, bool) {
    let event = format!("cron:{}:{}", cron.trigger, tick.unix_timestamp());
    let mut found = Vec::new();
    for (id, started_by, schedule, content) in runs(pool, cron).await {
        let input: Value = serde_json::from_slice(&content).expect("JSON input");
        if input["event"]["id"] == event.as_str() {
            assert_eq!(started_by, "system:engine");
            let fired = OffsetDateTime::parse(
                input["data"]["firedAt"].as_str().expect("firedAt"),
                &Rfc3339,
            )
            .expect("firedAt");
            let expected = cron_document(cron.trigger, tick, fired);
            let expected_value: Value = serde_json::from_slice(&expected).expect("document");
            assert_eq!(
                input, expected_value,
                "feature 004's cron.fired document of the tick"
            );
            found.push((id, schedule, content == expected));
        }
    }
    assert_eq!(
        found.len(),
        1,
        "exactly one run for the tick {tick}: {found:?}"
    );
    found[0]
}

#[tokio::test]
#[ignore = "requires verified actual released source; use the release-upgrade profile runner"]
async fn a_released_cron_trigger_fires_once_a_tick_through_the_migration_a_mixed_fleet_and_the_handover()
 {
    assert!(
        std::env::var_os("SAGAT_TEST_DATABASE_URL").is_none(),
        "owns its disposable server"
    );
    let manifest =
        PathBuf::from(std::env::var_os("SAGAT_V02_BUILD").expect("verified build manifest"));
    let build: Value = serde_json::from_slice(&std::fs::read(&manifest).expect("build manifest"))
        .expect("build JSON");
    assert_eq!(build["releaseCommit"], RELEASE);
    assert_eq!(build["scenario"], SCENARIO);
    assert_eq!(
        build["coordinatorPath"],
        "tests/integration/workflow-primitives/release_upgrade_cron_trigger.rs"
    );
    assert_eq!(
        build["coordinatorSha256"],
        control::sha(include_bytes!("release_upgrade_cron_trigger.rs"))
    );
    super::release_upgrade_inflight::verify_overlays(&build);
    let binary = PathBuf::from(build["binary"].as_str().expect("released binary"));
    assert_eq!(
        binary.canonicalize().expect("binary").parent(),
        manifest.canonicalize().expect("manifest").parent()
    );
    assert_eq!(
        super::release_upgrade::file_sha(&binary),
        build["binarySha256"]
    );
    assert!(
        build["productionFilesVerified"]
            .as_u64()
            .expect("verified source count")
            > 600
    );
    bounded::lifecycle("released cron trigger upgrade", Duration::from_secs(300), |progress| async move {
        let owner = support::fresh_db().await;
        let (parent, old) = std::os::unix::net::UnixStream::pair().expect("private phase socket");
        parent.set_nonblocking(true).expect("nonblocking parent");
        let mut stream = UnixStream::from_std(parent).expect("async parent");
        let mut command = tokio::process::Command::new(binary);
        command
            .arg(OLD_CASE)
            .args(["--exact", "--nocapture"])
            .env("SAGAT_TEST_DATABASE_URL", &owner.admin_url)
            .env("RUST_TEST_THREADS", "1")
            .stdin(Stdio::from(OwnedFd::from(old)))
            .kill_on_drop(true);
        // its database sessions carry the released name, as in every released profile (D022-141)
        super::release_upgrade::label_released(&mut command);
        let mut old_process = command.spawn().expect("actual released process");

        progress.enter("1. the released legacy pass fires the first tick");
        let witness = bounded::within("released first tick", Duration::from_secs(120), control::receive(&mut stream)).await;
        assert_eq!(witness["releaseCommit"], RELEASE);
        let before = witness["maxMigration"].as_i64().expect("released schema");
        assert_eq!(before, 20261220000000);
        let db = support::attach_existing_db(&owner.admin_url, witness["database"].as_str().expect("database")).await;
        let pool = db.pool.clone();
        let today = instant(witness["today"].as_i64().expect("today"));
        let first = instant(witness["first"].as_i64().expect("first tick"));
        assert_eq!(today - first, time::Duration::days(1));
        let (default, old_first, new_first, race, handover) = (
            cron_of(&witness, DEFAULT),
            cron_of(&witness, OLD_FIRST),
            cron_of(&witness, NEW_FIRST),
            cron_of(&witness, RACE),
            cron_of(&witness, HANDOVER),
        );
        let released_runs = runs(&pool, default).await;
        assert_eq!(released_runs.len(), 1, "the release started one run");
        let released_run = released_runs[0].0;
        assert_eq!(keys(&pool, default).await, [key(first)], "scope trigger:{{id}}, key cron-{{epoch}}");
        let (released, _, released_bytes_equal) = one_run(&pool, default, first).await;
        assert_eq!(released, released_run);
        let planned = trigger_state(&pool, default).await;
        assert!(planned.0 > today && !planned.1, "the release moved the trigger past now");

        progress.enter("2. the current migrations, with the released process still up");
        sagat_infra_postgres::migrate(&pool).await.expect("current migrations");
        let after: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&pool).await.expect("current schema boundary");
        assert_eq!(after, 20270217000100, "the drop of the flow-control activation function (feature 022) is the last migration; the automatic handover's (ADR-0091) is before it");
        let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover").fetch_one(&pool).await.expect("handover state");
        assert_eq!(state, "LEGACY");
        let schedules: i64 = sqlx::query_scalar("SELECT count(*) FROM schedules").fetch_one(&pool).await.expect("schedules");
        assert_eq!(schedules, 0, "a migration makes no schedule: the reconciliation does");

        progress.enter("3. a current engine with the default configuration makes the schedules");
        let idp = TestIdp::start().await;
        let env = default_engine_env(&db, &idp);
        let mut engine = RunningEngine::start(env.clone()).await;
        let capabilities: Vec<String> = sqlx::query_scalar("SELECT capabilities FROM serving_instances WHERE kind = 'EXECUTION_ENGINE'").fetch_one(&pool).await.expect("registered");
        // the registry stores the list sorted by name; feature 006 (ADR-0098, decision D006-004): every engine of this build
        // lists `lease-withdrawal-v1` and, since the follow-up of pull request #260, `lease-withdrawal-v2`, whatever it runs
        assert_eq!(
            capabilities,
            ["lease-withdrawal-v1", "lease-withdrawal-v2", "run-input-validation-v1", "schedules-v1"],
            "the default engine runs the schedule pass, in enforce (design 043) validates run input, and replays the withdrawal of a lease (feature 006)"
        );
        until("the reconciliation converges every trigger", || async {
            let made: i64 = sqlx::query_scalar("SELECT count(*) FROM schedules WHERE origin = 'TRIGGER'").fetch_one(&pool).await.expect("schedules");
            let unconverged: i64 = sqlx::query_scalar("SELECT count(*) FROM schedule_trigger_drift").fetch_one(&pool).await.expect("drift");
            (made == 5 && unconverged == 0).then_some(())
        }).await;
        for cron in [default, old_first, new_first, race, handover] {
            let (name, state, next): (String, String, Option<OffsetDateTime>) = sqlx::query_as("SELECT name, state, next_action_at FROM schedules WHERE trigger_id = $1")
                .bind(cron.trigger).fetch_one(&pool).await.expect("schedule");
            assert_eq!(name, format!("trigger-{}", cron.trigger.simple()));
            assert_eq!(state, "ACTIVE");
            assert_eq!(next, Some(trigger_state(&pool, cron).await.0), "while LEGACY, the trigger's planned time");
        }
        // The next tick of the default workflow comes due for both passes of the engine at once.
        due(&pool, default, today, true, true).await;
        let (outcome, decided) = decision(&pool, default, today).await;
        let (run, provenance, default_bytes_equal) = one_run(&pool, default, today).await;
        assert_eq!(decided, Some(run), "the decision stands for the run");
        match outcome.as_str() {
            "STARTED" => assert_eq!(provenance, Some(schedule_of(&pool, default).await)),
            "ALREADY_ACTED" => assert_eq!(provenance, None, "the legacy pass of the engine started it"),
            other => panic!("decision {other}"),
        }
        until("the legacy pass of the engine moves the trigger on", || async {
            let (next, backing_off) = trigger_state(&pool, default).await;
            (next > today && !backing_off).then_some(())
        }).await;
        // The tick the release fired is not fired again: the same run, its key, and no decision for it.
        let (still, ..) = one_run(&pool, default, first).await;
        assert_eq!(still, released_run);
        assert_eq!(runs(&pool, default).await.len(), 2);
        assert_eq!(keys(&pool, default).await, [key(first), key(today)]);
        let decided_first: i64 = sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1 AND nominal_at <= $2")
            .bind(schedule_of(&pool, default).await).bind(first).fetch_one(&pool).await.expect("actions");
        assert_eq!(decided_first, 0, "no schedule decision reaches back to the released tick");
        let default_outcome = outcome;
        engine.finish().await.expect("a clean stop");

        progress.enter("4a. mixed fleet: the released legacy pass first, the current schedule pass second");
        due(&pool, old_first, today, true, false).await;
        assert_eq!(tick_released(&mut stream).await, 1, "the released pass, on the migrated schema, fires the tick");
        let (next, backing_off) = trigger_state(&pool, old_first).await;
        assert!(next > today && !backing_off, "the released pass moved the trigger on");
        let (released_tick, released_provenance, _) = one_run(&pool, old_first, today).await;
        assert_eq!(released_provenance, None);
        due(&pool, old_first, today, false, true).await;
        // 4b's first half: the current schedule pass reaches a tick no legacy pass has reached
        due(&pool, new_first, today, false, true).await;
        let mut engine = RunningEngine::start(env.clone()).await;
        let (outcome, decided) = decision(&pool, old_first, today).await;
        assert_eq!((outcome.as_str(), decided), ("ALREADY_ACTED", Some(released_tick)), "the current schedule pass finds the released run");
        let (outcome, decided) = decision(&pool, new_first, today).await;
        assert_eq!(outcome, "STARTED");
        let (current_tick, current_provenance, new_first_bytes_equal) = one_run(&pool, new_first, today).await;
        assert_eq!(decided, Some(current_tick));
        assert_eq!(current_provenance, Some(schedule_of(&pool, new_first).await));

        progress.enter("4c. mixed fleet: both processes up, one tick due for the three passes at once");
        // No barrier orders them: the released pass is asked to tick while the engine's two passes poll, so this is one
        // interleaving of the race, whichever it was, and not all of them (4a and 4b fix the two orders).
        due(&pool, race, today, true, true).await;
        let released_started = tick_released(&mut stream).await;
        assert!(released_started <= 1);
        let (race_outcome, decided) = decision(&pool, race, today).await;
        let (race_run, race_provenance, _) = one_run(&pool, race, today).await;
        assert_eq!(decided, Some(race_run));
        match race_outcome.as_str() {
            "STARTED" => assert_eq!(race_provenance, Some(schedule_of(&pool, race).await)),
            "ALREADY_ACTED" => assert_eq!(race_provenance, None, "a legacy pass started it"),
            other => panic!("decision {other}"),
        }
        let (next, backing_off) = trigger_state(&pool, race).await;
        assert!(next > today && !backing_off, "a legacy pass moved the trigger on");
        assert_eq!(keys(&pool, race).await, [key(today)]);
        assert_eq!(runs(&pool, race).await.len(), 1);
        engine.finish().await.expect("a clean stop");
        assert_eq!(keys(&pool, old_first).await, [key(today)]);
        assert_eq!(runs(&pool, old_first).await.len(), 1);

        progress.enter("4b. mixed fleet: the current schedule pass first, the released legacy pass second");
        let (planned, _) = trigger_state(&pool, new_first).await;
        assert!(planned > today, "the trigger was never due before");
        due(&pool, new_first, today, true, false).await;
        assert_eq!(tick_released(&mut stream).await, 0, "the released pass finds the run under the shared key");
        let (next, backing_off) = trigger_state(&pool, new_first).await;
        assert!(next > today && !backing_off, "the released pass took the run as its own and moved on, it did not fail");
        let (again, ..) = one_run(&pool, new_first, today).await;
        assert_eq!(again, current_tick);
        assert_eq!(keys(&pool, new_first).await, [key(today)]);
        assert_eq!(runs(&pool, new_first).await.len(), 1);

        progress.enter("5. the released process stops, then the automatic job hands over (ADR-0091)");
        control::send(&mut stream, &json!({"stop": true})).await;
        assert_eq!(bounded::within("released stop", Duration::from_secs(30), control::receive(&mut stream)).await["stopped"], true);
        let exit = bounded::within("join the released process", Duration::from_secs(30), old_process.wait()).await.expect("join");
        assert!(exit.success());
        let mut engine = RunningEngine::start(env.clone()).await;
        let unconverged: i64 = sqlx::query_scalar("SELECT count(*) FROM schedule_trigger_drift").fetch_one(&pool).await.expect("drift");
        assert_eq!(unconverged, 0, "convergence is 0");
        // the reconciliation made the schedules of the released triggers: its marks hold the job a horizon after them
        let marked: i64 = sqlx::query_scalar("SELECT count(*) FROM schedules WHERE reconciled_at IS NOT NULL").fetch_one(&pool).await.expect("marks");
        assert!(marked > 0, "the upgrade's backlog leaves the reconciliation's marks");
        // the scenario runs no control-plane process: one is registered, by the runtime role, as a control plane of this release
        // does, with what one in the default `enforce` mode lists (design 043, feature 006) and no flow-control gate
        // (feature 022: its switches are off by default)
        use sagat_application::instances::InstanceStore as _;
        let token = sagat_application::instances::InstanceToken::from_bytes([5; 32]);
        let control_plane = Uuid::new_v4();
        sagat_infra_postgres::PgInstanceStore::new(db.runtime_pool().await)
            .register(&sagat_application::instances::Registration {
                instance: control_plane,
                token: &token,
                kind: sagat_application::instances::InstanceKind::ControlPlane,
                scheduler_owner: None,
                binary_version: "0.3.0-scenario",
                capabilities: &sagat_control_plane::registry::capabilities(sagat_application::run_input::InputValidationMode::Enforce, &[]),
            })
            .await
            .expect("a control plane registers");
        let database = sagat_control_plane::config::DatabaseConfig {
            url: db.url.clone(),
            max_connections: 2,
            statement_timeout: Duration::from_secs(5),
        };
        let args = |dry_run, apply| sagat_control_plane::cli::HandoverArgs {
            operator: Some("release-upgrade-operator".to_owned()),
            expect_live: Some(1),
            dry_run,
            apply,
            ..sagat_control_plane::cli::HandoverArgs::default()
        };
        let said = sagat_control_plane::handover::run_handover_command(&database, &args(true, false)).await.expect("dry run");
        assert!(said.starts_with("dry run: nothing stops the handover"), "{said}");
        // the job, not --apply: as sagat_handover, its waits instant and the database's times moved back as the owner
        let run = Uuid::new_v4();
        // the control plane's row heartbeats at every wait of the job, as the process would (correctness review nit)
        let pace = auto::Backdate::new(pool.clone(), run, control_plane, vec![5; 32]);
        let job_database = sagat_control_plane::config::DatabaseConfig {
            url: db.url_without_password(support::HANDOVER_ROLE),
            max_connections: 2,
            statement_timeout: Duration::from_secs(5),
        };
        let request = sagat_control_plane::handover_auto::AutoRequest {
            ask: sagat_infra_postgres::AutoAsk {
                run,
                stable: 3,
                horizon: Duration::from_secs(600),
                min_live: 1,
                wait: Duration::from_secs(7_200),
            },
            retire_role: false,
            dry_run: false,
            operator: Some("release-upgrade".to_owned()),
            activation: sagat_infra_postgres::OperatorBounds::default(),
        };
        let done = bounded::within(
            "the automatic handover",
            Duration::from_secs(120),
            sagat_control_plane::handover_auto::auto_handover(
                &job_database,
                Some(support::HANDOVER_ROLE_PASSWORD),
                &request,
                &pace,
                &sagat_control_plane::handover_auto::Interrupt::default(),
            ),
        )
        .await
        .expect("the job hands over");
        assert!(matches!(done, sagat_control_plane::handover_auto::AutoOutcome::HandedOver { .. }), "{done:?}");
        let reasons = pace.reasons();
        // the backlog's marks first, and only them, then the rounds count to the handover
        let blocked = reasons.iter().take_while(|reason| reason.as_deref() == Some("RECONCILIATION_RECENT")).count();
        assert!(blocked >= 1, "{reasons:?}");
        assert!(reasons[blocked..].iter().all(Option::is_none), "nothing but RECONCILIATION_RECENT, then counting: {reasons:?}");
        let (state, by, mode): (String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT state, changed_by, changed_mode FROM schedule_trigger_handover").fetch_one(&pool).await.expect("handover");
        let actor = format!("system:handover-job:{run}");
        assert_eq!((state.as_str(), by.as_deref(), mode.as_deref()), ("HANDED_OVER", Some(actor.as_str()), Some("AUTOMATIC")));
        let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'schedule.trigger_handover'").fetch_one(&pool).await.expect("audit");
        assert!(audited >= 1, "the tenant audit of the handover");
        engine.finish().await.expect("a clean stop");
        // The tick comes due for both passes: the legacy pass of the current code starts nothing and leaves the trigger alone
        due(&pool, handover, today, true, true).await;
        let legacy = EventDispatcher::new(
            sagat_infra_postgres::PgRunStore::new(db.runtime_pool().await, DefinitionCache::new(Arc::new(JsonIrCodec), 8)),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
            EventDispatchSettings::default(),
        )
        .with_cron(Arc::new(sagat_infra_ir::cron::CronCalendar));
        assert_eq!(legacy.tick().await.runs_started, 0, "the legacy pass stopped");
        assert_eq!(trigger_state(&pool, handover).await, (today, false), "nothing moved the trigger");
        assert!(runs(&pool, handover).await.is_empty());
        // and the schedule pass of the engine fires it, as the only pass that does
        let mut engine = RunningEngine::start(env).await;
        let (outcome, decided) = decision(&pool, handover, today).await;
        assert_eq!(outcome, "STARTED");
        let (after_handover, provenance, handover_bytes_equal) = one_run(&pool, handover, today).await;
        assert_eq!(decided, Some(after_handover));
        assert_eq!(provenance, Some(schedule_of(&pool, handover).await));
        assert_eq!(keys(&pool, handover).await, [key(today)]);
        engine.finish().await.expect("a clean stop");
        assert_eq!(trigger_state(&pool, handover).await, (today, false), "the engine's legacy pass never moved it");

        let result = json!({
            "scenario": SCENARIO, "releaseCommit": RELEASE, "schemaBefore": before, "schemaAfter": after,
            "releasedTickRuns": 1, "releasedTickRefired": false,
            "defaultEngineCapabilities": capabilities, "schedulesMadeByReconciliation": 5,
            "defaultEngineTickOutcome": default_outcome, "defaultEngineTickRuns": 1,
            "mixedReleasedFirst": "ALREADY_ACTED", "mixedCurrentFirstReleasedRunsStarted": 0,
            "mixedConcurrentRace": {"releasedRunsStarted": released_started, "scheduleDecision": race_outcome, "runs": 1},
            "handover": "HANDED_OVER", "handoverMode": "AUTOMATIC", "handoverReasons": reasons,
            "handoverTenantAuditRecords": audited, "legacyPassAfterHandoverRunsStarted": 0, "scheduleAfterHandover": "STARTED",
            "documentBytesEqualCurrent": {"released": released_bytes_equal, "defaultEngine": default_bytes_equal,
                "currentFirst": new_first_bytes_equal, "afterHandover": handover_bytes_equal},
            "releasedReaderAfterHandover": false,
        });
        std::fs::write(std::env::var_os("SAGAT_V02_RESULT").expect("selected phase result path"), serde_json::to_vec_pretty(&result).expect("result JSON")).expect("bounded result evidence");
    }).await;
}

/// The waits of the automatic job, made instant for the scenario: each records the reason of the round before it and moves the
/// run's times, and the reconciliation's marks once they were seen, back as the owner (ADR-0091's test plan: backdated, never slept).
mod auto {
    use std::sync::Mutex;

    use sagat_control_plane::handover_auto::{Interrupt, Pace, Paced};
    use sqlx::PgPool;
    use uuid::Uuid;

    pub(super) struct Backdate {
        owner: PgPool,
        run: Uuid,
        heartbeat: (Uuid, Vec<u8>),
        reasons: Mutex<Vec<Option<String>>>,
    }

    impl Backdate {
        pub(super) fn new(owner: PgPool, run: Uuid, instance: Uuid, token: Vec<u8>) -> Self {
            Self {
                owner,
                run,
                heartbeat: (instance, token),
                reasons: Mutex::new(Vec::new()),
            }
        }

        pub(super) fn reasons(&self) -> Vec<Option<String>> {
            self.reasons.lock().expect("reasons").clone()
        }
    }

    impl Pace for Backdate {
        fn wait<'a>(&'a self, _wait: std::time::Duration, interrupt: &'a Interrupt) -> Paced<'a> {
            Box::pin(async move {
                let reason: Option<String> = sqlx::query_scalar(
                    "SELECT last_reason FROM schedule_trigger_handover_attempts WHERE run_id = $1",
                )
                .bind(self.run)
                .fetch_optional(&self.owner)
                .await
                .expect("reason")
                .flatten();
                sqlx::query("SELECT * FROM instance_heartbeat($1, $2)")
                    .bind(self.heartbeat.0)
                    .bind(&self.heartbeat.1)
                    .execute(&self.owner)
                    .await
                    .expect("the control plane heartbeats");
                let seen = {
                    let mut reasons = self.reasons.lock().expect("reasons");
                    reasons.push(reason);
                    reasons.len()
                };
                if seen > 1 {
                    sqlx::query("UPDATE schedules SET reconciled_at = reconciled_at - interval '11 minutes' WHERE reconciled_at IS NOT NULL")
                        .execute(&self.owner)
                        .await
                        .expect("the marks are a horizon old");
                }
                sqlx::query(
                    "UPDATE schedule_trigger_handover_attempts \
                        SET first_round_at = least(first_round_at, clock_timestamp() - horizon - interval '1 minute'), \
                            last_stable_at = last_stable_at - interval '16 seconds' WHERE run_id = $1",
                )
                .bind(self.run)
                .execute(&self.owner)
                .await
                .expect("the run's rounds are a heartbeat apart");
                if seen > 40 {
                    interrupt.raise();
                }
            })
        }
    }
}
