"""The REST client against a fake HTTP server in the same process."""

from __future__ import annotations

import logging
import re
from collections.abc import Callable
from datetime import datetime, timedelta, timezone
from typing import Any

import httpx
import pytest

from sagat_octopus_orchestrator import ClientConfig, RetryPolicy, Run, SagatClient, SagatError, parse_instant

from .fakes import FakeHttpServer, Seen, reply

WORKFLOW = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee"
RUN = "0190aaaa-0000-7000-8000-000000000001"
TRACE = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
MakeClient = Callable[..., SagatClient]


def run_body(**overrides: Any) -> dict[str, Any]:
    return {
        "runId": "0190aaaa-0000-7000-8000-000000000001",
        "workflowId": WORKFLOW,
        "versionId": "0190aaaa-0000-7000-8000-000000000002",
        "status": "RUNNING",
        "version": 3,
        "paused": False,
        "startedBy": "alice",
        "createdAt": "2026-10-10T10:00:00Z",
        "updatedAt": "2026-10-10T10:00:01Z",
        "finishedAt": None,
        "deadline": None,
        "traceId": "abcdefabcdefabcdefabcdefabcdefab",
        "correlationId": None,
        "depth": 0,
        "parent": None,
        "futureField": 7,
        **overrides,
    }


def refusal(code: str, message: str = "no", retryable: bool = False, **extra: Any) -> dict[str, Any]:
    return {"code": code, "message": message, "retryable": retryable, "traceId": "e" * 32, **extra}


async def test_a_start_sends_the_headers_and_the_body_the_api_expects_and_returns_a_typed_run_that_keeps_the_raw_answer(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(202, run_body()))
    result = await make_client().start(
        WORKFLOW,
        {"orderId": "o-1"},
        idempotency_key="key-12345678",
        correlation_id="corr-1",
        traceparent=TRACE,
        deadline=datetime(2030, 1, 1, tzinfo=timezone.utc),
    )
    [request] = http.seen
    assert (request.method, request.url) == ("POST", "/v1/runs")
    assert request.headers["authorization"] == "Bearer tok-1"
    assert request.headers["x-octopus-namespace"] == "payments"
    assert request.headers["idempotency-key"] == "key-12345678"
    assert request.headers["traceparent"] == TRACE
    assert request.headers["content-type"] == "application/json"
    assert re.match(r"^sagat-sdk-python/\d+\.\d+\.\d+", request.headers["user-agent"])
    assert request.body == {
        "workflowId": WORKFLOW,
        "input": {"orderId": "o-1"},
        "correlationId": "corr-1",
        "deadline": "2030-01-01T00:00:00.000Z",
    }
    assert result.status == "RUNNING" and not result.is_terminal
    assert result.created_at == datetime(2026, 10, 10, 10, 0, tzinfo=timezone.utc)
    assert result.created_at.utcoffset() == timedelta(0)
    assert result.raw["futureField"] == 7, "a field of a newer server is reachable"


