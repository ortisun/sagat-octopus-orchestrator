"""A relay double for the tests of the container's side (feature 035, D035-078).

It speaks the wire of `crates/infra-lake/src/relay.rs` byte for byte, so the forwarder and the host are tested against
the bytes the real relay sends and expects: a first line `SAGAT-RELAY/1 <token>`, then `CONNECT host:port HTTP/1.1` with
its headers or `TCP <name>` or `CONTROL`, and the replies of the Rust constants. The Rust relay serves `CONTROL` through
the attempt driver (T035-035); the double serves it with a handler of the test, so the host can be exercised.

The two implementations can drift. The canaries that run the real clients through the real relay (T035-031) are what
would show it.
"""

from __future__ import annotations

import asyncio
import contextlib
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from pathlib import Path

PREFACE = b"SAGAT-RELAY/1 "
ESTABLISHED = b"HTTP/1.1 200 Connection Established\r\n\r\n"
FORBIDDEN = b"HTTP/1.1 403 Forbidden\r\n\r\n"
BAD_REQUEST = b"HTTP/1.1 400 Bad Request\r\n\r\n"
TCP_OK = b"OK\n"
TCP_DENIED = b"DENIED\n"

Tunnel = Callable[[asyncio.StreamReader, asyncio.StreamWriter], Awaitable[None]]


async def echo(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    """A destination that sends back what it receives."""
    while data := await reader.read(65536):
        writer.write(data)
        await writer.drain()


@dataclass
class Seen:
    """What one connection asked for."""

    kind: str  # "connect", "tcp", "control", "refused"
    target: str = ""
    headers: list[bytes] = field(default_factory=list)


class FakeRelay:
    def __init__(
        self,
        token: str,
        *,
        connects: dict[str, Tunnel] | None = None,
        mappings: dict[str, Tunnel] | None = None,
        control: Tunnel | None = None,
    ) -> None:
        self.token = token.encode()
        self.connects = connects or {}
        self.mappings = mappings or {}
        self.control = control
        self.seen: list[Seen] = []
        self._server: asyncio.AbstractServer | None = None
        self._tasks: set[asyncio.Task[None]] = set()

    async def start(self, path: Path) -> None:
        self._server = await asyncio.start_unix_server(self._serve, path=str(path))

    async def stop(self) -> None:
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()
        for task in list(self._tasks):
            task.cancel()
        for task in list(self._tasks):
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await task

    async def _serve(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        task = asyncio.current_task()
        if task is not None:
            self._tasks.add(task)
        try:
            await self._conversation(reader, writer)
        except (ConnectionError, asyncio.IncompleteReadError):
            pass
        finally:
            writer.close()
            with contextlib.suppress(ConnectionError):
                await writer.wait_closed()
            if task is not None:
                self._tasks.discard(task)

    async def _conversation(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        first = (await reader.readline()).rstrip(b"\n")
        if first != PREFACE + self.token:
            self.seen.append(Seen("refused"))
            return  # no reply: a process that is not the attempt's host learns nothing
        line = (await reader.readline()).rstrip(b"\n")
        if line.startswith(b"CONNECT "):
            headers = []
            while (header := await reader.readline()) not in (b"\r\n", b""):
                headers.append(header)
            target = line.split(b" ")[1].decode()
            self.seen.append(Seen("connect", target, headers))
            handler = self.connects.get(target)
            if handler is None:
                writer.write(FORBIDDEN)
                return
            writer.write(ESTABLISHED)
            await writer.drain()
            await handler(reader, writer)
        elif line.startswith(b"TCP "):
            name = line[4:].decode()
            self.seen.append(Seen("tcp", name))
            handler = self.mappings.get(name)
            if handler is None:
                writer.write(TCP_DENIED)
                return
            writer.write(TCP_OK)
            await writer.drain()
            await handler(reader, writer)
        elif line == b"CONTROL" and self.control is not None:
            self.seen.append(Seen("control"))
            await self.control(reader, writer)
        else:
            self.seen.append(Seen("refused", line.decode(errors="replace")))
            writer.write(BAD_REQUEST)
