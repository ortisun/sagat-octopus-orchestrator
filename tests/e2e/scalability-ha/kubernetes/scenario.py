"""Creates a real workflow backlog and verifies that real workers drain it."""

from __future__ import annotations

import asyncio
import os
import sys
import time
import uuid

import httpx

from sagat_octopus_orchestrator import ClientConfig, SagatClient


API = os.environ.get("SAGAT_SERVER", "http://sagat-control-plane:8080/v1").rstrip("/")
TOKEN = os.environ["SAGAT_TOKEN"]
NAMESPACE = os.environ.get("SAGAT_NAMESPACE", "payments")
RUNS = int(os.environ.get("G247_RUNS", "60"))
WORKFLOW = "g247-keda"


def headers(*, key: str | None = None) -> dict[str, str]:
    result = {
        "authorization": f"Bearer {TOKEN}",
        "content-type": "application/json",
        "x-octopus-namespace": NAMESPACE,
    }
    if key:
        result["idempotency-key"] = key
    return result


async def checked(response: httpx.Response) -> dict[str, object]:
    if response.is_error:
        raise RuntimeError(f"{response.request.method} {response.request.url.path}: HTTP {response.status_code} {response.text[:400]}")
    body = response.json()
    if not isinstance(body, dict):
        raise RuntimeError(f"{response.request.url.path}: expected an object")
    return body


async def setup_and_enqueue() -> None:
    async with httpx.AsyncClient(timeout=30) as http:
        namespaces = await checked(await http.get(f"{API}/namespaces", headers=headers()))
        namespace = next(item for item in namespaces["items"] if item["slug"] == NAMESPACE)
        await checked(
            await http.put(
                f"{API}/namespaces/{namespace['id']}/worker-bindings/dev-admin",
                headers=headers(),
                json={"activities": ["g247.work"]},
            )
        )

        workflows = await checked(await http.get(f"{API}/workflows?limit=200", headers=headers()))
        workflow = next((item for item in workflows["items"] if item["name"] == WORKFLOW), None)
        if workflow is None:
            ir = {
                "apiVersion": "sagat-octopus-orchestrator.io/v1",
                "kind": "Workflow",
                "metadata": {"name": WORKFLOW},
                "entrypoint": "work",
                "triggers": [{"kind": "manual"}],
                "nodes": [{"id": "work", "type": "activity", "activity": "g247.work"}],
            }
            workflow = await checked(
                await http.post(
                    f"{API}/workflows",
                    headers=headers(key=f"g247-create-{uuid.uuid4()}"),
                    json={"name": WORKFLOW, "ir": ir},
                )
            )
        if workflow.get("activeVersion") is None:
            await checked(
                await http.post(
                    f"{API}/workflows/{workflow['id']}/publish",
                    headers=headers(key=f"g247-publish-{uuid.uuid4()}"),
                    json={"draftRevision": workflow["draftRevision"], "releaseNotes": "G247 Kubernetes validation"},
                )
            )

    async with SagatClient(ClientConfig(api_url=API, namespace=NAMESPACE, token=TOKEN)) as api:
        async def start(sequence: int) -> None:
            await api.start(WORKFLOW, {"sequence": sequence}, idempotency_key=f"g247-run-{uuid.uuid4()}")

        for offset in range(0, RUNS, 10):
            await asyncio.gather(*(start(sequence) for sequence in range(offset, min(offset + 10, RUNS))))
    print(f"G247_ENQUEUED={RUNS}", flush=True)


async def verify_drained() -> None:
    deadline = time.monotonic() + 240
    async with SagatClient(ClientConfig(api_url=API, namespace=NAMESPACE, token=TOKEN)) as api:
        workflow_id = await api.resolve_workflow(WORKFLOW)
        while time.monotonic() < deadline:
            runs = [run async for run in api.list_all(workflow_id=workflow_id, limit=RUNS)]
            succeeded = sum(run.status == "SUCCEEDED" for run in runs)
            failed = [run.status for run in runs if run.status in {"FAILED", "CANCELLED", "PARTIAL"}]
            if failed:
                raise RuntimeError(f"terminal failures while draining: {failed}")
            if len(runs) >= RUNS and succeeded >= RUNS:
                print(f"G247_DRAINED={succeeded}", flush=True)
                return
            await asyncio.sleep(2)
    raise RuntimeError("the real worker pool did not drain the backlog within 240 seconds")


async def run() -> None:
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "enqueue":
        await setup_and_enqueue()
    elif command == "verify":
        await verify_drained()
    else:
        raise RuntimeError("use scenario.py enqueue|verify")


if __name__ == "__main__":
    asyncio.run(run())
