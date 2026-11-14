"""A small synchronous facade of the client, for code that has no event loop (scripts, Django views, notebooks).

It runs the asyncio client on a private event loop in a daemon thread and waits for each call, so every rule of
the async client (retries, keys, errors) is the same code. It is not for use inside a running event loop: there,
use ``SagatClient`` and ``await``.
"""

from __future__ import annotations

import asyncio
import functools
import threading
from collections.abc import Awaitable, Callable, Iterator
from concurrent.futures import Future
from types import TracebackType
from typing import Concatenate, Generic, Literal, ParamSpec, TypeVar

from .client import ApprovalListItem, ChainRun, Run, SagatClient
from .config import ClientConfig
from .errors import SagatError
from .operations import BatchOperations, BusinessIds, Schedule, Schedules

P = ParamSpec("P")
R = TypeVar("R")
G = TypeVar("G")


def _blocking(
    method: Callable[Concatenate[SagatClient, P], Awaitable[R]],
) -> Callable[Concatenate[SyncSagatClient, P], R]:
    @functools.wraps(method)
    def call(self: SyncSagatClient, *args: P.args, **kwargs: P.kwargs) -> R:
        return self._run(lambda: method(self._client, *args, **kwargs))

    return call  # type: ignore[return-value]  # functools.wraps keeps the signature; mypy cannot see it


class _SyncGroup(Generic[G]):
    """A group of the async client (``schedules``, ``batch``, ``business_ids``) with blocking methods."""

    def __init__(self, owner: SyncSagatClient, group: G) -> None:
        self._owner = owner
        self._group = group


def _grouped(
    method: Callable[Concatenate[G, P], Awaitable[R]],
) -> Callable[Concatenate[_SyncGroup[G], P], R]:
    @functools.wraps(method)
    def call(self: _SyncGroup[G], *args: P.args, **kwargs: P.kwargs) -> R:
        return self._owner._run(lambda: method(self._group, *args, **kwargs))

    return call  # type: ignore[return-value]  # functools.wraps keeps the signature; mypy cannot see it


class SyncSchedules(_SyncGroup[Schedules]):
    """``client.schedules`` with blocking methods."""

    create = _grouped(Schedules.create)
    get = _grouped(Schedules.get)
    list = _grouped(Schedules.list)
    update = _grouped(Schedules.update)
    delete = _grouped(Schedules.delete)
    pause = _grouped(Schedules.pause)
    resume = _grouped(Schedules.resume)
    trigger = _grouped(Schedules.trigger)
    backfill = _grouped(Schedules.backfill)
    cancel_backfill = _grouped(Schedules.cancel_backfill)
    list_backfills = _grouped(Schedules.list_backfills)
    list_actions = _grouped(Schedules.list_actions)

    def list_all(
        self,
        namespace_id: str,
        *,
        limit: int | None = None,
        state: Literal["ACTIVE", "PAUSED"] | None = None,
        origin: Literal["API", "TRIGGER"] | None = None,
        workflow_id: str | None = None,
        name: str | None = None,
        traceparent: str | None = None,
    ) -> Iterator[Schedule]:
        """Every schedule the filter selects, following the opaque cursors."""
        cursor: str | None = None
        while True:
            page = self.list(
                namespace_id,
                cursor=cursor,
                limit=limit,
                state=state,
                origin=origin,
                workflow_id=workflow_id,
                name=name,
                traceparent=traceparent,
            )
            yield from page.items
            cursor = page.next_cursor
            if not cursor:
                return


class SyncBatchOperations(_SyncGroup[BatchOperations]):
    """``client.batch`` with blocking methods."""

    preview = _grouped(BatchOperations.preview)
    create = _grouped(BatchOperations.create)
    get = _grouped(BatchOperations.get)
    list = _grouped(BatchOperations.list)
    cancel = _grouped(BatchOperations.cancel)
    list_items = _grouped(BatchOperations.list_items)


class SyncBusinessIds(_SyncGroup[BusinessIds]):
    """``client.business_ids`` with blocking methods."""

    get = _grouped(BusinessIds.get)


