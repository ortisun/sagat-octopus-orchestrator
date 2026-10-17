"""Serve order-processing activities, start one canonical workflow, and wait for its result.

The activities simulate effects locally; they do not contact an external provider.
"""

from __future__ import annotations

import asyncio
import contextlib
import os
import sys
from typing import Literal, TypedDict

from sagat_octopus_orchestrator import (
    ActivityContext,
    ActivityFailure,
    ClientConfig,
    SagatClient,
    SagatError,
    WorkerConfig,
    WorkerHost,
)

Scenario = Literal["routing", "fulfillment", "reminder"]
WORKFLOWS: dict[Scenario, str] = {
    "routing": "order-routing",
    "fulfillment": "order-fulfillment",
    "reminder": "order-reminder",
}


class OrderInput(TypedDict):
    orderId: str
    amount: int
    failNotify: bool


def setting(name: str) -> str:
    value = os.environ.get(name, "")
    if not value:
        print(f"{name} is not set (see the README)", file=sys.stderr)
        raise SystemExit(2)
    return value


def scenario_setting() -> Scenario:
    value = os.environ.get("SAGAT_SCENARIO") or "routing"
    if value not in WORKFLOWS:
        print("SAGAT_SCENARIO must be routing, fulfillment or reminder", file=sys.stderr)
        raise SystemExit(2)
    return value


async def validate(order: OrderInput, _ctx: ActivityContext) -> dict[str, bool]:
    print("activity order.validate", flush=True)
    return {"accepted": order["amount"] > 0}


async def reserve(order: OrderInput, _ctx: ActivityContext) -> dict[str, object]:
    print("activity order.reserve", flush=True)
    return {"reserved": True, "orderId": order["orderId"]}


async def quote(_order: OrderInput, _ctx: ActivityContext) -> dict[str, int]:
    print("activity order.quote", flush=True)
    return {"shippingCents": 500}


async def notify(order: OrderInput, _ctx: ActivityContext) -> dict[str, object]:
    print("activity order.notify", flush=True)
    # Routing and fulfillment expose earlier outputs through optional ctx.steps keys; reminder-delay
    # is a timer and need not produce an output. Real code should check a key before consuming it.
    # A real notification would send ctx.idempotency_key to the provider. This example only
    # simulates the effect and deliberately keeps payloads and credentials out of its output.
    if order["failNotify"]:
        raise ActivityFailure.permanent(
            "NOTIFICATION_REJECTED", "the simulated notification was rejected"
        )
    return {"notified": True, "orderId": order["orderId"]}


async def run(api_url: str, grpc_url: str, namespace: str, token: str, scenario: Scenario) -> int:
    order: OrderInput = {
        "orderId": "demo-order-001",
        "amount": -1 if os.environ.get("SAGAT_REJECT_ORDER") == "1" else 100,
        "failNotify": os.environ.get("SAGAT_FAIL_NOTIFY") == "1",
    }
    host = WorkerHost(
        WorkerConfig(
            grpc_url=grpc_url,
            namespace=namespace,
            token=token,
            worker_name="order-processing-python",
        )
    )
    host.register_activity("order.validate", validate)
    host.register_activity("order.reserve", reserve)
    host.register_activity("order.quote", quote)
    host.register_activity("order.notify", notify)

    try:
        await host.start()
        async with SagatClient(ClientConfig(api_url=api_url, namespace=namespace, token=token)) as api:
            started = await api.start(WORKFLOWS[scenario], order)
            done = await api.wait_for_completion(started.run_id, timeout=60)
        print(f"scenario {scenario} run {done.run_id} {done.status}", flush=True)
        return 0 if done.status == "SUCCEEDED" else 1
    finally:
        with contextlib.suppress(SagatError):
            await host.stop(timeout=10)


def main() -> int:
    api_url, grpc_url, namespace, token = (
        setting("SAGAT_SERVER"),
        setting("SAGAT_GRPC"),
        setting("SAGAT_NAMESPACE"),
        setting("SAGAT_TOKEN"),
    )
    scenario = scenario_setting()
    try:
        return asyncio.run(run(api_url, grpc_url, namespace, token, scenario))
    except SagatError as failure:
        print(f"{failure.code} (trace {failure.trace_id}): {failure.remediation}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
