"""The client surface of feature 018: schedules, business identifiers, batch jobs, the with-start calls and the
reset of a run (``specs/018-schedules-bulk-operations/contracts/sdk-conformance.md``).

Three groups are reached from ``SagatClient`` (``schedules``, ``batch``, ``business_ids``); the calls about a run
(``signal_with_start``, ``update_with_start``, ``reset_points``, ``preview_reset``, ``reset``...) are methods of the
client itself and use the readers at the end of this module.

Rules shared with ``sdks/SPEC.md`` section 3:

* every call that changes something carries one ``Idempotency-Key``, given or generated once per call and kept by
  every retry of that call;
* a value of an enumeration this SDK does not know is kept as the raw string it came as, never an error;
* every typed answer keeps the JSON it was read from in ``raw``;
* an instant you send is an aware ``datetime`` or RFC 3339 text and goes out in UTC (``Z``); a naive ``datetime``
  or text without an offset is refused, because the SDK never guesses a local offset. An instant you read is an
  aware ``datetime`` in UTC.
"""

from __future__ import annotations

import re
from collections.abc import AsyncIterator, Callable, Mapping, Sequence
from dataclasses import dataclass, field, fields, is_dataclass
from datetime import datetime
from typing import Any, Generic, Literal, TypeVar
from urllib.parse import quote

from ._http import HttpRequest, HttpTransport
from ._json import format_instant, parse_instant
from ._wire import (
    id_of,
    idempotency_key_of,
    instant_of,
    int_of,
    namespace_id_of,
    object_of,
    opt_number,
    opt_str,
    positive_int,
    revision,
    str_of,
)
from .errors import SagatError, invalid

T = TypeVar("T")

OverlapPolicy = Literal["SKIP", "BUFFER_ONE", "BUFFER_ALL", "CANCEL_OTHER", "ALLOW_ALL"]
BusinessIdConflictPolicy = Literal["FAIL", "USE_EXISTING"]
BusinessIdReusePolicy = Literal["ALLOW_DUPLICATE", "ALLOW_DUPLICATE_FAILED_ONLY", "REJECT_DUPLICATE"]
ResetReapply = Literal["NONE", "SIGNALS", "UPDATES", "ALL"]
ResetSourcePolicy = Literal["REQUIRE_TERMINAL", "CANCEL_SOURCE"]
KeepOrDrop = Literal["KEEP", "DROP"]

Trace = Callable[[str | None], str | None]
"""Checks an explicit ``traceparent`` or supplies the ambient one (``ClientConfig.trace_context``)."""

_BUSINESS_ID_PATH = re.compile(r"[\x21-\x7e]{1,200}")


@dataclass(frozen=True)
class Page(Generic[T]):
    """One page of an answer. ``next_cursor`` is opaque; ``None`` on the last page."""

    items: tuple[T, ...]
    next_cursor: str | None


# ------------------------------------------------------------------------------------------- the wire

_INSTANT: Any = {"instant": True}


def _camel(name: str) -> str:
    head, *tail = name.split("_")
    return head + "".join(part.capitalize() for part in tail)


def _when(value: datetime | str, what: str) -> str:
    """An instant given as an aware ``datetime`` or as RFC 3339 text, sent in UTC as a ``Z`` instant."""
    if isinstance(value, datetime):
        return format_instant(value)
    parsed = parse_instant(value)
    if parsed is None:
        raise invalid(f"{what} is not an instant: use an aware datetime or RFC 3339 text with an offset")
    return format_instant(parsed)


def _wire(value: Any) -> Any:
    """A request dataclass as the JSON the API takes: camelCase names, a field that is ``None`` left out."""
    if is_dataclass(value) and not isinstance(value, type):
        out: dict[str, Any] = {}
        for item in fields(value):
            held = getattr(value, item.name)
            if held is not None:
                out[_camel(item.name)] = _when(held, item.name) if item.metadata.get("instant") else _wire(held)
        return out
    if isinstance(value, (list, tuple)):
        return [_wire(entry) for entry in value]
    return value


def _body(value: Any, kind: type, what: str) -> dict[str, Any]:
    if not isinstance(value, kind):
        raise invalid(f"{what} is a {kind.__name__}")
    body: dict[str, Any] = _wire(value)
    return body


def _page(body: Any, what: str, read: Callable[[Any], T]) -> Page[T]:
    raw = object_of(body, what)
    items = raw.get("items")
    if not isinstance(items, list):
        raise SagatError("PROTOCOL", "items is missing from the page")
    return Page(tuple(read(item) for item in items), opt_str(raw, "nextCursor"))


def _strings(value: Any) -> tuple[str, ...]:
    return tuple(str(item) for item in value) if isinstance(value, list) else ()


