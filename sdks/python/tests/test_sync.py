"""The synchronous facade of the client."""

from __future__ import annotations

import threading
from typing import Any

import pytest

from sagat_octopus_orchestrator import ClientConfig, SagatError
from sagat_octopus_orchestrator.sync import SyncSagatClient

from .conftest import FAST
from .fakes import FakeHttpServer, Seen, reply
from .test_client import RUN, WORKFLOW, run_body

NAMESPACE_ID = "0190aaaa-0000-7000-8000-000000000010"
CHAIN_ID = "0190aaaa-0000-7000-8000-000000000011"
HEAD_ID = "0190aaaa-0000-7000-8000-000000000012"


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
        client.get("0190aaaa-0000-7000-8000-000000000001")


def test_the_blocking_client_can_be_shared_between_threads_and_stops_its_thread(http: FakeHttpServer) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    client = SyncSagatClient(ClientConfig(api_url=http.base, namespace="p", token="t", retry=FAST))
    found: list[str] = []
    workers = [
        threading.Thread(target=lambda: found.append(client.get("0190aaaa-0000-7000-8000-000000000001").run_id))
        for _ in range(8)
    ]
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


def test_blocking_chain_wait_follows_final_outcome_and_preserves_segment_wait(http: FakeHttpServer) -> None:
    inspections = 0

    def serve(seen: Seen) -> Any:
        nonlocal inspections
        if seen.url == f"/v1/runs/{CHAIN_ID}":
            return reply(200, run_body(runId=CHAIN_ID, status="SUCCEEDED"))
        inspections += 1
        return reply(
            200,
            {
                "chainId": CHAIN_ID,
                "currentRunId": HEAD_ID,
                "revision": inspections,
                "cancelRequested": False,
                "finalOutcome": "SUCCEEDED" if inspections > 1 else None,
                "segments": [
                    {"index": 0, "runId": CHAIN_ID, "predecessorRunId": None, "successorRunId": HEAD_ID},
                    {"index": 1, "runId": HEAD_ID, "predecessorRunId": CHAIN_ID, "successorRunId": None},
                ],
            },
        )

    http.serve(serve)
    with SyncSagatClient(ClientConfig(api_url=http.base, namespace="p", token="t", retry=FAST)) as client:
        assert client.wait_for_completion(CHAIN_ID).status == "SUCCEEDED"
        chain = client.wait_for_chain_completion(NAMESPACE_ID, CHAIN_ID, timeout=2, poll_interval=0.001)
        assert chain.final_outcome == "SUCCEEDED" and chain.current_run_id == HEAD_ID
        assert inspections == 2
        assert [seen.url for seen in http.seen[1:]] == [f"/v1/namespaces/{NAMESPACE_ID}/chains/{CHAIN_ID}"] * 2
        http.serve(lambda _seen: reply(200, {**chain.raw, "finalOutcome": None}))
        with pytest.raises(SagatError) as caught:
            client.wait_for_chain_completion(NAMESPACE_ID, CHAIN_ID, timeout=0)
        assert caught.value.reason == "WAIT_TIMEOUT"


def test_blocking_chain_update_reuses_the_key_and_expected_head_on_retry(http: FakeHttpServer) -> None:
    def serve(_seen: Seen) -> Any:
        if len(http.seen) == 1:
            return reply(503, "busy")
        return reply(
            200,
            {
                "runId": HEAD_ID,
                "operation": "account.credit",
                "updateKey": "credit-1",
                "status": "APPLIED",
                "result": {"balance": 3},
                "beforeStateRevision": 0,
                "afterStateRevision": 1,
                "runVersion": 7,
                "replayed": True,
            },
        )

    http.serve(serve)
    with SyncSagatClient(ClientConfig(api_url=http.base, namespace="p", token="t", retry=FAST)) as client:
        result = client.update_chain(
            NAMESPACE_ID,
            CHAIN_ID,
            "account.credit",
            {"amount": 3},
            update_key="credit-1",
            expected_run_id=HEAD_ID,
            expected_state_revision=0,
        )
        assert result.replayed and result.run_id == HEAD_ID and result.result == {"balance": 3}
        assert len(http.seen) == 2 and http.seen[0].body == http.seen[1].body == {
            "updateKey": "credit-1",
            "args": {"amount": 3},
            "expectedRunId": HEAD_ID,
            "expectedStateRevision": 0,
        }
        assert http.seen[0].url == f"/v1/namespaces/{NAMESPACE_ID}/chains/{CHAIN_ID}/updates/account.credit"


def test_blocking_rerun_as_new_returns_the_new_run_and_preserves_the_key(http: FakeHttpServer) -> None:
    http.serve(
        lambda _seen: reply(
            202,
            {
                "operationId": "op-2",
                "accepted": True,
                "runStatus": "SUCCEEDED",
                "runVersion": 9,
                "newRunId": HEAD_ID,
            },
        )
    )
    with SyncSagatClient(ClientConfig(api_url=http.base, namespace="p", token="t", retry=FAST)) as client:
        result = client.rerun_as_new(RUN, "retry after fix", idempotency_key="rerun-key-001")
        assert result.accepted and result.new_run_id == HEAD_ID
        assert http.seen[0].url == f"/v1/runs/{RUN}/operations"
        assert http.seen[0].body == {"action": "RERUN_AS_NEW", "reason": "retry after fix"}
        assert http.seen[0].headers["idempotency-key"] == "rerun-key-001"
