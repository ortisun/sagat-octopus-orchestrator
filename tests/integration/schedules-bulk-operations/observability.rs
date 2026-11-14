//! What leaves the platform about the schedule, batch, reset and business-id paths (`T018-071`, `SC-018-014`,
//! `FR-018-044`, `FR-018-045`, `NFR-018-006`): the customer's text (an identifier, a schedule's name, note and input, a
//! reason, a selector) is in no span of the OTLP export, in no metric label and in no log line, and each ceiling the
//! feature adds refuses before any work with its reason, is counted in the series of its resource and leaves no record.
//!
//! The export is the real exporter to a collector that keeps the bodies it receives (`support/collector.rs`), so what is
//! searched is the OTLP wire conversion of the spans, the live ones of the process and the ones the telemetry projector
//! makes of the journal of a run (`G1971`). Real PostgreSQL, real services, a real API.
//!
//! **A search that finds nothing proves something only when it can find.** Three things keep these tests from passing on
//! a pipeline that exports nothing:
//! * the search is shown to find a canary put into a span, a log line and a metric label the way the pipeline would export
//!   it (`the_search_finds_…`), and `assert_absent` is shown to refuse it;
//! * the flow runs inside a root span with a trace id, because the exporter takes a span only when it has a trace (its own,
//!   or its parent's): a span a service created with customer text in it is exported, and a mutant that creates one turns
//!   the test red (it was shown to, once). And spans of the platform's crates, not only the probes, are required to have
//!   reached the collector;
//! * every class of canary is shown to have been carried (stored where the platform keeps it: the prefix of a schedule's action
//!   in its revision, the prefix of a batch selector in its job, both again in the ids of the runs they made or reached), and the
//!   run each class concerns is shown to have been told to the collector.
//!
//! The canaries are `[a-z0-9-]`, which JSON writes as they are: the wire is searched for the text as it is.
#![allow(clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::time::Duration as StdDuration;

use reqwest::Method;
use sagat_application::runs::{
    BusinessStart, RerunBusinessId, ResetRequest, ResetSourcePolicy, RunOperation, StartOptions,
    StartRun,
};
use sagat_domain::batch::{Action, Filter, Justification, Selector};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::execution::{ResetReapply, RunId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::Note;
use sagat_scheduler::{ProjectorSettings, TelemetryProjector};
use sagat_telemetry::{Metrics, TraceId};
use serde_json::{Value, json};
use time::Duration;
use tracing::Instrument;
use uuid::Uuid;

use crate::batch_executor::{
    create as create_job, executor, live, select, service as batch_service, settle, write,
};
use crate::counted::series;
use crate::firing::{fixture, hourly};
use crate::reset::ended_chain;
use crate::support::collector::Collector;
use crate::support::digests::{all_table_digests, changed};
use crate::support::engine::ctx;
use crate::support::worker::{chain_ir, platform};

/// What `body` does on a runtime of one thread, with a recorder of the test's own and the page it rendered.
fn observed<T, F: Future<Output = T>>(
    runtime: &tokio::runtime::Runtime,
    body: impl FnOnce() -> F,
) -> (T, String) {
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let out = metrics::with_local_recorder(&recorder, || runtime.block_on(body()));
    (out, metrics.render())
}

fn current_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// A piece of customer text of a class, unique to the test; a failure names the class and never the text.
struct Canary {
    class: &'static str,
    text: String,
}

fn canary(class: &'static str, text: String) -> Canary {
    assert!(
        text.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "a canary is [a-z0-9-], which JSON writes as it is"
    );
    Canary { class, text }
}

fn canaries(suffix: &str) -> Vec<Canary> {
    vec![
        canary("schedule name", format!("canary-obs-name-{suffix}")),
        canary("schedule note", format!("canary-obs-note-{suffix}")),
        canary("schedule input", format!("canary-obs-input-{suffix}")),
        canary("pause note", format!("canary-obs-pause-{suffix}")),
        canary("business id", format!("canary-obs-business-{suffix}")),
        canary("batch reason", format!("canary-obs-batch-reason-{suffix}")),
        canary("batch selector", format!("canary-obs-prefix-{suffix}")),
        canary(
            "schedule prefix",
            format!("canary-obs-schedprefix-{suffix}"),
        ),
        canary("reset reason", format!("canary-obs-reset-reason-{suffix}")),
    ]
}

/// The classes of the canaries that `haystack` holds.
fn leaked(haystack: &str, canaries: &[Canary]) -> Vec<&'static str> {
    canaries
        .iter()
        .filter(|canary| haystack.contains(&canary.text))
        .map(|canary| canary.class)
        .collect()
}

