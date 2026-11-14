"""The client surface of feature 018 against a recording server: what each call sends, what it keeps across a retry,
what it refuses before sending, and how it reads an answer that is newer than the SDK."""

from __future__ import annotations

import copy
import pickle
from collections.abc import Awaitable, Callable
from datetime import datetime, timedelta, timezone
from typing import Any

import pytest

from sagat_octopus_orchestrator import (
    BatchFilter,
    BatchRequest,
    BatchSelector,
    ClientConfig,
    RunResetFrom,
    RunSchedule,
    SagatClient,
    SagatError,
    ScheduleAction,
    ScheduleCalendarSpec,
    SchedulePolicies,
    ScheduleRange,
    ScheduleSpec,
    ScheduleWrite,
)
from sagat_octopus_orchestrator.sync import SyncSagatClient

from .conftest import FAST
from .fakes import FakeHttpServer, Reply, Seen, reply
from .test_client import refusal, run_body

MakeClient = Callable[..., SagatClient]

NS = "0190aaaa-0000-7000-8000-00000000aaaa"
WF = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee"
SCHEDULE = "0190aaaa-0000-7000-8000-0000000000a1"
RUN = "0190aaaa-0000-7000-8000-000000000001"
BATCH = "0190aaaa-0000-7000-8000-0000000000b1"


def schedule_body(**overrides: Any) -> dict[str, Any]:
    return {
        "scheduleId": SCHEDULE,
        "name": "nightly",
        "origin": "API",
        "state": "ACTIVE",
        "revision": 2,
        "createdAt": "2026-10-10T10:00:00Z",
        "action": {"workflowId": WF, "versionPolicy": "ACTIVE", "inputDigest": "a" * 64},
        "note": None,
        "remainingActions": None,
        "nextActionAt": "2099-01-01T09:00:00Z",
        "updatedAt": "2026-10-10T10:00:00Z",
        **overrides,
    }


def action_body(**overrides: Any) -> dict[str, Any]:
    return {
        "nominalTime": "2026-10-10T10:00:00Z",
        "actualTime": "2026-10-10T10:00:01Z",
        "kind": "TRIGGERED",
        "origin": "MANUAL",
        "revision": 2,
        "outcome": "STARTED",
        "reason": None,
        "runId": RUN,
        "backfillId": None,
        "decidedAt": "2026-10-10T10:00:01Z",
        **overrides,
    }


def only(http: FakeHttpServer) -> Seen:
    assert len(http.seen) == 1, "exactly one request"
    return http.seen[0]


async def refused(call: Awaitable[Any]) -> SagatError:
    with pytest.raises(SagatError) as caught:
        await call
    return caught.value


def simple_write(**overrides: Any) -> ScheduleWrite:
    return ScheduleWrite(
        **{
            "name": "nightly",
            "spec": ScheduleSpec(cron=["0 9 * * *"]),
            "action": ScheduleAction(workflow_id=WF, input={}),
            **overrides,
        }
    )


# ------------------------------------------------------------------------------------------ start options


async def test_a_start_sends_the_business_identifier_its_policies_and_the_delay_once_and_says_whether_it_created_the_run(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(202, run_body(businessId="order-1", startDelayedUntil="2026-10-10T10:00:05Z")))
    started = await make_client().start(
        WF,
        {"a": 1},
        idempotency_key="key-12345678",
        business_id="Order/1 ",
        business_id_conflict_policy="USE_EXISTING",
        business_id_reuse_policy="REJECT_DUPLICATE",
        start_delay=timedelta(milliseconds=5000.9),
    )
    assert only(http).body == {
        "workflowId": WF,
        "input": {"a": 1},
        "businessId": "Order/1 ",
        "businessIdConflictPolicy": "USE_EXISTING",
        "businessIdReusePolicy": "REJECT_DUPLICATE",
        "startDelayMs": 5000,
    }
    assert started.started is True
    assert started.business_id == "order-1"
    assert started.start_delayed_until == datetime(2026, 10, 10, 10, 0, 5, tzinfo=timezone.utc)
    http.serve(lambda _seen: reply(202, run_body()))
    plain = await make_client().start(WF, {}, idempotency_key="key-12345678")
    assert plain.business_id is None
    assert plain.start_delayed_until is None


# ------------------------------------------------------------------------------------------ where a run came from


