"""The request line of an `icebergWrite`, parsed (feature 035: T035-057, `contracts/lake-runtime.md`, D035-062).

The runner writes one JSON line; the host reads it before it does anything else. The runner is the platform's own, but
what the line carries comes from definitions people write (a table, a mapping, a ref) and from a connection that holds
credentials, so every field is checked for its type, its shape and its bounds, a field the host does not know is a
protocol break, and a break is answered with `LAKE_HOST_PROTOCOL` and nothing else: no field name, no value.
"""

from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

import pytest

from sagat_lake.failures import Failure
from sagat_lake.request import parse_write

ATTEMPT = "0190a5e2-7b3c-7d2e-9f10-aabbccddeeff"


def document(**changes: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "protocol": "sagat.lake.v1",
        "operation": "icebergWrite",
        "effectKey": "8f1c0000:save-orders",
        "firstAttemptStartedAt": "2026-10-27T18:02:11.204Z",
        "snapshotProperties": {
            "sagat.effect-key": "8f1c0000:save-orders",
            "sagat.run-id": "run-1",
            "sagat.step-id": "save-orders",
            "sagat.workflow": "orders-sync",
            "sagat.workflow-version": "12",
        },
        "connection": {
            "catalog": {"type": "rest", "uri": "https://catalog.example/api"},
            "storage": {"type": "s3", "region": "eu-west-1"},
            "warehouseLocations": ["s3://lake/warehouse/"],
            "credentials": {"storage": {"accessKeyId": "AKIA", "secretAccessKey": "secret"}},
        },
        "relay": {"socket": f"/work/{ATTEMPT}/relay.sock", "token": "t" * 40},
        "table": {"namespace": ["sales"], "name": "orders"},
        "ref": "main",
        "mode": "append",
        "overwrite": None,
        "upsert": None,
        "inputs": [{"file": "in/inline.ndjson", "format": "ndjson"}, {"file": "in/blob-0.ndjson", "format": "ndjson"}],
        "mapping": [{"field": "/amount", "column": "amount"}, {"field": "/a~1b/c~0d", "column": "ab"}],
        "ignoreUnknownFields": True,
        "limits": {
            "maxCommitRetries": 4,
            "maxDecodedBytes": "17179869184",
            "maxMetadataBytes": "67108864",
            "deadlineAt": "2026-10-27T18:12:11Z",
        },
    }
    return {**base, **changes}


def attempt_dir(tmp_path: Path) -> Path:
    path = tmp_path / ATTEMPT
    path.mkdir(parents=True)
    return path


def refused(tmp_path: Path, doc: dict[str, Any]) -> Failure:
    with pytest.raises(Failure) as caught:
        parse_write(doc, attempt_dir=attempt_dir(tmp_path))
    failure = caught.value
    assert (failure.code, failure.reason, failure.column) == ("LAKE_HOST_PROTOCOL", None, None)
    return failure


# --------------------------------------------------------------------------------------------------------------- valid
def test_a_request_of_the_contract_is_parsed_into_what_the_append_needs(tmp_path: Path) -> None:
    directory = attempt_dir(tmp_path)
    parsed = parse_write(document(), attempt_dir=directory)

    assert parsed.namespace == ("sales",)
    assert parsed.table == "orders"
    assert parsed.deadline_at_ms == 1_793_124_731_000
    append = parsed.append
    assert append.effect_key == "8f1c0000:save-orders"
    assert append.first_attempt_started_at_ms == 1_793_124_131_204
    assert dict(append.snapshot_properties) == document()["snapshotProperties"]
    assert append.ref == "main"
    assert append.attempt_id == ATTEMPT
    assert [(file.path, file.format) for file in append.inputs] == [
        (directory / "in" / "inline.ndjson", "ndjson"),
        (directory / "in" / "blob-0.ndjson", "ndjson"),
    ]
    assert list(append.warehouse_locations) == ["s3://lake/warehouse/"]
    assert list(append.mapping) == [("/amount", "amount"), ("/a~1b/c~0d", "ab")]
    assert append.ignore_unknown_fields is True
    assert append.max_commit_retries == 4
    assert append.max_decoded_bytes == 17_179_869_184
    assert append.max_metadata_bytes == 67_108_864
    assert append.allow_empty is False
    assert append.prior_attempt_may_have_left_files is False


