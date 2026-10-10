"""The synchronous facade of the client."""

from __future__ import annotations

import threading
from typing import Any

import pytest

from sagat_octopus_orchestrator import ClientConfig, SagatError
from sagat_octopus_orchestrator.sync import SyncSagatClient

from .conftest import FAST
from .fakes import FakeHttpServer, Seen, reply
from .test_client import WORKFLOW, run_body


def test_a_blocking_client_makes_the_same_calls_with_the_same_rules(http: FakeHttpServer) -> None:
    attempts = 0

    def serve(seen: Seen) -> Any:
        nonlocal attempts
        if seen.method == "POST":
            attempts += 1
            return reply(503, "busy") if attempts == 1 else reply(202, run_body())
        if "cursor=n2" in seen.url:
            return reply(200, {"items": [run_body(runId="r-2")], "nextCursor": None})
        return reply(200, {"items": [run_body(runId="r-1")], "nextCursor": "n2"})

    http.serve(serve)
    with SyncSagatClient(ClientConfig(api_url=http.base, namespace="payments", token="tok", retry=FAST)) as client:
        run = client.start(WORKFLOW, {"orderId": "o-1"})
        assert run.status == "RUNNING"
        posts = [seen for seen in http.seen if seen.method == "POST"]
        assert len(posts) == 2 and posts[0].headers["idempotency-key"] == posts[1].headers["idempotency-key"]
        assert [item.run_id for item in client.list_all(limit=1)] == ["r-1", "r-2"]
        with pytest.raises(SagatError) as caught:
            client.start(WORKFLOW, {}, idempotency_key="short")
        assert caught.value.code == "VALIDATION_FAILED"
    with pytest.raises(SagatError):
        client.get("r")


def test_the_blocking_client_can_be_shared_between_threads_and_stops_its_thread(http: FakeHttpServer) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    client = SyncSagatClient(ClientConfig(api_url=http.base, namespace="p", token="t", retry=FAST))
    found: list[str] = []
    workers = [threading.Thread(target=lambda: found.append(client.get("r").run_id)) for _ in range(8)]
    for worker in workers:
        worker.start()
    for worker in workers:
        worker.join()
    assert len(found) == 8
    client.close()
    client.close()
    assert not any(thread.name == "sagat-client" and thread.is_alive() for thread in threading.enumerate())


def test_a_configuration_that_cannot_work_does_not_leave_a_thread_behind() -> None:
    with pytest.raises(SagatError):
        SyncSagatClient(ClientConfig(api_url="", namespace="p", token="t"))