async def test_a_run_says_which_schedule_started_it_by_which_kind_of_action_for_which_nominal_time_and_under_which_revision_and_a_run_no_schedule_started_has_none(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            200,
            run_body(
                startedBy=f"system:schedule:{SCHEDULE}",
                schedule={
                    "scheduleId": SCHEDULE,
                    "nominalTime": "2026-11-04T02:00:00.123456Z",
                    "kind": "BACKFILL",
                    "revision": 3,
                },
            ),
        )
    )
    scheduled = await make_client().get(RUN)
    assert scheduled.schedule == RunSchedule(
        schedule_id=SCHEDULE,
        nominal_time=datetime(2026, 11, 4, 2, 0, 0, 123456, tzinfo=timezone.utc),
        kind="BACKFILL",
        revision=3,
    )
    assert scheduled.schedule is not None
    assert scheduled.schedule.nominal_time.isoformat() == "2026-11-04T02:00:00.123456+00:00", "an instant is parsed"
    assert scheduled.raw["schedule"]["nominalTime"] == "2026-11-04T02:00:00.123456Z", "the exact text stays in `raw`"
    assert scheduled.reset_from is None

    # a kind this SDK does not know is kept as the server sent it, and a plain run has neither field
    http.serve(
        lambda _seen: reply(
            200,
            run_body(
                schedule={
                    "scheduleId": SCHEDULE,
                    "nominalTime": "2026-11-04T02:00:00Z",
                    "kind": "SOMETHING_NEW",
                    "revision": 1,
                }
            ),
        )
    )
    unknown = await make_client().get(RUN)
    assert unknown.schedule is not None
    assert unknown.schedule.kind == "SOMETHING_NEW"
    http.serve(lambda _seen: reply(200, run_body()))
    plain = await make_client().get(RUN)
    assert plain.schedule is None
    assert plain.reset_from is None
    # a JSON null is an absent field
    http.serve(lambda _seen: reply(200, run_body(schedule=None, resetFrom=None)))
    nulled = await make_client().get(RUN)
    assert nulled.schedule is None
    assert nulled.reset_from is None


