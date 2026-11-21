"""The forwarder: the loopback ports the client libraries use, and the one connection each of them makes to the relay
(feature 035, T035-030, D035-049, D035-078).

The container has no route but the relay's Unix socket. The forwarder turns an HTTP proxy port and one TCP port per
declared mapping into connections to that socket, each one presenting the attempt's token first. It carries bytes and
reads none of them, so TLS stays between the client library and the destination.
"""

from __future__ import annotations

import asyncio
import contextlib
import tempfile
from collections.abc import Awaitable, Callable
from pathlib import Path

import pytest

from relay_double import ESTABLISHED, FORBIDDEN, FakeRelay, echo
from sagat_lake.forwarder import Forwarder

TOKEN = "relay-token-canary-0123456789abcdef"
CONNECT = b"CONNECT catalog.example:443 HTTP/1.1\r\nHost: catalog.example:443\r\nProxy-Connection: keep-alive\r\n\r\n"


def scenario(
    relay: FakeRelay,
    body: Callable[[Forwarder, FakeRelay], Awaitable[None]],
    *,
    mappings: tuple[str, ...] = ("metastore",),
    token: str = TOKEN,
) -> None:
    """Runs `body` with a relay double and a forwarder on its socket, and stops both."""

    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            socket = Path(directory) / "relay.sock"
            await relay.start(socket)
            forwarder = Forwarder(str(socket), token, mappings)
            await forwarder.start()
            try:
                await body(forwarder, relay)
            finally:
                await forwarder.stop()
                await relay.stop()

    asyncio.run(main())


async def connect_to(port: int) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    return await asyncio.open_connection("127.0.0.1", port)


async def closed(reader: asyncio.StreamReader) -> bytes:
    """What is left until the peer closes, within a limit. A peer that closes with a request of ours unread resets the
    connection instead of ending it: both mean closed, and only the timeout means a hang."""
    try:
        return await asyncio.wait_for(reader.read(), timeout=5)
    except ConnectionResetError:
        return b""


async def finish(writer: asyncio.StreamWriter) -> None:
    writer.close()
    with contextlib.suppress(ConnectionError):
        await writer.wait_closed()


def test_a_connect_reaches_the_relay_with_the_token_first_and_the_tunnel_carries_the_bytes() -> None:
    relay = FakeRelay(TOKEN, connects={"catalog.example:443": echo})

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        reader, writer = await connect_to(forwarder.http_port)
        writer.write(CONNECT)
        assert await reader.readexactly(len(ESTABLISHED)) == ESTABLISHED
        # a TLS ClientHello's first bytes, and binary: the forwarder reads none of it
        payload = b"\x16\x03\x01\x02\x00" + bytes(range(256)) * 16
        writer.write(payload)
        assert await reader.readexactly(len(payload)) == payload
        await finish(writer)
        assert [(seen.kind, seen.target) for seen in relay.seen] == [("connect", "catalog.example:443")]
        assert b"Proxy-Connection: keep-alive\r\n" in relay.seen[0].headers

    scenario(relay, body)


def test_a_denied_connect_is_answered_as_the_relay_answered_and_closed() -> None:
    relay = FakeRelay(TOKEN, connects={})

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        reader, writer = await connect_to(forwarder.http_port)
        writer.write(CONNECT)

        assert await closed(reader) == FORBIDDEN
        await finish(writer)

    scenario(relay, body)


def test_a_tcp_port_asks_the_relay_for_its_mapping_and_hides_the_relays_answer() -> None:
    relay = FakeRelay(TOKEN, mappings={"metastore": echo})

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        reader, writer = await connect_to(forwarder.mapping_port("metastore"))
        writer.write(b"\x80\x01\x00\x01thrift")
        # the first bytes the client sees are its own, echoed: `OK` was the forwarder's to read
        assert await reader.readexactly(len(b"\x80\x01\x00\x01thrift")) == b"\x80\x01\x00\x01thrift"
        await finish(writer)
        assert [(seen.kind, seen.target) for seen in relay.seen] == [("tcp", "metastore")]

    scenario(relay, body)


def test_a_mapping_the_relay_refuses_closes_the_client_with_nothing() -> None:
    relay = FakeRelay(TOKEN, mappings={})

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        reader, writer = await connect_to(forwarder.mapping_port("metastore"))
        writer.write(b"anything")

        assert await closed(reader) == b""
        await finish(writer)

    scenario(relay, body)


