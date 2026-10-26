"""Round 5: the canonical text refuses by value, the Retry-After field is capped, and the survivors of the mutation pass."""

from __future__ import annotations

import asyncio
import json
import logging
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

import grpc
import pytest

from conformance.runner import build as build_scenario
from sagat_octopus_orchestrator import (
    ActivityContext,
    ClientConfig,
    RetryPolicy,
    SagatClient,
    SagatError,
    canonical_json,
    workflow,
)
from sagat_octopus_orchestrator.worker import host as host_module
from sagat_octopus_orchestrator.worker._grpc import error_of_status

from .fakes import FakeHttpServer, FakeWorkerService, lease, reply, until
from .test_client import RUN, WORKFLOW, MakeClient, refusal, run_body
from .test_round3 import MakeHost

CONTRACT = Path(__file__).resolve().parent.parent.parent.parent / "tests" / "contract" / "sdk-conformance"


# --------------------------------------------------------------------------- 1. refused by value, in the runner


@pytest.mark.parametrize(
    "number",
    [
        2**53,
        2**53 + 1,
        9007199254740993,
        -(2**53),
        1e20,
        -1e20,
        9007199254740992.0,
        2.5e19,
        9.007199254740993e15 * 2,
        999999999999999868928.0,
    ],
)
def test_a_whole_number_beyond_2_to_the_53_minus_1_that_ecmascript_prints_in_digits_is_refused_however_it_is_written(
    number: float,
) -> None:
    with pytest.raises(SagatError) as caught:
        canonical_json({"maximum": number})
    assert caught.value.code == "VALIDATION_FAILED"
    with pytest.raises(SagatError):
        workflow("unsafe-number").step("a", "x.y").input_schema({"maximum": number}).to_json()


@pytest.mark.parametrize(
    ("number", "text"),
    [
        (2**53 - 1, "9007199254740991"),
        (-(2**53 - 1), "-9007199254740991"),
        (1e21, "1e+21"),
        (1.5e21, "1.5e+21"),
        (10**30, "1e+30"),
        (-(10**22), "-1e+22"),
        (0.5, "0.5"),
        (123456789.125, "123456789.125"),
        (4503599627370495.5, "4503599627370495.5"),
        (1e-7, "1e-7"),
    ],
)
def test_the_numbers_ecmascript_writes_with_an_exponent_or_a_fraction_are_numbers(number: float, text: str) -> None:
    assert canonical_json(number) == text


def test_the_float_just_below_the_limit_is_still_a_safe_integer() -> None:
    assert canonical_json(9007199254740991.0) == "9007199254740991"
    assert canonical_json(2.0**52) == "4503599627370496"


def test_the_invalid_workflows_of_the_builders_scenario_are_refused_from_build_or_from_the_canonical_text() -> None:
    scenario = json.loads((CONTRACT / "scenarios" / "builders.json").read_text(encoding="utf-8"))
    expected = json.loads((CONTRACT / "expected" / "builders.workflows.json").read_text(encoding="utf-8"))
    built = build_scenario(scenario)
    assert built["errors"] == expected["errors"]
    assert built["errors"]["unsafe-integer"] == {"raised": True, "code": "VALIDATION_FAILED", "mentions": True}
    assert built["errors"]["unsafe-integer-written-as-a-float"]["raised"] is True


# --------------------------------------------------------------------------- 2. the Retry-After field


async def test_retry_after_is_capped_at_60_seconds_in_the_error_too(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    for header, expected_ms in (("120", 60_000), ("60", 60_000), ("59", 59_000), ("86400", 60_000), ("0", 0)):
        http.serve(lambda _seen, header=header: reply(429, refusal("RATE_LIMITED", "slow", True), retry_after=header))  # type: ignore[misc]
        with pytest.raises(SagatError) as caught:
            await make_client(retry=RetryPolicy(max_attempts=1)).get(RUN)
        assert caught.value.retry_after_ms == expected_ms, header


def test_a_grpc_retry_after_ms_is_capped_at_60_seconds_in_the_error_too() -> None:
    def failure(value: str) -> grpc.aio.AioRpcError:
        return grpc.aio.AioRpcError(
            grpc.StatusCode.RESOURCE_EXHAUSTED,
            grpc.aio.Metadata(),
            grpc.aio.Metadata(("retry-after-ms", value)),
            details="x",
        )

    assert error_of_status("PollTasks", failure("120000")).retry_after_ms == 60_000
    assert error_of_status("PollTasks", failure("59999")).retry_after_ms == 59_999
    assert error_of_status("PollTasks", failure("99999999999999999999")).retry_after_ms == 60_000


# --------------------------------------------------------------------------- 3. the survivors


async def test_stop_ends_the_heartbeats_of_a_task_that_will_not_end(
    fake: FakeWorkerService, make_host: MakeHost, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    monkeypatch.setattr(host_module, "SHUTDOWN_GRACE", 0.1)
    host = make_host(heartbeat_interval=0.02)
    release = asyncio.Event()

    async def stuck(_i: Any, _c: Any) -> int:
        await release.wait()
        return 1

    host.register_activity("charge", stuck)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("Heartbeat")) >= 2, "the task, beating")
    await host.stop(0.05)
    beats = len(fake.of("Heartbeat"))
    with caplog.at_level(logging.DEBUG, logger="sagat_octopus_orchestrator"):
        caplog.clear()
        await asyncio.sleep(0.25)
    assert len(fake.of("Heartbeat")) == beats, "no heartbeat reaches the server once the host has stopped"
    assert "heartbeat" not in caplog.text, "and none is even tried"
    release.set()
    await asyncio.sleep(0.05)