fn assert_absent(surface: &str, haystack: &str, canaries: &[Canary]) {
    if let Some(class) = leaked(haystack, canaries).first() {
        panic!("the {class} canary reached {surface}");
    }
}

fn text_of(canaries: &[Canary], class: &str) -> String {
    canaries
        .iter()
        .find(|c| c.class == class)
        .expect("a class")
        .text
        .clone()
}

/// The names of the tables that keep `text`, as text or as the hex of its bytes (a payload is bytes).
async fn tables_holding(pool: &sqlx::PgPool, text: &str) -> Vec<String> {
    let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables WHERE schemaname = current_schema() ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("tables");
    let mut holding = Vec::new();
    for table in tables {
        let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%' OR t::text LIKE '%' || $2 || '%'"
        )))
        .bind(text)
        .bind(&hex)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("search {table}: {error}"));
        if hits > 0 {
            holding.push(table);
        }
    }
    holding
}

// --- the control: the search can find ---------------------------------------------------------------------------------------

#[test]
fn the_search_finds_a_canary_that_is_put_where_the_pipeline_exports_it_and_assert_absent_refuses_it()
 {
    let control = canary(
        "control",
        format!("canary-obs-control-{}", Uuid::now_v7().simple()),
    );
    let runtime = current_thread();
    let collector = runtime.block_on(Collector::start());
    collector.keep_bodies();
    let _enter = runtime.enter();
    let (guard, sink, logs) = collector.install_with_logs(1.0);
    let ((), page) = observed(&runtime, || async {
        let span = tracing::info_span!(
            target: "sagat_observability_control",
            "observability_control",
            trace_id = %TraceId::generate(),
            customer = %control.text
        );
        async {
            tracing::warn!(target: "sagat_observability_control", customer = %control.text, "a control line");
            metrics::counter!("observability_control_total", "label" => control.text.clone())
                .increment(1);
        }
        .instrument(span)
        .await;
    });
    runtime.block_on(sink.shutdown(StdDuration::from_secs(5)));
    drop(guard);

    let wire = collector.bodies().join("\n");
    let log_text = logs.text();
    for (surface, haystack) in [
        ("the OTLP export", wire.as_str()),
        ("a log line", log_text.as_str()),
        ("a metric label", page.as_str()),
    ] {
        assert_eq!(
            leaked(haystack, std::slice::from_ref(&control)),
            ["control"],
            "the search finds a canary on {surface}"
        );
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_absent(surface, haystack, std::slice::from_ref(&control));
        }));
        assert!(refused.is_err(), "assert_absent refuses it on {surface}");
    }
}

// --- the flows -----------------------------------------------------------------------------------------------------------------