def test_a_client_that_closes_its_side_still_receives_what_the_destination_sends() -> None:
    async def reply_then_close(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        received = await reader.read()
        writer.write(b"you said: " + received)
        await writer.drain()

    relay = FakeRelay(TOKEN, connects={"catalog.example:443": reply_then_close})

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        reader, writer = await connect_to(forwarder.http_port)
        writer.write(CONNECT)
        assert await reader.readexactly(len(ESTABLISHED)) == ESTABLISHED
        writer.write(b"hello")
        writer.write_eof()

        assert await closed(reader) == b"you said: hello"
        await finish(writer)

    scenario(relay, body)


def test_every_client_gets_a_connection_of_its_own_to_the_relay() -> None:
    relay = FakeRelay(TOKEN, connects={"catalog.example:443": echo})

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        clients = [await connect_to(forwarder.http_port) for _ in range(3)]
        for reader, writer in clients:
            writer.write(CONNECT)
            assert await reader.readexactly(len(ESTABLISHED)) == ESTABLISHED
        assert len(relay.seen) == 3
        for _, writer in clients:
            await finish(writer)

    scenario(relay, body)


def test_a_relay_that_is_not_there_closes_the_client_instead_of_hanging() -> None:
    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            forwarder = Forwarder(str(Path(directory) / "no-such.sock"), TOKEN, ("metastore",))
            await forwarder.start()
            try:
                for port in (forwarder.http_port, forwarder.mapping_port("metastore")):
                    reader, writer = await connect_to(port)
                    writer.write(CONNECT)
                    assert await closed(reader) == b""
                    await finish(writer)
            finally:
                await forwarder.stop()

    asyncio.run(main())


def test_a_wrong_token_is_refused_by_the_relay_and_the_client_learns_nothing() -> None:
    relay = FakeRelay(TOKEN, connects={"catalog.example:443": echo})

    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            socket = Path(directory) / "relay.sock"
            await relay.start(socket)
            forwarder = Forwarder(str(socket), "another-token-0123456789abcdef-0123456789", ())
            await forwarder.start()
            try:
                reader, writer = await connect_to(forwarder.http_port)
                writer.write(CONNECT)
                assert await closed(reader) == b""
                await finish(writer)
                assert [seen.kind for seen in relay.seen] == ["refused"]
            finally:
                await forwarder.stop()
                await relay.stop()

    asyncio.run(main())


def test_the_ports_are_on_the_loopback_interface_and_distinct() -> None:
    relay = FakeRelay(TOKEN)

    async def body(forwarder: Forwarder, relay: FakeRelay) -> None:
        assert forwarder.address == "127.0.0.1"
        ports = {forwarder.http_port, forwarder.mapping_port("metastore"), forwarder.mapping_port("catalog-db")}
        assert len(ports) == 3
        assert all(0 < port < 65536 for port in ports)
        with pytest.raises(KeyError):
            forwarder.mapping_port("undeclared")

    scenario(relay, body, mappings=("metastore", "catalog-db"))


def test_a_stopped_forwarder_accepts_no_more_clients() -> None:
    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            forwarder = Forwarder(str(Path(directory) / "relay.sock"), TOKEN, ())
            await forwarder.start()
            port = forwarder.http_port
            await forwarder.stop()
            with pytest.raises(ConnectionRefusedError):
                await connect_to(port)

    asyncio.run(main())


@pytest.mark.parametrize(
    "token",
    ["", "short", "a" * 31, "a" * 129, "has space " + "a" * 30, "newline\n" + "a" * 30, "colon:" + "a" * 30],
)
def test_a_token_that_could_not_be_one_is_refused_when_the_forwarder_is_built(token: str) -> None:
    with pytest.raises(ValueError, match="token"):
        Forwarder("/tmp/relay.sock", token, ())


@pytest.mark.parametrize("name", ["", "Metastore", "meta store", "meta_store", "../x", "a" * 33, "metastore\n"])
def test_a_mapping_name_the_relay_would_not_know_is_refused_when_the_forwarder_is_built(name: str) -> None:
    with pytest.raises(ValueError, match="mapping"):
        Forwarder("/tmp/relay.sock", TOKEN, (name,))


def test_two_mappings_of_one_name_are_refused() -> None:
    with pytest.raises(ValueError, match="mapping"):
        Forwarder("/tmp/relay.sock", TOKEN, ("metastore", "metastore"))