async def test_a_call_without_a_key_gets_a_fresh_one_per_call_and_keeps_it_across_its_retries(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    attempts = 0

    def serve(_seen: Seen) -> Any:
        nonlocal attempts
        attempts += 1
        return reply(503, refusal("UNAVAILABLE", "busy", True)) if attempts < 3 else reply(202, run_body())

    http.serve(serve)
    await make_client().start(WORKFLOW, {})
    assert len(http.seen) == 3
    keys = {seen.headers["idempotency-key"] for seen in http.seen}
    assert len(keys) == 1, "the same key on every attempt"
    (key,) = keys
    assert re.fullmatch(r"[0-9a-f-]{36}", key)
    attempts, first = 0, http.seen[:]
    http.seen.clear()
    await make_client().start(WORKFLOW, {})
    assert http.seen[0].headers["idempotency-key"] != key, "another call, another key"
    assert first[0].headers["idempotency-key"] == key


async def test_an_event_without_an_id_gets_one_per_call_kept_across_retries_so_a_repetition_is_a_duplicate_not_a_second_event(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    attempts = 0

    def serve(_seen: Seen) -> Any:
        nonlocal attempts
        attempts += 1
        return (
            reply(503, "upstream down")
            if attempts < 2
            else reply(202, {"eventId": "e", "source": "api", "status": "ACCEPTED"})
        )

    http.serve(serve)
    accepted = await make_client().publish_event("order.created", {"id": 1})
    assert accepted.status == "ACCEPTED"
    assert len(http.seen) == 2
    ids = [seen.body["eventId"] for seen in http.seen]
    assert ids[0] == ids[1]
    assert re.fullmatch(r"[0-9a-f-]{36}", ids[0])


async def test_an_approval_decision_uses_its_body_key_and_returns_a_typed_receipt(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    approval_id = "0190aaaa-0000-7000-8000-000000000099"
    http.serve(
        lambda _seen: reply(
            200,
            {
                "receiptId": "0190aaaa-0000-7000-8000-000000000100",
                "approvalId": approval_id,
                "decisionKey": "review-1",
                "decision": "APPROVE",
                "reasonCode": "POLICY_OK",
                "actor": "alice",
                "decidedAt": "2026-10-15T10:00:00Z",
                "outcomeSequence": 7,
                "replayed": False,
                "futureField": "kept",
            },
        )
    )
    receipt = await make_client().decide_approval(
        approval_id, "APPROVE", decision_key="review-1", reason_code="POLICY_OK", traceparent=TRACE
    )
    [request] = http.seen
    assert request.url == f"/v1/approvals/{approval_id}/decisions"
    assert request.body == {"decisionKey": "review-1", "decision": "APPROVE", "reasonCode": "POLICY_OK"}
    assert "idempotency-key" not in request.headers
    assert receipt.decided_at == datetime(2026, 10, 15, 10, 0, tzinfo=timezone.utc)
    assert receipt.outcome_sequence == 7 and receipt.raw["futureField"] == "kept"


async def test_approval_pages_are_typed_and_all_pages_follow_the_opaque_cursor(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    approval_ids = ["0190aaaa-0000-7000-8000-000000000099", "0190aaaa-0000-7000-8000-000000000100"]

    def serve(seen: Seen) -> Any:
        second = "cursor=next" in seen.url
        approval_id = approval_ids[1 if second else 0]
        return reply(
            200,
            {
                "items": [
                    {
                        "approvalId": approval_id,
                        "runId": RUN,
                        "nodeId": "review",
                        "instancePath": [{"scopeNodeId": "items", "iterationIndex": 3}],
                        "status": "APPROVED" if second else "PENDING",
                        "deadline": "2026-10-16T10:00:00Z",
                        "settledAt": "2026-10-15T10:00:00Z" if second else None,
                    }
                ],
                "nextCursor": None if second else "next",
            },
        )

    http.serve(serve)
    client = make_client()
    page = await client.list_approvals(RUN, cursor="from", limit=1, traceparent=TRACE)
    assert page.items[0].status == "PENDING"
    assert page.items[0].instance_path[0].iteration_index == 3
    assert page.items[0].deadline == datetime(2026, 10, 16, 10, 0, tzinfo=timezone.utc)
    assert http.seen[0].url == f"/v1/runs/{RUN}/approvals?cursor=from&limit=1"
    http.seen.clear()
    items = [item async for item in client.list_all_approvals(RUN, limit=1)]
    assert [item.approval_id for item in items] == approval_ids
    assert http.seen[1].url == f"/v1/runs/{RUN}/approvals?cursor=next&limit=1"


async def test_an_approval_decision_surfaces_authorization_and_idempotency_failures_and_rejects_bad_input(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    approval_id = "0190aaaa-0000-7000-8000-000000000099"
    client = make_client()

    def denied(status: int, code: str, reason: str) -> Callable[[Seen], Any]:
        return lambda _seen: reply(status, refusal(code, details={"reason": reason}))

    for status, code, reason in ((403, "FORBIDDEN", "NOT_ELIGIBLE"), (409, "CONFLICT", "IDEMPOTENCY_KEY_REUSED")):
        http.serve(denied(status, code, reason))
        with pytest.raises(SagatError) as caught:
            await client.decide_approval(approval_id, "REJECT", decision_key="review-1")
        assert (caught.value.status, caught.value.code, caught.value.reason) == (status, code, reason)
    http.serve(lambda _seen: reply(500))
    with pytest.raises(SagatError):
        await client.decide_approval("../other", "APPROVE", decision_key="review-1")
    with pytest.raises(SagatError):
        await client.decide_approval(approval_id, "APPROVE", decision_key="bad key")
    assert not http.seen


async def test_declared_query_and_update_preserve_revisions_result_and_update_key(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    def serve(seen: Seen) -> Any:
        if "/queries/" in seen.url:
            return reply(200, {"runId": RUN, "runVersion": 4, "stateRevision": 1, "result": {"balance": 12}})
        return reply(
            200,
            {
                "runId": RUN,
                "operation": "account.credit",
                "updateKey": "credit-1",
                "status": "APPLIED",
                "result": {"balance": 15},
                "beforeStateRevision": 1,
                "afterStateRevision": 2,
                "runVersion": 5,
                "replayed": False,
                "future": True,
            },
        )

    http.serve(serve)
    client = make_client()
    queried = await client.query(RUN, "account.balance", {"currency": "BRL"})
    updated = await client.update(
        RUN, "account.credit", {"amount": 3}, update_key="credit-1", expected_state_revision=1, traceparent=TRACE
    )
    assert [seen.url for seen in http.seen] == [
        f"/v1/runs/{RUN}/queries/account.balance",
        f"/v1/runs/{RUN}/updates/account.credit",
    ]
    assert http.seen[0].body == {"args": {"currency": "BRL"}}
    assert http.seen[1].body == {"updateKey": "credit-1", "args": {"amount": 3}, "expectedStateRevision": 1}
    assert "idempotency-key" not in http.seen[1].headers
    assert queried.state_revision == 1 and queried.result == {"balance": 12}
    assert updated.after_state_revision == 2 and updated.raw["future"] is True


async def test_declared_update_retries_with_the_same_body_key_and_rejects_bad_input_locally(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    attempts = 0

    def serve(_seen: Seen) -> Any:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            return reply(503, refusal("UNAVAILABLE", "busy", True))
        return reply(
            200,
            {
                "runId": RUN,
                "operation": "account.credit",
                "updateKey": "credit-1",
                "status": "APPLIED",
                "result": 15,
                "beforeStateRevision": 1,
                "afterStateRevision": 2,
                "runVersion": 5,
                "replayed": False,
            },
        )

    http.serve(serve)
    client = make_client()
    await client.update(RUN, "account.credit", {"amount": 3}, update_key="credit-1")
    assert len(http.seen) == 2 and http.seen[0].body == http.seen[1].body
    http.seen.clear()
    with pytest.raises(SagatError):
        await client.query(RUN, "bad operation")
    with pytest.raises(SagatError):
        await client.update(RUN, "credit", {}, update_key="bad key")
    with pytest.raises(SagatError):
        await client.update(RUN, "credit", {}, update_key="credit-2", expected_state_revision=-1)
    assert not http.seen


async def test_chain_inspection_and_commands_use_expected_head_and_durable_keys(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    namespace_id = "0190aaaa-0000-7000-8000-000000000010"
    chain_id = "0190aaaa-0000-7000-8000-000000000011"
    head = "0190aaaa-0000-7000-8000-000000000012"

    def serve(seen: Seen) -> Any:
        if "/signals/" in seen.url:
            return reply(202, {"deliveredTo": ["wait"]})
        if seen.url.endswith("/cancel"):
            return reply(202)
        return reply(
            200,
            {
                "chainId": chain_id,
                "currentRunId": head,
                "revision": 2,
                "cancelRequested": False,
                "finalOutcome": None,
                "segments": [
                    {"index": 0, "runId": chain_id, "predecessorRunId": None, "successorRunId": head},
                    {"index": 1, "runId": head, "predecessorRunId": chain_id, "successorRunId": None},
                ],
            },
        )

    http.serve(serve)
    client = make_client()
    inspection = await client.inspect_chain(namespace_id, chain_id)
    signalled = await client.signal_chain(
        namespace_id, chain_id, "chain.release", {"ok": True}, expected_run_id=head, idempotency_key="chain-signal-1"
    )
    await client.cancel_chain(
        namespace_id,
        chain_id,
        "operator requested cancellation",
        expected_run_id=head,
        idempotency_key="chain-cancel-1",
    )
    assert inspection.segments[1].predecessor_run_id == chain_id and signalled.delivered_to == ("wait",)
    assert http.seen[1].body == {"expectedRunId": head, "payload": {"ok": True}}
    assert http.seen[1].headers["idempotency-key"] == "chain-signal-1"
    assert http.seen[2].body == {"expectedRunId": head, "reason": "operator requested cancellation"}


async def test_get_is_retried_on_unavailability_and_on_rate_limiting_honouring_retry_after_and_gives_up_after_the_attempts(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    attempts = 0

    def serve(_seen: Seen) -> Any:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            return reply(429, refusal("RATE_LIMITED", "slow", True), retry_after="0")
        return reply(200, run_body())

    http.serve(serve)
    got = await make_client().get(RUN)
    assert got.run_id == "0190aaaa-0000-7000-8000-000000000001"
    assert len(http.seen) == 2

    http.serve(lambda _seen: reply(503, "<html>bad gateway</html>"))
    with pytest.raises(SagatError) as caught:
        await make_client().get(RUN)
    assert (caught.value.code, caught.value.status) == ("UNAVAILABLE", 503)
    assert len(http.seen) == 4, "four attempts in all"


async def test_retry_after_longer_than_the_backoff_is_waited_for(http: FakeHttpServer, make_client: MakeClient) -> None:
    answers = [reply(429, refusal("RATE_LIMITED", "slow", True), retry_after="1"), reply(200, run_body())]
    http.serve(lambda _seen: answers.pop(0))
    began = asyncio_time()
    await make_client().get(RUN)
    assert asyncio_time() - began >= 0.9


def asyncio_time() -> float:
    import time

    return time.monotonic()


async def test_a_4xx_other_than_429_is_never_retried_and_keeps_what_the_server_said(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    body = refusal(
        "CONFLICT", "the idempotency key was used for another request", details={"reason": "IDEMPOTENCY_KEY_REUSED"}
    )
    body["traceId"] = "c" * 32
    http.serve(lambda _seen: reply(409, body))
    with pytest.raises(SagatError) as caught:
        await make_client().start(WORKFLOW, {}, idempotency_key="key-12345678")
    error = caught.value
    assert (error.code, error.reason, error.trace_id, error.retryable) == (
        "CONFLICT",
        "IDEMPOTENCY_KEY_REUSED",
        "c" * 32,
        False,
    )
    assert error.remediation
    assert len(http.seen) == 1

    # Even an envelope that says "retryable" does not make a 400 repeatable.
    http.serve(lambda _seen: reply(400, refusal("BAD_REQUEST", "bad", True)))
    with pytest.raises(SagatError):
        await make_client().get(RUN)
    assert len(http.seen) == 1


async def test_a_redirect_is_never_followed_and_is_a_protocol_error(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    elsewhere = FakeHttpServer()
    try:
        elsewhere.serve(lambda _seen: reply(200, run_body()))
        http.serve(lambda _seen: reply(302, "", location=elsewhere.base + "/runs/r"))
        client = make_client()
        for call in (client.get(RUN), client.start(WORKFLOW, {}), client.cancel(RUN, "no longer needed")):
            with pytest.raises(SagatError) as caught:
                await call
            assert (caught.value.code, caught.value.status, caught.value.retryable) == ("PROTOCOL", 302, False)
            assert caught.value.message == "the server redirected the call"
        assert len(http.seen) == 3, "a redirect is not repeated"
        assert elsewhere.seen == [], "and the other server never saw the call or the token"
    finally:
        elsewhere.close()


async def test_a_call_the_server_does_not_have_is_told_from_a_resource_that_is_missing(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    for status, body in ((501, ""), (404, "<html>no route</html>"), (404, {"unrelated": True})):
        http.serve(lambda _seen, status=status, body=body: reply(status, body))  # type: ignore[misc]
        with pytest.raises(SagatError) as caught:
            await make_client().get(RUN)
        assert (caught.value.code, caught.value.reason, caught.value.status) == (
            "NOT_FOUND",
            "UNSUPPORTED_BY_SERVER",
            status,
        )
    http.serve(lambda _seen: reply(404, refusal("NOT_FOUND", "no such run")))
    with pytest.raises(SagatError) as missing:
        await make_client().get(RUN)
    assert (missing.value.code, missing.value.reason, missing.value.message) == ("NOT_FOUND", None, "no such run")


async def test_a_refused_token_a_missing_resource_and_a_validation_error_are_told_apart(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    for status, code in ((401, "UNAUTHENTICATED"), (403, "FORBIDDEN"), (404, "NOT_FOUND"), (422, "VALIDATION_FAILED")):
        http.serve(lambda _seen, status=status, code=code: reply(status, refusal(code)))  # type: ignore[misc]
        with pytest.raises(SagatError) as caught:
            await make_client().get(RUN)
        assert (caught.value.code, caught.value.status) == (code, status)
        assert len(http.seen) == 1, str(status)


async def test_the_token_is_asked_for_before_every_request_so_a_rotated_one_is_used_at_once(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    count = 0

    def provider() -> str:
        nonlocal count
        count += 1
        return f"tok-{count}"

    async def async_provider() -> str:
        return "tok-async"

    client = make_client(token=provider)
    await client.get(RUN)
    await client.get(RUN)
    await make_client(token=async_provider).get(RUN)
    assert [seen.headers["authorization"] for seen in http.seen] == ["Bearer tok-1", "Bearer tok-2", "Bearer tok-async"]


async def test_a_name_is_resolved_to_the_id_of_its_workflow_through_every_page_and_remembered(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    def serve(seen: Seen) -> Any:
        if seen.url.startswith("/v1/workflows"):
            if "cursor=c2" in seen.url:
                return reply(200, {"items": [{"id": WORKFLOW, "name": "order-flow"}], "nextCursor": None})
            return reply(
                200, {"items": [{"id": "0190aaaa-0000-7000-8000-00000000ffff", "name": "other"}], "nextCursor": "c2"}
            )
        return reply(202, run_body())

    http.serve(serve)
    client = make_client()
    await client.start("order-flow", {})
    await client.start("order-flow", {})
    lookups = [seen for seen in http.seen if seen.url.startswith("/v1/workflows")]
    assert len(lookups) == 2, "two pages, once"
    posted = next(seen for seen in http.seen if seen.method == "POST")
    assert posted.body["workflowId"] == WORKFLOW

    http.serve(lambda _seen: reply(200, {"items": [], "nextCursor": None}))
    with pytest.raises(SagatError) as caught:
        await make_client().start("nothing-here", {})
    assert (caught.value.code, caught.value.reason) == ("NOT_FOUND", "WORKFLOW_NOT_FOUND")


async def test_a_malformed_traceparent_or_key_is_refused_before_anything_is_sent(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(202, run_body()))
    client = make_client()
    with pytest.raises(SagatError) as bad_trace:
        await client.start(WORKFLOW, {}, traceparent="not-a-trace")
    assert bad_trace.value.code == "BAD_REQUEST"
    with pytest.raises(SagatError) as bad_key:
        await client.start(WORKFLOW, {}, idempotency_key="short")
    assert bad_key.value.code == "VALIDATION_FAILED"
    with pytest.raises(SagatError) as naive:
        await client.start(WORKFLOW, {}, deadline=datetime(2030, 1, 1))
    assert naive.value.code == "VALIDATION_FAILED"
    with pytest.raises(SagatError):
        await client.start(WORKFLOW, {}, deadline=datetime(2030, 1, 1, tzinfo=timezone.utc), deadline_in=5)
    assert http.seen == []


async def test_the_ambient_trace_context_is_used_when_the_call_has_none(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(lambda _seen: reply(200, run_body()))
    await make_client(trace_context=lambda: TRACE).get(RUN)
    assert http.seen[0].headers["traceparent"] == TRACE
    await make_client(trace_context=lambda: None).get(RUN)
    assert "traceparent" not in http.seen[1].headers


async def test_a_list_follows_its_cursor_a_deadline_given_as_a_duration_is_computed_once_and_an_unsafe_integer_is_refused(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda seen: (
            reply(200, {"items": [run_body(runId="r-2")], "nextCursor": None})
            if "cursor=n2" in seen.url
            else reply(200, {"items": [run_body(runId="r-1")], "nextCursor": "n2"})
        )
    )
    ids = [item.run_id async for item in make_client().list_all(status="RUNNING", limit=1)]
    assert ids == ["r-1", "r-2"], "the order the server gave, across pages"
    assert "status=RUNNING" in http.seen[0].url and "limit=1" in http.seen[0].url

    http.serve(lambda _seen: reply(202, run_body()))
    before = datetime.now(timezone.utc)
    await make_client().start(WORKFLOW, {}, deadline_in=60)
    sent = parse_instant(http.seen[0].body["deadline"])
    assert sent is not None
    assert before + timedelta(seconds=60) <= sent < before + timedelta(seconds=61)
    assert http.seen[0].body["deadline"].endswith("Z")
    await make_client().start(WORKFLOW, {}, deadline_in=timedelta(minutes=1))

    for bad in (9007199254740993 * 2**20, float("nan")):
        with pytest.raises(SagatError) as caught:
            await make_client().start(WORKFLOW, {"big": bad})
        assert caught.value.code == "VALIDATION_FAILED"


async def test_no_answer_at_all_is_a_retryable_transport_error_and_a_lost_answer_is_retried_only_because_the_call_carries_a_key(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    class Lossy(httpx.AsyncBaseTransport):
        def __init__(self) -> None:
            self.inner = httpx.AsyncHTTPTransport()
            self.calls = 0

        async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
            self.calls += 1
            response = await self.inner.handle_async_request(request)
            if self.calls == 1:
                # The request reached the server (it is recorded) and the answer was lost.
                await response.aread()
                raise httpx.ReadError("connection reset", request=request)
            return response

        async def aclose(self) -> None:
            await self.inner.aclose()

    http.serve(lambda _seen: reply(202, run_body()))
    result = await make_client(transport=Lossy()).start(WORKFLOW, {}, idempotency_key="key-12345678")
    assert result.status == "RUNNING"
    assert len(http.seen) == 2
    assert http.seen[0].headers["idempotency-key"] == http.seen[1].headers["idempotency-key"]

    dead = SagatClient(
        ClientConfig(
            api_url="http://127.0.0.1:1/v1",
            namespace="p",
            token="t",
            retry=RetryPolicy(max_attempts=2, initial_delay=0.001, max_delay=0.002),
        )
    )
    with pytest.raises(SagatError) as caught:
        await dead.get(RUN)
    await dead.aclose()
    assert (caught.value.code, caught.value.retryable, caught.value.status) == ("TRANSPORT", True, None)


async def test_a_call_that_takes_longer_than_the_timeout_is_a_transport_error(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    import time

    def slow(_seen: Seen) -> Any:
        time.sleep(0.3)
        return reply(200, run_body())

    http.serve(slow)
    with pytest.raises(SagatError) as caught:
        await make_client(timeout=0.05, retry=RetryPolicy(max_attempts=1)).get(RUN)
    assert (caught.value.code, caught.value.retryable) == ("TRANSPORT", True)


async def test_cancel_and_signal_are_operations_with_a_reason_and_a_payload_and_the_result_is_typed(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda seen: (
            reply(202, {"operationId": "op-1", "accepted": True, "runStatus": "CANCELLING", "runVersion": 9})
            if seen.url.endswith("/operations")
            else reply(202, {"deliveredTo": ["wait"]})
        )
    )
    client = make_client()
    cancelled = await client.cancel(RUN, "no longer needed", expected_run_version=3)
    assert (cancelled.operation_id, cancelled.accepted, cancelled.run_status, cancelled.run_version) == (
        "op-1",
        True,
        "CANCELLING",
        9,
    )
    assert http.seen[0].body == {"action": "CANCEL", "reason": "no longer needed", "expectedRunVersion": 3}
    assert "idempotency-key" in http.seen[0].headers
    signalled = await client.signal(RUN, "payment-confirmed", {"paymentId": "p-1"})
    assert signalled.delivered_to == ("wait",)
    assert http.seen[1].url == f"/v1/runs/{RUN}/signals/payment-confirmed"
    assert http.seen[1].body == {"paymentId": "p-1"}
    await client.operate(RUN, "PAUSE", "maintenance window")
    assert http.seen[2].url == f"/v1/runs/{RUN}/operations"
    await client.signal(RUN, "a/b c", {})
    assert http.seen[3].url == f"/v1/runs/{RUN}/signals/a%2Fb%20c", "a name is a path segment, whatever it holds"


async def test_the_journal_of_a_run_is_read_a_page_at_a_time(http: FakeHttpServer, make_client: MakeClient) -> None:
    http.serve(
        lambda _seen: reply(
            200,
            {
                "items": [
                    {"seq": 1, "eventType": "run.started", "stepId": None, "attempt": None},
                    {"seq": 2, "eventType": "step.leased", "stepId": "a", "attempt": 1},
                ],
                "nextCursor": "c",
            },
        )
    )
    page = await make_client().events(RUN, limit=2)
    assert [(e.event_type, e.step_id, e.attempt) for e in page.items] == [
        ("run.started", None, None),
        ("step.leased", "a", 1),
    ]
    assert page.next_cursor == "c"
    assert "limit=2" in http.seen[0].url


async def test_waiting_for_completion_polls_until_the_run_is_terminal(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    polls = 0

    def serve(_seen: Seen) -> Any:
        nonlocal polls
        polls += 1
        return reply(200, run_body(status="RUNNING" if polls < 3 else "SUCCEEDED"))

    http.serve(serve)
    done = await make_client().wait_for_completion(RUN, poll_interval=0.005, timeout=5)
    assert done.status == "SUCCEEDED" and done.is_terminal
    assert polls == 3
    http.serve(lambda _seen: reply(200, run_body()))
    with pytest.raises(SagatError) as caught:
        await make_client().wait_for_completion(RUN, poll_interval=0.005, timeout=0.03)
    assert caught.value.reason == "WAIT_TIMEOUT"


async def test_a_run_with_a_status_a_newer_server_adds_and_unknown_fields_is_still_read(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    http.serve(
        lambda _seen: reply(
            200,
            run_body(
                status="QUARANTINED",
                parent={"runId": "p", "node": "n", "cancelPolicy": "DETACH"},
                deadline="2030-01-01T05:30:00+05:30",
            ),
        )
    )
    run: Run = await make_client().get(RUN)
    assert run.status == "QUARANTINED" and not run.is_terminal
    assert run.parent is not None and run.parent.cancel_policy == "DETACH"
    assert run.deadline == datetime(2030, 1, 1, tzinfo=timezone.utc)


async def test_an_answer_that_is_not_what_the_contract_says_is_a_protocol_error(
    http: FakeHttpServer, make_client: MakeClient
) -> None:
    for body in ("not json", "[1, 2]", {"runId": "r"}):
        http.serve(lambda _seen, body=body: reply(200, body))  # type: ignore[misc]
        with pytest.raises(SagatError) as caught:
            await make_client().get(RUN)
        assert caught.value.code == "PROTOCOL"


async def test_errors_and_logs_never_carry_a_token_or_a_payload(
    http: FakeHttpServer, make_client: MakeClient, caplog: pytest.LogCaptureFixture
) -> None:
    http.serve(lambda _seen: reply(503, "echo: Bearer tok-secret payload-marker"))
    with caplog.at_level(logging.DEBUG, logger="sagat_octopus_orchestrator"), pytest.raises(SagatError) as caught:
        await make_client(token="tok-secret").start(WORKFLOW, {"marker": "payload-marker"})
    everything = repr(caught.value.to_dict()) + caplog.text
    assert "tok-secret" not in everything and "payload-marker" not in everything
    assert "retrying a request" in caplog.text


async def test_plain_http_outside_loopback_warns_once_at_construction(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING, logger="sagat_octopus_orchestrator"):
        remote = SagatClient(ClientConfig(api_url="http://api.example.com/v1", namespace="p", token="t"))
        local = SagatClient(ClientConfig(api_url="http://127.0.0.1:9/v1", namespace="p", token="t"))
        secure = SagatClient(ClientConfig(api_url="https://api.example.com/v1", namespace="p", token="t"))
    assert [record.getMessage().split(" is plain")[0] for record in caplog.records] == ["api_url"]
    for client in (remote, local, secure):
        await client.aclose()


async def test_a_configuration_that_cannot_work_is_refused_at_construction() -> None:
    for config in (
        ClientConfig(api_url="", namespace="p", token="t"),
        ClientConfig(api_url="ftp://x/v1", namespace="p", token="t"),
        ClientConfig(api_url="http://x/v1", namespace="", token="t"),
    ):
        with pytest.raises(SagatError) as caught:
            SagatClient(config)
        assert caught.value.code == "VALIDATION_FAILED"


@pytest.mark.parametrize("value", [True, 1.5])
async def test_run_input_expected_size_is_a_runtime_integer(value: object, make_client: MakeClient) -> None:
    with pytest.raises(SagatError) as caught:
        await make_client().upload_run_input([b"body"], expected_size=value)  # type: ignore[arg-type]
    assert caught.value.code == "VALIDATION_FAILED"
