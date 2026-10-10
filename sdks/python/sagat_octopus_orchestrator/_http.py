"""The REST transport: one place for headers, error mapping and the retry rules of ``sdks/SPEC.md`` section 5."""

from __future__ import annotations

import asyncio
import logging
import re
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import urlsplit

import httpx

from ._json import decode_json, encode_json
from .config import SDK_VERSION, ClientConfig, ProviderFailure, log, resolve_token, warn_if_plaintext
from .errors import SagatError, code_of_http_status
from .retry import capped_ms, delay_after, parse_ms

# The codes the REST envelope may carry.
_ENVELOPE_CODES = frozenset(
    {
        "BAD_REQUEST",
        "UNAUTHENTICATED",
        "FORBIDDEN",
        "NOT_FOUND",
        "CONFLICT",
        "VALIDATION_FAILED",
        "RATE_LIMITED",
        "QUOTA_EXCEEDED",
        "UNAVAILABLE",
        "INTERNAL",
    }
)
# What may be repeated: the call has an effect at most once (a GET, or a POST with a key) AND the failure
# says nothing was done or it cannot be told (SPEC section 5). Any other answer, 4xx above all, is final.
_REPEATABLE = frozenset({"TRANSPORT", "UNAVAILABLE", "RATE_LIMITED", "INTERNAL"})
_HEX32 = re.compile(r"[0-9a-f]{32}")

MAX_BODY_BYTES = 32 * 1024 * 1024
"""A REST body is read up to this size (decoded), then the call fails as PROTOCOL (``sdks/SPEC.md`` section 10)."""

_UNSET: Any = object()


@dataclass(frozen=True)
class HttpRequest:
    method: str
    path: str
    """Under the base URL, starting with ``/``."""
    query: dict[str, str | int | None] | None = None
    body: Any = field(default=_UNSET, repr=False)
    idempotency_key: str | None = None
    traceparent: str | None = None
    safe_to_repeat: bool = True
    """True for a call whose repetition cannot do harm (a GET, or a POST that carries a key or an event id)."""


@dataclass(frozen=True)
class HttpResponse:
    status: int
    trace_id: str
    body: Any = field(repr=False)


def retry_after_ms(value: str | None) -> int | None:
    """``Retry-After`` as milliseconds: delta-seconds in ASCII digits only. An HTTP date, a fraction, a sign or
    anything else is ignored (no wait). Capped where it is used."""
    if value is None:
        return None
    seconds = parse_ms(value.strip(" \t"))
    return capped_ms(None if seconds is None else seconds * 1000)


