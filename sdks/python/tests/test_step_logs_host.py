"""Step-log capture by the worker host (design 041, ``sdks/SPEC.md`` section 6.9): numbering, batches, the answers of
``AppendStepLogs``, the carry on the report and the wait before it, against the in-process worker service."""

from __future__ import annotations

import asyncio
import logging
import threading
from collections.abc import Callable
from typing import Any

import grpc
import pytest

from sagat_octopus_orchestrator import (
    PROTOCOL_VERSION,
    TRACE,
    ActivityContext,
    ActivityFailure,
    SagatError,
    WorkerEvent,
    WorkerHost,
)
from sagat_octopus_orchestrator._proto import worker_pb2 as pb
from sagat_octopus_orchestrator.worker import _step_logs as step_logs

from .fakes import FakeWorkerService, Held, Scripted, lease, reached, until, verdict

MakeHost = Callable[..., WorkerHost]


@pytest.fixture
def capturing(fake: FakeWorkerService) -> FakeWorkerService:
    """The worker service at protocol 1.7 with capture on (``StepLogPolicy.enabled``)."""
    fake.accepted_version = "1.7"
    fake.step_logs = True
    return fake


_count = 0


def one_lease(**overrides: Any) -> pb.TaskLease:
    """A lease of the activity ``work`` with ids of its own."""
    global _count
    _count += 1
    fields: dict[str, Any] = {
        "task_id": f"run-{_count}/work/1",
        "run_id": f"run-{_count}",
        "step_id": f"step-{_count}",
        "activity_name": "work",
        "input_json": '{"input":{},"steps":{}}',
        "instance": pb.ExecutionInstanceIdentity(definition_node_id="work"),
    }
    fields.update(overrides)
    return lease(**fields)


def report(fake: FakeWorkerService) -> Any:
    """The first report the service received."""
    return (fake.of("CompleteStep") + fake.of("FailStep"))[0].request


def sequences(records: Any) -> list[int]:
    return [record.sequence for record in records]


def messages(records: Any) -> list[str]:
    return [record.message for record in records]


def span(first: int, last: int) -> list[int]:
    return list(range(first, last + 1))


async def run_one(
    fake: FakeWorkerService,
    make_host: MakeHost,
    count: int,
    end: Callable[[ActivityContext], Any] | None = None,
    **config: Any,
) -> Any:
    """Runs one activity (a plain function, so in a thread) that logs ``line 1`` to ``line <count>`` and returns; the
    report it made."""
    host = make_host(transport="poll", max_inflight=1, **config)

    def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, count + 1):
            ctx.log.info(f"line {n}")
        return end(ctx) if end is not None else {"ok": True}

    host.register_activity("work", work)
    await host.start()
    fake.leases = [one_lease()]
    await until(lambda: len(fake.of("CompleteStep")) + len(fake.of("FailStep")) > 0, "the report", 10.0)
    return report(fake)


def drops_of(events: list[WorkerEvent]) -> list[tuple[str | None, int | None]]:
    return sorted((event.reason, event.records) for event in events if event.type == "steplogs.dropped")


# ------------------------------------------------------------------------------------------------ the option


def test_the_sdk_offers_protocol_1_7_and_a_trace_level() -> None:
    assert PROTOCOL_VERSION == "1.7"
    assert TRACE == 5
    assert logging.getLevelName(TRACE) == "TRACE"


@pytest.mark.parametrize("level", ["INFO", None, True, -1, 2.5])
def test_a_capture_level_that_is_not_a_level_is_refused_when_the_host_is_made(
    fake: FakeWorkerService, make_host: MakeHost, level: Any
) -> None:
    with pytest.raises(SagatError) as refused:
        make_host(step_logs_level=level)
    assert refused.value.code == "VALIDATION_FAILED"
    with pytest.raises(SagatError):
        make_host(step_logs_capture="yes")


# ------------------------------------------------------------------------------------------------ a short step


