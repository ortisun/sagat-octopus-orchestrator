//! The independent Iceberg reader (T035-005, D035-026, D035-072).
//!
//! The write steps run in the Python lake runtime. A test must not check them with the code that wrote the
//! data, so this reader goes through another implementation (iceberg-rust, a test-only dependency enforced by
//! `scripts/check-architecture.py`): it opens a table from the metadata file the catalog points at, and returns
//! what a consumer would see, namely the rows, the snapshots with their summaries and the named references.
//!
//! It reads. It never commits, never creates a table and never talks to the catalog.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_cast::display::array_value_to_string;
use futures_util::TryStreamExt;
use iceberg::TableIdent;
use iceberg::io::{
    FileIO, FileIOBuilder, S3_ACCESS_KEY_ID, S3_ENDPOINT, S3_PATH_STYLE_ACCESS, S3_REGION,
    S3_SECRET_ACCESS_KEY,
};
use iceberg::table::StaticTable;
use iceberg_storage_opendal::OpenDalStorageFactory;
use serde_json::Value;

use super::bounded::{START_LIMIT, within};
use super::lake::{ACCESS_KEY, REGION, SECRET_KEY};

/// One row: column name to the value printed as the engine prints it, `None` for null.
pub type Row = BTreeMap<String, Option<String>>;

/// What the table metadata says about one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotView {
    pub id: i64,
    pub parent: Option<i64>,
    pub sequence_number: i64,
    /// `append`, `overwrite`, `delete` or `replace`.
    pub operation: String,
    /// The summary properties other than `operation` (`added-records`, `total-records`, ...).
    pub summary: BTreeMap<String, String>,
}

/// A branch or a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefView {
    pub snapshot_id: i64,
    /// `branch` or `tag`.
    pub kind: String,
}

/// Opens tables from the metadata file location the catalog returned.
pub struct LakeReader {
    file_io: FileIO,
}

impl LakeReader {
    /// A reader for an S3-compatible store reached at `s3_endpoint` with the fixture credentials.
    pub fn new(s3_endpoint: &str) -> Self {
        let file_io = FileIOBuilder::new(Arc::new(OpenDalStorageFactory::S3 {
            customized_credential_load: None,
        }))
        .with_prop(S3_ENDPOINT, s3_endpoint)
        .with_prop(S3_ACCESS_KEY_ID, ACCESS_KEY)
        .with_prop(S3_SECRET_ACCESS_KEY, SECRET_KEY)
        .with_prop(S3_REGION, REGION)
        .with_prop(S3_PATH_STYLE_ACCESS, "true")
        .build();
        Self::over(file_io)
    }

    /// A reader over any file IO: the tests of the reader itself use the in-memory one.
    pub fn over(file_io: FileIO) -> Self {
        Self { file_io }
    }

    /// Opens the table whose metadata file is `metadata_location` (`s3://bucket/...`, as the catalog says).
    pub async fn open(&self, metadata_location: &str) -> ReadTable {
        let ident = TableIdent::from_strs(["reader", "table"]).expect("a table identifier");
        let table = within(
            format!("open the table at {metadata_location}"),
            START_LIMIT,
            StaticTable::from_metadata_file(metadata_location, ident, self.file_io.clone()),
        )
        .await
        .unwrap_or_else(|error| panic!("open {metadata_location}: {error}"));

        // The reference map is private to the crate, so it is read from the metadata document itself.
        let document = within(format!("read {metadata_location}"), START_LIMIT, async {
            self.file_io
                .new_input(metadata_location)
                .expect("an input file")
                .read()
                .await
        })
        .await
        .unwrap_or_else(|error| panic!("read {metadata_location}: {error}"));
        let document: Value = serde_json::from_slice(&document).expect("the metadata is JSON");

        ReadTable { table, document }
    }
}

/// An open table, as of the metadata file it was opened from.
pub struct ReadTable {
    table: StaticTable,
    document: Value,
}

impl ReadTable {
    /// The column names of the current schema, in order.
    pub fn columns(&self) -> Vec<String> {
        self.table
            .metadata()
            .current_schema()
            .as_struct()
            .fields()
            .iter()
            .map(|field| field.name.clone())
            .collect()
    }

    /// Every snapshot of the table, oldest sequence number first.
    pub fn snapshots(&self) -> Vec<SnapshotView> {
        let mut views: Vec<SnapshotView> = self
            .table
            .metadata()
            .snapshots()
            .map(|snapshot| SnapshotView {
                id: snapshot.snapshot_id(),
                parent: snapshot.parent_snapshot_id(),
                sequence_number: snapshot.sequence_number(),
                operation: snapshot.summary().operation.as_str().to_string(),
                summary: snapshot
                    .summary()
                    .additional_properties
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            })
            .collect();
        views.sort_by_key(|view| view.sequence_number);
        views
    }

    /// The snapshot the `main` branch points at, if any commit happened.
    pub fn current_snapshot_id(&self) -> Option<i64> {
        self.table.metadata().current_snapshot_id()
    }

    /// The named references (branches and tags) as the metadata document lists them.
    pub fn refs(&self) -> BTreeMap<String, RefView> {
        self.document
            .get("refs")
            .and_then(Value::as_object)
            .map(|refs| {
                refs.iter()
                    .map(|(name, reference)| {
                        (
                            name.clone(),
                            RefView {
                                snapshot_id: reference["snapshot-id"]
                                    .as_i64()
                                    .expect("a reference names a snapshot"),
                                kind: reference["type"]
                                    .as_str()
                                    .expect("a reference has a type")
                                    .to_string(),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The table properties, as written in the metadata.
    pub fn properties(&self) -> BTreeMap<String, String> {
        self.table
            .metadata()
            .properties()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    /// The rows of the current snapshot, sorted by their printed values so a comparison does not depend on the
    /// order of the data files. A table without a snapshot has no rows.
    pub async fn rows(&self) -> Vec<Row> {
        if self.table.metadata().current_snapshot().is_none() {
            return Vec::new();
        }
        let scan = self
            .table
            .scan()
            .select_all()
            .build()
            .unwrap_or_else(|error| panic!("plan the scan: {error}"));
        let batches: Vec<_> = within("read the table rows", START_LIMIT, async {
            scan.to_arrow()
                .await
                .expect("a stream of record batches")
                .try_collect::<Vec<_>>()
                .await
        })
        .await
        .unwrap_or_else(|error| panic!("read the rows: {error}"));

        let mut rows = Vec::new();
        for batch in batches {
            let schema = batch.schema();
            for index in 0..batch.num_rows() {
                let mut row = Row::new();
                for (position, field) in schema.fields().iter().enumerate() {
                    let column = batch.column(position);
                    let value = if column.is_null(index) {
                        None
                    } else {
                        Some(array_value_to_string(column, index).expect("a printable cell"))
                    };
                    row.insert(field.name().clone(), value);
                }
                rows.push(row);
            }
        }
        rows.sort();
        rows
    }
}