def test_the_connection_is_kept_whole_for_the_catalog_loader_and_never_shown(tmp_path: Path) -> None:
    parsed = parse_write(document(), attempt_dir=attempt_dir(tmp_path))
    assert parsed.connection["credentials"] == {"storage": {"accessKeyId": "AKIA", "secretAccessKey": "secret"}}
    assert "secret" not in repr(parsed)
    assert "AKIA" not in repr(parsed)


def test_the_optional_fields_are_read_when_they_are_there(tmp_path: Path) -> None:
    doc = document(allowEmpty=True, priorAttemptMayHaveLeftFiles=True, mapping=[], ignoreUnknownFields=False)
    append = parse_write(doc, attempt_dir=attempt_dir(tmp_path)).append
    assert append.allow_empty is True
    assert append.prior_attempt_may_have_left_files is True
    assert list(append.mapping) == []
    assert append.ignore_unknown_fields is False


def test_the_time_may_have_no_fraction_and_up_to_six_digits(tmp_path: Path) -> None:
    for text, millis in [
        ("2026-10-27T18:02:11Z", 1_793_124_131_000),
        ("2026-10-27T18:02:11.2Z", 1_793_124_131_200),
        ("2026-10-27T18:02:11.204999Z", 1_793_124_131_204),
    ]:
        parsed = parse_write(document(firstAttemptStartedAt=text), attempt_dir=attempt_dir(tmp_path / str(millis)))
        assert parsed.append.first_attempt_started_at_ms == millis


# ------------------------------------------------------------------------------------------------------------ refused
def test_the_document_the_tests_start_from_is_valid(tmp_path: Path) -> None:
    parse_write(document(), attempt_dir=attempt_dir(tmp_path))


@pytest.mark.parametrize("key", sorted(document()))
def test_a_field_of_the_contract_that_is_missing_is_a_protocol_break_except_the_optional_ones(
    tmp_path: Path, key: str
) -> None:
    doc = document()
    del doc[key]
    optional = {"overwrite", "upsert", "relay", "mapping", "ignoreUnknownFields"}
    if key in optional:
        parse_write(doc, attempt_dir=attempt_dir(tmp_path))
    else:
        refused(tmp_path, doc)


def test_a_field_the_host_does_not_know_is_a_protocol_break(tmp_path: Path) -> None:
    refused(tmp_path, document(extra="x"))


