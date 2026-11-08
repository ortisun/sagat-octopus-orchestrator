//! A lake runner leases the step of a lake run, with the attempt input made from the run's data (feature 035, T035-056, D035-138).
//!
//! Through the real store, the real worker service and the runner's own reader: a connection, a published workflow, a run started on a
//! process that has the lake on, a lake runner of the `lake` profile, and a poll. It runs inside the lifecycle scenario of `main.rs`,
//! after the managed secret `db-credentials` exists in OpenBao.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append, iceberg_in, publish_on_lake, rows_schema, write};
use crate::support::engine::{World, ctx, task_ref};
use sagat_application::{
    clock::SystemClock,
    database::ConnectionService,
    entropy::SystemEntropy,
    execution::Engine,
    runs::{RunService, StartRun},
    worker::{Capabilities, WorkerService, WorkerSettings},
};
use sagat_domain::{
    binding::BindingProfile,
    classification::EgressCeiling,
    database::{ConnectionStatus, WriteMode},
    identity::Role,
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_lake::protocol::{AttemptInput, Records};
use serde_json::{Value, json};

pub async fn run(w: &World) {
    let ns = w.namespace.id();
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    catalog
        .put(
            &w.admin,
            ns,
            write(
                "lake-lease",
                0,
                ConnectionStatus::Active,
                iceberg_in(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Internal,
                    vec![WriteMode::Append],
                ),
            ),
            &ctx(),
        )
        .await
        .expect("the connection");
    let workflow = publish_on_lake(
        w,
        &w.admin,
        append(
            "lake-lease-flow",
            "lake-lease",
            &["/id", "/total"],
            Some(rows_schema()),
        ),
    )
    .await
    .expect("a workflow on the connection");

    // The process that admits runs and the one that leases them have the lake on.
    let lake_store = w.store.clone().with_lake_enabled(true);
    let runs = RunService::new(
        lake_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    );
    let workers = WorkerService::new(
        lake_store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let start = |key: &'static str, rows: &Value| {
        let input = serde_json::to_vec(&json!({ "rows": rows })).expect("input");
        let runs = &runs;
        let admin = &w.admin;
        async move {
            runs.start(
                admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &input,
                    correlation_id: None,
                    deadline: None,
                    key,
                },
                &ctx(),
            )
            .await
        }
    };

    // A lake runner: the role, the profile and the activity it serves.
    let runner = w.member("lake-lease-runner", Role::DatabaseRunner).await;
    w.bindings
        .bind_with_profile(
            &w.admin,
            ns,
            runner.subject().as_str(),
            &["sagat.database.iceberg.write".to_owned()],
            None,
            Some(BindingProfile::Lake),
            &ctx(),
        )
        .await
        .expect("bind the lake runner");
    let registered = workers
        .register(
            &runner,
            "payments",
            "lake-lease-machine",
            &Capabilities {
                activities: vec!["sagat.database.iceberg.write".to_owned()],
                languages: vec![],
                pools: vec![],
                max_inflight: 32,
                protocol_version: "1.6".into(),
            },
        )
        .await
        .expect("register the lake runner");
    let token = registered.session_token.expose().to_owned();
    let poll = || async {
        workers
            .poll(
                &runner,
                registered.worker_id,
                &token,
                8,
                Some("0af7651916cd43dd8448eb211c80319c"),
            )
            .await
            .expect("poll")
    };

    // The step is leased with the document made from the run's own input, and the runner's reader accepts it.
    let rows = json!([{"id": "a-1", "total": 12}, {"id": "a-2", "total": 7}]);
    let started = start("lake-lease-start-00001", &rows)
        .await
        .expect("a run of an Iceberg write with the lake on")
        .info
        .run_id;
    // Runs of Iceberg writes made earlier in this scenario are waiting too, each pinned to its own revision: the runner is
    // handed all that are admitted, and every document it is handed is one its own reader accepts.
    let tasks = poll().await;
    for leased in &tasks {
        assert_eq!(leased.activity.as_str(), "sagat.database.iceberg.write");
        AttemptInput::read(&leased.input)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&leased.input)));
    }
    let task = tasks
        .iter()
        .find(|task| task.run_id == started)
        .expect("the lake runner is handed the step of the run just started");
    let input =
        AttemptInput::read(&task.input).expect("the runner's own reader accepts the lease input");
    assert_eq!(
        input.effect_key(),
        task.idempotency_key,
        "the effect key is the key of the task, stable across its attempts"
    );
    assert!(matches!(input.records(), Records::Inline(items) if items.len() == 2));
    let document: Value = serde_json::from_slice(&task.input).expect("JSON");
    assert_eq!(
        document["records"]["inline"], rows,
        "the records are the values the expression selects"
    );
    assert_eq!(document["priorAttemptMayHaveLeftFiles"], false);
    assert_eq!(document["allowEmpty"], false);
    assert!(
        !String::from_utf8_lossy(&task.input).contains("db-credentials"),
        "no credential and no secret name travels in the lease"
    );

    // A healthy claim is not a denial: nothing was audited as one for this run.
    let denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2 AND resource_id = $3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(sagat_domain::audit::AuditAction::DatabaseDenied.as_str())
    .bind(started.to_string())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(
        denials, 0,
        "a lake step is claimed, not refused, by a lake runner"
    );

    // A second attempt of the same effect: the same key and the same first instant, and the host is told an earlier attempt
    // may have left files. A retryable failure of the catalog sends the step to its backoff; the scheduler wakes it.
    let first_instant = document["firstAttemptStartedAt"].clone();
    let fail = workers
        .fail(
            &runner,
            &task_ref(task),
            "CATALOG_UNAVAILABLE",
            true,
            "lake-lease-fail-1",
        )
        .await;
    assert!(fail.is_ok(), "{fail:?}");
    w.age_retry(started, "save").await;
    w.scheduler.tick().await;
    let second = poll()
        .await
        .into_iter()
        .find(|leased| leased.run_id == started)
        .expect("the second attempt is leased");
    assert_eq!(second.fence.attempt, 2);
    assert_eq!(second.idempotency_key, task.idempotency_key);
    let again: Value = serde_json::from_slice(&second.input).expect("JSON");
    assert_eq!(
        again["effectKey"], document["effectKey"],
        "the effect key is stable across attempts"
    );
    assert_eq!(
        again["firstAttemptStartedAt"], first_instant,
        "the instant of the first attempt is read from the journal, not made up again"
    );
    assert_eq!(again["priorAttemptMayHaveLeftFiles"], true);
    AttemptInput::read(&second.input).expect("the runner's reader accepts the second document too");

    // Data that cannot be leased fails the step for good, with a closed code, and nothing is leased for it. The schema does not
    // require `rows`, so a run without it is admitted: its data is judged when the step is leased.
    let broken = runs
        .start(
            &w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: br#"{}"#,
                correlation_id: None,
                deadline: None,
                key: "lake-lease-start-00002",
            },
            &ctx(),
        )
        .await
        .expect("the run is admitted")
        .info
        .run_id;
    assert!(
        poll().await.iter().all(|leased| leased.run_id != broken),
        "a run whose records cannot be selected is not leased"
    );
    // `step.failed` is a failure for good: one, with the closed code that names the cause.
    let failed: Vec<String> = sqlx::query_scalar(
        "SELECT detail->>'code' FROM run_events WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'step.failed'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(broken.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(failed, vec!["DATABASE_CONFIGURATION_INVALID".to_owned()]);
    assert_eq!(
        w.run_status(broken).await,
        sagat_domain::execution::RunStatus::Failed
    );

    // Two brakes, both real and both reversible: the process that leases has the lake off, or the connection is paused. A run
    // admitted before either waits, ready, and is leased once the brake is released.
    let waiting = start(
        "lake-lease-start-00003",
        &json!([{"id": "w-1", "total": 1}]),
    )
    .await
    .expect("a run")
    .info
    .run_id;
    let off = w
        .workers
        .poll(
            &runner,
            registered.worker_id,
            &token,
            8,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("poll");
    assert!(
        off.iter().all(|leased| leased.run_id != waiting),
        "a process that has the lake off leases no lake step"
    );
    let config = |status| {
        write(
            "lake-lease",
            if status == ConnectionStatus::Paused {
                1
            } else {
                2
            },
            status,
            iceberg_in(
                "https://catalog.example/api",
                "db-credentials",
                EgressCeiling::Internal,
                vec![WriteMode::Append],
            ),
        )
    };
    catalog
        .put(&w.admin, ns, config(ConnectionStatus::Paused), &ctx())
        .await
        .expect("pause the connection");
    assert!(
        poll().await.iter().all(|leased| leased.run_id != waiting),
        "a paused connection stops the claim of the runs that were admitted on it"
    );
    catalog
        .put(&w.admin, ns, config(ConnectionStatus::Active), &ctx())
        .await
        .expect("resume the connection");
    assert!(
        poll().await.iter().any(|leased| leased.run_id == waiting),
        "released, the waiting run is leased"
    );

    // Lake attempts have a budget of their own: eight at once in a namespace. Whatever is leased already counts, and the
    // rest of the runs wait, ready, without spending an attempt.
    let in_flight = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM steps WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3 \
             AND status IN ('LEASED', 'RUNNING') AND cardinality(instance_path_nodes) = 0",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(ns.as_uuid())
        .bind("sagat.database.iceberg.write")
        .fetch_one(&w.db.pool)
        .await
        .unwrap()
    };
    let before = in_flight().await;
    assert!(
        before >= 1,
        "the attempts leased above are in flight: {before}"
    );
    let mut bulk = Vec::new();
    for n in 0..9 {
        let key: &'static str = Box::leak(format!("lake-lease-bulk-{n:05}").into_boxed_str());
        bulk.push(
            start(key, &json!([{"id": format!("b-{n}"), "total": n}]))
                .await
                .expect("a run")
                .info
                .run_id,
        );
    }
    let mut leased = 0_i64;
    loop {
        let round = poll().await;
        let mine = round
            .iter()
            .filter(|task| bulk.contains(&task.run_id))
            .count();
        if mine == 0 {
            break;
        }
        leased += i64::try_from(mine).unwrap();
    }
    assert_eq!(
        before + leased,
        8,
        "eight lake attempts at once in a namespace, counting the ones leased before ({before})"
    );
    assert_eq!(in_flight().await, 8);
}
