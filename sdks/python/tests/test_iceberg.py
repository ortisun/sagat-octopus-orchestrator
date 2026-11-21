"""The Iceberg write step stays identical across SDKs; invalid values stay out of errors."""

import json
from pathlib import Path
from typing import Any

import pytest

from sagat_octopus_orchestrator import SagatError, workflow

BASE: dict[str, Any] = {
    "connection_ref": {"name": "lake-main", "revision": 3},
    "table": {"namespace": ["sales"], "name": "orders"},
    "records": {"inline": "steps.extract.output.rows"},
}


def node_of(**changes: Any) -> dict[str, Any]:
    built = workflow("lake").iceberg_write("w", **{**BASE, **changes}).end("done").build()
    node: dict[str, Any] = built["nodes"][0]
    return node


def test_the_iceberg_write_builder_matches_the_shared_valid_document() -> None:
    fixture = json.loads(
        (Path(__file__).parents[3] / "tests/contract/iceberg-writes/fixtures/ir/valid/append-inline.json").read_text()
    )
    built = (
        workflow(fixture["metadata"]["name"])
        .step("extract", "orders.extract")
        .iceberg_write("load", **BASE)
        .end("done")
        .build()
    )
    assert built == fixture


def test_the_mode_is_always_append_and_a_minimal_write_carries_nothing_it_was_not_given() -> None:
    node = node_of()
    assert node["mode"] == "append"
    assert node["type"] == "icebergWrite"
    for absent in ("ref", "mapping", "limits", "compensation", "ignoreUnknownFields", "retry", "secrets", "overwrite"):
        assert absent not in node, absent


def test_every_option_is_carried_and_the_three_record_sources_are_accepted() -> None:
    built = (
        workflow("lake")
        .iceberg_write(
            "w",
            **{
                **BASE,
                "ref": "staging",
                "mapping": [
                    {"field": "/amount", "column": "amount"},
                    {"field": "/customer/name", "column": "customer_name"},
                ],
                "ignore_unknown_fields": True,
                "limits": {"maxRecordBytes": 1024, "maxRecords": 100000000, "timeoutMs": 3600000},
                "compensation": "revert",
                "on_error": "recover",
            },
        )
        .step("recover", "orders.recover")
        .end("done")
        .build()
    )
    node = built["nodes"][0]
    assert node["ref"] == "staging"
    assert node["limits"] == {"maxRecordBytes": 1024, "maxRecords": 100000000, "timeoutMs": 3600000}
    assert node["compensation"] == "revert"
    assert node["onError"] == "recover"
    assert node["ignoreUnknownFields"] is True
    for records in ({"inline": "input.rows"}, {"blobs": "steps.a.output.parts"}, {"databaseResult": "read"}):
        node_of(records=records)


def test_the_node_is_a_snapshot_later_changes_to_the_options_do_not_reach_it() -> None:
    mapping = [{"field": "/a", "column": "a"}]
    table = {"namespace": ["sales"], "name": "orders"}
    flow = workflow("lake").iceberg_write("w", **{**BASE, "mapping": mapping, "table": table})
    mapping[0]["column"] = "changed"
    table["name"] = "changed"
    node = flow.end("done").build()["nodes"][0]
    assert node["mapping"] == [{"field": "/a", "column": "a"}]
    assert node["table"] == {"namespace": ["sales"], "name": "orders"}


