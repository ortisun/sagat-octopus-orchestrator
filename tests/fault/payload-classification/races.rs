//! Deterministic PostgreSQL race coverage for classified payload disclosure.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use sagat_application::classification::{
    ClassificationAnalysis, ClassificationInterpreter, ClassifiedNode, ClassifiedPayload,
    ProjectionFailure, ProjectionRequest, ScalarValue,
};
use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::payload_disclosure::{
    DisclosureSelector, PayloadDisclosure, PayloadDisclosureError, PayloadDisclosureResult,
    PayloadDisclosureService, PayloadDisclosureUseCase,
};
use sagat_application::reveal_grants::RevealGrantService;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Permission, Principal, Role};
use sagat_infra_ir::JsonPayloadProjector;
use sagat_infra_postgres::{PgRunStore, has_effective_payload_reveal_grant};
use sagat_infra_schema::classification::{CacheStats, Classifier};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;

use super::support::engine::{World, ctx, document, end_node, world};

#[derive(Clone)]
struct SharedClassifier(Arc<Classifier>);

impl ClassificationInterpreter for SharedClassifier {
    fn analyze(&self, document: &[u8]) -> ClassificationAnalysis {
        self.0.analyze(document)
    }

    fn project(
        &self,
        request: ProjectionRequest<'_>,
    ) -> Result<ClassifiedPayload, ProjectionFailure> {
        self.0.project(request)
    }
}

type Disclosure =
    PayloadDisclosureService<PgRunStore, SystemClock, SharedClassifier, JsonPayloadProjector>;

fn classified_document(name: &str) -> Value {
    let mut value = document(name, "done", vec![end_node("done")]);
    value["inputSchema"] = json!({
        "type": "object",
        "additionalProperties": false,
        "x-sagat-classification": "pii",
        "properties": {"personal": {"type": "string"}}
    });
    value
}

fn disclosure(world: &World, classifier: Arc<Classifier>) -> Disclosure {
    PayloadDisclosureService::new(
        world.store.clone(),
        SystemClock,
        SharedClassifier(classifier),
        JsonPayloadProjector,
    )
}

async fn reveal(
    service: &Disclosure,
    principal: &Principal,
    run: RunId,
) -> Result<PayloadDisclosure, PayloadDisclosureError> {
    let prepared = service
        .prepare(principal, run, DisclosureSelector::Input, true, &ctx())
        .await
        .map_err(PayloadDisclosureError::from)?;
    service.disclose(prepared).await
}

fn disclosed_string<'a>(node: &'a ClassifiedNode, name: &str) -> Option<&'a str> {
    let ClassifiedNode::Container { children, .. } = node else {
        return None;
    };
    children.iter().find_map(|child| {
        if child.name.as_deref() != Some(name) {
            return None;
        }
        match &*child.node {
            ClassifiedNode::Value {
                value: ScalarValue::String(value),
                ..
            } => Some(value.as_str()),
            _ => None,
        }
    })
}

fn assert_revealed(answer: PayloadDisclosure) {
    let PayloadDisclosureResult::Disclosed { projection, .. } = answer.result else {
        panic!("expected a disclosed classified payload")
    };
    assert_eq!(
        disclosed_string(&projection.tree, "personal"),
        Some("concurrent-pii-canary")
    );
}

