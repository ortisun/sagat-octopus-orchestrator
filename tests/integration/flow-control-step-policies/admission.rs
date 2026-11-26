//! The one admission point and the fleet activation of the two capabilities (T022-093, T022-094; FR-022-053,
//! FR-022-059; SC-022-027): every run creator passes `PgRunStore::create`, which admits a gated definition only when every
//! capability it needs is switched on, activated for the fleet and served by the process (D022-057: the concurrency
//! gate, with the deployment's digest keys; never the rate gate in this build); publication and starts wait for the
//! operator's activation; the activation waits for every live process to run the gate; and a continuation successor
//! never admits a run past the gate.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{DefinitionService, IrError};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::flow_control::{
    FlowControlCapability, FlowControlRefusal, FlowControlSwitches,
};
use sagat_application::instances::{
    Activation, Capability, InstanceKind, InstanceStore, Registered, Removed,
};
use sagat_application::runs::{RunService, StartRun};
use sagat_domain::identity::Subject;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgInstanceStore, PgRunStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, ctx, world};
use crate::support::instances::{TEST_TOKEN, nothing, registration};

const BOTH_ON: FlowControlSwitches = FlowControlSwitches {
    concurrency_keys: true,
    rate_limits: true,
};
const KINDS: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];

fn gated(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "fetch",
        "rateLimit": {"key": "input.providerId", "limit": 10, "per": "second"},
        "nodes": [{"id": "fetch", "type": "activity", "activity": "provider.fetch"}]
    })
}

fn operator() -> Subject {
    Subject::parse("operator:flow-control").expect("subject")
}

/// The services a control plane would build with the switches on, over the world's store.
fn definitions(w: &World) -> DefinitionService<PgRunStore, SystemClock, JsonIrCodec> {
    DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec).with_flow_control(BOTH_ON)
}