/// What the flows made: the run each class of canary concerns, and where each class was kept.
struct Flow {
    schedule_run: Uuid,
    /// The run of the schedule whose action names a business-id prefix, once its first id was held and once it was free.
    prefixed_run: Uuid,
    id_run: Uuid,
    /// A run a batch job reached by the prefix of its business id.
    selected_run: Uuid,
    batch_target: Uuid,
    reset_source: Uuid,
    reset_new: Uuid,
    carried: Vec<(&'static str, Vec<String>)>,
}

#[test]
fn customer_text_of_a_schedule_a_batch_a_reset_and_an_identifier_is_in_no_exported_span_label_or_log_line()
 {
    let suffix = Uuid::now_v7().simple().to_string();
    let canaries = canaries(&suffix);
    let text = |class: &str| text_of(&canaries, class);
    let runtime = current_thread();
    let collector = runtime.block_on(Collector::start());
    collector.keep_bodies();
    // every span of this thread is exported to the collector, and every log line, at any level, is kept
    let _enter = runtime.enter();
    let (guard, sink, logs) = collector.install_with_logs(1.0);
    tracing::warn!(probe = "observability-probe", "the capture is on");
    drop(tracing::info_span!(
        target: "sagat_observability_probe",
        "observability_span_probe",
        trace_id = %TraceId::generate(),
        probe = "kept"
    ));
    // the exporter takes a span that has a trace, or a parent that has one: the flow runs inside one, so that a span a
    // service creates (a `sagat_*` target) is exported with the trace of the flow
    let root = tracing::info_span!(
        target: "sagat_observability_flow",
        "observability_flow",
        trace_id = %TraceId::generate()
    );

    let (flow, page) = observed(&runtime, || {
        async {
            let f = fixture().await;
            let namespace = f.w.namespace.id();

            // --- a reset, with a reason (first: the workers that finish its steps take the next task, whoever's it is)
            let source = ended_chain(&f).await;
            let points =
                f.w.runs
                    .reset_points(&f.w.admin, source, &ctx())
                    .await
                    .expect("points");
            let reset_reason = text("reset reason");
            let made =
                f.w.runs
                    .reset(
                        &f.w.admin,
                        source,
                        ResetRequest {
                            sequence: points[1].sequence,
                            reason: &reset_reason,
                            acknowledge_reexecution: true,
                            business: RerunBusinessId::Keep,
                            reapply: ResetReapply::None,
                            source_policy: ResetSourcePolicy::RequireTerminal,
                        },
                        "observability-reset-key",
                        &ctx(),
                    )
                    .await
                    .expect("the reset");
            let reset_new = made.run.expect("the new run").info.run_id.as_uuid();

            // --- a schedule: its name, its note, its input, a pause and a run it starts. A note is kept with the state that carries
            // it (the latest one is the one the row holds), so each class is looked for right after the state that keeps it
            let mut carried: Vec<(&'static str, Vec<String>)> = Vec::new();
            let mut schedule = f.write_of(&text("schedule name"), &hourly(), &json!({}), None);
            schedule.input = json!({ "customer": text("schedule input") });
            schedule.paused = true;
            schedule.note = Some(Note::new(&text("schedule note")).expect("a note"));
            let schedule = f.try_create(schedule).await.expect("create the schedule");
            for class in ["schedule name", "schedule note", "schedule input"] {
                carried.push((class, tables_holding(&f.w.db.pool, &text(class)).await));
            }
            f.service
                .resume(
                    &f.w.admin,
                    schedule.id,
                    Note::new("resumed by the test").expect("a note"),
                    &ctx(),
                )
                .await
                .expect("resume");
            f.service
                .pause(
                    &f.w.admin,
                    schedule.id,
                    Note::new(&text("pause note")).expect("a note"),
                    &ctx(),
                )
                .await
                .expect("pause");
            carried.push((
                "pause note",
                tables_holding(&f.w.db.pool, &text("pause note")).await,
            ));
            f.service
                .resume(
                    &f.w.admin,
                    schedule.id,
                    Note::new("resumed again by the test").expect("a note"),
                    &ctx(),
                )
                .await
                .expect("resume");
            f.clock.advance(Duration::hours(1) + Duration::seconds(1));
            assert!(f.drain(&schedule).await >= 1, "the schedule started a run");
            let schedule_run = f.runs(&schedule).await[0];

            // --- a schedule whose action names a business-id prefix: the first id is held by a person's run (the action is a
            // recorded conflict), the next is free (the action starts a run under it)
            let prefix = text("schedule prefix");
            let prefixed = f
                .try_create({
                    let mut write = f.write_of(
                        "obs-prefixed",
                        &hourly(),
                        &json!({}),
                        Some(json!({
                            "workflowId": f.workflow.to_string(),
                            "businessIdPrefix": prefix,
                            "businessIdConflictPolicy": "FAIL"
                        })),
                    );
                    write.note = None;
                    write
                })
                .await
                .expect("create the schedule with a prefix");
            // a person holds the id of the first time of the schedule: 14:00 on the day of the fixture, the clock being at 13:00:01
            let held = format!("{prefix}-2026-10-28T14:00:00Z");
            f.w.runs
                .start_with_options(
                    &f.w.admin,
                    &StartRun {
                        namespace: "payments",
                        workflow: f.workflow,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: "observability-held",
                    },
                    StartOptions {
                        business: Some(BusinessStart {
                            id: BusinessId::new(&held).expect("an identifier"),
                            conflict: ConflictPolicy::Fail,
                            reuse: ReusePolicy::AllowDuplicate,
                            window: ReuseWindow::default(),
                        }),
                        ..StartOptions::default()
                    },
                    &ctx(),
                )
                .await
                .expect("the holder");
            f.clock.advance(Duration::hours(1));
            f.drain(&prefixed).await; // the held id: a recorded conflict
            f.clock.advance(Duration::hours(1));
            f.drain(&prefixed).await; // a free id: a run
            let decisions: Vec<String> = sqlx::query_scalar(
                "SELECT outcome FROM schedule_actions WHERE schedule_id = $1 ORDER BY nominal_at",
            )
            .bind(prefixed.id.as_uuid())
            .fetch_all(&f.w.db.pool)
            .await
            .expect("decisions");
            assert_eq!(
                decisions.first().map(String::as_str),
                Some("BUSINESS_ID_CONFLICT"),
                "the first action found its id held: {decisions:?}"
            );
            let prefixed_run = *f
                .runs(&prefixed)
                .await
                .last()
                .expect("the schedule with a prefix started a run");

            // --- an identifier: a start, a refusal, a returned holder, the lookup
            let id = text("business id");
            let start = |conflict: ConflictPolicy, key: &str| {
                let (f, id, key) = (&f, id.clone(), key.to_owned());
                async move {
                    f.w.runs
                        .start_with_options(
                            &f.w.admin,
                            &StartRun {
                                namespace: "payments",
                                workflow: f.workflow,
                                version: None,
                                input: b"{}",
                                correlation_id: None,
                                deadline: None,
                                key: &key,
                            },
                            StartOptions {
                                business: Some(BusinessStart {
                                    id: BusinessId::new(&id).expect("an identifier"),
                                    conflict,
                                    reuse: ReusePolicy::AllowDuplicate,
                                    window: ReuseWindow::default(),
                                }),
                                ..StartOptions::default()
                            },
                            &ctx(),
                        )
                        .await
                }
            };
            let id_run = start(ConflictPolicy::Fail, "observability-first")
                .await
                .expect("the first start")
                .run
                .info
                .run_id
                .as_uuid();
            let _ = start(ConflictPolicy::Fail, "observability-second").await;
            let _ = start(ConflictPolicy::UseExisting, "observability-third").await;
            let _ =
                f.w.runs
                    .business_binding(&f.w.admin, namespace, &BusinessId::new(&id).expect("id"))
                    .await;

            // --- a batch job by the prefix of a business id: the runs started under it are selected and cancelled, and the same
            // prefix is asked of the run list and the preview
            let selector_text = text("batch selector");
            let mut selected = Vec::new();
            for n in 0..2 {
                selected.push(
                    f.w.runs
                        .start_with_options(
                            &f.w.admin,
                            &StartRun {
                                namespace: "payments",
                                workflow: f.workflow,
                                version: None,
                                input: b"{}",
                                correlation_id: None,
                                deadline: None,
                                key: &format!("observability-selected-{n}"),
                            },
                            StartOptions {
                                business: Some(BusinessStart {
                                    id: BusinessId::new(&format!("{selector_text}-{n}"))
                                        .expect("an identifier"),
                                    conflict: ConflictPolicy::Fail,
                                    reuse: ReusePolicy::AllowDuplicate,
                                    window: ReuseWindow::default(),
                                }),
                                ..StartOptions::default()
                            },
                            &ctx(),
                        )
                        .await
                        .expect("a run under the prefix")
                        .run
                        .info
                        .run_id
                        .as_uuid(),
                );
            }
            let by_prefix = Selector::filter(Filter {
                business_id_prefix: Some(selector_text.clone()),
                ..Filter::default()
            })
            .expect("selector");
            let preview = batch_service(&f)
                .preview(&f.w.admin, namespace, &by_prefix, None)
                .await
                .expect("a preview by the prefix");
            assert_eq!(
                preview.matched, 2,
                "the preview counts the runs of the prefix"
            );
            let listed =
                f.w.runs
                    .list(
                        &f.w.admin,
                        "payments",
                        &sagat_application::runs::RunFilter {
                            business_id_prefix: Some(selector_text.clone()),
                            ..sagat_application::runs::RunFilter::default()
                        },
                        &sagat_application::identity::PageRequest::default(),
                    )
                    .await
                    .expect("the run list by the prefix");
            assert_eq!(listed.items.len(), 2);
            let mut by_prefix_job = write(Action::Cancel, by_prefix);
            by_prefix_job.reason = Justification::new(&text("batch reason")).expect("a reason");
            let by_prefix_job =
                create_job(&f, by_prefix_job, "observability-batch-prefix-key").await;
            live(&f);
            let exec = executor(&f);
            select(&exec, &by_prefix_job).await;
            let _ = settle(&exec, &by_prefix_job).await;
            let reached: Vec<Uuid> = sqlx::query_scalar(
                "SELECT run_id FROM batch_items WHERE job_id = $1 ORDER BY run_id",
            )
            .bind(by_prefix_job.id)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("items");
            let mut expected = selected.clone();
            expected.sort_unstable();
            assert_eq!(reached, expected, "the job reached the runs of the prefix");
            let selected_run = selected[0];
            // and one that is accepted, with a reason of the canary's class, that cancels a run of its own
            let batch_target = f.w.start(f.workflow, &json!({})).await;
            let mut accepted = write(
                Action::Cancel,
                Selector::run_ids(vec![batch_target.as_uuid()]).expect("selector"),
            );
            accepted.reason = Justification::new(&text("batch reason")).expect("a reason");
            let job = create_job(&f, accepted, "observability-batch-key").await;
            // the clock of the test follows the database's, which the job was stamped with
            live(&f);
            let exec = executor(&f);
            select(&exec, &job).await;
            let _ = settle(&exec, &job).await;
            assert!(
                f.w.run_status(batch_target).await.is_terminal()
                    || f.w.run_status(batch_target).await
                        == sagat_domain::execution::RunStatus::Cancelling,
                "the job acted on its run"
            );

            // --- where each class was kept: the flows carried the text to a place where a leak could have shown
            for class in [
                "business id",
                "batch reason",
                "reset reason",
                "batch selector",
                "schedule prefix",
            ] {
                carried.push((class, tables_holding(&f.w.db.pool, &text(class)).await));
            }

            // --- the runs that were cancelled end, and the runs of all of it are told by the projector
            let open: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM runs WHERE status IN ('RUNNING', 'CANCELLING')")
                    .fetch_all(&f.w.db.pool)
                    .await
                    .expect("the open runs");
            for run in open {
                f.w.operate_run(RunId::new(run), RunOperation::Cancel).await;
            }
            let passes = crate::business_ids::scheduler_of(&f).await;
            for _ in 0..6 {
                let _ = passes.tick().await;
            }
            let projector = TelemetryProjector::new(
                f.w.store.clone(),
                Some(collector.client()),
                ProjectorSettings::default(),
            );
            for _ in 0..12 {
                projector.run_once().await;
            }
            Flow {
                schedule_run,
                prefixed_run,
                id_run,
                selected_run,
                batch_target: batch_target.as_uuid(),
                reset_source: source.as_uuid(),
                reset_new,
                carried,
            }
        }
        .instrument(root)
    });
    runtime.block_on(sink.shutdown(StdDuration::from_secs(5)));
    drop(guard);

    // the capture was listening: the platform's runs were told, and our own probes are in the log
    let told: Vec<String> = collector
        .named("workflow.run")
        .iter()
        .filter_map(|span| span.attr("sagat.run.id").map(str::to_owned))
        .collect();
    // each class is shown to have been carried, and the run it concerns is shown to have reached the collector
    for (class, tables) in &flow.carried {
        assert!(
            !tables.is_empty(),
            "the flows did not carry the {class} canary anywhere: nothing was searched for it"
        );
    }
    let concerns = [
        ("schedule name", flow.schedule_run),
        ("schedule note", flow.schedule_run),
        ("schedule input", flow.schedule_run),
        ("pause note", flow.schedule_run),
        ("business id", flow.id_run),
        ("schedule prefix", flow.prefixed_run),
        ("batch selector", flow.selected_run),
        ("batch reason", flow.batch_target),
        ("reset reason (its source)", flow.reset_source),
        ("reset reason (its new run)", flow.reset_new),
    ];
    for (class, run) in concerns {
        assert!(
            told.contains(&run.to_string()),
            "the run the {class} canary concerns was not told to the collector: {told:?}"
        );
    }
    assert!(
        logs.text().contains("observability-probe"),
        "the log capture was listening"
    );
    let bodies = collector.bodies();
    assert!(!bodies.is_empty(), "something was exported");
    // the live spans of the process were exported too, not only the runs the projector told: the probe span is on the wire,
    // and so are spans of the platform's own crates (the scheduler's actions on the runs), not only ours
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("observability_span_probe")),
        "the exporter was listening to the spans of this thread"
    );
    let names: std::collections::BTreeSet<String> = collector
        .spans()
        .iter()
        .map(|span| span.name.clone())
        .collect();
    for (crate_, span) in [
        ("the scheduler", "scheduler.keeper.round"),
        ("the application", "secret.output.scan"),
        ("the PostgreSQL adapter", "database.admit"),
    ] {
        assert!(
            names.contains(span),
            "spans of {crate_} reached the collector, not only probes: {names:?}"
        );
    }
    assert!(
        names.contains("observability_flow"),
        "the root span of the flow was exported (its children are exported with its trace): {names:?}"
    );

