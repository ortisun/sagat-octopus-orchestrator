//! T035-005: the independent reader returns what a consumer of the table would see.
//!
//! Two kinds of case. The first reads a metadata document written by hand into the in-memory file IO, so the
//! snapshots, summaries, references and properties are known exactly. The second opens a table the REST
//! catalog created in the real object store. Rows after a commit need a writer, so the case that reads them
//! arrives with the first write-step test (T035-013); until then the rows of a table that was never committed
//! are the only rows this file can prove.

use iceberg::io::FileIO;
use serde_json::json;

use crate::support::lake::{Column, RestLake};
use crate::support::lake_reader::{LakeReader, RefView, SnapshotView};

/// A format version 2 metadata document with two snapshots (an append, then an overwrite), `main` on the second
/// and a tag on the first.
fn committed_table_metadata() -> String {
    json!({
        "format-version": 2,
        "table-uuid": "9c12d441-03fe-4693-9a96-a0705ddf69c1",
        "location": "memory://warehouse/orders",
        "last-sequence-number": 2,
        "last-updated-ms": 1_700_000_200_000_i64,
        "last-column-id": 2,
        "current-schema-id": 0,
        "schemas": [{
            "type": "struct",
            "schema-id": 0,
            "fields": [
                {"id": 1, "name": "id", "required": true, "type": "long"},
                {"id": 2, "name": "payload", "required": false, "type": "string"}
            ]
        }],
        "default-spec-id": 0,
        "partition-specs": [{"spec-id": 0, "fields": []}],
        "last-partition-id": 999,
        "default-sort-order-id": 0,
        "sort-orders": [{"order-id": 0, "fields": []}],
        "properties": {"write.format.default": "parquet", "sagat.run": "run-7"},
        "current-snapshot-id": 222,
        "snapshots": [
            {
                "snapshot-id": 111,
                "sequence-number": 1,
                "timestamp-ms": 1_700_000_100_000_i64,
                "manifest-list": "memory://warehouse/orders/metadata/snap-111.avro",
                "summary": {
                    "operation": "append",
                    "added-records": "3",
                    "total-records": "3"
                },
                "schema-id": 0
            },
            {
                "snapshot-id": 222,
                "parent-snapshot-id": 111,
                "sequence-number": 2,
                "timestamp-ms": 1_700_000_200_000_i64,
                "manifest-list": "memory://warehouse/orders/metadata/snap-222.avro",
                "summary": {
                    "operation": "overwrite",
                    "added-records": "1",
                    "deleted-records": "3",
                    "total-records": "1"
                },
                "schema-id": 0
            }
        ],
        "snapshot-log": [
            {"snapshot-id": 111, "timestamp-ms": 1_700_000_100_000_i64},
            {"snapshot-id": 222, "timestamp-ms": 1_700_000_200_000_i64}
        ],
        "metadata-log": [],
        "refs": {
            "main": {"snapshot-id": 222, "type": "branch"},
            "first-load": {"snapshot-id": 111, "type": "tag"}
        }
    })
    .to_string()
}

async fn memory_table(location: &str, document: String) -> crate::support::lake_reader::ReadTable {
    let file_io = FileIO::new_with_memory();
    file_io
        .new_output(location)
        .expect("an output file")
        .write(document.into_bytes().into())
        .await
        .expect("write the metadata document");
    LakeReader::over(file_io).open(location).await
}

#[tokio::test]
async fn the_reader_returns_snapshots_with_their_summaries_in_commit_order() {
    let table = memory_table(
        "memory://warehouse/orders/metadata/v3.metadata.json",
        committed_table_metadata(),
    )
    .await;

    let snapshots = table.snapshots();
    assert_eq!(
        snapshots,
        vec![
            SnapshotView {
                id: 111,
                parent: None,
                sequence_number: 1,
                operation: "append".into(),
                summary: [("added-records", "3"), ("total-records", "3")]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
            SnapshotView {
                id: 222,
                parent: Some(111),
                sequence_number: 2,
                operation: "overwrite".into(),
                summary: [
                    ("added-records", "1"),
                    ("deleted-records", "3"),
                    ("total-records", "1"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        ],
        "the operation is its own field and the other summary entries are kept as written"
    );
    assert_eq!(table.current_snapshot_id(), Some(222));
}

#[tokio::test]
async fn the_reader_returns_every_branch_and_tag() {
    let table = memory_table(
        "memory://warehouse/orders/metadata/v3.metadata.json",
        committed_table_metadata(),
    )
    .await;

    let refs = table.refs();
    assert_eq!(refs.len(), 2);
    assert_eq!(
        refs["main"],
        RefView {
            snapshot_id: 222,
            kind: "branch".into()
        }
    );
    assert_eq!(
        refs["first-load"],
        RefView {
            snapshot_id: 111,
            kind: "tag".into()
        },
        "a tag is not lost behind the current snapshot"
    );
}

#[tokio::test]
async fn the_reader_returns_the_schema_and_the_table_properties() {
    let table = memory_table(
        "memory://warehouse/orders/metadata/v3.metadata.json",
        committed_table_metadata(),
    )
    .await;

    assert_eq!(table.columns(), vec!["id", "payload"]);
    let properties = table.properties();
    assert_eq!(properties["sagat.run"], "run-7");
    assert_eq!(properties["write.format.default"], "parquet");
}

#[tokio::test]
#[should_panic(expected = "open memory://warehouse/nothing/metadata/v1.metadata.json")]
async fn opening_a_metadata_file_that_is_not_there_names_the_location() {
    LakeReader::over(FileIO::new_with_memory())
        .open("memory://warehouse/nothing/metadata/v1.metadata.json")
        .await;
}

/// Real containers: a table the catalog created, read from the bucket the catalog wrote it to.
#[tokio::test]
async fn a_table_nobody_wrote_to_has_columns_and_no_snapshot_ref_or_row() {
    let lake = RestLake::start().await;
    let admin = lake.admin();
    admin.create_namespace("reader").await;
    admin
        .create_table(
            "reader",
            "events",
            &[
                Column {
                    id: 1,
                    name: "id",
                    kind: "long",
                    required: true,
                },
                Column {
                    id: 2,
                    name: "payload",
                    kind: "string",
                    required: false,
                },
            ],
        )
        .await;
    let loaded = admin
        .load_table("reader", "events")
        .await
        .expect("the table is in the catalog");
    let location = loaded["metadata-location"]
        .as_str()
        .expect("a metadata location");

    let table = LakeReader::new(&lake.s3_endpoint).open(location).await;

    assert_eq!(table.columns(), vec!["id", "payload"]);
    assert!(table.snapshots().is_empty());
    assert_eq!(table.current_snapshot_id(), None);
    assert!(table.refs().is_empty());
    assert!(table.rows().await.is_empty());
}
