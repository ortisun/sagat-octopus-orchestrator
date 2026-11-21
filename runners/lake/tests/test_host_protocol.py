"""The host's side of `sagat.lake.v1` (feature 035, T035-032, D035-062, D035-067, D035-078).

The host authenticates on the relay with the attempt's token, asks for the control conversation, reads one request line,
writes progress lines at most once a second and exactly one result line, and exits 0. It never writes a value of the
request: the runner keeps of its standard error a class and a line count, and everything it reports is a closed code.
"""

from __future__ import annotations

import asyncio
import ctypes
import json
import os
import re
import sys
import tempfile
from collections.abc import Awaitable, Callable, Mapping
from pathlib import Path

import pytest

from relay_double import PREFACE, FakeRelay
from sagat_lake import host
from sagat_lake.failures import LAKE_HOST_PROTOCOL, WORKER_ERROR, Failure

TOKEN = "relay-token-canary-0123456789abcdef"
CANARY = "credential-canary-s3-secret-access-key"
ROOT = Path(__file__).resolve().parents[3]
RUNTIME = Path(__file__).resolve().parents[1]

Operation = Callable[[Mapping[str, object]], Awaitable[dict[str, object]]]
REAL_HARDEN = host.harden


@pytest.fixture(autouse=True)
def no_real_hardening(monkeypatch: pytest.MonkeyPatch) -> None:
    """`prctl` would change the process that runs pytest: only the test of hardening calls the real one."""
    monkeypatch.setattr(host, "harden", lambda: True)


def request(operation: str = "probe", **extra: object) -> dict[str, object]:
    return {
        "protocol": "sagat.lake.v1",
        "operation": operation,
        "effectKey": "run-7f3a/load/1",
        "firstAttemptStartedAt": "2026-11-05T10:00:00.000Z",
        "relay": {"socket": "/work/a/relay.sock", "token": TOKEN},
        "connection": {"credentials": {"s3.secret-access-key": CANARY}},
        **extra,
    }


class Conversation:
    """What the runner's side of the control conversation saw of the host."""

    def __init__(self, request_line: bytes, *, terminated: bool = True) -> None:
        self.request_line = request_line
        self.terminated = terminated
        self.lines: list[bytes] = []
        self.host_closed = False

    async def __call__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        writer.write(self.request_line + (b"\n" if self.terminated else b""))
        await writer.drain()
        while line := await reader.readline():
            self.lines.append(line)
        self.host_closed = True

    @property
    def heartbeats(self) -> list[bytes]:
        return [line for line in self.lines if line == b'{"heartbeat":true}\n']

    @property
    def results(self) -> list[dict[str, object]]:
        return [json.loads(line) for line in self.lines if line != b'{"heartbeat":true}\n']


def serve_attempt(
    request_line: bytes,
    operations: Mapping[str, Operation],
    *,
    interval: float = 1.0,
    limit: int = host.MAX_REQUEST_BYTES,
    token_file: str = TOKEN,
    terminated: bool = True,
) -> Conversation:
    """Runs the host against a relay double that serves the control conversation, and returns what the runner saw."""
    conversation = Conversation(request_line, terminated=terminated)
    relay = FakeRelay(TOKEN, control=conversation)

    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            attempt = Path(directory)
            await relay.start(attempt / "relay.sock")
            (attempt / "relay.token").write_text(token_file + "\n")
            (attempt / "relay.token").chmod(0o400)
            try:
                code = await asyncio.wait_for(
                    host.run(attempt, operations, heartbeat_interval=interval, max_request_bytes=limit),
                    timeout=10,
                )
                assert code == 0
                await asyncio.sleep(0)
            finally:
                await relay.stop()

    asyncio.run(main())
    assert [seen.kind for seen in relay.seen] == ["control"]
    return conversation


def encoded(document: object) -> bytes:
    return json.dumps(document, separators=(",", ":")).encode()


# -- the conversation -------------------------------------------------------------------------------------------------


def test_the_host_authenticates_reads_one_request_and_writes_exactly_one_result_line() -> None:
    seen: list[Mapping[str, object]] = []

    async def probe(received: Mapping[str, object]) -> dict[str, object]:
        seen.append(received)
        return {"kind": "iceberg", "outcome": "COMMITTED"}

    conversation = serve_attempt(encoded(request()), {"probe": probe})

    assert conversation.results == [{"receipt": {"kind": "iceberg", "outcome": "COMMITTED"}, "relayDenials": 0}]
    assert conversation.heartbeats == []
    assert conversation.host_closed
    assert len(seen) == 1
    assert seen[0]["effectKey"] == "run-7f3a/load/1"


