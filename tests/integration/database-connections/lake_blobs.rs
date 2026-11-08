//! What the catalog vouches for when a lake lease names blobs (feature 035, slice R, step R2).
//!
//! A descriptor in a workflow value proves nothing: it is a JSON value any earlier step, or any input of the run, may have forged. The
//! lease asks the catalog, inside its own transaction, and takes only what the run's own steps committed as outputs. Every other case
//! answers the same, nothing, so that the answer says nothing about what exists: another run, another namespace, another tenant, a run
//! input, an upload that is not committed, a blob being deleted, an id that does not exist.
#![allow(clippy::expect_used, clippy::panic)]
use crate::support::engine::{World, activity, document};
use sagat_application::{
    execution::RunTx,
    runs::{Lock, RunStore},
};
use sagat_domain::{
    blobs::{BlobDescriptor, BlobId, ContentType},
    execution::RunId,
    workflow::Digest,
};
use serde_json::json;
use uuid::Uuid;

const NDJSON: &str = "application/x-ndjson";
const PARQUET: &str = "application/vnd.apache.parquet";

/// The step of a run the blob would be an output of.
struct Step {
    id: Uuid,
    attempt: i32,
    generation: i64,
}

async fn step_of(w: &World, tenant: Uuid, run: RunId) -> Step {
    let (id, attempt, generation): (Uuid, i32, i64) = sqlx::query_as(
        "SELECT step_id, attempt, generation FROM steps WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the step of the run");
    Step {
        id,
        attempt,
        generation,
    }
}

/// A row of the catalog, written as the owner does: the state machine that reaches it is the blob service's, and is tested there.
struct Row {
    tenant: Uuid,
    namespace: Uuid,
    run: RunId,
    step: Option<Uuid>,
    attempt: Option<i32>,
    generation: Option<i64>,
    purpose: &'static str,
    state: &'static str,
    content_type: &'static str,
    size: i64,
    digest: u8,
}

impl Row {
    fn output(tenant: Uuid, namespace: Uuid, run: RunId, step: &Step) -> Self {
        Self {
            tenant,
            namespace,
            run,
            step: Some(step.id),
            attempt: Some(step.attempt),
            generation: Some(step.generation),
            purpose: "STEP_OUTPUT",
            state: "COMMITTED",
            content_type: NDJSON,
            size: 10,
            digest: 1,
        }
    }

    async fn insert(&self, w: &World) -> BlobDescriptor {
        let id = BlobId::generate();
        let measured = self.state != "UPLOADING";
        let deleting = self.state == "DELETING";
        sqlx::query(
            "INSERT INTO blobs (tenant_id, namespace_id, id, state, size_bytes, sha256, content_type, created_by, run_id, step_id, \
             attempt, generation, purpose, uploaded_at, committed_at, deletion_attempts, deletion_last_attempt_at, delete_after) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'lake-blobs-test', $8, $9, $10, $11, $12, \
             CASE WHEN $13 THEN now() END, CASE WHEN $4 = 'COMMITTED' THEN now() END, \
             CASE WHEN $14 THEN 1 ELSE 0 END, CASE WHEN $14 THEN now() END, CASE WHEN $14 THEN now() + interval '1 day' END)",
        )
        .bind(self.tenant)
        .bind(self.namespace)
        .bind(id.as_uuid())
        .bind(self.state)
        .bind(measured.then_some(self.size))
        .bind(measured.then(|| vec![self.digest; 32]))
        .bind(self.content_type)
        .bind(self.run.as_uuid())
        .bind(self.step)
        .bind(self.attempt)
        .bind(self.generation)
        .bind(self.purpose)
        .bind(measured)
        .bind(deleting)
        .execute(&w.db.pool)
        .await
        .unwrap_or_else(|error| panic!("a {} blob: {error}", self.state));
        BlobDescriptor::new(
            id,
            u64::try_from(self.size).expect("size"),
            Digest::new([self.digest; 32]),
            ContentType::parse(self.content_type).expect("media type"),
        )
    }
}