async def test_a_run_a_reset_made_says_the_run_it_came_from_and_the_journal_sequence_it_keeps(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    source = "0190aaaa-0000-7000-8000-0000000000aa"
    http.serve(lambda _seen: reply(200, run_body(rerunOf=source, resetFrom={"runId": source, "sequence": 4})))
    reset = await make_client().get(RUN)
    assert reset.reset_from == RunResetFrom(run_id=source, sequence=4)
    assert reset.rerun_of == source
    assert reset.schedule is None
    # started again, not reset: `rerun_of` and nothing else
    http.serve(lambda _seen: reply(200, run_body(rerunOf=source)))
    again = await make_client().get(RUN)
    assert again.rerun_of == source
    assert again.reset_from is None


@pytest.mark.parametrize(
    "broken",
    [
        {"schedule": {"scheduleId": SCHEDULE, "nominalTime": "yesterday", "kind": "SCHEDULED", "revision": 1}},
        {"schedule": {"scheduleId": SCHEDULE, "kind": "SCHEDULED", "revision": 1}},
        {
            "schedule": {
                "scheduleId": SCHEDULE,
                "nominalTime": "2026-11-04T02:00:00Z",
                "kind": "SCHEDULED",
                "revision": 0,
            }
        },
        {"schedule": {"scheduleId": SCHEDULE, "nominalTime": "2026-11-04T02:00:00Z", "kind": "SCHEDULED"}},
        {"schedule": {"nominalTime": "2026-11-04T02:00:00Z", "kind": "SCHEDULED", "revision": 1}},
        {"schedule": {"scheduleId": SCHEDULE, "nominalTime": "2026-11-04T02:00:00Z", "revision": 1}},
        {"schedule": "nightly"},
        {"resetFrom": {"runId": RUN}},
        {"resetFrom": {"runId": RUN, "sequence": 0}},
        {"resetFrom": {"sequence": 4}},
        {"resetFrom": "0190aaaa-0000-7000-8000-0000000000aa"},
    ],
    ids=[
        "schedule-nominal-time-unreadable",
        "schedule-nominal-time-missing",
        "schedule-revision-below-1",
        "schedule-revision-missing",
        "schedule-id-missing",
        "schedule-kind-missing",
        "schedule-not-an-object",
        "reset-sequence-missing",
        "reset-sequence-below-1",
        "reset-run-id-missing",
        "reset-not-an-object",
    ],
)
async def test_an_answer_whose_schedule_or_reset_is_not_what_the_contract_says_is_a_protocol_error_not_a_run_with_half_of_it(
    http: FakeHttpServer, make_client: MakeClient, broken: dict[str, Any]
) -> None:
    http.serve(lambda _seen: reply(200, run_body(**broken)))
    with pytest.raises(SagatError) as caught:
        await make_client().get(RUN)
    assert caught.value.code == "PROTOCOL"


async def test_the_200_of_use_existing_is_a_run_that_was_not_started_here_and_a_start_without_the_new_options_sends_none_of_them(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    existing = await make_client().start(
        WF, {}, idempotency_key="key-12345678", business_id="o-1", business_id_conflict_policy="USE_EXISTING"
    )
    assert existing.started is False
    assert existing.run_id == RUN, "it is still a Run"
    http.serve(lambda _seen: reply(202, run_body()))
    await make_client().start(WF, {}, idempotency_key="key-12345678")
    assert only(http).body == {"workflowId": WF, "input": {}}, "an old call is byte for byte what it was"


async def test_a_start_delay_that_is_not_a_duration_is_refused_before_anything_is_sent_and_a_retry_keeps_the_one_delay_it_computed(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(202, run_body()))
    client = make_client()
    for bad in (float("nan"), 1.5, 5, True, "5s"):
        error = await refused(client.start(WF, {}, start_delay=bad))  # type: ignore[arg-type]
        assert error.code == "VALIDATION_FAILED"
    assert http.seen == []
    attempts = 0

    def serve(_seen: Seen) -> Reply:
        nonlocal attempts
        attempts += 1
        return reply(503, refusal("UNAVAILABLE", "busy", True)) if attempts < 2 else reply(202, run_body())

    http.serve(serve)
    await client.start(WF, {}, business_id="o-2", start_delay=timedelta(milliseconds=1500))
    assert len(http.seen) == 2
    assert http.seen[0].body == http.seen[1].body, "the retry is the same body"
    assert http.seen[0].headers["idempotency-key"] == http.seen[1].headers["idempotency-key"]


@pytest.mark.parametrize(
    ("delay", "sent"),
    [
        (timedelta(milliseconds=5000.9), 5000),
        (timedelta(seconds=1.9999), 1999),
        (timedelta(microseconds=999), 0),
        (timedelta(0), 0),
        (timedelta(microseconds=-1500), -1),
        (timedelta(days=366), 31_622_400_000),
        (timedelta(days=366, microseconds=1000), 31_622_400_001),
    ],
)
async def test_a_start_delay_is_truncated_toward_zero_to_whole_milliseconds_and_its_range_is_left_to_the_server(
    http: FakeHttpServer, make_client: MakeClient, delay: timedelta, sent: int
) -> None:
    http.serve(lambda _seen: reply(202, run_body()))
    await make_client().start(WF, {}, start_delay=delay)
    assert only(http).body["startDelayMs"] == sent


# ------------------------------------------------------------------------------------------------ schedules


async def test_create_sends_the_schedule_with_its_instants_in_utc_and_one_key_and_reads_the_schedule_it_gets_back(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(201, schedule_body(futureField=1)))
    created = await make_client().schedules.create(
        NS,
        ScheduleWrite(
            name="nightly",
            spec=ScheduleSpec(
                calendars=[ScheduleCalendarSpec(hour=[ScheduleRange(start=9)], year=[ScheduleRange(start=2099)])],
                start_at="2099-01-01T01:00:00+01:00",
                timezone="UTC",
            ),
            policies=SchedulePolicies(overlap="SKIP"),
            action=ScheduleAction(workflow_id=WF, input={"n": 1}),
        ),
        idempotency_key="sched-key-1",
    )
    request = only(http)
    assert (request.method, request.url) == ("POST", f"/v1/namespaces/{NS}/schedules")
    assert request.headers["idempotency-key"] == "sched-key-1"
    assert request.body == {
        "name": "nightly",
        "spec": {
            "calendars": [{"hour": [{"start": 9}], "year": [{"start": 2099}]}],
            "startAt": "2099-01-01T00:00:00.000Z",
            "timezone": "UTC",
        },
        "policies": {"overlap": "SKIP"},
        "action": {"workflowId": WF, "input": {"n": 1}},
    }
    assert created.revision == 2
    assert created.next_action_at == datetime(2099, 1, 1, 9, tzinfo=timezone.utc)
    assert created.action.input_digest == "a" * 64, "the input is never returned, only its digest"
    assert created.raw["futureField"] == 1


async def test_describe_asks_for_the_counts_it_was_given_and_keeps_an_outcome_it_does_not_know_as_the_string_it_came_as(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            200,
            {
                **schedule_body(),
                "nextActions": [{"nominalTime": "2099-01-01T09:00:00Z", "actualTime": "2099-01-01T09:00:00Z"}],
                "recentActions": [action_body(), action_body(outcome="SOMETHING_NEWER")],
                "openRuns": [RUN],
                "bufferedCount": 1,
                "counters": {"STARTED": 1, "SOMETHING_NEWER": 1},
            },
        )
    )
    described = await make_client().schedules.get(NS, SCHEDULE, next_count=3, recent_count=5)
    request = only(http)
    assert request.method == "GET"
    assert request.url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}?nextCount=3&recentCount=5"
    assert described.next_actions[0].nominal_time == datetime(2099, 1, 1, 9, tzinfo=timezone.utc)
    assert described.recent_actions[1].outcome == "SOMETHING_NEWER"
    assert described.open_runs == (RUN,)
    assert dict(described.counters) == {"STARTED": 1, "SOMETHING_NEWER": 1}
    assert described.buffered_count == 1
    assert described.name == "nightly", "a description is a schedule"


