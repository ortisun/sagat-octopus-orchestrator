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
from typing import Concatenate, ParamSpec, TypeVar

from .client import Run, SagatClient
from .config import ClientConfig
from .errors import SagatError

P = ParamSpec("P")
R = TypeVar("R")


def _blocking(
    method: Callable[Concatenate[SagatClient, P], Awaitable[R]],
) -> Callable[Concatenate[SyncSagatClient, P], R]:
    @functools.wraps(method)
    def call(self: SyncSagatClient, *args: P.args, **kwargs: P.kwargs) -> R:
        return self._run(lambda: method(self._client, *args, **kwargs))

    return call  # type: ignore[return-value]  # functools.wraps keeps the signature; mypy cannot see it


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
    get = _blocking(SagatClient.get)
    list = _blocking(SagatClient.list)
    operate = _blocking(SagatClient.operate)
    cancel = _blocking(SagatClient.cancel)
    decide_approval = _blocking(SagatClient.decide_approval)
    query = _blocking(SagatClient.query)
    update = _blocking(SagatClient.update)
    inspect_chain = _blocking(SagatClient.inspect_chain)
    signal_chain = _blocking(SagatClient.signal_chain)
    cancel_chain = _blocking(SagatClient.cancel_chain)
    signal = _blocking(SagatClient.signal)
    publish_event = _blocking(SagatClient.publish_event)
    events = _blocking(SagatClient.events)
    wait_for_completion = _blocking(SagatClient.wait_for_completion)

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
