#![allow(clippy::expect_used)]
use sagat_octopus_orchestrator_sdk::{
    Value, json,
    workflow::{
        Builder, ConnectionRef, IcebergCompensation, IcebergLimits, IcebergMapping, IcebergRecords,
        IcebergTable, IcebergWriteOptions, StepOptions,
    },
};

fn base() -> IcebergWriteOptions {
    IcebergWriteOptions {
        connection_ref: ConnectionRef {
            name: "lake-main".into(),
            revision: Some(3),
        },
        table: IcebergTable {
            namespace: vec!["sales".into()],
            name: "orders".into(),
        },
        records: IcebergRecords::Inline("steps.extract.output.rows".into()),
        reference: None,
        mapping: None,
        ignore_unknown_fields: None,
        limits: None,
        compensation: None,
        on_error: None,
    }
}
fn node_of(options: IcebergWriteOptions) -> Value {
    let workflow = Builder::new("lake");
    workflow.iceberg_write("w", options).end("done", None);
    workflow.build().expect("valid workflow")["nodes"][0].clone()
}
fn refused(name: &str, options: IcebergWriteOptions) {
    let workflow = Builder::new("lake");
    workflow.iceberg_write("w", options).end("done", None);
    let error = workflow.build().expect_err(name);
    assert_eq!(error.code, "VALIDATION_FAILED", "{name}");
    assert_eq!(error.message, "invalid iceberg write", "{name}");
}
fn mapped(field: &str, column: &str) -> IcebergMapping {
    IcebergMapping {
        field: field.into(),
        column: column.into(),
    }
}

#[test]
fn matches_the_shared_valid_document() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/ir/valid/append-inline.json");
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(path).expect("fixture")).expect("fixture JSON");
    let workflow = Builder::new(fixture["metadata"]["name"].as_str().expect("name"));
    workflow
        .step("extract", "orders.extract", StepOptions::default())
        .iceberg_write("load", base())
        .end("done", None);
    assert_eq!(workflow.build().expect("valid workflow"), fixture);
}

#[test]
fn the_mode_is_always_append_and_a_minimal_write_carries_nothing_it_was_not_given() {
    let node = node_of(base());
    assert_eq!(node["mode"], "append");
    assert_eq!(node["type"], "icebergWrite");
    for absent in [
        "ref",
        "mapping",
        "limits",
        "compensation",
        "ignoreUnknownFields",
        "retry",
        "secrets",
        "overwrite",
    ] {
        assert!(node.get(absent).is_none(), "{absent}");
    }
}

#[test]
fn every_option_is_carried_and_the_three_record_sources_are_accepted() {
    let workflow = Builder::new("lake");
    workflow
        .iceberg_write(
            "w",
            IcebergWriteOptions {
                reference: Some("staging".into()),
                mapping: Some(vec![
                    mapped("/amount", "amount"),
                    mapped("/customer/name", "customer_name"),
                ]),
                ignore_unknown_fields: Some(true),
                limits: Some(IcebergLimits {
                    max_record_bytes: Some(1024),
                    max_records: Some(100_000_000),
                    timeout_ms: Some(3_600_000),
                }),
                compensation: Some(IcebergCompensation::Revert),
                on_error: Some("recover".into()),
                ..base()
            },
        )
        .step("recover", "orders.recover", StepOptions::default())
        .end("done", None);
    let node = workflow.build().expect("valid workflow")["nodes"][0].clone();
    assert_eq!(node["ref"], "staging");
    assert_eq!(
        node["limits"],
        json!({"maxRecordBytes":1024,"maxRecords":100_000_000,"timeoutMs":3_600_000})
    );
    assert_eq!(node["compensation"], "revert");
    assert_eq!(node["onError"], "recover");
    assert_eq!(node["ignoreUnknownFields"], true);
    for records in [
        IcebergRecords::Inline("input.rows".into()),
        IcebergRecords::Blobs("steps.a.output.parts".into()),
        IcebergRecords::DatabaseResult("read".into()),
    ] {
        node_of(IcebergWriteOptions { records, ..base() });
    }
}