async def test_list_sends_the_filters_it_was_given_and_list_all_follows_the_opaque_cursors_to_the_end(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    page = 0

    def serve(_seen: Seen) -> Reply:
        nonlocal page
        page += 1
        if page == 1:
            return reply(200, {"items": [schedule_body(name="a-1")], "nextCursor": "c1"})
        return reply(200, {"items": [schedule_body(name="a-2")], "nextCursor": None})

    http.serve(serve)
    names = [
        item.name
        async for item in make_client().schedules.list_all(NS, state="PAUSED", origin="API", name="a-", limit=1)
    ]
    assert names == ["a-1", "a-2"]
    assert http.seen[0].url == f"/v1/namespaces/{NS}/schedules?limit=1&state=PAUSED&origin=API&name=a-"
    assert http.seen[1].url == f"/v1/namespaces/{NS}/schedules?cursor=c1&limit=1&state=PAUSED&origin=API&name=a-"


async def test_update_carries_the_revision_it_expects_in_the_body_delete_in_the_query_and_both_carry_a_key(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, schedule_body(revision=3)))
    client = make_client()
    updated = await client.schedules.update(NS, SCHEDULE, simple_write(), 2, idempotency_key="upd-key-0001")
    assert updated.revision == 3
    request = only(http)
    assert request.method == "PUT"
    assert request.headers["idempotency-key"] == "upd-key-0001"
    assert request.body == {
        "name": "nightly",
        "spec": {"cron": ["0 9 * * *"]},
        "action": {"workflowId": WF, "input": {}},
        "expectedRevision": 2,
    }
    http.serve(lambda _seen: reply(204))
    await client.schedules.delete(NS, SCHEDULE, 3, idempotency_key="del-key-0001")
    request = only(http)
    assert request.method == "DELETE"
    assert request.url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}?expectedRevision=3"
    assert request.headers["idempotency-key"] == "del-key-0001"
    assert request.body is None


async def test_a_stale_revision_is_a_conflict_with_its_reason_and_a_revision_that_cannot_be_one_is_refused_before_it_is_sent(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            409,
            refusal("CONFLICT", "stale", False, details={"reason": "REVISION_CONFLICT", "currentRevision": 4}),
        )
    )
    client = make_client()
    error = await refused(client.schedules.update(NS, SCHEDULE, simple_write(name="n"), 2))
    assert (error.code, error.reason) == ("CONFLICT", "REVISION_CONFLICT")
    assert len(http.seen) == 1, "a conflict is never retried"
    http.seen.clear()
    assert (await refused(client.schedules.delete(NS, SCHEDULE, 0))).code == "VALIDATION_FAILED"
    assert (await refused(client.schedules.delete(NS, SCHEDULE, True))).code == "VALIDATION_FAILED"
    assert (await refused(client.schedules.update(NS, SCHEDULE, simple_write(), -1))).code == "VALIDATION_FAILED"
    assert (await refused(client.schedules.get(NS, "../x"))).code == "VALIDATION_FAILED"
    assert (await refused(client.schedules.get("payments", SCHEDULE))).code == "VALIDATION_FAILED"
    assert (await refused(client.schedules.create(NS, {"name": "x"}))).code == "VALIDATION_FAILED"  # type: ignore[arg-type]
    assert http.seen == []


async def test_pause_and_resume_send_the_note_trigger_sends_the_overlap_for_this_action_only_and_a_trigger_retry_keeps_its_key(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    client = make_client()
    http.serve(lambda _seen: reply(200, schedule_body(state="PAUSED", note="maintenance")))
    paused = await client.schedules.pause(NS, SCHEDULE, "maintenance", idempotency_key="pause-key-01")
    assert paused.state == "PAUSED"
    assert only(http).url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}/pause"
    assert only(http).body == {"note": "maintenance"}
    http.serve(lambda _seen: reply(200, schedule_body()))
    await client.schedules.resume(NS, SCHEDULE, "done", idempotency_key="resume-key-1")
    assert only(http).url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}/resume"
    http.seen.clear()
    assert (await refused(client.schedules.pause(NS, SCHEDULE, ""))).code == "VALIDATION_FAILED"
    assert http.seen == []

    attempts = 0

    def serve(_seen: Seen) -> Reply:
        nonlocal attempts
        attempts += 1
        if attempts < 2:
            return reply(503, refusal("UNAVAILABLE", "busy", True))
        return reply(202, action_body(outcome="SKIPPED_OVERLAP"))

    http.serve(serve)
    triggered = await client.schedules.trigger(NS, SCHEDULE, overlap="SKIP")
    assert triggered.outcome == "SKIPPED_OVERLAP"
    assert len(http.seen) == 2
    assert http.seen[0].headers["idempotency-key"] == http.seen[1].headers["idempotency-key"]
    assert http.seen[1].body == {"overlap": "SKIP"}
    http.serve(lambda _seen: reply(202, action_body()))
    await client.schedules.trigger(NS, SCHEDULE)
    assert only(http).body == {}, "no overlap given, none sent, and never a null"


