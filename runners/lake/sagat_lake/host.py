"""The lake host: the sandboxed side of `sagat.lake.v1` (feature 035, D035-062, D035-067, D035-078).

The runner starts it as `python3 -I -X utf8 /opt/sagat/lake/sagat_lake/host.py --attempt-dir /work/<attempt-id>`.
The host:

1. refuses to be read by a process of the same user (`PR_SET_DUMPABLE=0`), as its first action;
2. reads the attempt's token from `relay.token`, connects to `relay.sock`, presents the token and asks for the control
   conversation (`CONTROL`);
3. reads **one request line**, bounded, and checks that it speaks the protocol;
4. runs the operation it names, writing a **progress line** (`{"heartbeat":true}`) at most once a second while
   it works;
5. writes **exactly one result line**, `{"receipt": ...}` or `{"failure": ...}` with `relayDenials`, closes, and
   exits 0.

It writes no value of the request. A failure is a code of the closed set. An exception that no operation expected is
`WORKER_ERROR`, and only the *name of its class* reaches standard error: the runner keeps of that stream a class and a
line count, and a message could carry a credential. `relayDenials` is the runner's to know, since it owns the relay, so
the host writes 0 and the runner uses its own count (G035-028).

It serves `icebergWrite` (`write.py`, T035-057). Each other operation arrives with the task that tests it, and a request
for one that is not here is answered with `LAKE_HOST_PROTOCOL`, never with a pretence.
"""

from __future__ import annotations

import asyncio
import contextlib
import ctypes
import json
import re
import sys
from collections.abc import Awaitable, Callable, Mapping, Sequence
from pathlib import Path

# `python3 -I` ignores PYTHONPATH and does not put the script's directory on the path, so the package is made importable
# here, once, from the read-only directory this file lives in, and from nowhere else.
if __name__ == "__main__" and not __package__:
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from sagat_lake.failures import LAKE_HOST_PROTOCOL, WORKER_ERROR, Failure
from sagat_lake.write import operations_for

# (experiment, not for merge: a change to a source of runners/lake selects the four lake suites on CI)
PROTOCOL = "sagat.lake.v1"
PREFACE = b"SAGAT-RELAY/1 "
CONTROL = b"CONTROL\n"
HEARTBEAT = b'{"heartbeat":true}\n'
# At most one a second (the runner tolerates one more inside the same second, for a socket that delivers late).
HEARTBEAT_INTERVAL = 1.0
# The request holds the pinned connection with its resolved credentials, the table, the mapping and the limits.
MAX_REQUEST_BYTES = 4 * 1024 * 1024
REQUEST_TIMEOUT = 30.0

_TOKEN = re.compile(r"[A-Za-z0-9_-]{32,128}")
_PR_SET_DUMPABLE = 4

Operation = Callable[[Mapping[str, object]], Awaitable[dict[str, object]]]

# Each operation (`icebergWrite`, `icebergRevert`, `objectWrite`, `probe`, `browse`) arrives with its task;
# `icebergWrite` is served by `write.py`, for the attempt directory it is started with.


def harden() -> bool:
    """`PR_SET_DUMPABLE=0`: another process of the same user cannot attach to this one or read its memory, where the
    credentials of the request live. Linux only; the container is Linux. Returns whether it took effect."""
    if sys.platform != "linux":
        return False
    libc = ctypes.CDLL(None, use_errno=True)
    return bool(libc.prctl(_PR_SET_DUMPABLE, 0, 0, 0, 0) == 0)


def read_token(text: str) -> str:
    """The token of `relay.token`: a line of 32 to 128 letters, digits, '-' and '_'."""
    token = text.removesuffix("\n")
    if not _TOKEN.fullmatch(token):
        raise ValueError("relay.token does not hold a relay token")
    return token


def _line(document: Mapping[str, object]) -> bytes:
    return json.dumps(document, separators=(",", ":"), ensure_ascii=True).encode() + b"\n"


def _parse_request(raw: bytes) -> dict[str, object] | None:
    """The request, if it is an object that speaks this protocol and names an operation; otherwise `None`."""
    try:
        document = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None
    if not isinstance(document, dict) or document.get("protocol") != PROTOCOL:
        return None
    operation = document.get("operation")
    if not isinstance(operation, str) or not operation:
        return None
    return document


async def _heartbeats(writer: asyncio.StreamWriter, interval: float) -> None:
    while True:
        await asyncio.sleep(interval)
        writer.write(HEARTBEAT)
        await writer.drain()


async def _outcome(
    request: dict[str, object], operations: Mapping[str, Operation], writer: asyncio.StreamWriter, interval: float
) -> dict[str, object]:
    """The `receipt` or `failure` member of the result, while a heartbeat tells the runner the host is alive."""
    operation = operations.get(str(request["operation"]))
    if operation is None:
        return {"failure": Failure(LAKE_HOST_PROTOCOL).as_member()}
    beat = asyncio.ensure_future(_heartbeats(writer, interval))
    try:
        return {"receipt": await operation(request)}
    except Failure as failure:
        return {"failure": failure.as_member()}
    except Exception as error:
        print(type(error).__name__, file=sys.stderr)
        return {"failure": Failure(WORKER_ERROR).as_member()}
    finally:
        beat.cancel()
        with contextlib.suppress(asyncio.CancelledError, ConnectionError):
            await beat


async def run(
    attempt_dir: Path,
    operations: Mapping[str, Operation],
    *,
    heartbeat_interval: float = HEARTBEAT_INTERVAL,
    max_request_bytes: int = MAX_REQUEST_BYTES,
    request_timeout: float = REQUEST_TIMEOUT,
) -> int:
    """One attempt: the conversation with the runner. The exit code is 0 when a result line was written."""
    token = read_token((attempt_dir / "relay.token").read_text(encoding="utf-8"))
    reader, writer = await asyncio.open_unix_connection(str(attempt_dir / "relay.sock"), limit=max_request_bytes + 1)
    try:
        writer.write(PREFACE + token.encode() + b"\n" + CONTROL)
        await writer.drain()
        try:
            raw = await asyncio.wait_for(reader.readline(), timeout=request_timeout)
        except (ValueError, asyncio.LimitOverrunError, TimeoutError):
            raw = b""
        payload = raw.removesuffix(b"\n")
        request = _parse_request(payload) if len(payload) <= max_request_bytes else None
        if request is None:
            outcome: dict[str, object] = {"failure": Failure(LAKE_HOST_PROTOCOL).as_member()}
        else:
            outcome = await _outcome(request, operations, writer, heartbeat_interval)
        writer.write(_line({**outcome, "relayDenials": 0}))
        await writer.drain()
        return 0
    finally:
        writer.close()
        with contextlib.suppress(ConnectionError):
            await writer.wait_closed()


def main(argv: Sequence[str]) -> int:
    harden()
    if len(argv) != 2 or argv[0] != "--attempt-dir":
        return 2
    attempt_dir = Path(argv[1])
    return asyncio.run(run(attempt_dir, operations_for(attempt_dir)))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
