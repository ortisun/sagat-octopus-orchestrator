"""The hand-back of ``sdks/SPEC.md`` section 6.2: on a session of protocol 1.8 or later, a lease the host received and
will not run is reported with ``FailStep`` ``TASK_NOT_STARTED`` before any handler of it runs, once, and never for a task
whose handler started; below 1.8 it lapses."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Callable
from typing import Any

import grpc
import pytest

from sagat_octopus_orchestrator import ActivityContext, ActivityFailure, WorkerEvent, WorkerHost
from sagat_octopus_orchestrator._proto import worker_pb2 as pb

from .fakes import FakeWorkerService, Scripted, lease, until

MakeHost = Callable[..., WorkerHost]


def numbered(n: int, **overrides: Any) -> pb.TaskLease:
    fields: dict[str, Any] = {"task_id": f"run-{n}/charge/1", "run_id": f"run-{n}", "lease_token": f"LEASE-TOKEN-{n}"}
    fields.update(overrides)
    return lease(**fields)


def handed_back(fake: FakeWorkerService) -> list[str]:
    """The task ids of the hand-backs, sorted, after checking each is the whole hand-back of the specification."""
    ids = []
    for call in fake.of("FailStep"):
        request = call.request
        if request.error_code != "TASK_NOT_STARTED":
            continue
        number = request.run_id.removeprefix("run-")
        assert request.retryable
        assert request.safe_message == "the host did not start this task"
        assert request.idempotency_key == f"failed-{request.task_id}"
        assert (request.attempt, request.generation, request.step_id) == (1, 1, "charge")
        assert request.lease_token == f"LEASE-TOKEN-{number}"
        ids.append(request.task_id)
    return sorted(ids)


def beats_of(fake: FakeWorkerService, task_id: str) -> int:
    return sum(1 for call in fake.of("Heartbeat") if call.request.task_id == task_id)


def hand_back_host(fake: FakeWorkerService, make_host: MakeHost, **extra: Any) -> WorkerHost:
    """A host whose session negotiates protocol 1.8, the first minor on which a host hands a lease back (decision
    D006-080)."""
    fake.accepted_version = "1.8"
    return make_host(protocol_version="1.8", **extra)


async def test_below_protocol_1_8_a_lease_the_host_drops_lapses_without_a_hand_back(
    fake: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    """Below protocol 1.8 the host drops the same leases and reports none of them: a server of 1.6 or 1.7 would record
    ``TASK_NOT_STARTED`` as an ordinary failure that spends the attempt, so the lease lapses as before (decision
    D006-080)."""
    fake.accepted_version = "1.7"
    fake.hold_stream = True
    fake.stream_batches = [[numbered(1), numbered(2), numbered(3)]]
    events: list[WorkerEvent] = []
    host = make_host(protocol_version="1.7", max_inflight=1, on_event=events.append)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    with caplog.at_level(logging.WARNING, logger="sagat_octopus_orchestrator"):
        await host.start()
        await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
        # Stop waits for every hand-back on its way, so a hand-back, had one been queued, has reached the fake.
        await host.stop(5)
    assert fake.of("FailStep") == [], "a dropped lease lapses below protocol 1.8, it is never handed back"
    assert [e for e in events if e.type in ("task.failed", "task.dropped")] == [], "a lapse emits no failure event"
    lapsed = sorted(record.getMessage() for record in caplog.records if "its lease lapses" in record.getMessage())
    assert lapsed == [
        f"the host drops a task it did not start; its lease lapses task=run-{n}/charge/1 reason=credit" for n in (2, 3)
    ]


async def test_from_protocol_1_8_a_lease_the_host_drops_is_handed_back(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    """From protocol 1.8 the same leases are handed back (decision D006-080)."""
    fake.hold_stream = True
    fake.stream_batches = [[numbered(1), numbered(2), numbered(3)]]
    host = hand_back_host(fake, make_host, max_inflight=1)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
    await host.stop(5)
    assert handed_back(fake) == ["run-2/charge/1", "run-3/charge/1"]


async def test_a_poll_answer_beyond_the_credit_is_handed_back(fake: FakeWorkerService, make_host: MakeHost) -> None:
    fake.ignore_max_tasks = True
    fake.leases = [numbered(1), numbered(2), numbered(3)]
    ran: list[str] = []

    async def charge(_input: Any, ctx: ActivityContext) -> int:
        ran.append(ctx.task_id)
        return 1

    host = hand_back_host(fake, make_host, transport="poll", max_inflight=1)
    host.register_activity("charge", charge)
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
    await host.stop(5)
    assert handed_back(fake) == ["run-2/charge/1", "run-3/charge/1"]
    assert ran == ["run-1/charge/1"]
    assert beats_of(fake, "run-2/charge/1") + beats_of(fake, "run-3/charge/1") == 0


async def test_a_stream_frame_beyond_the_credit_is_handed_back_and_the_started_task_is_not(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.hold_stream = True
    fake.stream_batches = [[numbered(1), numbered(2), numbered(3)]]
    release = asyncio.Event()

    async def charge(_input: Any, _ctx: ActivityContext) -> int:
        await release.wait()
        return 1

    host = hand_back_host(fake, make_host, max_inflight=1)
    host.register_activity("charge", charge)
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 2, "the two hand-backs")
    release.set()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
    await host.stop(5)
    assert handed_back(fake) == ["run-2/charge/1", "run-3/charge/1"]
    assert len(fake.of("FailStep")) == 2


async def test_the_rest_of_a_frame_over_the_input_budget_is_handed_back(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    big = '{"input":"' + "x" * (5 * 1024 * 1024) + '"}'
    fake.stream_batches = [[numbered(1, input_json=big), numbered(2, input_json=big), numbered(3)]]
    host = hand_back_host(fake, make_host, max_inflight=3)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
    await until(lambda: len(fake.of("FailStep")) == 2, "the hand-backs")
    await host.stop(5)
    assert handed_back(fake) == ["run-2/charge/1", "run-3/charge/1"]


async def test_a_stream_frame_read_while_stopping_is_handed_back_whole(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.hold_stream = True
    fake.stream_batches = [[numbered(1), numbered(2)]]
    read = asyncio.Event()
    proceed = asyncio.Event()

    async def hold(_leases: int) -> None:
        read.set()
        await proceed.wait()

    ran: list[str] = []
    host = hand_back_host(fake, make_host, max_inflight=4)
    host._frame_received = hold
    host.register_activity("charge", lambda _i, ctx: ran.append(ctx.task_id), blocking=False)
    await host.start()
    await read.wait()
    stopping = asyncio.create_task(host.stop(5))
    await until(lambda: host._stopping, "the stop to begin")
    proceed.set()
    await stopping
    # Stop returned: the hand-backs were made before the connection closed.
    assert handed_back(fake) == ["run-1/charge/1", "run-2/charge/1"]
    assert ran == []


async def test_a_poll_answer_that_arrives_while_stopping_is_handed_back_and_the_running_task_is_not(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.leases = [numbered(1)]
    started = asyncio.Event()
    release = asyncio.Event()

    async def charge(_input: Any, _ctx: ActivityContext) -> int:
        started.set()
        await release.wait()
        return 1

    hold = asyncio.Event()
    answers = 0

    async def held(leases: int) -> None:
        nonlocal answers
        if leases > 0:
            answers += 1
            if answers == 2:
                await hold.wait()

    host = hand_back_host(fake, make_host, transport="poll", max_inflight=4)
    host._frame_received = held
    host.register_activity("charge", charge)
    await host.start()
    await started.wait()
    fake.leases = [numbered(2), numbered(3)]
    await until(lambda: answers == 2, "the second answer to be held")
    stopping = asyncio.create_task(host.stop(5))
    await until(lambda: host._stopping, "the stop to begin")
    hold.set()
    await until(lambda: len(fake.of("FailStep")) == 2, "the hand-backs")
    release.set()
    await stopping
    assert handed_back(fake) == ["run-2/charge/1", "run-3/charge/1"]
    assert len(fake.of("CompleteStep")) == 1
    assert len(fake.of("FailStep")) == 2


async def test_a_duplicate_lease_is_ignored_and_never_handed_back(fake: FakeWorkerService, make_host: MakeHost) -> None:
    fake.hold_stream = True
    # The second is the running task's own lease again; the fourth repeats a lease handed back.
    fake.stream_batches = [[numbered(1), numbered(1), numbered(2), numbered(2)]]
    release = asyncio.Event()

    async def charge(_input: Any, _ctx: ActivityContext) -> int:
        await release.wait()
        return 1

    host = hand_back_host(fake, make_host, max_inflight=1)
    host.register_activity("charge", charge)
    await host.start()
    await until(lambda: len(fake.of("FailStep")) >= 1, "the hand-back")
    release.set()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
    await host.stop(5)
    assert handed_back(fake) == ["run-2/charge/1"]


async def test_sibling_iterations_that_share_a_task_id_are_not_duplicates(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.accepted_version = "1.5"
    fake.hold_stream = True

    def sibling(index: int) -> pb.TaskLease:
        identity = pb.ExecutionInstanceIdentity(
            definition_node_id="charge",
            instance_path=[pb.ScopePathSegment(scope_node_id="each", iteration_index=index)],
        )
        return numbered(1, instance=identity)

    fake.stream_batches = [[sibling(0), sibling(1)]]
    host = make_host(max_inflight=2)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 2, "both iterations")
    await host.stop(5)
    assert fake.of("FailStep") == []


async def test_a_hand_back_is_retried_with_the_same_key_and_ends_quietly_when_stale(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.ignore_max_tasks = True
    fake.leases = [numbered(1), numbered(2), numbered(3)]
    fake.failures["FailStep"] = [
        Scripted(grpc.StatusCode.UNAVAILABLE),
        Scripted(grpc.StatusCode.ABORTED, "STALE_ATTEMPT"),
    ]
    events: list[WorkerEvent] = []
    host = hand_back_host(fake, make_host, transport="poll", max_inflight=1, on_event=events.append)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(
        lambda: len([e for e in events if e.type in ("task.failed", "task.stale")]) == 2, "both hand-backs to end"
    )
    await host.stop(5)
    keys = {call.request.idempotency_key for call in fake.of("FailStep")}
    assert len(fake.of("FailStep")) == 3
    assert len(keys) == 2
    failed = [e for e in events if e.type == "task.failed"]
    assert [(e.code, e.retryable) for e in failed] == [("TASK_NOT_STARTED", True)]
    assert len([e for e in events if e.type == "task.stale"]) == 1


async def test_a_flood_of_leases_is_handed_back_eight_at_a_time(fake: FakeWorkerService, make_host: MakeHost) -> None:
    total = 1000
    fake.ignore_max_tasks = True
    fake.leases = [numbered(i) for i in range(1, total + 1)]
    release = asyncio.Event()
    inside = peak = 0

    async def gate() -> None:
        nonlocal inside, peak
        inside += 1
        peak = max(peak, inside)
        await release.wait()
        inside -= 1

    fake.fail_gate = gate
    host = hand_back_host(fake, make_host, transport="poll", max_inflight=1)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    # Eight hand-backs are held at the server; every other one waits its turn in the host.
    await until(lambda: inside == 8 and len(host._hand_back_queue) == total - 1 - 8, "eight held and the rest queued")
    assert len(host._hand_back_workers) == 8
    assert len(fake.of("FailStep")) == 8
    release.set()
    await until(lambda: len(fake.of("FailStep")) == total - 1, "every hand-back", seconds=60)
    await host.stop(5)
    assert peak <= 8
    assert len(handed_back(fake)) == total - 1


async def test_a_lease_whose_hand_back_is_on_its_way_is_a_duplicate(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.ignore_max_tasks = True
    fake.leases = [numbered(1), numbered(2), numbered(3)]
    release = asyncio.Event()

    async def gate() -> None:
        await release.wait()

    fake.fail_gate = gate
    blocked = asyncio.Event()
    ran: list[str] = []

    async def charge(_input: Any, ctx: ActivityContext) -> int:
        ran.append(ctx.task_id)
        if ctx.task_id == "run-1/charge/1":
            await blocked.wait()
        return 1

    with_leases = 0
    looked = asyncio.Event()

    async def seen(leases: int) -> None:
        nonlocal with_leases
        if leases > 0:
            with_leases += 1
        elif with_leases == 2:
            looked.set()

    host = hand_back_host(fake, make_host, transport="poll", max_inflight=2)
    host._frame_received = seen
    host.register_activity("charge", charge)
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 1, "the hand-back of lease 3 held at the server")
    await until(lambda: len(fake.of("CompleteStep")) == 1, "lease 2 done")
    fake.leases = [numbered(3)]
    await looked.wait()
    release.set()
    blocked.set()
    await until(lambda: len(fake.of("CompleteStep")) == 2, "lease 1 done")
    await host.stop(5)
    assert sorted(ran) == ["run-1/charge/1", "run-2/charge/1"]
    assert handed_back(fake) == ["run-3/charge/1"]


async def test_a_sibling_iteration_beyond_the_credit_is_handed_back_with_its_instance(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.hold_stream = True

    def sibling(index: int) -> pb.TaskLease:
        identity = pb.ExecutionInstanceIdentity(
            definition_node_id="charge",
            instance_path=[pb.ScopePathSegment(scope_node_id="each", iteration_index=index)],
        )
        return numbered(1, instance=identity)

    fake.stream_batches = [[sibling(0), sibling(1)]]
    host = hand_back_host(fake, make_host, max_inflight=1)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(
        lambda: len(fake.of("CompleteStep")) == 1 and len(fake.of("FailStep")) == 1, "the iteration and the hand-back"
    )
    await host.stop(5)
    request = fake.of("FailStep")[0].request
    assert request.error_code == "TASK_NOT_STARTED"
    assert request.instance.instance_path[0].iteration_index == 1


async def test_stop_cancels_a_receive_whose_call_begins_after_stop_did(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    """A token provider that is waited for when the poll is made: ``stop`` begins before the call exists, and the call
    is cancelled as soon as it does, instead of being waited for to its end."""
    fake.poll_delay = 10.0
    calls = 0
    waiting = asyncio.Event()
    gate = asyncio.Event()

    async def provider() -> str:
        nonlocal calls
        calls += 1
        if calls > 1:
            waiting.set()
            await gate.wait()
        return "tok"

    host = make_host(token=provider)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await waiting.wait()
    stopping = asyncio.create_task(host.stop(30))
    await until(lambda: host._stopping, "the stop to begin")
    gate.set()
    began = asyncio.get_running_loop().time()
    await stopping
    assert asyncio.get_running_loop().time() - began < 2, "the poll made after the stop began was cancelled"


async def test_a_hand_back_still_on_its_way_when_the_grace_ends_is_dropped_and_its_lease_lapses(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    total = 12
    fake.ignore_max_tasks = True
    fake.leases = [numbered(i) for i in range(1, total + 1)]
    never = asyncio.Event()

    async def gate() -> None:
        # The server never answers a hand-back before the stop has ended: eight are held, the rest queued.
        await never.wait()

    fake.fail_gate = gate
    events: list[WorkerEvent] = []
    host = hand_back_host(fake, make_host, transport="poll", max_inflight=1, on_event=events.append)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await until(
        lambda: len(fake.of("FailStep")) == 8 and len(host._hand_back_queue) == total - 1 - 8,
        "eight held and the rest queued",
    )
    began = asyncio.get_running_loop().time()
    await host.stop(0.2)
    assert asyncio.get_running_loop().time() - began < 0.2 + 2.0 + 1.0, "the grace and its two seconds bound the stop"
    # Every lease that was not reported ends as dropped, once, and lapses.
    await until(lambda: len([e for e in events if e.type == "task.dropped"]) == total - 1, "every hand-back dropped")
    dropped = sorted(e.task_id or "" for e in events if e.type == "task.dropped")
    assert dropped == sorted(f"run-{i}/charge/1" for i in range(2, total + 1))
    assert [e for e in events if e.type in ("task.failed", "task.stale")] == []
    assert len(fake.of("FailStep")) == 8, "the queued hand-backs are not sent on a closed connection"


async def test_a_frame_after_the_credit_is_used_up_is_read_and_handed_back(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.hold_stream = True
    fake.stream_batches = [[numbered(1)], [numbered(2), numbered(3)]]
    release = asyncio.Event()

    async def charge(_input: Any, _ctx: ActivityContext) -> int:
        await release.wait()
        return 1

    host = hand_back_host(fake, make_host, max_inflight=1)
    host.register_activity("charge", charge)
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 2, "the two hand-backs")
    release.set()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the kept task")
    await host.stop(5)
    assert handed_back(fake) == ["run-2/charge/1", "run-3/charge/1"]


async def test_a_hand_back_queued_as_the_last_worker_leaves_is_made_once(
    fake: FakeWorkerService, make_host: MakeHost, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The race of a worker that has seen the queue empty: a hand-back queued in that moment is neither stranded nor
    made twice. With one worker, the report of the first lease queues the second as its very last act, with
    ``call_soon``: that callback runs after the worker has looked at the queue (empty) and before the event loop runs
    the callbacks of the finished worker task, which is exactly the moment of the race."""
    from sagat_octopus_orchestrator.worker import host as host_module

    monkeypatch.setattr(host_module, "HAND_BACK_CONCURRENCY", 1)
    host = hand_back_host(fake, make_host, transport="poll")
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    reported: list[str] = []
    loop = asyncio.get_running_loop()

    async def report(task: Any, code: str, message: str, retryable: bool, started: float) -> None:
        reported.append(task.lease.task_id)
        if task.lease.task_id == "run-1/charge/1":
            loop.call_soon(host._hand_back, numbered(2), "run-2/charge/1", "credit")

    monkeypatch.setattr(host, "_fail", report)
    host._hand_back(numbered(1), "run-1/charge/1", "credit")
    await until(lambda: reported == ["run-1/charge/1", "run-2/charge/1"], "both hand-backs")
    await until(lambda: not host._hand_back_workers and not host._handing_back, "the worker to leave")
    assert reported == ["run-1/charge/1", "run-2/charge/1"]
    await host.stop(1)


