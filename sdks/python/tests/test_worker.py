"""The worker host against a fake ``WorkerService`` served in the same process."""

from __future__ import annotations

import asyncio
import json
import logging
import threading
import time
from collections.abc import Callable
from typing import Any

import grpc
import pytest

from sagat_octopus_orchestrator import (
    ActivityContext,
    ActivityFailure,
    SagatError,
    Verdict,
    WorkerEvent,
    WorkerHost,
    run_blocking,
)

from .fakes import FakeWorkerService, Scripted, lease, until

MakeHost = Callable[..., WorkerHost]
TRACE = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"


def named(*names: str) -> list[Any]:
    """One lease per activity name, each its own task."""
    return [lease(task_id=f"run-{i}/{name}/1", activity_name=name, step_id=name) for i, name in enumerate(names)]


async def test_a_worker_registers_its_activities_and_the_newest_protocol_it_speaks_with_the_token_as_metadata(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(languages=["python"], max_inflight=7)
    host.register_activity("b.second", lambda _i, _c: 1).register_compensation("a.undo", lambda _i, _c: 1)
    host.register_reconciler("c.lookup", lambda _i, _c: Verdict.unknown())
    await host.start()
    [register] = fake.of("RegisterWorker")
    request = register.request
    assert (request.worker_name, request.namespace) == ("test-worker", "payments")
    assert list(request.capabilities.languages) == ["python"]
    assert list(request.capabilities.activities) == ["a.undo", "b.second", "c.lookup"]
    assert list(request.capabilities.pools) == []
    assert (request.capabilities.max_inflight, request.capabilities.protocol_version) == (7, "1.2")
    assert register.metadata["authorization"] == "Bearer tok"
    assert host.protocol_version == "1.2"


async def test_a_task_is_run_with_its_context_and_completed_once_with_a_key_that_is_the_same_on_every_retry(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    seen: list[ActivityContext] = []
    received: list[Any] = []
    events: list[WorkerEvent] = []
    host = make_host(on_event=events.append)

    async def charge(run_input: dict[str, int], ctx: ActivityContext) -> dict[str, int]:
        seen.append(ctx)
        received.append(run_input)
        return {"charged": run_input["amount"] * 2}

    host.register_activity("charge", charge)
    fake.failures["CompleteStep"] = [
        Scripted(grpc.StatusCode.UNAVAILABLE),
        Scripted(grpc.StatusCode.RESOURCE_EXHAUSTED, "RATE_LIMITED", retry_after_ms=1),
    ]
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 3, "three attempts to report")
    assert received == [{"amount": 5}]
    [ctx] = seen
    assert (ctx.run_id, ctx.node, ctx.step_id, ctx.attempt, ctx.generation) == ("run-1", "charge", "charge", 1, 1)
    assert (ctx.idempotency_key, ctx.task_kind, ctx.traceparent) == ("run-1:charge", "FORWARD", TRACE)
    assert dict(ctx.steps) == {"earlier": {"ok": True}}
    assert ctx.activity_name == "charge" and ctx.task_id == "run-1/charge/1"
    keys = {call.request.idempotency_key for call in fake.of("CompleteStep")}
    assert keys == {"done-run-1/charge/1"}
    last = fake.of("CompleteStep")[2].request
    assert (last.output_json, last.lease_token, last.generation, last.traceparent) == ('{"charged":10}', "lt", 1, TRACE)
    assert fake.of("CompleteStep")[2].metadata["traceparent"] == TRACE
    await until(lambda: any(e.type == "task.completed" for e in events), "the completed event")
    assert [e.type for e in events if e.type in ("registered", "task.taken")] == ["registered", "task.taken"]


async def test_the_node_is_the_one_of_the_step_document_and_the_step_id_the_one_of_the_lease(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    nodes: list[tuple[str, str]] = []
    host.register_activity("charge", lambda _i, ctx: nodes.append((ctx.node, ctx.step_id)), blocking=False)
    document = json.dumps({"run": {"id": "run-1"}, "node": "charge-node", "input": {}, "steps": {}})
    fake.leases = [lease(step_id="0190aaaa-0000-7000-8000-000000000009", input_json=document)]
    fake.leases += [lease(task_id="run-2/charge/1", step_id="uuid-2", input_json="")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 2, "both completions")
    assert sorted(nodes) == [("charge-node", "0190aaaa-0000-7000-8000-000000000009"), ("uuid-2", "uuid-2")]


async def test_a_failure_on_purpose_carries_its_code_and_whether_it_may_be_retried_any_other_error_says_only_its_type(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()

    def permanent(_i: Any, _c: Any) -> None:
        raise ActivityFailure.permanent("INSUFFICIENT_FUNDS", "not enough")

    def unknown(_i: Any, _c: Any) -> None:
        raise ActivityFailure.outcome_unknown("the provider timed out")

    async def boom(_i: Any, _c: Any) -> None:
        raise TypeError("secret card 4111 leaked in the message")

    async def badcode(_i: Any, _c: Any) -> None:
        raise ActivityFailure("not a code!", "x", False)

    async def long_message(_i: Any, _c: Any) -> None:
        raise ActivityFailure.retryable("LONG", "m" * 1000)

    for name, handler in (
        ("permanent", permanent),
        ("unknown", unknown),
        ("boom", boom),
        ("badcode", badcode),
        ("long", long_message),
    ):
        host.register_activity(name, handler)
    fake.leases = named("permanent", "unknown", "boom", "badcode", "long")
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 5, "five failures")
    by = {call.request.step_id: call.request for call in fake.of("FailStep")}
    assert (by["permanent"].error_code, by["permanent"].retryable, by["permanent"].safe_message) == (
        "INSUFFICIENT_FUNDS",
        False,
        "not enough",
    )
    assert (by["unknown"].error_code, by["unknown"].retryable) == ("OUTCOME_UNKNOWN", False)
    assert (by["boom"].error_code, by["boom"].retryable, by["boom"].safe_message) == (
        "ACTIVITY_ERROR",
        True,
        "TypeError",
    )
    assert by["badcode"].error_code == "WORKER_ERROR"
    assert len(by["long"].safe_message) == 512
    assert by["permanent"].idempotency_key == "failed-run-0/permanent/1"
    assert "4111" not in str([call.request for call in fake.calls])


async def test_a_stale_report_is_dropped_and_never_retried(fake: FakeWorkerService, make_host: MakeHost) -> None:
    events: list[WorkerEvent] = []
    host = make_host(on_event=events.append)
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    fake.failures["CompleteStep"] = [Scripted(grpc.StatusCode.ABORTED, "STALE_ATTEMPT")]
    fake.leases = [lease()]
    await host.start()
    await until(lambda: any(e.type == "task.stale" for e in events), "the stale event")
    assert len(fake.of("CompleteStep")) == 1


async def test_an_input_the_handler_cannot_take_an_activity_nobody_registered_and_an_output_too_large_are_failures_for_good(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()

    def refuse(_raw: Any) -> Any:
        raise ValueError("not an order")

    host.register_activity("typed", lambda _i, _c: 1, parse=refuse)
    host.register_activity("huge", lambda _i, _c: "x" * (300 * 1024))
    host.register_activity("bad-number", lambda _i, _c: {"n": float("nan")})
    host.register_activity("bad-type", lambda _i, _c: {"when": time})
    fake.leases = [
        lease(task_id="r/typed/1", activity_name="typed", step_id="typed"),
        lease(task_id="r/huge/1", activity_name="huge", step_id="huge"),
        lease(task_id="r/nobody/1", activity_name="nobody", step_id="nobody"),
        lease(task_id="r/bad-number/1", activity_name="bad-number", step_id="bad-number"),
        lease(task_id="r/junk/1", activity_name="typed", step_id="junk", input_json="{nope"),
        lease(task_id="r/bad-type/1", activity_name="bad-type", step_id="bad-type"),
    ]
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 6, "six failures")

    def code(step: str) -> str:
        return str(next(call.request.error_code for call in fake.of("FailStep") if call.request.step_id == step))

    assert code("typed") == "INPUT_INVALID"
    assert code("huge") == "PAYLOAD_TOO_LARGE"
    assert code("nobody") == "NO_HANDLER"
    assert code("bad-number") == "OUTPUT_INVALID"
    assert code("bad-type") == "OUTPUT_INVALID"
    assert code("junk") == "INPUT_INVALID"
    assert all(call.request.retryable is False for call in fake.of("FailStep"))
    assert not fake.of("CompleteStep")


async def test_an_output_of_exactly_256_kib_is_sent_and_a_null_result_is_the_json_null(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    host.register_activity(
        "exact", lambda _i, _c: "x" * (256 * 1024 - 2), blocking=False
    )  # the two quotes make 256 KiB
    host.register_activity("nothing", lambda _i, _c: None, blocking=False)
    fake.leases = named("exact", "nothing")
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 2, "both completions")
    outputs = {call.request.step_id: call.request.output_json for call in fake.of("CompleteStep")}
    assert len(outputs["exact"].encode()) == 256 * 1024 and outputs["nothing"] == "null"


async def test_the_heartbeat_starts_before_the_handler_ends_carries_the_step_identity_and_moves_the_deadline(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)
    deadlines: list[float] = []

    async def slow(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        deadlines.append(ctx.deadline.timestamp())
        await asyncio.sleep(0.15)
        deadlines.append(ctx.deadline.timestamp())
        return {"ok": True}

    host.register_activity("slow", slow)
    fake.heartbeat_expires_ms = int(time.time() * 1000) + 120_000
    fake.leases = [lease(activity_name="slow", step_id="slow", task_id="run-1/slow/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    beats = fake.of("Heartbeat")
    assert len(beats) >= 3, f"{len(beats)} beats"
    first = beats[0].request
    assert (first.run_id, first.step_id, first.attempt, first.lease_token, first.worker_id) == (
        "run-1",
        "slow",
        1,
        "lt",
        "w-1",
    )
    assert deadlines[1] > deadlines[0], "the lease was extended"
    count = len(fake.of("Heartbeat"))
    await asyncio.sleep(0.1)
    assert len(fake.of("Heartbeat")) == count, "no beat after the report"


async def test_a_cancellation_reaches_the_activity_as_a_cancelled_error_and_on_its_token_and_it_ends_however_it_likes(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)
    seen: dict[str, Any] = {}

    async def long(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        try:
            await asyncio.sleep(60)
        except asyncio.CancelledError as cancelled:
            seen["error"] = cancelled.args
            seen["reason"] = ctx.cancel_reason
            seen["flag"] = ctx.cancellation.is_cancelled
        return {"stopped": True}

    host.register_activity("long", long)
    fake.cancel_requested = True
    fake.leases = [lease(activity_name="long", step_id="long", task_id="run-1/long/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert seen == {"error": ("run-cancelled",), "reason": "run-cancelled", "flag": True}
    assert fake.of("CompleteStep")[0].request.output_json == '{"stopped":true}'


async def test_an_activity_that_waits_on_the_token_is_woken_by_it(fake: FakeWorkerService, make_host: MakeHost) -> None:
    host = make_host(heartbeat_interval=0.02)

    async def long(_i: Any, ctx: ActivityContext) -> str:
        return await ctx.cancellation.wait()

    host.register_activity("long", long)
    fake.cancel_requested = True
    fake.leases = [lease(activity_name="long", step_id="long", task_id="run-1/long/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) + len(fake.of("FailStep")) >= 1, "a report")
    # Either the token or the cancelled task is seen first; the answer is a report all the same.
    assert fake.of("CompleteStep") or fake.of("FailStep")[0].request.error_code == "CANCELLED"


async def test_an_activity_that_lets_its_cancellation_through_is_failed_for_good_as_cancelled(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)
    host.register_activity("long", lambda _i, _c: asyncio.sleep(60))
    host.register_activity("raises", _raises_its_cancellation)
    fake.cancel_requested = True
    fake.leases = named("long", "raises")
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 2, "the failures")
    assert {(call.request.error_code, call.request.retryable) for call in fake.of("FailStep")} == {("CANCELLED", False)}


async def _raises_its_cancellation(_i: Any, ctx: ActivityContext) -> None:
    await ctx.cancellation.wait()
    ctx.cancellation.raise_if_cancelled()


async def test_a_lease_lost_during_the_activity_stops_it_and_its_result_is_never_reported(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    host = make_host(heartbeat_interval=0.02, on_event=events.append)
    seen: list[str | None] = []

    async def slow(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        try:
            await ctx.cancellation.wait()
        finally:
            seen.append(ctx.cancel_reason)
        return {"late": True}

    host.register_activity("slow", slow)
    fake.failures["Heartbeat"] = [Scripted(grpc.StatusCode.ABORTED, "STALE_ATTEMPT")]
    fake.leases = [lease(activity_name="slow", step_id="slow", task_id="run-1/slow/1")]
    await host.start()
    await until(lambda: any(e.type == "task.stale" for e in events), "the stale event")
    assert seen == ["lease-lost"]
    assert not fake.of("CompleteStep") and not fake.of("FailStep")


async def test_a_compensation_sees_its_kind_and_the_effect_it_undoes_and_a_reconciler_answers_exactly_a_verdict(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    seen: list[dict[str, Any]] = []
    host = make_host()

    def release(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        seen.append(
            {
                "kind": ctx.task_kind,
                "target": ctx.target_node,
                "target_key": ctx.target_idempotency_key,
                "key": ctx.idempotency_key,
            }
        )
        return {"released": True}

    host.register_compensation("limit.release", release)
    host.register_reconciler("limit.lookup", lambda _i, _c: Verdict.applied({"id": "r-9"}))
    host.register_reconciler("limit.none", lambda _i, _c: Verdict.not_applied())
    host.register_reconciler("limit.empty", lambda _i, _c: Verdict.applied())
    host.register_reconciler("limit.null", lambda _i, _c: Verdict.applied(None))
    host.register_reconciler("limit.bad", lambda _i, _c: {"yes": True})
    reconciliation = {"task_kind": "RECONCILIATION"}
    fake.leases = [
        lease(task_id="run-1/reserve~c/2", step_id="reserve~c", activity_name="limit.release", task_kind="COMPENSATION",
              target_node="reserve", target_idempotency_key="run-1:reserve", idempotency_key="compensation:c1:run-1"),
        lease(task_id="run-1/a~r/1", step_id="a~r", activity_name="limit.lookup", target_node="a", target_idempotency_key="k", **reconciliation),
        lease(task_id="run-1/b~r/1", step_id="b~r", activity_name="limit.none", target_node="b", **reconciliation),
        lease(task_id="run-1/c~r/1", step_id="c~r", activity_name="limit.bad", target_node="c", **reconciliation),
        lease(task_id="run-1/d~r/1", step_id="d~r", activity_name="limit.empty", target_node="d", **reconciliation),
        lease(task_id="run-1/e~r/1", step_id="e~r", activity_name="limit.null", target_node="e", **reconciliation),
    ]  # fmt: skip
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 5 and len(fake.of("FailStep")) == 1, "every report")
    assert seen == [
        {"kind": "COMPENSATION", "target": "reserve", "target_key": "run-1:reserve", "key": "compensation:c1:run-1"}
    ]

    def out(step: str) -> str:
        return str(next(call.request.output_json for call in fake.of("CompleteStep") if call.request.step_id == step))

    assert out("a~r") == '{"effect":"applied","output":{"id":"r-9"}}'
    assert out("b~r") == '{"effect":"not_applied"}'
    assert out("d~r") == '{"effect":"applied"}'
    assert out("e~r") == '{"effect":"applied","output":null}'
    assert fake.of("FailStep")[0].request.error_code == "RECONCILIATION_ANSWER_INVALID"


async def test_the_host_runs_at_the_version_the_server_agreed_to_against_1_0_it_sends_no_step_identity_and_cannot_resolve_secrets(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.accepted_version = "1"
    host = make_host(heartbeat_interval=0.02, protocol_version="1.9")
    outcome: list[Any] = []

    async def slow(_i: Any, ctx: ActivityContext) -> int:
        await asyncio.sleep(0.08)
        try:
            await ctx.secrets()
        except SagatError as error:
            outcome.append(error)
        return 1

    host.register_activity("slow", slow)
    fake.leases = [lease(activity_name="slow", step_id="slow", task_id="run-1/slow/1")]
    await host.start()
    assert fake.of("RegisterWorker")[0].request.capabilities.protocol_version == "1.9"
    assert host.protocol_version == "1"
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    beat = fake.of("Heartbeat")[0].request
    assert (beat.run_id, beat.step_id, beat.attempt) == ("", "", 0)
    assert beat.lease_token == "lt" and beat.generation == 1 and beat.task_id == "run-1/slow/1"
    assert outcome and outcome[0].code == "UNSUPPORTED_PROTOCOL"
    assert not fake.of("ResolveSecrets")


async def test_secrets_are_resolved_once_per_attempt_on_first_use_as_a_read_only_map_and_dropped_when_the_handler_returns(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    kept: list[Any] = []
    text: list[str] = []

    async def pay(_i: Any, ctx: ActivityContext) -> dict[str, Any]:
        first, second = await ctx.secrets(), await ctx.secrets()
        assert first is second
        text.append(first.text("stripe-key"))
        kept.append(first)
        assert "stripe-key" in first and len(first) == 1 and "sk_live" not in repr(first)
        with pytest.raises(SagatError) as missing:
            first.get("other")
        assert missing.value.code == "NOT_FOUND"
        return {"names": first.names(), "length": len(first.get("stripe-key"))}

    host.register_activity("pay", pay)
    fake.secrets = [_secret("stripe-key", b"sk_live_x")]
    fake.leases = [lease(activity_name="pay", step_id="pay", task_id="run-1/pay/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert text == ["sk_live_x"]
    assert len(fake.of("ResolveSecrets")) == 1
    resolve = fake.of("ResolveSecrets")[0]
    assert (resolve.request.lease_token, resolve.request.step_id, resolve.request.attempt) == ("lt", "pay", 1)
    output = fake.of("CompleteStep")[0].request.output_json
    assert "sk_live" not in output and json.loads(output) == {"names": ["stripe-key"], "length": 9}
    with pytest.raises(SagatError):
        kept[0].get("stripe-key")


def _secret(name: str, value: bytes) -> Any:
    from sagat_octopus_orchestrator._proto import worker_pb2 as pb

    return pb.ResolvedSecret(name=name, value=value, version=3)


async def test_a_session_that_is_gone_is_registered_again_and_the_worker_goes_on(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    host = make_host(on_event=events.append)
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    await host.start()
    fake.failures["PollTasks"] = [Scripted(grpc.StatusCode.UNAUTHENTICATED)]
    await until(lambda: len(fake.of("RegisterWorker")) == 2, "a second registration")
    fake.leases = [lease()]
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the task after it")
    assert any(e.type == "reconnect" for e in events)


async def test_a_poll_that_fails_is_followed_by_another_after_a_pause_and_a_registration_that_never_left_is_repeated(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    fake.failures["RegisterWorker"] = [Scripted(grpc.StatusCode.UNAVAILABLE)]
    fake.failures["PollTasks"] = [
        Scripted(grpc.StatusCode.UNAVAILABLE),
        Scripted(grpc.StatusCode.RESOURCE_EXHAUSTED, "RATE_LIMITED", 5),
    ]
    fake.leases = [lease()]
    await host.start()
    assert len(fake.of("RegisterWorker")) == 2
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the task after the failed polls")
    assert len(fake.of("PollTasks")) >= 3


async def test_an_empty_poll_is_followed_by_a_wait_never_by_another_poll_at_once(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    await host.start()
    await asyncio.sleep(0.4)
    polls = len(fake.of("PollTasks"))
    assert 2 <= polls <= 10, f"{polls} polls in 400 ms"


async def test_no_more_tasks_are_taken_than_the_host_has_room_for_and_stop_waits_for_the_ones_in_flight(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(max_inflight=2)
    running = peak = 0

    async def charge(_i: Any, _c: Any) -> int:
        nonlocal running, peak
        running += 1
        peak = max(peak, running)
        await asyncio.sleep(0.1)
        running -= 1
        return 1

    host.register_activity("charge", charge)
    fake.leases = [lease(task_id=f"run-{i}/charge/1", run_id=f"run-{i}") for i in (1, 2, 3, 4)]
    await host.start()
    await until(lambda: len(fake.of("PollTasks")) > 0, "a poll")
    assert fake.of("PollTasks")[0].request.max_tasks <= 2
    await host.stop(5)
    assert peak <= 2
    assert running == 0, "nothing was left running"
    assert len(fake.of("CompleteStep")) >= 2, "what was in flight was reported"


async def test_stop_while_a_poll_is_in_flight_does_not_wait_out_the_pause_an_empty_answer_asks_for(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.poll_delay = 0.15
    fake.poll_retry_after_ms = 10_000
    host = make_host()
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(lambda: len(fake.of("PollTasks")) == 1, "the first poll")
    began = time.monotonic()
    await host.stop(1)
    assert time.monotonic() - began < 1.5, f"stop took {time.monotonic() - began:.2f} s"


async def test_a_task_that_outlives_the_stop_is_cancelled_with_shutdown_and_nothing_is_reported_for_it(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    host = make_host(on_event=events.append)
    reasons: list[str | None] = []

    async def stuck(_i: Any, ctx: ActivityContext) -> None:
        try:
            await asyncio.sleep(60)
        finally:
            reasons.append(ctx.cancel_reason)

    host.register_activity("stuck", stuck)
    fake.leases = [lease(activity_name="stuck", step_id="stuck", task_id="run-1/stuck/1")]
    await host.start()
    await until(lambda: any(e.type == "task.taken" for e in events), "the task")
    await host.stop(0.1)
    assert reasons == ["shutdown"]
    assert not fake.of("CompleteStep") and not fake.of("FailStep"), "its lease lapses and the server leases it again"
    assert any(e.type == "task.dropped" and e.reason == "shutdown" for e in events)


async def test_the_host_can_be_started_again_after_it_stopped(fake: FakeWorkerService, make_host: MakeHost) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    await host.start()
    await host.stop(1)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the task after the restart")
    assert len(fake.of("RegisterWorker")) == 2


async def test_a_plain_function_runs_in_a_thread_and_does_not_stop_the_event_loop(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(heartbeat_interval=0.02)
    threads: list[str] = []
    cancelled = threading.Event()

    def blocking(_i: Any, ctx: ActivityContext) -> dict[str, bool]:
        threads.append(threading.current_thread().name)
        if ctx.cancellation.wait_blocking(30):  # cooperative: a thread cannot be interrupted
            cancelled.set()
        return {"thread": True}

    host.register_activity("blocking", blocking)
    fake.cancel_requested = True
    fake.leases = [lease(activity_name="blocking", step_id="blocking", task_id="run-1/blocking/1")]
    await host.start()
    ticks = 0
    while not fake.of("CompleteStep") and ticks < 500:  # the loop keeps turning while the thread waits
        await asyncio.sleep(0.01)
        ticks += 1
    assert cancelled.is_set() and threads[0] != threading.current_thread().name
    assert threads[0].startswith("sagat-activity")
    assert fake.of("CompleteStep")[0].request.output_json == '{"thread":true}'


async def test_run_blocking_runs_a_function_in_a_thread() -> None:
    main = threading.current_thread()
    assert await run_blocking(lambda a, b=1: (a + b, threading.current_thread() is main), 2, b=3) == (5, False)


async def test_a_callable_object_with_an_async_call_is_run_as_a_coroutine(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()

    class Callable:
        async def __call__(self, _i: Any, _c: Any) -> str:
            return "called"

    host.register_activity("object", Callable())
    fake.leases = named("object")
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert fake.of("CompleteStep")[0].request.output_json == '"called"'


async def test_the_context_logger_carries_identifiers_and_never_the_payload(
    fake: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    host = make_host()

    async def charge(_i: Any, ctx: ActivityContext) -> int:
        ctx.log.info("charging")
        return 1

    host.register_activity("charge", charge)
    fake.leases = [lease()]
    with caplog.at_level(logging.DEBUG, logger="sagat_octopus_orchestrator"):
        await host.start()
        await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert "charging" in caplog.text and "task=run-1/charge/1" in caplog.text
    assert "lt" not in {record.getMessage() for record in caplog.records} and "Bearer" not in caplog.text


async def test_registration_is_refused_clearly_an_unsupported_protocol_an_unbound_identity(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: 1)
    fake.failures["RegisterWorker"] = [Scripted(grpc.StatusCode.INVALID_ARGUMENT, "UNSUPPORTED_PROTOCOL")]
    with pytest.raises(SagatError) as unsupported:
        await host.start()
    assert (unsupported.value.code, unsupported.value.retryable, unsupported.value.status) == (
        "UNSUPPORTED_PROTOCOL",
        False,
        "INVALID_ARGUMENT",
    )
    assert unsupported.value.reason == "UNSUPPORTED_PROTOCOL"
    assert len(fake.of("RegisterWorker")) == 1, "not retried"

    second = make_host(worker_name="second")
    second.register_activity("charge", lambda _i, _c: 1)
    fake.failures["RegisterWorker"] = [Scripted(grpc.StatusCode.PERMISSION_DENIED, "NOT_BOUND")]
    with pytest.raises(SagatError) as unbound:
        await second.start()
    assert (unbound.value.code, unbound.value.status, unbound.value.reason) == (
        "NOT_BOUND",
        "PERMISSION_DENIED",
        "NOT_BOUND",
    )
    assert len(fake.of("RegisterWorker")) == 2
    assert second.protocol_version == ""


async def test_a_token_provider_gets_one_more_chance_when_the_registration_is_refused_as_unauthenticated(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    tokens: list[str] = []

    def provider() -> str:
        tokens.append(f"tok-{len(tokens) + 1}")
        return tokens[-1]

    host = make_host(token=provider)
    host.register_activity("charge", lambda _i, _c: 1)
    fake.failures["RegisterWorker"] = [Scripted(grpc.StatusCode.UNAUTHENTICATED)]
    await host.start()
    assert [call.metadata["authorization"] for call in fake.of("RegisterWorker")] == ["Bearer tok-1", "Bearer tok-2"]


async def test_registering_twice_or_after_the_start_is_a_mistake_and_says_so(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    host.register_activity("a", lambda _i, _c: 1)
    with pytest.raises(SagatError) as twice:
        host.register_activity("a", lambda _i, _c: 2)
    assert twice.value.code == "VALIDATION_FAILED"
    for bad in (0, 1001, "3"):
        with pytest.raises(SagatError):
            make_host(max_inflight=bad)
    with pytest.raises(SagatError) as empty:
        await make_host().start()
    assert "at least one activity" in empty.value.message
    await host.start()
    with pytest.raises(SagatError):
        host.register_activity("late", lambda _i, _c: 1)


async def test_a_failing_event_hook_never_stops_the_host(fake: FakeWorkerService, make_host: MakeHost) -> None:
    def broken(_event: WorkerEvent) -> None:
        raise RuntimeError("metrics are down")

    host = make_host(on_event=broken)
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")


async def test_plain_grpc_outside_loopback_warns_once_at_start(caplog: pytest.LogCaptureFixture) -> None:
    from sagat_octopus_orchestrator.config import warn_if_plaintext

    logger = logging.getLogger("sagat_octopus_orchestrator")
    with caplog.at_level(logging.WARNING, logger="sagat_octopus_orchestrator"):
        warn_if_plaintext(logger, "http://worker.example.com:443", "grpc_url")
        warn_if_plaintext(logger, "http://localhost:5000", "grpc_url")
        warn_if_plaintext(logger, "http://[::1]:5000", "grpc_url")
        warn_if_plaintext(logger, "https://worker.example.com:443", "grpc_url")
    assert len(caplog.records) == 1 and "grpc_url" in caplog.text
