"""The correctness review's findings (round 3): each of these failed before its fix."""

from __future__ import annotations

import asyncio
import json
import re
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import grpc
import pytest

from conformance.runner import build as build_scenario
from sagat_octopus_orchestrator import (
    ActivityContext,
    ActivityFailure,
    RetryPolicy,
    SagatError,
    WorkerHost,
    _http,
    backoff_delay,
    canonical_json,
    workflow,
)
from sagat_octopus_orchestrator.worker import host as host_module

from .fakes import FakeHttpServer, FakeWorkerService, Scripted, lease, reply, until
from .test_client import RUN, WORKFLOW, MakeClient, refusal, run_body

ROOT = Path(__file__).resolve().parent.parent.parent.parent
CONTRACT = ROOT / "tests" / "contract" / "sdk-conformance"
MakeHost = Callable[..., WorkerHost]
TRACE = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"


# ------------------------------------------------------------------------------------ B1. the exponent


def test_the_backoff_of_a_very_long_outage_is_the_cap_not_an_overflow() -> None:
    policy = RetryPolicy()
    for n in (62, 1023, 1024, 2000, 10**9):
        assert backoff_delay(policy, n, lambda: 1.0) == pytest.approx(policy.max_delay), n
    assert backoff_delay(RetryPolicy(initial_delay=0.0), 5000, lambda: 1.0) == 0.0


async def test_a_host_that_met_1100_failed_polls_in_a_row_polls_on_and_serves(
    fake: FakeWorkerService, make_host: MakeHost, monkeypatch: pytest.MonkeyPatch
) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)

    async def brief(_seconds: float) -> None:
        await asyncio.sleep(0.0005)

    monkeypatch.setattr(host, "_idle", brief)
    fake.failures["PollTasks"] = [Scripted(grpc.StatusCode.UNAVAILABLE) for _ in range(1100)]
    await host.start()
    await until(lambda: len(fake.of("PollTasks")) > 1105, "the polls after the 1100 failures", seconds=30)
    assert host._loop_task is not None and not host._loop_task.done()
    fake.leases = [lease()]
    await until(lambda: len(fake.of("CompleteStep")) == 1, "a task after the outage")


# ---------------------------------------------------------------------------------- B2. cooperative


async def test_a_handler_that_ignores_the_token_runs_to_its_end_after_run_cancelled_and_is_reported(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)
    interrupted: list[bool] = []

    async def stubborn(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        try:
            await asyncio.sleep(0.3)  # never looks at its token
        except asyncio.CancelledError:
            interrupted.append(True)
            raise
        return {"finished": True, "saw": ctx.cancel_reason is not None}

    host.register_activity("charge", stubborn)
    fake.cancel_requested = True
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) + len(fake.of("FailStep")) == 1, "a report")
    assert not interrupted, "the host never injects a cancellation into the handler"
    assert not fake.of("FailStep")
    assert json.loads(fake.of("CompleteStep")[0].request.output_json) == {"finished": True, "saw": True}


async def test_a_handler_that_ignores_the_token_is_reported_as_usual_after_shutdown(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    interrupted: list[bool] = []
    reasons: list[str | None] = []

    async def stubborn(_i: Any, ctx: ActivityContext) -> int:
        try:
            await asyncio.sleep(0.3)
        except asyncio.CancelledError:
            interrupted.append(True)
            raise
        reasons.append(ctx.cancel_reason)
        return 7

    host.register_activity("charge", stubborn)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("PollTasks")) >= 1 and bool(host._inflight), "the task")
    await host.stop(0.05)
    assert reasons == ["shutdown"] and not interrupted
    assert [call.request.output_json for call in fake.of("CompleteStep")] == ["7"], "returned after shutdown: reported"
    assert not fake.of("FailStep")


async def test_stop_cancels_the_tokens_after_its_grace_and_returns_without_forcing_the_handlers(
    fake: FakeWorkerService, make_host: MakeHost, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(host_module, "SHUTDOWN_GRACE", 0.1)
    host = make_host()
    release = asyncio.Event()
    events: list[str] = []

    async def stuck(_i: Any, ctx: ActivityContext) -> int:
        try:
            await release.wait()  # waits for the test, not for the token
        except asyncio.CancelledError:
            events.append("interrupted")
            raise
        events.append(f"ended:{ctx.cancel_reason}")
        return 1

    host.register_activity("charge", stuck)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: bool(host._inflight), "the task")
    began = asyncio.get_running_loop().time()
    await host.stop(0.05)
    assert asyncio.get_running_loop().time() - began < 1.0
    assert events == [], "the handler was left alone"
    release.set()
    await asyncio.sleep(0.1)
    assert events == ["ended:shutdown"]
    assert not fake.of("CompleteStep"), "the connection is gone: its lease lapses and the server leases it again"