struct PausingClassifier {
    classifier: Classifier,
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl ClassificationInterpreter for PausingClassifier {
    fn analyze(&self, document: &[u8]) -> ClassificationAnalysis {
        self.classifier.analyze(document)
    }

    fn project(
        &self,
        request: ProjectionRequest<'_>,
    ) -> Result<ClassifiedPayload, ProjectionFailure> {
        if let Some(entered) = self.entered.lock().expect("entry barrier").take() {
            entered.send(()).expect("announce pinned read");
            self.resume
                .lock()
                .expect("resume barrier")
                .recv_timeout(Duration::from_secs(30))
                .expect("release pinned read");
        }
        self.classifier.project(request)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_publication_and_activation_never_reinterpret_a_pinned_disclosure() {
    let world = world().await;
    let source = classified_document("publication-race");
    let (workflow, first) = world.publish(source.clone()).await;
    let run = world
        .start(workflow, &json!({"personal":"concurrent-pii-canary"}))
        .await;
    let reader = world.member("publication-reader", Role::Operator).await;
    let grants = RevealGrantService::new(world.store.clone(), SystemClock);
    let scope = grants
        .prepare(&world.admin, world.namespace.id(), &ctx())
        .await
        .expect("grant scope");
    grants
        .grant(&scope, reader.subject().as_str(), &ctx())
        .await
        .expect("grant reader");
    let (entered, pinned) = tokio::sync::oneshot::channel();
    let (resume, released) = std::sync::mpsc::channel();
    let service = PayloadDisclosureService::new(
        world.store.clone(),
        SystemClock,
        PausingClassifier {
            classifier: Classifier::default(),
            entered: std::sync::Mutex::new(Some(entered)),
            resume: std::sync::Mutex::new(released),
        },
        JsonPayloadProjector,
    );
    let prepared = service
        .prepare(&reader, run, DisclosureSelector::Input, true, &ctx())
        .await
        .expect("prepared reveal");
    // The synchronous interpreter barrier must not occupy a Tokio I/O worker while publication
    // and activation need that executor to progress.
    let runtime = tokio::runtime::Handle::current();
    let reading = tokio::task::spawn_blocking(move || runtime.block_on(service.disclose(prepared)));
    tokio::time::timeout(Duration::from_secs(10), pinned)
        .await
        .expect("read barrier timeout")
        .expect("pinned schema loaded");
    let mut changed = source;
    changed["inputSchema"]["x-sagat-classification"] = json!("secret");
    let second = tokio::time::timeout(
        Duration::from_secs(10),
        world.publish_new_version(workflow, changed),
    )
    .await
    .expect("publish must complete while the old read is paused");
    assert_ne!(first, second);
    tokio::time::timeout(Duration::from_secs(10), async {
        world
            .publishing
            .activate(
                &world.admin,
                workflow,
                1,
                "race baseline",
                None,
                false,
                &ctx(),
            )
            .await
            .expect("activate first");
        world
            .publishing
            .activate(
                &world.admin,
                workflow,
                2,
                "race replacement",
                Some(1),
                false,
                &ctx(),
            )
            .await
            .expect("activate second");
    })
    .await
    .expect("activation must complete while the old read is paused");
    resume.send(()).expect("resume pinned disclosure");
    let answer = reading.await.expect("read task").expect("pinned reveal");
    let PayloadDisclosureResult::Disclosed { ref schema, .. } = answer.result else {
        panic!("pinned disclosure")
    };
    assert_eq!(schema.version_number, 1);
    assert_revealed(answer);
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 1);

    let next = world
        .start(workflow, &json!({"personal":"concurrent-pii-canary"}))
        .await;
    let current = disclosure(&world, Arc::new(Classifier::default()));
    let answer = reveal(&current, &reader, next)
        .await
        .expect("new active-version disclosure");
    let PayloadDisclosureResult::Disclosed {
        schema, projection, ..
    } = answer.result
    else {
        panic!("new version projection")
    };
    assert_eq!(schema.version_number, 2);
    assert!(matches!(
        projection.tree,
        ClassifiedNode::Withheld {
            level: sagat_domain::classification::Level::Secret,
            ..
        }
    ));
}

async fn reveal_audit_count(world: &World, actor: &Principal, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND actor=$2 AND resource_id=$3 \
         AND action='payload.pii_revealed' AND reason='classified-v1:input:reveal=pii'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(actor.subject().as_str())
    .bind(run.to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("PII reveal audit count")
}

async fn wait_for_database_lock(world: &World, pid: i32, operation: &str) {
    super::support::eventually(operation, || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid=$1 AND NOT granted)",
        )
        .bind(pid)
        .fetch_one(&world.db.pool)
        .await
        .expect("database lock observation")
        .then_some(())
    })
    .await;
}

