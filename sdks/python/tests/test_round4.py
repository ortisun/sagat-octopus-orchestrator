"""The follow-up security review (round 4): each of these failed before its fix."""

from __future__ import annotations

import asyncio
import dataclasses
import json
import pickle
import shutil
from collections.abc import Callable
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
    TlsOptions,
    WorkerConfig,
    WorkerHost,
    flow,
    workflow,
)

from .fakes import FakeHttpServer, FakeWorkerService, Scripted, lease, reply, until
from .test_client import RUN, refusal, run_body
from .test_tls import Pair, make_pair, secure_grpc, secure_http
from .test_worker import _secret

MakeHost = Callable[..., WorkerHost]
CANARY = "CANARY-eyJhbGciOiJSUzI1NiJ9-do-not-show"
needs_openssl = pytest.mark.skipif(shutil.which("openssl") is None, reason="needs the openssl command")


# ------------------------------------------------------------------------------ 1. secrets after the handler


async def test_secrets_resolved_after_the_handler_returned_are_dropped_and_refused(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.resolve_delay = 0.15
    fake.secrets = [_secret("stripe-key", b"sk_live_SECRET")]
    host = make_host()
    late: list[asyncio.Future[Any]] = []

    async def pay(_i: Any, ctx: ActivityContext) -> int:
        late.append(asyncio.ensure_future(ctx.secrets()))  # scheduled, not awaited: it outlives the handler
        await asyncio.sleep(0.02)
        return 1

    host.register_activity("pay", pay)
    fake.leases = [lease(activity_name="pay", step_id="pay", task_id="run-1/pay/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    await until(lambda: late[0].done(), "the late resolution", seconds=3)
    error = late[0].exception()
    assert isinstance(error, SagatError), "the secrets of an attempt that is over are not handed out"
    assert host._inflight == set()


async def test_a_first_secrets_call_after_the_handler_returned_does_not_call_resolve_secrets(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    fake.secrets = [_secret("stripe-key", b"sk_live_SECRET")]
    host = make_host()
    kept: list[ActivityContext] = []

    async def pay(_i: Any, ctx: ActivityContext) -> int:
        kept.append(ctx)
        return 1

    host.register_activity("pay", pay)
    fake.leases = [lease(activity_name="pay", step_id="pay", task_id="run-1/pay/1")]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    with pytest.raises(SagatError):
        await kept[0].secrets()
    assert fake.of("ResolveSecrets") == []


# ----------------------------------------------------------------------- 2. a report is not lost to the provider


async def test_a_result_is_reported_even_if_the_token_provider_fails_once_at_report_time(
    fake: FakeWorkerService, make_host: MakeHost, caplog: pytest.LogCaptureFixture
) -> None:
    armed = {"on": False, "raised": 0}

    def provider() -> str:
        if armed["on"] and armed["raised"] == 0:
            armed["raised"] += 1
            raise RuntimeError("identity provider hiccup " + CANARY)
        return "tok"

    async def work(_i: Any, _c: Any) -> dict[str, bool]:
        armed["on"] = True  # the next call to the provider is the one that reports
        return {"ok": True}

    host = make_host(token=provider, heartbeat_interval=60)
    host.register_activity("charge", work)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion, repeated after the provider failed")
    assert armed["raised"] == 1
    assert CANARY not in caplog.text


async def test_a_failure_is_reported_even_if_the_token_provider_fails_once_at_report_time(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    armed = {"on": False, "raised": 0}

    def provider() -> str:
        if armed["on"] and armed["raised"] == 0:
            armed["raised"] += 1
            raise RuntimeError("hiccup")
        return "tok"

    async def work(_i: Any, _c: Any) -> None:
        armed["on"] = True
        raise ValueError("boom")

    host = make_host(token=provider, heartbeat_interval=60)
    host.register_activity("charge", work)
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("FailStep")) == 1, "the failure, repeated after the provider failed")


async def test_a_token_provider_that_fails_is_repeated_before_anything_is_sent_and_then_reported(
    http: FakeHttpServer,
) -> None:
    calls = {"n": 0}

    def flaky() -> str:
        calls["n"] += 1
        if calls["n"] < 3:
            raise RuntimeError("hiccup")
        return "tok"

    http.serve(lambda _seen: reply(200, run_body()))
    client = SagatClient(
        ClientConfig(
            api_url=http.base, namespace="p", token=flaky, retry=RetryPolicy(initial_delay=0.001, max_delay=0.002)
        )
    )
    assert (await client.get(RUN)).run_id
    assert calls["n"] == 3 and len(http.seen) == 1

    def broken() -> str:
        calls["n"] += 1
        raise RuntimeError("down")

    calls["n"] = 0
    dead = SagatClient(
        ClientConfig(
            api_url=http.base, namespace="p", token=broken, retry=RetryPolicy(initial_delay=0.001, max_delay=0.002)
        )
    )
    with pytest.raises(SagatError) as caught:
        await dead.get(RUN)
    assert (caught.value.code, caught.value.reason) == ("UNAUTHENTICATED", "TOKEN_PROVIDER_FAILED")
    assert calls["n"] == 4, "the attempts are used up"
    assert len(http.seen) == 1, "and nothing was sent"
    await client.aclose()
    await dead.aclose()


@pytest.mark.parametrize("status", [400, 401, 403, 409])
async def test_a_reason_a_server_sends_never_makes_a_call_repeatable(http: FakeHttpServer, status: int) -> None:
    body = refusal({400: "BAD_REQUEST", 401: "UNAUTHENTICATED", 403: "FORBIDDEN", 409: "CONFLICT"}[status], "no", True)
    body["details"] = {"reason": "TOKEN_PROVIDER_FAILED"}
    http.serve(lambda _seen: reply(status, body))
    client = SagatClient(
        ClientConfig(
            api_url=http.base, namespace="p", token="t", retry=RetryPolicy(initial_delay=0.001, max_delay=0.002)
        )
    )
    with pytest.raises(SagatError) as caught:
        await client.get(RUN)
    assert caught.value.reason == "TOKEN_PROVIDER_FAILED"
    assert len(http.seen) == 1, f"{status}: not repeated"
    await client.aclose()


@pytest.mark.parametrize(
    "code",
    [
        grpc.StatusCode.UNAUTHENTICATED,
        grpc.StatusCode.PERMISSION_DENIED,
        grpc.StatusCode.ABORTED,
        grpc.StatusCode.INVALID_ARGUMENT,
    ],
)
async def test_a_reason_a_worker_service_sends_never_makes_a_report_repeatable(
    fake: FakeWorkerService, make_host: MakeHost, code: grpc.StatusCode
) -> None:
    host = make_host()
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    fake.always["CompleteStep"] = Scripted(code, "TOKEN_PROVIDER_FAILED")
    fake.leases = [lease()]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) >= 1, "the report")
    await asyncio.sleep(0.2)
    assert len(fake.of("CompleteStep")) == 1


async def test_a_poll_whose_token_fetch_fails_does_not_register_again_as_if_the_session_were_gone(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    state = {"fail_polls": 0}

    def provider() -> str:
        if state["fail_polls"] and fake.of("PollTasks"):
            state["fail_polls"] -= 1
            raise RuntimeError("hiccup")
        return "tok"

    host = make_host(token=provider)
    host.register_activity("charge", lambda _i, _c: {"ok": True}, blocking=False)
    await host.start()
    registers = len(fake.of("RegisterWorker"))
    state["fail_polls"] = 1  # only the poll's own fetch fails: a registration would have succeeded
    fake.leases = [lease()]
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the task after the provider recovered")
    assert len(fake.of("RegisterWorker")) == registers, "the session was never in question"


# --------------------------------------------------------------------------------- 3. credentials in dumps


def leaks(obj: object, needle: str, *, depth: int = 0, seen: set[int] | None = None) -> list[str]:
    """Where, walking ``vars()`` and containers, a plain string holding ``needle`` is reachable from ``obj``."""
    from sagat_octopus_orchestrator.config import Secret

    seen = seen if seen is not None else set()
    if id(obj) in seen or depth > 6:
        return []
    seen.add(id(obj))
    found: list[str] = []
    if isinstance(obj, Secret):
        return found  # a wrapper: what it holds is behind reveal()
    if isinstance(obj, (str, bytes, bytearray)):
        text = obj.decode("latin-1") if isinstance(obj, (bytes, bytearray)) else obj
        return [type(obj).__name__] if needle in text else []
    if isinstance(obj, dict):
        for key, value in obj.items():
            found += [f"[{key!r}]" + path for path in leaks(value, needle, depth=depth + 1, seen=seen)]
        return found
    if isinstance(obj, (list, tuple, set, frozenset)):
        for index, value in enumerate(obj):
            found += [f"[{index}]" + path for path in leaks(value, needle, depth=depth + 1, seen=seen)]
        return found
    if hasattr(obj, "DESCRIPTOR"):  # a protobuf message
        return [type(obj).__name__] if needle in str(obj) else []
    if isinstance(obj, type) or (callable(obj) and not hasattr(obj, "__dict__")):
        return found
    try:
        attributes = vars(obj)
    except TypeError:
        return found
    for name, value in attributes.items():
        found += [f".{name}" + path for path in leaks(value, needle, depth=depth + 1, seen=seen)]
    return found


def forms(thing: object) -> str:
    text = repr(thing) + str(thing) + f"{thing!r}" + f"{thing}"
    if hasattr(thing, "__dict__"):
        text += repr(vars(thing)) + str(vars(thing))
    if dataclasses.is_dataclass(thing) and not isinstance(thing, type):
        text += repr(dataclasses.asdict(thing)) + json.dumps(dataclasses.asdict(thing), default=repr)
    return text


async def test_no_form_of_a_config_a_client_or_a_host_shows_a_credential(
    fake: FakeWorkerService, make_host: MakeHost
) -> None:
    session_canary = "SESSION-" + CANARY
    lease_canary = "LEASE-" + CANARY
    key = "-----BEGIN PRIVATE KEY-----\n" + CANARY + "\n-----END PRIVATE KEY-----"
    tls = TlsOptions(key=key)
    client_config = ClientConfig(api_url="http://127.0.0.1:9/v1", namespace="p", token=CANARY, tls=tls)
    worker_config = WorkerConfig(grpc_url=fake.url, namespace="p", token=CANARY, worker_name="w", tls=tls)
    client = SagatClient(client_config)
    fake.session_token = session_canary
    host = make_host(token=CANARY)
    release = asyncio.Event()

    async def hold(_i: Any, _c: Any) -> int:
        await release.wait()
        return 1

    host.register_activity("charge", hold)
    fake.leases = [lease(lease_token=lease_canary)]
    await host.start()
    await until(lambda: bool(host._inflight), "a task in flight")
    try:
        for thing in (tls, client_config, worker_config, client, host):
            for needle in (CANARY, session_canary, lease_canary):
                assert needle not in forms(thing), (type(thing).__name__, needle)
                assert leaks(thing, needle) == [], (type(thing).__name__, needle, leaks(thing, needle))
        assert "<redacted>" in repr(client_config) or "token" not in repr(client_config)
        assert "<redacted>" in repr(vars(client_config)["token"])
        with pytest.raises(TypeError):
            pickle.dumps(client_config)  # a credential is not written to a pickle
    finally:
        release.set()
        await client.aclose()


async def test_the_wrapped_credentials_still_reach_the_wire(
    fake: FakeWorkerService, make_host: MakeHost, http: FakeHttpServer
) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    client = SagatClient(ClientConfig(api_url=http.base, namespace="p", token=CANARY))
    await client.get(RUN)
    assert http.seen[0].headers["authorization"] == "Bearer " + CANARY
    fake.session_token = "s-" + CANARY
    host = make_host(token=lambda: CANARY)
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    fake.leases = [lease(lease_token="lt-" + CANARY)]
    await host.start()
    await until(lambda: len(fake.of("CompleteStep")) == 1, "the completion")
    assert fake.of("RegisterWorker")[0].metadata["authorization"] == "Bearer " + CANARY
    assert fake.of("PollTasks")[0].request.session_token == "s-" + CANARY
    assert fake.of("CompleteStep")[0].request.lease_token == "lt-" + CANARY
    await client.aclose()


# --------------------------------------------------------------------------------------- 4. the server name


@needs_openssl
async def test_the_rest_client_verifies_the_server_name_it_is_given_and_a_wrong_one_fails(
    http: FakeHttpServer, tmp_path: Path
) -> None:
    pair = make_pair(tmp_path, "named", san="DNS:service.internal")
    secure_http(http, pair, None, tmp_path)
    base = http.base.replace("http://", "https://")  # 127.0.0.1: not a name the certificate has
    http.serve(lambda _seen: reply(200, run_body()))

    def client(**tls: Any) -> SagatClient:
        return SagatClient(
            ClientConfig(
                api_url=base,
                namespace="p",
                token="t",
                tls=TlsOptions(ca=pair.cert, **tls),
                retry=RetryPolicy(max_attempts=1),
            )
        )

    right, wrong, none = client(server_name="service.internal"), client(server_name="other.internal"), client()
    assert (await right.get(RUN)).run_id
    for refused in (wrong, none):
        with pytest.raises(SagatError) as caught:
            await refused.get(RUN)
        assert caught.value.code == "TRANSPORT"
    assert len(http.seen) == 1, "only the verified connection carried a request"
    for c in (right, wrong, none):
        await c.aclose()


@needs_openssl
@pytest.mark.parametrize(("name", "works"), [("service.internal", True), ("other.internal", False), (None, False)])
async def test_the_worker_host_verifies_the_server_name_it_is_given_and_a_wrong_one_fails(
    tmp_path: Path, name: str | None, works: bool
) -> None:
    pair: Pair = make_pair(tmp_path, "named", san="DNS:service.internal")
    service = FakeWorkerService()
    server = await secure_grpc(service, pair, None, bind="127.0.0.1:0")
    host = WorkerHost(
        WorkerConfig(
            grpc_url=f"https://127.0.0.1:{service.port}",
            namespace="p",
            token="t",
            worker_name="w",
            tls=TlsOptions(ca=pair.cert, server_name=name),
            retry=RetryPolicy(max_attempts=1),
            timeout=3,
        )
    )
    host.register_activity("charge", lambda _i, _c: 1, blocking=False)
    try:
        if works:
            await host.start()
            assert service.of("RegisterWorker")
        else:
            with pytest.raises(SagatError) as caught:
                await host.start()
            assert caught.value.code == "TRANSPORT" and not service.calls
    finally:
        await host.stop(1)
        await server.stop(None)


def test_there_is_no_option_that_skips_verification() -> None:
    names = {field.name for field in dataclasses.fields(TlsOptions)}
    assert names == {"ca", "cert", "key", "server_name"}


# ------------------------------------------------------------------------------ 5. a flow is read at build


def test_a_step_added_to_a_branch_or_a_saga_body_after_it_was_joined_is_part_of_the_document() -> None:
    branch, other, body = flow().step("a", "x.a"), flow().step("b", "x.b"), flow().step("s1", "x.s1")
    builder = workflow("late-flow").parallel("p", "all", [branch, other]).saga("tx", body).step("done", "x.done")
    before = builder.build()
    assert [n["id"] for n in before["nodes"]] == ["p", "tx", "done", "a", "b", "s1"]
    branch.step("a2", "x.a2")
    body.step("s2", "x.s2", compensate_with="x.undo")
    other.sleep("b-pause", 100)
    after = builder.build()
    nodes = {n["id"]: n for n in after["nodes"]}
    assert nodes["a"]["next"] == "a2" and "next" not in nodes["a2"]
    assert nodes["s1"]["next"] == "s2" and nodes["s2"]["compensateWith"] == "x.undo"
    assert nodes["b"]["next"] == "b-pause"
    assert [n["id"] for n in after["nodes"]] == ["p", "tx", "done", "a", "a2", "b", "b-pause", "s1", "s2"]
    assert (
        builder.to_json()
        == workflow("late-flow")
        .parallel(
            "p", "all", [flow().step("a", "x.a").step("a2", "x.a2"), flow().step("b", "x.b").sleep("b-pause", 100)]
        )
        .saga("tx", flow().step("s1", "x.s1").step("s2", "x.s2", compensate_with="x.undo"))
        .step("done", "x.done")
        .to_json()
    )


def test_an_id_that_collides_after_a_flow_was_joined_is_caught_when_added_or_at_build() -> None:
    branch = flow().step("a", "x.a")
    builder = workflow("late-dup").parallel("p", "all", [branch, flow().step("b", "x.b")])
    builder.build()
    branch.step("b", "x.again")  # the same id as the other branch's node: only build() sees both
    with pytest.raises(SagatError) as caught:
        builder.build()
    assert caught.value.code == "VALIDATION_FAILED" and "used twice" in caught.value.message
    own = flow().step("a", "x.a")
    with pytest.raises(SagatError) as same:
        own.step("a", "x.a2")  # in its own flow it is caught at once
    assert "used twice" in same.value.message
    outer = flow()
    outer.saga("s", flow().step("inner", "x.i"))
    with pytest.raises(SagatError) as nested:
        outer.step("inner", "x.j")  # an id of a flow joined to this one counts too
    assert "used twice" in nested.value.message
