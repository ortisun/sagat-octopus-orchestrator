//! Deterministic PostgreSQL race coverage for classified payload disclosure.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use sagat_application::classification::{
    ClassificationAnalysis, ClassificationInterpreter, ClassifiedNode, ClassifiedPayload,
    ProjectionFailure, ProjectionRequest, ScalarValue,
};
use sagat_application::clock::SystemClock;
use sagat_application::payload_disclosure::{
    DisclosureSelector, PayloadDisclosure, PayloadDisclosureError, PayloadDisclosureResult,
    PayloadDisclosureService, PayloadDisclosureUseCase,
};
use sagat_application::reveal_grants::RevealGrantService;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Principal, Role};
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