#[test]
fn the_node_is_a_snapshot_later_changes_to_the_options_do_not_reach_it() {
    let mut options = IcebergWriteOptions {
        mapping: Some(vec![mapped("/a", "a")]),
        ..base()
    };
    let workflow = Builder::new("lake");
    workflow
        .iceberg_write("w", options.clone())
        .end("done", None);
    options.table.namespace[0] = "changed".into();
    options.mapping = Some(vec![mapped("/changed", "changed")]);
    let text = workflow.json().expect("valid workflow");
    assert!(!text.contains("changed"), "{text}");
}

#[test]
fn what_the_server_would_refuse_is_refused_here_with_one_fixed_message_that_shows_no_content() {
    let with_table = |namespace: &[&str], name: &str| IcebergWriteOptions {
        table: IcebergTable {
            namespace: namespace.iter().map(|level| (*level).to_owned()).collect(),
            name: name.into(),
        },
        ..base()
    };
    let with_connection = |name: &str, revision: Option<u32>| IcebergWriteOptions {
        connection_ref: ConnectionRef {
            name: name.into(),
            revision,
        },
        ..base()
    };
    let with_mapping = |rows: Vec<IcebergMapping>| IcebergWriteOptions {
        mapping: Some(rows),
        ..base()
    };
    let with_limits = |limits: IcebergLimits| IcebergWriteOptions {
        limits: Some(limits),
        ..base()
    };
    let with_records = |records: IcebergRecords| IcebergWriteOptions { records, ..base() };
    refused(
        "url",
        with_connection("postgres://credential-canary@host/db", None),
    );
    refused("revision zero", with_connection("lake-main", Some(0)));
    refused(
        "revision over i32",
        with_connection("lake-main", Some(2_147_483_648)),
    );
    refused("double dash", with_connection("lake--main", None));
    refused("no namespace", with_table(&[], "orders"));
    refused(
        "nine levels",
        with_table(&["a", "b", "c", "d", "e", "f", "g", "h", "i"], "orders"),
    );
    refused("empty name", with_table(&["sales"], ""));
    refused("dot dot name", with_table(&["sales"], ".."));
    refused("dot level", with_table(&["."], "orders"));
    refused("control in a level", with_table(&["sales\0"], "orders"));
    refused(
        "empty ref",
        IcebergWriteOptions {
            reference: Some(String::new()),
            ..base()
        },
    );
    refused(
        "long ref",
        IcebergWriteOptions {
            reference: Some("r".repeat(256)),
            ..base()
        },
    );
    refused(
        "empty expression",
        with_records(IcebergRecords::Inline(String::new())),
    );
    refused(
        "expression of 1025 bytes",
        with_records(IcebergRecords::Inline("x".repeat(1025))),
    );
    refused(
        "513 two-byte letters",
        with_records(IcebergRecords::Blobs("é".repeat(513))),
    );
    refused(
        "empty database result",
        with_records(IcebergRecords::DatabaseResult(String::new())),
    );
    refused(
        "pointer without slash",
        with_mapping(vec![mapped("amount", "amount")]),
    );
    refused(
        "pointer with empty segment",
        with_mapping(vec![mapped("/a//b", "a")]),
    );
    refused(
        "pointer with trailing slash",
        with_mapping(vec![mapped("/a/", "a")]),
    );
    refused(
        "pointer with a bad escape",
        with_mapping(vec![mapped("/a~2", "a")]),
    );
    refused(
        "pointer ending in a tilde",
        with_mapping(vec![mapped("/a~", "a")]),
    );
    refused(
        "pointer with 17 segments",
        with_mapping(vec![mapped(&format!("/{}a", "a/".repeat(16)), "a")]),
    );
    refused("empty column", with_mapping(vec![mapped("/a", "")]));
    refused(
        "column twice",
        with_mapping(vec![mapped("/a", "c"), mapped("/b", "c")]),
    );
    refused(
        "field twice",
        with_mapping(vec![mapped("/a", "c"), mapped("/a", "d")]),
    );
    refused(
        "zero records",
        with_limits(IcebergLimits {
            max_records: Some(0),
            ..Default::default()
        }),
    );
    refused(
        "too many records",
        with_limits(IcebergLimits {
            max_records: Some(100_000_001),
            ..Default::default()
        }),
    );
    refused(
        "record bytes below the edge",
        with_limits(IcebergLimits {
            max_record_bytes: Some(1023),
            ..Default::default()
        }),
    );
    refused(
        "record bytes above the edge",
        with_limits(IcebergLimits {
            max_record_bytes: Some(2_147_483_649),
            ..Default::default()
        }),
    );
    refused(
        "timeout below the edge",
        with_limits(IcebergLimits {
            timeout_ms: Some(999),
            ..Default::default()
        }),
    );
    refused(
        "timeout above the edge",
        with_limits(IcebergLimits {
            timeout_ms: Some(3_600_001),
            ..Default::default()
        }),
    );
    refused(
        "empty on error",
        IcebergWriteOptions {
            on_error: Some(String::new()),
            ..base()
        },
    );
}