async def test_a_short_step_at_1_7_carries_its_lines_on_its_report_and_makes_no_append_call(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    request = await run_one(capturing, make_host, 3)
    assert capturing.of("AppendStepLogs") == []
    assert request.HasField("step_logs")
    assert messages(request.step_logs.records) == ["line 1", "line 2", "line 3"]
    assert sequences(request.step_logs.records) == [1, 2, 3]
    assert request.step_logs.highest_sequence == 3
    assert request.step_logs.records[0].level == pb.STEP_LOG_LEVEL_INFO
    assert request.step_logs.records[0].worker_time_epoch_ms > 0


@pytest.mark.parametrize(
    ("accepted", "policy", "option"),
    [("1.5", True, True), ("1.7", False, True), ("1.7", None, True), ("1.7", True, False)],
    ids=["below-1.7", "policy-off", "no-policy", "option-off"],
)
async def test_nothing_is_captured_below_1_7_with_the_policy_off_without_a_policy_or_with_the_option_off(
    fake: FakeWorkerService, make_host: MakeHost, accepted: str, policy: bool | None, option: bool
) -> None:
    fake.accepted_version = accepted
    fake.step_logs = policy
    request = await run_one(fake, make_host, 300, step_logs_capture=option)
    assert not request.HasField("step_logs")
    assert fake.of("AppendStepLogs") == []


# ------------------------------------------------------------------------------------------------ batches


async def test_a_long_step_sends_the_batch_its_256th_call_cut_waits_for_its_answer_and_carries_the_rest(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    held = Held()
    capturing.append_script = [held]
    reported = asyncio.ensure_future(run_one(capturing, make_host, 260))
    await until(lambda: len(capturing.of("AppendStepLogs")) == 1, "the batch")
    [batch] = capturing.of("AppendStepLogs")
    assert sequences(batch.request.records) == span(1, 256)
    assert (batch.request.worker_id, batch.request.session_token) == ("w-1", "session-1")
    assert (batch.request.task_id, batch.request.run_id, batch.request.step_id) == (
        f"run-{_count}/work/1",
        f"run-{_count}",
        f"step-{_count}",
    )
    assert (batch.request.attempt, batch.request.generation, batch.request.lease_token) == (1, 1, "lt")
    assert batch.request.instance.definition_node_id == "work"
    # The report waits for the batch in flight: a bounded wait for its absence, then the order of the two calls.
    assert not await reached(lambda: len(capturing.of("CompleteStep")) > 0, 0.3)
    held.release.set()
    request = await reported
    assert sequences(request.step_logs.records) == span(257, 260)
    assert request.step_logs.highest_sequence == 260
    assert capturing.of("CompleteStep")[0].at >= capturing.append_answered[0]


async def test_an_answer_held_past_2_seconds_is_given_up_and_the_report_carries_the_newest_records(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    held = Held(then=Scripted(grpc.StatusCode.UNAVAILABLE))
    capturing.append_script = [held]
    try:
        request = await run_one(capturing, make_host, 260, on_event=events.append)
        assert capturing.append_answered == [], "the report went while the batch was still held"
        assert capturing.of("CompleteStep")[0].at - capturing.of("AppendStepLogs")[0].at >= 2.0
        assert sequences(request.step_logs.records) == span(257, 260)
        assert request.step_logs.highest_sequence == 260
        assert drops_of(events) == [("report_deadline", 256)], "the batch given up is counted once"
    finally:
        held.release.set()
    # The late answer is ignored: a batch given up at the end of the wait is not sent again (its retry would come
    # 200 ms after the failure).
    await until(lambda: len(capturing.append_answered) == 1, "the late answer")
    assert not await reached(lambda: len(capturing.of("AppendStepLogs")) > 1, 1.0)
    assert drops_of(events) == [("report_deadline", 256)]


async def test_stop_leaves_an_attempt_waiting_before_its_report_to_it_and_its_carry_holds_the_tail(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    held = Held()
    capturing.append_script = [held]
    host = make_host(transport="poll", max_inflight=1, on_event=events.append)
    returned = threading.Event()

    def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, 261):
            ctx.log.info(f"line {n}")
        returned.set()
        return {"ok": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease()]
    await until(lambda: returned.is_set() and len(capturing.of("AppendStepLogs")) == 1, "the handler's end")
    try:
        # The attempt waits for its held batch before its report: stop flushes nothing of it and lets the wait end.
        await host.stop(0.5)
    finally:
        held.release.set()
    assert len(capturing.of("AppendStepLogs")) == 1
    request = capturing.of("CompleteStep")[0].request
    assert sequences(request.step_logs.records) == span(257, 260)
    assert drops_of(events) == [("report_deadline", 256)]


async def test_a_real_cancelled_status_is_retried_as_any_status_not_listed(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    capturing.append_script = [Scripted(grpc.StatusCode.CANCELLED)]
    request = await run_one(capturing, make_host, 260, on_event=events.append)
    appends = capturing.of("AppendStepLogs")
    assert len(appends) == 2
    assert appends[1].request.records == appends[0].request.records
    assert sequences(request.step_logs.records) == span(257, 260)
    assert drops_of(events) == []


async def test_a_batch_in_its_back_off_is_not_sent_again_once_the_session_stopped_capture(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    # Attempt 1 is in its 1-second back-off after two UNAVAILABLE when attempt 2's batch is answered STOP_CAPTURE.
    capturing.append_script = [
        Scripted(grpc.StatusCode.UNAVAILABLE),
        Scripted(grpc.StatusCode.UNAVAILABLE),
        verdict("STEP_LOG_VERDICT_STOP_CAPTURE", "STEP_LOG_STOP_REASON_CAPTURE_OFF"),
    ]
    events: list[WorkerEvent] = []
    host = make_host(transport="poll", max_inflight=2, on_event=events.append)
    second_may_log = asyncio.Event()

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        if ctx.attempt == 2:
            await second_may_log.wait()
        for n in range(1, 257):
            ctx.log.info(f"line {n}")
        if ctx.attempt == 1:
            await until(lambda: len(capturing.of("AppendStepLogs")) == 2, "the first retry")
            second_may_log.set()
            await until(lambda: len(capturing.append_answered) == 3, "the stopping answer")
        return {"ok": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease(), one_lease(attempt=2)]
    await until(lambda: len(capturing.of("CompleteStep")) == 2, "both reports")
    assert len(capturing.of("AppendStepLogs")) == 3, "no second retry after the session stopped"
    for call in capturing.of("CompleteStep"):
        assert not call.request.HasField("step_logs")
    # The batch given up is counted once; the one answered STOP_CAPTURE was answered, not dropped by the SDK.
    assert drops_of(events) == [("send_failed", 256)]


async def test_a_batch_holds_at_most_500_records(capturing: FakeWorkerService, make_host: MakeHost) -> None:
    held = Held()
    capturing.append_script = [held]
    host = make_host(transport="poll", max_inflight=1)

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, 901):
            ctx.log.info(f"line {n}")
        # 644 records are buffered behind the batch in flight; its answer cuts the next batch from them.
        held.release.set()
        await until(lambda: len(capturing.of("AppendStepLogs")) == 2, "the second batch")
        return {"ok": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease()]
    await until(lambda: len(capturing.of("CompleteStep")) == 1, "the report")
    appends = capturing.of("AppendStepLogs")
    assert [(call.request.records[0].sequence, len(call.request.records)) for call in appends] == [
        (1, 256),
        (257, 500),
        (757, 80),
    ]
    assert sequences(report(capturing).step_logs.records) == span(837, 900)


async def test_a_batch_holds_at_most_192_kib_and_the_attempt_buffer_stops_at_its_bound(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    host = make_host(transport="poll", max_inflight=1, on_event=events.append)
    big = "m" * 8000

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        # Lines of 8 KB: the attempt buffer (256 KiB) is full long before 256 records, so the tick of a second sends.
        for n in range(1, 101):
            ctx.log.info(f"{n} {big}")
        await until(lambda: len(capturing.of("AppendStepLogs")) == 1, "the batch of the tick")
        return {"ok": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease()]
    await until(lambda: len(capturing.of("CompleteStep")) == 1, "the report")
    batch = capturing.of("AppendStepLogs")[0].request
    size = batch.ByteSize()
    assert size <= 192 * 1024
    assert size > 192 * 1024 - 8100, "the batch is filled up to its bound"
    assert ("buffer_full", 100 - 33) in drops_of(events), drops_of(events)
    request = report(capturing)
    assert request.step_logs.highest_sequence == 100


# ------------------------------------------------------------------------------------------------ the answers


async def test_final_only_drops_what_is_buffered_and_later_calls_and_the_report_carries_the_mark_alone(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.append_script = [verdict("STEP_LOG_VERDICT_FINAL_ONLY")]
    request = await run_one(capturing, make_host, 300)
    assert len(capturing.of("AppendStepLogs")) == 1
    assert request.HasField("step_logs")
    assert list(request.step_logs.records) == []
    assert request.step_logs.highest_sequence == 300


async def test_stop_attempt_sends_no_carry(capturing: FakeWorkerService, make_host: MakeHost) -> None:
    capturing.append_script = [verdict("STEP_LOG_VERDICT_STOP_ATTEMPT", "STEP_LOG_STOP_REASON_LEASE")]
    request = await run_one(capturing, make_host, 300)
    assert len(capturing.of("AppendStepLogs")) == 1
    assert not request.HasField("step_logs")


async def test_stop_capture_stops_the_session_until_it_registers_again(
    capturing: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    capturing.append_script = [verdict("STEP_LOG_VERDICT_STOP_CAPTURE", "STEP_LOG_STOP_REASON_CAPTURE_OFF")]
    host = make_host(transport="poll", max_inflight=1)

    def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, 301):
            ctx.log.info(f"line {n}")
        return {"ok": True}

    host.register_activity("work", work)
    with caplog.at_level(logging.WARNING, logger="sagat_octopus_orchestrator"):
        await host.start()
        capturing.leases = [one_lease(), one_lease()]
        await until(lambda: len(capturing.of("CompleteStep")) == 2, "both reports")
    assert len(capturing.of("AppendStepLogs")) == 1, "the second attempt of the session sends nothing"
    for call in capturing.of("CompleteStep"):
        assert not call.request.HasField("step_logs")
    # One local line per process, whatever answer stopped capture: written by this test or by one before it.
    assert step_logs.SESSION_STOPPED_LINE in step_logs._WARNED
    assert caplog.text.count(step_logs.SESSION_STOPPED_LINE) <= 1


async def test_stop_capture_for_the_session_is_answered_by_sending_once_more_with_the_token_now_held(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    held = Held(then=verdict("STEP_LOG_VERDICT_STOP_CAPTURE", "STEP_LOG_STOP_REASON_SESSION"))
    capturing.append_script = [held]
    host = make_host(transport="poll", max_inflight=1)

    def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, 261):
            ctx.log.info(f"line {n}")
        return {"ok": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease()]
    await until(lambda: len(capturing.of("AppendStepLogs")) == 1, "the batch")
    # The batch is answered once the host has registered again: the session it sent is not the one it holds.
    capturing.session_token = "session-2"
    await host._register()
    held.release.set()
    await until(lambda: len(capturing.of("CompleteStep")) == 1, "the report")
    appends = capturing.of("AppendStepLogs")
    assert [call.request.session_token for call in appends] == ["session-1", "session-2"]
    assert sequences(appends[1].request.records) == span(1, 256)
    assert sequences(report(capturing).step_logs.records) == span(257, 260)


async def test_stop_capture_for_the_session_with_the_token_still_held_stops_and_sends_no_carry(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.append_script = [verdict("STEP_LOG_VERDICT_STOP_CAPTURE", "STEP_LOG_STOP_REASON_SESSION")]
    request = await run_one(capturing, make_host, 260)
    assert len(capturing.of("AppendStepLogs")) == 1
    assert not request.HasField("step_logs")


async def test_retry_later_and_resource_exhausted_send_the_same_batch_again_without_counting_a_retry(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.append_script = [
        verdict("STEP_LOG_VERDICT_RETRY_LATER", retry_after_ms=60),
        Scripted(grpc.StatusCode.RESOURCE_EXHAUSTED, "RATE_LIMITED", retry_after_ms=60),
        Scripted(grpc.StatusCode.UNAVAILABLE),
        Scripted(grpc.StatusCode.UNAVAILABLE),
    ]
    request = await run_one(capturing, make_host, 260)
    appends = capturing.of("AppendStepLogs")
    assert len(appends) == 5, "two waits, two retries, then the answer"
    for call in appends:
        assert call.request.records == appends[0].request.records
    assert appends[1].at - appends[0].at >= 0.055
    assert appends[2].at - appends[1].at >= 0.055
    assert sequences(request.step_logs.records) == span(257, 260)


async def test_unavailable_three_times_drops_the_batch_after_its_two_retries_and_the_carry_still_goes(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    capturing.append_script = [Scripted(grpc.StatusCode.UNAVAILABLE)] * 3
    request = await run_one(capturing, make_host, 260, on_event=events.append)
    appends = capturing.of("AppendStepLogs")
    assert len(appends) == 3
    assert appends[1].at - appends[0].at >= 0.16, "the first retry after 200 ms, less 20%"
    assert appends[2].at - appends[1].at >= 0.8, "the second after 1 s, less 20%"
    assert sequences(request.step_logs.records) == span(257, 260)
    assert drops_of(events) == [("send_failed", 256)]
    assert all(event.task_id == f"run-{_count}/work/1" for event in events if event.type == "steplogs.dropped")


async def test_invalid_argument_drops_the_batch_at_once_and_goes_on(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.append_script = [Scripted(grpc.StatusCode.INVALID_ARGUMENT)]
    request = await run_one(capturing, make_host, 260)
    assert len(capturing.of("AppendStepLogs")) == 1
    assert sequences(request.step_logs.records) == span(257, 260)


async def test_unauthenticated_stops_the_session_as_capture_off(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.append_script = [Scripted(grpc.StatusCode.UNAUTHENTICATED)]
    request = await run_one(capturing, make_host, 260)
    assert len(capturing.of("AppendStepLogs")) == 1
    assert not request.HasField("step_logs")


async def test_unimplemented_stops_the_batches_of_the_attempt_and_the_report_carries_the_newest_64(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    events: list[WorkerEvent] = []
    capturing.append_script = [Scripted(grpc.StatusCode.UNIMPLEMENTED)]
    request = await run_one(capturing, make_host, 400, on_event=events.append)
    assert len(capturing.of("AppendStepLogs")) == 1
    assert sequences(request.step_logs.records) == span(337, 400)
    assert request.step_logs.highest_sequence == 400
    assert drops_of(events) == [("report_deadline", 80), ("send_failed", 256)]


async def test_after_unimplemented_no_call_cuts_another_batch_however_many_records_it_buffers(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.append_script = [Scripted(grpc.StatusCode.UNIMPLEMENTED)]

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, 257):
            ctx.log.info(f"line {n}")
        capture = ctx.log._capture
        assert capture is not None
        await until(lambda: capture._batches_stopped, "the refused batch applied")
        for n in range(257, 2257):
            ctx.log.info(f"line {n}")
        return {"ok": True}

    events: list[WorkerEvent] = []
    host = make_host(transport="poll", max_inflight=1, on_event=events.append)
    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease()]
    await until(lambda: len(capturing.of("CompleteStep")) == 1, "the report")
    assert len(capturing.of("AppendStepLogs")) == 1
    carried = report(capturing).step_logs
    assert carried.highest_sequence == 2256
    # From UNIMPLEMENTED on, only the newest records that fit one carry are kept: never the attempt buffer's bound.
    assert sequences(carried.records) == span(2193, 2256)
    assert drops_of(events) == [("report_deadline", 2000 - 64), ("send_failed", 256)]


# ------------------------------------------------------------------------------------------------ the report


async def test_a_lost_answer_to_a_carrying_report_sends_the_same_carry_again_and_a_failure_carries_too(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.failures["CompleteStep"] = [Scripted(grpc.StatusCode.UNAVAILABLE, "UNAVAILABLE")]
    await run_one(capturing, make_host, 3)
    await until(lambda: len(capturing.of("CompleteStep")) == 2, "the report sent again")
    first, second = capturing.of("CompleteStep")
    assert first.request.HasField("step_logs")
    assert second.request.step_logs == first.request.step_logs


async def test_a_failure_report_carries_the_lines_of_its_attempt(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    def broken(_ctx: ActivityContext) -> Any:
        raise ActivityFailure.permanent("BROKEN", "broken")

    request = await run_one(capturing, make_host, 2, end=broken)
    assert capturing.of("FailStep")[0].request.error_code == "BROKEN"
    assert messages(request.step_logs.records) == ["line 1", "line 2"]


async def test_a_report_made_before_any_handler_ran_carries_nothing(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(transport="poll", max_inflight=1)
    host.register_activity("work", lambda _i, _c: {"ok": True})
    await host.start()
    capturing.leases = [one_lease(activity_name="nobody")]
    await until(lambda: len(capturing.of("FailStep")) == 1, "the refusal")
    assert capturing.of("FailStep")[0].request.error_code == "NO_HANDLER"
    assert not capturing.of("FailStep")[0].request.HasField("step_logs")


async def test_the_host_never_logs_a_batch_or_a_report_that_holds_a_carry(
    capturing: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    capturing.failures["CompleteStep"] = [Scripted(grpc.StatusCode.UNAVAILABLE, "UNAVAILABLE")]
    capturing.append_script = [Scripted(grpc.StatusCode.UNAVAILABLE)] * 3
    with caplog.at_level(logging.DEBUG, logger="sagat_octopus_orchestrator"):
        await run_one(capturing, make_host, 260)
        await until(lambda: len(capturing.of("CompleteStep")) == 2, "the report sent again")
    host_lines = [record for record in caplog.records if not record.getMessage().startswith("line ")]
    assert host_lines, "the host logged its own lines"
    for record in host_lines:
        text = record.getMessage() + repr(getattr(record, "sagat", ""))
        assert "line 1" not in text and "highest_sequence" not in text and "records" not in text, text


# ------------------------------------------------------------------------------------------------ the logger


async def test_the_capture_level_is_the_option_below_it_a_call_takes_no_sequence_and_the_host_level_does_not_matter(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    host_lines: list[logging.LogRecord] = []

    class Keep(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            host_lines.append(record)

    logger = logging.getLogger("step-logs-host-level")
    logger.setLevel(logging.ERROR)
    logger.propagate = False
    logger.addHandler(Keep())

    def end(ctx: ActivityContext) -> Any:
        ctx.log.trace("trace line")
        ctx.log.debug("debug line")
        ctx.log.info("info line", extra={"k": 1})
        ctx.log.warning("warn line")
        ctx.log.error("error line", exc_info=RuntimeError("the payment provider refused"))
        return {"ok": True}

    request = await run_one(capturing, make_host, 0, end, step_logs_level=logging.DEBUG, logger=logger)
    records = request.step_logs.records
    assert messages(records) == ["debug line", "info line", "warn line", "error line"]
    assert sequences(records) == [1, 2, 3, 4]
    assert [record.level for record in records] == [
        pb.STEP_LOG_LEVEL_DEBUG,
        pb.STEP_LOG_LEVEL_INFO,
        pb.STEP_LOG_LEVEL_WARN,
        pb.STEP_LOG_LEVEL_ERROR,
    ]
    assert records[3].exception == "RuntimeError: the payment provider refused"
    assert [(field.key, field.int_value) for field in records[1].fields] == [("k", 1)]
    assert [line.getMessage().split(" task=")[0] for line in host_lines] == ["error line"], (
        "the host still logs what it logged"
    )


async def test_a_call_is_captured_as_python_formats_it_with_its_extra_its_exception_and_its_stack(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    def end(ctx: ActivityContext) -> Any:
        ctx.log.info("charging %s of %d", "EUR", 1999, extra={"order": "o-1", "amount": 1999})
        try:
            raise ValueError("refused")
        except ValueError:
            ctx.log.exception("the provider refused")
        ctx.log.warning("look here", stack_info=True)
        ctx.log.info("%(a)s-%(b)s", {"a": 1, "b": 2})
        ctx.log.critical("critical line")
        ctx.log.log(25, "between")
        ctx.log.log(TRACE, "trace by number")
        ctx.log.info("bad %d", "format")
        return {"ok": True}

    request = await run_one(capturing, make_host, 0, end)
    records = request.step_logs.records
    assert messages(records) == [
        "charging EUR of 1999",
        "the provider refused",
        "look here",
        "1-2",
        "critical line",
        "between",
        "bad %d",
    ]
    assert [(field.key, field.WhichOneof("value")) for field in records[0].fields] == [
        ("amount", "int_value"),
        ("order", "string_value"),
    ]
    assert records[1].level == pb.STEP_LOG_LEVEL_ERROR
    assert records[1].exception.startswith("Traceback (most recent call last):\n")
    assert records[1].exception.endswith("ValueError: refused")
    assert records[2].exception.startswith("Stack (most recent call last):\n")
    assert "in end" in records[2].exception
    assert records[4].level == pb.STEP_LOG_LEVEL_ERROR
    assert records[5].level == pb.STEP_LOG_LEVEL_INFO
    for record in records:
        assert all(field.key not in ("task", "run", "node", "attempt") for field in record.fields)


async def test_the_host_logging_keeps_its_caller_and_its_extra_as_before(
    capturing: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    def end(ctx: ActivityContext) -> Any:
        ctx.log.info("by info")
        ctx.log.log(logging.INFO, "by log")
        ctx.log.trace("by trace")
        ctx.log.info("with extra", extra={"order": "o-1"})
        return {"ok": True}

    with caplog.at_level(TRACE, logger="sagat_octopus_orchestrator"):
        await run_one(capturing, make_host, 0, end)
    lines = {record.getMessage().split(" task=")[0]: record for record in caplog.records}
    for text in ("by info", "by log", "by trace", "with extra"):
        assert lines[text].pathname == __file__, (text, lines[text].pathname)
        assert lines[text].funcName == "end", (text, lines[text].funcName)
    assert lines["by trace"].levelno == TRACE
    assert lines["with extra"].sagat["task"] == f"run-{_count}/work/1"  # type: ignore[attr-defined]
    assert not hasattr(lines["with extra"], "order"), "the host's line keeps its extra as before"


async def test_a_line_is_redacted_with_every_secret_the_process_resolved_also_after_the_host_started_again(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    capturing.secrets = [pb.ResolvedSecret(name="key", value=b"canary-value-9Kx2", version=1)]
    host = make_host(transport="poll", max_inflight=1)

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        if ctx.attempt == 1:
            await ctx.secrets()
            return {"first": True}
        try:
            raise RuntimeError("failure canary-value-9Kx2")
        except RuntimeError:
            ctx.log.error("%s", "canary-value-9Kx2", extra={"canary-value-9Kx2": "canary-value-9Kx2"}, exc_info=True)
        return {"second": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease(), one_lease(attempt=2)]
    await until(lambda: len(capturing.of("CompleteStep")) == 2, "both reports")
    [record] = capturing.of("CompleteStep")[1].request.step_logs.records
    assert record.message == "[REDACTED]"
    assert [(field.key, field.string_value) for field in record.fields] == [("[REDACTED]", "[REDACTED]")]
    assert record.exception.endswith("RuntimeError: failure [REDACTED]")
    assert all("canary-value-9Kx2" not in str(call.request) for call in capturing.of("CompleteStep"))
    # The same host, stopped and started again in the process, still redacts what an earlier attempt resolved.
    await host.stop(1.0)
    capturing.calls.clear()
    await host.start()
    capturing.leases = [one_lease(attempt=2)]
    await until(lambda: len(capturing.of("CompleteStep")) == 1, "the report after the restart")
    assert capturing.of("CompleteStep")[0].request.step_logs.records[0].message == "[REDACTED]"


async def test_a_logger_call_never_raises_into_the_activity_whatever_it_is_handed(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    class Hostile:
        def __str__(self) -> str:
            raise RuntimeError("str")

        __repr__ = __str__

    class HostileMapping(dict[str, Any]):
        def items(self) -> Any:
            raise RuntimeError("items")

        def keys(self) -> Any:
            raise RuntimeError("keys")

    seen: list[ActivityContext] = []

    class LogsAgain:
        def __str__(self) -> str:
            seen[0].log.info("logged while formatting")
            return "formatted"

    circular: dict[str, Any] = {}
    circular["self"] = circular
    calls = 0

    def end(ctx: ActivityContext) -> Any:
        nonlocal calls
        seen.append(ctx)
        ctx.log.info("circular", extra={"circular": circular, "hostile": Hostile()})
        ctx.log.info(Hostile())
        ctx.log.info("%s", Hostile())
        ctx.log.info("mapping", extra=HostileMapping(a=1))
        ctx.log.info("again %s", LogsAgain())
        ctx.log.info("level", exc_info=(1, 2, 3))  # type: ignore[arg-type]
        ctx.log.log("INFO", "a level that is not a number")  # type: ignore[arg-type]
        calls = 1
        return {"ok": True}

    request = await run_one(capturing, make_host, 0, end, logger=logging.getLogger("step-logs-hostile"))
    assert calls == 1
    assert len(capturing.of("CompleteStep")) == 1, "the activity completed: no call raised into it"
    assert capturing.of("FailStep") == []
    # Six captured calls, and the line logged while one of them was formatted (once for capture, once more when the
    # host's own logging formats the message).
    assert request.step_logs.highest_sequence == 7
    assert sequences(request.step_logs.records) == sorted(sequences(request.step_logs.records))
    assert "again formatted" in messages(request.step_logs.records)
    assert "logged while formatting" in messages(request.step_logs.records)


# ------------------------------------------------------------------------------------------------ shutdown


async def test_stop_sends_the_lines_of_an_attempt_still_running_once_and_its_report_carries_the_rest(
    capturing: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(transport="poll", max_inflight=1)
    logged = threading.Event()

    async def work(_input: Any, ctx: ActivityContext) -> Any:
        for n in range(1, 11):
            ctx.log.info(f"line {n}")
        logged.set()
        await ctx.cancellation.wait()
        ctx.log.info("after the stop")  # the handler still runs: captured, and carried by its report
        return {"ok": True}

    host.register_activity("work", work)
    await host.start()
    capturing.leases = [one_lease()]
    await until(logged.is_set, "the lines")
    await host.stop(0.0)
    [batch] = capturing.of("AppendStepLogs")
    assert sequences(batch.request.records) == span(1, 10)
    request = capturing.of("CompleteStep")[0].request
    assert messages(request.step_logs.records) == ["after the stop"]
    assert request.step_logs.highest_sequence == 11
