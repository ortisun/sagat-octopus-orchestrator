"""The fault plan of the conformance runner: it is part of what the suite measures, so it is tested too."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

import grpc
import pytest

from conformance.faults import Fault, FaultPlan
from sagat_octopus_orchestrator import ClientConfig, RetryPolicy, SagatClient, SagatError, WorkerConfig, WorkerHost

from .conftest import FAST
from .fakes import FakeHttpServer, FakeWorkerService, lease, reply, until
from .test_client import run_body


def fault(**raw: Any) -> Fault:
    return Fault.of({"channel": "http", "action": "status", "status": 503, **raw})


def test_a_call_is_counted_against_every_fault_that_matches_it_and_the_first_due_one_answers() -> None:
    plan = FaultPlan(
        [
            fault(match="POST /runs", times=1),
            fault(match="POST /runs", after=1, times=1),
            fault(match="GET /runs", after=1),
        ]
    )
    answers = [plan.next("http", "POST /runs/abc/operations") for _ in range(4)]
    assert [a is not None for a in answers] == [True, True, False, False]
    assert plan.applied() == [1, 1, 0]
    assert plan.next("http", "GET /runs") is None, "the first GET is skipped by `after`"
    assert plan.next("http", "GET /runs/1") is not None
    assert plan.applied() == [1, 1, 1]
    assert plan.next("grpc", "POST /runs") is None, "another channel"


def test_a_fault_that_is_not_due_still_counts_the_calls_it_matched() -> None:
    plan = FaultPlan([fault(match="GET /runs", times=1), fault(match="GET /runs", after=2, times=1)])
    assert plan.next("http", "GET /runs") is not None  # answered by the first; counted by both
    assert plan.next("http", "GET /runs") is None
    assert plan.next("http", "GET /runs") is not None  # the second fault's third matching call
    assert plan.applied() == [1, 1]


async def test_http_faults_fail_before_answer_with_a_status_or_lose_the_answer_after_the_server_applied_the_call(
    http: FakeHttpServer,
) -> None:
    plan = FaultPlan(
        [
            fault(match="POST /runs", action="failBefore", times=1),
            fault(match="POST /runs", action="dropResponse", after=1, times=1),
            fault(
                match="GET /runs",
                status=429,
                retryAfterMs=1000,
                body={"code": "RATE_LIMITED", "message": "x", "retryable": True, "traceId": ""},
            ),
        ]
    )
    http.serve(lambda _seen: reply(202, run_body()))
    client = SagatClient(
        ClientConfig(api_url=http.base, namespace="p", token="t", retry=FAST, transport=plan.transport("/v1"))
    )
    await client.start("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", {}, idempotency_key="key-12345678")
    assert len(http.seen) == 2, (
        "the first attempt never left; the second was applied and its answer lost; the third was served"
    )
    one_shot = SagatClient(
        ClientConfig(
            api_url=http.base,
            namespace="p",
            token="t",
            retry=RetryPolicy(max_attempts=1),
            transport=plan.transport("/v1"),
        )
    )
    with pytest.raises(SagatError) as caught:
        await one_shot.get("0190aaaa-0000-7000-8000-000000000001")
    assert (caught.value.code, caught.value.retry_after_ms, caught.value.status) == ("RATE_LIMITED", 1000, 429)
    assert plan.applied() == [1, 1, 1]
    await client.aclose()
    await one_shot.aclose()


async def test_grpc_faults_fail_before_answer_with_a_status_or_lose_the_answer_after_the_server_applied_the_call(
    fake: FakeWorkerService, make_host: Callable[..., WorkerHost]
) -> None:
    plan = FaultPlan(
        [
            Fault.of({"channel": "grpc", "match": "RegisterWorker", "action": "failBefore", "times": 1}),
            Fault.of(
                {"channel": "grpc", "match": "PollTasks", "action": "status", "code": "UNAUTHENTICATED", "times": 1}
            ),
            Fault.of({"channel": "grpc", "match": "CompleteStep", "action": "dropResponse", "times": 1}),
        ]
    )
    host = make_host(interceptors=plan.interceptors())
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 2, "the completion repeated after its answer was lost")
    assert plan.applied() == [1, 1, 1]
    assert len(fake.of("RegisterWorker")) == 2, "the first never left; the second came after the session was refused"
    assert {call.request.idempotency_key for call in fake.of("CompleteStep")} == {"done-run-1/charge/1"}


def test_a_plan_with_no_grpc_fault_installs_no_interceptor() -> None:
    assert FaultPlan([fault(match="GET /runs")]).interceptors() == []
    assert FaultPlan([Fault.of({"channel": "grpc", "match": "X", "action": "failBefore"})]).interceptors() != []


async def test_the_status_of_a_grpc_fault_is_the_one_the_scenario_names(fake: FakeWorkerService) -> None:
    plan = FaultPlan(
        [
            Fault.of(
                {
                    "channel": "grpc",
                    "match": "RegisterWorker",
                    "action": "status",
                    "code": "PERMISSION_DENIED",
                    "reason": "NOT_BOUND",
                }
            )
        ]
    )
    host = WorkerHost(
        WorkerConfig(
            grpc_url=fake.url,
            namespace="p",
            token="t",
            worker_name="w",
            retry=RetryPolicy(max_attempts=1),
            interceptors=plan.interceptors(),
        )
    )
    host.register_activity("a", lambda _i, _c: 1)
    with pytest.raises(SagatError) as caught:
        await host.start()
    assert (caught.value.code, caught.value.status, caught.value.reason) == (
        "NOT_BOUND",
        grpc.StatusCode.PERMISSION_DENIED.name,
        "NOT_BOUND",
    )
    assert not fake.calls, "an injected status never reaches the server"