fn runs(store: PgRunStore) -> RunService<PgRunStore, SystemClock, JsonEvaluator> {
    RunService::new(
        store,
        SystemClock,
        Engine::new(JsonEvaluator),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
}

async fn create(w: &World, doc: &Value) -> sagat_domain::definition::WorkflowId {
    let name = doc["metadata"]["name"].as_str().expect("name");
    definitions(w)
        .create_workflow(
            &w.admin,
            "payments",
            name,
            &serde_json::to_vec(doc).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created")
        .workflow
        .id()
}

async fn publish(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
) -> Result<sagat_application::definitions::Published, AppError> {
    definitions(w)
        .publish(
            &w.admin,
            workflow,
            1,
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
}

async fn start(
    service: &RunService<PgRunStore, SystemClock, JsonEvaluator>,
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
) -> Result<sagat_domain::execution::RunId, AppError> {
    let input = serde_json::to_vec(&json!({"providerId": "p-1"})).expect("json");
    service
        .start(
            &w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: &input,
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::now_v7()),
            },
            &ctx(),
        )
        .await
        .map(|started| started.info.run_id)
}

async fn runs_of(w: &World, workflow: sagat_domain::definition::WorkflowId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM runs WHERE workflow_id = $1")
        .bind(workflow.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

/// SC-022-027: with one live engine that does not run the gate, the operator's activation
/// (`control-plane capability-activate flow-control-rate-v1`, task T022-116) is refused, naming it; once every live
/// control plane and engine lists the capability it is recorded.
#[tokio::test]
async fn the_activation_is_refused_while_a_live_process_lacks_the_capability() {
    use sagat_control_plane::capability_activation::{
        ActivationArgs, ActivationOutcome, capability_activate,
    };
    let w = world().await;
    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(1::smallint, 'operator')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("the digest key version is activated");
    assert_eq!(outcome, "ACTIVATED");
    let database = sagat_control_plane::config::DatabaseConfig {
        url: w.db.url.clone(),
        max_connections: 2,
        statement_timeout: StdDuration::from_secs(5),
    };
    let command = ActivationArgs {
        capability: "flow-control-rate-v1".to_owned(),
        operator: Some("ops:ana".to_owned()),
        expect_live: Some(3),
        dry_run: false,
    };
    let registry = PgInstanceStore::new(w.db.pool.clone());
    let lacking = Uuid::new_v4();
    assert!(matches!(
        registry
            .register(&registration(lacking, InstanceKind::ExecutionEngine, &[]))
            .await
            .expect("an engine of this build registers"),
        Registered::Serving { .. }
    ));
    for kind in KINDS {
        registry
            .register(&registration(
                Uuid::new_v4(),
                kind,
                &[Capability::FlowControlRateV1],
            ))
            .await
            .expect("a process that runs the gate");
    }
    match capability_activate(&database, &command).await {
        Err(sagat_control_plane::RunError::Declined(message)) => {
            assert!(
                message.contains("1 live process(es) do not list flow-control-rate-v1")
                    && message.contains(&lacking.to_string()),
                "{message}"
            );
        }
        other => panic!("expected a refusal naming the engine, got {other:?}"),
    }
    registry
        .deregister(lacking, &TEST_TOKEN)
        .await
        .expect("the old engine leaves");
    assert!(matches!(
        capability_activate(
            &database,
            &ActivationArgs {
                expect_live: Some(2),
                ..command
            }
        )
        .await,
        Ok(ActivationOutcome::Activated { live: 2, .. })
    ));
}

/// FR-022-053, FR-022-059, D022-057, T022-072: before the activation, publication is refused
/// `FLOW_CONTROL_ACTIVATION_PENDING`; after it, the one admission point admits a rate-limited start exactly when the
/// process's switch is on and it holds the digest keys (`BINARY_CAPABILITY_MISSING` without the keys,
/// `CAPABILITY_DISABLED` with the switch off); the admitted run holds its slot from its grant; when the requirement is
/// removed, the start is refused `FLOW_CONTROL_ACTIVATION_PENDING`. Only the admitted run exists.
#[tokio::test]
async fn publication_and_starts_wait_for_the_activation_and_a_serving_process_admits_a_rate_limited_run()
 {
    let w = world().await;
    let workflow = create(&w, &gated("provider-sync")).await;
    match publish(&w, workflow).await {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(
                *error,
                IrError::FlowControlActivationPending {
                    capability: "rateLimits"
                }
            ),
            "{error:?}"
        ),
        other => panic!("expected FLOW_CONTROL_ACTIVATION_PENDING, got {other:?}"),
    }

    let registry = PgInstanceStore::new(w.db.pool.clone());
    for kind in KINDS {
        registry
            .register(&registration(
                Uuid::new_v4(),
                kind,
                &[Capability::FlowControlRateV1],
            ))
            .await
            .expect("processes that would run the gate");
    }
    let actor = operator();
    registry
        .activate(
            Activation {
                capability: Capability::FlowControlRateV1,
                kinds: &KINDS,
                actor: &actor,
            },
            nothing,
        )
        .await
        .expect("activated");
    publish(&w, workflow)
        .await
        .expect("published once activated");

    let on = runs(
        w.store
            .clone()
            .with_flow_control(BOTH_ON)
            .with_flow_control_keys(Arc::new(
                sagat_application::keyed_digest::KeyRing::parse(crate::support::flow_control::RING)
                    .expect("a ring"),
            )),
    );
    let without_keys = runs(w.store.clone().with_flow_control(BOTH_ON));
    assert_eq!(
        start(&without_keys, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::BinaryCapabilityMissing
        )),
        "a process without the digest keys serves no gate"
    );
    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(1::smallint, 'operator')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("the digest key version is activated");
    assert_eq!(outcome, "ACTIVATED");
    let run = start(&on, &w, workflow)
        .await
        .expect("admitted by the one admission point");
    let (granted, leased_from_grant): (i64, bool) = sqlx::query_as(
        "SELECT count(*), coalesce(bool_and(leased_at = granted_at), false) FROM rate_limit_grants \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("grant");
    assert_eq!(
        (granted, leased_from_grant),
        (1, true),
        "its slot counts from its grant"
    );
    assert_eq!(
        start(&runs(w.store.clone()), &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::CapabilityDisabled {
                capability: FlowControlCapability::RateLimits
            }
        )),
        "a deployment that switched the capability off starts none"
    );
    assert!(matches!(
        registry
            .remove(Capability::FlowControlRateV1, &actor, nothing)
            .await,
        Ok(Removed::Removed { .. })
    ));
    assert_eq!(
        start(&on, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::ActivationPending {
                capability: FlowControlCapability::RateLimits
            }
        ))
    );
    assert_eq!(
        runs_of(&w, workflow).await,
        1,
        "only the admitted run exists"
    );
}