def test_the_result_line_is_one_compact_line_with_the_two_members_the_protocol_defines() -> None:
    async def probe(_: Mapping[str, object]) -> dict[str, object]:
        return {"kind": "iceberg", "note": "line one\nline two\u2028three \u00e9"}

    conversation = serve_attempt(encoded(request()), {"probe": probe})

    assert len(conversation.lines) == 1
    assert conversation.lines[0].endswith(b"\n")
    assert conversation.lines[0].count(b"\n") == 1
    # a line separator or an accent stays an escape, so no reader of lines can split the result
    assert conversation.lines[0].isascii()
    assert set(json.loads(conversation.lines[0])) == {"receipt", "relayDenials"}


@pytest.mark.parametrize("text", ["", "short\n", "a" * 129, "has space " + "a" * 30, "colon:" + "a" * 30, "\n"])
def test_a_token_file_that_does_not_hold_a_token_is_refused(text: str) -> None:
    with pytest.raises(ValueError, match="token"):
        host.read_token(text)


def test_the_token_file_holds_the_token_and_a_newline() -> None:
    assert host.read_token(TOKEN + "\n") == TOKEN
    assert host.read_token(TOKEN) == TOKEN


# -- progress lines ---------------------------------------------------------------------------------------------------


def test_a_long_operation_is_announced_by_heartbeats_that_come_before_the_result() -> None:
    async def slow(_: Mapping[str, object]) -> dict[str, object]:
        await asyncio.sleep(0.45)
        return {"kind": "iceberg"}

    conversation = serve_attempt(encoded(request()), {"probe": slow}, interval=0.1)

    assert 3 <= len(conversation.heartbeats) <= 5
    assert conversation.lines[-1] == encoded({"receipt": {"kind": "iceberg"}, "relayDenials": 0}) + b"\n"
    assert all(line == b'{"heartbeat":true}\n' for line in conversation.lines[:-1])


def test_the_heartbeat_is_at_most_one_a_second_by_default() -> None:
    assert host.HEARTBEAT_INTERVAL >= 1.0


def test_a_failing_heartbeat_does_not_outlive_the_result() -> None:
    async def quick(_: Mapping[str, object]) -> dict[str, object]:
        return {"kind": "iceberg"}

    conversation = serve_attempt(encoded(request()), {"probe": quick}, interval=0.05)

    assert conversation.lines[-1].startswith(b'{"receipt"')
    assert len(conversation.results) == 1


# -- failures ---------------------------------------------------------------------------------------------------------


def test_a_failure_of_the_operation_is_reported_with_its_closed_code_column_and_reason() -> None:
    async def refuses(_: Mapping[str, object]) -> dict[str, object]:
        raise Failure("ICEBERG_SCHEMA_MISMATCH", column="amount", reason="SCALE_OVERFLOW")

    conversation = serve_attempt(encoded(request()), {"probe": refuses})

    assert conversation.results == [
        {
            "failure": {"code": "ICEBERG_SCHEMA_MISMATCH", "column": "amount", "reason": "SCALE_OVERFLOW"},
            "relayDenials": 0,
        }
    ]


def test_a_failure_cannot_carry_free_text() -> None:
    for column, reason in [
        ("", None),
        ("c" * 129, None),
        ("a\nb", None),
        (None, "scale overflow"),
        (None, "R" * 49),
        (None, ""),
    ]:
        with pytest.raises(ValueError, match="failure"):
            Failure("ICEBERG_SCHEMA_MISMATCH", column=column, reason=reason)
    for code in ["", "lower_case", "HAS SPACE", "X" * 65, "WITH-DASH"]:
        with pytest.raises(ValueError, match="failure"):
            Failure(code)
    assert Failure("STORAGE_UNAVAILABLE").as_member() == {"code": "STORAGE_UNAVAILABLE"}


def test_an_operation_the_host_does_not_serve_is_a_protocol_failure() -> None:
    conversation = serve_attempt(encoded(request("objectWrite")), {})

    assert conversation.results == [{"failure": {"code": LAKE_HOST_PROTOCOL}, "relayDenials": 0}]


