"""What an activity handler sees and how it answers (``sdks/SPEC.md`` sections 6.3 to 6.7)."""

from __future__ import annotations

import asyncio
import logging
import threading
from collections.abc import AsyncIterable, AsyncIterator, Awaitable, Callable, Iterable, Mapping
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Literal, TypeVar

from ..errors import SagatError

TaskKind = Literal["FORWARD", "COMPENSATION", "RECONCILIATION"]
CancelReason = Literal["run-cancelled", "lease-lost", "shutdown"]

T = TypeVar("T")


@dataclass(frozen=True)
class ExecutionScope:
    """One repeated scope that addresses a runtime activation of a definition node."""

    scope_node_id: str
    iteration_index: int


@dataclass(frozen=True)
class ExecutionInstanceIdentity:
    """The run-local identity supplied for a task by worker protocol 1.5."""

    instance_path: tuple[ExecutionScope, ...]
    definition_node_id: str


@dataclass(frozen=True)
class BlobDescriptor:
    """Verified identity of an immutable protocol 1.3 blob."""

    id: str
    size: int
    sha256: str
    content_type: str


class BlobOutput:
    """Explicitly selects a descriptor as the step output; ordinary dictionaries remain JSON."""

    __slots__ = ("descriptor",)

    def __init__(self, descriptor: BlobDescriptor) -> None:
        self.descriptor = descriptor


class ActivityFailure(Exception):
    """How an activity fails on purpose.

    ``code`` is a stable machine code (``[A-Z0-9_]{1,64}``; anything else is reported as ``WORKER_ERROR``);
    ``message`` is recorded with the step, so it must not carry a payload or a secret.
    """

    code: str
    message: str
    is_retryable: bool

    def __init__(self, code: str, message: str, retryable: bool = True) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.is_retryable = retryable

    def __reduce__(self) -> tuple[Any, tuple[Any, ...]]:
        return (type(self), (self.code, self.message, self.is_retryable))

    @classmethod
    def retryable(cls, code: str, message: str) -> ActivityFailure:
        """The step may be tried again as its node's retry policy says."""
        return cls(code, message, True)

    @classmethod
    def permanent(cls, code: str, message: str) -> ActivityFailure:
        """The step is failed for good: the policy goes to its error branch or its saga compensates."""
        return cls(code, message, False)

    @classmethod
    def outcome_unknown(cls, message: str) -> ActivityFailure:
        """The activity sent a request and cannot tell whether it was applied (a timeout after connecting, an
        answer that never arrived). The platform then reconciles the step, when it has a ``reconcile_with``,
        instead of retrying it blindly."""
        return cls("OUTCOME_UNKNOWN", message, False)


class _Unset:
    def __repr__(self) -> str:
        return "UNSET"


_UNSET: Any = _Unset()


class Verdict:
    """What a reconciler says about the step it looks up (``sdks/SPEC.md`` section 6.6)."""

    __slots__ = ("effect", "output")

    effect: Literal["applied", "not_applied", "unknown"]
    output: Any

    def __init__(self, effect: Literal["applied", "not_applied", "unknown"], output: Any = _UNSET) -> None:
        self.effect = effect
        self.output = output

    @classmethod
    def applied(cls, output: Any = _UNSET) -> Verdict:
        """The effect happened; ``output`` is what the step would have returned (leave it out for none)."""
        return cls("applied", output)

    @classmethod
    def not_applied(cls) -> Verdict:
        """The effect did not happen: the platform may try it again, with the same key."""
        return cls("not_applied")

    @classmethod
    def unknown(cls) -> Verdict:
        """Cannot tell: the decision goes to a person."""
        return cls("unknown")

    def to_json_value(self) -> dict[str, Any]:
        """Exactly ``{"effect": ..., "output": ...}`` (``output`` only for ``applied`` and only when given)."""
        if self.effect == "applied" and self.output is not _UNSET:
            return {"effect": self.effect, "output": self.output}
        return {"effect": self.effect}

    def __repr__(self) -> str:
        return f"Verdict({self.effect!r})"


class CancellationToken:
    """Set when the run is cancelled, the lease is lost or the host stops, with the reason.

    An ``async`` handler is also cancelled the asyncio way (``asyncio.CancelledError`` at its current
    ``await``); this token is for code that cannot be cancelled that way, a blocking handler in a thread above all.
    """

    def __init__(self) -> None:
        self._flag = threading.Event()
        self._event = asyncio.Event()
        self._reason: CancelReason | None = None

    @property
    def is_cancelled(self) -> bool:
        return self._flag.is_set()

    @property
    def reason(self) -> CancelReason | None:
        """``run-cancelled``, ``lease-lost`` or ``shutdown``; ``None`` while the activity is not cancelled."""
        return self._reason

    async def wait(self) -> CancelReason:
        """Returns when the activity is cancelled."""
        await self._event.wait()
        assert self._reason is not None
        return self._reason

    def wait_blocking(self, timeout: float | None = None) -> bool:
        """For a thread: waits until cancelled or ``timeout`` seconds; True if cancelled."""
        return self._flag.wait(timeout)

    def raise_if_cancelled(self) -> None:
        """Raises ``SagatError(CANCELLED)`` when cancelled: the cancellation error a blocking handler can raise."""
        if self._flag.is_set():
            raise SagatError("CANCELLED", f"the activity was cancelled ({self._reason})")

    def cancel(self, reason: CancelReason) -> bool:
        """Cancels (host only); the first reason stays. False if it was cancelled already."""
        if self._flag.is_set():
            return False
        self._reason = reason
        self._flag.set()
        self._event.set()
        return True


