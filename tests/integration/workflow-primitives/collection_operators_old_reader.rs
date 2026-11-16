//! T042-50 (FR-042-003, NFR-042-004, design 042 section 10): a binary of the release before 0.3.0 refuses what a
//! chained collection stores, and never misreads it.
//!
//! The fixture is that release's reader itself, not a binary: v0.2.0 (`bfda3420`) reads a stored definition through
//! its closed IR schema (kept here as `fixtures/v0.2.0/workflow.schema.json`, extracted with
//! `git show v0.2.0:specs/001-platform-foundation/contracts/workflow.schema.json`, SHA-256 below) and restores a
//! collection's membership through the check of `collection_progress` in its `crates/infra-postgres/src/primitives.rs`
//! (lines 1429 to 1447), with `SourceManifest::parse` of its `crates/application/src/collection_source.rs`; both are
//! copied below as they were, but for the error type. The scopes they read are written by this build.
//!
//! The fleet rule this proves safe is in `docs/runbooks/workflow-primitives.md`: publish a chained `forEach` only
//! once every scheduler and control-plane replica runs 0.3.0.

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::collection_operators::{Mode, Watched, chained_ir, replica, scope_of};
use crate::support::blob_collections::{
    blob_source, collection_ir, platform_with_operators, scopes, start_with_blob, upload,
};
use crate::support::worker::Platform;

const SCHEMA: &str = include_str!("fixtures/v0.2.0/workflow.schema.json");
const SCHEMA_SHA256: &str = "bad20bde507a90e538fa92aefa4bd392f6c5a516b5254218db53b57919720c48";

fn released_schema() -> jsonschema::Validator {
    assert_eq!(
        sagat_domain::workflow::Digest::new(Sha256::digest(SCHEMA.as_bytes()).into()).to_hex(),
        SCHEMA_SHA256,
        "the fixture is the v0.2.0 schema, unchanged"
    );
    let schema: Value = serde_json::from_str(SCHEMA).expect("json");
    jsonschema::options()
        .build(&schema)
        .expect("the v0.2.0 schema compiles")
}

/// v0.2.0's `SourceManifest::parse`, over the stored bytes: `Some(count, itemsSha256)` for `$collectionSource` and
/// nothing else.
fn released_manifest(bytes: &[u8]) -> Option<(u64, String)> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let body = object.get("$collectionSource")?.as_object()?;
    if body.len() != 3 {
        return None;
    }
    let blob = body.get("blob")?.as_object()?;
    if blob.len() != 4 {
        return None;
    }
    Some((
        body.get("count")?.as_u64()?,
        body.get("itemsSha256")?.as_str()?.to_owned(),
    ))
}

/// v0.2.0's restore check of a collection's membership: the manifest of a blob read, or the snapshot array itself.
fn released_restore(snapshot: &[u8], items: &[Vec<u8>]) -> Result<(), &'static str> {
    if let Some((count, digest)) = released_manifest(snapshot) {
        if usize::try_from(count).ok() != Some(items.len())
            || sagat_application::collection_source::items_digest(items).to_hex() != digest
        {
            return Err("collection membership projection");
        }
    } else {
        let snapshot_items = serde_json::from_slice::<Vec<Value>>(snapshot)
            .map_err(|_| "collection snapshot payload")?
            .into_iter()
            .map(|item| serde_json::to_vec(&item).map_err(|_| "collection snapshot payload"))
            .collect::<Result<Vec<_>, _>>()?;
        if snapshot_items != items {
            return Err("collection membership projection");
        }
    }
    Ok(())
}

#[test]
fn a_release_before_0_3_0_refuses_a_chained_definition() {
    let schema = released_schema();
    let chained = chained_ir(
        "operators-old-reader",
        "input.lines",
        &json!([{"filter": {"where": "element > 0"}}]),
        10,
    );
    let errors: Vec<String> = schema
        .iter_errors(&chained)
        .map(|error| format!("{} {}", error.instance_path(), error))
        .collect();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("/nodes/0") && error.contains("operators")),
        "the closed v0.2.0 schema refuses `operators` on the forEach: {errors:?}"
    );
    let mut plain = chained;
    plain["nodes"][0]
        .as_object_mut()
        .expect("node")
        .remove("operators");
    assert!(
        schema.is_valid(&plain),
        "the same document without the chain is a v0.2.0 document"
    );
}

async fn stored_items(p: &Platform, run: uuid::Uuid) -> Vec<Vec<u8>> {
    sqlx::query_scalar(
        "SELECT payload.content FROM collection_scope_items item JOIN payloads payload \
           ON payload.tenant_id = item.tenant_id AND payload.digest = item.item_digest \
         JOIN iteration_scopes scope ON scope.tenant_id = item.tenant_id \
          AND scope.run_id = item.run_id AND scope.scope_id = item.scope_id \
         WHERE scope.run_id = $1 ORDER BY item.iteration_index",
    )
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("stored items")
}

#[tokio::test]
async fn a_release_before_0_3_0_refuses_a_chained_blob_scope_without_misreading_it() {
    let p = platform_with_operators().await;
    let source = br#"[{"v": 1}, {"v": -2}, {"v": 3}]"#;

    // A blob scope without a chain, entered by this build: the released reader reads it (the positive control).
    let plain = p
        .publish(
            "operators-old-plain",
            &collection_ir("operators-old-plain", "input", 10),
        )
        .await;
    let blob = upload(&p, source).await;
    let run = start_with_blob(&p, &plain, blob["id"].as_str().expect("id")).await;
    let (scheduler, keeper) = replica(&p, Watched::new(Mode::Apply), Some(blob_source(&p))).await;
    scheduler.tick().await;
    assert_eq!(scopes(&p, run).await, 1);
    let (_, snapshot) = scope_of(&p, run).await;
    assert_eq!(
        released_restore(&snapshot, &stored_items(&p, run).await),
        Ok(()),
        "a scope without a chain is what v0.2.0 wrote"
    );

    // A chained blob scope: refused as invalid storage, never taken for a plain manifest or an array.
    let chained = p
        .publish(
            "operators-old-chained",
            &chained_ir(
                "operators-old-chained",
                "input",
                &json!([{"filter": {"where": "element.v > 0"}}]),
                10,
            ),
        )
        .await;
    let blob = upload(&p, source).await;
    let run = start_with_blob(&p, &chained, blob["id"].as_str().expect("id")).await;
    scheduler.tick().await;
    keeper.leave().await;
    let (count, snapshot) = scope_of(&p, run).await;
    assert_eq!(count, 2);
    let items = stored_items(&p, run).await;
    assert_eq!(released_manifest(&snapshot), None);
    assert_eq!(
        released_restore(&snapshot, &items),
        Err("collection snapshot payload"),
        "the v0.2.0 restore refuses the chained manifest as invalid storage"
    );
}