async def test_a_backfill_sends_its_range_as_utc_instants_a_cancel_names_the_backfill_and_the_history_can_be_filtered_by_outcome(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    client = make_client()
    backfill_body = {
        "backfillId": BATCH,
        "start": "2026-10-10T08:00:00Z",
        "end": "2026-10-10T10:00:00Z",
        "cursor": None,
        "state": "PENDING",
        "overlap": "ALLOW_ALL",
        "counts": {"STARTED": 2},
    }
    http.serve(lambda _seen: reply(202, backfill_body))
    backfill = await client.schedules.backfill(
        NS,
        SCHEDULE,
        datetime(2026, 10, 10, 8, tzinfo=timezone.utc),
        "2026-10-10T10:00:00+00:00",
        overlap="ALLOW_ALL",
        idempotency_key="back-key-001",
    )
    assert only(http).body == {
        "start": "2026-10-10T08:00:00.000Z",
        "end": "2026-10-10T10:00:00.000Z",
        "overlap": "ALLOW_ALL",
    }
    assert backfill.cursor is None
    assert dict(backfill.counts) == {"STARTED": 2}
    http.seen.clear()
    now = datetime.now(timezone.utc)
    assert (await refused(client.schedules.backfill(NS, SCHEDULE, "not a time", now))).code == "VALIDATION_FAILED"
    naive = datetime(2026, 10, 10, 8)
    assert (await refused(client.schedules.backfill(NS, SCHEDULE, naive, now))).code == "VALIDATION_FAILED"
    no_offset = "2026-10-10T08:00:00"
    assert (await refused(client.schedules.backfill(NS, SCHEDULE, no_offset, now))).code == "VALIDATION_FAILED"
    assert http.seen == [], "an instant with no zone is never guessed"
    http.serve(lambda _seen: reply(200, {**backfill_body, "state": "CANCELLED", "counts": {}}))
    cancelled = await client.schedules.cancel_backfill(NS, SCHEDULE, BATCH)
    assert cancelled.state == "CANCELLED"
    request = only(http)
    assert request.url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}/backfills/{BATCH}/cancel"
    assert request.body is None
    assert len(request.headers["idempotency-key"]) >= 8, "a key is made when none is given"
    http.serve(lambda _seen: reply(200, {"items": [action_body(outcome="ALREADY_ACTED")], "nextCursor": None}))
    actions = await client.schedules.list_actions(NS, SCHEDULE, outcome="ALREADY_ACTED", limit=10)
    assert only(http).url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}/actions?limit=10&outcome=ALREADY_ACTED"
    assert actions.items[0].outcome == "ALREADY_ACTED"
    http.serve(lambda _seen: reply(200, {"items": [backfill_body]}))
    listed = await client.schedules.list_backfills(NS, SCHEDULE)
    assert only(http).url == f"/v1/namespaces/{NS}/schedules/{SCHEDULE}/backfills"
    assert [item.state for item in listed] == ["PENDING"]


# --------------------------------------------------------------------------------------------- business ids


async def test_a_business_identifier_is_percent_encoded_as_given_and_one_that_cannot_be_a_path_segment_is_refused_locally(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            200,
            {
                "businessId": "Ord/1#x?y",
                "executionId": RUN,
                "currentRunId": RUN,
                "open": False,
                "lastStatus": "SUCCEEDED",
                "workflowId": WF,
                "boundAt": "2026-10-10T10:00:00Z",
                "reusableAfter": "2026-11-09T10:00:00Z",
            },
        )
    )
    client = make_client()
    binding = await client.business_ids.get(NS, "Ord/1#x?y")
    assert only(http).url == f"/v1/namespaces/{NS}/business-ids/Ord%2F1%23x%3Fy", "no trimming and no case folding"
    assert binding.open is False
    assert binding.reusable_after == datetime(2026, 11, 9, 10, tzinfo=timezone.utc)
    http.seen.clear()
    for bad in ("", " padded", "with space", "x" * 201, "ünï", "tab\t", ".", ".."):
        assert (await refused(client.business_ids.get(NS, bad))).code == "VALIDATION_FAILED"
    assert http.seen == [], "a whole dot segment would be a step in the path, so it is never sent"
    await client.business_ids.get(NS, "x" * 200)
    assert len(http.seen) == 1
    http.seen.clear()
    for dotted in ("...", "a..b", ".a", "a."):
        await client.business_ids.get(NS, dotted)
    assert [seen.url.rsplit("/", 1)[1] for seen in http.seen] == ["...", "a..b", ".a", "a."], "sent as given"


# ------------------------------------------------------------------------------------------------- batch


