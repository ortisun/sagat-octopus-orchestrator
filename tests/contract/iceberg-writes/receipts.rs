//! The receipts of Iceberg writes and destination writes (FR-035-027, FR-035-043): the schema accepts the
//! samples, and refuses a field it does not name. A receipt never carries a record value, a dropped field's name
//! or an object key, and the way the schema keeps that true is `unevaluatedProperties: false` on every branch.

use std::collections::BTreeSet;

use jsonschema::Validator;
use serde_json::{Value, json};

use super::{Edit, apply, fixtures, fragments, json_files, read_json, refusal};

fn receipts() -> Validator {
    let schema = read_json(&fragments().join("receipts.schema.json"));
    jsonschema::options()
        .build(&schema)
        .expect("the receipt schema compiles without external retrieval")
}

fn samples() -> Vec<(String, Value)> {
    json_files(&fixtures().join("receipts"))
}

fn sample(name: &str) -> Value {
    samples()
        .into_iter()
        .find(|(candidate, _)| candidate == name)
        .unwrap_or_else(|| panic!("no receipt sample named {name}"))
        .1
}

#[test]
fn every_sample_is_accepted() {
    let samples = samples();
    assert!(samples.len() >= 5, "the 5 planning samples, one per kind");
    let validator = receipts();
    let refused: Vec<String> = samples
        .iter()
        .filter_map(|(name, receipt)| {
            refusal(&validator, receipt).map(|reason| format!("{name}: {reason}"))
        })
        .collect();
    assert!(
        refused.is_empty(),
        "refused samples:\n{}",
        refused.join("\n")
    );
}

#[test]
fn the_samples_cover_every_kind_and_the_continued_failure() {
    let kinds: BTreeSet<String> = samples()
        .iter()
        .filter(|(_, receipt)| receipt.get("failureCode").is_none())
        .map(|(_, receipt)| receipt["kind"].as_str().expect("a kind").to_string())
        .collect();
    assert_eq!(
        kinds,
        [
            "iceberg",
            "postgresql",
            "object_storage",
            "clickhouse",
            "broker_stream"
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    assert!(
        samples()
            .iter()
            .any(|(_, receipt)| receipt["outcome"] == "FAILED_CONTINUED"),
        "a destination ended by the continue policy has a receipt"
    );
}

/// The injected field is the leak the schema exists to stop: a value, a field name or an object key riding along in
/// a field nobody named.
#[test]
fn an_injected_extra_field_is_refused_in_every_sample() {
    let validator = receipts();
    let accepted: Vec<String> = samples()
        .into_iter()
        .filter_map(|(name, mut receipt)| {
            receipt["recordValues"] = json!(["ada@example.com"]);
            refusal(&validator, &receipt).is_none().then_some(name)
        })
        .collect();
    assert!(
        accepted.is_empty(),
        "an unnamed field is accepted in: {accepted:?}"
    );
}

#[test]
fn a_field_of_another_kind_is_refused() {
    let mut receipt = sample("clickhouse-inserted");
    receipt["rowsAdded"] = json!("1");
    assert!(refusal(&receipts(), &receipt).is_some());
}

#[test]
fn each_malformed_receipt_is_a_sample_plus_one_edit_that_is_refused() {
    let validator = receipts();
    let cases: Vec<(&str, &str, Edit)> = vec![
        (
            "a count with a leading zero",
            "iceberg-committed",
            Edit::Set("/rowsAdded", json!("01")),
        ),
        (
            "a negative count",
            "iceberg-committed",
            Edit::Set("/rowsAdded", json!("-1")),
        ),
        (
            "a count as a JSON number",
            "iceberg-committed",
            Edit::Set("/rowsAdded", json!(1000)),
        ),
        (
            "a count of twenty digits",
            "iceberg-committed",
            Edit::Set("/bytesWritten", json!("10000000000000000000")),
        ),
        (
            "an outcome outside the Iceberg set",
            "iceberg-committed",
            Edit::Set("/outcome", json!("ROLLED_BACK")),
        ),
        (
            "a format version 3",
            "iceberg-committed",
            Edit::Set("/formatVersion", json!(3)),
        ),
        (
            "a snapshot id that is not a number",
            "iceberg-committed",
            Edit::Set("/snapshotId", json!("abc")),
        ),
        (
            "no effect key",
            "iceberg-committed",
            Edit::Remove("/effectKey"),
        ),
        (
            "no lookup flag",
            "iceberg-committed",
            Edit::Remove("/foundByLookup"),
        ),
        ("no mode", "iceberg-committed", Edit::Remove("/mode")),
        (
            "a PostgreSQL outcome other than committed",
            "postgresql-upsert",
            Edit::Set("/outcome", json!("INSERTED")),
        ),
        (
            "a PostgreSQL mode of append",
            "postgresql-upsert",
            Edit::Set("/mode", json!("append")),
        ),
        (
            "an object-name hash that is not hex",
            "object-storage-already-present",
            Edit::Set("/objectNameSha256", json!("not-a-hash")),
        ),
        (
            "a ClickHouse outcome of delivered",
            "clickhouse-inserted",
            Edit::Set("/outcome", json!("DELIVERED")),
        ),
        (
            "a failure code of 65 characters",
            "failed-continued",
            Edit::Set("/failureCode", json!("X".repeat(65))),
        ),
        (
            "a failed receipt without a failure code",
            "failed-continued",
            Edit::Remove("/failureCode"),
        ),
        (
            "a failed receipt of a kind that is not a destination",
            "failed-continued",
            Edit::Set("/kind", json!("mysql")),
        ),
    ];
    assert!(cases.len() >= 14);

    let mut problems = Vec::new();
    for (name, base, edit) in &cases {
        let mut receipt = sample(base);
        assert_eq!(
            refusal(&validator, &receipt),
            None,
            "{name}: its base sample {base} must be accepted"
        );
        apply(&mut receipt, edit);
        if refusal(&validator, &receipt).is_none() {
            problems.push(*name);
        }
    }
    assert!(
        problems.is_empty(),
        "accepted, but must be refused: {problems:?}"
    );
}