#[test]
fn the_edges_that_are_accepted_are_accepted() {
    let long_pointer = format!("/{}a", "a/".repeat(15));
    for options in [
        IcebergWriteOptions {
            table: IcebergTable {
                namespace: "abcdefgh".chars().map(String::from).collect(),
                name: "n".repeat(255),
            },
            ..base()
        },
        IcebergWriteOptions {
            reference: Some("r".repeat(255)),
            ..base()
        },
        IcebergWriteOptions {
            records: IcebergRecords::Inline("x".repeat(1024)),
            ..base()
        },
        IcebergWriteOptions {
            records: IcebergRecords::Inline("é".repeat(512)),
            ..base()
        },
        IcebergWriteOptions {
            mapping: Some(vec![mapped(&long_pointer, &"c".repeat(255))]),
            ..base()
        },
        IcebergWriteOptions {
            mapping: Some(vec![mapped("/a~0b/c~1d", "c")]),
            ..base()
        },
        IcebergWriteOptions {
            limits: Some(IcebergLimits {
                max_record_bytes: Some(2_147_483_648),
                max_records: Some(1),
                timeout_ms: Some(1000),
            }),
            ..base()
        },
        IcebergWriteOptions {
            connection_ref: ConnectionRef {
                name: "abc".into(),
                revision: None,
            },
            ..base()
        },
    ] {
        node_of(options);
    }
    let empty = node_of(IcebergWriteOptions {
        mapping: Some(vec![]),
        limits: Some(IcebergLimits::default()),
        ..base()
    });
    assert_eq!(empty["mapping"], json!([]));
    assert_eq!(empty["limits"], json!({}));
}

#[test]
fn unknown_members_do_not_decode_so_a_typo_is_not_silently_dropped() {
    for bad in [
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t","catalog":"x"},"records":{"inline":"r"}}),
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t"},"records":{"inline":"r","blobs":"b"}}),
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t"},"records":{"files":"r"}}),
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t"},"records":{"inline":"r"},"mode":"overwrite"}),
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t"},"records":{"inline":"r"},"limits":{"maxCommitRetries":4}}),
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t"},"records":{"inline":"r"},"compensation":"undo"}),
        json!({"connectionRef":{"name":"lake-main"},"table":{"namespace":["a"],"name":"t"},"records":{"inline":"r"},"mapping":[{"field":"/a","column":"c","type":"int"}]}),
    ] {
        assert!(
            serde_json::from_value::<IcebergWriteOptions>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_node_chains_like_any_other_next_is_linked_by_the_flow() {
    let workflow = Builder::new("lake");
    workflow
        .step("extract", "orders.extract", StepOptions::default())
        .iceberg_write("load", base())
        .step("notify", "notify.done", StepOptions::default())
        .end("done", None);
    let nodes = workflow.build().expect("valid workflow")["nodes"].clone();
    assert_eq!(nodes[0]["next"], "load");
    assert_eq!(nodes[1]["next"], "notify");
}
