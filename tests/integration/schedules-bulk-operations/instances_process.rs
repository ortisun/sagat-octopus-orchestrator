//! The processes in the instance-capability registry (`T018-080`, `FR-018-052`, `SC-018-017`): a real control plane
//! (`serve`) and a real engine (`serve`) register before they serve or run a pass, beat while they run, answer
//! `BINARY_CAPABILITY_MISSING` or stop their passes when the platform requires what their binary lacks, refuse to
//! start when it does already, and leave when they stop. The tables are the ones of migration
//! `20270113000000_instance_registry`; the age of a heartbeat is written, never waited for.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::instances::{Capability, InstanceKind};
use sagat_control_plane::RunError;
use sagat_control_plane::config::{Config, DatabaseConfig, Environment, ServeConfig};
use sagat_domain::identity::Subject;
use sagat_telemetry::LogFormat;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::support::eventually;
use crate::support::instances::{Registry, nothing, registry};
use crate::support::oidc::TestIdp;
use crate::support::worker::{RunningEngine, engine_env};

const BEAT_MS: u64 = 40;
/// For the tests that age a row and record a requirement in the gap between two beats.
const SLOW_BEAT_MS: u64 = 250;

// --- the control plane ------------------------------------------------------------------------------------------------

struct ControlPlane {
    addr: std::net::SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), RunError>>>,
}

fn serve_config(heartbeat_ms: u64) -> ServeConfig {
    let lookup = |name: &str| {
        [
            ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
            ("SAGAT_OIDC_AUDIENCE", "sagat"),
            ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
            ("SAGAT_ADMIN_ADDR", "off"),
        ]
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| (*value).to_owned())
    };
    let mut serve = ServeConfig::from_lookup(&lookup).expect("a configuration");
    serve.registry.heartbeat = Duration::from_millis(heartbeat_ms);
    serve
}

impl ControlPlane {
    /// Starts `serve` on the database of `r`; a control plane that refuses returns what `serve` returned.
    async fn start(r: &Registry, heartbeat_ms: u64) -> Result<Self, RunError> {
        let config = Config {
            environment: Environment::Dev,
            log_format: LogFormat::Json,
            database: DatabaseConfig {
                url: r.db.url.clone(),
                max_connections: 4,
                statement_timeout: Duration::from_secs(3),
            },
        };
        let serve = serve_config(heartbeat_ms);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stop, stopped) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            sagat_control_plane::serve(&config, &serve, listener, async {
                let _ = stopped.await;
            })
            .await
        });
        let mut running = Self {
            addr,
            stop: Some(stop),
            handle: Some(handle),
        };
        for _ in 0..200 {
            if running.handle.as_ref().is_some_and(JoinHandle::is_finished) {
                return Err(running.finish().await.expect_err("it ended early"));
            }
            if running.get("/v1/health").await.0 == 200 {
                return Ok(running);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the control plane never became alive");
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        match reqwest::get(format!("http://{}{path}", self.addr)).await {
            Ok(response) => {
                let status = response.status().as_u16();
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let mut body: Value = response.json().await.unwrap_or(Value::Null);
                if let (Some(retry_after), Value::Object(map)) = (retry_after, &mut body) {
                    map.insert("_retryAfter".into(), Value::String(retry_after));
                }
                (status, body)
            }
            Err(_) => (0, Value::Null),
        }
    }

    async fn finish(&mut self) -> Result<(), RunError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let handle = self.handle.take().expect("already finished");
        tokio::time::timeout(Duration::from_secs(20), handle)
            .await
            .expect("the control plane stops within its grace period")
            .expect("the control plane task did not panic")
    }
}