class Secrets:
    """The secrets the step declares, read-only. The values are credentials: never log them.

    Every read hands out a copy. The SDK's own buffers are overwritten with zeros and dropped when the handler
    returns; a read after that raises.
    """

    def __init__(self, values: Mapping[str, bytes]) -> None:
        self._values: dict[str, bytearray] | None = {name: bytearray(value) for name, value in values.items()}

    def _get(self, name: str) -> bytes:
        if self._values is None:
            raise SagatError("VALIDATION_FAILED", "the secrets of this attempt were dropped when the handler returned")
        try:
            return bytes(self._values[name])
        except KeyError:
            raise SagatError("NOT_FOUND", f"the step declares no secret named {name}") from None

    def get(self, name: str) -> bytes:
        """A copy of the bytes of a secret the step declares."""
        return self._get(name)

    def text(self, name: str) -> str:
        """The secret as UTF-8 text."""
        return self._get(name).decode("utf-8")

    def names(self) -> list[str]:
        return list(self._values or ())

    def __contains__(self, name: object) -> bool:
        return self._values is not None and name in self._values

    def __len__(self) -> int:
        return len(self._values or ())

    def _drop(self) -> None:
        values, self._values = self._values, None
        for buffer in (values or {}).values():
            buffer[:] = bytes(len(buffer))  # overwritten in place before it is let go

    def __repr__(self) -> str:  # never the values
        return f"Secrets(names={self.names()!r})"


class ActivityContext:
    """What an activity knows about the attempt it is running (``sdks/SPEC.md`` section 6.3)."""

    def __init__(
        self,
        *,
        activity_name: str,
        task_id: str,
        run_id: str,
        node: str,
        step_id: str,
        attempt: int,
        generation: int,
        idempotency_key: str,
        traceparent: str,
        task_kind: TaskKind,
        target_node: str,
        target_idempotency_key: str,
        steps: Mapping[str, Any],
        cancellation: CancellationToken,
        log: logging.LoggerAdapter[logging.Logger],
        deadline_ms: Callable[[], int],
        secrets: Callable[[], Awaitable[Secrets]],
        write_blob: Callable[[AsyncIterable[bytes] | Iterable[bytes], str, int, str], Awaitable[BlobDescriptor]],
        read_blob: Callable[[BlobDescriptor], AsyncIterator[bytes]],
        instance: ExecutionInstanceIdentity | None = None,
        item: Any = None,
        index: int | None = None,
    ) -> None:
        self.activity_name = activity_name
        self.task_id = task_id
        self.run_id = run_id
        self.node = node
        """The node id of the step in the workflow definition (``charge``), from the step document."""
        self.step_id = step_id
        """The platform's id of the step (``step_id`` of the lease), which the reports carry."""
        self.attempt = attempt
        self.generation = generation
        self.instance = instance
        """Present only when protocol 1.5 was negotiated and the lease supplied an identity."""
        self.item = item
        """The captured collection item for this scoped task; use ``index`` to distinguish a JSON null item."""
        self.index = index
        """The captured collection index; ``None`` for root and non-collection work."""
        self.idempotency_key = idempotency_key
        """Stable across the attempts of the step: send it to the provider of an external effect."""
        self.traceparent = traceparent
        self.task_kind = task_kind
        self.target_node = target_node
        """For a compensation or a reconciliation: the forward step it is about."""
        self.target_idempotency_key = target_idempotency_key
        """The key that identifies the effect of that step at the provider."""
        self.steps = steps
        """The outputs of the steps that completed before this one, by node."""
        self.cancellation = cancellation
        self.log = log
        self._deadline_ms = deadline_ms
        self._secrets = secrets
        self._write_blob = write_blob
        self._read_blob = read_blob

    @property
    def deadline(self) -> datetime:
        """The end of the lease (an aware UTC instant); it moves forward with every heartbeat."""
        return datetime.fromtimestamp(self._deadline_ms() / 1000, tz=timezone.utc)

    @property
    def cancel_reason(self) -> CancelReason | None:
        """Why the activity was cancelled: ``run-cancelled``, ``lease-lost`` or ``shutdown``; ``None`` if it was not."""
        return self.cancellation.reason

    async def secrets(self) -> Secrets:
        """The secrets the step declares; resolved once per attempt, on first use. Never log them."""
        return await self._secrets()

    async def write_blob(
        self,
        source: AsyncIterable[bytes] | Iterable[bytes],
        *,
        content_type: str = "",
        expected_size: int = 0,
        expected_sha256: str = "",
    ) -> BlobDescriptor:
        """Streams a one-pass source and verifies the service descriptor against the bytes sent."""
        return await self._write_blob(source, content_type, expected_size, expected_sha256)

    def read_blob(self, descriptor: BlobDescriptor) -> AsyncIterator[bytes]:
        """Streams bytes; successful exhaustion means size and SHA-256 matched the descriptor."""
        return self._read_blob(descriptor)


ActivityHandler = Callable[[Any, ActivityContext], Any]
"""``async def handler(input, ctx)`` or a plain function (run in a thread); its result is the step's output."""


async def run_blocking(func: Callable[..., T], /, *args: Any, **kwargs: Any) -> T:
    """Runs a blocking function in a thread so that it does not stop the event loop (``asyncio.to_thread``)."""
    return await asyncio.to_thread(func, *args, **kwargs)


def is_cancellation(error: BaseException) -> bool:
    """True for the error an aborted activity raises: ``asyncio.CancelledError`` or ``SagatError(CANCELLED)``."""
    return isinstance(error, asyncio.CancelledError) or (isinstance(error, SagatError) and error.code == "CANCELLED")