def _counters(value: Any) -> Mapping[str, int | float]:
    if not isinstance(value, dict):
        return {}
    return {
        name: count for name, count in value.items() if isinstance(count, (int, float)) and not isinstance(count, bool)
    }


class _Group:
    def __init__(self, http: HttpTransport, trace: Trace) -> None:
        self._http = http
        self._trace = trace


# ------------------------------------------------------------------------------------------- schedules


@dataclass(frozen=True)
class ScheduleRange:
    """A range of a calendar field. ``end`` is inclusive and equals ``start`` when absent."""

    start: int
    end: int | None = None
    step: int | None = None


@dataclass(frozen=True)
class ScheduleCalendarSpec:
    """A structured calendar. Each field is a union of ranges; the server checks the domain of each."""

    second: Sequence[ScheduleRange] | None = None
    minute: Sequence[ScheduleRange] | None = None
    hour: Sequence[ScheduleRange] | None = None
    day_of_month: Sequence[ScheduleRange] | None = None
    month: Sequence[ScheduleRange] | None = None
    year: Sequence[ScheduleRange] | None = None
    day_of_week: Sequence[ScheduleRange] | None = None
    comment: str | None = None


@dataclass(frozen=True)
class ScheduleInterval:
    every_ms: int
    offset_ms: int | None = None


@dataclass(frozen=True)
class ScheduleDst:
    gap: Literal["SKIP", "SHIFT_FORWARD"] | None = None
    ambiguous: Literal["ONCE_EARLIER", "TWICE"] | None = None


@dataclass(frozen=True)
class ScheduleSpec:
    """When a schedule acts: at least one of ``calendars``, ``cron`` and ``intervals`` (the server judges)."""

    calendars: Sequence[ScheduleCalendarSpec] | None = None
    cron: Sequence[str] | None = None
    intervals: Sequence[ScheduleInterval] | None = None
    exclusions: Sequence[ScheduleCalendarSpec] | None = None
    start_at: datetime | str | None = field(default=None, metadata=_INSTANT)
    end_at: datetime | str | None = field(default=None, metadata=_INSTANT)
    timezone: str | None = None
    jitter_ms: int | None = None
    dst: ScheduleDst | None = None


@dataclass(frozen=True)
class SchedulePolicies:
    overlap: OverlapPolicy | None = None
    catchup_window_ms: int | None = None
    catchup_mode: Literal["ONE", "ALL"] | None = None
    pause_on_failure: bool | None = None
    buffer_limit: int | None = None


@dataclass(frozen=True)
class ScheduleAction:
    """What a schedule starts."""

    workflow_id: str
    input: Mapping[str, Any]
    """The input of every run, at most 64 KiB. It is never returned: only its digest is."""
    version_policy: Literal["ACTIVE", "PINNED"] | None = None
    version_id: str | None = None
    nominal_time_field: str | None = None
    run_timeout_ms: int | None = None
    business_id_prefix: str | None = field(default=None, repr=False)
    """Every run the schedule starts gets the business identifier ``<prefix>-<nominal time>`` (the nominal time in
    RFC 3339 UTC to the second, ending in ``Z``). Customer text, sent as given (no trimming, no case folding, no
    local check: the server judges it, ``VALIDATION_FAILED`` with the reason ``SCHEDULE_SPEC_INVALID``) and kept out
    of ``repr``. ``None`` sends nothing."""
    business_id_conflict_policy: BusinessIdConflictPolicy | None = None
    """Only with a prefix: what the action does when an open run holds the identifier (the server's default is
    ``FAIL``)."""
    business_id_reuse_policy: BusinessIdReusePolicy | None = None
    """Only with a prefix: what the action does when a closed run still remembers the identifier (the server's
    default is ``ALLOW_DUPLICATE``)."""


@dataclass(frozen=True)
class ScheduleWrite:
    """What a schedule is written as (``schedules.create`` and ``schedules.update``)."""

    name: str
    spec: ScheduleSpec
    action: ScheduleAction
    policies: SchedulePolicies | None = None
    paused: bool | None = None
    note: str | None = None
    remaining_actions: int | None = None


@dataclass(frozen=True)
class ScheduleActionSummary:
    """The action as the server keeps it: without the input, with its digest."""

    workflow_id: str | None
    version_policy: str | None
    version_id: str | None
    input_digest: str | None
    nominal_time_field: str | None
    run_timeout_ms: int | float | None
    business_id_prefix: str | None = field(default=None, repr=False)
    """The prefix of the business identifiers of the runs the schedule starts, as the server sent it (customer text,
    kept out of ``repr``); ``None`` when the action has none."""
    business_id_conflict_policy: str | None = None
    """The server's word for the conflict policy (``FAIL``, ``USE_EXISTING`` or a value newer than this SDK, kept
    as sent); ``None`` when the action has no prefix."""
    business_id_reuse_policy: str | None = None
    """The server's word for the reuse policy (``ALLOW_DUPLICATE``, ``ALLOW_DUPLICATE_FAILED_ONLY``,
    ``REJECT_DUPLICATE`` or a value newer than this SDK, kept as sent); ``None`` when the action has no prefix."""


