"""The workflow builders (``sdks/SPEC.md`` section 7)."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

import pytest

from sagat_octopus_orchestrator import RetryOptions, SagatError, flow, workflow


def nodes_of(document: dict[str, Any]) -> list[dict[str, Any]]:
    nodes: list[dict[str, Any]] = document["nodes"]
    return nodes


def test_a_chain_of_steps_is_linked_in_order_and_starts_at_the_first() -> None:
    doc = (
        workflow("order-flow")
        .description("charges and ships")
        .step(
            "charge",
            "payments.charge",
            retry=RetryOptions(
                max_attempts=3, strategy="exponential", initial_delay_ms=100, max_delay_ms=1000, jitter=True
            ),
        )
        .sleep("pause", 1500)
        .step("ship", "shipping.ship")
        .build()
    )
    assert doc["entrypoint"] == "charge"
    assert doc["metadata"] == {"name": "order-flow", "description": "charges and ships"}
    assert nodes_of(doc) == [
        {
            "id": "charge",
            "type": "activity",
            "activity": "payments.charge",
            "retry": {
                "maxAttempts": 3,
                "strategy": "exponential",
                "initialDelayMs": 100,
                "maxDelayMs": 1000,
                "jitter": True,
            },
            "next": "pause",
        },
        {"id": "pause", "type": "timer", "durationMs": 1500, "next": "ship"},
        {"id": "ship", "type": "activity", "activity": "shipping.ship"},
    ]


def test_absent_options_are_left_out_and_never_null() -> None:
    doc = workflow("plain-flow").step("a", "act.a", compensate_with=None, retry=RetryOptions()).build()
    assert nodes_of(doc) == [{"id": "a", "type": "activity", "activity": "act.a", "retry": {}}]
    assert "null" not in workflow("plain-flow").step("a", "act.a").to_json()


def test_compensation_reconciliation_and_the_other_step_options_are_written_as_the_schema_names_them() -> None:
    doc = (
        workflow("saga-flow")
        .step(
            "reserve",
            "limit.reserve",
            compensate_with="limit.release",
            reconcile_with="limit.lookup",
            idempotency_template="reserve:${run.id}",
            secrets=["payments-key"],
            compensation_retry={"maxAttempts": 5},
            compensation_timeout_ms=10000,
            high_risk=True,
        )
        .build()
    )
    assert nodes_of(doc)[0] == {
        "id": "reserve",
        "type": "activity",
        "activity": "limit.reserve",
        "compensateWith": "limit.release",
        "reconcileWith": "limit.lookup",
        "idempotencyTemplate": "reserve:${run.id}",
        "secrets": ["payments-key"],
        "compensationRetry": {"maxAttempts": 5},
        "compensationTimeoutMs": 10000,
        "highRisk": True,
    }


def test_a_parallel_lists_the_first_node_of_each_branch_and_continues_with_the_next_node_of_its_flow() -> None:
    doc = (
        workflow("fan-out")
        .parallel(
            "split",
            "all",
            [flow().step("card", "pay.card").step("card-2", "pay.card2"), flow().step("stock", "stock.hold")],
        )
        .step("done", "order.done")
        .build()
    )
    assert nodes_of(doc) == [
        {"id": "split", "type": "parallel", "branches": ["card", "stock"], "join": "all", "next": "done"},
        {"id": "done", "type": "activity", "activity": "order.done"},
        {"id": "card", "type": "activity", "activity": "pay.card", "next": "card-2"},
        {"id": "card-2", "type": "activity", "activity": "pay.card2"},
        {"id": "stock", "type": "activity", "activity": "stock.hold"},
    ]


def test_a_saga_begins_at_its_body_holds_no_end_and_continues_with_the_next_node_of_its_flow() -> None:
    doc = (
        workflow("saga-flow")
        .saga(
            "tx",
            flow().step("reserve", "a.reserve", compensate_with="a.release").step("submit", "a.submit"),
            on_compensation_failure="fail",
            cancel_policy="abort_no_compensation",
        )
        .step("notify", "a.notify")
        .build()
    )
    assert nodes_of(doc)[0] == {
        "id": "tx",
        "type": "saga",
        "begin": "reserve",
        "onCompensationFailure": "fail",
        "cancelPolicy": "abort_no_compensation",
        "next": "notify",
    }
    assert [node["id"] for node in nodes_of(doc)] == ["tx", "notify", "reserve", "submit"]


def test_wait_for_an_event_invoke_a_child_decide_and_end() -> None:
    doc = (
        workflow("mixed-flow")
        .wait_for_event(
            "paid",
            event_type="payment.confirmed",
            correlation_expr="input.orderId",
            timeout_ms=60000,
            on_timeout="cancel",
            lookback_ms=1000,
        )
        .invoke_child("ship", "shipping-flow", version=2, cancel_policy="detach", input="input.order")
        .condition("big", "input.amount > 100", "review", "cancel")
        .step("review", "order.review", on_error="cancel")
        .step("cancel", "order.cancel")
        .end("done", output_expr="steps.ship.output")
        .build()
    )
    nodes = nodes_of(doc)
    assert "next" not in nodes[2], "a condition names both branches and has no next"
    assert nodes[0] == {
        "id": "paid", "type": "waitEvent", "eventType": "payment.confirmed", "correlationExpr": "input.orderId",
        "timeoutMs": 60000, "onTimeout": "cancel", "lookbackMs": 1000, "next": "ship",
    }  # fmt: skip
    assert nodes[1] == {
        "id": "ship", "type": "child", "workflowId": "shipping-flow", "version": 2, "cancelPolicy": "detach",
        "input": "input.order", "next": "big",
    }  # fmt: skip
    assert nodes[5] == {"id": "done", "type": "end", "outputExpr": "steps.ship.output"}


def test_triggers_labels_and_schemas_go_where_the_document_keeps_them() -> None:
    doc = (
        workflow("with-meta")
        .label("team", "payments")
        .trigger("event", event_type="order.created", source="api")
        .trigger("manual")
        .input_schema({"type": "object"})
        .output_schema({})
        .step("a", "act.a")
        .build()
    )
    assert doc["metadata"] == {"name": "with-meta", "labels": {"team": "payments"}}
    assert doc["triggers"] == [{"kind": "event", "eventType": "order.created", "source": "api"}, {"kind": "manual"}]
    assert doc["inputSchema"] == {"type": "object"}
    assert doc["outputSchema"] == {}, "an empty schema is still a schema"


def test_what_the_server_would_refuse_and_the_builder_can_see_is_refused_here() -> None:
    def refused(build: Callable[[], object], text: str) -> None:
        with pytest.raises(SagatError) as caught:
            build()
        assert caught.value.code == "VALIDATION_FAILED"
        assert text in caught.value.message

    refused(lambda: workflow("Bad Name"), "workflow name")
    refused(lambda: workflow("ab"), "workflow name")
    refused(lambda: workflow("empty-flow").build(), "at least one node")
    refused(lambda: workflow("dup-ids").step("a", "x").step("a", "y").build(), "used twice")
    refused(
        lambda: (
            workflow("dup-nested")
            .step("a", "x")
            .parallel("p", "all", [flow().step("a", "y"), flow().step("b", "z")])
            .build()
        ),
        "used twice",
    )
    refused(lambda: workflow("one-branch").parallel("p", "all", [flow().step("a", "x")]), "two branches")
    refused(lambda: workflow("empty-branch").parallel("p", "all", [flow().step("a", "x"), flow()]), "empty branch")
    refused(lambda: workflow("empty-saga").saga("s", flow()), "empty body")
    refused(lambda: workflow("saga-end").saga("s", flow().step("a", "x").end("e")), "end node")
    refused(lambda: workflow("both").step("a", "x", irreversible=True, compensate_with="undo"), "irreversible")
    refused(lambda: workflow("bad-secret").step("a", "x", secrets=["Bad Secret"]), "secret")
    refused(lambda: workflow("bad-timer").sleep("t", 0), "duration")
    refused(lambda: workflow("bad-timer").sleep("t", 1.5), "duration")  # type: ignore[arg-type]
    refused(lambda: workflow("dangling").condition("c", "true", "nowhere", "a").step("a", "x").build(), "not a node")
    refused(
        lambda: (
            workflow("dangling-timeout")
            .wait_for_event("w", event_type="e", correlation_expr="x", timeout_ms=1, on_timeout="nowhere")
            .build()
        ),
        "not a node",
    )


def test_the_canonical_text_sorts_keys_at_every_depth_and_leaves_out_whitespace() -> None:
    text = workflow("canon-flow").step("a", "act.a", retry=RetryOptions(max_attempts=2)).to_json()
    assert text.startswith('{"apiVersion":')
    assert " " not in text
    assert text == (
        '{"apiVersion":"sagat-octopus-orchestrator.io/v1","entrypoint":"a","kind":"Workflow","metadata":{"name":"canon-flow"},'
        '"nodes":[{"activity":"act.a","id":"a","retry":{"maxAttempts":2},"type":"activity"}]}'
    )