@pytest.mark.parametrize(
    ("how", "reported"),
    [("run-cancelled", "CANCELLED"), ("lease-lost", None), ("shutdown", None)],
)
async def test_a_handler_that_raises_its_own_cancellation_after_the_token_is_cancelled_is_read_by_the_reason(
    fake: FakeWorkerService, make_host: MakeHost, how: str, reported: str | None
) -> None:
    host = make_host(heartbeat_interval=0.02)

    async def cancels_itself(_i: Any, ctx: ActivityContext) -> None:
        await ctx.cancellation.wait()
        raise asyncio.CancelledError

    host.register_activity("charge", cancels_itself)
    fake.leases = [lease()]
    if how == "run-cancelled":
        fake.cancel_requested = True
    if how == "lease-lost":
        fake.failures["Heartbeat"] = [Scripted(grpc.StatusCode.ABORTED, "STALE_ATTEMPT")]
    await host.start()
    if how == "shutdown":
        await until(lambda: bool(host._inflight), "the task")
        await host.stop(0.05)
    else:
        await until(lambda: host._inflight == set() and len(fake.of("PollTasks")) > 2, "the task to end")
        await asyncio.sleep(0.1)
    fails = [(call.request.error_code, call.request.retryable) for call in fake.of("FailStep")]
    assert fails == ([(reported, False)] if reported else [])
    assert not fake.of("CompleteStep")


# ---------------------------------------------------------------------------------- B3. canonical text


def test_the_canonical_text_refuses_an_integer_beyond_2_to_the_53_minus_1() -> None:
    assert canonical_json({"n": 2**53 - 1, "m": -(2**53 - 1)}) == '{"m":-9007199254740991,"n":9007199254740991}'
    for big in (2**53, 2**53 + 1, 9007199254740993, -(2**53), 10**20):
        with pytest.raises(SagatError) as caught:
            canonical_json({"n": big})
        assert caught.value.code == "VALIDATION_FAILED", big
    with pytest.raises(SagatError):
        workflow("big-int").step("a", "x").input_schema({"maximum": 9007199254740993}).to_json()
    assert canonical_json(1e21) == "1e+21"
    assert canonical_json({"huge": 1e300}) == '{"huge":1e+300}'


def test_the_canonical_text_of_the_builders_scenario_is_the_golden_text_byte_for_byte() -> None:
    scenario = json.loads((CONTRACT / "scenarios" / "builders.json").read_text(encoding="utf-8"))
    expected = json.loads((CONTRACT / "expected" / "builders.workflows.json").read_text(encoding="utf-8"))
    built = build_scenario(scenario)
    assert built["canonical"] == expected["canonical"]
    assert "canonical-edges" in built["canonical"]
    edges = built["canonical"]["canonical-edges"]
    assert edges.index('"\U0001f600"') < edges.index('"' + chr(0xFF5E) + '"'), "UTF-16 order"
    assert '"maximum":1e+21' in edges and '"minimum":-1e-7' in edges and '"exclusiveMinimum":0.000001' in edges


# --------------------------------------------------------------------------------- B4. the safe message


async def test_a_safe_message_is_cut_by_code_points_and_a_lone_surrogate_never_loses_the_report(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()

    def failing(message: str) -> Callable[[Any, Any], None]:
        def handler(_i: Any, _c: Any) -> None:
            raise ActivityFailure.permanent("BAD", message)

        return handler

    cut = "a" * 511 + "\U0001f600" + "tail"
    host.register_activity("emoji", failing(cut), blocking=False)
    host.register_activity("lone", failing("bad\ud800 message \udfff end"), blocking=False)
    host.register_activity("exact", failing("é" * 512), blocking=False)
    fake.leases = [
        lease(task_id="r/emoji/1", activity_name="emoji", step_id="emoji"),
        lease(task_id="r/lone/1", activity_name="lone", step_id="lone"),
        lease(task_id="r/exact/1", activity_name="exact", step_id="exact"),
    ]
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 3, "all three failures reported")
    by = {call.request.step_id: call.request.safe_message for call in fake.of("FailStep")}
    assert by["emoji"] == "a" * 511 + "\U0001f600" and len(by["emoji"]) == 512
    assert by["lone"] == "bad� message � end"
    assert by["exact"] == "é" * 512
    for message in by.values():
        message.encode("utf-8")  # valid UTF-8, always