@dataclass(frozen=True)
class Schedule:
    schedule_id: str
    name: str
    origin: str
    state: str
    revision: int
    exhausted: bool
    action: ScheduleActionSummary
    note: str | None
    note_by: str | None
    remaining_actions: int | float | None
    next_action_at: datetime | None
    created_at: datetime
    updated_at: datetime | None
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class ScheduleActionRecord:
    nominal_time: datetime
    actual_time: datetime | None
    kind: str
    origin: str | None
    revision: int
    outcome: str
    """One of the outcomes this SDK knows, or another string: the set only grows."""
    reason: str | None
    run_id: str | None
    backfill_id: str | None
    summary_count: int | float | None
    summary_last_nominal_time: datetime | None
    decided_at: datetime
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class ScheduledTime:
    nominal_time: datetime
    actual_time: datetime


@dataclass(frozen=True)
class ScheduleDescription(Schedule):
    """A schedule with its next times, its latest decisions, the runs it has open and its counters."""

    next_actions: tuple[ScheduledTime, ...]
    recent_actions: tuple[ScheduleActionRecord, ...]
    open_runs: tuple[str, ...]
    buffered_count: int | float
    counters: Mapping[str, int | float]


@dataclass(frozen=True)
class ScheduleBackfill:
    backfill_id: str
    start: datetime
    end: datetime
    cursor: datetime | None
    state: str
    overlap: str
    counts: Mapping[str, int | float]
    raw: Mapping[str, Any] = field(repr=False, compare=False)


def _opt_instant(raw: Mapping[str, Any], key: str) -> datetime | None:
    return parse_instant(raw.get(key))


def to_schedule(value: Any) -> Schedule:
    raw = object_of(value, "a schedule")
    held = raw.get("action")
    action = held if isinstance(held, dict) else {}
    return Schedule(
        schedule_id=str_of(raw, "scheduleId"),
        name=str_of(raw, "name"),
        origin=str_of(raw, "origin"),
        state=str_of(raw, "state"),
        revision=revision(raw, "revision", 1),
        exhausted=raw.get("exhausted") is True,
        action=ScheduleActionSummary(
            workflow_id=opt_str(action, "workflowId"),
            version_policy=opt_str(action, "versionPolicy"),
            version_id=opt_str(action, "versionId"),
            input_digest=opt_str(action, "inputDigest"),
            nominal_time_field=opt_str(action, "nominalTimeField"),
            run_timeout_ms=opt_number(action, "runTimeoutMs"),
            business_id_prefix=opt_str(action, "businessIdPrefix"),
            business_id_conflict_policy=opt_str(action, "businessIdConflictPolicy"),
            business_id_reuse_policy=opt_str(action, "businessIdReusePolicy"),
        ),
        note=opt_str(raw, "note"),
        note_by=opt_str(raw, "noteBy"),
        remaining_actions=opt_number(raw, "remainingActions"),
        next_action_at=_opt_instant(raw, "nextActionAt"),
        created_at=instant_of(raw, "createdAt"),
        updated_at=_opt_instant(raw, "updatedAt"),
        raw=raw,
    )


def to_schedule_action_record(value: Any) -> ScheduleActionRecord:
    raw = object_of(value, "a schedule action")
    return ScheduleActionRecord(
        nominal_time=instant_of(raw, "nominalTime"),
        actual_time=_opt_instant(raw, "actualTime"),
        kind=str_of(raw, "kind"),
        origin=opt_str(raw, "origin"),
        revision=revision(raw, "revision", 1),
        outcome=str_of(raw, "outcome"),
        reason=opt_str(raw, "reason"),
        run_id=opt_str(raw, "runId"),
        backfill_id=opt_str(raw, "backfillId"),
        summary_count=opt_number(raw, "summaryCount"),
        summary_last_nominal_time=_opt_instant(raw, "summaryLastNominalTime"),
        decided_at=instant_of(raw, "decidedAt"),
        raw=raw,
    )


def to_schedule_description(value: Any) -> ScheduleDescription:
    raw = object_of(value, "a schedule description")
    base = to_schedule(raw)
    upcoming = raw.get("nextActions")
    recent = raw.get("recentActions")
    buffered = opt_number(raw, "bufferedCount")
    next_actions: tuple[ScheduledTime, ...] = ()
    if isinstance(upcoming, list):
        next_actions = tuple(
            ScheduledTime(
                instant_of(entry, "nominalTime"),
                instant_of(entry, "actualTime"),
            )
            for entry in (object_of(item, "a next action") for item in upcoming)
        )
    return ScheduleDescription(
        **{item.name: getattr(base, item.name) for item in fields(Schedule)},
        next_actions=next_actions,
        recent_actions=tuple(to_schedule_action_record(item) for item in recent) if isinstance(recent, list) else (),
        open_runs=_strings(raw.get("openRuns")),
        buffered_count=buffered if buffered is not None else 0,
        counters=_counters(raw.get("counters")),
    )