@pytest.mark.parametrize(
    ("changes", "why"),
    [
        ({"mode": "overwrite"}, "overwrite is not served yet"),
        ({"mode": "upsert"}, "upsert is not served yet"),
        ({"mode": "merge"}, "no such mode"),
        ({"mode": None}, "no mode"),
        ({"overwrite": {"scope": "table"}}, "an overwrite section with the append mode"),
        ({"upsert": {"keys": ["id"]}}, "an upsert section with the append mode"),
        ({"effectKey": ""}, "empty key"),
        ({"effectKey": "k" * 513}, "key too long"),
        ({"effectKey": "a\nb"}, "control character in the key"),
        ({"effectKey": 5}, "key not text"),
        ({"firstAttemptStartedAt": "2026-10-27 18:02:11"}, "time without T and Z"),
        ({"firstAttemptStartedAt": "2026-10-27T18:02:11+00:00"}, "time with an offset"),
        ({"firstAttemptStartedAt": "2026-13-27T18:02:11Z"}, "month 13"),
        ({"firstAttemptStartedAt": "2026-10-27T18:02:11.1234567Z"}, "seven digits of fraction"),
        ({"firstAttemptStartedAt": 1}, "time not text"),
        ({"ref": ""}, "empty ref"),
        ({"ref": "r" * 256}, "ref too long"),
        ({"ref": "é" * 128}, "ref of 256 bytes"),
        ({"ref": "a\tb"}, "control character in the ref"),
        ({"ref": 5}, "ref not text"),
        ({"table": {"namespace": [], "name": "orders"}}, "no namespace"),
        ({"table": {"namespace": ["sales"], "name": ""}}, "empty table name"),
        ({"table": {"namespace": [""], "name": "orders"}}, "empty namespace level"),
        ({"table": {"namespace": ["a"] * 9, "name": "orders"}}, "namespace too deep"),
        ({"table": {"namespace": ["sales"], "name": "o" * 256}}, "table name too long"),
        ({"table": {"namespace": ["sales"], "name": "a\x00b"}}, "NUL in the name"),
        ({"table": {"namespace": "sales", "name": "orders"}}, "namespace not a list"),
        # `requests` removes dot segments from the path, so these would send the catalog request somewhere else
        ({"table": {"namespace": ["."], "name": "orders"}}, "a namespace level that is a dot segment"),
        ({"table": {"namespace": ["sales", ".."], "name": "orders"}}, "a namespace level that is a double dot"),
        ({"table": {"namespace": [".."], "name": "orders"}}, "a single namespace level that is a double dot"),
        ({"table": {"namespace": ["sales"], "name": "."}}, "a table named by a dot segment"),
        ({"table": {"namespace": ["sales"], "name": ".."}}, "a table named by a double dot"),
        ({"table": {"namespace": ["sales"], "name": "orders", "extra": 1}}, "unknown table field"),
        ({"inputs": []}, "no input"),
        ({"inputs": [{"file": "in/a.ndjson", "format": "csv"}]}, "no such format"),
        ({"inputs": [{"file": "../etc/passwd", "format": "ndjson"}]}, "a way out"),
        ({"inputs": [{"file": "in/../relay.token", "format": "ndjson"}]}, "a way out below in/"),
        ({"inputs": [{"file": "/etc/passwd", "format": "ndjson"}]}, "absolute"),
        ({"inputs": [{"file": "relay.token", "format": "ndjson"}]}, "outside in/"),
        ({"inputs": [{"file": "in/sub/a.ndjson", "format": "ndjson"}]}, "a subdirectory"),
        ({"inputs": [{"file": "in/a b.ndjson", "format": "ndjson"}]}, "a space"),
        ({"inputs": [{"file": "in/.hidden", "format": "ndjson"}]}, "a dot file"),
        ({"inputs": [{"file": "in/a.ndjson"}]}, "no format"),
        ({"inputs": [{"file": "in/a.ndjson", "format": "ndjson", "extra": 1}]}, "unknown input field"),
        ({"inputs": [{"file": "in/a.ndjson", "format": "ndjson"}] * 33}, "more inputs than a step has"),
        ({"mapping": [{"field": "amount", "column": "amount"}]}, "a pointer with no slash"),
        ({"mapping": [{"field": "/a~2", "column": "amount"}]}, "a bad escape"),
        ({"mapping": [{"field": "/a~", "column": "amount"}]}, "a dangling tilde"),
        ({"mapping": [{"field": "/a", "column": ""}]}, "an empty column"),
        ({"mapping": [{"field": "/a", "column": "c" * 256}]}, "a column too long"),
        ({"mapping": [{"field": "/a", "column": "a\nb"}]}, "a column with a control character"),
        ({"mapping": [{"field": "/a" * 300, "column": "c"}]}, "a pointer too long"),
        ({"mapping": [{"field": "/" + "é" * 256, "column": "c"}]}, "a pointer of 513 bytes"),
        ({"mapping": [{"field": "/a" * 17, "column": "c"}]}, "a pointer deeper than 16"),
        ({"mapping": [{"field": "/a", "column": "c"}] * 1025}, "more mappings than a table has columns"),
        ({"mapping": [{"field": "/a"}]}, "no column"),
        ({"mapping": {"field": "/a", "column": "c"}}, "not a list"),
        ({"mapping": 5}, "a number"),
        ({"mapping": "x"}, "text"),
        ({"mapping": None}, "null"),
        ({"mapping": ["x"]}, "an entry that is not an object"),
        ({"mapping": [{"field": "/a", "column": "c", "extra": 1}]}, "an unknown mapping field"),
        ({"mapping": [{"field": "/\ud800", "column": "c"}]}, "a lone surrogate in a pointer"),
        ({"ref": "\ud800"}, "a lone surrogate in the ref"),
        ({"ignoreUnknownFields": 1}, "not a boolean"),
        ({"allowEmpty": "yes"}, "not a boolean"),
        ({"priorAttemptMayHaveLeftFiles": 0}, "not a boolean"),
        ({"snapshotProperties": {"added-records": "1"}}, "a key PyIceberg owns"),
        ({"snapshotProperties": {"sagat.effect-key": 1}}, "a property that is not text"),
        ({"snapshotProperties": {"sagat.other": "x"}}, "a sagat key the host does not know"),
        ({"snapshotProperties": {"sagat.run-id": "x" * 1025}}, "a value too long"),
        ({"snapshotProperties": {"sagat.run-id": "a\nb"}}, "a control character"),
        ({"snapshotProperties": []}, "not an object"),
        ({"protocol": "sagat.lake.v2"}, "another protocol"),
        ({"operation": "objectWrite"}, "another operation"),
    ],
)
def test_a_field_that_does_not_hold_what_the_contract_says_is_a_protocol_break(
    tmp_path: Path, changes: dict[str, Any], why: str
) -> None:
    assert why
    refused(tmp_path, document(**changes))


