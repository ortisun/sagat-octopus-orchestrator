//! The selector of a batch job by business-id prefix (`G2027`, `FR-018-022`, `FR-018-027`, `FR-018-028`): the runs of the
//! namespace whose business id starts with a literal prefix, counted by the preview and selected by the job, through the index
//! on the id of the runs. Real PostgreSQL, the real run service and executor.
//!
//! The ids are customer text, so the cases below stay in the tables that hold them; the only thing looked at is which runs a
//! prefix reaches.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::runs::StartRun;
use sagat_domain::batch::{Action, Filter, JobState, Selector};
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::execution::RunStatus;
use time::Duration;
use uuid::Uuid;

use sagat_application::batch_executor::ExecutorSettings;

use crate::batch_executor::{
    create, due, executor, executor_with, items, job, live, runs_started, select, service, settle,
    status_of, write,
};
use crate::business_ids::{business, key, start};
use crate::firing::{Fixture, fixture};
use crate::support::engine::{chain, ctx};

/// Starts a run under `id` in the namespace `payments` and returns it.
async fn under(f: &Fixture, id: &str) -> Uuid {
    start(
        f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a run under the id")
    .run
    .info
    .run_id
    .as_uuid()
}

fn by_prefix(prefix: &str) -> Selector {
    Selector::filter(Filter {
        business_id_prefix: Some(prefix.to_owned()),
        ..Filter::default()
    })
    .expect("selector")
}

async fn matched(f: &Fixture, prefix: &str) -> Vec<Uuid> {
    let mut sample = service(f)
        .preview(&f.w.admin, f.w.namespace.id(), &by_prefix(prefix), None)
        .await
        .expect("a preview by prefix is answered, not refused")
        .sample;
    sample.sort_unstable();
    sample
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn a_prefix_selects_the_runs_whose_business_id_starts_with_it_and_only_those() {
    let f = fixture().await;
    let one = under(&f, "order-1").await;
    let two = under(&f, "order-2").await;
    let thirty = under(&f, "order-30").await;
    let upper = under(&f, "Order-1").await;
    let short = under(&f, "orde").await;
    let other = under(&f, "other-1").await;
    let plain = runs_started(&f, 1).await[0];

    assert_eq!(
        matched(&f, "order-").await,
        sorted(vec![one, two, thirty]),
        "the runs that start with it"
    );
    assert_eq!(
        matched(&f, "order-1").await,
        [one],
        "a whole id is a prefix of itself, and of nothing shorter"
    );
    assert_eq!(matched(&f, "Order").await, [upper], "case is kept");
    assert_eq!(
        matched(&f, "orde").await.len(),
        4,
        "orde, order-1, order-2, order-30"
    );
    let everything_ordish = matched(&f, "o").await;
    assert_eq!(
        everything_ordish,
        sorted(vec![one, two, thirty, short, other])
    );
    assert!(
        !everything_ordish.contains(&plain),
        "a run with no business id is never selected by a prefix"
    );
}

#[tokio::test]
async fn the_characters_of_a_like_pattern_in_a_prefix_match_only_themselves() {
    let f = fixture().await;
    let dash = under(&f, "order-1").await;
    let underscore = under(&f, "order_1").await;
    let percent = under(&f, "order%1").await;
    let backslash = under(&f, "order\\1").await;
    let tilde = under(&f, "order~1").await;
    // `_` is one character and `%` any run of them in a pattern: here they are neither
    assert_eq!(matched(&f, "order_").await, [underscore]);
    assert_eq!(matched(&f, "order%").await, [percent]);
    assert_eq!(matched(&f, "order\\").await, [backslash]);
    assert_eq!(
        matched(&f, "order~").await,
        [tilde],
        "the last visible character ends a range"
    );
    assert_eq!(matched(&f, "%").await, Vec::<Uuid>::new(), "not everything");
    assert_eq!(
        matched(&f, "_").await,
        Vec::<Uuid>::new(),
        "not any one character"
    );
    assert_eq!(matched(&f, "\\").await, Vec::<Uuid>::new());
    assert_eq!(matched(&f, "order-").await, [dash]);
    assert_eq!(
        matched(&f, "order").await,
        sorted(vec![dash, underscore, percent, backslash, tilde])
    );
}

/// The three ways a prefix reaches runs, which are three statements (the list of runs, with `LIKE ... ESCAPE`; the preview and the
/// selection of a job, with a range of the index), must reach the same runs, also at the edges of the range: the characters of a
/// pattern, the last visible characters (`|`, `}`, `~`), a prefix that ends in one of them, one that is made of them, and ids that sit
/// just outside the range on either side. For each prefix the runs whose id starts with it are what the model says (`starts_with`),
/// and the list, the preview and the items of a job are each compared with it.
#[tokio::test]
async fn the_list_the_preview_and_the_job_reach_the_same_runs_for_a_prefix_at_every_edge() {
    use sagat_application::identity::PageRequest;
    use sagat_application::runs::RunFilter;

    let f = fixture().await;
    let ids = [
        "edge", "edge!", "edge$", "edge%", "edge%1", "edge_", "edge_1", "edge-", "edge\\",
        "edge\\1", "edge|", "edge|1", "edge}", "edge}1", "edge~", "edge~1", "edge~~", "edge~~1",
        "edgd", "edgf", "Edge", "~", "~~", "~a", "}", "}~", "|", "|}", "|}~",
    ];
    let mut made: Vec<(&str, Uuid)> = Vec::new();
    for id in ids {
        made.push((id, under(&f, id).await));
    }
    let prefixes = [
        "edge", "edge%", "edge_", "edge\\", "edge|", "edge}", "edge~", "edge~~", "edg", "ed", "~",
        "~~", "}", "|", "|}", "|}~", "%", "_", "\\", "nothing",
    ];
    live(&f);
    let exec = executor(&f);
    for (n, prefix) in prefixes.into_iter().enumerate() {
        let expected = sorted(
            made.iter()
                .filter(|(id, _)| id.starts_with(prefix))
                .map(|(_, run)| *run)
                .collect(),
        );
        assert!(
            expected.len() <= 20,
            "the sample of a preview holds twenty: {prefix:?}"
        );

        // the list of runs
        let mut listed: Vec<Uuid> =
            f.w.runs
                .list(
                    &f.w.admin,
                    "payments",
                    &RunFilter {
                        business_id_prefix: Some(prefix.to_owned()),
                        ..RunFilter::default()
                    },
                    &PageRequest::default(),
                )
                .await
                .expect("the list of runs by prefix")
                .items
                .iter()
                .map(|run| run.info.run_id.as_uuid())
                .collect();
        listed.sort_unstable();
        assert_eq!(listed, expected, "the list, for the prefix {prefix:?}");

        // the preview
        let preview = service(&f)
            .preview(&f.w.admin, f.w.namespace.id(), &by_prefix(prefix), None)
            .await
            .expect("the preview by prefix");
        assert_eq!(
            usize::try_from(preview.matched).expect("a count"),
            expected.len(),
            "the count of the preview, for the prefix {prefix:?}"
        );
        assert_eq!(
            sorted(preview.sample),
            expected,
            "the sample of the preview, for the prefix {prefix:?}"
        );

        // the items a job selects, through the statement of the selection
        let created = create(
            &f,
            write(Action::Cancel, by_prefix(prefix)),
            &format!("edge-prefix-job-{n}"),
        )
        .await;
        select(&exec, &created).await;
        let selected: Vec<Uuid> =
            sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1")
                .bind(created.id)
                .fetch_all(&f.w.db.pool)
                .await
                .expect("the items");
        assert_eq!(
            sorted(selected),
            expected,
            "the items of the job, for the prefix {prefix:?}"
        );
        // the job ends before the next one starts, so that the limit of active jobs is not what is tested
        for _ in 0..200 {
            if matches!(
                job(&f, created.id).await.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            ) {
                break;
            }
            f.clock.advance(Duration::seconds(1));
            exec.step(&due(&created), None).await.expect("step");
        }
        assert_eq!(job(&f, created.id).await.state, JobState::Completed);
    }
}

#[tokio::test]
async fn a_prefix_that_matches_nothing_counts_zero_and_a_job_over_it_completes_with_no_item() {
    let f = fixture().await;
    under(&f, "order-1").await;
    let preview = service(&f)
        .preview(&f.w.admin, f.w.namespace.id(), &by_prefix("invoice-"), None)
        .await
        .expect("a count of zero is an answer");
    assert_eq!(
        (preview.matched, preview.capped, preview.sample.len()),
        (0, false, 0)
    );

    live(&f);
    let exec = executor(&f);
    let created = create(
        &f,
        write(Action::Cancel, by_prefix("invoice-")),
        "by-prefix-none",
    )
    .await;
    select(&exec, &created).await;
    settle(&exec, &created).await;
    let done = job(&f, created.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.counts.selected, 0);
}

#[tokio::test]
async fn a_prefix_reaches_only_the_runs_of_its_own_namespace_and_tenant() {
    let f = fixture().await;
    let own = under(&f, "order-1").await;

    // the same prefix in another namespace of the tenant
    f.w.add_namespace("billing").await;
    let doc = chain("billed");
    let created =
        f.w.defs
            .create_workflow(
                &f.w.admin,
                "billing",
                "billed",
                &serde_json::to_vec(&doc).expect("json"),
                &format!("create-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("workflow");
    f.w.defs
        .publish(
            &f.w.admin,
            created.workflow.id(),
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("publish");
    let elsewhere =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "billing",
                    workflow: created.workflow.id(),
                    version: None,
                    input: b"{}",
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business("order-2", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &ctx(),
            )
            .await
            .expect("a run of another namespace")
            .run
            .info
            .run_id
            .as_uuid();

    // and in another tenant
    let foreign_workflow = f.w.publish_as_other_tenant(chain("foreign")).await;
    let foreign =
        f.w.runs
            .start_with_options(
                &f.w.other_admin,
                &StartRun {
                    namespace: "payments",
                    workflow: foreign_workflow,
                    version: None,
                    input: b"{}",
                    correlation_id: None,
                    deadline: None,
                    key: &key(),
                },
                business("order-3", ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &ctx(),
            )
            .await
            .expect("a run of another tenant")
            .run
            .info
            .run_id
            .as_uuid();

    assert_eq!(matched(&f, "order-").await, [own]);

    live(&f);
    let exec = executor(&f);
    let created = create(
        &f,
        write(Action::Cancel, by_prefix("order-")),
        "by-prefix-scope",
    )
    .await;
    select(&exec, &created).await;
    settle(&exec, &created).await;
    let selected: Vec<Uuid> =
        sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1")
            .bind(created.id)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("items");
    assert_eq!(selected, [own]);
    for untouched in [elsewhere, foreign] {
        assert_ne!(status_of(&f, untouched).await, "CANCELLED");
    }
}

#[tokio::test]
async fn a_job_by_prefix_takes_its_runs_in_pages_and_applies_the_action_to_those_runs_only() {
    let f = fixture().await;
    let mut chosen = Vec::new();
    for n in 0..5 {
        chosen.push(under(&f, &format!("batch-{n}")).await);
    }
    let kept = under(&f, "keep-1").await;
    let plain = runs_started(&f, 1).await[0];
    live(&f);
    let exec = executor_with(
        &f,
        ExecutorSettings {
            select_page: 2,
            ..ExecutorSettings::default()
        },
    );
    let created = create(
        &f,
        write(Action::Cancel, by_prefix("batch-")),
        "by-prefix-pages",
    )
    .await;
    select(&exec, &created).await;
    for _ in 0..50 {
        f.clock.advance(Duration::seconds(1));
        if matches!(
            job(&f, created.id).await.state,
            JobState::Completed | JobState::Failed
        ) {
            break;
        }
        exec.step(&due(&created), None).await.expect("step");
    }
    let done = job(&f, created.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!((done.counts.selected, done.counts.succeeded), (5, 5));
    let taken: Vec<Uuid> =
        sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1 ORDER BY ordinal")
            .bind(created.id)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("items");
    assert_eq!(taken, sorted(chosen.clone()), "by id, in order of ordinal");
    assert_eq!(items(&f, created.id).await.len(), 5);
    for run in chosen {
        assert_eq!(status_of(&f, run).await, "CANCELLED");
    }
    for untouched in [kept, plain] {
        assert_ne!(status_of(&f, untouched).await, "CANCELLED");
    }
}

#[tokio::test]
async fn the_prefix_composes_with_the_other_filters_of_the_selector() {
    let f = fixture().await;
    let ended = under(&f, "mix-1").await;
    let open = under(&f, "mix-2").await;
    let other = under(&f, "else-1").await;
    f.end(ended, "CANCELLED").await;
    f.end(other, "CANCELLED").await;
    let selector = Selector::filter(Filter {
        business_id_prefix: Some("mix-".to_owned()),
        statuses: vec![RunStatus::Cancelled],
        workflow_id: Some(f.workflow.as_uuid()),
        ..Filter::default()
    })
    .expect("selector");
    let preview = service(&f)
        .preview(&f.w.admin, f.w.namespace.id(), &selector, None)
        .await
        .expect("preview");
    assert_eq!(
        preview.sample,
        [ended],
        "the prefix and the status, together"
    );
    assert_ne!(preview.sample, [open]);
}

#[tokio::test]
async fn a_run_is_selected_by_the_id_it_carries_whether_or_not_its_binding_is_still_kept() {
    let f = fixture().await;
    let run = under(&f, "kept-1").await;
    f.end(run, "SUCCEEDED").await;
    // the binding of a closed holder is purged once its window has passed: the run keeps the id it was started under
    sqlx::query("DELETE FROM business_ids WHERE business_id = 'kept-1'")
        .execute(&f.w.db.pool)
        .await
        .expect("purge the binding");
    assert_eq!(matched(&f, "kept-").await, [run]);
}

#[tokio::test]
async fn a_selective_prefix_is_answered_from_the_index_on_the_business_id_of_the_runs() {
    use sagat_infra_postgres::{PREVIEW_BY_PREFIX_SQL, SELECTION_BY_PREFIX_SQL};
    let f = fixture().await;
    let model = under(&f, "model-1").await;
    // a namespace with enough runs to have statistics: copies of one run under ids of their own
    sqlx::query(
        "WITH made AS (SELECT gen_random_uuid() AS id, g FROM generate_series(1, 6000) g) \
         INSERT INTO runs SELECT (jsonb_populate_record(r, \
             jsonb_build_object('id', made.id, 'root_run_id', made.id, 'business_id', 'bulk-' || made.g))).* \
         FROM runs r, made WHERE r.id = $1",
    )
    .bind(model)
    .execute(&f.w.db.pool)
    .await
    .expect("copy the run");
    sqlx::query("ANALYZE runs")
        .execute(&f.w.db.pool)
        .await
        .expect("statistics");
    let tenant: Uuid = sqlx::query_scalar("SELECT tenant_id FROM runs WHERE id = $1")
        .bind(model)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("tenant");
    let namespace = f.w.namespace.id().as_uuid();
    // a prepared statement lives in its session: one connection for all of it
    let mut session = f.w.db.pool.acquire().await.expect("a connection");
    for (name, statement, last) in [
        ("selection", SELECTION_BY_PREFIX_SQL, "NULL::uuid, 500"),
        ("preview", PREVIEW_BY_PREFIX_SQL, "501"),
    ] {
        // the types of the parameters, in order, then what a job by the prefix `bulk-5999` binds
        let types = "uuid, uuid, uuid, uuid, text[], text, uuid, timestamptz, timestamptz, uuid[], timestamptz, text, text";
        let cursor_type = if name == "selection" {
            ", uuid, bigint"
        } else {
            ", bigint"
        };
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PREPARE by_prefix({types}{cursor_type}) AS {statement}"
        )))
        .execute(&mut *session)
        .await
        .expect("prepare the statement");
        let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (COSTS OFF) EXECUTE by_prefix('{tenant}', '{namespace}', NULL, NULL, '{{}}', NULL, NULL, NULL, NULL, NULL, \
             now() + interval '1 hour', 'bulk-5999', 'bulk-599:', {last})"
        )))
        .fetch_all(&mut *session)
        .await
        .expect("explain");
        let plan = plan.join("\n");
        assert!(
            plan.contains("runs_business_id_idx"),
            "the {name} of a prefix that picks a few of the {} runs reads the index of the business ids, not every run:\n{plan}",
            6001
        );
        if name == "preview" {
            // it counts and samples: it stops at its limit, with no sort (the selection pages by run id, and does sort: G2089)
            assert!(
                !plan.contains("Sort"),
                "the preview does not sort what the prefix reaches:\n{plan}"
            );
        }
        sqlx::query("DEALLOCATE by_prefix")
            .execute(&mut *session)
            .await
            .expect("deallocate");
    }

    // A WIDE prefix, which matches nearly every run: the preview only counts (up to the most it reads) and samples, so it takes the
    // first matches it finds and stops, with no sort of every match (D2191). The selection of a job cannot do the same, it
    // pages by run id (G2089).
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PREPARE wide_preview({}) AS {PREVIEW_BY_PREFIX_SQL}",
        "uuid, uuid, uuid, uuid, text[], text, uuid, timestamptz, timestamptz, uuid[], timestamptz, text, text, bigint"
    )))
    .execute(&mut *session)
    .await
    .expect("prepare the preview");
    let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (COSTS OFF) EXECUTE wide_preview('{tenant}', '{namespace}', NULL, NULL, '{{}}', NULL, NULL, NULL, NULL, NULL, \
         now() + interval '1 hour', 'bulk-', 'bulk.', 501)"
    )))
    .fetch_all(&mut *session)
    .await
    .expect("explain");
    let plan = plan.join("\n");
    // the planner may read a few hundred rows of the table (nearly every run matches, so the first matches come at once) or the index:
    // what it must not do is read and sort every run the prefix reaches to hand back the first few
    assert!(
        plan.starts_with("Limit"),
        "the preview of a wide prefix stops at its limit:\n{plan}"
    );
    assert!(
        !plan.contains("Sort"),
        "and does not sort every run the prefix reaches to count and sample a few:\n{plan}"
    );
}