    // the OTLP wire conversion: every body, whole, as the exporter sent it
    let wire = bodies.join("\n");
    assert_absent("the OTLP export", &wire, &canaries);
    assert_absent("a log line", &logs.text(), &canaries);
    assert_absent("a metric label", &page, &canaries);
    // and the closed labels of the series this feature adds
    for line in page.lines().filter(|line| {
        [
            "schedule_actions_total",
            "schedule_action_lag_seconds",
            "batch_items_total",
            "batch_item_duration_seconds",
            "workflow_resets_total",
            "quota_exceeded_total",
        ]
        .iter()
        .any(|name| line.starts_with(name))
    }) {
        let inside = &line[line.find('{').map_or(0, |i| i + 1)..line.rfind('}').unwrap_or(0)];
        for pair in inside.split(',').filter(|pair| !pair.is_empty()) {
            let key = pair.split('=').next().unwrap_or_default();
            assert!(
                ["kind", "outcome", "action", "le", "limit", "state"].contains(&key),
                "an open label `{key}` on a series of the feature"
            );
        }
    }
}

// --- the same through the API ----------------------------------------------------------------------------------------------

async fn set_limit(
    p: &crate::support::worker::Platform,
    resource: &str,
    namespace: bool,
    value: i64,
) {
    let namespace_id = if namespace {
        Some(
            p.namespace_id()
                .await
                .parse::<Uuid>()
                .expect("the namespace id"),
        )
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, $3, $4, 'admin')",
    )
    .bind(p.tenant.as_uuid())
    .bind(namespace_id)
    .bind(resource)
    .bind(value)
    .execute(&p.api.db.pool)
    .await
    .expect("a limit");
}