async def test_a_batch_is_previewed_created_with_one_key_read_cancelled_and_its_items_filtered_by_state(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    job = {
        "batchId": BATCH,
        "action": "CANCEL",
        "state": "RUNNING",
        "createdBy": "alice",
        "createdAt": "2026-10-10T10:00:00Z",
        "counts": {"selected": 2, "succeeded": 1, "skipped": 0, "failed": 0, "cancelled": 0, "deferred": 1},
    }
    client = make_client()
    http.serve(lambda _seen: reply(200, {"matched": 2, "capped": False, "sample": [RUN]}))
    preview = await client.batch.preview(NS, BatchSelector(run_ids=[RUN]), 50)
    assert only(http).url == f"/v1/namespaces/{NS}/batch-operations/preview"
    assert only(http).body == {"selector": {"runIds": [RUN]}, "maxItems": 50}
    assert (preview.matched, preview.capped, preview.sample) == (2, False, (RUN,))

    http.serve(lambda _seen: reply(202, job))
    created = await client.batch.create(
        NS,
        BatchRequest(
            selector=BatchSelector(filter=BatchFilter(status=["RUNNING"], created_after="2026-10-10T02:00:00+02:00")),
            action="CANCEL",
            reason="clean up the day",
            max_items=10,
        ),
        idempotency_key="batch-key-01",
    )
    assert only(http).body == {
        "selector": {"filter": {"status": ["RUNNING"], "createdAfter": "2026-10-10T00:00:00.000Z"}},
        "action": "CANCEL",
        "reason": "clean up the day",
        "maxItems": 10,
    }
    assert only(http).headers["idempotency-key"] == "batch-key-01"
    counts = created.counts
    assert (counts.selected, counts.succeeded, counts.skipped, counts.failed, counts.cancelled, counts.deferred) == (
        2,
        1,
        0,
        0,
        0,
        1,
    )
    assert created.has_payload is False

    http.serve(lambda _seen: reply(202, {**job, "state": "CANCELLING"}))
    cancelling = await client.batch.cancel(NS, BATCH, "stop the clean up", idempotency_key="batch-cancel-1")
    assert only(http).url == f"/v1/namespaces/{NS}/batch-operations/{BATCH}/cancel"
    assert only(http).body == {"reason": "stop the clean up"}
    assert cancelling.state == "CANCELLING"

    http.serve(
        lambda _seen: reply(
            200,
            {
                "items": [
                    {"ordinal": 0, "runId": RUN, "state": "SKIPPED", "reason": "ALREADY_TERMINAL"},
                    {"ordinal": 1, "runId": RUN, "state": "NEWER_STATE"},
                ],
                "nextCursor": "n",
            },
        )
    )
    items = await client.batch.list_items(NS, BATCH, state="SKIPPED", limit=2)
    assert only(http).url == f"/v1/namespaces/{NS}/batch-operations/{BATCH}/items?limit=2&state=SKIPPED"
    assert items.items[0].reason == "ALREADY_TERMINAL"
    assert items.items[1].state == "NEWER_STATE", "a state of a newer server is kept as a string"
    assert items.next_cursor == "n"

    http.serve(lambda _seen: reply(200, {"items": [job], "nextCursor": None}))
    jobs = await client.batch.list(NS, state="RUNNING", limit=1)
    assert only(http).url == f"/v1/namespaces/{NS}/batch-operations?limit=1&state=RUNNING"
    assert jobs.items[0].batch_id == BATCH
    http.serve(lambda _seen: reply(200, job))
    assert (await client.batch.get(NS, BATCH)).action == "CANCEL"
    assert only(http).url == f"/v1/namespaces/{NS}/batch-operations/{BATCH}"
    http.seen.clear()
    assert (await refused(client.batch.get(NS, "not-a-uuid"))).code == "VALIDATION_FAILED"
    assert (await refused(client.batch.preview(NS, {"runIds": [RUN]}))).code == "VALIDATION_FAILED"  # type: ignore[arg-type]
    assert http.seen == []


async def test_a_selector_is_sent_as_given_an_empty_list_is_not_a_missing_one(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(422, refusal("VALIDATION_FAILED", "no", False, details={"reason": "EMPTY"})))
    error = await refused(
        make_client().batch.create(
            NS, BatchRequest(selector=BatchSelector(run_ids=[]), action="CANCEL", reason="nothing to select")
        )
    )
    assert error.reason == "EMPTY"
    assert only(http).body["selector"] == {"runIds": []}


# --------------------------------------------------------------------------------------------- with start


async def test_signal_with_start_names_the_workflow_by_id_carries_the_signal_and_one_key_and_says_what_the_signal_woke(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(202, {"runId": RUN, "started": True, "deliveredTo": ["wait"]}))
    result = await make_client().signal_with_start(
        WF,
        "order-9",
        {"a": 1},
        "go",
        {"b": 2},
        idempotency_key="sws-key-0001",
        business_id_reuse_policy="ALLOW_DUPLICATE_FAILED_ONLY",
    )
    assert only(http).url == "/v1/runs/signal-with-start"
    assert only(http).body == {
        "workflowId": WF,
        "businessId": "order-9",
        "input": {"a": 1},
        "signal": {"name": "go", "payload": {"b": 2}},
        "businessIdReusePolicy": "ALLOW_DUPLICATE_FAILED_ONLY",
    }
    assert only(http).headers["idempotency-key"] == "sws-key-0001"
    assert (result.run_id, result.started, result.delivered_to) == (RUN, True, ("wait",))


async def test_update_with_start_requires_the_callers_key_keeps_it_across_a_retry_and_reads_a_rejection_as_an_answer(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    receipt = {
        "runId": RUN,
        "operation": "set",
        "updateKey": "u-1",
        "status": "APPLIED",
        "result": 7,
        "beforeStateRevision": 0,
        "afterStateRevision": 1,
        "runVersion": 4,
        "replayed": True,
    }
    client = make_client()
    http.serve(
        lambda _seen: reply(200, {"runId": RUN, "started": True, "update": {"outcome": "APPLIED", "receipt": receipt}})
    )
    applied = await client.update_with_start(
        WF,
        "order-9",
        {},
        "set",
        "u-1",
        {"value": 7},
        idempotency_key="uws-key-0001",
        business_id_conflict_policy="FAIL",
        start_delay=timedelta(seconds=2),
        expected_state_revision=0,
    )
    assert only(http).url == "/v1/runs/update-with-start"
    assert only(http).body == {
        "workflowId": WF,
        "businessId": "order-9",
        "input": {},
        "update": {"name": "set", "updateKey": "u-1", "args": {"value": 7}, "expectedStateRevision": 0},
        "businessIdConflictPolicy": "FAIL",
        "startDelayMs": 2000,
    }
    assert only(http).headers["idempotency-key"] == "uws-key-0001"
    assert applied.update.outcome == "APPLIED"
    assert applied.update.receipt is not None
    assert applied.update.receipt.after_state_revision == 1
    assert applied.update.receipt.result == 7
    assert applied.update.receipt.replayed is True, "a replay says so"

    http.serve(
        lambda _seen: reply(
            200, {"runId": RUN, "started": False, "update": {"outcome": "REJECTED", "reason": "REVISION_CONFLICT"}}
        )
    )
    rejected = await client.update_with_start(WF, "order-9", {}, "set", "u-2", {}, idempotency_key="uws-key-0002")
    assert (rejected.started, rejected.update.outcome, rejected.update.reason, rejected.update.receipt) == (
        False,
        "REJECTED",
        "REVISION_CONFLICT",
        None,
    )

    http.seen.clear()
    assert (await refused(client.update_with_start(WF, "order-9", {}, "set", "", {}))).code == "VALIDATION_FAILED"
    assert (
        await refused(client.update_with_start(WF, "order-9", {}, "set", "u" * 129, {}))
    ).code == "VALIDATION_FAILED"
    negative = client.update_with_start(WF, "order-9", {}, "set", "u-3", {}, expected_state_revision=-1)
    assert (await refused(negative)).code == "VALIDATION_FAILED"
    assert http.seen == []

    attempts = 0

    def serve(_seen: Seen) -> Reply:
        nonlocal attempts
        attempts += 1
        if attempts < 2:
            return reply(503, refusal("UNAVAILABLE", "busy", True))
        return reply(200, {"runId": RUN, "started": True, "update": {"outcome": "APPLIED", "receipt": receipt}})

    http.serve(serve)
    await client.update_with_start(WF, "order-9", {}, "set", "u-4", {})
    assert len(http.seen) == 2
    assert http.seen[0].body == http.seen[1].body
    assert http.seen[0].headers["idempotency-key"] == http.seen[1].headers["idempotency-key"]


# ------------------------------------------------------------------------------------------------- reset


async def test_the_reset_calls_list_the_points_preview_the_plan_reset_with_a_key_and_read_the_register(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    client = make_client()
    http.serve(
        lambda _seen: reply(
            200,
            {
                "items": [
                    {"sequence": 1, "kind": "RUN_STARTED", "node": None, "occurredAt": "2026-10-10T10:00:00Z"},
                    {"sequence": 4, "kind": "SOMETHING_NEWER", "node": "a", "occurredAt": "2026-10-10T10:00:01Z"},
                ],
                "nextCursor": None,
            },
        )
    )
    points = await client.reset_points(RUN, limit=5)
    assert only(http).url == f"/v1/runs/{RUN}/reset-points?limit=5"
    assert [(p.sequence, p.kind, p.node) for p in points.items] == [
        (1, "RUN_STARTED", None),
        (4, "SOMETHING_NEWER", "a"),
    ]

    http.serve(
        lambda _seen: reply(
            200,
            {
                "sequence": 4,
                "restored": [{"node": "a", "kind": "STEP"}],
                "reexecuted": [{"node": "b", "completedInSource": True, "markers": ["irreversible"]}],
                "reapply": [],
                "requiresAcknowledgement": True,
                "prefixEvents": 6,
            },
        )
    )
    plan = await client.preview_reset(RUN, 4, "NONE")
    assert only(http).body == {"sequence": 4, "reapply": "NONE"}
    assert plan.requires_acknowledgement is True
    assert [(s.node, s.completed_in_source, s.markers) for s in plan.reexecuted] == [("b", True, ("irreversible",))]
    http.seen.clear()
    assert (await refused(client.preview_reset(RUN, 0))).code == "VALIDATION_FAILED"
    assert (await refused(client.preview_reset("not-a-run", 1))).code == "VALIDATION_FAILED"
    assert http.seen == []

    http.serve(
        lambda _seen: reply(
            202,
            {
                "resetId": BATCH,
                "sourceRunId": RUN,
                "sequence": 4,
                "state": "CREATED",
                "newRunId": SCHEDULE,
                "reapplyReport": [],
                "createdAt": "2026-10-10T10:00:02Z",
            },
        )
    )
    made = await client.reset(
        RUN,
        sequence=4,
        reason="start over from a",
        acknowledge_reexecution=True,
        business_id="DROP",
        idempotency_key="reset-key-001",
    )
    assert only(http).url == f"/v1/runs/{RUN}/resets"
    assert only(http).headers["idempotency-key"] == "reset-key-001"
    assert only(http).body == {
        "sequence": 4,
        "reason": "start over from a",
        "acknowledgeReexecution": True,
        "businessId": "DROP",
    }
    assert (made.state, made.new_run_id, made.failure_reason) == ("CREATED", SCHEDULE, None)
    http.serve(
        lambda _seen: reply(202, {"resetId": BATCH, "sourceRunId": RUN, "sequence": 1, "state": "PENDING_SOURCE"})
    )
    await client.reset(RUN, sequence=1, reason="start over from the top")
    assert only(http).body == {"sequence": 1, "reason": "start over from the top"}, "what was not asked is not sent"
    assert len(only(http).headers["idempotency-key"]) >= 8, "a key is made when none is given"

    http.serve(
        lambda _seen: reply(200, {"items": [{"resetId": BATCH, "sourceRunId": RUN, "sequence": 4, "state": "CREATED"}]})
    )
    assert len(await client.list_resets(RUN)) == 1
    http.serve(lambda _seen: reply(200, {"resetId": BATCH, "sourceRunId": RUN, "sequence": 4, "state": "CREATED"}))
    assert (await client.get_reset(RUN, BATCH)).reset_id == BATCH
    assert only(http).url == f"/v1/runs/{RUN}/resets/{BATCH}"


# ------------------------------------------------------------------------------------------- the rest


async def test_the_token_does_not_appear_in_anything_a_group_of_the_client_exposes(make_client: MakeClient) -> None:
    client = make_client()
    for group in (client.schedules, client.batch, client.business_ids):
        for text in (repr(group), str(vars(group)), repr(vars(group)), repr(copy.copy(group).__dict__)):
            assert "tok-1" not in text
        for value in vars(group).values():
            assert "tok-1" not in repr(value)
    with pytest.raises(TypeError):
        pickle.dumps(client._config.token)


async def test_a_deadline_and_a_deadline_in_together_are_refused_by_the_with_start_calls_too(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(202, {}))
    client = make_client()
    deadline = datetime(2030, 1, 1, tzinfo=timezone.utc)
    sws = client.signal_with_start(WF, "b-1", {}, "go", deadline=deadline, deadline_in=5)
    assert (await refused(sws)).code == "VALIDATION_FAILED"
    uws = client.update_with_start(WF, "b-1", {}, "set", "u-1", {}, deadline=deadline, deadline_in=5)
    assert (await refused(uws)).code == "VALIDATION_FAILED"
    assert http.seen == []


def test_the_blocking_client_has_the_groups_and_the_new_calls(http: FakeHttpServer) -> None:
    def serve(seen: Seen) -> Reply:
        if seen.url.startswith(f"/v1/namespaces/{NS}/schedules?"):
            if "cursor=c1" in seen.url:
                return reply(200, {"items": [schedule_body(name="a-2")], "nextCursor": None})
            return reply(200, {"items": [schedule_body(name="a-1")], "nextCursor": "c1"})
        if "/business-ids/" in seen.url:
            return reply(
                200,
                {
                    "businessId": "order-1",
                    "executionId": RUN,
                    "currentRunId": RUN,
                    "open": True,
                    "workflowId": WF,
                    "boundAt": "2026-10-10T10:00:00Z",
                },
            )
        if seen.url.endswith("/batch-operations/preview"):
            return reply(200, {"matched": 1, "capped": False, "sample": [RUN]})
        if seen.url == "/v1/runs/signal-with-start":
            return reply(202, {"runId": RUN, "started": True, "deliveredTo": ["wait"]})
        if seen.url.endswith("/resets"):
            return reply(202, {"resetId": BATCH, "sourceRunId": RUN, "sequence": 1, "state": "CREATED"})
        return reply(404)

    http.serve(serve)
    config = ClientConfig(api_url=http.base, namespace="payments", token="tok-1", retry=FAST)
    with SyncSagatClient(config) as client:
        assert [s.name for s in client.schedules.list_all(NS, limit=1)] == ["a-1", "a-2"]
        assert client.schedules.list(NS, name="a-").items[0].name == "a-1"
        assert client.business_ids.get(NS, "order-1").open is True
        assert client.batch.preview(NS, BatchSelector(run_ids=[RUN])).sample == (RUN,)
        assert client.signal_with_start(WF, "order-1", {}, "go", {}).delivered_to == ("wait",)
        assert client.reset(RUN, sequence=1, reason="start over from the top").state == "CREATED"
        with pytest.raises(SagatError) as caught:
            client.schedules.get(NS, "../x")
        assert caught.value.code == "VALIDATION_FAILED"
        seen_before = len(http.seen)
        for dot_segment in (".", ".."):
            with pytest.raises(SagatError) as dotted:
                client.business_ids.get(NS, dot_segment)
            assert dotted.value.code == "VALIDATION_FAILED"
        assert len(http.seen) == seen_before, "nothing is sent for a dot segment"