pub async fn run(w: &World) {
    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let (flow, _) = w
        .publish(document(
            "lake-blobs-flow",
            "store",
            vec![activity("store")],
        ))
        .await;
    let run = w.start(flow, &json!({})).await;
    let other_run = w.start(flow, &json!({})).await;
    let step = step_of(w, tenant, run).await;
    let other_step = step_of(w, tenant, other_run).await;

    // What the run's own steps committed.
    let first = Row::output(tenant, namespace, run, &step).insert(w).await;
    let second = Row {
        content_type: PARQUET,
        size: 20,
        digest: 2,
        ..Row::output(tenant, namespace, run, &step)
    }
    .insert(w)
    .await;

    // Everything else, each as a row that exists and is not the run's to read.
    let other_namespace = w.add_namespace("lake-blobs-other").await.id().as_uuid();
    let outsiders = [
        (
            "a blob of another run of the same namespace",
            Row::output(tenant, namespace, other_run, &other_step)
                .insert(w)
                .await,
        ),
        (
            "a blob of this run in another namespace of the tenant",
            Row::output(tenant, other_namespace, run, &step)
                .insert(w)
                .await,
        ),
        (
            "an input of the run",
            Row {
                step: None,
                attempt: None,
                generation: None,
                purpose: "RUN_INPUT",
                ..Row::output(tenant, namespace, run, &step)
            }
            .insert(w)
            .await,
        ),
        (
            "an upload confirmed and not committed",
            Row {
                state: "UPLOADED",
                ..Row::output(tenant, namespace, run, &step)
            }
            .insert(w)
            .await,
        ),
        (
            "an upload not finished",
            Row {
                state: "UPLOADING",
                ..Row::output(tenant, namespace, run, &step)
            }
            .insert(w)
            .await,
        ),
        (
            "a blob being deleted",
            Row {
                state: "DELETING",
                ..Row::output(tenant, namespace, run, &step)
            }
            .insert(w)
            .await,
        ),
        (
            "an id that does not exist",
            BlobDescriptor::new(
                BlobId::generate(),
                10,
                Digest::new([1; 32]),
                ContentType::parse(NDJSON).expect("media type"),
            ),
        ),
    ];
    // Another tenant's blob, of a run of its own.
    let other_flow = w
        .publish_as_other_tenant(document(
            "lake-blobs-theirs",
            "store",
            vec![activity("store")],
        ))
        .await;
    let theirs = w
        .start_as_other_tenant(other_flow, &json!({}))
        .await
        .expect("a run of the other tenant");
    let their_tenant = w.other_admin.tenant_id().as_uuid();
    let their_step = step_of(w, their_tenant, theirs).await;
    let foreign = Row::output(
        their_tenant,
        w.other_namespace.id().as_uuid(),
        theirs,
        &their_step,
    )
    .insert(w)
    .await;

    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("the store")
        .expect("the run");
    let mut ids: Vec<BlobId> = vec![first.id(), second.id(), foreign.id()];
    ids.extend(outsiders.iter().map(|(_, descriptor)| descriptor.id()));
    let mut found = tx.lake_record_blobs(&ids).await.expect("the catalog");
    found.sort_by_key(BlobDescriptor::id);
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort_by_key(BlobDescriptor::id);
    assert_eq!(
        found, expected,
        "what the run's own steps committed, with the descriptor the catalog holds, and nothing else"
    );

    // Each of the rest, asked for by itself, is nothing: the answer says nothing about whether the row exists.
    for (name, descriptor) in &outsiders {
        assert!(
            tx.lake_record_blobs(&[descriptor.id()])
                .await
                .expect("the catalog")
                .is_empty(),
            "{name}"
        );
    }
    assert!(
        tx.lake_record_blobs(&[foreign.id()])
            .await
            .expect("the catalog")
            .is_empty(),
        "a blob of another tenant"
    );
    assert!(
        tx.lake_record_blobs(&[])
            .await
            .expect("the catalog")
            .is_empty(),
        "nothing asked, nothing answered"
    );
    // The store does not take an unbounded list on the caller's word: a lease names at most 32 blobs, and 33 are refused.
    let too_many: Vec<BlobId> = (0..33).map(|_| BlobId::generate()).collect();
    assert_eq!(
        tx.lake_record_blobs(&too_many).await,
        Err(sagat_application::error::StoreError::DatabaseLimit),
        "33 ids"
    );
    assert!(
        tx.lake_record_blobs(&too_many[..32])
            .await
            .expect("32 ids")
            .is_empty()
    );
    // The other run's own blob is its own: asked from its transaction it is found, so the refusal above is the scope and not the row.
    drop(tx);
    let mut theirs_tx = w
        .store
        .begin(w.admin.tenant_id(), other_run, Lock::Wait)
        .await
        .expect("the store")
        .expect("the run");
    assert_eq!(
        theirs_tx
            .lake_record_blobs(&[outsiders[0].1.id(), first.id()])
            .await
            .expect("the catalog"),
        vec![outsiders[0].1.clone()],
        "a run reads its own blob and not another run's"
    );
}
