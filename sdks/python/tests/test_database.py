"""The shared typed operation stays identical across SDKs; invalid values stay out of errors."""

import json
from pathlib import Path
from typing import Any

import pytest

from sagat_octopus_orchestrator import SagatError, workflow


def test_shared_typed_database_fixture() -> None:
    fixture = json.loads((Path(__file__).parents[3] / "tests/contract/database-connections/typed-parameters.workflow.json").read_text())
    node = fixture["nodes"][0]
    built = workflow(fixture["metadata"]["name"]).database(
        node["id"], connection_ref=node["connectionRef"], operation=node["operation"], statement=node["statement"],
        parameters=node["parameters"],
    ).build()
    assert built == fixture


def test_database_snapshot_and_safe_validation() -> None:
    base: dict[str, Any] = {
        "connection_ref": {"name": "orders-db"}, "operation": "query", "statement": "SELECT $1",
        "parameters": [{"literal": {"type": "integer", "value": "9223372036854775807"}}],
    }
    builder = workflow("database-chain").database("q", **base).end("done")
    base["parameters"][0]["literal"]["value"] = "changed"
    node = builder.build()["nodes"][0]
    assert node["parameters"][0]["literal"]["value"] == "9223372036854775807"
    assert node["next"] == "done"
    assert "retry" not in node and "secrets" not in node
    base["parameters"] = []
    changes: list[dict[str, Any]] = [
        {"connection_ref": {"name": "postgres://credential-canary@host/db"}},
        {"operation": "credential-canary"}, {"statement": ""},
        {"parameters": [{"literal": {"type": "integer", "value": 123}}]},
        {"parameters": [{"literal": {"type": "decimal", "value": "credential-canary"}}]},
        {"limits": {"timeoutMs": True}}, {"connection_ref": {"name": "orders-db", "revision": 0}},
    ]
    for change in changes:
        with pytest.raises(SagatError) as error:
            workflow("database-invalid").database("q", **(base | change))
        assert error.value.code == "VALIDATION_FAILED"
        assert "credential-canary" not in str(error.value)
