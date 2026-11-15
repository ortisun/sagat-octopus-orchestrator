"""The operator factory of a collection and the ``operators`` of ``for_each`` and ``map`` (design 042)."""

from __future__ import annotations

from typing import Any

import pytest

from sagat_octopus_orchestrator import Operator, SagatError, body, ops, workflow


def _line() -> Any:
    return body().step("price", "line.price").return_value("return", "steps.price.output")


def test_the_factory_writes_one_stage_per_operator_keyed_by_its_name() -> None:
    assert ops.filter("element.total > 0").to_json() == {"filter": {"where": "element.total > 0"}}
    assert ops.map("element.lines").to_json() == {"map": {"select": "element.lines"}}
    assert ops.flat_map("element").to_json() == {"flatMap": {"select": "element"}}
    assert ops.skip(0).to_json() == {"skip": {"count": 0}}
    assert ops.take(1000).to_json() == {"take": {"count": 1000}}
    assert ops.distinct("element.sku").to_json() == {"distinct": {"by": "element.sku"}}
    assert ops.distinct().to_json() == {"distinct": {}}
    assert ops.window(25).to_json() == {"window": {"count": 25}}
    assert isinstance(ops.window(25), Operator)
    assert ops.flat_map("element").name == "flatMap"


def test_the_factory_only_assembles_and_leaves_ranges_to_the_server() -> None:
    assert ops.window(0).to_json() == {"window": {"count": 0}}
    assert ops.take(1001).to_json() == {"take": {"count": 1001}}


def test_a_stage_is_a_fresh_copy() -> None:
    operator = ops.filter("element.ok")
    stage = operator.to_json()
    stage["filter"]["where"] = "true"
    assert operator.to_json() == {"filter": {"where": "element.ok"}}


def test_for_each_and_map_carry_the_operators_in_the_order_given() -> None:
    chain = [ops.filter("element.ok"), ops.distinct(), ops.window(2)]
    document = (
        workflow("chained-flow")
        .body("line", _line())
        .for_each("lines", "input.lines", "line", "bodyResult", max_items=9, max_concurrency=2, operators=chain)
        .map("prices", "input.lines", "line", "bodyResult", max_items=9, max_concurrency=2, operators=(ops.take(3),))
        .end("done")
        .build()
    )
    assert document["nodes"][0]["operators"] == [
        {"filter": {"where": "element.ok"}},
        {"distinct": {}},
        {"window": {"count": 2}},
    ]
    assert document["nodes"][1]["operators"] == [{"take": {"count": 3}}]
    assert document["nodes"][1]["type"] == "forEach"
    chain.append(ops.take(1))
    document["nodes"][0]["operators"][0]["filter"]["where"] = "true"
    assert chain[0].to_json() == {"filter": {"where": "element.ok"}}


def test_every_present_collection_call_still_builds_without_operators() -> None:
    document = (
        workflow("plain-flow")
        .body("line", _line())
        .for_each("lines", "input.lines", "line", "bodyResult", max_items=17, max_concurrency=3)
        .map("prices", "input.lines", "line", "bodyResult", max_items=17, max_concurrency=3)
        .for_each("empty", "input.lines", "line", "bodyResult", max_items=17, max_concurrency=3, operators=[])
        .end("done")
        .build()
    )
    assert all("operators" not in node for node in document["nodes"][:3])


def test_a_stage_not_built_by_the_factory_is_a_validation_failure() -> None:
    builder = workflow("bad-flow").body("line", _line())
    with pytest.raises(SagatError) as raised:
        builder.for_each(
            "lines",
            "input.lines",
            "line",
            "bodyResult",
            max_items=2,
            max_concurrency=1,
            operators=[{"take": {"count": 1}}],  # type: ignore[list-item]
        )
    assert raised.value.code == "VALIDATION_FAILED"
    assert "forEach lines operator 0" in str(raised.value)


def test_operators_is_keyword_only() -> None:
    with pytest.raises(TypeError):
        workflow("positional-flow").for_each(  # type: ignore[call-arg]
            "lines", "input.lines", "line", "bodyResult", 2, 1, [ops.take(1)]
        )
