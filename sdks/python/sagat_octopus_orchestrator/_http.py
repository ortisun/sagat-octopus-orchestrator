"""The REST transport: one place for headers, error mapping and the retry rules of ``sdks/SPEC.md`` section 5."""

from __future__ import annotations

import asyncio
import logging
import re
from dataclasses import dataclass
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from typing import Any
from urllib.parse import urlsplit

import httpx

from ._json import decode_json, encode_json
from .config import SDK_VERSION, ClientConfig, log, resolve_token, warn_if_plaintext
from .errors import SagatError, code_of_http_status
from .retry import delay_after

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
_DIGITS = re.compile(r"^\d+$")

_UNSET: Any = object()


@dataclass(frozen=True)
class HttpRequest:
    method: str
    path: str
    """Under the base URL, starting with ``/``."""
    query: dict[str, str | int | None] | None = None
    body: Any = _UNSET
    idempotency_key: str | None = None
    traceparent: str | None = None
    safe_to_repeat: bool = True
    """True for a call whose repetition cannot do harm (a GET, or a POST that carries a key or an event id)."""


@dataclass(frozen=True)
class HttpResponse:
    status: int
    trace_id: str
    body: Any


def retry_after_ms(value: str | None) -> int | None:
    """``Retry-After`` as milliseconds: delta-seconds or an HTTP date."""
    if value is None:
        return None
    value = value.strip()
    if _DIGITS.match(value):
        return int(value) * 1000
    try:
        moment = parsedate_to_datetime(value)
    except (TypeError, ValueError):
        return None
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return max(0, int((moment - datetime.now(timezone.utc)).total_seconds() * 1000))


class HttpTransport:
    def __init__(self, config: ClientConfig) -> None:
        self._config = config
        self._base = config.api_url.rstrip("/")
        parts = urlsplit(self._base)
        if parts.scheme not in ("http", "https") or not parts.hostname:
            raise SagatError("VALIDATION_FAILED", "api_url is an http(s) URL including /v1")
        warn_if_plaintext(config.logger, config.api_url, "api_url")
        verify: Any = config.tls.ssl_context() if config.tls is not None else True
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
        attempts = max(1, policy.max_attempts) if req.safe_to_repeat else 1
        last: SagatError | None = None
        for n in range(attempts):
            try:
                return await self._once(req)
            except SagatError as error:
                last = error
                if error.code not in _REPEATABLE or not error.retryable or n + 1 >= attempts:
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

    async def _once(self, req: HttpRequest) -> HttpResponse:
        token = await resolve_token(self._config.token)
        headers = {
            "authorization": f"Bearer {token}",
            "accept": "application/json",
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
            response = await asyncio.wait_for(
                self._client.request(
                    req.method, self._base + req.path, params=params, headers=headers, content=content
                ),
                timeout=self._config.timeout,
            )
        except asyncio.TimeoutError:
            raise SagatError("TRANSPORT", "the request timed out") from None
        except (httpx.HTTPError, OSError):
            # httpx keeps the exception's text (a URL, an address); none of it is worth a place in a message.
            raise SagatError("TRANSPORT", "no answer from the server") from None

        trace_id = response.headers.get("x-trace-id", "")
        if 200 <= response.status_code < 300:
            if not response.content:
                return HttpResponse(response.status_code, trace_id, None)
            try:
                return HttpResponse(response.status_code, trace_id, decode_json(response.content))
            except ValueError:
                raise SagatError(
                    "PROTOCOL", "the answer is not JSON", status=response.status_code, trace_id=trace_id
                ) from None
        if 300 <= response.status_code < 400:
            # A redirect is never followed (it could carry the token to another host) and never retried.
            raise SagatError(
                "PROTOCOL", "the server redirected the call", status=response.status_code, trace_id=trace_id
            )
        raise self._error_of(response, trace_id)

    @staticmethod
    def _error_of(response: httpx.Response, trace_id: str) -> SagatError:
        envelope: dict[str, Any] | None = None
        try:
            parsed = decode_json(response.content)
            if isinstance(parsed, dict):
                envelope = parsed
        except ValueError:
            pass  # an intermediary's page: classify by status
        env = envelope or {}
        raw_code = env.get("code")
        given = raw_code if isinstance(raw_code, str) and raw_code in _ENVELOPE_CODES else None
        code: Any = given or code_of_http_status(response.status_code)
        details = env.get("details")
        raw_reason = details.get("reason") if isinstance(details, dict) else None
        message = env.get("message")
        retryable = env.get("retryable")
        envelope_trace = env.get("traceId")
        reason = raw_reason if isinstance(raw_reason, str) else None
        status = response.status_code
        if status == 501 or (status == 404 and given is None):
            # A call the server does not have (an older server, SPEC section 9) is told from a resource that is
            # missing by the lack of the platform's own error envelope.
            code = "NOT_FOUND"
            reason = reason or "UNSUPPORTED_BY_SERVER"
            given = None
        return SagatError(
            code,
            message[:512] if isinstance(message, str) and given else f"the server answered {status}",
            status=status,
            retryable=retryable if isinstance(retryable, bool) and given else None,
            trace_id=envelope_trace if isinstance(envelope_trace, str) else trace_id,
            reason=reason,
            retry_after_ms=retry_after_ms(response.headers.get("retry-after")),
        )