REFUSED: list[dict[str, Any]] = [
    {"connection_ref": {"name": "postgres://credential-canary@host/db"}},
    {"connection_ref": {"name": "lake-main", "revision": 0}},
    {"connection_ref": {"name": "lake--main"}},
    {"table": {"namespace": [], "name": "orders"}},
    {"table": {"namespace": list("abcdefghi"), "name": "orders"}},
    {"table": {"namespace": ["sales"], "name": ""}},
    {"table": {"namespace": ["sales"], "name": ".."}},
    {"table": {"namespace": ["sales\x00"], "name": "orders"}},
    {"table": {"namespace": ["sales"], "name": "orders", "catalog": "other"}},
    {"table": {"namespace": "sales", "name": "orders"}},
    {"ref": ""},
    {"ref": "r" * 256},
    {"records": {}},
    {"records": {"inline": "a", "blobs": "b"}},
    {"records": {"files": "a"}},
    {"records": {"inline": ""}},
    {"records": {"inline": "x" * 1025}},
    {"records": {"inline": "é" * 513}},
    {"records": {"inline": 5}},
    {"records": {"databaseResult": ""}},
    {"mapping": [{"field": "amount", "column": "amount"}]},
    {"mapping": [{"field": "/a//b", "column": "a"}]},
    {"mapping": [{"field": "/a~2", "column": "a"}]},
    {"mapping": [{"field": "/" + "/".join(["a"] * 17), "column": "a"}]},
    {"mapping": [{"field": "/a", "column": ""}]},
    {"mapping": [{"field": "/a", "column": "c"}, {"field": "/b", "column": "c"}]},
    {"mapping": [{"field": "/a", "column": "c"}, {"field": "/a", "column": "d"}]},
    {"mapping": [{"field": "/a", "column": "c", "type": "int"}]},
    {"mapping": "x"},
    {"ignore_unknown_fields": "yes"},
    {"limits": {"maxRecords": 0}},
    {"limits": {"maxRecords": 100000001}},
    {"limits": {"maxRecordBytes": 1023}},
    {"limits": {"timeoutMs": 999}},
    {"limits": {"timeoutMs": 1.5}},
    {"limits": {"timeoutMs": True}},
    {"limits": {"maxCommitRetries": 4}},
    {"compensation": "undo"},
    {"on_error": ""},
]


@pytest.mark.parametrize("changes", REFUSED, ids=lambda c: json.dumps(c)[:60])
def test_what_the_server_would_refuse_is_refused_here_with_one_fixed_message_that_shows_no_content(
    changes: dict[str, Any],
) -> None:
    with pytest.raises(SagatError) as raised:
        workflow("lake").iceberg_write("w", **{**BASE, **changes})
    assert raised.value.code == "VALIDATION_FAILED"
    assert str(raised.value.message) == "invalid iceberg write"
    assert "credential-canary" not in str(raised.value)


def test_unknown_options_are_not_accepted_at_all() -> None:
    for unknown in ("mode", "overwrite", "upsert", "record_schema", "retry"):
        with pytest.raises(TypeError):
            workflow("lake").iceberg_write("w", **BASE, **{unknown: "x"})  # type: ignore[arg-type]


EDGES: list[dict[str, Any]] = [
    {"table": {"namespace": list("abcdefgh"), "name": "n" * 255}},
    {"ref": "r" * 255},
    {"records": {"inline": "x" * 1024}},
    {"records": {"inline": "é" * 512}},
    {"mapping": [{"field": "/" + "/".join(["a"] * 16), "column": "c" * 255}]},
    {"mapping": [{"field": "/a~0b/c~1d", "column": "c"}]},
    {"mapping": []},
    {"limits": {"maxRecordBytes": 2147483648, "maxRecords": 1, "timeoutMs": 1000}},
    {"limits": {}},
    {"connection_ref": {"name": "abc"}},
]


@pytest.mark.parametrize("changes", EDGES, ids=lambda c: json.dumps(c)[:60])
def test_the_edges_that_are_accepted_are_accepted(changes: dict[str, Any]) -> None:
    node_of(**changes)


def test_a_node_chains_like_any_other_next_is_linked_by_the_flow() -> None:
    nodes = (
        workflow("lake")
        .step("extract", "orders.extract")
        .iceberg_write("load", **BASE)
        .step("notify", "notify.done")
        .end("done")
        .build()["nodes"]
    )
    assert nodes[0]["next"] == "load"
    assert nodes[1]["next"] == "notify"