@pytest.mark.parametrize(
    "line",
    [
        b"",
        b"   ",
        b"not json",
        b"[]",
        b"7",
        b"null",
        b"{}",
        encoded({"protocol": "sagat.lake.v2", "operation": "probe"}),
        encoded({"operation": "probe"}),
        encoded({"protocol": "sagat.lake.v1"}),
        encoded({"protocol": "sagat.lake.v1", "operation": 5}),
        encoded({"protocol": "sagat.lake.v1", "operation": ""}),
        b"\xff\xfe{}",
    ],
)
def test_a_request_that_does_not_speak_the_protocol_is_answered_with_one_protocol_failure(line: bytes) -> None:
    async def never(_: Mapping[str, object]) -> dict[str, object]:
        raise AssertionError("not called")

    conversation = serve_attempt(line, {"probe": never})

    assert conversation.results == [{"failure": {"code": LAKE_HOST_PROTOCOL}, "relayDenials": 0}]


def test_a_request_line_above_the_limit_is_refused_without_being_read_to_its_end() -> None:
    async def never(_: Mapping[str, object]) -> dict[str, object]:
        raise AssertionError("not called")

    big = encoded(request(padding="p" * 5000))

    conversation = serve_attempt(big, {"probe": never}, limit=1024)

    assert conversation.results == [{"failure": {"code": LAKE_HOST_PROTOCOL}, "relayDenials": 0}]


def sized(length: int) -> bytes:
    """A request of exactly `length` bytes."""
    base = len(encoded(request(padding="")))
    return encoded(request(padding="p" * (length - base)))


def test_a_request_of_exactly_the_limit_is_served_and_one_byte_more_is_refused() -> None:
    async def probe(_: Mapping[str, object]) -> dict[str, object]:
        return {"kind": "iceberg"}

    exact = serve_attempt(sized(2048), {"probe": probe}, limit=2048)
    over = serve_attempt(sized(2049), {"probe": probe}, limit=2048)

    assert exact.results == [{"receipt": {"kind": "iceberg"}, "relayDenials": 0}]
    assert over.results == [{"failure": {"code": LAKE_HOST_PROTOCOL}, "relayDenials": 0}]


def test_a_request_that_never_ends_is_refused_once_it_passes_the_limit_and_not_waited_for() -> None:
    async def never(_: Mapping[str, object]) -> dict[str, object]:
        raise AssertionError("not called")

    # no newline and the connection stays open: only the bound on the read lets the host answer (the harness fails it
    # after 10 s, and the host's own wait for a request is 30)
    conversation = serve_attempt(b"a" * 4096, {"probe": never}, limit=1024, terminated=False)

    assert conversation.results == [{"failure": {"code": LAKE_HOST_PROTOCOL}, "relayDenials": 0}]


def test_an_unexpected_exception_is_a_worker_error_and_only_its_class_is_written(
    capsys: pytest.CaptureFixture[str],
) -> None:
    async def crashes(received: Mapping[str, object]) -> dict[str, object]:
        raise ValueError(f"{CANARY} {received['connection']}")

    conversation = serve_attempt(encoded(request()), {"probe": crashes})

    assert conversation.results == [{"failure": {"code": WORKER_ERROR}, "relayDenials": 0}]
    captured = capsys.readouterr()
    assert CANARY not in captured.out + captured.err
    assert captured.out == ""
    assert captured.err.strip().splitlines() == ["ValueError"]
    assert CANARY.encode() not in b"".join(conversation.lines)


def test_nothing_of_the_request_reaches_standard_output_or_the_result_when_an_operation_succeeds(
    capsys: pytest.CaptureFixture[str],
) -> None:
    async def probe(_: Mapping[str, object]) -> dict[str, object]:
        return {"kind": "iceberg", "outcome": "COMMITTED"}

    conversation = serve_attempt(encoded(request()), {"probe": probe})

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert CANARY.encode() not in b"".join(conversation.lines)


# -- hardening and the entry point ------------------------------------------------------------------------------------


@pytest.mark.skipif(sys.platform != "linux", reason="PR_SET_DUMPABLE is Linux's")
def test_the_host_stops_a_process_of_the_same_user_from_reading_its_memory() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    pr_get_dumpable, pr_set_dumpable = 3, 4
    before = libc.prctl(pr_get_dumpable, 0, 0, 0, 0)
    try:
        assert REAL_HARDEN() is True
        assert libc.prctl(pr_get_dumpable, 0, 0, 0, 0) == 0
    finally:
        libc.prctl(pr_set_dumpable, before, 0, 0, 0)


