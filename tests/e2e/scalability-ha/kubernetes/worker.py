"""A deliberately slow real SDK worker used to make KEDA observe and drain a backlog."""

from __future__ import annotations

import asyncio
import os
import signal

from sagat_octopus_orchestrator import WorkerConfig, WorkerHost


def required(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise RuntimeError(f"{name} is required")
    return value


async def work(run_input: dict[str, object], _context: object) -> dict[str, object]:
    await asyncio.sleep(float(os.environ.get("G247_WORK_SECONDS", "2.5")))
    return {"accepted": True, "sequence": run_input.get("sequence")}


async def main() -> None:
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for watched in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(watched, stop.set)

    host = WorkerHost(
        WorkerConfig(
            grpc_url=required("SAGAT_GRPC"),
            namespace=required("SAGAT_NAMESPACE"),
            token=required("SAGAT_TOKEN"),
            worker_name=f"g247-{os.environ.get('HOSTNAME', 'worker')}",
            max_inflight=1,
            pools=(required("SAGAT_WORKER_POOL"),),
        )
    )
    host.register_activity("g247.work", work)
    await host.start()
    print("G247 worker registered", flush=True)
    try:
        await stop.wait()
    finally:
        await host.stop(timeout=20)


if __name__ == "__main__":
    asyncio.run(main())
