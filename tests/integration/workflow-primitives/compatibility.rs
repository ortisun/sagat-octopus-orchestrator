//! Upgrade and rollback compatibility of the workflow primitives against real PostgreSQL
//! (T014-041, FR-014-016, NFR-014-004, SC-014-005).
//!
//! Four boundaries are exercised here; the version/capability matrix in `docs/sdk-compatibility.md`
//! names the test behind each cell:
//!
//! 1. an active run that predates the primitives keeps running through the migrations and the new
//!    engine, with its journal untouched;
//! 2. worker protocol minors `1.0`..`1.4` negotiate and are never offered scoped work, while a `1.5`
//!    worker is, concurrently, on the same run;
//! 3. documents that use only pre-primitive nodes validate under the additive schema and publish
//!    identically through every reader;
//! 4. a reader that does not support a capability refuses it with a typed reason, stores nothing and
//!    leaves an active run that uses it untouched until a capable reader returns.

use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{IrCodec, IrError, IrIssue, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::worker::{Capabilities, WorkerService, WorkerSettings};
use sagat_domain::execution::{RunId, RunStatus};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{MIGRATOR, PgRunStore, migrate};
use sagat_scheduler::{Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use super::identity_storage::insert_legacy_root_run;
use super::scoped_worker::{self, Fixture};
use crate::support::engine::{
    World, activity, chain, child_node, condition_node, document, effect_node, end_node,
    parallel_node, saga_node, timer_node, wait_node, with, world,
};

/// The last migration before the scoped-instance storage foundation (the first of feature 014).
const BEFORE: i64 = 20_261_026_000_000;
const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";

// ------------------------------------------------------------------------------------ active old run

/// A run started by the pre-primitive engine, on a definition with no primitives, is carried through
/// the 014 migrations and finished by the 014 engine: same pinned version, same journal prefix, a
/// consistent replay, and its root steps behave as the empty-path instance.
#[tokio::test]
async fn an_active_pre_014_run_survives_the_migrations_and_finishes_on_the_new_engine() {
    let w = world().await;
    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("return to the schema that predates the primitives");
    let (run, version) = insert_legacy_root_run(&w, "legacy-active-run").await;
    let tenant = w.admin.tenant_id().as_uuid();

    let old_document: Vec<u8> = sqlx::query_scalar(
        "SELECT document FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant)
    .bind(version.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("pinned document before the upgrade");
    let journal_before = journal_rows(&w, run).await;
    assert_eq!(
        journal_before.len(),
        2,
        "the pre-014 run has its two events"
    );

    migrate(&w.db.pool).await.expect("apply the 014 migrations");

    // The pinned version is byte-identical and the old events are not reinterpreted.
    let pinned: (Uuid, Vec<u8>) = sqlx::query_as(
        "SELECT workflow_version_id, \
                (SELECT document FROM workflow_versions v \
                  WHERE v.tenant_id = r.tenant_id AND v.id = r.workflow_version_id) \
         FROM runs r WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("pinned version after the upgrade");
    assert_eq!(pinned.0, version.as_uuid(), "the run keeps its version");
    assert_eq!(pinned.1, old_document, "the definition is not rewritten");
    assert_eq!(
        journal_rows(&w, run).await,
        journal_before,
        "the migration does not touch history"
    );
    assert_eq!(w.run_status(run).await, RunStatus::Running);
    w.assert_consistent(run).await;

    // The 014 engine finishes it through the ordinary worker protocol.
    let session = w.session(&["act.a", "act.b"]).await;
    let a = w.one_task(&session).await;
    assert_eq!(a.instance.node().as_str(), "a");
    assert!(
        a.instance.path().is_root(),
        "an old step is a root instance"
    );
    w.complete(&session, &a, &json!({"approved": true}))
        .await
        .expect("complete the pre-014 step");
    let b = w.one_task(&session).await;
    assert_eq!(b.instance.node().as_str(), "b");
    let ack = w
        .complete(&session, &b, &json!({}))
        .await
        .expect("complete the routed step");
    assert_eq!(ack.run_status, RunStatus::Succeeded);
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;

    let journal_after = journal_rows(&w, run).await;
    assert_eq!(
        journal_after[..journal_before.len()],
        journal_before[..],
        "the new events extend the old prefix"
    );
    assert!(
        journal_after.len() > journal_before.len(),
        "the new engine appended its events"
    );

    // A run started after the upgrade from the same old definition behaves the same way.
    let fresh = w
        .start(
            sqlx::query_scalar::<_, Uuid>(
                "SELECT workflow_id FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant)
            .bind(version.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .map(sagat_domain::definition::WorkflowId::new)
            .expect("workflow of the old version"),
            &json!({}),
        )
        .await;
    let a = w.one_task(&session).await;
    w.complete(&session, &a, &json!({})).await.expect("a");
    let b = w.one_task(&session).await;
    let ack = w.complete(&session, &b, &json!({})).await.expect("b");
    assert_eq!(ack.run_status, RunStatus::Succeeded);
    w.assert_consistent(fresh).await;
}

async fn journal_rows(w: &World, run: RunId) -> Vec<(i64, String, Option<String>, Value)> {
    sqlx::query(
        "SELECT seq, event_type, node, detail FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("journal rows")
    .into_iter()
    .map(|row| {
        (
            row.get("seq"),
            row.get("event_type"),
            row.get("node"),
            row.get("detail"),
        )
    })
    .collect()
}

// -------------------------------------------------------------------------------- old worker minors

fn caps(activities: &[&str], offer: &str) -> Capabilities {
    Capabilities {
        activities: activities.iter().map(|a| (*a).to_owned()).collect(),
        languages: vec!["rust".into()],
        pools: Vec::new(),
        max_inflight: 8,
        protocol_version: offer.into(),
    }
}

/// Every protocol-1 minor from `1` (and its alias `1.0`) to `1.4` is accepted at exactly the version
/// it offers by the production scoped-capable adapter, and none of them is offered scoped work that
/// is ready at the same time as a `1.5` worker leases it. The old worker even advertises the scoped
/// activity names, so a flattened scoped task would match it.
#[tokio::test]
async fn old_worker_minors_negotiate_and_are_never_offered_scoped_work() {
    let f = scoped_worker::fixture("compat-old-minors").await;
    let tenant = f.world.admin.tenant_id().as_uuid();

    for (offer, accepted) in [
        ("1", "1"),
        ("1.0", "1"),
        ("1.1", "1.1"),
        ("1.2", "1.2"),
        ("1.3", "1.3"),
        ("1.4", "1.4"),
    ] {
        let registered = f
            .workers
            .register(
                &f.world.worker,
                "payments",
                &format!("old-minor-{}-{}", offer, Uuid::now_v7()),
                &caps(&["scoped.a", "scoped.b"], offer),
            )
            .await
            .unwrap_or_else(|why| panic!("offer {offer}: {why}"));
        assert_eq!(registered.accepted_protocol_version, accepted, "{offer}");
        let tasks = f
            .workers
            .poll(
                &f.world.worker,
                registered.worker_id,
                registered.session_token.expose(),
                8,
                Some(TRACE),
            )
            .await
            .expect("an old worker polls");
        assert!(
            tasks.is_empty(),
            "a {offer} worker must not receive scoped work, got {} task(s)",
            tasks.len()
        );
    }

    // Nothing was leased, retried or attempted by the old workers.
    let scoped: Vec<(String, Option<String>, i32)> = sqlx::query_as(
        "SELECT status, leased_by, attempt FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND cardinality(instance_path_nodes) > 0 \
         ORDER BY instance_path_indices",
    )
    .bind(tenant)
    .bind(f.run.as_uuid())
    .fetch_all(&f.world.db.pool)
    .await
    .expect("scoped steps after the old polls");
    assert_eq!(scoped.len(), 2);
    for (status, leased_by, attempt) in &scoped {
        assert_eq!(status, "READY");
        assert!(leased_by.is_none(), "no old worker holds a scoped lease");
        assert_eq!(*attempt, 0, "no attempt was started");
    }

    // The same ready work is leased to a 1.5 worker.
    let tasks = scoped_worker::poll(&f, 8).await;
    let mut indices: Vec<u32> = tasks.iter().map(scoped_worker::index).collect();
    indices.sort_unstable();
    assert_eq!(indices, [0, 1], "the 1.5 worker leases both scoped tasks");
    assert!(tasks.iter().all(|task| !task.instance.path().is_root()));
}

/// A store that has not proven scoped claim and report caps every offer at `1.4`, so even a worker that
/// offers `1.5` (or a future minor) cannot be handed scoped identity it cannot echo.
#[tokio::test]
async fn a_root_only_adapter_caps_a_newer_offer_at_1_4_and_withholds_scoped_work() {
    let f = scoped_worker::fixture("compat-root-only-cap").await;
    for offer in ["1.5", "1.99"] {
        let registered = f
            .world
            .workers
            .register(
                &f.world.worker,
                "payments",
                &format!("capped-{offer}-{}", Uuid::now_v7()),
                &caps(&["scoped.a", "scoped.b"], offer),
            )
            .await
            .expect("register against the root-only adapter");
        assert_eq!(registered.accepted_protocol_version, "1.4", "{offer}");
        let tasks = f
            .world
            .workers
            .poll(
                &f.world.worker,
                registered.worker_id,
                registered.session_token.expose(),
                8,
                Some(TRACE),
            )
            .await
            .expect("poll");
        assert!(
            tasks.is_empty(),
            "{offer} capped at 1.4 gets no scoped task"
        );
    }
    for offer in ["2", "2.0", "0.9", "1.x", "1.05", ""] {
        let refused = f
            .world
            .workers
            .register(
                &f.world.worker,
                "payments",
                &format!("refused-{}", Uuid::now_v7()),
                &caps(&["scoped.a"], offer),
            )
            .await;
        assert!(
            matches!(refused, Err(AppError::NotAllowed(_) | AppError::Invalid(_))),
            "offer {offer:?} must be refused, got {refused:?}"
        );
    }
}

// ----------------------------------------------------------------------------- additive IR v1 reads

fn payment_saga() -> Value {
    serde_json::from_str(include_str!("../../../examples/payment-saga.workflow.json"))
        .expect("the pre-014 example is JSON")
}

/// Documents that use only the nodes that existed before the primitives. Every shape of the
/// pre-014 vocabulary that has a builder in the shared support is here.
fn pre_primitive_documents() -> Vec<(&'static str, Value)> {
    vec![
        ("payment-saga-example", payment_saga()),
        ("activity-chain", chain("compat-chain")),
        (
            "timer-then-activity",
            document(
                "compat-timer",
                "pause",
                vec![
                    with(timer_node("pause", 1_000), "next", json!("work")),
                    activity("work"),
                ],
            ),
        ),
        (
            "wait-for-event",
            document(
                "compat-wait",
                "hold",
                vec![
                    with(
                        wait_node("hold", "payment.settled", "input.orderId", 60_000, None),
                        "next",
                        json!("work"),
                    ),
                    activity("work"),
                ],
            ),
        ),
        (
            "condition",
            document(
                "compat-condition",
                "check",
                vec![
                    condition_node("check", "input.big", "x", "y"),
                    activity("x"),
                    activity("y"),
                ],
            ),
        ),
        (
            "parallel",
            document(
                "compat-parallel",
                "fan",
                vec![
                    with(
                        with(parallel_node("fan", &["l", "r"], "all"), "next", json!("j")),
                        "join",
                        json!("all"),
                    ),
                    activity("l"),
                    activity("r"),
                    activity("j"),
                ],
            ),
        ),
        (
            "child",
            document(
                "compat-child",
                "kid",
                vec![child_node("kid", "other-workflow", None)],
            ),
        ),
        (
            "saga",
            document(
                "compat-saga",
                "tx",
                vec![
                    saga_node("tx", "reserve", None),
                    effect_node("reserve", Some("submit"), true),
                    effect_node("submit", None, false),
                ],
            ),
        ),
    ]
}

/// A pre-primitive document validates under the additive schema, and the publication reader and the
/// structured reader give the same canonical bytes and digest, so the digest stored with an old
/// version stays valid.
#[test]
fn documents_with_only_pre_primitive_nodes_validate_identically_in_every_reader() {
    for (label, document) in pre_primitive_documents() {
        let bytes = serde_json::to_vec(&document).expect("document bytes");
        let published = JsonIrCodec
            .parse(&bytes)
            .unwrap_or_else(|why| panic!("{label} must still publish: {why}"));
        let structured = JsonIrCodec
            .parse_structured(&bytes)
            .unwrap_or_else(|why| panic!("{label} must still validate structurally: {why}"));
        assert_eq!(published.canonical, structured.canonical, "{label}");
        assert_eq!(published.digest, structured.digest, "{label}");
        assert!(
            published.canonical.windows(7).all(|w| w != b"\"bodies"),
            "{label}: no primitive field is introduced by reading an old document"
        );
    }
}

/// The canonical digest of documents that predate the primitives is a stable identity: it is the value
/// the pre-014 build (commit 07d673f) computed for the same document. Pinned versions that carry one of
/// these digests remain verifiable after the upgrade.
#[test]
fn pre_primitive_documents_keep_the_digest_the_pre_014_build_computed() {
    let expected: &[(&str, &str)] = PRE_014_DIGESTS;
    assert!(
        !expected.is_empty(),
        "the pinned digests were recorded from the pre-014 build"
    );
    let documents: std::collections::BTreeMap<_, _> =
        pre_primitive_documents().into_iter().collect();
    for (label, digest) in expected {
        let document = documents
            .get(label)
            .unwrap_or_else(|| panic!("document {label}"));
        let parsed = JsonIrCodec
            .parse(&serde_json::to_vec(document).expect("bytes"))
            .expect("parse");
        assert_eq!(
            parsed.digest.to_hex(),
            *digest,
            "{label} changed its canonical digest since the pre-014 build"
        );
    }
}

/// Digests that the build at commit `07d673f` (the last one before any primitive code; it changes
/// no file under `crates/`) computed for exactly these documents. They were produced by parsing the
/// serialized documents of [`pre_primitive_documents`] with that build's `JsonIrCodec::parse`, in a
/// detached worktree, and compared with the values this build prints (all eight were equal).
const PRE_014_DIGESTS: &[(&str, &str)] = &[
    (
        "payment-saga-example",
        "e757aa04ac3fa293f29bb2aef04063f449974fef5dc3c710d0d99717213e0c35",
    ),
    (
        "activity-chain",
        "0215e844918453e8fb9786aaafcc7fb7a5914e9b9aebc3285d6de57f89cf259c",
    ),
    (
        "timer-then-activity",
        "9061f3b4f1985caff503992f39110aa6de992245fbeb275354b023c1813649e4",
    ),
    (
        "wait-for-event",
        "9e5609b823ee825ad4be65f9a4af0364dae960ebad45001e611f185e8bf07013",
    ),
    (
        "condition",
        "9c956f858e300a39eb80434c1b681d5c90566724e5f0c190e50ef4996ba9aaf0",
    ),
    (
        "parallel",
        "898aebac8242ecb6746a6310ec991bac3e1101a9052ea0c06b1cf79258131385",
    ),
    (
        "child",
        "15333ab838b1eb28ebd1ccb2f534c16bd967e2d03cde38598c28e16c239600fb",
    ),
    (
        "saga",
        "e33aa52bac3927227cb5ac25d504f5b09478b5f34bba0517c6e6511ad0c63246",
    ),
];

// ------------------------------------------------------------------------- readers without support

/// Combinations that the production reader does not enable are refused with a typed reason, before
/// anything is stored. The HTTP mapping of each reason is `details.reason = UNSUPPORTED_NODE`
/// (`collections_http` and `loops_http` assert it on the wire).
#[tokio::test]
async fn combinations_the_reader_does_not_enable_are_refused_with_a_typed_reason_and_store_nothing()
{
    let w = world().await;
    let body = json!({"entrypoint": "work", "nodes": [
        {"id": "work", "type": "activity", "activity": "line.process", "next": "returned"},
        {"id": "returned", "type": "return", "result": "steps.work.output"}
    ]});
    let collection = |name: &str, nodes: Value, extra: &[(&str, Value)]| {
        let mut doc = json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "items",
            "bodies": {"line": body.clone()},
            "nodes": nodes
        });
        for (key, value) in extra {
            doc[*key] = value.clone();
        }
        doc
    };
    let for_each = json!({"id": "items", "type": "forEach", "items": "input.items", "body": "line",
        "result": "bodyResult", "maxItems": 3, "maxConcurrency": 1, "next": "done"});
    let done = json!({"id": "done", "type": "end"});

    struct Case {
        label: &'static str,
        document: Value,
        /// `Ok(node kind)` for `IrError::Unsupported`, `Err(field)` for `UnsupportedDeclaration`.
        refusal: Result<&'static str, &'static str>,
    }
    let cases = vec![
        Case {
            label: "collection followed by an activity",
            document: collection(
                "compat-coll-activity",
                json!([
                    with(for_each.clone(), "next", json!("after")),
                    activity("after")
                ]),
                &[],
            ),
            refusal: Ok("activity"),
        },
        Case {
            label: "collection with a trigger",
            document: collection(
                "compat-coll-trigger",
                json!([for_each.clone(), done.clone()]),
                &[("triggers", json!([{"kind": "manual"}]))],
            ),
            refusal: Err("triggers"),
        },
        Case {
            label: "collection with a chain policy",
            document: collection(
                "compat-coll-chain",
                json!([for_each.clone(), done.clone()]),
                &[("chainPolicy", json!({"maxSegments": 2, "timeoutMs": 1_000}))],
            ),
            refusal: Err("chainPolicy"),
        },
        Case {
            label: "approval together with parallel execution",
            document: document(
                "compat-approval-parallel",
                "review",
                vec![
                    json!({"id": "review", "type": "approval", "timeoutMs": 86_400_000,
                           "policy": {"roles": ["operator"]},
                           "onApprove": "fan", "onReject": "fan", "onExpire": "fan"}),
                    with(parallel_node("fan", &["l", "r"], "all"), "next", json!("j")),
                    activity("l"),
                    activity("r"),
                    activity("j"),
                ],
            ),
            refusal: Ok("parallel"),
        },
        // Declared state together with an approval used to be refused here; D295 enables it, and
        // `approval_combinations` proves the combination.
    ];

    for case in cases {
        let name = case.document["metadata"]["name"]
            .as_str()
            .expect("name")
            .to_owned();
        let outcome = w.try_publish_as(&w.admin, case.document).await;
        match (&case.refusal, &outcome) {
            (Ok(kind), Err(AppError::InvalidDefinition(error))) => match &**error {
                IrError::Unsupported { kind: found, .. } => {
                    assert_eq!(found, kind, "{}", case.label);
                }
                other => panic!(
                    "{}: expected an unsupported node, got {other:?}",
                    case.label
                ),
            },
            (Err(field), Err(AppError::InvalidDefinition(error))) => match &**error {
                IrError::UnsupportedDeclaration { field: found, .. } => {
                    assert_eq!(found, field, "{}", case.label);
                }
                other => panic!(
                    "{}: expected an unsupported declaration, got {other:?}",
                    case.label
                ),
            },
            _ => panic!("{}: expected a typed refusal, got {outcome:?}", case.label),
        }
        let stored: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM workflow_versions v JOIN workflows w \
               ON w.tenant_id = v.tenant_id AND w.id = v.workflow_id \
             WHERE v.tenant_id = $1 AND w.name = $2",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(&name)
        .fetch_one(&w.db.pool)
        .await
        .expect("versions stored for the refused workflow");
        assert_eq!(
            stored, 0,
            "{}: a refused document stores no version",
            case.label
        );
    }
}

/// A reader of an earlier release. A pre-014 schema has `additionalProperties: false` at the top
/// level and no node type for the primitives, so it refuses the capability fields as a schema
/// violation. That was observed by running the build at commit `07d673f` on the nine builder-golden
/// documents (see `docs/sdk-compatibility.md`); this type reproduces the answer, it is not that build. It is injected where production injects its codec, the definition cache of the store.
#[derive(Debug)]
struct ReaderWithoutPrimitives;

impl IrCodec for ReaderWithoutPrimitives {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        IrCodec::check_draft(&JsonIrCodec, document)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        let value: Value = serde_json::from_slice(document).map_err(|_| IrError::Malformed)?;
        let unexpected: Vec<IrIssue> = ["bodies", "state", "chainPolicy", "limits"]
            .into_iter()
            .filter(|field| value.get(*field).is_some())
            .map(|field| IrIssue {
                path: String::new(),
                message: format!(
                    "Additional properties are not allowed ('{field}' was unexpected)"
                ),
            })
            .collect();
        if unexpected.is_empty() {
            JsonIrCodec.parse(document)
        } else {
            Err(IrError::Schema(unexpected))
        }
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        IrCodec::diff(&JsonIrCodec, from, to)
    }
}

type OldWorkers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

async fn run_fingerprint(f: &Fixture) -> (String, i64, String) {
    let tenant = f.world.admin.tenant_id().as_uuid();
    let run: String = sqlx::query_scalar(
        "SELECT to_jsonb(r)::text FROM runs r WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(tenant)
    .bind(f.run.as_uuid())
    .fetch_one(&f.world.db.pool)
    .await
    .expect("run row");
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id = $1 AND run_id = $2")
            .bind(tenant)
            .bind(f.run.as_uuid())
            .fetch_one(&f.world.db.pool)
            .await
            .expect("event count");
    let steps: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(to_jsonb(s)::text, '|' \
                ORDER BY s.node, s.instance_path_nodes, s.instance_path_indices), '') \
         FROM steps s WHERE s.tenant_id = $1 AND s.run_id = $2",
    )
    .bind(tenant)
    .bind(f.run.as_uuid())
    .fetch_one(&f.world.db.pool)
    .await
    .expect("steps");
    (run, events, steps)
}

/// Rolling the reader back below the capability must not damage runs that already use it. An engine
/// whose reader refuses the pinned definition leaves the run row, its steps and its journal exactly as
/// they were (no claim, no failure, no cancellation), and the capable reader resumes the same run.
#[tokio::test]
async fn a_reader_without_the_capability_leaves_an_active_structured_run_untouched_until_it_returns()
 {
    let f = scoped_worker::fixture("compat-rolled-back-reader").await;
    let before = run_fingerprint(&f).await;
    let pinned: Vec<u8> = sqlx::query_scalar(
        "SELECT v.document FROM runs r JOIN workflow_versions v \
           ON v.tenant_id = r.tenant_id AND v.id = r.workflow_version_id \
         WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(f.world.admin.tenant_id().as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.world.db.pool)
    .await
    .expect("pinned structured document");
    assert!(
        matches!(
            ReaderWithoutPrimitives.parse(&pinned),
            Err(IrError::Schema(_))
        ),
        "the rolled-back reader must really refuse the pinned definition"
    );

    let old_store = PgRunStore::new(
        f.world.db.pool.clone(),
        DefinitionCache::new(Arc::new(ReaderWithoutPrimitives), 8),
    )
    .with_scoped_worker_protocol();
    let old_workers: OldWorkers = WorkerService::new(
        old_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(f.world.keys.cipher());
    let registered = old_workers
        .register(
            &f.world.worker,
            "payments",
            &format!("rolled-back-{}", Uuid::now_v7()),
            &caps(&["scoped.a", "scoped.b"], "1.5"),
        )
        .await
        .expect("the rolled-back build still registers workers");
    let polled = old_workers
        .poll(
            &f.world.worker,
            registered.worker_id,
            registered.session_token.expose(),
            8,
            Some(TRACE),
        )
        .await;
    // A run this build cannot read is skipped for the poll (G460): the poll succeeds with nothing.
    let tasks = polled.expect("an unreadable run is skipped, it does not fail the poll");
    assert!(
        tasks.is_empty(),
        "a reader that cannot read the definition must not lease its steps"
    );

    let scheduler = Scheduler::new(
        old_store,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(f.placement.clone());
    let report = scheduler.tick().await;
    assert_eq!(
        report.approvals_opened, 0,
        "the rolled-back scheduler opened nothing: {report:?}"
    );

    assert_eq!(
        run_fingerprint(&f).await,
        before,
        "the rolled-back reader changed the run, its steps or its journal"
    );
    assert_eq!(
        f.world.run_status(f.run).await,
        RunStatus::Running,
        "the run is not failed or cancelled by a reader that cannot read it"
    );

    // The capable reader comes back and the same run is served: its scoped work is leased.
    let tasks = scoped_worker::poll(&f, 8).await;
    assert_eq!(tasks.len(), 2, "the capable reader resumes the same run");
}

/// G460: runs this build cannot read, however many are ahead of a readable one in the poll's
/// candidate list, neither fail the poll nor use up the failures it tolerates (three), so the
/// readable run is still served. Before the fix the fourth candidate was never reached and the poll
/// answered `Internal`.
#[tokio::test]
async fn runs_a_reader_cannot_read_do_not_starve_the_ones_it_can() {
    let w = world().await;
    let mut unreadable = Vec::new();
    for n in 0..4 {
        // `limits` is a field the rolled-back reader refuses (see `ReaderWithoutPrimitives`).
        let mut doc = document(
            &format!("compat-newer-{n}"),
            "a",
            vec![with(
                json!({ "id": "a", "type": "activity", "activity": "compat.work" }),
                "next",
                json!("done"),
            ), end_node("done")],
        );
        doc["limits"] = json!({ "maxInstances": 20 });
        let (workflow, _) = w.publish(doc).await;
        unreadable.push(w.start(workflow, &json!({})).await);
    }
    let (readable_workflow, _) = w
        .publish(document(
            "compat-readable",
            "a",
            vec![with(
                json!({ "id": "a", "type": "activity", "activity": "compat.work" }),
                "next",
                json!("done"),
            ), end_node("done")],
        ))
        .await;
    let readable = w.start(readable_workflow, &json!({})).await;

    let old_store = PgRunStore::new(
        w.db.pool.clone(),
        DefinitionCache::new(Arc::new(ReaderWithoutPrimitives), 8),
    );
    let old_workers: OldWorkers = WorkerService::new(
        old_store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(w.keys.cipher());
    let registered = old_workers
        .register(
            &w.worker,
            "payments",
            &format!("rolled-back-{}", Uuid::now_v7()),
            &caps(&["compat.work"], "1.4"),
        )
        .await
        .expect("the rolled-back build still registers workers");
    let tasks = old_workers
        .poll(
            &w.worker,
            registered.worker_id,
            registered.session_token.expose(),
            8,
            Some(TRACE),
        )
        .await
        .expect("unreadable runs are skipped, they do not fail the poll");
    assert_eq!(tasks.len(), 1, "only the readable run's step is leased");
    assert_eq!(tasks[0].run_id, readable);
    for run in unreadable {
        assert_eq!(
            w.step_status(run, "a").await,
            Some(sagat_domain::execution::StepStatus::Ready),
            "a run the reader cannot read is left exactly as it was"
        );
    }
}
