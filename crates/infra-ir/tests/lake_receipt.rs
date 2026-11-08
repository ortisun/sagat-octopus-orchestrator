//! The server rebuilds the receipt of an Iceberg write before it stores one (feature 035, T035-059, D035-142): the runner's JSON is read
//! into typed fields that accept only what the contract names, checked against the step, and written again from those fields.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use sagat_application::database::{ExpectedReceipt, InvalidLakeReceipt, LakeReceiptCodec};
use sagat_domain::database::WriteMode;
use sagat_infra_ir::JsonLakeReceiptCodec;
use serde_json::{Value, json};

const KEY: &str = "run-7f3a/load/1";

fn sample(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/receipts")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).expect("a fixture")).expect("JSON")
}

fn expected() -> ExpectedReceipt<'static> {
    ExpectedReceipt {
        effect_key: KEY,
        reference: "main",
        mode: WriteMode::Append,
    }
}

fn rebuild(receipt: &Value) -> Result<Vec<u8>, InvalidLakeReceipt> {
    JsonLakeReceiptCodec.rebuild(&serde_json::to_vec(receipt).unwrap(), &expected())
}

#[test]
fn the_receipt_of_a_commit_is_stored_as_the_typed_fields_it_holds() {
    let committed = sample("iceberg-committed.json");
    let rebuilt = rebuild(&committed).expect("a receipt of this step");
    let read: Value = serde_json::from_slice(&rebuilt).expect("JSON");
    assert_eq!(read, committed, "nothing is lost and nothing is added");
    // Whitespace and the order of the members are the runner's, not the platform's: the bytes stored are the same for both.
    let spaced = serde_json::to_vec_pretty(&committed).unwrap();
    assert_eq!(
        JsonLakeReceiptCodec.rebuild(&spaced, &expected()),
        Ok(rebuilt.clone())
    );
    let mut reordered = String::from("{");
    let members = committed.as_object().unwrap();
    for (n, (name, value)) in members.iter().rev().enumerate() {
        if n > 0 {
            reordered.push(',');
        }
        reordered.push_str(&format!("{}:{value}", json!(name)));
    }
    reordered.push('}');
    assert_eq!(
        JsonLakeReceiptCodec.rebuild(reordered.as_bytes(), &expected()),
        Ok(rebuilt)
    );
}

#[test]
fn an_empty_batch_that_committed_nothing_is_a_receipt_of_the_step() {
    let mut empty = sample("iceberg-committed.json");
    empty["outcome"] = json!("EMPTY_NO_COMMIT");
    empty.as_object_mut().unwrap().remove("snapshotId");
    assert!(rebuild(&empty).is_ok());
}

#[test]
fn a_receipt_about_another_step_is_refused() {
    for (member, value) in [
        ("effectKey", json!("run-7f3a/other/1")),
        ("ref", json!("audit")),
        ("mode", json!("overwrite")),
    ] {
        let mut receipt = sample("iceberg-committed.json");
        receipt[member] = value;
        assert_eq!(
            rebuild(&receipt),
            Err(InvalidLakeReceipt::Mismatch),
            "{member}"
        );
    }
}

#[test]
fn a_receipt_of_another_kind_or_outcome_is_refused() {
    assert_eq!(
        rebuild(&sample("object-storage-already-present.json")),
        Err(InvalidLakeReceipt::Kind),
        "an Iceberg write does not produce an object-storage receipt"
    );
    let mut reverted = sample("iceberg-committed.json");
    reverted["outcome"] = json!("REVERTED");
    assert_eq!(
        rebuild(&reverted),
        Err(InvalidLakeReceipt::Outcome),
        "the revert of a commit is the receipt of a compensation, not of the step"
    );
    for name in [
        "postgresql-upsert.json",
        "clickhouse-inserted.json",
        "broker-stream-delivered.json",
        "failed-continued.json",
    ] {
        assert_eq!(
            rebuild(&sample(name)),
            Err(InvalidLakeReceipt::Malformed),
            "{name}: written by the runner or the platform, never by a host"
        );
    }
}

#[test]
fn what_the_contract_does_not_name_is_refused_and_not_forwarded() {
    // a record value, a file location or an object key cannot ride along in a member the schema does not have
    let mut extra = sample("iceberg-committed.json");
    extra["firstRow"] = json!({"email": "someone@example.com"});
    assert_eq!(rebuild(&extra), Err(InvalidLakeReceipt::Malformed));
    let mut location = sample("iceberg-committed.json");
    location["dataFile"] = json!("s3://lake/warehouse/data/00000.parquet");
    assert_eq!(rebuild(&location), Err(InvalidLakeReceipt::Malformed));
    // a count is a canonical decimal string
    let mut leading = sample("iceberg-committed.json");
    leading["rowsAdded"] = json!("007");
    assert_eq!(rebuild(&leading), Err(InvalidLakeReceipt::Malformed));
    let mut number = sample("iceberg-committed.json");
    number["rowsAdded"] = json!(1000);
    assert_eq!(rebuild(&number), Err(InvalidLakeReceipt::Malformed));
    // not JSON at all, and JSON that is not an object
    for text in [&b"not json"[..], b"[]", b"null", b""] {
        assert_eq!(
            JsonLakeReceiptCodec.rebuild(text, &expected()),
            Err(InvalidLakeReceipt::Malformed)
        );
    }
}

#[test]
fn a_receipt_is_at_most_sixteen_kibibytes() {
    let mut padded = serde_json::to_vec(&sample("iceberg-committed.json")).unwrap();
    padded.extend(std::iter::repeat_n(b' ', 16 * 1024));
    assert_eq!(
        JsonLakeReceiptCodec.rebuild(&padded, &expected()),
        Err(InvalidLakeReceipt::TooLarge)
    );
}