class HttpTransport:
    def __init__(self, config: ClientConfig) -> None:
        self._config = config
        self._base = config.api_url.rstrip("/")
        parts = urlsplit(self._base)
        if parts.scheme not in ("http", "https") or not parts.hostname:
            raise SagatError("VALIDATION_FAILED", "api_url is an http(s) URL including /v1")
        warn_if_plaintext(config.logger, config.api_url, "api_url")
        verify: Any = config.tls.ssl_context() if config.tls is not None else True
        # The server name the certificate is verified against, when it is not the URL's host (verified, not skipped).
        self._extensions: dict[str, Any] = (
            {"sni_hostname": config.tls.server_name} if config.tls is not None and config.tls.server_name else {}
        )
        self._client = httpx.AsyncClient(
            transport=config.transport,
            verify=verify,
            follow_redirects=False,
            timeout=httpx.Timeout(config.timeout),
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    async def request(self, req: HttpRequest) -> HttpResponse:
        policy = self._config.retry
        most = max(1, policy.max_attempts)
        attempts = most if req.safe_to_repeat else 1
        last: SagatError | None = None
        for n in range(most):
            try:
                return await self._once(req)
            except SagatError as error:
                last = error
                # A token provider that failed sent nothing, so repeating it cannot do harm, whatever the call is.
                if isinstance(error, ProviderFailure):
                    if n + 1 >= most:
                        raise
                elif not _may_repeat(error) or n + 1 >= attempts:
                    raise
                wait = delay_after(policy, n, error)
                log(
                    self._config.logger,
                    logging.WARNING,
                    "retrying a request",
                    method=req.method,
                    path=req.path,
                    code=error.code,
                    attempt=n + 1,
                    wait_ms=round(wait * 1000),
                )
                await asyncio.sleep(wait)
        raise last or SagatError("INTERNAL", "no attempt was made")

    async def _exchange(self, request: httpx.Request) -> tuple[httpx.Response, bytes]:
        """Sends the request and reads the body, never more than ``MAX_BODY_BYTES`` of it (decoded)."""
        response = await self._client.send(request, stream=True)
        try:
            trace_id = _valid_trace(response.headers.get("x-trace-id", ""))
            declared = parse_ms(response.headers.get("content-length"))
            if declared is not None and declared > MAX_BODY_BYTES:
                raise _too_big(response.status_code, trace_id)
            chunks: list[bytes] = []
            total = 0
            async for chunk in response.aiter_bytes():
                total += len(chunk)
                if total > MAX_BODY_BYTES:
                    raise _too_big(response.status_code, trace_id)
                chunks.append(chunk)
            return response, b"".join(chunks)
        finally:
            await response.aclose()

    async def _once(self, req: HttpRequest) -> HttpResponse:
        token = await resolve_token(self._config.token)
        headers = {
            "authorization": f"Bearer {token}",
            "accept": "application/json",
            "accept-encoding": "identity",
            "x-octopus-namespace": self._config.namespace,
            "user-agent": f"sagat-sdk-python/{SDK_VERSION}"
            + (f" {self._config.user_agent}" if self._config.user_agent else ""),
        }
        if req.traceparent:
            headers["traceparent"] = req.traceparent
        if req.idempotency_key:
            headers["idempotency-key"] = req.idempotency_key
        content: bytes | None = None
        if req.body is not _UNSET:
            content = encode_json(req.body).encode("utf-8")
            headers["content-type"] = "application/json"
        params = {key: value for key, value in (req.query or {}).items() if value is not None} or None

        try:
            request = self._client.build_request(
                req.method,
                self._base + req.path,
                params=params,
                headers=headers,
                content=content,
                extensions=self._extensions,
            )
            response, body = await asyncio.wait_for(self._exchange(request), timeout=self._config.timeout)
        except asyncio.TimeoutError:
            raise SagatError("TRANSPORT", "the request timed out") from None
        except (httpx.HTTPError, OSError):
            # httpx keeps the exception's text (a URL, an address); none of it is worth a place in a message.
            raise SagatError("TRANSPORT", "no answer from the server") from None

        status = response.status_code
        trace_id = _valid_trace(response.headers.get("x-trace-id", ""))
        if 200 <= status < 300:
            if not body:
                return HttpResponse(status, trace_id, None)
            try:
                return HttpResponse(status, trace_id, decode_json(body))
            except ValueError:
                raise SagatError("PROTOCOL", "the answer is not JSON", status=status, trace_id=trace_id) from None
        if 300 <= status < 400:
            # A redirect is never followed (it could carry the token to another host) and never retried.
            raise SagatError("PROTOCOL", "the server redirected the call", status=status, trace_id=trace_id)
        raise self._error_of(status, response.headers, body, trace_id)

    @staticmethod
    def _error_of(status: int, headers: httpx.Headers, body: bytes, trace_id: str) -> SagatError:
        envelope: dict[str, Any] | None = None
        try:
            parsed = decode_json(body)
            if isinstance(parsed, dict):
                envelope = parsed
        except ValueError:
            pass  # an intermediary's page: classify by status
        env = envelope or {}
        raw_code = env.get("code")
        given = raw_code if isinstance(raw_code, str) and raw_code in _ENVELOPE_CODES else None
        code: Any = given or code_of_http_status(status)
        details = env.get("details")
        raw_reason = details.get("reason") if isinstance(details, dict) else None
        message = env.get("message")
        retryable = env.get("retryable")
        envelope_trace = env.get("traceId")
        reason = raw_reason if isinstance(raw_reason, str) else None
        if (
            status == 501
            or (status == 404 and (given is None or reason == "ROUTE_NOT_FOUND"))
            or (status == 405 and reason == "METHOD_NOT_ALLOWED")
        ):
            # A call the server does not have (SPEC section 9): a 501, a 404 for a route, a 405 for a method, or a 404
            # that is not the platform's envelope at all. A 404 for a run that does not exist is a plain NOT_FOUND.
            code = "NOT_FOUND"
            reason = "UNSUPPORTED_BY_SERVER"
            given = None
        return SagatError(
            code,
            message[:512] if isinstance(message, str) and given else f"the server answered {status}",
            status=status,
            retryable=retryable if isinstance(retryable, bool) and given else None,
            trace_id=_valid_trace(envelope_trace) if isinstance(envelope_trace, str) else trace_id,
            reason=reason,
            retry_after_ms=retry_after_ms(headers.get("retry-after")),
        )


def _may_repeat(error: SagatError) -> bool:
    """SPEC section 5: by code (TRANSPORT, UNAVAILABLE, RATE_LIMITED, INTERNAL) and only if the error says it may be
    repeated; a body that claims so on a 4xx other than 408 and 429 is not believed."""
    if error.code not in _REPEATABLE or not error.retryable:
        return False
    status = error.status
    return not (isinstance(status, int) and 400 <= status < 500 and status not in (408, 429))


def _valid_trace(value: str) -> str:
    """A trace id is repeated only if it is 32 lowercase hexadecimal characters; whatever else a server sends is not."""
    return value if _HEX32.fullmatch(value) else ""


def _too_big(status: int, trace_id: str) -> SagatError:
    return SagatError("PROTOCOL", "the answer is over 32 MiB", status=status, trace_id=trace_id)