class SyncSagatClient:
    """``SagatClient`` with blocking methods: ``client.start(...)`` returns the ``Run``.

    Safe to share between threads. Close it (or use ``with``) to stop its thread.
    """

    def __init__(self, config: ClientConfig) -> None:
        self._loop = asyncio.new_event_loop()
        self._thread = threading.Thread(target=self._serve, name="sagat-client", daemon=True)
        self._closed = False
        self._thread.start()
        try:
            self._client: SagatClient = self._run(lambda: self._create(config))
        except BaseException:
            self._closed = True
            self._loop.call_soon_threadsafe(self._loop.stop)
            raise
        self.schedules = SyncSchedules(self, self._client.schedules)
        self.batch = SyncBatchOperations(self, self._client.batch)
        self.business_ids = SyncBusinessIds(self, self._client.business_ids)

    def _serve(self) -> None:
        asyncio.set_event_loop(self._loop)
        self._loop.run_forever()
        self._loop.close()

    @staticmethod
    async def _create(config: ClientConfig) -> SagatClient:
        return SagatClient(config)  # made on the loop that will use it

    def _run(self, make: Callable[[], Awaitable[R]]) -> R:
        if self._closed:
            raise SagatError("VALIDATION_FAILED", "the client is closed")
        if threading.current_thread() is self._thread:
            raise SagatError("VALIDATION_FAILED", "a blocking call from the client's own thread would never return")

        async def runner() -> R:
            return await make()

        future: Future[R] = asyncio.run_coroutine_threadsafe(runner(), self._loop)
        try:
            return future.result()
        except BaseException:  # also a KeyboardInterrupt while waiting: stop the call too
            future.cancel()
            raise

    def close(self) -> None:
        """Closes the connections and stops the thread. Idempotent."""
        if self._closed:
            return
        self._run(self._client.aclose)
        self._closed = True
        self._loop.call_soon_threadsafe(self._loop.stop)
        self._thread.join(timeout=5)

    def __enter__(self) -> SyncSagatClient:
        return self

    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None:
        self.close()

    resolve_workflow = _blocking(SagatClient.resolve_workflow)
    start = _blocking(SagatClient.start)
    upload_run_input = _blocking(SagatClient.upload_run_input)
    start_with_blob = _blocking(SagatClient.start_with_blob)
    signal_with_start = _blocking(SagatClient.signal_with_start)
    update_with_start = _blocking(SagatClient.update_with_start)
    reset_points = _blocking(SagatClient.reset_points)
    preview_reset = _blocking(SagatClient.preview_reset)
    reset = _blocking(SagatClient.reset)
    list_resets = _blocking(SagatClient.list_resets)
    get_reset = _blocking(SagatClient.get_reset)
    get = _blocking(SagatClient.get)
    list = _blocking(SagatClient.list)
    operate = _blocking(SagatClient.operate)
    cancel = _blocking(SagatClient.cancel)
    rerun_as_new = _blocking(SagatClient.rerun_as_new)
    decide_approval = _blocking(SagatClient.decide_approval)
    list_approvals = _blocking(SagatClient.list_approvals)
    query = _blocking(SagatClient.query)
    query_chain = _blocking(SagatClient.query_chain)
    update = _blocking(SagatClient.update)
    update_chain = _blocking(SagatClient.update_chain)
    inspect_chain = _blocking(SagatClient.inspect_chain)
    list_chain_runs = _blocking(SagatClient.list_chain_runs)
    signal_chain = _blocking(SagatClient.signal_chain)
    cancel_chain = _blocking(SagatClient.cancel_chain)
    signal = _blocking(SagatClient.signal)
    publish_event = _blocking(SagatClient.publish_event)
    events = _blocking(SagatClient.events)
    wait_for_completion = _blocking(SagatClient.wait_for_completion)
    wait_for_chain_completion = _blocking(SagatClient.wait_for_chain_completion)

    def list_all(
        self, *, workflow_id: str | None = None, status: str | None = None, limit: int | None = None
    ) -> Iterator[Run]:
        """Every run that matches, page after page, in the order the server gave."""
        cursor: str | None = None
        while True:
            page = self.list(workflow_id=workflow_id, status=status, limit=limit, cursor=cursor)
            yield from page.items
            cursor = page.next_cursor
            if not cursor:
                return

    def list_all_chain_runs(
        self, namespace_id: str, chain_id: str, *, limit: int | None = None, traceparent: str | None = None
    ) -> Iterator[ChainRun]:
        """Every run of a chain, in segment order."""
        cursor: str | None = None
        while True:
            page = self.list_chain_runs(namespace_id, chain_id, cursor=cursor, limit=limit, traceparent=traceparent)
            yield from page.items
            cursor = page.next_cursor
            if not cursor:
                return

    def list_all_approvals(
        self, run_id: str, *, limit: int | None = None, traceparent: str | None = None
    ) -> Iterator[ApprovalListItem]:
        """Every approval request for a run in immutable opening order."""
        cursor: str | None = None
        while True:
            page = self.list_approvals(run_id, cursor=cursor, limit=limit, traceparent=traceparent)
            yield from page.items
            cursor = page.next_cursor
            if not cursor:
                return