#[tokio::test]
async fn revoke_and_membership_delete_wait_for_the_in_flight_grant_snapshot() {
    let world = world().await;
    let namespace = world.namespace.id();
    let reader = world.member("racing-reader", Role::Developer).await;
    let grants = RevealGrantService::new(world.store.clone(), SystemClock);
    let scope = grants
        .prepare(&world.admin, namespace, &ctx())
        .await
        .expect("grant scope");
    grants
        .grant(&scope, reader.subject().as_str(), &ctx())
        .await
        .expect("initial grant");

    let mut disclosure = world.db.pool.begin().await.expect("disclosure transaction");
    assert!(
        has_effective_payload_reveal_grant(
            &mut disclosure,
            world.admin.tenant_id(),
            namespace,
            reader.subject(),
        )
        .await
        .expect("effective grant")
    );

    let revoke_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&world.db.url)
        .await
        .expect("dedicated revoke pool");
    let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&revoke_pool)
        .await
        .expect("revoke backend pid");
    let revoke_store = PgRunStore::new(revoke_pool.clone(), world.cache.clone());
    let revoke_service = RevealGrantService::new(revoke_store, SystemClock);
    let revoke_scope = revoke_service
        .prepare(&world.admin, namespace, &ctx())
        .await
        .expect("dedicated revoke scope");
    let revoke_subject = reader.subject().as_str().to_owned();
    let revoke = tokio::spawn(async move {
        revoke_service
            .revoke(&revoke_scope, &revoke_subject, &ctx())
            .await
    });
    wait_for_database_lock(
        &world,
        revoke_pid,
        "revoke blocked by the shared grant fence",
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM payload_reveal_grants \
             WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
        )
        .bind(world.admin.tenant_id().as_uuid())
        .bind(namespace.as_uuid())
        .bind(reader.subject().as_str())
        .fetch_one(&world.db.pool)
        .await
        .expect("grant while revoke waits"),
        1
    );
    disclosure
        .commit()
        .await
        .expect("finish disclosure critical section");
    assert!(
        tokio::time::timeout(Duration::from_secs(5), revoke)
            .await
            .expect("revoke released after commit")
            .expect("revoke task")
            .expect("revoke result")
    );
    drop(revoke_pool);

    grants
        .grant(&scope, reader.subject().as_str(), &ctx())
        .await
        .expect("grant again for membership race");
    let mut disclosure = world
        .db
        .pool
        .begin()
        .await
        .expect("second disclosure transaction");
    assert!(
        has_effective_payload_reveal_grant(
            &mut disclosure,
            world.admin.tenant_id(),
            namespace,
            reader.subject(),
        )
        .await
        .expect("second effective grant")
    );

    let mut delete_connection = world.db.pool.acquire().await.expect("delete connection");
    let delete_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *delete_connection)
        .await
        .expect("delete backend pid");
    let tenant = world.admin.tenant_id();
    let subject = reader.subject().as_str().to_owned();
    let delete = tokio::spawn(async move {
        sqlx::query("DELETE FROM memberships WHERE tenant_id=$1 AND subject=$2")
            .bind(tenant.as_uuid())
            .bind(subject)
            .execute(&mut *delete_connection)
            .await
    });
    wait_for_database_lock(
        &world,
        delete_pid,
        "membership delete blocked by the disclosure membership snapshot",
    )
    .await;
    disclosure
        .rollback()
        .await
        .expect("abort disclosure before its commit boundary");
    let deleted = tokio::time::timeout(Duration::from_secs(5), delete)
        .await
        .expect("membership delete released after rollback")
        .expect("membership delete task")
        .expect("membership delete result");
    assert_eq!(deleted.rows_affected(), 1);
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payload_reveal_grants \
         WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(namespace.as_uuid())
    .bind(reader.subject().as_str())
    .fetch_one(&world.db.pool)
    .await
    .expect("grant cascade after membership deletion");
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn concurrent_duplicate_reveals_each_audit_and_service_recreation_starts_with_an_empty_cache()
{
    let world = world().await;
    let (workflow, _) = world
        .publish(classified_document("concurrent-reveal"))
        .await;
    let run = world
        .start(workflow, &json!({"personal": "concurrent-pii-canary"}))
        .await;
    let reader = world.member("concurrent-reader", Role::Developer).await;
    sqlx::query(
        "INSERT INTO payload_reveal_grants \
         (tenant_id,namespace_id,subject,granted_by,granted_at) \
         VALUES ($1,$2,$3,$4,clock_timestamp())",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(reader.subject().as_str())
    .bind(world.admin.subject().as_str())
    .execute(&world.db.pool)
    .await
    .expect("reveal grant");

    let classifier = Arc::new(Classifier::default());
    assert_eq!(classifier.cache_stats(), CacheStats::default());
    let service = Arc::new(disclosure(&world, Arc::clone(&classifier)));
    let first = service
        .prepare(&reader, run, DisclosureSelector::Input, true, &ctx())
        .await
        .expect("first prepared reveal");
    let second = service
        .prepare(&reader, run, DisclosureSelector::Input, true, &ctx())
        .await
        .expect("second prepared reveal");
    let first_service = Arc::clone(&service);
    let second_service = Arc::clone(&service);
    let (first, second) = tokio::join!(
        async move { first_service.disclose(first).await },
        async move { second_service.disclose(second).await },
    );
    assert_revealed(first.expect("first concurrent reveal"));
    assert_revealed(second.expect("second concurrent reveal"));
    assert_eq!(
        reveal_audit_count(&world, &reader, run).await,
        2,
        "concurrent duplicate requests retain one audit each"
    );
    assert!(classifier.cache_stats().misses >= 1);

    drop(service);
    drop(classifier);
    let recreated_classifier = Arc::new(Classifier::default());
    assert_eq!(
        recreated_classifier.cache_stats(),
        CacheStats::default(),
        "a newly constructed service has no process-local classifier cache state"
    );
    let recreated = disclosure(&world, Arc::clone(&recreated_classifier));
    assert_revealed(
        reveal(&recreated, &reader, run)
            .await
            .expect("reveal after service recreation"),
    );
    assert_eq!(recreated_classifier.cache_stats().misses, 1);
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 3);
}