def to_schedule_backfill(value: Any) -> ScheduleBackfill:
    raw = object_of(value, "a backfill")
    return ScheduleBackfill(
        backfill_id=str_of(raw, "backfillId"),
        start=instant_of(raw, "start"),
        end=instant_of(raw, "end"),
        cursor=_opt_instant(raw, "cursor"),
        state=str_of(raw, "state"),
        overlap=str_of(raw, "overlap"),
        counts=_counters(raw.get("counts")),
        raw=raw,
    )


class Schedules(_Group):
    """Schedules: create, describe, list, update, delete, pause, resume, trigger, backfill and their history."""

    @staticmethod
    def _path(namespace_id: str, schedule_id: str | None = None, tail: str = "") -> str:
        base = f"/namespaces/{quote(namespace_id_of(namespace_id), safe='')}/schedules"
        if schedule_id is None:
            return base
        return f"{base}/{quote(id_of(schedule_id, 'a schedule id'), safe='')}{tail}"

    async def create(
        self,
        namespace_id: str,
        write: ScheduleWrite,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> Schedule:
        """Creates a schedule. A name in use is ``CONFLICT``; the same key and body answers what the first call did."""
        request = HttpRequest(
            "POST",
            self._path(namespace_id),
            body=_body(write, ScheduleWrite, "a schedule"),
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_schedule((await self._http.request(request)).body)

    async def get(
        self,
        namespace_id: str,
        schedule_id: str,
        *,
        next_count: int | None = None,
        recent_count: int | None = None,
        traceparent: str | None = None,
    ) -> ScheduleDescription:
        """A schedule with its next times, its latest decisions, the runs it has open and its counters."""
        request = HttpRequest(
            "GET",
            self._path(namespace_id, schedule_id),
            query={"nextCount": next_count, "recentCount": recent_count},
            traceparent=self._trace(traceparent),
        )
        return to_schedule_description((await self._http.request(request)).body)

    async def list(
        self,
        namespace_id: str,
        *,
        cursor: str | None = None,
        limit: int | None = None,
        state: Literal["ACTIVE", "PAUSED"] | None = None,
        origin: Literal["API", "TRIGGER"] | None = None,
        workflow_id: str | None = None,
        name: str | None = None,
        traceparent: str | None = None,
    ) -> Page[Schedule]:
        """One page of schedules. ``name`` selects the names that start with that text."""
        request = HttpRequest(
            "GET",
            self._path(namespace_id),
            query={
                "cursor": cursor,
                "limit": limit,
                "state": state,
                "origin": origin,
                "name": name,
                "workflowId": None if workflow_id is None else id_of(workflow_id, "a workflow id"),
            },
            traceparent=self._trace(traceparent),
        )
        return _page((await self._http.request(request)).body, "a page of schedules", to_schedule)

    async def list_all(
        self,
        namespace_id: str,
        *,
        limit: int | None = None,
        state: Literal["ACTIVE", "PAUSED"] | None = None,
        origin: Literal["API", "TRIGGER"] | None = None,
        workflow_id: str | None = None,
        name: str | None = None,
        traceparent: str | None = None,
    ) -> AsyncIterator[Schedule]:
        """Every schedule the filter selects, following the opaque cursors."""
        cursor: str | None = None
        while True:
            page = await self.list(
                namespace_id,
                cursor=cursor,
                limit=limit,
                state=state,
                origin=origin,
                workflow_id=workflow_id,
                name=name,
                traceparent=traceparent,
            )
            for item in page.items:
                yield item
            cursor = page.next_cursor
            if not cursor:
                return

    async def update(
        self,
        namespace_id: str,
        schedule_id: str,
        write: ScheduleWrite,
        expected_revision: int,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> Schedule:
        """Replaces the spec, the policies and the action as a new revision.

        A stale ``expected_revision`` is ``CONFLICT`` with the reason ``REVISION_CONFLICT``.
        """
        path = self._path(namespace_id, schedule_id)
        positive_int(expected_revision, "expected_revision")
        request = HttpRequest(
            "PUT",
            path,
            body={**_body(write, ScheduleWrite, "a schedule"), "expectedRevision": expected_revision},
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_schedule((await self._http.request(request)).body)

    async def delete(
        self,
        namespace_id: str,
        schedule_id: str,
        expected_revision: int,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> None:
        path = self._path(namespace_id, schedule_id)
        positive_int(expected_revision, "expected_revision")
        await self._http.request(
            HttpRequest(
                "DELETE",
                path,
                query={"expectedRevision": expected_revision},
                idempotency_key=idempotency_key_of(idempotency_key),
                traceparent=self._trace(traceparent),
            )
        )

    async def _note(
        self, namespace_id: str, schedule_id: str, tail: str, note: str, key: str | None, traceparent: str | None
    ) -> Schedule:
        path = self._path(namespace_id, schedule_id, tail)
        if not isinstance(note, str) or note == "":
            raise invalid("a note is a non-empty text")
        request = HttpRequest(
            "POST",
            path,
            body={"note": note},
            idempotency_key=idempotency_key_of(key),
            traceparent=self._trace(traceparent),
        )
        return to_schedule((await self._http.request(request)).body)

    async def pause(
        self,
        namespace_id: str,
        schedule_id: str,
        note: str,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> Schedule:
        return await self._note(namespace_id, schedule_id, "/pause", note, idempotency_key, traceparent)

    async def resume(
        self,
        namespace_id: str,
        schedule_id: str,
        note: str,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> Schedule:
        return await self._note(namespace_id, schedule_id, "/resume", note, idempotency_key, traceparent)

    async def trigger(
        self,
        namespace_id: str,
        schedule_id: str,
        *,
        overlap: OverlapPolicy | None = None,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> ScheduleActionRecord:
        """Acts on the schedule now, also while it is paused. The answer says what became of the action."""
        body: dict[str, Any] = {}
        if overlap is not None:
            body["overlap"] = overlap
        request = HttpRequest(
            "POST",
            self._path(namespace_id, schedule_id, "/trigger"),
            body=body,
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_schedule_action_record((await self._http.request(request)).body)

    async def backfill(
        self,
        namespace_id: str,
        schedule_id: str,
        start: datetime | str,
        end: datetime | str,
        *,
        overlap: OverlapPolicy | None = None,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> ScheduleBackfill:
        """Acts on every nominal time of the past range ``[start, end)`` that has no action yet."""
        path = self._path(namespace_id, schedule_id, "/backfills")
        body: dict[str, Any] = {"start": _when(start, "start"), "end": _when(end, "end")}
        if overlap is not None:
            body["overlap"] = overlap
        request = HttpRequest(
            "POST",
            path,
            body=body,
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_schedule_backfill((await self._http.request(request)).body)

    async def cancel_backfill(
        self,
        namespace_id: str,
        schedule_id: str,
        backfill_id: str,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> ScheduleBackfill:
        tail = f"/backfills/{quote(id_of(backfill_id, 'a backfill id'), safe='')}/cancel"
        request = HttpRequest(
            "POST",
            self._path(namespace_id, schedule_id, tail),
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_schedule_backfill((await self._http.request(request)).body)

    async def list_backfills(
        self, namespace_id: str, schedule_id: str, *, traceparent: str | None = None
    ) -> tuple[ScheduleBackfill, ...]:
        """At most the latest 100 backfills of the schedule."""
        request = HttpRequest(
            "GET", self._path(namespace_id, schedule_id, "/backfills"), traceparent=self._trace(traceparent)
        )
        items = object_of((await self._http.request(request)).body, "a list of backfills").get("items")
        return tuple(to_schedule_backfill(item) for item in items) if isinstance(items, list) else ()

    async def list_actions(
        self,
        namespace_id: str,
        schedule_id: str,
        *,
        cursor: str | None = None,
        limit: int | None = None,
        outcome: str | None = None,
        traceparent: str | None = None,
    ) -> Page[ScheduleActionRecord]:
        """The decisions the schedule has made, newest first."""
        request = HttpRequest(
            "GET",
            self._path(namespace_id, schedule_id, "/actions"),
            query={"cursor": cursor, "limit": limit, "outcome": outcome},
            traceparent=self._trace(traceparent),
        )
        return _page((await self._http.request(request)).body, "a page of schedule actions", to_schedule_action_record)


# ---------------------------------------------------------------------------------------- business ids


@dataclass(frozen=True)
class BusinessIdBinding:
    business_id: str
    execution_id: str
    """The run that holds the identifier, or the initial run of the continuation chain that does."""
    current_run_id: str
    """The run that carries the work now."""
    open: bool
    last_status: str | None
    workflow_id: str
    bound_at: datetime
    reusable_after: datetime | None
    raw: Mapping[str, Any] = field(repr=False, compare=False)


class BusinessIds(_Group):
    """How a business identifier stands in a namespace."""

    async def get(self, namespace_id: str, business_id: str, *, traceparent: str | None = None) -> BusinessIdBinding:
        """An identifier nobody holds (or that cannot be one) is ``NOT_FOUND``.

        It is sent as given, percent-encoded: no trimming and no case folding. Only what cannot be a path segment
        (not 1 to 200 visible ASCII characters) is refused here, and so is ``.`` or ``..`` alone: a whole dot segment
        is a step in the path for every URL resolver (``%2E`` too), so the request would reach another route. An
        identifier that merely contains dots (``...``, ``a..b``) is valid.
        """
        if (
            not isinstance(business_id, str)
            or not _BUSINESS_ID_PATH.fullmatch(business_id)
            or business_id in (".", "..")
        ):
            raise invalid("a business identifier is 1 to 200 visible ASCII characters and not . or ..")
        path = f"/namespaces/{quote(namespace_id_of(namespace_id), safe='')}/business-ids/{quote(business_id, safe='')}"
        raw = object_of(
            (await self._http.request(HttpRequest("GET", path, traceparent=self._trace(traceparent)))).body,
            "a business id binding",
        )
        return BusinessIdBinding(
            business_id=str_of(raw, "businessId"),
            execution_id=str_of(raw, "executionId"),
            current_run_id=str_of(raw, "currentRunId"),
            open=raw.get("open") is True,
            last_status=opt_str(raw, "lastStatus"),
            workflow_id=str_of(raw, "workflowId"),
            bound_at=instant_of(raw, "boundAt"),
            reusable_after=_opt_instant(raw, "reusableAfter"),
            raw=raw,
        )


# ---------------------------------------------------------------------------------------------- batch


@dataclass(frozen=True)
class BatchFilter:
    workflow_id: str | None = None
    version_id: str | None = None
    status: Sequence[str] | None = None
    origin: str | None = None
    schedule_id: str | None = None
    created_after: datetime | str | None = field(default=None, metadata=_INSTANT)
    created_before: datetime | str | None = field(default=None, metadata=_INSTANT)
    business_id_prefix: str | None = None


@dataclass(frozen=True)
class BatchSelector:
    """Which runs a batch is about: a ``filter``, explicit ``run_ids`` or a ``query``. The server judges the choice."""

    filter: BatchFilter | None = None
    run_ids: Sequence[str] | None = None
    query: str | None = None


@dataclass(frozen=True)
class BatchSignal:
    name: str
    payload: Mapping[str, Any]


@dataclass(frozen=True)
class BatchUpdate:
    name: str
    args: Any = None


@dataclass(frozen=True)
class BatchRequest:
    selector: BatchSelector
    action: str
    """``CANCEL``, ``SIGNAL``, ``RERUN``, ``RETRY_STEP``, ``PAUSE``, ``RESUME``...: the set only grows."""
    reason: str
    signal: BatchSignal | None = None
    update: BatchUpdate | None = None
    step_id: str | None = None
    business_id: KeepOrDrop | None = None
    max_items: int | None = None
    items_per_second: int | float | None = None


@dataclass(frozen=True)
class BatchPreview:
    matched: int
    capped: bool
    sample: tuple[str, ...]
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class BatchCounts:
    selected: int | float
    succeeded: int | float
    skipped: int | float
    failed: int | float
    cancelled: int | float
    deferred: int | float


@dataclass(frozen=True)
class BatchJob:
    batch_id: str
    action: str
    state: str
    failure_reason: str | None
    created_by: str
    has_payload: bool
    max_items: int | float | None
    items_per_second: int | float | None
    counts: BatchCounts
    created_at: datetime
    started_at: datetime | None
    finished_at: datetime | None
    expires_at: datetime | None
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class BatchItem:
    ordinal: int
    run_id: str
    state: str
    """``SUCCEEDED``, ``SKIPPED``...: another string is kept as it came."""
    reason: str | None
    error_code: str | None
    attempts: int
    new_run_id: str | None
    raw: Mapping[str, Any] = field(repr=False, compare=False)


def _count(counts: Mapping[str, Any], key: str) -> int | float:
    value = opt_number(counts, key)
    return value if value is not None else 0


def to_batch_job(value: Any) -> BatchJob:
    raw = object_of(value, "a batch job")
    held = raw.get("counts")
    counts = held if isinstance(held, dict) else {}
    return BatchJob(
        batch_id=str_of(raw, "batchId"),
        action=str_of(raw, "action"),
        state=str_of(raw, "state"),
        failure_reason=opt_str(raw, "failureReason"),
        created_by=str_of(raw, "createdBy"),
        has_payload=raw.get("hasPayload") is True,
        max_items=opt_number(raw, "maxItems"),
        items_per_second=opt_number(raw, "itemsPerSecond"),
        counts=BatchCounts(
            selected=_count(counts, "selected"),
            succeeded=_count(counts, "succeeded"),
            skipped=_count(counts, "skipped"),
            failed=_count(counts, "failed"),
            cancelled=_count(counts, "cancelled"),
            deferred=_count(counts, "deferred"),
        ),
        created_at=instant_of(raw, "createdAt"),
        started_at=_opt_instant(raw, "startedAt"),
        finished_at=_opt_instant(raw, "finishedAt"),
        expires_at=_opt_instant(raw, "expiresAt"),
        raw=raw,
    )


def to_batch_item(value: Any) -> BatchItem:
    raw = object_of(value, "a batch item")
    return BatchItem(
        ordinal=revision(raw, "ordinal", 0),
        run_id=str_of(raw, "runId"),
        state=str_of(raw, "state"),
        reason=opt_str(raw, "reason"),
        error_code=opt_str(raw, "errorCode"),
        attempts=int_of(raw, "attempts", 0),
        new_run_id=opt_str(raw, "newRunId"),
        raw=raw,
    )


class BatchOperations(_Group):
    """Batch jobs over many runs: preview, create, inspect, cancel and list the items."""

    @staticmethod
    def _path(namespace_id: str, tail: str = "") -> str:
        return f"/namespaces/{quote(namespace_id_of(namespace_id), safe='')}/batch-operations{tail}"

    async def preview(
        self,
        namespace_id: str,
        selector: BatchSelector,
        max_items: int | None = None,
        *,
        traceparent: str | None = None,
    ) -> BatchPreview:
        """Counts what a selector matches, up to ``max_items``; changes nothing."""
        path = self._path(namespace_id, "/preview")
        body: dict[str, Any] = {"selector": _body(selector, BatchSelector, "a selector")}
        if max_items is not None:
            body["maxItems"] = max_items
        request = HttpRequest("POST", path, body=body, traceparent=self._trace(traceparent))
        raw = object_of((await self._http.request(request)).body, "a batch preview")
        return BatchPreview(
            matched=revision(raw, "matched", 0),
            capped=raw.get("capped") is True,
            sample=_strings(raw.get("sample")),
            raw=raw,
        )

    async def create(
        self,
        namespace_id: str,
        request: BatchRequest,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> BatchJob:
        """Creates a durable job. A request sent again with the same key and body is answered with the first job."""
        path = self._path(namespace_id)
        call = HttpRequest(
            "POST",
            path,
            body=_body(request, BatchRequest, "a batch request"),
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_batch_job((await self._http.request(call)).body)

    async def get(self, namespace_id: str, batch_id: str, *, traceparent: str | None = None) -> BatchJob:
        path = self._path(namespace_id, f"/{quote(id_of(batch_id, 'a batch id'), safe='')}")
        return to_batch_job(
            (await self._http.request(HttpRequest("GET", path, traceparent=self._trace(traceparent)))).body
        )

    async def list(
        self,
        namespace_id: str,
        *,
        cursor: str | None = None,
        limit: int | None = None,
        state: str | None = None,
        traceparent: str | None = None,
    ) -> Page[BatchJob]:
        request = HttpRequest(
            "GET",
            self._path(namespace_id),
            query={"cursor": cursor, "limit": limit, "state": state},
            traceparent=self._trace(traceparent),
        )
        return _page((await self._http.request(request)).body, "a page of batch jobs", to_batch_job)

    async def cancel(
        self,
        namespace_id: str,
        batch_id: str,
        reason: str,
        *,
        idempotency_key: str | None = None,
        traceparent: str | None = None,
    ) -> BatchJob:
        """Stops admitting items; what was applied stays applied. A job that already ended is ``CONFLICT``."""
        path = self._path(namespace_id, f"/{quote(id_of(batch_id, 'a batch id'), safe='')}/cancel")
        request = HttpRequest(
            "POST",
            path,
            body={"reason": reason},
            idempotency_key=idempotency_key_of(idempotency_key),
            traceparent=self._trace(traceparent),
        )
        return to_batch_job((await self._http.request(request)).body)

    async def list_items(
        self,
        namespace_id: str,
        batch_id: str,
        *,
        cursor: str | None = None,
        limit: int | None = None,
        state: str | None = None,
        traceparent: str | None = None,
    ) -> Page[BatchItem]:
        request = HttpRequest(
            "GET",
            self._path(namespace_id, f"/{quote(id_of(batch_id, 'a batch id'), safe='')}/items"),
            query={"cursor": cursor, "limit": limit, "state": state},
            traceparent=self._trace(traceparent),
        )
        return _page((await self._http.request(request)).body, "a page of batch items", to_batch_item)


# --------------------------------------------------------------------------------------------- with start


@dataclass(frozen=True)
class SignalWithStartResult:
    run_id: str
    started: bool
    """Whether this request created the run."""
    delivered_to: tuple[str, ...]
    """The nodes the signal woke."""
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class UpdateWithStartReceipt:
    run_id: str
    operation: str
    update_key: str
    status: str
    result: Any
    before_state_revision: int
    after_state_revision: int
    run_version: int
    replayed: bool
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class UpdateWithStartUpdate:
    outcome: str
    """``APPLIED`` or ``REJECTED``: a refusal of the validator or a stale revision is an answer, not an error."""
    receipt: UpdateWithStartReceipt | None
    reason: str | None


@dataclass(frozen=True)
class UpdateWithStartResult:
    run_id: str
    started: bool
    update: UpdateWithStartUpdate
    raw: Mapping[str, Any] = field(repr=False, compare=False)


def to_signal_with_start(value: Any) -> SignalWithStartResult:
    raw = object_of(value, "a signal-with-start result")
    return SignalWithStartResult(
        run_id=str_of(raw, "runId"),
        started=raw.get("started") is True,
        delivered_to=_strings(raw.get("deliveredTo")),
        raw=raw,
    )


def to_update_with_start(value: Any) -> UpdateWithStartResult:
    raw = object_of(value, "an update-with-start result")
    update = object_of(raw.get("update"), "the update of the result")
    receipt: UpdateWithStartReceipt | None = None
    held = update.get("receipt")
    if held is not None:
        r = object_of(held, "an update receipt")
        receipt = UpdateWithStartReceipt(
            run_id=str_of(r, "runId"),
            operation=str_of(r, "operation"),
            update_key=str_of(r, "updateKey"),
            status=str_of(r, "status"),
            result=r.get("result"),
            before_state_revision=revision(r, "beforeStateRevision", 0),
            after_state_revision=revision(r, "afterStateRevision", 0),
            run_version=revision(r, "runVersion", 0),
            replayed=r.get("replayed") is True,
            raw=r,
        )
    return UpdateWithStartResult(
        run_id=str_of(raw, "runId"),
        started=raw.get("started") is True,
        update=UpdateWithStartUpdate(str_of(update, "outcome"), receipt, opt_str(update, "reason")),
        raw=raw,
    )


# ------------------------------------------------------------------------------------------------ reset


@dataclass(frozen=True)
class ResetPoint:
    sequence: int
    kind: str
    """``RUN_STARTED``, ``STEP_COMPLETED``...: the set only grows."""
    node: str | None
    occurred_at: datetime
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class RestoredStep:
    node: str
    kind: str


@dataclass(frozen=True)
class ReexecutedStep:
    node: str
    completed_in_source: bool
    markers: tuple[str, ...]


@dataclass(frozen=True)
class ResetPlan:
    sequence: int
    restored: tuple[RestoredStep, ...]
    reexecuted: tuple[ReexecutedStep, ...]
    reapply: tuple[Any, ...]
    requires_acknowledgement: bool
    prefix_events: int
    raw: Mapping[str, Any] = field(repr=False, compare=False)


@dataclass(frozen=True)
class Reset:
    reset_id: str
    source_run_id: str
    sequence: int
    state: str
    failure_reason: str | None
    new_run_id: str | None
    reapply_report: tuple[Any, ...]
    created_at: datetime | None
    raw: Mapping[str, Any] = field(repr=False, compare=False)


def to_reset_point(value: Any) -> ResetPoint:
    raw = object_of(value, "a reset point")
    return ResetPoint(
        sequence=revision(raw, "sequence", 1),
        kind=str_of(raw, "kind"),
        node=opt_str(raw, "node"),
        occurred_at=instant_of(raw, "occurredAt"),
        raw=raw,
    )


def to_reset_plan(value: Any) -> ResetPlan:
    raw = object_of(value, "a reset plan")
    restored = raw.get("restored")
    reexecuted = raw.get("reexecuted")
    reapply = raw.get("reapply")
    return ResetPlan(
        sequence=revision(raw, "sequence", 1),
        restored=tuple(
            RestoredStep(str_of(r, "node"), str_of(r, "kind"))
            for r in (object_of(item, "a restored step") for item in (restored if isinstance(restored, list) else ()))
        ),
        reexecuted=tuple(
            ReexecutedStep(str_of(r, "node"), r.get("completedInSource") is True, _strings(r.get("markers")))
            for r in (
                object_of(item, "a re-executed step") for item in (reexecuted if isinstance(reexecuted, list) else ())
            )
        ),
        reapply=tuple(reapply) if isinstance(reapply, list) else (),
        requires_acknowledgement=raw.get("requiresAcknowledgement") is True,
        prefix_events=revision(raw, "prefixEvents", 1),
        raw=raw,
    )


def to_reset(value: Any) -> Reset:
    raw = object_of(value, "a reset")
    report = raw.get("reapplyReport")
    return Reset(
        reset_id=str_of(raw, "resetId"),
        source_run_id=str_of(raw, "sourceRunId"),
        sequence=revision(raw, "sequence", 1),
        state=str_of(raw, "state"),
        failure_reason=opt_str(raw, "failureReason"),
        new_run_id=opt_str(raw, "newRunId"),
        reapply_report=tuple(report) if isinstance(report, list) else (),
        created_at=_opt_instant(raw, "createdAt"),
        raw=raw,
    )