@pytest.mark.parametrize(
    ("section", "changes"),
    [
        ("limits", {"maxCommitRetries": -1}),
        ("limits", {"maxCommitRetries": 17}),
        ("limits", {"maxCommitRetries": True}),
        ("limits", {"maxCommitRetries": "4"}),
        ("limits", {"maxDecodedBytes": 17179869184}),
        ("limits", {"maxDecodedBytes": "-1"}),
        ("limits", {"maxDecodedBytes": "007"}),
        ("limits", {"maxDecodedBytes": "9223372036854775808"}),
        ("limits", {"maxDecodedBytes": "1" * 20}),
        ("limits", {"maxDecodedBytes": " 5"}),
        ("limits", {"maxMetadataBytes": "abc"}),
        ("limits", {"maxMetadataBytes": ""}),
        ("limits", {"deadlineAt": "tomorrow"}),
        ("limits", {"extra": 1}),
        ("connection", {"catalog": "rest"}),
        ("connection", {"storage": None}),
        ("connection", {"credentials": ["a"]}),
        ("connection", {"credentials": {"k": 1}}),
        ("connection", {"credentials": {"k": {"a": "b"}}}),  # a secret document the host does not know
        ("connection", {"credentials": {"storage": "x"}}),
        ("connection", {"credentials": {"storage": {"k": 1}}}),
        ("connection", {"credentials": {"catalog": ["a"]}}),
        ("connection", {"credentials": {"catalog": {"token": None}}}),
        ("connection", {"warehouseLocations": []}),
        ("connection", {"warehouseLocations": ["s3://lake/"] * 17}),
        ("connection", {"warehouseLocations": [5]}),
        ("connection", {"warehouseLocations": "s3://lake/"}),
        ("connection", {"extra": 1}),
    ],
)
def test_a_section_with_a_wrong_field_is_a_protocol_break(
    tmp_path: Path, section: str, changes: dict[str, Any]
) -> None:
    doc = document()
    doc[section] = {**copy.deepcopy(doc[section]), **changes}
    refused(tmp_path, doc)