/// A reveal through the production path, paused inside its read transaction by the interpreter
/// (which runs after the authority checks and before the audit insert).
struct PausedReveal {
    reading: tokio::task::JoinHandle<Result<PayloadDisclosure, PayloadDisclosureError>>,
    resume: std::sync::mpsc::Sender<()>,
}

async fn pause_a_reveal(world: &World, reader: &Principal, run: RunId) -> PausedReveal {
    let (entered, pinned) = tokio::sync::oneshot::channel();
    let (resume, released) = std::sync::mpsc::channel();
    let service = PayloadDisclosureService::new(
        world.store.clone(),
        SystemClock,
        PausingClassifier {
            classifier: Classifier::default(),
            entered: std::sync::Mutex::new(Some(entered)),
            resume: std::sync::Mutex::new(released),
        },
        JsonPayloadProjector,
    );
    let prepared = service
        .prepare(reader, run, DisclosureSelector::Input, true, &ctx())
        .await
        .expect("prepared reveal");
    // The barrier is synchronous: it must not hold a Tokio worker that the database waits need.
    let runtime = tokio::runtime::Handle::current();
    let reading = tokio::task::spawn_blocking(move || runtime.block_on(service.disclose(prepared)));
    tokio::time::timeout(Duration::from_secs(10), pinned)
        .await
        .expect("read barrier timeout")
        .expect("the read is inside its transaction");
    PausedReveal { reading, resume }
}

impl PausedReveal {
    async fn finish(self) -> Result<PayloadDisclosure, PayloadDisclosureError> {
        self.resume.send(()).expect("resume the paused reveal");
        self.reading.await.expect("read task")
    }
}