# ----------------------------------------------------------------------------------- B5. task kind <1.2


async def test_against_protocol_1_1_every_task_is_a_forward_task_and_the_target_fields_are_not_read(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.accepted_version = "1.1"
    host = make_host()
    seen: list[tuple[str, str, str]] = []

    def release(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        seen.append((ctx.task_kind, ctx.target_node, ctx.target_idempotency_key))
        return {"released": True}

    host.register_compensation("limit.release", release, blocking=False)
    host.register_reconciler("limit.lookup", lambda _i, _c: {"not": "a verdict"}, blocking=False)
    fake.leases = [
        lease(task_id="r/a/2", step_id="a", activity_name="limit.release", task_kind="COMPENSATION",
              target_node="reserve", target_idempotency_key="k"),
        lease(task_id="r/b/1", step_id="b", activity_name="limit.lookup", task_kind="RECONCILIATION", target_node="b"),
    ]  # fmt: skip
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 2, "both completions")
    assert seen == [("FORWARD", "", "")]
    assert not fake.of("FailStep"), "a task of a server at 1.1 is a forward task: no verdict is asked of it"


# ----------------------------------------------------------------------------- B6. concurrent start


async def test_two_concurrent_starts_register_once_and_run_one_loop(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    await asyncio.gather(host.start(), host.start(), host.start())
    assert len(fake.of("RegisterWorker")) == 1
    fake.leases = [lease()]
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the task")
    await asyncio.sleep(0.3)
    assert len(fake.of("CompleteStep")) == 1
    polls = [call.at for call in fake.of("PollTasks")]
    assert len(polls) <= 12, f"{len(polls)} polls in 0.3 s: one loop, not three"
    await asyncio.gather(host.stop(1), host.stop(1))


# ------------------------------------------------------------------------------ B7. Retry-After, B8


def test_a_retry_after_http_date_is_ignored() -> None:
    assert _http.retry_after_ms("Wed, 21 Oct 2099 07:28:00 GMT") is None
    assert _http.retry_after_ms("0") == 0 and _http.retry_after_ms("2") == 2000
    assert _http.retry_after_ms("1.5") is None and _http.retry_after_ms("-1") is None


async def test_a_retry_after_http_date_makes_no_wait(http: FakeHttpServer, make_client: MakeClient) -> None:
    http.serve(
        lambda _seen: reply(429, refusal("RATE_LIMITED", "slow", True), retry_after="Wed, 21 Oct 2099 07:28:00 GMT")
    )
    with pytest.raises(SagatError) as caught:
        await make_client(retry=RetryPolicy(max_attempts=1)).get(RUN)
    assert caught.value.retry_after_ms is None


@pytest.mark.parametrize(
    ("status", "body", "unsupported"),
    [
        (404, refusal("NOT_FOUND", "no route", details={"reason": "ROUTE_NOT_FOUND"}), True),
        (405, refusal("BAD_REQUEST", "wrong method", details={"reason": "METHOD_NOT_ALLOWED"}), True),
        (501, "", True),
        (404, "<html>nginx</html>", True),
        (404, {"unrelated": True}, True),
        (404, refusal("NOT_FOUND", "no such run"), False),
        (404, refusal("NOT_FOUND", "no such run", details={"reason": "RUN_NOT_FOUND"}), False),
        (405, refusal("BAD_REQUEST", "wrong method"), False),
    ],
)
async def test_a_call_the_server_does_not_have_is_told_from_a_resource_that_is_missing_by_the_reasons_of_the_contract(
    http: FakeHttpServer, make_client: MakeClient, status: int, body: Any, unsupported: bool
) -> None:
    http.serve(lambda _seen: reply(status, body))
    with pytest.raises(SagatError) as caught:
        await make_client(retry=RetryPolicy(max_attempts=1)).get(RUN)
    error = caught.value
    assert error.status == status
    if unsupported:
        assert (error.code, error.reason) == ("NOT_FOUND", "UNSUPPORTED_BY_SERVER")
    else:
        assert error.reason != "UNSUPPORTED_BY_SERVER" and (status != 404 or error.code == "NOT_FOUND")


# ---------------------------------------------------------------------------------- B9. the retry table


@pytest.mark.parametrize(
    ("status", "body", "attempts"),
    [
        (503, refusal("UNAVAILABLE", "busy", True), 4),
        (502, "<html>bad gateway</html>", 4),
        (408, "", 4),
        (408, refusal("UNAVAILABLE", "request timeout", True), 4),
        (429, refusal("RATE_LIMITED", "slow", True), 4),
        (429, refusal("QUOTA_EXCEEDED", "full", False), 1),
        (429, refusal("QUOTA_EXCEEDED", "full", True), 1),
        (500, refusal("INTERNAL", "boom"), 1),
        (500, refusal("INTERNAL", "boom", True), 4),
        (400, refusal("BAD_REQUEST", "bad", True), 1),
        (409, refusal("CONFLICT", "conflict", True), 1),
        (422, refusal("VALIDATION_FAILED", "invalid", True), 1),
        (404, refusal("UNAVAILABLE", "a 404 that claims it may be repeated", True), 1),
        (401, refusal("UNAUTHENTICATED", "no", True), 1),
    ],
)
async def test_the_retry_rule_is_by_code_and_a_4xx_that_says_retryable_is_not_believed(
    http: FakeHttpServer, make_client: MakeClient, status: int, body: Any, attempts: int
) -> None:
    http.serve(lambda _seen: reply(status, body))
    with pytest.raises(SagatError):
        await make_client().get(RUN)
    assert len(http.seen) == attempts, f"{status} {body!r}"


# ---------------------------------------------------------------- B10. what the suite did not see


async def test_the_poll_wakes_when_a_task_finishes_instead_of_waiting_out_the_pause(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.poll_retry_after_ms = 10_000  # an empty poll asks for a very long pause
    host = make_host(max_inflight=2)

    async def slow(_i: Any, _c: Any) -> int:
        await asyncio.sleep(0.3)
        return 1

    host.register_activity("charge", slow)
    fake.leases = [lease(task_id="run-1/charge/1", run_id="run-1")]
    await host.start()
    await until(lambda: len(fake.of("PollTasks")) >= 2, "the poll that came back empty, and its long pause")
    fake.leases = [lease(task_id="run-2/charge/1", run_id="run-2")]  # it waits for the next poll
    # Nothing but the finish of the first task can bring that poll within seconds instead of ~10 s.
    await until(
        lambda: len(fake.of("CompleteStep")) == 2, "the second task, picked up when the first finished", seconds=3
    )


async def test_the_heartbeat_goes_on_while_the_report_is_being_retried(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    fake.failures["CompleteStep"] = [Scripted(grpc.StatusCode.UNAVAILABLE) for _ in range(3)]
    fake.leases = [lease(expires_at_epoch_ms=0)]
    fake.always["Heartbeat"] = Scripted(grpc.StatusCode.UNAVAILABLE)
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 4, "the report, after three failures")
    first_report = fake.of("CompleteStep")[0].at
    beats_during = [call for call in fake.of("Heartbeat") if call.at > first_report]
    assert len(beats_during) >= 1, "the lease is kept alive until the report is made"


async def test_idempotency_key_is_sent_on_every_post_that_changes_something(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda seen: reply(202, {"operationId": "o", "accepted": True, "deliveredTo": []}))
    client = make_client()
    await client.signal(RUN, "go", {})
    await client.operate(RUN, "PAUSE", "maintenance window")
    await client.cancel(RUN, "no longer needed")
    http.serve(lambda _seen: reply(202, run_body()))
    await client.start(WORKFLOW, {})
    for seen in http.seen:
        assert re.fullmatch(r"[0-9a-f-]{36}", seen.headers["idempotency-key"]), seen.url
    assert len({seen.headers["idempotency-key"] for seen in http.seen}) == len(http.seen)
    http.serve(lambda _seen: reply(202, {"operationId": "o", "accepted": True, "deliveredTo": []}))
    await client.signal(RUN, "go", {}, idempotency_key="given-key-123")
    await client.operate(RUN, "PAUSE", "maintenance window", idempotency_key="given-key-456")
    assert [seen.headers["idempotency-key"] for seen in http.seen[-2:]] == ["given-key-123", "given-key-456"]


async def test_the_traceparent_of_the_lease_is_on_the_heartbeat_the_report_and_the_secrets_call(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)

    async def work(_i: Any, ctx: ActivityContext) -> int:
        await ctx.secrets()
        await asyncio.sleep(0.1)
        return 1

    host.register_activity("charge", work)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    for method in ("Heartbeat", "ResolveSecrets", "CompleteStep"):
        calls = fake.of(method)
        assert calls, method
        assert all(call.metadata.get("traceparent") == TRACE for call in calls), method
    assert fake.of("CompleteStep")[0].request.traceparent == TRACE
    assert fake.of("ResolveSecrets")[0].request.traceparent == TRACE


async def test_a_conflict_answered_to_a_heartbeat_is_a_lost_lease(fake: FakeWorkerService, make_host: MakeHost) -> None:
    host = make_host(heartbeat_interval=0.02)
    reasons: list[str | None] = []

    async def slow(_i: Any, ctx: ActivityContext) -> int:
        await ctx.cancellation.wait()
        reasons.append(ctx.cancel_reason)
        return 1

    host.register_activity("charge", slow)
    fake.failures["Heartbeat"] = [Scripted(grpc.StatusCode.ABORTED, "SOMETHING_ELSE")]  # CONFLICT, not STALE_ATTEMPT
    fake.leases = [lease()]
    await host.start()
    await until(lambda: bool(reasons), "the token to be cancelled")
    await asyncio.sleep(0.1)
    assert reasons == ["lease-lost"]
    assert not fake.of("CompleteStep") and not fake.of("FailStep")


@pytest.mark.parametrize(
    ("scripted", "limit"),
    [
        (Scripted(grpc.StatusCode.RESOURCE_EXHAUSTED, "RATE_LIMITED"), 8),
        (Scripted(grpc.StatusCode.RESOURCE_EXHAUSTED, "RATE_LIMITED", 300), 3),
        (Scripted(grpc.StatusCode.UNAVAILABLE), 5),
        (Scripted(grpc.StatusCode.INTERNAL), 5),
        (Scripted(grpc.StatusCode.UNAUTHENTICATED), 5),
    ],
)
async def test_a_refused_poll_does_not_make_the_host_poll_in_a_hot_loop(
    fake: FakeWorkerService, make_host: MakeHost, scripted: Scripted, limit: int
) -> None:
    fake.always["PollTasks"] = scripted
    host = make_host(retry=RetryPolicy())
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await asyncio.sleep(0.6)
    polls = len(fake.of("PollTasks"))
    assert 1 <= polls <= limit, f"{polls} polls in 0.6 s"


async def test_a_full_host_neither_polls_nor_spins(fake: FakeWorkerService, make_host: MakeHost) -> None:
    host = make_host(max_inflight=1)
    release = asyncio.Event()

    async def hold(_i: Any, _c: Any) -> int:
        await release.wait()
        return 1

    host.register_activity("charge", hold)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: bool(host._inflight), "the task")
    before, cpu = len(fake.of("PollTasks")), time.process_time()
    await asyncio.sleep(0.6)
    polls, spent = len(fake.of("PollTasks")) - before, time.process_time() - cpu
    release.set()
    assert polls == 0, f"{polls} polls while there was no room for a task"
    assert spent < 0.05, f"{spent:.3f} s of CPU in 0.6 s of waiting: the loop spins"


async def test_an_empty_poll_is_followed_by_a_wait_in_a_fixed_window(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.poll_retry_after_ms = 200
    host = make_host()
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await asyncio.sleep(1.0)
    polls = len(fake.of("PollTasks"))
    assert 3 <= polls <= 7, f"{polls} polls in 1 s with a pause of 200 ms (+-20 %)"


# ------------------------------------------------------------------- hooks and observers that raise


async def test_a_logger_that_raises_does_not_stop_the_poll_loop_or_drop_a_leased_task(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    import logging

    class Exploding(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            raise RuntimeError("the log shipper is down " + record.getMessage())

    logger = logging.getLogger("sagat-test-exploding")
    logger.addHandler(Exploding())
    logger.propagate = False
    logger.setLevel(logging.DEBUG)
    host = make_host(logger=logger, heartbeat_interval=0.02)
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    fake.failures["PollTasks"] = [Scripted(grpc.StatusCode.UNAVAILABLE), Scripted(grpc.StatusCode.UNAUTHENTICATED)]
    fake.failures["CompleteStep"] = [Scripted(grpc.StatusCode.UNAVAILABLE)]
    fake.leases = [lease()]
    await host.start()  # logs "worker registered" through the exploding handler
    await until(lambda: len(fake.of("CompleteStep")) == 2, "the task, reported after a failure, with the logger down")
    assert host._loop_task is not None and not host._loop_task.done()


def test_an_exponent_is_never_expanded_in_memory() -> None:
    parsed = json.loads('{"huge": 1e999999999, "tiny": 1e-999999999, "ok": 1e21}')
    with pytest.raises(SagatError) as caught:
        canonical_json(parsed)  # 1e999999999 is infinity in a float: refused, not a billion digits
    assert caught.value.code == "VALIDATION_FAILED"
    assert canonical_json({"tiny": parsed["tiny"], "ok": parsed["ok"]}) == '{"ok":1e+21,"tiny":0}'
