//! The attempt input a server builds is the attempt input the runner reads (feature 035, T035-056, D035-121, D035-131).
//!
//! The two ends cannot link each other (the server is `sagat-application`, the reader is `sagat-infra-lake`), so this suite, which
//! depends on both, is where they are held together: the builder reproduces **the document the runner's corpus is built on**, member
//! for member; everything it builds is accepted by the runner's reader and read back as what was put in; the bounds the two share
//! (the size of a document, the limits' ranges) are equal; and what the builder refuses it refuses itself, with a name, because a
//! document the runner would refuse is not worth leasing.

use sagat_application::worker::lake_input::{
    BlobFormat, LakeBlob, LakeFacts, LakeInputError, LakeRecords, MAX_INPUT_BYTES, SCHEMA,
    lake_attempt_input,
};
use sagat_domain::blobs::BlobId;
use sagat_domain::database::{
    ColumnMapping, ConnectionRef, EffectiveLakeLimits, IcebergWrite, IcebergWriteParts, LakeLimits,
    RecordSource, TableIdentifier, WriteIntent,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::Expression;
use sagat_infra_lake::protocol::{
    AttemptInput, InputFormat, MAX_INPUT_BYTES as READER_MAX, Records,
};
use serde_json::{Value, json};

use super::{fixtures, read_json};

const BLOB_A: &str = "0192f5aa-0000-7000-8000-000000000001";
const BLOB_B: &str = "0192f5aa-0000-7000-8000-000000000002";

fn corpus_base() -> Value {
    read_json(&fixtures().join("attempt-input/cases.json"))["base"].clone()
}

fn write(namespace: &[&str], mapping: &[(&str, &str)], ignore_unknown: bool) -> IcebergWrite {
    IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("slug"), None).expect("ref"),
        table: TableIdentifier::new(
            namespace.iter().map(|s| (*s).to_owned()).collect(),
            "orders",
        )
        .expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::Inline(Expression::parse("input.rows").expect("expression")),
        mapping: mapping
            .iter()
            .map(|(field, column)| ColumnMapping::new(field, column).expect("mapping"))
            .collect(),
        ignore_unknown_fields: ignore_unknown,
        limits: LakeLimits::default(),
        revert: false,
    })
    .expect("a write")
}

fn base_write() -> IcebergWrite {
    write(&["sales"], &[("/amount", "amount")], true)
}

fn facts() -> LakeFacts<'static> {
    LakeFacts {
        effect_key: "run-7f3a/load/1",
        first_attempt_started_at: time::macros::datetime!(2026-11-05 10:00:00 UTC),
        run_id: "run-7f3a",
        step_id: "load",
        workflow: "orders-sync",
        workflow_version: "12",
        prior_attempt_may_have_left_files: false,
        allow_empty: false,
    }
}

const BASE_LIMITS: EffectiveLakeLimits = EffectiveLakeLimits {
    max_record_bytes: 1_048_576,
    max_records: 1_000_000,
    timeout_ms: 600_000,
};

fn inline() -> LakeRecords {
    LakeRecords::Inline(vec![json!({"amount": "12.50", "sku": "A-1"})])
}

fn build(
    write: &IcebergWrite,
    facts: &LakeFacts<'_>,
    records: LakeRecords,
    limits: EffectiveLakeLimits,
) -> Vec<u8> {
    lake_attempt_input(write, facts, records, limits).expect("a document")
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("JSON")
}

#[test]
fn the_builder_reproduces_the_document_the_runners_corpus_is_built_on() {
    let built = build(&base_write(), &facts(), inline(), BASE_LIMITS);
    assert_eq!(json_of(&built), corpus_base());
    assert_eq!(json_of(&built)["schema"], SCHEMA);
}

#[test]
fn what_the_builder_makes_the_reader_accepts_and_reads_back_as_what_was_put_in() {
    let input = AttemptInput::read(&build(&base_write(), &facts(), inline(), BASE_LIMITS))
        .expect("accepted");
    assert_eq!(input.effect_key(), "run-7f3a/load/1");
    assert_eq!(input.first_attempt_started_at(), "2026-11-05T10:00:00.000Z");
    assert_eq!(input.table().namespace(), ["sales"]);
    assert_eq!(input.reference(), "main");
    assert_eq!(input.mapping().len(), 1);
    assert_eq!(input.limits().max_records, 1_000_000);
    assert!(matches!(input.records(), Records::Inline(items) if items.len() == 1));
}

