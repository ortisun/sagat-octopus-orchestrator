"""The forwarder: loopback ports for the client libraries, each one a connection to the relay (feature 035, D035-049).

The container has no route but the Unix socket of the relay. PyIceberg and its file systems speak HTTP through a proxy
and TCP to a metastore or a database, so this turns one HTTP proxy port and one TCP port per declared mapping into
connections to that socket. Each client connection gets a relay connection of its own that presents the attempt's token
first (`SAGAT-RELAY/1 <token>`), then either the client's own `CONNECT` request (the proxy port) or `TCP <name>` (a
mapping port, whose one-line answer the forwarder reads and hides). After that it copies bytes in both directions and
reads none of them, so TLS stays between the client library and the destination.

Only the loopback interface is bound, and the ports are the system's to choose. The relay decides what is reachable;
the forwarder decides nothing.
"""

from __future__ import annotations

import asyncio
import contextlib
import re
from collections.abc import Awaitable, Callable, Sequence

PREFACE = b"SAGAT-RELAY/1 "
_TOKEN = re.compile(r"[A-Za-z0-9_-]{32,128}")
_MAPPING = re.compile(r"[a-z0-9-]{1,32}")
_CHUNK = 65536
_ANSWER_TIMEOUT = 10.0

Handler = Callable[[asyncio.StreamReader, asyncio.StreamWriter], Awaitable[None]]


async def _pump(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    """Copies until the reader ends, then tells the writer's peer that nothing more comes (a half close)."""
    try:
        while data := await reader.read(_CHUNK):
            writer.write(data)
            await writer.drain()
        if writer.can_write_eof():
            writer.write_eof()
    except (ConnectionError, OSError):
        pass


async def _close(writer: asyncio.StreamWriter) -> None:
    writer.close()
    with contextlib.suppress(ConnectionError, OSError):
        await writer.wait_closed()


class Forwarder:
    address = "127.0.0.1"

    def __init__(self, relay_socket: str, token: str, mappings: Sequence[str] = ()) -> None:
        if not _TOKEN.fullmatch(token):
            raise ValueError("a relay token is 32 to 128 letters, digits, '-' and '_'")
        if any(not _MAPPING.fullmatch(name) for name in mappings) or len(set(mappings)) != len(mappings):
            raise ValueError("a mapping name is 1 to 32 lower-case letters, digits and '-', once each")
        self._socket = relay_socket
        self._preface = PREFACE + token.encode() + b"\n"
        self._mappings = tuple(mappings)
        self._servers: list[asyncio.Server] = []
        self._ports: dict[str, int] = {}
        self._http_port = 0
        self._connections: set[asyncio.Task[None]] = set()

    @property
    def http_port(self) -> int:
        return self._http_port

    def mapping_port(self, name: str) -> int:
        return self._ports[name]

    async def start(self) -> None:
        proxy = await asyncio.start_server(self._handler(None), host=self.address, port=0)
        self._servers.append(proxy)
        self._http_port = proxy.sockets[0].getsockname()[1]
        for name in self._mappings:
            server = await asyncio.start_server(self._handler(name), host=self.address, port=0)
            self._servers.append(server)
            self._ports[name] = server.sockets[0].getsockname()[1]

    async def stop(self) -> None:
        for server in self._servers:
            server.close()
        for task in list(self._connections):
            task.cancel()
        for task in list(self._connections):
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await task
        for server in self._servers:
            await server.wait_closed()
        self._servers.clear()

    def _handler(self, mapping: str | None) -> Handler:
        async def handle(client_reader: asyncio.StreamReader, client_writer: asyncio.StreamWriter) -> None:
            task = asyncio.current_task()
            if task is not None:
                self._connections.add(task)
            try:
                await self._tunnel(client_reader, client_writer, mapping)
            finally:
                if task is not None:
                    self._connections.discard(task)

        return handle

    async def _tunnel(
        self, client_reader: asyncio.StreamReader, client_writer: asyncio.StreamWriter, mapping: str | None
    ) -> None:
        try:
            relay_reader, relay_writer = await asyncio.open_unix_connection(self._socket)
        except OSError:
            await _close(client_writer)
            return
        try:
            relay_writer.write(self._preface)
            if mapping is not None:
                relay_writer.write(b"TCP " + mapping.encode() + b"\n")
                await relay_writer.drain()
                answer = await asyncio.wait_for(relay_reader.readline(), timeout=_ANSWER_TIMEOUT)
                if answer != b"OK\n":
                    return
            pumps = {
                asyncio.ensure_future(_pump(client_reader, relay_writer)),
                asyncio.ensure_future(_pump(relay_reader, client_writer)),
            }
            try:
                await asyncio.wait(pumps)
            finally:
                for pump in pumps:
                    pump.cancel()
                await asyncio.gather(*pumps, return_exceptions=True)
        except (ConnectionError, OSError, TimeoutError, asyncio.IncompleteReadError):
            pass
        finally:
            await _close(relay_writer)
            await _close(client_writer)
