"""Step-log capture (design 041, ``sdks/SPEC.md`` section 6.9).

What an activity writes to ``ctx.log`` is numbered, redacted with the process's secret set, cut to the line ceilings,
buffered per attempt, sent through ``AppendStepLogs`` while the attempt runs and, for its last records, carried on its
report. A logger call never waits for the network and never raises into the activity: a failure of capture costs the
record, which is counted (Requirement 041-005). A call may come from the event loop or from the thread of a blocking
handler; the state of an attempt is behind a lock that is never held while code of the activity runs (formatting a
value may log again), and every send runs on the event loop.
"""

from __future__ import annotations

import asyncio
import contextlib
import copy
import dataclasses
import json
import logging
import math
import random
import re
import sys
import threading
import time
import traceback
from collections.abc import Awaitable, Callable, Iterable, Mapping
from dataclasses import dataclass, field
from types import FrameType
from typing import Any, Literal, NoReturn, Protocol

from .._json import _es_number
from .._proto import worker_pb2 as pb
from ..config import Secret, log

DropReason = Literal[
    "report_deadline", "buffer_full", "send_failed", "after_ceiling", "process_bound", "secret_set_full"
]
"""Why the SDK dropped a record (data-model.md section 7): counted locally, never sent."""

# ---- the constants of section 6.9 (Requirement 041-015; never sent by the server) ---------------------------------

ATTEMPT_RECORDS = 1000
ATTEMPT_BYTES = 256 * 1024
PROCESS_BYTES = 16 * 1024 * 1024
FLUSH_RECORDS = 256
FLUSH_SECONDS = 1.0
BATCH_RECORDS = 500
BATCH_BYTES = 192 * 1024
BATCH_AGE_SECONDS = 10.0
RETRY_DELAYS = (0.2, 1.0)
CARRY_RECORDS = 64
CARRY_BYTES = 16 * 1024
REPORT_WAIT_SECONDS = 2.0
SHUTDOWN_SEND_SECONDS = 1.5
MESSAGE_BYTES = 8192
FIELD_KEYS = 32
FIELD_BYTES = 2048
SECRET_VALUES = 1024
SECRET_BYTES = 256 * 1024
SECRET_MIN_BYTES = 6
REDACTED = "[REDACTED]"
LONGEST_SESSION_TOKEN = 128
"""A batch's bound is counted with a session token of its longest, so that a resend under a new token stays within."""
SESSION_STOPPED_LINE = "the platform stopped step-log capture for this session"
_INT64_MIN = -(2**63)
_INT64_MAX = 2**63 - 1
_COPY_DEPTH = 64


def wire_level(level: int) -> int:
    """A ``logging`` level onto the five of the wire: below 10 trace, 10 to 19 debug, 20 to 29 info, 30 to 39 warn, 40
    and above error (``CRITICAL`` included)."""
    if level < logging.DEBUG:
        return pb.STEP_LOG_LEVEL_TRACE
    if level < logging.INFO:
        return pb.STEP_LOG_LEVEL_DEBUG
    if level < logging.WARNING:
        return pb.STEP_LOG_LEVEL_INFO
    if level < logging.ERROR:
        return pb.STEP_LOG_LEVEL_WARN
    return pb.STEP_LOG_LEVEL_ERROR


# ---- protobuf sizes: a record's own size is ByteSize(); these frame it -------------------------------------------


def varint_size(value: int) -> int:
    """The bytes of a protobuf varint (a negative number is sign-extended to ten)."""
    if value < 0:
        return 10
    size = 1
    while value >= 0x80:
        value >>= 7
        size += 1
    return size


def delimited(size: int) -> int:
    """The bytes of one length-delimited field of ``size`` bytes, its one-byte tag included."""
    return 1 + varint_size(size) + size


def carry_size(record_sizes: Iterable[int], highest: int) -> int:
    """The encoded size of a ``StepLogCarry`` holding records of these sizes."""
    size = 1 + varint_size(highest) if highest else 0
    return size + sum(delimited(record) for record in record_sizes)


@dataclass(frozen=True)
class AttemptIdentity:
    """The attempt as ``Heartbeat`` names it; the lease token behind a ``Secret``."""

    task_id: str
    run_id: str
    step_id: str
    attempt: int
    generation: int
    lease_token: Secret = field(repr=False)
    instance: pb.ExecutionInstanceIdentity | None = field(default=None, repr=False)


def request_of(
    identity: AttemptIdentity, worker_id: str, session_token: str, records: Iterable[pb.StepLogRecord]
) -> pb.AppendStepLogsRequest:
    """One ``AppendStepLogsRequest`` of the attempt, with the session the host holds now."""
    request = pb.AppendStepLogsRequest(
        worker_id=worker_id,
        session_token=session_token,
        task_id=identity.task_id,
        generation=identity.generation,
        lease_token=identity.lease_token.reveal(),
        run_id=identity.run_id,
        step_id=identity.step_id,
        attempt=identity.attempt,
        records=records,
    )
    if identity.instance is not None:
        request.instance.CopyFrom(identity.instance)
    return request


# ---- text: well formed, cut at a character boundary ---------------------------------------------------------------

_ILL_FORMED = re.compile("[\x00\ud800-\udfff]")
_SURROGATE = re.compile("[\ud800-\udfff]")