#[test]
fn a_great_many_shapes_of_write_are_all_accepted() {
    let deep = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let writes = [
        write(&["sales"], &[], false),
        write(&deep, &[("/a/b/c", "c"), ("/x", "x")], true),
        write(&["sales"], &[("/customer/name", "customer_name")], false),
    ];
    let records = [
        LakeRecords::Inline(vec![]),
        inline(),
        LakeRecords::Inline(vec![
            json!({"nested": {"list": [1, 2, {"deep": null}]}, "unicode": "é\u{1F600}"}),
        ]),
        LakeRecords::Blobs(vec![LakeBlob {
            id: BLOB_A.parse().expect("id"),
            format: BlobFormat::Parquet,
        }]),
    ];
    for write in &writes {
        for records in &records {
            for (prior, allow) in [(false, false), (true, false), (false, true), (true, true)] {
                let mut facts = facts();
                facts.prior_attempt_may_have_left_files = prior;
                facts.allow_empty = allow;
                let built = build(write, &facts, records.clone(), BASE_LIMITS);
                let input = AttemptInput::read(&built)
                    .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&built)));
                assert_eq!(input.table(), write.table());
                assert_eq!(input.mapping(), write.mapping());
                assert_eq!(input.reference(), write.reference());
                // the two flags have no accessor on the reader: they are read in the document
                let document = json_of(&built);
                assert_eq!(document["allowEmpty"], allow);
                assert_eq!(document["priorAttemptMayHaveLeftFiles"], prior);
                assert_eq!(
                    document["ignoreUnknownFields"],
                    write.ignore_unknown_fields()
                );
            }
        }
    }
}

#[test]
fn blobs_are_read_back_with_their_ids_and_formats_in_the_order_given() {
    let blobs = vec![
        LakeBlob {
            id: BLOB_B.parse::<BlobId>().expect("id"),
            format: BlobFormat::Parquet,
        },
        LakeBlob {
            id: BLOB_A.parse::<BlobId>().expect("id"),
            format: BlobFormat::Ndjson,
        },
    ];
    let built = build(
        &base_write(),
        &facts(),
        LakeRecords::Blobs(blobs),
        BASE_LIMITS,
    );
    let input = AttemptInput::read(&built).expect("accepted");
    let Records::Blobs(read) = input.records() else {
        panic!("not blobs");
    };
    let shown: Vec<(String, InputFormat)> = read
        .iter()
        .map(|blob| (blob.id.to_string(), blob.format))
        .collect();
    assert_eq!(
        shown,
        [
            (BLOB_B.to_owned(), InputFormat::Parquet),
            (BLOB_A.to_owned(), InputFormat::Ndjson)
        ]
    );
}

#[test]
fn the_limits_at_both_ends_of_every_range_are_accepted_by_the_reader() {
    for (bytes, records, timeout) in [
        (1024, 1, 1000),
        (2_147_483_648, 100_000_000, 3_600_000),
        (1_048_576, 1_000_000, 600_000),
    ] {
        let limits = EffectiveLakeLimits {
            max_record_bytes: bytes,
            max_records: records,
            timeout_ms: timeout,
        };
        let built = build(&base_write(), &facts(), inline(), limits);
        let input = AttemptInput::read(&built)
            .unwrap_or_else(|error| panic!("{bytes} {records} {timeout}: {error}"));
        assert_eq!(input.limits().max_record_bytes, bytes);
        assert_eq!(input.limits().max_records, records);
        assert_eq!(input.limits().timeout_ms, timeout);
    }
}

#[test]
fn the_effective_limits_of_the_domain_are_always_inside_what_the_reader_takes() {
    // the platform defaults, and every combination of an absent, a lowest and a highest limit on each side
    let ends = [
        LakeLimits::default(),
        LakeLimits {
            max_record_bytes: Some(1024),
            max_records: Some(1),
            timeout_ms: Some(1000),
        },
        LakeLimits {
            max_record_bytes: Some(2_147_483_648),
            max_records: Some(100_000_000),
            timeout_ms: Some(3_600_000),
        },
    ];
    for step in ends {
        for connection in ends {
            let limits = step.effective(connection);
            let built = build(&base_write(), &facts(), inline(), limits);
            AttemptInput::read(&built)
                .unwrap_or_else(|error| panic!("{step:?} {connection:?}: {error}"));
        }
    }
}