async def test_stop_is_bounded_by_its_timeout_whatever_the_handlers_and_the_poll_do(
    fake: FakeWorkerService, make_host: MakeHost, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(host_module, "SHUTDOWN_GRACE", 0.1)
    host = make_host()
    release = asyncio.Event()

    async def stuck(_i: Any, _c: Any) -> int:
        await release.wait()
        return 1

    host.register_activity("charge", stuck)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: bool(host._inflight), "the task")
    began = asyncio.get_running_loop().time()
    await asyncio.wait_for(host.stop(0.2), timeout=5)  # a stop that waits for the handler would hang here
    assert asyncio.get_running_loop().time() - began < 1.5
    release.set()


async def test_stop_is_bounded_by_its_timeout_when_the_server_holds_the_poll(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.poll_delay = 5.0
    host = make_host()
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(lambda: len(fake.of("PollTasks")) == 1, "the poll the server holds")
    began = asyncio.get_running_loop().time()
    await asyncio.wait_for(host.stop(0.3), timeout=5)
    assert asyncio.get_running_loop().time() - began < 1.5


async def test_the_optional_fields_of_a_request_are_sent(http: FakeHttpServer) -> None:
    http.serve(
        lambda seen: reply(
            202,
            {"eventId": "e", "source": "api", "status": "ACCEPTED"}
            if seen.url.endswith("/events")
            else {"operationId": "o", "accepted": True, **run_body()},
        )
    )
    client = SagatClient(ClientConfig(api_url=http.base, namespace="p", token="t", user_agent="my-app/2.1 (test)"))
    await client.start(WORKFLOW, {"n": 1}, version_id="0190aaaa-0000-7000-8000-0000000000aa", correlation_id="corr-1")
    await client.operate(
        RUN, "RESOLVE_STEP", "decided by a person", step_id="0190aaaa-0000-7000-8000-0000000000bb",
        outcome="FAILED", error_code="NO_STOCK", expected_run_version=7,
    )  # fmt: skip
    when = datetime(2026, 10, 10, 12, 30, 15, 250000, tzinfo=timezone(timedelta(hours=-3)))
    await client.publish_event(
        "payment.confirmed", {"id": 1}, event_id="evt-12345678", subject="order/42", occurred_at=when,
        correlation_key="ord-42", dedupe_ttl_seconds=3600,
    )  # fmt: skip
    start, operate, event = http.seen
    assert start.body == {
        "workflowId": WORKFLOW, "input": {"n": 1}, "versionId": "0190aaaa-0000-7000-8000-0000000000aa", "correlationId": "corr-1",
    }  # fmt: skip
    assert operate.body == {
        "action": "RESOLVE_STEP", "reason": "decided by a person", "stepId": "0190aaaa-0000-7000-8000-0000000000bb",
        "outcome": "FAILED", "errorCode": "NO_STOCK", "expectedRunVersion": 7,
    }  # fmt: skip
    assert event.body == {
        "eventId": "evt-12345678", "type": "payment.confirmed", "data": {"id": 1}, "subject": "order/42",
        "occurredAt": "2026-10-10T15:30:15.250Z", "correlationKey": "ord-42", "dedupeTtlSeconds": 3600,
    }  # fmt: skip
    for seen in http.seen:
        assert seen.headers["user-agent"] == "sagat-sdk-python/0.2.0 my-app/2.1 (test)"
    await client.aclose()


async def test_the_optional_fields_that_are_absent_are_not_sent(http: FakeHttpServer, make_client: MakeClient) -> None:
    http.serve(lambda _seen: reply(202, {"operationId": "o", "accepted": True, **run_body()}))
    client = make_client()
    await client.start(WORKFLOW)
    await client.operate(RUN, "PAUSE", "maintenance window")
    assert http.seen[0].body == {"workflowId": WORKFLOW, "input": {}}
    assert http.seen[1].body == {"action": "PAUSE", "reason": "maintenance window"}
    assert http.seen[0].headers["user-agent"] == "sagat-sdk-python/0.2.0"


async def test_the_list_filters_are_sent(http: FakeHttpServer, make_client: MakeClient) -> None:
    http.serve(lambda _seen: reply(200, {"items": [], "nextCursor": None}))
    await make_client().list(workflow_id=WORKFLOW, status="RUNNING", limit=5, cursor="c1")
    url = http.seen[0].url
    for part in (f"workflowId={WORKFLOW}", "status=RUNNING", "limit=5", "cursor=c1"):
        assert part in url


async def test_a_context_has_the_fields_the_spec_lists(fake: FakeWorkerService, make_host: MakeHost) -> None:
    host = make_host()
    seen: list[ActivityContext] = []
    host.register_activity("charge", lambda _i, ctx: seen.append(ctx), blocking=False)
    fake.leases = [lease(attempt=3, generation=7)]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    ctx = seen[0]
    assert (ctx.attempt, ctx.generation, ctx.task_id, ctx.activity_name) == (3, 7, "run-1/charge/1", "charge")
    assert ctx.deadline > datetime.now(timezone.utc)