fn assert_refused(
    resource: &str,
    (status, body): &(u16, Value),
    reason: &str,
    limit: &str,
    canaries: &[Canary],
) {
    assert_eq!(*status, 429, "{resource}: {body}");
    assert_eq!(body["code"], "QUOTA_EXCEEDED", "{resource}: {body}");
    assert_eq!(body["details"]["reason"], reason, "{resource}: {body}");
    assert_eq!(body["details"]["limit"], limit, "{resource}: {body}");
    assert_eq!(body["retryable"], false, "{resource}: {body}");
    assert_absent(
        &format!("the refusal of {resource}"),
        &body.to_string(),
        canaries,
    );
}

/// One refusal as the test saw it: what the wire said, and the digest of every table before and after it.
struct Refusal {
    resource: &'static str,
    answer: (u16, Value),
    reason: &'static str,
    limit: String,
    before: std::collections::BTreeMap<String, String>,
    after: std::collections::BTreeMap<String, String>,
}

#[test]
fn every_ceiling_the_feature_adds_refuses_before_work_says_why_is_counted_by_its_resource_and_leaves_nothing()
 {
    let suffix = Uuid::now_v7().simple().to_string();
    let canaries = canaries(&suffix);
    let text = |class: &str| text_of(&canaries, class);
    let runtime = current_thread();
    let collector = runtime.block_on(Collector::start());
    collector.keep_bodies();
    let _enter = runtime.enter();
    let (guard, sink, logs) = collector.install_with_logs(1.0);
    let (outcome, page) = observed(&runtime, || async {
        let p = platform().await;
        let namespace = p.namespace_id().await;
        let workflow = p.publish("reports", &chain_ir("reports")).await;
        let schedules = format!("/namespaces/{namespace}/schedules");
        let batches = format!("/namespaces/{namespace}/batch-operations");
        let schedule_body = |name: &str| {
            json!({
                "name": name,
                "spec": {"intervals": [{"everyMs": 3_600_000}]},
                "action": {"workflowId": workflow, "input": {"customer": text("schedule input")}}
            })
        };
        let job_body = json!({
            "selector": {"filter": {"status": ["RUNNING"]}},
            "action": "CANCEL",
            "reason": text("batch reason")
        });
        let mut refusals: Vec<Refusal> = Vec::new();

        // an identifier is carried in the URL of a request: a start that binds it, and the lookup of the binding
        let id = text("business id");
        let (status, started) = p
            .rest(
                Method::POST,
                "/runs",
                Some(&json!({"workflowId": workflow, "input": {"item": "book"}, "businessId": id})),
            )
            .await;
        assert_eq!(status, 202, "{started}");
        let (status, binding) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{namespace}/business-ids/{id}"),
                None,
            )
            .await;
        assert_eq!(status, 200, "{binding}");

        // one schedule is accepted first, and its creation is audited, without the text
        let (status, created) = p
            .rest(
                Method::POST,
                &schedules,
                Some(&schedule_body(&text("schedule name"))),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        let schedule = created["scheduleId"].as_str().expect("id").to_owned();
        let creations: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'schedule.created'",
        )
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the audit trail");
        assert_eq!(creations, 1, "the accepted creation is audited");

        // schedules of the namespace
        set_limit(&p, "schedules", true, 0).await;
        let before = all_table_digests(&p.api.db.pool).await;
        let answer = p
            .rest(
                Method::POST,
                &schedules,
                Some(&schedule_body(&format!("{}-two", text("schedule name")))),
            )
            .await;
        refusals.push(Refusal {
            resource: "schedules",
            answer,
            reason: "SCHEDULE_LIMIT",
            limit: QuotaResource::Schedules.limit_name().to_owned(),
            before,
            after: all_table_digests(&p.api.db.pool).await,
        });

        // backfills of the namespace
        set_limit(&p, "schedule_backfills", true, 0).await;
        let before = all_table_digests(&p.api.db.pool).await;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let end = now - now % 3_600;
        let rfc = |s: i64| {
            time::OffsetDateTime::from_unix_timestamp(s)
                .expect("instant")
                .format(&time::format_description::well_known::Rfc3339)
                .expect("format")
        };
        let answer = p
            .rest(
                Method::POST,
                &format!("{schedules}/{schedule}/backfills"),
                Some(&json!({"start": rfc(end - 3 * 3_600), "end": rfc(end)})),
            )
            .await;
        refusals.push(Refusal {
            resource: "schedule_backfills",
            answer,
            reason: "BACKFILL_CONCURRENCY",
            limit: QuotaResource::ScheduleBackfills.limit_name().to_owned(),
            before,
            after: all_table_digests(&p.api.db.pool).await,
        });

        // batch jobs of the namespace
        set_limit(&p, "batch_jobs", true, 0).await;
        let before = all_table_digests(&p.api.db.pool).await;
        let answer = p.rest(Method::POST, &batches, Some(&job_body)).await;
        refusals.push(Refusal {
            resource: "batch_jobs",
            answer,
            reason: "BATCH_LIMIT",
            limit: QuotaResource::BatchJobs.limit_name().to_owned(),
            before,
            after: all_table_digests(&p.api.db.pool).await,
        });

        // batch jobs of the tenant: the namespace has room, the tenant has none
        sqlx::query("UPDATE quota_limits SET max_value = 1 WHERE resource = 'batch_jobs'")
            .execute(&p.api.db.pool)
            .await
            .expect("room in the namespace");
        set_limit(&p, "tenant_batch_jobs", false, 0).await;
        let before = all_table_digests(&p.api.db.pool).await;
        let answer = p.rest(Method::POST, &batches, Some(&job_body)).await;
        refusals.push(Refusal {
            resource: "tenant_batch_jobs",
            answer,
            reason: "BATCH_LIMIT",
            limit: QuotaResource::TenantBatchJobs.limit_name().to_owned(),
            before,
            after: all_table_digests(&p.api.db.pool).await,
        });

        // resets that wait for their source
        set_limit(&p, "pending_resets", true, 0).await;
        let run = started["runId"].as_str().expect("the run").to_owned();
        let mut worker = p.worker("observability-worker").await;
        worker.register(&["act.a", "act.b"]).await;
        p.tick().await;
        let _task = worker.one_task().await;
        let before = all_table_digests(&p.api.db.pool).await;
        let response = p
            .api
            .as_user(
                Method::POST,
                &format!("/runs/{run}/resets"),
                crate::support::worker::HUMAN,
                crate::support::worker::TENANT,
            )
            .header("idempotency-key", "observability-pending-key")
            .json(&json!({
                "sequence": 1,
                "reason": text("reset reason"),
                "sourcePolicy": "CANCEL_SOURCE",
                "acknowledgeReexecution": true
            }))
            .send()
            .await
            .expect("send");
        let status = response.status().as_u16();
        let body = response.text().await.expect("body");
        refusals.push(Refusal {
            resource: "pending_resets",
            answer: (status, serde_json::from_str(&body).unwrap_or(Value::Null)),
            reason: "PENDING_RESET_LIMIT",
            limit: QuotaResource::PendingResets.limit_name().to_owned(),
            before,
            after: all_table_digests(&p.api.db.pool).await,
        });

        let audit_text: String = sqlx::query_scalar(
            "SELECT coalesce(string_agg(t::text, ' '), '') FROM audit_records t",
        )
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the audit trail");
        (refusals, audit_text)
    });
    runtime.block_on(sink.shutdown(StdDuration::from_secs(5)));
    drop(guard);
    let (refusals, audit_text) = outcome;

    let mut limits = Vec::new();
    for refusal in &refusals {
        let Refusal {
            resource,
            answer,
            reason,
            limit,
            before,
            after,
        } = refusal;
        assert_refused(resource, answer, reason, limit, &canaries);
        // before work: no table gained, lost or changed a row (the audit trail, the receipts, the counters of a schedule among them)
        // (the rate limiter counts every request, a refused one included: that is the one table a request changes)
        let touched: Vec<String> = changed(before, after)
            .into_iter()
            .filter(|table| table != "quota_rate_buckets")
            .collect();
        assert_eq!(
            touched,
            Vec::<String>::new(),
            "{resource}: the refusal changed these tables"
        );
        // and it is counted once, in the series of its own resource
        assert_eq!(
            series(&page, "quota_exceeded_total", &[("limit", limit)]),
            Some(1.0),
            "{resource}: the refusal is counted once, in the series of its resource"
        );
        limits.push(limit.clone());
    }
    assert_absent("the audit trail", &audit_text, &canaries);
    assert_absent("a log line", &logs.text(), &canaries);
    assert_absent("a metric label", &page, &canaries);

    // the API's own spans were exported (the exporter listened), and an identifier that is in a URL is in no span
    let requests = collector.named("http.request");
    assert!(
        requests.len() >= refusals.len() + 3,
        "the requests of the test were exported as spans: {}",
        requests.len()
    );
    assert!(
        requests.iter().any(|span| span
            .attr("http.route")
            .is_some_and(|r| r.contains("business-ids"))),
        "the lookup of the identifier was exported, as its route template"
    );
    let wire = collector.bodies().join("\n");
    assert_absent("the OTLP export of the API's spans", &wire, &canaries);

    // the labels of the quota series are closed, and there is one series for each resource that refused
    let mut seen: Vec<String> = page
        .lines()
        .filter(|line| line.starts_with("quota_exceeded_total{"))
        .map(|line| {
            let inside =
                &line[line.find('{').expect("labels") + 1..line.rfind('}').expect("labels")];
            assert!(
                inside.split(',').all(|pair| pair.starts_with("limit=")),
                "an open label on `{line}`"
            );
            inside
                .trim_start_matches("limit=")
                .trim_matches('"')
                .to_owned()
        })
        .collect();
    seen.sort();
    limits.sort();
    assert_eq!(seen, limits, "one series for each resource that refused");
}
