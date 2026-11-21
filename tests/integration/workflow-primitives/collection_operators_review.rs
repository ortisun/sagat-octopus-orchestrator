//! Findings of the independent review of design 042's slice S1 (2026-11-15), against real PostgreSQL: a chained
//! manifest that does not describe its items is refused at entry and at restore, the tick takes a bounded number of
//! chained entries, a chained entry that cannot be stored for the tenant's content quota stops the run typed, and the
//! FR-042-012 snapshot of a raw document with `-0` is today's.

use std::time::Duration;

use sagat_application::collection_source::{ChainedSourceManifest, SourceManifest, items_digest};
use sagat_application::primitives::{
    CollectionEntry, CollectionLimits, CollectionScopeId, CollectionStore, CollectionTransition,
    PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::shards::ShardStore;
use sagat_domain::blobs::{BlobDescriptor, ContentType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::Subject;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::Digest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::collection_operators::{Mode, Watched, chained_ir, owner, replica, scope_of};
use crate::support::blob_collections::{
    blob_source, namespace_of, platform_with_operators, scopes, start_inline, start_with_blob,
    status_and_code, upload,
};
use crate::support::worker::Platform;

fn descriptor(upload: &Value) -> BlobDescriptor {
    BlobDescriptor::new(
        upload["id"].as_str().expect("id").parse().expect("blob id"),
        upload["size"].as_u64().expect("size"),
        Digest::from_hex(upload["sha256"].as_str().expect("sha256")).expect("digest"),
        ContentType::parse(upload["contentType"].as_str().expect("type")).expect("type"),
    )
}

async fn chained_blob_run(p: &Platform, name: &str, source: &[u8]) -> (Uuid, Value) {
    let workflow = p
        .publish(
            name,
            &chained_ir(
                name,
                "input",
                &json!([{"filter": {"where": "element.v > 0"}}]),
                10,
            ),
        )
        .await;
    let blob = upload(p, source).await;
    let run = start_with_blob(p, &workflow, blob["id"].as_str().expect("id")).await;
    (run, blob)
}

/// An entry whose items are not what its chained manifest says is refused by the store, whoever builds it.
#[tokio::test]
async fn an_entry_whose_items_differ_from_its_chained_manifest_is_refused() {
    let p = platform_with_operators().await;
    let source = br#"[{"v": 1}, {"v": -1}, {"v": 2}]"#;
    let (run, blob) = chained_blob_run(&p, "operators-manifest-entry", source).await;
    let store = p.api.executions.clone();
    let lease = store
        .acquire(
            "operators-manifest-entry",
            &[Shard::of(run)],
            Duration::from_secs(60),
        )
        .await
        .expect("acquire")
        .into_iter()
        .next()
        .expect("the run's shard");
    let version = p.run(&run.to_string()).await["version"]
        .as_u64()
        .expect("version");
    let items = vec![br#"{"v":1}"#.to_vec(), br#"{"v":2}"#.to_vec()];
    let source_items =
        sagat_application::collection_source::canonical_items(source, 1000).expect("items");
    let source_manifest = SourceManifest::new(descriptor(&blob), 3, items_digest(&source_items));
    let namespace = namespace_of(&p).await;
    let entry = |manifest: &ChainedSourceManifest, items: Vec<Vec<u8>>| {
        CollectionEntry::from_chained_blob_source(
            CollectionTransition::new(
                p.tenant,
                namespace,
                RunId::new(run),
                version,
                owner(),
                SchedulerAuthority::new("operators-manifest-entry", lease),
                Subject::engine(),
                None,
                crate::support::now(),
                None,
            ),
            CollectionScopeId::generate(),
            manifest,
            items,
            CollectionLimits::new(10, 4, 100_000, 8).expect("limits"),
        )
        .expect("entry")
    };
    for (what, manifest, carried) in [
        (
            "a count the items do not have",
            ChainedSourceManifest::new(
                &source_manifest,
                Digest::new([1; 32]),
                3,
                items_digest(&items),
            ),
            items.clone(),
        ),
        (
            "a digest of other items",
            ChainedSourceManifest::new(
                &source_manifest,
                Digest::new([1; 32]),
                2,
                items_digest(&[b"1".to_vec(), b"2".to_vec()]),
            ),
            items.clone(),
        ),
        (
            "items in another order",
            ChainedSourceManifest::new(
                &source_manifest,
                Digest::new([1; 32]),
                2,
                items_digest(&items),
            ),
            vec![items[1].clone(), items[0].clone()],
        ),
    ] {
        let refused = store.enter_collection(&entry(&manifest, carried)).await;
        assert!(
            matches!(refused, Err(PrimitiveStoreError::InvalidTransition)),
            "{what}: {refused:?}"
        );
        assert_eq!(scopes(&p, run).await, 0, "{what}: nothing recorded");
    }
}

/// A stored chained manifest that no longer describes its items is invalid storage, for the restore and `verify_run`.
#[tokio::test]
async fn a_tampered_chained_manifest_is_refused_by_the_restore_and_verify_run() {
    let p = platform_with_operators().await;
    let source = br#"[{"v": 1}, {"v": -1}, {"v": 2}]"#;
    for (what, tamper) in [
        ("count", ("count", json!(3))),
        ("itemsSha256", ("itemsSha256", json!("ab".repeat(32)))),
    ] {
        let (run, _) = chained_blob_run(
            &p,
            &format!("operators-tampered-{}", what.to_lowercase()),
            source,
        )
        .await;
        let (scheduler, keeper) =
            replica(&p, Watched::new(Mode::Apply), Some(blob_source(&p))).await;
        scheduler.tick().await;
        keeper.leave().await;
        let (count, snapshot) = scope_of(&p, run).await;
        assert_eq!(count, 2);
        p.api
            .executions
            .verify_run(p.tenant, RunId::new(run))
            .await
            .expect("the untouched scope verifies");
        let mut manifest: Value = serde_json::from_slice(&snapshot).expect("json");
        manifest["$collectionChainSource"][tamper.0] = tamper.1.clone();
        let bytes = serde_json::to_vec(&manifest).expect("json");
        let digest = Sha256::digest(&bytes).to_vec();
        sqlx::query(
            "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(p.tenant.as_uuid())
        .bind(&digest)
        .bind(&bytes)
        .execute(&p.api.db.pool)
        .await
        .expect("a tampered manifest");
        sqlx::query("UPDATE iteration_scopes SET snapshot_digest = $2 WHERE run_id = $1")
            .bind(run)
            .bind(&digest)
            .execute(&p.api.db.pool)
            .await
            .expect("point the scope at it");
        let restored = p
            .api
            .executions
            .collection_progress(p.tenant, RunId::new(run), &owner())
            .await;
        assert!(
            matches!(restored, Err(PrimitiveStoreError::Storage(_))),
            "{what}: the restore refuses it as invalid storage: {restored:?}"
        );
        assert!(
            p.api
                .executions
                .verify_run(p.tenant, RunId::new(run))
                .await
                .is_err(),
            "{what}: verify_run refuses it"
        );
    }
}

/// A tick takes a bounded number of chained entries (review of S1): the rest wait for the next tick, so one tick
/// carries at most that many evaluations whatever the backlog.
#[tokio::test]
async fn a_tick_enters_a_bounded_number_of_chained_collections() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-per-tick",
            &chained_ir(
                "operators-per-tick",
                "input.lines",
                &json!([{"take": {"count": 1}}]),
                10,
            ),
        )
        .await;
    let mut runs = Vec::new();
    for _ in 0..6 {
        runs.push(start_inline(&p, &workflow, &json!({"lines": [1, 2]})).await);
    }
    let evaluator = Watched::new(Mode::Apply);
    let (scheduler, _keeper) = replica(&p, evaluator.clone(), None).await;
    scheduler.tick().await;
    assert_eq!(
        entered(&p, &runs).await,
        i64::try_from(sagat_scheduler::MAX_CHAINED_ENTRIES_PER_TICK).expect("small"),
        "one tick enters at most its bound"
    );
    assert_eq!(
        evaluator.calls(),
        sagat_scheduler::MAX_CHAINED_ENTRIES_PER_TICK,
        "the others were not evaluated"
    );
    scheduler.tick().await;
    assert_eq!(entered(&p, &runs).await, 6, "the next tick takes the rest");
}

async fn set_content_quota_to_current_usage(pool: &sqlx::PgPool, tenant: Uuid) {
    let used: i64 = sqlx::query_scalar(
        "SELECT content_bytes + content_reserved_bytes FROM tenant_retained_usage WHERE tenant_id=$1",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("retained usage");
    let mut tx = pool.begin().await.expect("content quota transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("quota-policy:tenant_content_bytes:{tenant}"))
        .execute(&mut *tx)
        .await
        .expect("content quota policy fence");
    sqlx::query(
        "DELETE FROM quota_limits WHERE tenant_id=$1 AND namespace_id IS NULL \
         AND resource='tenant_content_bytes'",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("clear content quota");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by) \
         VALUES ($1,NULL,'tenant_content_bytes',$2,'collection-operators-review')",
    )
    .bind(tenant)
    .bind(used)
    .execute(&mut *tx)
    .await
    .expect("close content quota");
    tx.commit().await.expect("commit content quota");
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true")
        .execute(pool)
        .await
        .expect("enable content enforcement");
}

/// G042-110: a chained entry the tenant's content quota cannot hold. The chain is evaluated once, the entry is refused
/// for the quota, and the run is stopped with the quota's code, typed and journaled, instead of trying again.
#[tokio::test]
async fn a_chained_entry_over_the_content_quota_stops_the_run_typed() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-quota",
            &chained_ir(
                "operators-quota",
                "input.lines",
                &json!([{"map": {"select": "[element, input.pad]"}}]),
                10,
            ),
        )
        .await;
    let run = start_inline(
        &p,
        &workflow,
        &json!({"lines": [1, 2, 3], "pad": "x".repeat(4096)}),
    )
    .await;
    set_content_quota_to_current_usage(&p.api.db.pool, p.tenant.as_uuid()).await;
    let evaluator = Watched::new(Mode::Apply);
    let (scheduler, _keeper) = replica(&p, evaluator.clone(), None).await;
    let advance = scheduler
        .advance_collection_from_run(p.tenant, RunId::new(run), &owner())
        .await;
    println!("G042-110 quota entry: {advance:?}");
    for _ in 0..5 {
        scheduler.tick().await;
    }
    let outcome = status_and_code(&p, run).await;
    println!(
        "G042-110 quota outcome: {outcome:?}, chain calls {}",
        evaluator.calls()
    );
    assert_eq!(
        scopes(&p, run).await,
        0,
        "nothing of the refused entry is recorded"
    );
    assert_eq!(
        outcome,
        (
            "FAILED".to_owned(),
            Some("STORAGE_QUOTA_EXCEEDED".to_owned())
        )
    );
    assert_eq!(
        evaluator.calls(),
        1,
        "evaluated once; the stop does not evaluate it again"
    );
}

async fn entered(p: &Platform, runs: &[Uuid]) -> i64 {
    let mut count = 0;
    for run in runs {
        count += scopes(p, *run).await;
    }
    count
}