def well_formed(text: str) -> str:
    """NUL and every surrogate become U+FFFD (step 5): what is sent is valid UTF-8 without a NUL. A Python string never
    pairs two surrogates into one character, so every surrogate in it is lone."""
    return _ILL_FORMED.sub("�", text)


def cut_utf8(text: str, limit: int) -> tuple[str, bool]:
    """The longest prefix of a well-formed ``text`` of at most ``limit`` UTF-8 bytes that ends at a character boundary,
    and whether it cut."""
    if len(text) * 4 <= limit:
        return text, False
    encoded = text.encode("utf-8")
    if len(encoded) <= limit:
        return text, False
    # Decoding drops only the character the cut split, which is incomplete at the end.
    return encoded[:limit].decode("utf-8", "ignore"), True


def _utf8_order(key: str) -> bytes:
    """The UTF-8 bytes a key is ordered by, a surrogate counted as the U+FFFD it becomes."""
    return _SURROGATE.sub("�", key).encode("utf-8")


# ---- the secret set and its redaction ----------------------------------------------------------------------------


class Redactor(Protocol):
    def redact(self, text: str) -> str: ...


class _Prefilter:
    """Whether any value of the set occurs in a text, in one pass over a sample of the text's q-grams.

    Every value is at least ``shortest`` characters long; every q-gram of every value is indexed; the text is sampled at
    every ``stride``-th position, ``stride = shortest - q + 1``, so an occurrence of any value covers a sampled q-gram
    whole. The sample is a necessary condition: when no sampled q-gram is indexed, no value occurs, and the text is the
    result of the rule unchanged; when one is, the rule is applied in full. A per-character automaton written in Python
    scans an 8 KiB text in about 0.45 ms; this sample does it in about 0.03 ms.
    """

    __slots__ = ("grams", "ordered", "q", "shortest", "stride")

    def __init__(self, values: tuple[str, ...]) -> None:
        shortest = min(len(value) for value in values)
        q = min(shortest, max(6, (shortest + 1) // 2))
        self.shortest = shortest
        self.q = q
        self.stride = shortest - q + 1
        self.grams = frozenset(value[i : i + q] for value in values for i in range(len(value) - q + 1))
        # The longest first by UTF-8 bytes; equal lengths keep the order they joined the set (a stable sort).
        self.ordered = tuple(sorted(values, key=lambda value: -len(value.encode("utf-8"))))

    def occurs_in(self, text: str) -> bool:
        if len(text) < self.shortest:
            return False
        q = self.q
        return not self.grams.isdisjoint([text[i : i + q] for i in range(0, len(text) - q + 1, self.stride)])


class SecretSet:
    """Values ``ResolveSecrets`` returned (``sdks/SPEC.md`` section 6.7): distinct values of at least 6 UTF-8 bytes, at
    most 1,024 of them or 256 KiB. Never printed, pickled or copied. The process's own set is not reachable from this
    module: only ``add_process_secrets`` adds to it."""

    __slots__ = ("_bytes", "_filter", "_full", "_known", "_lock", "_values")

    def __init__(self) -> None:
        self._values: list[str] = []
        self._known: set[str] = set()
        self._bytes = 0
        self._full = False
        self._filter: _Prefilter | None = None
        self._lock = threading.Lock()

    @property
    def full(self) -> bool:
        return self._full

    def add(self, values: Iterable[bytes]) -> bool:
        """Adds what one resolution returned; True when the set stopped capture with it (once)."""
        stopped = False
        with self._lock:
            changed = False
            for raw in values:
                value = bytes(raw)
                try:
                    text = value.decode("utf-8")
                except UnicodeDecodeError:
                    # Not UTF-8: the text an activity can log is its lenient decoding (section 6.7).
                    text = value.decode("utf-8", "replace")
                size = len(text.encode("utf-8"))
                if size < SECRET_MIN_BYTES or text in self._known:
                    continue
                if self._full or len(self._values) + 1 > SECRET_VALUES or self._bytes + size > SECRET_BYTES:
                    if not self._full:
                        stopped = True
                    self._full = True
                    continue
                self._values.append(text)
                self._known.add(text)
                self._bytes += size
                changed = True
            if changed:
                self._filter = _Prefilter(tuple(self._values))
        return stopped

    def redact(self, text: str) -> str:
        """The rule of ``sagat_domain::scripts::redact`` without its shapes, equal to it on every text (section 6.9):
        every value, the longest first, replaced by ``[REDACTED]`` in the text the previous replacement left."""
        prefilter = self._filter
        if prefilter is None or not prefilter.occurs_in(text):
            return text
        for value in prefilter.ordered:
            if value in text:
                text = text.replace(value, REDACTED)
        return text

    def __repr__(self) -> str:
        return f"SecretSet({len(self._values)} values)"

    __str__ = __repr__

    def __format__(self, format_spec: str) -> str:
        return repr(self)

    def __reduce__(self) -> NoReturn:
        raise TypeError("a secret set is not pickled")

    def __copy__(self) -> NoReturn:
        raise TypeError("a secret set is not copied")

    def __deepcopy__(self, memo: dict[int, Any]) -> NoReturn:
        raise TypeError("a secret set is not copied")


# ---- formatting one call -----------------------------------------------------------------------------------------


class _NotEncoded(Exception):
    """A value the compact JSON text cannot hold: it is captured as its string form."""


def _bytes_text(value: object) -> str | None:
    """A byte buffer as its UTF-8 text, each invalid sequence replaced by U+FFFD: never numbers or base64."""
    if isinstance(value, (bytes, bytearray, memoryview)):
        return bytes(value).decode("utf-8", "replace")
    return None


def _redacted_copy(value: Any, secrets: Redactor, as_json: bool, memo: dict[int, Any], depth: int) -> Any:
    """A copy of a value in which every string is redacted, so that an encoder's or ``repr``'s escaping of ``"``, ``\\``
    or a control character cannot hide a value from redaction (section 6.9).

    ``as_json`` is the copy a field value is encoded from: its keys are redacted too and a byte buffer, at any depth,
    is its redacted text. Otherwise it is the copy of a message's arguments, which keeps every type (a byte buffer stays
    bytes, a mapping's keys stay what ``%(name)s`` looks up)."""
    if depth > _COPY_DEPTH:
        raise _NotEncoded
    if isinstance(value, str):
        return secrets.redact(str.__str__(value))
    text = _bytes_text(value)
    if text is not None:
        if as_json:
            return secrets.redact(text)
        redacted = secrets.redact(text)
        return value if redacted == text else redacted.encode("utf-8")
    known = memo.get(id(value))
    if known is not None:
        return known
    kind = type(value)
    if kind is dict or (as_json and isinstance(value, dict)):
        out: dict[Any, Any] = {}
        memo[id(value)] = out
        for key, item in list(value.items()):
            new_key = secrets.redact(str.__str__(key)) if as_json and isinstance(key, str) else key
            out[new_key] = _redacted_copy(item, secrets, as_json, memo, depth + 1)
        return out
    if kind is list or (as_json and isinstance(value, list)):
        items: list[Any] = []
        memo[id(value)] = items
        items.extend(_redacted_copy(item, secrets, as_json, memo, depth + 1) for item in list(value))
        return items
    if kind is tuple or (as_json and isinstance(value, tuple)):
        return tuple(_redacted_copy(item, secrets, as_json, memo, depth + 1) for item in value)
    if kind in (set, frozenset):
        return kind(_redacted_copy(item, secrets, as_json, memo, depth + 1) for item in value)
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        duplicate = copy.copy(value)
        memo[id(value)] = duplicate
        for member in dataclasses.fields(value):
            object.__setattr__(
                duplicate, member.name, _redacted_copy(getattr(value, member.name), secrets, as_json, memo, depth + 1)
            )
        return duplicate
    return value


_JSON_KEYS: dict[Any, str] = {True: "true", False: "false", None: "null"}


def _json_text(value: Any, out: list[str], active: set[int]) -> None:
    """Compact JSON (no insignificant whitespace), numbers as ECMAScript writes them and a non-finite double as
    ``null``, as JavaScript's ``JSON.stringify`` writes the same value; any other type is not encoded."""
    if value is None or value is True or value is False:
        out.append(_JSON_KEYS[value])
    elif isinstance(value, str):
        out.append(json.dumps(value, ensure_ascii=False))
    elif isinstance(value, int):
        out.append(str(int(value)))
    elif isinstance(value, float):
        out.append(_es_number(value) if math.isfinite(value) else "null")
    elif isinstance(value, (list, tuple, dict)):
        if id(value) in active:
            raise _NotEncoded
        active.add(id(value))
        if isinstance(value, dict):
            out.append("{")
            for index, (key, item) in enumerate(value.items()):
                if index:
                    out.append(",")
                if isinstance(key, str):
                    name = key
                elif key is None or isinstance(key, bool):
                    name = _JSON_KEYS[key]
                elif isinstance(key, int):
                    name = str(int(key))
                elif isinstance(key, float) and math.isfinite(key):
                    name = _es_number(key)
                else:
                    raise _NotEncoded
                out.append(json.dumps(name, ensure_ascii=False))
                out.append(":")
                _json_text(item, out, active)
            out.append("}")
        else:
            out.append("[")
            for index, item in enumerate(value):
                if index:
                    out.append(",")
                _json_text(item, out, active)
            out.append("]")
        active.discard(id(value))
    else:
        raise _NotEncoded


def _other_text(value: Any, secrets: Redactor) -> str:
    """A value that is not a scalar: its compact JSON text where JSON holds it, else its string form, both made from
    the redacted copy (and redacted again with the string)."""
    try:
        duplicate = _redacted_copy(value, secrets, True, {}, 0)
    except Exception:
        duplicate = value
    try:
        out: list[str] = []
        _json_text(duplicate, out, set())
        return "".join(out)
    except Exception:
        pass
    try:
        return str(duplicate)
    except Exception:
        return object.__repr__(value)


def _field_value(value: Any, secrets: Redactor) -> tuple[str, Any] | None:
    """One field value as the wire carries it: the member of the ``StepLogField.value`` oneof and its value."""
    if value is None:
        return None
    if isinstance(value, bool):
        return "bool_value", value
    if isinstance(value, int):
        number = int(value)
        if _INT64_MIN <= number <= _INT64_MAX:
            return "int_value", number
        return "string_value", str(number)
    if isinstance(value, float):
        if math.isnan(value):
            return "string_value", "NaN"
        if math.isinf(value):
            return "string_value", "Infinity" if value > 0 else "-Infinity"
        return "double_value", float(value)
    if isinstance(value, str):
        return "string_value", str.__str__(value)
    text = _bytes_text(value)
    if text is not None:
        return "string_value", text
    return "string_value", _other_text(value, secrets)


def _json_string_size(text: str) -> int:
    """The UTF-8 bytes of a JSON string of data-model.md section 5: only ``"``, ``\\`` and U+0000 to U+001F escaped, the
    controls as ``\\u00XX``."""
    size = len(text.encode("utf-8")) + 2
    for char in text:
        if char in '"\\':
            size += 1
        elif char < " ":
            size += 5
    return size


def _json_value_size(member: str, value: Any) -> int:
    if member == "string_value":
        return _json_string_size(value)
    if member == "int_value":
        return len(str(value))
    if member == "double_value":
        return len(_es_number(value))
    return 4 if value else 5


def build_fields(given: Mapping[Any, Any] | None, secrets: Redactor) -> tuple[list[pb.StepLogField], bool]:
    """The fields of a call by data-model.md section 5: taken in the order of their original keys' UTF-8 bytes,
    redacted, keys made unique with ``~2``, ``~3``, then kept in key order up to 32 keys and 2,048 bytes of canonical
    JSON; and whether any was dropped."""
    if not given:
        return [], False
    taken: list[tuple[str, tuple[str, Any]]] = []
    for key, value in list(given.items()):
        try:
            name = key if isinstance(key, str) else str(key)
            converted = _field_value(value, secrets)
        except Exception:
            continue  # a value whose text cannot be made costs its field
        if converted is not None:
            taken.append((str.__str__(name), converted))
    taken.sort(key=lambda pair: _utf8_order(pair[0]))
    used: set[str] = set()
    made: list[tuple[str, str, Any]] = []
    for original, (member, value) in taken:
        # Redacted on the text as given, then made well formed (steps 4 and 5).
        redacted = well_formed(secrets.redact(original))
        key = redacted
        suffix = 2
        while key in used:
            key = f"{redacted}~{suffix}"
            suffix += 1
        used.add(key)
        if member == "string_value":
            value = well_formed(secrets.redact(value))
        elif member in ("int_value", "double_value"):
            text = str(value) if member == "int_value" else _es_number(value)
            if secrets.redact(text) != text:
                member, value = "string_value", REDACTED
        made.append((key, member, value))
    made.sort(key=lambda entry: entry[0].encode("utf-8"))
    kept: list[pb.StepLogField] = []
    size = 2  # the braces
    for key, member, value in made:
        entry = _json_string_size(key) + 1 + _json_value_size(member, value) + (1 if kept else 0)
        if len(kept) + 1 > FIELD_KEYS or size + entry > FIELD_BYTES:
            return kept, True
        field_message = pb.StepLogField(key=key)
        setattr(field_message, member, value)
        kept.append(field_message)
        size += entry
    return kept, False


def format_message(msg: object, args: tuple[Any, ...], secrets: Redactor) -> str:
    """The message as ``logging`` formats it for its handlers (``str(msg) % args``, a single non-empty mapping being
    the arguments), from arguments whose strings were redacted first; the unformatted text when formatting fails."""
    try:
        text = str(msg)
    except Exception:
        return ""
    if not args:
        return text
    arguments: Any = args
    if len(args) == 1 and isinstance(args[0], Mapping) and args[0]:
        arguments = args[0]
    with contextlib.suppress(Exception):
        arguments = _redacted_copy(arguments, secrets, False, {}, 0)
    try:
        return str(text % arguments)
    except Exception:
        return text


def exception_text(exc_info: Any, stack: FrameType | None) -> str:
    """The exception of a call as ``logging`` formats it: ``exc_info`` through ``traceback.format_exception``, then the
    stack of ``stack_info=True``."""
    parts: list[str] = []
    if exc_info:
        try:
            if isinstance(exc_info, BaseException):
                info: Any = (type(exc_info), exc_info, exc_info.__traceback__)
            elif isinstance(exc_info, tuple):
                info = exc_info
            else:
                info = sys.exc_info()
            text = "".join(traceback.format_exception(*info))
            parts.append(text[:-1] if text.endswith("\n") else text)
        except Exception:
            pass
    if stack is not None:
        try:
            text = "Stack (most recent call last):\n" + "".join(traceback.format_stack(stack))
            parts.append(text[:-1] if text.endswith("\n") else text)
        except Exception:
            pass
    return "\n".join(parts)


def build_record(
    sequence: int,
    epoch_ms: int,
    level: int,
    msg: object,
    args: tuple[Any, ...],
    extra: Mapping[Any, Any] | None,
    exception: str,
    secrets: Redactor,
) -> pb.StepLogRecord:
    """Steps 3 to 6 of a call: format, redact, make well formed, cut."""
    message, message_cut = cut_utf8(well_formed(secrets.redact(format_message(msg, args, secrets))), MESSAGE_BYTES)
    exception, exception_cut = cut_utf8(well_formed(secrets.redact(exception)), MESSAGE_BYTES)
    fields, fields_cut = build_fields(extra, secrets)
    return pb.StepLogRecord(
        sequence=sequence,
        worker_time_epoch_ms=epoch_ms,
        level=wire_level(level),  # type: ignore[arg-type]
        message=message,
        fields=fields,
        exception=exception,
        truncated=message_cut or exception_cut or fields_cut,
    )


# ---- what the whole process shares ---------------------------------------------------------------------------------


def _process_scope() -> tuple[
    Callable[[Iterable[bytes]], bool],
    Callable[[], bool],
    Callable[[int, int, int, object, tuple[Any, ...], Any, str], pb.StepLogRecord],
]:
    """The process's secret set, which lives for the process (section 6.7, decision D041-164), held by these closures
    only: the module exposes a way to add to it, whether it is full, and the capture of one record, never a reader of
    the set or a redaction of a text with it."""
    secrets = SecretSet()

    def add(values: Iterable[bytes]) -> bool:
        return secrets.add(values)

    def stopped() -> bool:
        return secrets.full

    def record(
        sequence: int, epoch_ms: int, level: int, msg: object, args: tuple[Any, ...], extra: Any, exception: str
    ) -> pb.StepLogRecord:
        given = extra if isinstance(extra, Mapping) else None
        return build_record(sequence, epoch_ms, level, msg, args, given, exception, secrets)

    return add, stopped, record


add_process_secrets, process_capture_stopped, _capture_record = _process_scope()
add_process_secrets.__doc__ = """Adds the values one resolution returned to the process's secret set, before the handler
can see them; True when the set stopped capture for the process with them (once)."""
process_capture_stopped.__doc__ = """Whether capture stopped for the process because its secret set is full."""


class _Held:
    """The encoded bytes of the records every attempt of the process holds, the batches in flight included."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self.value = 0

    def add(self, size: int) -> None:
        with self._lock:
            self.value += size


_HELD = _Held()
_WARNED: set[str] = set()
_WARNED_LOCK = threading.Lock()


def warn_once(logger: logging.Logger, line: str) -> None:
    """The one local line per process when the server stops capture or the secret set is full."""
    with _WARNED_LOCK:
        if line in _WARNED:
            return
        _WARNED.add(line)
    log(logger, logging.WARNING, line)


# ---- one attempt -------------------------------------------------------------------------------------------------


class CaptureSession:
    """The capture state a registration gives a session: shared by its attempts, stopped by ``STOP_CAPTURE``."""

    __slots__ = ("enabled", "stopped")

    def __init__(self, enabled: bool) -> None:
        self.enabled = enabled
        self.stopped = False


ABANDONED = "ABANDONED"
"""The status of a call the host gave up because it is stopping (its channel closed or replaced): not retried."""


@dataclass(frozen=True)
class Outcome:
    """The answer of one ``AppendStepLogs``, or the gRPC status of its failure."""

    answer: pb.AppendStepLogsResponse | None = None
    status: str | None = None
    retry_after_ms: int | None = None


@dataclass(frozen=True)
class CaptureHost:
    """What an attempt needs of its host."""

    session: Callable[[], tuple[str, str]]
    """The session id and token the host holds now."""
    append: Callable[[pb.AppendStepLogsRequest, float], Awaitable[Outcome]]
    """One ``AppendStepLogs`` with its own deadline in seconds; never raises."""
    timeout: float
    """The configured call timeout, in seconds."""
    dropped: Callable[[str, int], None]
    """The drops of the attempt, once per reason when it ends."""
    warn_once: Callable[[str], None]


@dataclass(eq=False)
class _Batch:
    held: list[tuple[pb.StepLogRecord, int]]
    done: asyncio.Future[None]
    first_sent: float
    abandoned: bool = False
    """Given up by the attempt (the report's wait ended, or the attempt ended without a report): its records are
    counted, and it is not sent again."""


_ON, _FINAL_ONLY, _STOPPED = 0, 1, 2


def _jittered(seconds: float) -> float:
    return seconds * (0.8 + random.random() * 0.4)


class AttemptCapture:
    """The capture of one attempt: its sequences, its buffer, its sender and, at its end, its carry."""

    def __init__(self, host: CaptureHost, session: CaptureSession, identity: AttemptIdentity, level: int) -> None:
        self._host = host
        self._session = session
        self._identity = identity
        self._level = wire_level(level)
        self._loop = asyncio.get_running_loop()
        self._lock = threading.Lock()
        self._state = _ON
        self._last = 0
        self._buffer: list[tuple[pb.StepLogRecord, int]] = []
        self._buffer_bytes = 0
        self._in_flight: _Batch | None = None
        self._batches_stopped = False
        self._ended = False
        self._ended_at = 0.0
        self._closed = False
        self._closing = asyncio.Event()
        self._tick: asyncio.TimerHandle | None = None
        self._ticking = False
        self._drops: dict[str, int] = {}
        self._sends: set[asyncio.Task[None]] = set()

    def __repr__(self) -> str:
        return f"AttemptCapture(task={self._identity.task_id!r})"

    @property
    def highest_sequence(self) -> int:
        """The last sequence taken: the carry's ``highest_sequence``."""
        return self._last

    def _count(self, reason: DropReason, records: int = 1) -> None:
        if records > 0:
            self._drops[reason] = self._drops.get(reason, 0) + records

    def _stopped(self) -> bool:
        return self._state == _STOPPED or self._session.stopped

    # ---- a logger call -----------------------------------------------------------------------------------------

    def log(
        self,
        level: int,
        msg: object,
        args: tuple[Any, ...],
        extra: Any,
        exc_info: Any,
        stack: FrameType | None,
    ) -> None:
        """A logger call (section 6.9, "A call, in order"). Never raises and never waits for the network."""
        try:
            if self._ended or not isinstance(level, int) or wire_level(level) < self._level:
                return
            with self._lock:
                if self._ended or self._closed or self._stopped():
                    return
                self._last += 1
                sequence = self._last
                epoch_ms = time.time_ns() // 1_000_000
                if self._state == _FINAL_ONLY:
                    self._count("after_ceiling")
                    return
                if process_capture_stopped():
                    self._count("secret_set_full")
                    return
                if len(self._buffer) >= ATTEMPT_RECORDS or self._buffer_bytes >= ATTEMPT_BYTES:
                    self._count("buffer_full")
                    return
                if _HELD.value >= PROCESS_BYTES:
                    self._count("process_bound")
                    return
            # Outside the lock: formatting runs code of the activity, which may log again.
            record = _capture_record(sequence, epoch_ms, level, msg, args, extra, exception_text(exc_info, stack))
            self._buffered(record, record.ByteSize())
        except Exception:
            pass  # a bug of the capture path costs the record, never the activity

    def _buffered(self, record: pb.StepLogRecord, size: int) -> None:
        """Step 7: the record joins the buffer in the order of its sequence, and the call that buffers the 256th record
        cuts the batch, in the call."""
        spawn: _Batch | None = None
        arm = False
        with self._lock:
            if self._closed:
                return
            if self._state == _FINAL_ONLY:
                self._count("after_ceiling")
                return
            if self._stopped():
                self._count("send_failed")
                return
            at = len(self._buffer)
            while at > 0 and self._buffer[at - 1][0].sequence > record.sequence:
                at -= 1
            self._buffer.insert(at, (record, size))
            self._buffer_bytes += size
            _HELD.add(size)
            if self._batches_stopped:
                self._keep_one_carry()
            if not self._ticking and not self._ended:
                self._ticking = arm = True
            if len(self._buffer) >= FLUSH_RECORDS and self._in_flight is None and not self._batches_stopped:
                if process_capture_stopped():
                    self._drop_buffer("secret_set_full")
                else:
                    spawn = self._in_flight = self._cut(len(self._buffer))
        if arm:
            self._loop.call_soon_threadsafe(self._arm_tick)
        if spawn is not None:
            self._loop.call_soon_threadsafe(self._spawn, spawn)

    # ---- sending -----------------------------------------------------------------------------------------------

    def _arm_tick(self) -> None:
        if not self._ended and not self._closed and self._tick is None:
            self._tick = self._loop.call_later(FLUSH_SECONDS, self._on_tick)

    def _on_tick(self) -> None:
        try:
            with self._lock:
                self._tick = None
                if self._closed or self._ended:
                    return
                self._tick = self._loop.call_later(FLUSH_SECONDS, self._on_tick)
                if (
                    self._in_flight is not None
                    or not self._buffer
                    or self._state != _ON
                    or self._stopped()
                    or self._batches_stopped
                ):
                    return
                if process_capture_stopped():
                    self._drop_buffer("secret_set_full")
                    return
                batch = self._in_flight = self._cut(len(self._buffer))
            self._spawn(batch)
        except Exception:
            pass  # the next tick tries again

    def _cut(self, limit: int) -> _Batch:
        """Takes the oldest of the first ``limit`` buffered records that one batch holds: at most 500, and at most 192
        KiB of encoded ``AppendStepLogsRequest``. The caller holds the lock."""
        worker_id, _ = self._host.session()
        size = request_of(self._identity, worker_id, "x" * LONGEST_SESSION_TOKEN, ()).ByteSize()
        count = 0
        while count < min(limit, BATCH_RECORDS):
            following = delimited(self._buffer[count][1])
            if count > 0 and size + following > BATCH_BYTES:
                break
            size += following
            count += 1
        held, self._buffer = self._buffer[:count], self._buffer[count:]
        self._buffer_bytes -= sum(one[1] for one in held)
        return _Batch(held, self._loop.create_future(), time.monotonic())

    def _spawn(self, batch: _Batch, once: bool = False, within: float | None = None) -> asyncio.Task[None]:
        task = self._loop.create_task(self._deliver(batch, once, self._host.timeout if within is None else within))
        self._sends.add(task)
        task.add_done_callback(self._sends.discard)
        return task

    async def _pause(self, seconds: float) -> bool:
        """Waits ``seconds``; False when the attempt closed first."""
        try:
            await asyncio.wait_for(self._closing.wait(), seconds)
        except asyncio.TimeoutError:
            return True
        return False

    async def _deliver(self, batch: _Batch, once: bool, within: float) -> None:
        """Sends one batch until an answer settles it (contracts.md section 4). ``once`` sends it a single time, as the
        shutdown does: an answer that would send it again drops it instead."""
        try:
            await self._send_until_settled(batch, once, within)
        except Exception:
            self._failed(batch)
        finally:
            self._finish(batch)

    async def _send_until_settled(self, batch: _Batch, once: bool, within: float) -> None:
        records = [record for record, _ in batch.held]
        worker_id, token = self._host.session()
        request = request_of(self._identity, worker_id, token, records)
        retries = 0
        session_resent = False
        while True:
            if batch.abandoned:
                return
            if process_capture_stopped():
                self._failed(batch, "secret_set_full")
                return
            if self._stopped():
                self._failed(batch)
                return
            left = BATCH_AGE_SECONDS - (time.monotonic() - batch.first_sent)
            if left <= 0:
                self._failed(batch)
                return
            outcome = await self._host.append(request, min(within, left))
            if once:
                # Sent once, with no resend: a batch the answer did not take is counted as not sent.
                if outcome.answer is None or outcome.answer.verdict != pb.STEP_LOG_VERDICT_CONTINUE:
                    self._failed(batch)
                return
            if batch.abandoned:
                return
            wait = 0.0
            answer = outcome.answer
            if answer is not None:
                verdict = answer.verdict
                if verdict == pb.STEP_LOG_VERDICT_CONTINUE:
                    return
                if verdict == pb.STEP_LOG_VERDICT_FINAL_ONLY:
                    self._final_only()
                    return
                if verdict == pb.STEP_LOG_VERDICT_STOP_ATTEMPT:
                    self._stop()
                    return
                if verdict == pb.STEP_LOG_VERDICT_STOP_CAPTURE:
                    now_id, now = self._host.session()
                    if answer.stop_reason == pb.STEP_LOG_STOP_REASON_SESSION and not session_resent and now != token:
                        session_resent, token = True, now
                        request = request_of(self._identity, now_id, now, records)
                        continue
                    self._stop_session()
                    return
                if verdict == pb.STEP_LOG_VERDICT_RETRY_LATER:
                    wait = min(5.0, max(0.05, answer.retry_after_ms / 1000))
                # An answer this SDK does not know is a failure of the call: it counts toward the two retries.
            else:
                status = outcome.status
                if status == "RESOURCE_EXHAUSTED":
                    after = 200 if outcome.retry_after_ms is None else outcome.retry_after_ms
                    wait = min(5.0, max(0.05, after / 1000))
                elif status in ("INVALID_ARGUMENT", ABANDONED):
                    # INVALID_ARGUMENT: the server counts the gap. ABANDONED: the host is stopping and closed the
                    # call; nothing is sent again (a real CANCELLED status is retried as any status not listed).
                    self._failed(batch)
                    return
                elif status in ("UNAUTHENTICATED", "PERMISSION_DENIED"):
                    self._stop_session()
                    return
                elif status == "UNIMPLEMENTED":
                    with self._lock:
                        self._batches_stopped = True
                        self._keep_one_carry()
                    self._failed(batch)
                    return
            if wait > 0:
                # RETRY_LATER and RESOURCE_EXHAUSTED: the same batch again, not counted as a retry.
                if not await self._pause(wait):
                    return
                continue
            # UNAVAILABLE, DEADLINE_EXCEEDED, a transport failure, any other status or verdict: two retries.
            if retries >= len(RETRY_DELAYS):
                self._failed(batch)
                return
            if not await self._pause(_jittered(RETRY_DELAYS[retries])):
                return
            retries += 1

    def _failed(self, batch: _Batch, reason: DropReason = "send_failed") -> None:
        """The batch is let go: its records are counted once, unless the attempt already gave it up and counted them."""
        with self._lock:
            if not batch.abandoned:
                batch.abandoned = True
                self._count(reason, len(batch.held))

    def _abandon(self, reason: DropReason) -> None:
        """Gives up the batch in flight, if any. The caller holds the lock."""
        batch = self._in_flight
        if batch is not None and not batch.abandoned:
            batch.abandoned = True
            self._count(reason, len(batch.held))

    def _finish(self, batch: _Batch) -> None:
        """A batch is settled: its bytes are released, and the answer cuts the next batch when 256 or more are
        buffered then."""
        following: _Batch | None = None
        with self._lock:
            _HELD.add(-sum(one[1] for one in batch.held))
            if self._in_flight is batch:
                self._in_flight = None
            if (
                not self._closed
                and not self._ended
                and self._state == _ON
                and not self._stopped()
                and not self._batches_stopped
                and len(self._buffer) >= FLUSH_RECORDS
                and self._in_flight is None
            ):
                if process_capture_stopped():
                    self._drop_buffer("secret_set_full")
                else:
                    following = self._in_flight = self._cut(len(self._buffer))
        if not batch.done.done():
            batch.done.set_result(None)
        if following is not None:
            self._spawn(following)
        self._emit_late_drops()

    def _emit_late_drops(self) -> None:
        """Drops counted after the attempt closed (a batch it gave up that then failed is not among them: it was
        counted when it was given up) are still told, once."""
        with self._lock:
            if not self._closed or not self._drops:
                return
            drops, self._drops = self._drops, {}
        for reason in sorted(drops):
            self._host.dropped(reason, drops[reason])

    def _drop_buffer(self, reason: DropReason) -> None:
        """The caller holds the lock."""
        self._count(reason, len(self._buffer))
        _HELD.add(-self._buffer_bytes)
        self._buffer, self._buffer_bytes = [], 0

    def _final_only(self) -> None:
        with self._lock:
            if self._state == _ON:
                self._state = _FINAL_ONLY
                self._drop_buffer("after_ceiling")

    def _stop(self) -> None:
        with self._lock:
            self._state = _STOPPED
            # Records the server will not take: counted as records whose sending failed.
            self._drop_buffer("send_failed")

    def _stop_session(self) -> None:
        self._session.stopped = True
        self._host.warn_once(SESSION_STOPPED_LINE)
        self._stop()

    def _keep_one_carry(self) -> None:
        """After ``UNIMPLEMENTED``: only the newest records that fit one carry are kept, the oldest dropped
        (``report_deadline``). The caller holds the lock."""
        beyond = len(self._buffer) - self._carry_fits()
        if beyond > 0:
            self._count("report_deadline", beyond)
            dropped = sum(one[1] for one in self._buffer[:beyond])
            _HELD.add(-dropped)
            self._buffer_bytes -= dropped
            self._buffer = self._buffer[beyond:]

    # ---- the end of the attempt ----------------------------------------------------------------------------------

    def end(self) -> None:
        """The handler ended: its logger captures nothing more. Called on the event loop."""
        with self._lock:
            if not self._ended:
                self._ended = True
                self._ended_at = self._loop.time()
            tick, self._tick = self._tick, None
        if tick is not None:
            tick.cancel()

    def _carry_fits(self) -> int:
        """How many of the newest buffered records fit one carry. The caller holds the lock."""
        size = 1 + varint_size(self._last)
        count = 0
        for _, record_size in reversed(self._buffer):
            if count >= CARRY_RECORDS:
                break
            following = delimited(record_size)
            if size + following > CARRY_BYTES:
                break
            size += following
            count += 1
        return count

    def _mark_only(self) -> pb.StepLogCarry:
        """The caller holds the lock."""
        self._drop_buffer("after_ceiling" if self._state == _FINAL_ONLY else "secret_set_full")
        return pb.StepLogCarry(highest_sequence=self._last)

    async def carry(self) -> pb.StepLogCarry | None:
        """The carry of the attempt's report (section 6.9, "The carry"), after at most 2 seconds from the handler's end
        of waiting for its batch in flight and sending what does not fit one carry; ``None`` when the attempt sends
        none. It closes the attempt and never raises."""
        self.end()
        try:
            return await self._carry()
        except Exception:
            return None
        finally:
            self._close()

    async def _carry(self) -> pb.StepLogCarry | None:
        deadline = self._ended_at + REPORT_WAIT_SECONDS
        while True:
            with self._lock:
                if self._closed or self._last == 0 or self._stopped():
                    return None
                if self._state == _FINAL_ONLY or process_capture_stopped():
                    return self._mark_only()
                batch = self._in_flight
                if batch is None:
                    fits = self._carry_fits()
                    if fits == len(self._buffer) or self._batches_stopped or self._loop.time() >= deadline:
                        break
                    batch = self._in_flight = self._cut(len(self._buffer) - fits)
                    self._spawn(batch)
            left = deadline - self._loop.time()
            if left > 0:
                _, late = await asyncio.wait({batch.done}, timeout=left)
                if not late:
                    continue
            # The 2 seconds ended with the batch on its way: it is given up, and an answer that comes later is ignored.
            with self._lock:
                self._abandon("report_deadline")
            break
        with self._lock:
            if self._stopped():
                return None
            if self._state == _FINAL_ONLY or process_capture_stopped():
                return self._mark_only()
            self._keep_one_carry()
            return pb.StepLogCarry(records=[record for record, _ in self._buffer], highest_sequence=self._last)

    def _close(self, reported: bool = True) -> None:
        """The attempt is over: nothing more is sent, its records are let go, and its drops are counted once. Without a
        report, what it still holds (its batch in flight included) is counted as not sent."""
        with self._lock:
            if self._closed:
                return
            if not reported:
                self._abandon("send_failed")
                self._count("send_failed", len(self._buffer))
            self._closed = True
            self._ended = True
            _HELD.add(-self._buffer_bytes)
            self._buffer, self._buffer_bytes = [], 0
            drops, self._drops = self._drops, {}
            tick, self._tick = self._tick, None
        if tick is not None:
            tick.cancel()
        self._closing.set()
        for reason in sorted(drops):
            self._host.dropped(reason, drops[reason])

    def discard(self) -> None:
        """The attempt makes no report (its lease was lost, or the host stopped): nothing more is sent."""
        self.end()
        self._close(reported=False)

    def shutdown_batch(self) -> _Batch | None:
        """The host stops after its grace with this attempt's handler still running: its oldest buffered records become
        its batch in flight, sent once with no resend; what does not fit stays buffered for a report it still makes.
        ``None`` when there is none to send, or when the handler ended (the attempt waits before its report and is
        left to it) or a batch is in flight."""
        with self._lock:
            if (
                self._ended
                or self._closed
                or self._stopped()
                or self._state != _ON
                or self._batches_stopped
                or self._in_flight is not None
                or not self._buffer
            ):
                return None
            if process_capture_stopped():
                self._drop_buffer("secret_set_full")
                return None
            batch = self._in_flight = self._cut(len(self._buffer))
            return batch

    async def send_once(self, batch: _Batch, within: float) -> None:
        """Sends the shutdown batch once, with its own deadline."""
        await self._spawn(batch, once=True, within=within)
