"""What a hostile or broken server, and a careless caller, must not be able to do (``sdks/SPEC.md`` sections 1, 6, 10)."""

from __future__ import annotations

import asyncio
import logging
import re
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

import grpc
import pytest

from sagat_octopus_orchestrator import (
    ActivityContext,
    ClientConfig,
    RetryPolicy,
    SagatClient,
    SagatError,
    SyncSagatClient,
    TlsOptions,
    WorkerConfig,
    WorkerHost,
    _http,
    check_traceparent,
    is_traceparent,
    parse_instant,
    workflow,
)
from sagat_octopus_orchestrator.retry import delay_after
from sagat_octopus_orchestrator.worker import host as host_module
from sagat_octopus_orchestrator.worker._grpc import GrpcConfig, error_of_status

from .conftest import FAST
from .fakes import FakeHttpServer, FakeWorkerService, Reply, Scripted, lease, reply, until
from .test_client import RUN, WORKFLOW, MakeClient, refusal, run_body

PACKAGE = Path(__file__).resolve().parent.parent
MakeHost = Callable[..., WorkerHost]
CANARY = "CANARY-eyJhbGciOiJSUzI1NiJ9-do-not-print"


# ----------------------------------------------------------------------------------- 1. the retry storm


async def test_a_poll_that_keeps_being_refused_as_unauthenticated_does_not_hammer_the_server(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.always["PollTasks"] = Scripted(grpc.StatusCode.UNAUTHENTICATED)
    host = make_host(retry=RetryPolicy())  # the defaults: 4 attempts, 200 ms to 5 s
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    await asyncio.sleep(2)
    registers = len(fake.of("RegisterWorker"))
    assert 2 <= registers <= 10, f"{registers} registrations in 2 s"


async def test_the_first_refusal_registers_again_at_once_and_a_successful_poll_resets_the_backoff(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host(retry=RetryPolicy())
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    await host.start()
    fake.failures["PollTasks"] = [Scripted(grpc.StatusCode.UNAUTHENTICATED)]
    await until(lambda: len(fake.of("RegisterWorker")) == 2, "the registration after the first refusal")
    refused = [call for call in fake.of("PollTasks") if call.at <= fake.of("RegisterWorker")[1].at][-1]
    assert fake.of("RegisterWorker")[1].at - refused.at < 0.1, "no wait the first time"
    await until(lambda: len(fake.of("PollTasks")) >= 3, "a successful poll after it")
    fake.failures["PollTasks"] = [Scripted(grpc.StatusCode.UNAUTHENTICATED), Scripted(grpc.StatusCode.UNAUTHENTICATED)]
    await until(lambda: len(fake.of("RegisterWorker")) == 4, "two more registrations")
    # The refusal after a successful poll registered at once (reset); the second one in a row waited first.
    polls = fake.of("PollTasks")
    registers = fake.of("RegisterWorker")
    first_refused = [call for call in polls if call.at <= registers[2].at][-1]
    assert registers[2].at - first_refused.at < 0.1, "a refusal after a successful poll registers at once"
    assert registers[3].at - registers[2].at >= 0.08, "the second refusal in a row waited the backoff"


# ------------------------------------------------------------------------------------ 2. credentials in repr


def shown(thing: object) -> str:
    return repr(thing) + str(thing) + f"{thing!r}" + f"{thing}"


async def test_no_config_client_or_host_prints_a_credential(fake: FakeWorkerService) -> None:
    key = "-----BEGIN PRIVATE KEY-----\n" + CANARY + "\n-----END PRIVATE KEY-----"
    tls = TlsOptions(key=key)
    client_config = ClientConfig(api_url="http://127.0.0.1:9/v1", namespace="p", token=CANARY, tls=tls)
    worker_config = WorkerConfig(grpc_url=fake.url, namespace="p", token=CANARY, worker_name="w", tls=tls)
    client = SagatClient(client_config)
    host = WorkerHost(worker_config)
    host.register_activity("a", lambda _i, _c: 1)
    for thing in (
        tls,
        client_config,
        worker_config,
        GrpcConfig(grpc_url=fake.url, token=CANARY, tls=tls),
        client,
        host,
    ):
        assert CANARY not in shown(thing), type(thing).__name__
    await host.start()
    try:
        assert CANARY not in shown(host)
        assert "session-1" not in shown(host), "nor the session token"
    finally:
        await host.stop(1)
        await client.aclose()


def test_the_blocking_client_prints_no_credential(http: FakeHttpServer) -> None:
    with SyncSagatClient(ClientConfig(api_url=http.base, namespace="p", token=CANARY)) as client:
        assert CANARY not in shown(client)


async def test_a_lease_token_is_not_in_the_repr_of_the_state_of_a_task(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    seen: list[str] = []
    host.register_activity("charge", lambda _i, _c: seen.append(shown(next(iter(host._inflight)))), blocking=False)
    fake.leases = [lease(lease_token=CANARY)]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert seen and CANARY not in seen[0]


# ------------------------------------------------------------------------------------ 3. the heartbeat lives


async def test_a_token_provider_that_raises_once_does_not_kill_the_heartbeat(
    fake: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    raised = False

    def provider() -> str:
        nonlocal raised
        if fake.of("Heartbeat") and not raised:
            raised = True
            raise RuntimeError("vault said " + CANARY)
        return "tok"

    host = make_host(token=provider, heartbeat_interval=0.02)

    async def slow(_i: Any, _c: Any) -> int:
        await asyncio.sleep(0.4)
        return 1

    host.register_activity("charge", slow)
    fake.leases = [lease()]
    with caplog.at_level(logging.DEBUG, logger="sagat_octopus_orchestrator"):
        await host.start()
        await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert raised
    assert len(fake.of("Heartbeat")) >= 5, "the heartbeat went on after the provider failed once"
    assert CANARY not in caplog.text, "the text of the exception is never logged"


async def test_a_heartbeat_survives_any_exception_not_only_ours(fake: FakeWorkerService, make_host: MakeHost) -> None:
    host = make_host(heartbeat_interval=0.02)
    host.register_activity("charge", lambda _i, _c: asyncio.sleep(0.3), blocking=False)
    fake.leases = [lease()]
    await host.start()
    assert host._rpc is not None
    real = host._rpc.heartbeat
    broken = {"left": 1}

    async def flaky(*args: Any, **kwargs: Any) -> Any:
        if broken["left"]:
            broken["left"] -= 1
            raise RuntimeError("a bug in a hook " + CANARY)
        return await real(*args, **kwargs)

    host._rpc.heartbeat = flaky  # type: ignore[method-assign]
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert len(fake.of("Heartbeat")) >= 3


async def test_a_token_provider_that_fails_is_one_sagat_error_with_no_text_of_its_own(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    def broken() -> str:
        raise RuntimeError("could not reach the vault with " + CANARY)

    http.serve(lambda _seen: reply(200, run_body()))
    with pytest.raises(SagatError) as caught:
        await make_client(token=broken).get(RUN)
    assert caught.value.code == "UNAUTHENTICATED" and CANARY not in repr(caught.value.to_dict())
    assert http.seen == []


# ------------------------------------------------------------------------------------------ 4. names and ids


async def test_a_run_id_is_a_uuid_and_a_name_in_a_path_is_never_dot_dotdot_or_empty(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    client = make_client()
    bad_ids = ("..", ".", "", "not-a-uuid", RUN + "\n", "../x", RUN + "/events", "%2e%2e")
    calls: list[tuple[str, Callable[[str], Any]]] = [
        ("get", lambda rid: client.get(rid)),
        ("events", lambda rid: client.events(rid)),
        ("cancel", lambda rid: client.cancel(rid, "no longer needed")),
        ("operate", lambda rid: client.operate(rid, "PAUSE", "maintenance window")),
        ("signal", lambda rid: client.signal(rid, "go", {})),
        ("wait", lambda rid: client.wait_for_completion(rid, timeout=1)),
    ]
    for name, call in calls:
        for bad in bad_ids:
            with pytest.raises(SagatError) as caught:
                await call(bad)
            assert caught.value.code == "VALIDATION_FAILED", (name, bad)
    for bad_name in ("..", ".", ""):
        with pytest.raises(SagatError) as signal_name:
            await client.signal(RUN, bad_name, {})
        assert signal_name.value.code == "VALIDATION_FAILED", bad_name
        with pytest.raises(SagatError) as event_type:
            await client.publish_event(bad_name, {})
        assert event_type.value.code == "VALIDATION_FAILED", bad_name
        with pytest.raises(SagatError) as workflow_name:
            await client.start(bad_name, {})
        assert workflow_name.value.code == "VALIDATION_FAILED", bad_name
    assert http.seen == [], "nothing was sent"
    assert (await client.get(RUN.upper())).run_id, "a UUID in either case is a UUID"


# ----------------------------------------------------------------------------------------- 5. a bounded body


def endless(chunk: int, total: int) -> Iterator[bytes]:
    yield b'{"pad":"'
    sent = 0
    while sent < total:
        yield b"x" * chunk
        sent += chunk
    yield b'"}'


async def test_a_rest_body_is_read_up_to_32_mib_and_then_the_call_fails_as_protocol(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    assert _http.MAX_BODY_BYTES == 32 * 1024 * 1024
    http.serve(lambda _seen: Reply(200, stream=endless(1024 * 1024, 40 * 1024 * 1024)))
    with pytest.raises(SagatError) as caught:
        await make_client(retry=RetryPolicy(max_attempts=1), timeout=60).get(RUN)
    assert (caught.value.code, caught.value.status) == ("PROTOCOL", 200)
    assert "32 MiB" in caught.value.message


async def test_a_declared_length_over_the_cap_fails_before_the_body_is_read(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: Reply(200, headers={"content-length": str(64 * 1024 * 1024)}, stream=iter([b"{"])))
    with pytest.raises(SagatError) as caught:
        await make_client(retry=RetryPolicy(max_attempts=1)).get(RUN)
    assert caught.value.code == "PROTOCOL"


async def test_a_body_just_under_the_cap_is_read(http: FakeHttpServer, make_client: MakeClient) -> None:
    body = run_body(pad="x" * (1024 * 1024))
    http.serve(lambda _seen: reply(200, body))
    assert (await make_client().get(RUN)).raw["pad"] == "x" * (1024 * 1024)


# ------------------------------------------------------------------------------------ 6. max in flight


async def test_a_poll_answer_with_more_tasks_than_were_asked_for_runs_no_more_than_the_room_there_is(
    fake: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    fake.ignore_max_tasks = True
    host = make_host(max_inflight=2)
    running = peak = 0

    async def charge(_i: Any, ctx: ActivityContext) -> int:
        nonlocal running, peak
        running += 1
        peak = max(peak, running)
        await asyncio.sleep(0.15)
        running -= 1
        return 1

    host.register_activity("charge", charge)
    fake.leases = [lease(task_id=f"run-{i}/charge/1", run_id=f"run-{i}", input_json="SECRET-INPUT") for i in range(6)]
    with caplog.at_level(logging.DEBUG, logger="sagat_octopus_orchestrator"):
        await host.start()
        await until(lambda: len(fake.of("CompleteStep")) + len(fake.of("FailStep")) >= 2, "the first two tasks")
        await asyncio.sleep(0.3)
    assert peak <= 2
    assert len(fake.of("CompleteStep")) + len(fake.of("FailStep")) == 2, "the other four were left to lapse"
    assert "4" in caplog.text and "SECRET-INPUT" not in caplog.text and "lt" not in caplog.text.split()


# ------------------------------------------------------------------------------- 7. what the server says


def test_a_wait_the_server_asks_for_is_capped_at_60_seconds() -> None:
    policy = RetryPolicy()
    huge = SagatError("RATE_LIMITED", "slow", retry_after_ms=86_400_000)
    assert delay_after(policy, 0, huge) <= 60.0
    assert delay_after(
        policy, 0, SagatError("RATE_LIMITED", "slow", retry_after_ms=1500), lambda: 1.0
    ) == pytest.approx(1.5)


async def test_retry_after_of_a_day_is_waited_for_60_seconds_at_most(
    http: FakeHttpServer, make_client: MakeClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    slept: list[float] = []
    real_sleep = asyncio.sleep

    async def recording(seconds: float, *args: Any) -> None:
        slept.append(seconds)
        await real_sleep(0)

    answers = [reply(429, refusal("RATE_LIMITED", "slow", True), retry_after="86400"), reply(200, run_body())]
    http.serve(lambda _seen: answers.pop(0))
    monkeypatch.setattr(asyncio, "sleep", recording)
    await make_client().get(RUN)
    assert slept and max(slept) <= 60.0


def test_a_retry_after_header_is_read_as_digits_only_and_capped_where_it_is_used() -> None:
    assert _http.retry_after_ms("12\n") is None
    assert _http.retry_after_ms("١٢") is None, "Arabic-Indic digits are not delay-seconds"
    assert _http.retry_after_ms("30") == 30_000


def test_a_grpc_retry_after_that_is_not_ascii_digits_is_ignored_not_a_crash() -> None:
    def failure(value: str) -> grpc.aio.AioRpcError:
        return grpc.aio.AioRpcError(
            grpc.StatusCode.RESOURCE_EXHAUSTED,
            grpc.aio.Metadata(),
            grpc.aio.Metadata(("retry-after-ms", value)),
            details="x",
        )

    assert error_of_status("PollTasks", failure("²")).retry_after_ms is None
    assert error_of_status("PollTasks", failure("1500")).retry_after_ms == 1500


async def test_the_pause_after_an_empty_poll_is_capped_at_60_seconds(
    fake: FakeWorkerService, make_host: MakeHost, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake.poll_retry_after_ms = 2_000_000_000
    host = make_host()
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    asked: list[float] = []
    original = host._idle

    async def recording(seconds: float) -> None:
        asked.append(seconds)
        await original(0.01)

    monkeypatch.setattr(host, "_idle", recording)
    await host.start()
    await until(lambda: bool(asked), "a pause")
    assert max(asked) <= 60.0


async def test_the_message_of_a_server_error_is_cut_and_a_trace_id_is_repeated_only_if_it_is_32_hex_characters(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    async def failing(trace: Any, header: str | None = None) -> SagatError:
        body = refusal("FORBIDDEN", "m" * 5000)
        body["traceId"] = trace
        http.serve(lambda _seen: reply(403, body, **({"x_trace_id": header} if header else {})))
        with pytest.raises(SagatError) as caught:
            await make_client().get(RUN)
        return caught.value

    long = await failing("a" * 32)
    assert len(long.message) == 512 and long.trace_id == "a" * 32
    for bad in ("<script>alert(1)</script>", "A" * 32, "a" * 31, "a" * 33, "a" * 32 + "\n", "g" * 32):
        assert (await failing(bad)).trace_id == "", repr(bad)
    assert (await failing(12345)).trace_id == "f" * 32, "not a string: the header (valid here) is used instead"
    assert (await failing("ab\ncd" * 20)).trace_id == ""
    assert (await failing(None, header="b" * 32)).trace_id == "b" * 32, "the header, when the envelope has none"
    assert (await failing(None, header="not a trace")).trace_id == ""
    assert (await failing(None, header="B" * 32)).trace_id == ""


# ------------------------------------------------------------------------------------- 8. anchored patterns


async def test_a_trailing_newline_is_not_accepted_by_a_pattern(http: FakeHttpServer, make_client: MakeClient) -> None:
    http.serve(lambda _seen: reply(202, run_body()))
    client = make_client()
    with pytest.raises(SagatError) as key:
        await client.start(WORKFLOW, {}, idempotency_key="abcdefgh\n")
    assert key.value.code == "VALIDATION_FAILED"
    with pytest.raises(SagatError) as trace:
        await client.start(WORKFLOW, {}, traceparent="00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01\n")
    assert trace.value.code == "BAD_REQUEST"
    assert http.seen == [], "nothing was sent"


def test_a_trailing_newline_is_not_accepted_in_a_name_a_secret_a_traceparent_or_an_instant() -> None:
    with pytest.raises(SagatError):
        workflow("abc\n")
    with pytest.raises(SagatError):
        workflow("abc-flow").step("a", "x", secrets=["payments\n"])
    good = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
    assert is_traceparent(good) and not is_traceparent(good + "\n")
    with pytest.raises(SagatError):
        check_traceparent(good + "\n")
    assert parse_instant("2026-10-10T10:00:00Z") is not None and parse_instant("2026-10-10T10:00:00Z\n") is None
    assert host_module._minor_of("1.2") == 2 and host_module._minor_of("1.2\n") == 0


# --------------------------------------------------------------------------------------------- 9. supply chain


def test_every_pin_of_the_lock_files_carries_hashes_and_the_installer_requires_them() -> None:
    for name in ("requirements.lock.txt", "requirements-dev.lock.txt"):
        text = (PACKAGE / name).read_text(encoding="utf-8")
        entries = re.split(r"\n(?=[A-Za-z0-9_.-]+==)", text)
        pinned = [entry for entry in entries if re.match(r"[A-Za-z0-9_.-]+==", entry)]
        assert len(pinned) >= 8, name
        for entry in pinned:
            assert "--hash=sha256:" in entry, f"{name}: {entry.splitlines()[0]} has no hash"
    prepare = (PACKAGE / "conformance" / "prepare.sh").read_text(encoding="utf-8")
    assert "--require-hashes" in prepare


# ------------------------------------------------------------------------------- 6.7 secrets, and the scheme


async def test_secrets_are_copies_wiped_when_the_handler_returns_and_refused_afterwards(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    kept: list[Any] = []

    async def pay(_i: Any, ctx: ActivityContext) -> int:
        secrets = await ctx.secrets()
        value = secrets.get("stripe-key")
        assert value == b"sk_live_x" and secrets.get("stripe-key") is not value, "a copy each time"
        assert secrets._values is not None
        kept.extend([ctx, secrets, secrets._values["stripe-key"]])
        return 1

    host.register_activity("pay", pay)
    from .test_worker import _secret

    fake.secrets = [_secret("stripe-key", b"sk_live_x")]
    fake.leases = [lease(activity_name="pay", step_id="pay", task_id="run-1/pay/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    ctx, secrets, stored = kept
    assert bytes(stored) == b"\x00" * 9, "the buffer is wiped, not only forgotten"
    with pytest.raises(SagatError):
        await ctx.secrets()
    with pytest.raises(SagatError):
        secrets.get("stripe-key")


async def test_a_failed_resolution_is_not_cached_for_the_rest_of_the_attempt(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    host = make_host()
    outcomes: list[str] = []

    async def pay(_i: Any, ctx: ActivityContext) -> int:
        for _ in range(2):
            try:
                outcomes.append((await ctx.secrets()).text("stripe-key"))
            except SagatError as error:
                outcomes.append(error.code)
        return 1

    host.register_activity("pay", pay)
    from .test_worker import _secret

    fake.secrets = [_secret("stripe-key", b"sk_live_x")]
    fake.failures["ResolveSecrets"] = [Scripted(grpc.StatusCode.PERMISSION_DENIED, "SECRET_NOT_USABLE")]
    fake.leases = [lease(activity_name="pay", step_id="pay", task_id="run-1/pay/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert outcomes == ["SECRET_NOT_USABLE", "sk_live_x"]


async def test_a_worker_url_with_another_scheme_is_refused_and_never_plaintext(fake: FakeWorkerService) -> None:
    for url in (
        "grpc://127.0.0.1:5000",
        "ftp://127.0.0.1:5000",
        "127.0.0.1:5000",
        "unix:///tmp/x.sock",
        "dns:///localhost:5000",
    ):
        host = WorkerHost(WorkerConfig(grpc_url=url, namespace="p", token="t", worker_name="w", retry=FAST))
        host.register_activity("a", lambda _i, _c: 1)
        with pytest.raises(SagatError) as caught:
            await host.start()
        assert caught.value.code == "VALIDATION_FAILED", url
    assert not fake.calls