/// T016-050: a grant revoked and a membership deleted *while a reveal runs*, through the production
/// disclose path (not the helper alone). The change waits for the reveal that already passed its
/// authority check, the reveal completes with its audit record, and the next reveal is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_and_membership_delete_wait_for_an_in_flight_production_reveal() {
    let world = world().await;
    let namespace = world.namespace.id();
    let (workflow, _) = world.publish(classified_document("in-flight-revoke")).await;
    let run = world
        .start(workflow, &json!({"personal":"concurrent-pii-canary"}))
        .await;
    let reader = world.member("in-flight-reader", Role::Developer).await;
    let grants = RevealGrantService::new(world.store.clone(), SystemClock);
    let scope = grants
        .prepare(&world.admin, namespace, &ctx())
        .await
        .expect("grant scope");
    let grant_rows = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM payload_reveal_grants \
             WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
        )
        .bind(world.admin.tenant_id().as_uuid())
        .bind(namespace.as_uuid())
        .bind(reader.subject().as_str())
        .fetch_one(&world.db.pool)
        .await
        .expect("grant rows")
    };
    let next_reveal = || async {
        reveal(
            &disclosure(&world, Arc::new(Classifier::default())),
            &reader,
            run,
        )
        .await
    };

    // The grant is revoked while the reveal is paused inside its transaction.
    grants
        .grant(&scope, reader.subject().as_str(), &ctx())
        .await
        .expect("grant");
    let paused = pause_a_reveal(&world, &reader, run).await;
    let revoke_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&world.db.url)
        .await
        .expect("dedicated revoke pool");
    let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&revoke_pool)
        .await
        .expect("revoke backend pid");
    let revoke_service = RevealGrantService::new(
        PgRunStore::new(revoke_pool.clone(), world.cache.clone()),
        SystemClock,
    );
    let revoke_scope = revoke_service
        .prepare(&world.admin, namespace, &ctx())
        .await
        .expect("revoke scope");
    let subject = reader.subject().as_str().to_owned();
    let revoke =
        tokio::spawn(async move { revoke_service.revoke(&revoke_scope, &subject, &ctx()).await });
    wait_for_database_lock(&world, revoke_pid, "revoke waits for the paused reveal").await;
    assert_eq!(
        grant_rows().await,
        1,
        "the grant stands while the reveal runs"
    );
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 0);
    assert_revealed(paused.finish().await.expect("the paused reveal completes"));
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 1);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), revoke)
            .await
            .expect("revoke released after the reveal committed")
            .expect("revoke task")
            .expect("revoke result"),
        "the grant was removed"
    );
    drop(revoke_pool);
    assert_eq!(
        next_reveal().await.map(|_| ()),
        Err(PayloadDisclosureError::PiiRevealNotGranted)
    );
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 1);

    // The membership is deleted while the next reveal is paused.
    grants
        .grant(&scope, reader.subject().as_str(), &ctx())
        .await
        .expect("grant again");
    let paused = pause_a_reveal(&world, &reader, run).await;
    let mut delete_connection = world.db.pool.acquire().await.expect("delete connection");
    let delete_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *delete_connection)
        .await
        .expect("delete backend pid");
    let tenant = world.admin.tenant_id();
    let subject = reader.subject().as_str().to_owned();
    let delete = tokio::spawn(async move {
        sqlx::query("DELETE FROM memberships WHERE tenant_id=$1 AND subject=$2")
            .bind(tenant.as_uuid())
            .bind(subject)
            .execute(&mut *delete_connection)
            .await
    });
    wait_for_database_lock(
        &world,
        delete_pid,
        "membership delete waits for the paused reveal",
    )
    .await;
    assert_eq!(grant_rows().await, 1);
    assert_revealed(paused.finish().await.expect("the paused reveal completes"));
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 2);
    let deleted = tokio::time::timeout(Duration::from_secs(5), delete)
        .await
        .expect("membership delete released after the reveal committed")
        .expect("delete task")
        .expect("delete result");
    assert_eq!(deleted.rows_affected(), 1);
    assert_eq!(grant_rows().await, 0, "the grant went with the membership");
    // Without a membership the caller holds no permission at all, not even to read the run.
    assert_eq!(
        next_reveal().await.map(|_| ()),
        Err(PayloadDisclosureError::Application(AppError::Forbidden {
            permission: Permission::RunRead
        }))
    );
    assert_eq!(reveal_audit_count(&world, &reader, run).await, 2);
}