@pytest.mark.parametrize("section_key", ["maxCommitRetries", "maxDecodedBytes", "maxMetadataBytes", "deadlineAt"])
def test_a_limit_that_is_missing_is_a_protocol_break(tmp_path: Path, section_key: str) -> None:
    doc = document()
    del doc["limits"][section_key]
    refused(tmp_path, doc)


@pytest.mark.parametrize("section_key", ["catalog", "storage", "warehouseLocations", "credentials"])
def test_a_part_of_the_connection_that_is_missing_is_a_protocol_break(tmp_path: Path, section_key: str) -> None:
    doc = document()
    del doc["connection"][section_key]
    refused(tmp_path, doc)


@pytest.mark.parametrize("name", ["x" * 61, "-lead", "a.b", "a b", "ä"])
def test_an_attempt_directory_that_is_not_named_by_an_attempt_id_is_a_protocol_break(tmp_path: Path, name: str) -> None:
    directory = tmp_path / name
    directory.mkdir()
    with pytest.raises(Failure) as caught:
        parse_write(document(), attempt_dir=directory)
    assert caught.value.code == "LAKE_HOST_PROTOCOL"


def test_a_document_that_is_not_an_object_is_a_protocol_break(tmp_path: Path) -> None:
    bad_documents: list[object] = [None, [], "x", 5]
    for bad in bad_documents:
        with pytest.raises(Failure) as caught:
            parse_write(bad, attempt_dir=attempt_dir(tmp_path / f"d{bad!r}"))
        assert caught.value.code == "LAKE_HOST_PROTOCOL"


def test_a_failure_carries_nothing_of_the_request(tmp_path: Path) -> None:
    failure = refused(tmp_path, document(effectKey="secret-key\n"))
    assert str(failure) == "LAKE_HOST_PROTOCOL"
    assert failure.as_member() == {"code": "LAKE_HOST_PROTOCOL"}


@pytest.mark.parametrize(
    "changes",
    [
        {"ref": "feature/x"},
        {"ref": "a b"},
        {"ref": "../main"},
        {"ref": "é" * 127 + "a"},
        {"ref": "r" * 255},
        {"table": {"namespace": ["a"] * 8, "name": "n" * 255}},
        {"table": {"namespace": ["sales ünï"], "name": "orders 2026"}},
        {"table": {"namespace": ["...", "..a", "a..", ".a", "a.b"], "name": "..."}},
        {"mapping": [{"field": "/a" * 16, "column": "c" * 255}]},
        {"mapping": [{"field": "/" + "é" * 255, "column": "ü"}]},
        {"mapping": [{"field": "/a", "column": "c"}] * 1024},
    ],
)
def test_what_the_platform_publishes_is_accepted_up_to_the_limits_of_the_domain(
    tmp_path: Path, changes: dict[str, Any]
) -> None:
    parse_write(document(**changes), attempt_dir=attempt_dir(tmp_path))


def test_a_parquet_input_is_read_as_a_parquet_input(tmp_path: Path) -> None:
    inputs = [{"file": "in/blob-0.parquet", "format": "parquet"}, {"file": "in/inline.ndjson", "format": "ndjson"}]
    directory = attempt_dir(tmp_path)
    append = parse_write(document(inputs=inputs), attempt_dir=directory).append
    assert [(file.path, file.format) for file in append.inputs] == [
        (directory / "in" / "blob-0.parquet", "parquet"),
        (directory / "in" / "inline.ndjson", "ndjson"),
    ]


@pytest.mark.parametrize("format", ["Parquet", "PARQUET", "parquet ", "avro", "orc", "csv", "json", "", None, 5])
def test_a_format_the_host_does_not_read_is_a_break(tmp_path: Path, format: Any) -> None:
    refused(tmp_path, document(inputs=[{"file": "in/a.bin", "format": format}]))