async def test_a_handler_cannot_say_task_not_started(fake: FakeWorkerService, make_host: MakeHost) -> None:
    """Only the host may say that no handler ran: a handler's ``TASK_NOT_STARTED`` is an ordinary failure, with its
    own message and ``retryable``, whatever the kind of the handler."""
    fake.accepted_version = "1.2"
    fake.leases = [
        lease(task_id="run-a/forward/1", run_id="run-a", activity_name="forward", step_id="forward"),
        lease(task_id="run-b/permanent/1", run_id="run-b", activity_name="permanent", step_id="permanent"),
        lease(task_id="run-c/undo/1", run_id="run-c", activity_name="undo", step_id="undo", task_kind="COMPENSATION"),
        lease(
            task_id="run-d/check/1", run_id="run-d", activity_name="check", step_id="check", task_kind="RECONCILIATION"
        ),
    ]

    def failing(retryable: bool) -> Callable[[Any, ActivityContext], Any]:
        def handler(_input: Any, _ctx: ActivityContext) -> Any:
            raise ActivityFailure("TASK_NOT_STARTED", "custom", retryable)

        return handler

    host = make_host(max_inflight=4)
    host.register_activity("forward", failing(True), blocking=False)
    host.register_activity("permanent", failing(False), blocking=False)
    host.register_compensation("undo", failing(True), blocking=False)
    host.register_reconciler("check", failing(True), blocking=False)
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 4, "the four failures")
    await host.stop(5)
    expected = {"forward": True, "permanent": False, "undo": True, "check": True}
    for call in fake.of("FailStep"):
        request = call.request
        assert (request.error_code, request.safe_message, request.retryable) == (
            "WORKER_ERROR",
            "custom",
            expected[request.step_id],
        ), request.step_id