def test_hardening_is_the_first_thing_the_entry_point_does(monkeypatch: pytest.MonkeyPatch) -> None:
    order: list[str] = []

    def fake_harden() -> bool:
        order.append("harden")
        return True

    monkeypatch.setattr(host, "harden", fake_harden)

    async def fake_run(*_: object, **__: object) -> int:
        order.append("run")
        return 0

    monkeypatch.setattr(host, "run", fake_run)

    assert host.main(["--attempt-dir", "/tmp/nowhere"]) == 0
    assert order[0] == "harden"


def test_the_entry_point_refuses_arguments_it_does_not_know() -> None:
    assert host.main([]) == 2
    assert host.main(["--attempt-dir"]) == 2
    assert host.main(["--attempt-dir", "/tmp/x", "--debug"]) == 2


def test_the_real_entry_point_runs_isolated_and_answers_an_operation_it_does_not_serve() -> None:
    """`python3 -I` ignores PYTHONPATH and the script's directory, so the entry puts its package on the path."""
    conversation = Conversation(encoded(request("objectWrite")))
    relay = FakeRelay(TOKEN, control=conversation)

    async def main() -> tuple[int, str, str]:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            attempt = Path(directory)
            await relay.start(attempt / "relay.sock")
            (attempt / "relay.token").write_text(TOKEN + "\n")
            try:
                process = await asyncio.create_subprocess_exec(
                    sys.executable,
                    "-I",
                    "-X",
                    "utf8",
                    str(RUNTIME / "sagat_lake" / "host.py"),
                    "--attempt-dir",
                    str(attempt),
                    cwd="/",
                    env={"PATH": os.environ.get("PATH", "")},
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE,
                )
                out, err = await asyncio.wait_for(process.communicate(), timeout=30)
                return process.returncode or 0, out.decode(), err.decode()
            finally:
                await relay.stop()

    code, out, err = asyncio.run(main())

    assert (code, out, err) == (0, "", "")
    assert conversation.results == [{"failure": {"code": LAKE_HOST_PROTOCOL}, "relayDenials": 0}]


def test_the_wire_constants_are_the_relays() -> None:
    assert host.PREFACE == PREFACE
    assert host.PROTOCOL == "sagat.lake.v1"


def test_every_code_the_skeleton_raises_is_in_the_one_table_of_the_domain() -> None:
    table = (ROOT / "crates/domain/src/database/failure.rs").read_text()
    for code in (LAKE_HOST_PROTOCOL, WORKER_ERROR):
        assert re.search(rf'"{code}"', table), code


def test_every_code_the_host_raises_is_one_the_runner_reads() -> None:
    """The runner reads only the codes a host may report (``HOST_MAY_REPORT`` in ``protocol.rs``) and turns any other
    into ``LAKE_HOST_PROTOCOL``. A host that starts to raise a code that is not on that list would have it lost without
    a word, so the code literals of the host's sources must all be on it. (The same is checked from the Rust side; this
    runs when only the host changes.)"""
    table = (ROOT / "crates/domain/src/database/failure.rs").read_text()
    code_of = dict(re.findall(r'^\s*(\w+) => "([A-Z0-9_]+)", (?:true|false);', table, re.MULTILINE))
    protocol = (ROOT / "crates/infra-lake/src/protocol.rs").read_text()
    listed = re.search(r"const HOST_MAY_REPORT: \[DatabaseFailure; \d+\] = \[(.*?)\];", protocol, re.DOTALL)
    assert listed is not None
    assert code_of
    allowed = {code_of[name] for name in re.findall(r"DatabaseFailure::(\w+)", listed.group(1))}
    assert len(allowed) >= 15

    raised: set[str] = set()
    for source in (RUNTIME / "sagat_lake").rglob("*.py"):
        if source.name == "probe_codes.py":
            # the codes a check of a probe may fail with are a result, not a failure line: the server's codec holds
            # them to ``ProbeCheck::allows`` and ``test_probe_operation.py`` holds this file to that table, both ways
            continue
        text = source.read_text(encoding="utf-8")
        raised |= {code for code in code_of.values() if f'"{code}"' in text}
    assert "ICEBERG_TABLE_NOT_FOUND" in raised
    assert raised <= allowed, sorted(raised - allowed)
