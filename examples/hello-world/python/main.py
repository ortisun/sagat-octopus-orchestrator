"""The hello-world workflow served and started with the Python SDK: a worker that answers the two
activities of the workflow (templates/hello-world), a run of it, and the end of the run.

    SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 \
    SAGAT_NAMESPACE=payments SAGAT_TOKEN=... .venv/bin/python main.py [NAME]
"""

from __future__ import annotations

import asyncio
import contextlib
import os
import sys
from typing import TypedDict

from sagat_octopus_orchestrator import (
    ActivityContext,
    ClientConfig,
    SagatClient,
    SagatError,
    WorkerConfig,
    WorkerHost,
)


# What a step receives: the input of the run (the first argument) and the outputs of the steps that ran
# before it (`ctx.steps`, by step id).
class Input(TypedDict):
    name: str


class Greeting(TypedDict):
    greeting: str


class Delivery(TypedDict):
    delivered: bool
    greeting: str


async def compose(run_input: Input, ctx: ActivityContext) -> Greeting:
    return {"greeting": f"Hello, {run_input['name']}!"}


async def deliver(run_input: Input, ctx: ActivityContext) -> Delivery:
    # A real delivery is an external effect: send ctx.idempotency_key to the provider, which is the
    # same for every attempt of this step.
    greeting: Greeting = ctx.steps["compose"]
    print(f"delivering {greeting['greeting']}", flush=True)
    return {"delivered": True, "greeting": greeting["greeting"]}


def setting(name: str) -> str:
    value = os.environ.get(name, "")
    if not value:
        print(f"{name} is not set (see the README)", file=sys.stderr)
        sys.exit(2)
    return value


async def run(api_url: str, grpc_url: str, namespace: str, token: str, name: str) -> int:
    # The worker: it takes the tasks of the two activities and answers them.
    host = WorkerHost(
        WorkerConfig(grpc_url=grpc_url, namespace=namespace, token=token, worker_name="hello-world-python")
    )
    host.register_activity("greeting.compose", compose)
    host.register_activity("greeting.deliver", deliver)
    await host.start()
    try:
        print(f"serving greeting.compose and greeting.deliver at protocol {host.protocol_version}", flush=True)

        # The client: start the run and wait for it.
        async with SagatClient(ClientConfig(api_url=api_url, namespace=namespace, token=token)) as api:
            started = await api.start("hello-world", {"name": name})
            print(f"run {started.run_id} started", flush=True)
            done = await api.wait_for_completion(started.run_id, timeout=60)
            print(f"run {done.run_id} {done.status}", flush=True)
        if done.status != "SUCCEEDED":
            print(f"the run ended {done.status}", file=sys.stderr)
            return 1
        return 0
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
    name = sys.argv[1] if len(sys.argv) > 1 else "Ada"
    try:
        return asyncio.run(run(api_url, grpc_url, namespace, token, name))
    except SagatError as failure:
        # The code, the trace and what to do about it; never the payload or the token.
        print(f"{failure.code} (trace {failure.trace_id}): {failure.remediation}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