#[test]
fn the_bound_on_a_document_is_the_readers_and_the_builder_refuses_one_byte_over_it_itself() {
    assert_eq!(
        MAX_INPUT_BYTES, READER_MAX,
        "the two ends read the same bound"
    );

    // pad one record so that the document is exactly the bound, then one byte more
    let padded = |extra: usize| {
        let probe = build(
            &base_write(),
            &facts(),
            LakeRecords::Inline(vec![json!({"pad": ""})]),
            BASE_LIMITS,
        );
        let fill = MAX_INPUT_BYTES + extra - probe.len();
        lake_attempt_input(
            &base_write(),
            &facts(),
            LakeRecords::Inline(vec![json!({"pad": "x".repeat(fill)})]),
            BASE_LIMITS,
        )
    };
    let exact = padded(0).expect("a document of exactly the bound");
    assert_eq!(exact.len(), MAX_INPUT_BYTES);
    AttemptInput::read(&exact).expect("the reader takes it");
    assert_eq!(padded(1), Err(LakeInputError::TooLarge));
}

#[test]
fn what_the_builder_refuses_it_refuses_by_name() {
    let blob = |id: &str| LakeBlob {
        id: id.parse().expect("id"),
        format: BlobFormat::Ndjson,
    };
    let refuse =
        |records: LakeRecords| lake_attempt_input(&base_write(), &facts(), records, BASE_LIMITS);

    assert_eq!(
        refuse(LakeRecords::Inline(vec![json!(1)])),
        Err(LakeInputError::Record)
    );
    assert_eq!(
        refuse(LakeRecords::Inline(vec![json!({"a": 1}), json!([1])])),
        Err(LakeInputError::Record)
    );
    assert_eq!(
        refuse(LakeRecords::Blobs(vec![])),
        Err(LakeInputError::Blobs)
    );
    assert_eq!(
        refuse(LakeRecords::Blobs(vec![blob(BLOB_A), blob(BLOB_A)])),
        Err(LakeInputError::Blobs)
    );
    let thirty_three: Vec<LakeBlob> = (0..33)
        .map(|n| blob(&format!("0192f5aa-0000-7000-8000-{n:012}")))
        .collect();
    assert_eq!(
        refuse(LakeRecords::Blobs(thirty_three.clone())),
        Err(LakeInputError::Blobs)
    );
    assert!(
        refuse(LakeRecords::Blobs(thirty_three[..32].to_vec())).is_ok(),
        "32 are the most"
    );

    for bad in ["", &"k".repeat(513), "a\u{7}b"] {
        let mut facts = facts();
        facts.effect_key = bad;
        assert_eq!(
            lake_attempt_input(&base_write(), &facts, inline(), BASE_LIMITS),
            Err(LakeInputError::EffectKey)
        );
    }
    let mut longest = facts();
    let key = "k".repeat(512);
    longest.effect_key = &key;
    assert!(lake_attempt_input(&base_write(), &longest, inline(), BASE_LIMITS).is_ok());
}

#[test]
fn an_instant_in_another_offset_is_written_in_utc_to_the_millisecond() {
    let mut facts = facts();
    facts.first_attempt_started_at = time::macros::datetime!(2026-11-05 12:30:15.1239 +02:00);
    let built = build(&base_write(), &facts, inline(), BASE_LIMITS);
    assert_eq!(
        json_of(&built)["firstAttemptStartedAt"],
        "2026-11-05T10:30:15.123Z"
    );
    AttemptInput::read(&built).expect("accepted");
}

#[test]
fn the_document_carries_no_credential_no_relay_and_no_connection() {
    let built = json_of(&build(&base_write(), &facts(), inline(), BASE_LIMITS));
    let members: Vec<&str> = built
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    for forbidden in [
        "credentials",
        "relay",
        "connection",
        "deadlineAt",
        "secrets",
        "token",
    ] {
        assert!(!members.contains(&forbidden), "{forbidden}");
    }
}