impl Drop for ControlPlane {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn instance_of_kind(r: &Registry, kind: &str) -> Option<(uuid::Uuid, String, Vec<String>)> {
    sqlx::query_as::<_, (uuid::Uuid, String, Vec<String>)>(
        "SELECT instance_id, binary_version, capabilities FROM serving_instances WHERE kind = $1",
    )
    .bind(kind)
    .fetch_optional(&r.db.pool)
    .await
    .expect("rows")
}

#[tokio::test]
async fn a_control_plane_registers_before_it_serves_and_its_row_is_gone_when_it_stops() {
    let r = registry().await;
    let mut cp = ControlPlane::start(&r, BEAT_MS).await.expect("serves");
    let (instance, version, capabilities) = instance_of_kind(&r, "CONTROL_PLANE")
        .await
        .expect("it registered before it answered its first call");
    assert_eq!(version, sagat_control_plane::registry::BINARY_VERSION);
    assert!(capabilities.is_empty(), "{capabilities:?}");
    assert_eq!(
        cp.get("/v1/ready").await.0,
        200,
        "and it serves while the platform requires nothing of it"
    );

    // it beats: the row's last_seen moves without anyone touching it
    r.age(instance, 25).await;
    eventually("a heartbeat refreshes the row", || async {
        let fresh: bool = sqlx::query_scalar(
            "SELECT last_seen > clock_timestamp() - interval '5 seconds' FROM serving_instances WHERE instance_id = $1",
        )
        .bind(instance)
        .fetch_one(&r.db.pool)
        .await
        .expect("fresh");
        fresh.then_some(())
    })
    .await;
    cp.finish().await.expect("a clean stop");
    assert!(
        instance_of_kind(&r, "CONTROL_PLANE").await.is_none(),
        "a clean stop deletes the row"
    );
}

#[tokio::test]
async fn a_control_plane_whose_binary_lacks_a_required_capability_stays_up_and_serves_nothing_until_it_is_let()
 {
    let r = registry().await;
    r.require(Capability::AccessNarrowingV1, &[InstanceKind::ControlPlane])
        .await
        .expect("record the requirement");
    let mut cp = ControlPlane::start(&r, BEAT_MS)
        .await
        .expect("it stays up: alive, not ready");
    let (status, body) = cp.get("/v1/auth/context").await;
    assert_eq!(status, 503);
    assert_eq!(
        body["details"]["reason"], "BINARY_CAPABILITY_MISSING",
        "{body}"
    );
    let (ready, body) = cp.get("/v1/ready").await;
    assert_eq!(ready, 503);
    assert_eq!(body["details"]["reason"], "BINARY_CAPABILITY_MISSING");
    assert_eq!(
        cp.get("/v1/health").await.0,
        200,
        "alive: it is not restarted"
    );
    assert!(
        instance_of_kind(&r, "CONTROL_PLANE").await.is_none(),
        "a process that does not serve is not counted"
    );

    // the operator removes the requirement: the next try registers it and it serves, without a restart
    r.operator
        .remove(Capability::AccessNarrowingV1, &Subject::operator(), nothing)
        .await
        .expect("remove");
    eventually("the control plane serves once it is let", || async {
        (cp.get("/v1/auth/context").await.0 == 401).then_some(())
    })
    .await;
    assert!(instance_of_kind(&r, "CONTROL_PLANE").await.is_some());
    cp.finish().await.expect("a clean stop");
}

#[tokio::test]
async fn a_registry_that_cannot_be_asked_at_the_start_means_the_control_plane_serves_nothing() {
    let r = registry().await;
    drop_the_register_function(&r).await;
    let mut cp = ControlPlane::start(&r, BEAT_MS)
        .await
        .expect("alive, not ready: as a process started before its migration always was");
    let (status, body) = cp.get("/v1/auth/context").await;
    assert_eq!(status, 503);
    assert!(
        body["details"]["reason"].is_null(),
        "no capability is missing: the plain 503 of every other cause, {body}"
    );
    assert_eq!(cp.get("/v1/ready").await.0, 503);
    assert_eq!(cp.get("/v1/health").await.0, 200);
    assert!(instance_of_kind(&r, "CONTROL_PLANE").await.is_none());
    cp.finish().await.expect("a clean stop");
}

#[tokio::test]
async fn a_requirement_recorded_while_it_serves_makes_every_call_answer_503_until_it_is_removed() {
    let r = registry().await;
    let mut cp = ControlPlane::start(&r, SLOW_BEAT_MS).await.expect("serves");
    let (instance, _, _) = instance_of_kind(&r, "CONTROL_PLANE").await.expect("row");
    // an unauthenticated call is answered by the API itself: 401 while it serves
    assert_eq!(cp.get("/v1/auth/context").await.0, 401);

    // while it beats it is live, and a requirement it lacks cannot be recorded
    assert!(
        r.require(Capability::AccessNarrowingV1, &[InstanceKind::ControlPlane])
            .await
            .is_err()
    );
    // it fell silent (a partition), the operator records the requirement; it learns at its next beat
    record_while_silent(
        &r,
        instance,
        Capability::AccessNarrowingV1,
        &[InstanceKind::ControlPlane],
    )
    .await;
    let refused = eventually("the control plane stops serving", || async {
        let (status, body) = cp.get("/v1/auth/context").await;
        (status == 503).then_some(body)
    })
    .await;
    assert_eq!(refused["code"], "UNAVAILABLE", "{refused}");
    assert_eq!(refused["details"]["reason"], "BINARY_CAPABILITY_MISSING");
    assert_eq!(refused["retryable"], true);
    assert!(
        refused["remediation"]
            .as_str()
            .is_some_and(|text| text.contains("The service cannot serve this call now")),
        "{refused}"
    );
    assert_eq!(refused["_retryAfter"], "10");
    // readiness fails with the same reason; liveness does not (a restart would only be refused)
    let (ready, body) = cp.get("/v1/ready").await;
    assert_eq!(ready, 503);
    assert_eq!(body["details"]["reason"], "BINARY_CAPABILITY_MISSING");
    assert_eq!(cp.get("/v1/health").await.0, 200);

    // the operator removes the requirement: the next beat opens the gate again
    r.operator
        .remove(Capability::AccessNarrowingV1, &Subject::operator(), nothing)
        .await
        .expect("remove");
    eventually("the control plane serves again", || async {
        (cp.get("/v1/auth/context").await.0 == 401).then_some(())
    })
    .await;
    cp.finish().await.expect("a clean stop");
}

/// Records a requirement while the process is silent. The process beats while the test runs, so the row is aged and
/// the requirement recorded in the gap between two beats, and tried again when a beat came in between (the beat is
/// slow in these tests; the loop only covers the rare overlap).
async fn record_while_silent(
    r: &Registry,
    instance: uuid::Uuid,
    capability: Capability,
    kinds: &[InstanceKind],
) {
    for _ in 0..60 {
        r.age(instance, 60).await;
        if r.require(capability, kinds).await.is_ok() {
            return;
        }
    }
    panic!("the process beat every time the test aged it");
}

// --- the engine -------------------------------------------------------------------------------------------------------

async fn engine_env_for(r: &Registry, idp: &TestIdp) -> std::collections::HashMap<String, String> {
    engine_env(
        &r.db,
        idp,
        &[
            ("SAGAT_REPLICA_ID", "registry-engine"),
            ("SAGAT_SCHEDULER_INTERVAL_MS", "40"),
            // the shortest lease, so that the keeper rounds every 600 ms
            ("SAGAT_SHARD_LEASE_SECS", "3"),
        ],
    )
}

async fn start_engine(
    r: &Registry,
    idp: &TestIdp,
    heartbeat_ms: u64,
) -> Result<RunningEngine, RunError> {
    start_engine_with(r, idp, heartbeat_ms, &[]).await
}

/// An engine with settings of its own on top of the ones of these tests (`SAGAT_SCHEDULES_ENABLED`, for one).
async fn start_engine_with(
    r: &Registry,
    idp: &TestIdp,
    heartbeat_ms: u64,
    extra: &[(&str, &str)],
) -> Result<RunningEngine, RunError> {
    let mut env = engine_env_for(r, idp).await;
    for (name, value) in extra {
        env.insert((*name).to_owned(), (*value).to_owned());
    }
    let lookup = move |name: &str| env.get(name).cloned();
    let config = Config::from_lookup(&lookup).expect("config");
    let mut engine = sagat_execution_engine::EngineConfig::from_lookup(&lookup, config.environment)
        .expect("engine config");
    engine.registry.heartbeat = Duration::from_millis(heartbeat_ms);
    RunningEngine::try_start_with(config, engine).await
}

#[tokio::test]
async fn an_engine_registers_before_its_first_pass_with_the_capabilities_of_its_binary_and_leaves_when_it_stops()
 {
    let r = registry().await;
    let idp = TestIdp::start().await;
    let mut engine = start_engine_with(&r, &idp, BEAT_MS, &[("SAGAT_SCHEDULES_ENABLED", "true")])
        .await
        .expect("starts");
    let row = sqlx::query(
        "SELECT binary_version, capabilities, scheduler_owner FROM serving_instances WHERE kind = 'EXECUTION_ENGINE'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("it registered");
    use sqlx::Row;
    assert_eq!(
        row.get::<Vec<String>, _>("capabilities"),
        vec!["schedules-v1"],
        "the capabilities of the binary, narrowed by the passes its configuration runs"
    );
    assert_eq!(
        row.get::<String, _>("binary_version"),
        sagat_control_plane::registry::BINARY_VERSION
    );
    let owner: String = row.get("scheduler_owner");
    assert!(owner.starts_with("registry-engine-"), "{owner}");
    // the owner is the name the keeper holds its leases under
    eventually("the keeper announces the same owner", || async {
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM scheduler_members WHERE owner = $1)")
                .bind(&owner)
                .fetch_one(&r.db.pool)
                .await
                .expect("members");
        known.then_some(())
    })
    .await;
    assert_eq!(engine.status("/readyz").await, Some(200));

    engine.finish().await.expect("a clean stop");
    assert_eq!(r.rows().await, 0, "a clean stop deletes the row");
}

#[tokio::test]
async fn an_engine_whose_binary_lacks_a_required_capability_stays_up_runs_no_pass_and_serves_no_worker_until_it_is_let()
 {
    let r = registry().await;
    let idp = TestIdp::start().await;
    r.require(
        Capability::PayloadEncodingV1,
        &[InstanceKind::ExecutionEngine],
    )
    .await
    .expect("record the requirement");
    let mut engine = start_engine(&r, &idp, BEAT_MS)
        .await
        .expect("it stays up: alive, not ready");
    assert_eq!(engine.status("/livez").await, Some(200));
    assert_eq!(engine.status("/readyz").await, Some(503));
    let answer: Value =
        serde_json::from_str(&engine.body("/readyz").await).expect("an error envelope");
    assert_eq!(
        answer["details"]["reason"], "BINARY_CAPABILITY_MISSING",
        "{answer}"
    );
    assert_eq!(r.rows().await, 0, "it is not counted");
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM scheduler_members")
        .fetch_one(&r.db.pool)
        .await
        .expect("members");
    assert_eq!(
        members, 0,
        "its keeper never announced itself: no pass was started"
    );
    let shards: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scheduler_shards WHERE generation > 0")
            .fetch_one(&r.db.pool)
            .await
            .expect("shards");
    assert_eq!(
        shards, 0,
        "it never took a shard, not even for the moment before its keeper loop gave it back"
    );

    // the worker protocol obeys the registry too: a worker is answered UNAVAILABLE (before it is even authenticated),
    // so that another replica serves it
    let mut client = sagat_api_grpc::pb::worker_service_client::WorkerServiceClient::connect(
        engine.grpc_url.clone(),
    )
    .await
    .expect("connect");
    let refused = client
        .register_worker(crate::support::worker::Worker::register_message(
            &["act.a"],
            1,
        ))
        .await
        .expect_err("a replica that may not serve answers no worker");
    assert_eq!(refused.code(), tonic::Code::Unavailable, "{refused:?}");
    assert_eq!(
        refused
            .metadata()
            .get("x-sagat-error")
            .and_then(|value| value.to_str().ok()),
        Some("BINARY_CAPABILITY_MISSING")
    );

    // the operator removes the requirement: it registers, takes its shards and answers its workers
    r.operator
        .remove(Capability::PayloadEncodingV1, &Subject::operator(), nothing)
        .await
        .expect("remove");
    eventually("the engine is ready once it is let", || async {
        (engine.status("/readyz").await == Some(200)).then_some(())
    })
    .await;
    assert_eq!(r.rows().await, 1);
    let served = client
        .register_worker(crate::support::worker::Worker::register_message(
            &["act.a"],
            1,
        ))
        .await
        .expect_err("without a token the call meets its authentication, which it lacks");
    assert_eq!(served.code(), tonic::Code::Unauthenticated, "{served:?}");
    engine.finish().await.expect("a clean stop");
}

#[tokio::test]
async fn an_engine_stops_its_passes_and_fails_readiness_when_a_requirement_it_lacks_is_recorded() {
    let r = registry().await;
    let idp = TestIdp::start().await;
    let mut engine = start_engine(&r, &idp, SLOW_BEAT_MS).await.expect("starts");
    let owner: String = sqlx::query_scalar(
        "SELECT scheduler_owner FROM serving_instances WHERE kind = 'EXECUTION_ENGINE'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("owner");
    let instance: uuid::Uuid = sqlx::query_scalar(
        "SELECT instance_id FROM serving_instances WHERE kind = 'EXECUTION_ENGINE'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("instance");
    let holds_shards = || async {
        let held: i64 =
            sqlx::query_scalar("SELECT count(*) FROM scheduler_shards WHERE owner = $1 AND expires_at > clock_timestamp()")
                .bind(&owner)
                .fetch_one(&r.db.pool)
                .await
                .expect("shards");
        held
    };
    eventually("the engine holds shards", || async {
        (holds_shards().await > 0).then_some(())
    })
    .await;

    // silent for a while, then the operator records a requirement the binary lacks
    record_while_silent(
        &r,
        instance,
        Capability::PayloadEncodingV1,
        &[InstanceKind::ExecutionEngine],
    )
    .await;
    assert_eq!(r.epoch_now().await, 1, "the requirement is recorded");
    // Not ready is not yet "not ready for this reason": while the engine has been silent, a 503 can come from the age of its
    // last beat (the answer then names no capability), and CI read the body of that first 503 (`details.reason` null). The
    // test waits for the answer that names the capability it lacks, which is the one the heartbeat that finds the
    // requirement gives.
    let answer: Value = eventually(
        "the engine fails readiness for the capability it lacks",
        || async {
            if engine.status("/readyz").await != Some(503) {
                return None;
            }
            let body: Value = serde_json::from_str(&engine.body("/readyz").await).ok()?;
            (body["details"]["reason"] == "BINARY_CAPABILITY_MISSING").then_some(body)
        },
    )
    .await;
    assert_eq!(answer["code"], "UNAVAILABLE", "{answer}");
    assert_eq!(
        answer["details"]["reason"], "BINARY_CAPABILITY_MISSING",
        "the reason is in the answer"
    );
    assert_eq!(
        engine.status("/livez").await,
        Some(200),
        "alive: a restart would only be refused"
    );
    eventually("it gives its shards to the others", || async {
        (holds_shards().await == 0).then_some(())
    })
    .await;
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scheduler_members WHERE owner = $1")
            .bind(&owner)
            .fetch_one(&r.db.pool)
            .await
            .expect("members");
    assert_eq!(
        members, 0,
        "and leaves the replicas that count for the shard shares"
    );

    // the operator removes the requirement: it holds shards and is ready again
    r.operator
        .remove(Capability::PayloadEncodingV1, &Subject::operator(), nothing)
        .await
        .expect("remove");
    eventually("the engine is ready again", || async {
        (engine.status("/readyz").await == Some(200)).then_some(())
    })
    .await;
    eventually("and takes its shards again", || async {
        (holds_shards().await > 0).then_some(())
    })
    .await;
    engine.finish().await.expect("a clean stop");
}
/// Removes `instance_register` (whatever its signature is), so that the registry cannot be asked.
async fn drop_the_register_function(r: &Registry) {
    let drops: Vec<String> = sqlx::query_scalar(
        "SELECT format('DROP FUNCTION %s', p.oid::regprocedure) FROM pg_proc p WHERE p.proname = 'instance_register'",
    )
    .fetch_all(&r.db.pool)
    .await
    .expect("the functions");
    assert!(!drops.is_empty());
    for drop in drops {
        sqlx::query(sqlx::AssertSqlSafe(drop))
            .execute(&r.db.pool)
            .await
            .expect("the registry is not there");
    }
}

// --- the capability follows the passes the configuration turns on (review A) ---------------------------------------------

#[tokio::test]
async fn an_engine_lists_schedules_v1_only_when_it_runs_the_schedule_pass_and_an_activation_refuses_one_that_does_not()
 {
    let r = registry().await;
    let idp = TestIdp::start().await;

    // the default configuration runs no schedule pass: the engine does not list the capability, whatever its code can do
    let mut quiet = start_engine(&r, &idp, BEAT_MS).await.expect("starts");
    let (quiet_id, _, capabilities) = instance_of_kind(&r, "EXECUTION_ENGINE")
        .await
        .expect("registered");
    assert!(capabilities.is_empty(), "{capabilities:?}");
    let refused = r
        .require(Capability::SchedulesV1, &[InstanceKind::ExecutionEngine])
        .await
        .expect_err(
            "a live engine that runs no schedule pass would fire no cron after the handover",
        );
    assert_eq!(
        refused,
        sagat_application::instances::ChangeRefused::InstancesLackCapability {
            missing: 1,
            examples: vec![quiet_id]
        },
        "the activation names it"
    );
    quiet.finish().await.expect("a clean stop");

    // with the schedule pass on, it lists it and the activation goes through
    let mut running = start_engine_with(&r, &idp, BEAT_MS, &[("SAGAT_SCHEDULES_ENABLED", "true")])
        .await
        .expect("starts");
    let (_, _, capabilities) = instance_of_kind(&r, "EXECUTION_ENGINE")
        .await
        .expect("registered");
    assert_eq!(capabilities, vec!["schedules-v1"]);
    assert_eq!(
        r.require(Capability::SchedulesV1, &[InstanceKind::ExecutionEngine])
            .await
            .expect("activate"),
        sagat_application::instances::Activated::Recorded { epoch: 1 }
    );
    running.finish().await.expect("a clean stop");

    // restarted with the flag off after the handover: it is refused at registration and its whole gate is closed
    let mut restarted = start_engine(&r, &idp, BEAT_MS).await.expect("it stays up");
    assert_eq!(restarted.status("/livez").await, Some(200));
    assert_eq!(restarted.status("/readyz").await, Some(503));
    let answer: Value =
        serde_json::from_str(&restarted.body("/readyz").await).expect("an error envelope");
    assert_eq!(
        answer["details"]["reason"], "BINARY_CAPABILITY_MISSING",
        "{answer}"
    );
    assert!(instance_of_kind(&r, "EXECUTION_ENGINE").await.is_none());
    restarted.finish().await.expect("a clean stop");
}

// --- the probe distinguishes why the gate is closed (review H) ----------------------------------------------------------------

#[tokio::test]
async fn an_engine_that_cannot_reach_the_registry_is_not_ready_and_names_no_capability() {
    let r = registry().await;
    let idp = TestIdp::start().await;
    drop_the_register_function(&r).await;
    let mut engine = start_engine(&r, &idp, BEAT_MS).await.expect("it stays up");
    assert_eq!(engine.status("/livez").await, Some(200));
    assert_eq!(engine.status("/readyz").await, Some(503));
    let answer: Value =
        serde_json::from_str(&engine.body("/readyz").await).expect("an error envelope");
    assert_eq!(answer["code"], "UNAVAILABLE", "{answer}");
    assert!(
        answer["details"]["reason"].is_null(),
        "no capability is missing, so none is named: {answer}"
    );
    engine.finish().await.expect("a clean stop");
}

#[tokio::test]
async fn an_engine_that_finds_the_table_full_still_serves_and_says_it_is_not_counted() {
    let r = registry().await;
    let idp = TestIdp::start().await;
    r.seed(1000, InstanceKind::ControlPlane, 1).await;
    let mut engine = start_engine(&r, &idp, BEAT_MS).await.expect("starts");
    eventually(
        "the engine is ready though it could not be counted",
        || async { (engine.status("/readyz").await == Some(200)).then_some(()) },
    )
    .await;
    assert_eq!(r.rows().await, 1000, "it took no place");
    engine.finish().await.expect("a clean stop");
}

// --- a closed engine acts on nothing: not a schedule, not a connector (review D) -------------------------------------------------

#[tokio::test]
async fn a_closed_engine_fires_no_schedule_and_reads_no_connector_and_the_same_engine_does_both_once_it_is_let()
 {
    use crate::firing::{fixture, hourly};
    use sagat_application::instances::Activation;
    use sagat_infra_postgres::PgInstanceStore;

    let f = fixture().await;
    let idp = TestIdp::start().await;
    // a schedule that is due (the fixture's clock is in October), and a connector the engine would read as soon as it may
    let schedule = f
        .create("due", &hourly(), &serde_json::json!({}), None)
        .await;
    let connector = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_connectors (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) \
         VALUES ($1,$2,$3,'closed-engine','KAFKA','{\"servers\":[\"tls://broker.invalid:9092\"],\"topic\":\"closed\",\"startFrom\":\"LATEST\"}',10,'admin')",
    )
    .bind(connector)
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a connector");
    let operator = PgInstanceStore::new(f.w.db.pool.clone());
    let actor = Subject::operator();
    // the platform requires what this engine lacks
    operator
        .activate(
            Activation {
                capability: Capability::PayloadEncodingV1,
                kinds: &[InstanceKind::ExecutionEngine],
                actor: &actor,
            },
            nothing,
        )
        .await
        .expect("record the requirement");

    let key = sagat_infra_secrets::generate_key().expect("a key");
    let secrets = format!("k1:{key}");
    let env = engine_env(
        &f.w.db,
        &idp,
        &[
            ("SAGAT_REPLICA_ID", "closed-engine"),
            ("SAGAT_SCHEDULER_INTERVAL_MS", "40"),
            ("SAGAT_SHARD_LEASE_SECS", "3"),
            ("SAGAT_SCHEDULES_ENABLED", "true"),
            ("SAGAT_SCHEDULE_POLL_MS", "40"),
            ("SAGAT_CONNECTOR_REFRESH_SECS", "1"),
            ("SAGAT_SECRETS_KEYS", secrets.as_str()),
        ],
    );
    let lookup = move |name: &str| env.get(name).cloned();
    let config = Config::from_lookup(&lookup).expect("config");
    let mut engine_config =
        sagat_execution_engine::EngineConfig::from_lookup(&lookup, config.environment)
            .expect("engine config");
    engine_config.registry.heartbeat = Duration::from_millis(BEAT_MS);
    let mut engine = RunningEngine::try_start_with(config, engine_config)
        .await
        .expect("it stays up");
    assert_eq!(engine.status("/readyz").await, Some(503));

    // An absence is shown by waiting: two refreshes of the connectors and about sixty schedule polls pass. The same
    // engine acts at once when it is let (below), which is what shows that the wait could have seen it.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(
        f.runs(&schedule).await.is_empty(),
        "a closed engine fired a schedule"
    );
    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM event_connectors WHERE id = $1")
            .bind(connector)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the connector");
    assert_eq!(last_error, None, "a closed engine read a connector");

    operator
        .remove(Capability::PayloadEncodingV1, &actor, nothing)
        .await
        .expect("remove");
    eventually("the engine fires the schedule once it is let", || async {
        (!f.runs(&schedule).await.is_empty()).then_some(())
    })
    .await;
    eventually("and reads the connector", || async {
        let error: Option<String> =
            sqlx::query_scalar("SELECT last_error FROM event_connectors WHERE id = $1")
                .bind(connector)
                .fetch_one(&f.w.db.pool)
                .await
                .expect("the connector");
        error.map(|_| ())
    })
    .await;
    engine.finish().await.expect("a clean stop");
}