/// D022-057 (G1198): a concurrency-gated definition is admitted by the one admission point, with no test door, exactly
/// when the process's switch is on, the fleet activated the capability, and the process holds the deployment's digest
/// keys; a process with the switch and the activation but without the keys refuses it `BINARY_CAPABILITY_MISSING`, and
/// one without the switch `CAPABILITY_DISABLED`. The admitted run holds its permit.
#[tokio::test]
async fn a_process_with_the_switch_the_activation_and_the_keys_admits_a_concurrency_gated_run() {
    let w = world().await;
    crate::support::flow_control::activate(&w.db.pool).await;
    let workflow = create(
        &w,
        &json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": "provider-serial"},
            "entrypoint": "fetch",
            "concurrency": {"group": "providers", "key": "input.providerId", "limit": 1},
            "nodes": [{"id": "fetch", "type": "activity", "activity": "provider.fetch"}]
        }),
    )
    .await;
    publish(&w, workflow).await.expect("published");
    let ring = || {
        Arc::new(
            sagat_application::keyed_digest::KeyRing::parse(crate::support::flow_control::RING)
                .expect("a ring"),
        )
    };
    let switched_off = runs(w.store.clone().with_flow_control_keys(ring()));
    assert_eq!(
        start(&switched_off, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::CapabilityDisabled {
                capability: FlowControlCapability::ConcurrencyKeys
            }
        ))
    );
    let without_keys = runs(w.store.clone().with_flow_control(BOTH_ON));
    assert_eq!(
        start(&without_keys, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::BinaryCapabilityMissing
        )),
        "a process without the digest keys serves no gate"
    );
    assert_eq!(runs_of(&w, workflow).await, 0, "nothing was admitted");
    let serving = runs(
        w.store
            .clone()
            .with_flow_control(BOTH_ON)
            .with_flow_control_keys(ring()),
    );
    let run = start(&serving, &w, workflow)
        .await
        .expect("admitted by the one admission point");
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM concurrency_holders WHERE run_id = $1 AND holder_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("holders");
    assert_eq!(held, 1, "the run holds its permit");
    w.assert_consistent(run).await;
}

/// D022-057 (G1198; review of the fixes, architecture M2): a real engine started the way the binary starts, with the
/// switches and the ring of `support::flow_control::process_env`, registers listing both gates (T022-072); one whose
/// ring lacks the version the database has active refuses to start, naming the variable and no key.
#[tokio::test]
async fn a_real_engine_with_the_ring_lists_the_gate_and_one_missing_the_active_version_does_not_start()
 {
    use crate::support::worker::{RunningEngine, engine_env};
    let api = crate::support::api::start_with(crate::support::api::Options::default()).await;
    let activate = |version: i16| {
        let pool = api.db.pool.clone();
        async move {
            let (outcome,): (String,) = sqlx::query_as(
                "SELECT out_outcome FROM flow_control_key_activate($1::smallint, 'operator')",
            )
            .bind(version)
            .fetch_one(&pool)
            .await
            .expect("activate a key version");
            assert_eq!(outcome, "ACTIVATED");
        }
    };
    activate(1).await;
    let pairs = crate::support::flow_control::process_env();
    let mut extra: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    extra.push(("SAGAT_REPLICA_ID", "flow-control-engine"));
    extra.push(("SAGAT_LIVENESS_GRACE_SECS", "30"));
    let mut engine = RunningEngine::start(engine_env(&api.db, &api.idp, &extra)).await;
    let listed = crate::support::eventually("the engine registered", || async {
        let rows: Vec<Vec<String>> = sqlx::query_scalar(
            "SELECT capabilities FROM serving_instances WHERE kind = 'EXECUTION_ENGINE'",
        )
        .fetch_all(&api.db.pool)
        .await
        .expect("registry");
        rows.into_iter().next()
    })
    .await;
    assert!(
        listed.iter().any(|c| c == "flow-control-concurrency-v1")
            && listed.iter().any(|c| c == "flow-control-rate-v1"),
        "this build serves both gates (T022-072, T022-074): {listed:?}"
    );
    engine.finish().await.expect("stopped");

    activate(2).await;
    let refused = RunningEngine::try_start(engine_env(&api.db, &api.idp, &extra))
        .await
        .err()
        .expect("an engine without the active version does not start")
        .to_string();
    assert!(
        refused.contains("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE"),
        "{refused}"
    );
    assert!(
        !refused.contains("5a5a"),
        "no key in the refusal: {refused}"
    );
}

/// FR-022-059, T022-093: a continuation successor is the one other writer of runs. A definition with a run-level gate is
/// refused beside a continuation at publication; a stored one that has it anyway (here, a version changed behind the
/// store's back) is refused at the hand-off, and the same hand-off goes through once the document is what it was.
#[tokio::test]
async fn a_gated_definition_that_reaches_a_continuation_successor_is_refused() {
    let w = world().await;
    let name = format!(
        "gated-successor-{}",
        &Uuid::now_v7().simple().to_string()[..8]
    );
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    let (workflow, version) = w.publish(document.clone()).await;
    let chain = w
        .start(workflow, &json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    let session = w.session(&["segment.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete the activity");

    let mut tampered = document.clone();
    tampered["concurrency"] = json!({"limit": 1});
    let swap = |doc: Value| {
        let pool = w.db.pool.clone();
        async move {
            // each in a transaction of its own: the deferred accounting of the update must not hold the table
            sqlx::query(
                "ALTER TABLE workflow_versions DISABLE TRIGGER workflow_versions_immutable",
            )
            .execute(&pool)
            .await
            .expect("the owner lifts the guard");
            sqlx::query("UPDATE workflow_versions SET document = $2 WHERE id = $1")
                .bind(version.as_uuid())
                .bind(serde_json::to_vec(&doc).expect("json"))
                .execute(&pool)
                .await
                .expect("a version changed behind the store's back");
            sqlx::query(
                "ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_immutable",
            )
            .execute(&pool)
            .await
            .expect("the guard again");
        }
    };
    swap(tampered).await;

    let placement = Placement::new(format!("gated-successor-{name}"));
    Keeper::new(
        w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    // a fresh cache: the store reads the version as it is stored now
    let fresh = || {
        PgRunStore::new(
            w.execution_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        )
    };
    let scheduler = Scheduler::new(
        fresh(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement.clone());
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 0, "{report:?}");
    let segments: i32 = sqlx::query_scalar(
        "SELECT segment_count FROM execution_chains WHERE tenant_id = $1 AND chain_id = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the chain");
    assert_eq!(segments, 1, "no successor was admitted");

    // a gate on a step is refused the same way: the successor's steps would run ungated (security review m1)
    let mut node_gated = document.clone();
    node_gated["nodes"][0]["concurrency"] = json!({"limit": 1});
    swap(node_gated).await;
    let scheduler = Scheduler::new(
        fresh(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement.clone());
    assert_eq!(
        scheduler.tick().await.continuations_advanced,
        0,
        "a node-level gate is refused at the hand-off too"
    );

    swap(document).await;
    let scheduler = Scheduler::new(
        fresh(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let report = scheduler.tick().await;
    assert_eq!(
        report.continuations_advanced, 1,
        "the refusal was the gate's: {report:?}"
    );
}

/// D022-057, keys reviews' minors of 2026-11-20: a control plane checks its ring against the versions the database has
/// in force before it serves anything. During a rotation (version 1 retiring with a live holder, version 2 active) a
/// control plane whose ring lacks the retiring version, or the active one, does not start, naming the variable and no
/// key; a ring with both passes the check.
#[tokio::test]
async fn a_control_plane_missing_a_version_in_force_does_not_start_and_one_with_both_passes() {
    use sagat_control_plane::config::{Config, DatabaseConfig, Environment, ServeConfig};
    use std::io::Write as _;
    let w = crate::support::flow_control::gated_world().await;
    let workflow =
        crate::support::flow_control::publish(&w, crate::run_admission::serial_start("cp-keys", 1))
            .await;
    w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(2::smallint, 'operator')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("activated");
    assert_eq!(outcome, "ACTIVATED");
    let state: String =
        sqlx::query_scalar("SELECT state FROM flow_control_key_versions WHERE version = 1")
            .fetch_one(&w.db.pool)
            .await
            .expect("version 1");
    assert_eq!(state, "RETIRING", "a rotation with a live holder");
    let (one, two) = crate::support::flow_control::RING_TWO
        .split_once(',')
        .expect("two versions");
    for (ring, missing) in [(two, "the retiring version"), (one, "the active version")] {
        let mut file = tempfile::NamedTempFile::new().expect("a key file");
        file.write_all(ring.as_bytes()).expect("written");
        let path = file.path().to_str().expect("a path").to_owned();
        let lookup = |name: &str| match name {
            "SAGAT_OIDC_ISSUER" => Some("https://idp.example/".to_owned()),
            "SAGAT_OIDC_AUDIENCE" => Some("sagat".to_owned()),
            "SAGAT_OIDC_JWKS_URI" => Some("https://idp.example/jwks.json".to_owned()),
            "SAGAT_ADMIN_ADDR" => Some("off".to_owned()),
            "SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED" => Some("true".to_owned()),
            "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE" => Some(path.clone()),
            _ => None,
        };
        let serve = ServeConfig::from_lookup(&lookup).expect("a configuration");
        let config = Config {
            environment: Environment::Dev,
            log_format: sagat_telemetry::LogFormat::Json,
            database: DatabaseConfig {
                url: w.db.url.clone(),
                max_connections: 2,
                statement_timeout: std::time::Duration::from_secs(3),
            },
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let refused = sagat_control_plane::serve(&config, &serve, listener, async {})
            .await
            .expect_err(missing)
            .to_string();
        assert!(
            refused.contains("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE"),
            "{missing}: {refused}"
        );
        assert!(
            !refused.contains("5a5a") && !refused.contains("6b6b"),
            "{missing}: {refused}"
        );
    }
    let both =
        sagat_application::keyed_digest::KeyRing::parse(crate::support::flow_control::RING_TWO)
            .expect("a ring");
    sagat_control_plane::check_flow_control_keys(&w.db.pool, Some(&both))
        .await
        .expect("a ring with every version in force passes");
}

/// D022-062 as amended (re-review of 39157608, Major 1): with the concurrency gate required of the fleet, an engine
/// restarted with its switches off and the key file (to drain) registers listing the gate and serves, instead of being
/// refused by the registry; and a switched-off engine whose file lacks the version in force does not start (its ring is
/// checked whatever its switches).
#[tokio::test]
async fn a_draining_engine_with_the_keys_stays_in_a_fleet_that_requires_the_gate() {
    use crate::support::worker::{RunningEngine, engine_env};
    let api = crate::support::api::start_with(crate::support::api::Options {
        flow_control: true,
        ..crate::support::api::Options::default()
    })
    .await;
    let file = crate::support::flow_control::process_env()
        .into_iter()
        .find(|(name, _)| *name == "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE")
        .expect("the key file")
        .1;
    let extra = [
        ("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE", file.as_str()),
        ("SAGAT_REPLICA_ID", "draining-engine"),
        ("SAGAT_LIVENESS_GRACE_SECS", "30"),
    ];
    let mut engine = RunningEngine::try_start(engine_env(&api.db, &api.idp, &extra))
        .await
        .expect("a draining engine starts in a fleet that requires the gate");
    let listed = crate::support::eventually("the engine registered", || async {
        let rows: Vec<Vec<String>> = sqlx::query_scalar(
            "SELECT capabilities FROM serving_instances WHERE kind = 'EXECUTION_ENGINE' AND scheduler_owner IS NOT NULL",
        )
        .fetch_all(&api.db.pool)
        .await
        .expect("registry");
        rows.into_iter().next()
    })
    .await;
    assert!(
        listed.iter().any(|c| c == "flow-control-concurrency-v1"),
        "{listed:?}"
    );
    engine.finish().await.expect("stopped");

    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(2::smallint, 'operator')",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("activated");
    assert_eq!(outcome, "ACTIVATED");
    let refused = RunningEngine::try_start(engine_env(&api.db, &api.idp, &extra))
        .await
        .err()
        .expect("a switched-off engine without the active version does not start")
        .to_string();
    assert!(
        refused.contains("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE"),
        "{refused}"
    );
}
