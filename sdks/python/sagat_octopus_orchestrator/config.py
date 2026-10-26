"""Configuration and credentials (``sdks/SPEC.md`` section 1)."""

from __future__ import annotations

import inspect
import ipaddress
import logging
import ssl
import tempfile
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any, NoReturn
from urllib.parse import urlsplit

from .errors import SagatError
from .retry import DEFAULT_RETRY, RetryPolicy

if TYPE_CHECKING:
    import httpx

SDK_VERSION = "0.2.0"
"""Sent in ``User-Agent`` as ``sagat-sdk-python/<version>``."""

PROTOCOL_VERSION = "1.5"
"""The newest minor version of worker protocol 1 this SDK speaks (``sdks/SPEC.md`` section 6.1)."""

LANGUAGE = "python"

TokenSource = str | Callable[[], str | Awaitable[str]]
"""A bearer token, or a function called before every request that returns a fresh one (it may be ``async``)."""


class Secret:
    """A credential held where a dump cannot show it: ``repr``, ``str``, f-strings, ``vars()``, ``dataclasses.asdict()``
    and ``json.dumps(..., default=repr)`` of whatever holds it print ``<redacted>``, and it is never pickled.

    ``reveal()`` returns the credential; the SDK calls it only where the credential is sent.
    """

    __slots__ = ("_value",)

    def __init__(self, value: Any) -> None:
        self._value = value

    def reveal(self) -> Any:
        return self._value

    def __repr__(self) -> str:
        return "<redacted>"

    __str__ = __repr__

    def __format__(self, format_spec: str) -> str:
        return "<redacted>"

    def __copy__(self) -> Secret:
        return self

    def __deepcopy__(self, memo: dict[int, Any]) -> Secret:
        return self

    def __reduce__(self) -> NoReturn:
        raise TypeError("a credential is not pickled: pass a token provider to the process that needs one")


def is_provider(token: TokenSource | Secret) -> bool:
    """True when the token is a function that is asked for a fresh one, not a fixed string."""
    value = token.reveal() if isinstance(token, Secret) else token
    return callable(value)


class ProviderFailure(SagatError):
    """The token provider failed: nothing was sent.

    That it was the provider is known to the SDK by this class and nothing else, so a ``reason`` a server puts in an
    answer never makes a call repeatable (``sdks/SPEC.md`` section 5).
    """


TraceContext = Callable[[], str | None]
"""Returns the ambient W3C ``traceparent`` (say, from OpenTelemetry), or ``None``."""

LOG = logging.getLogger("sagat_octopus_orchestrator")
LOG.addHandler(logging.NullHandler())


def log(logger: logging.Logger, level: int, message: str, **fields: object) -> None:
    """A structured log line: ``message key=value ...``, with the fields also in ``record.sagat``.

    The fields are identifiers (run id, task id, node, attempt, trace id, error code), never a payload, a
    token, a lease token or a secret.
    """
    try:
        if logger.isEnabledFor(level):
            text = " ".join(f"{key}={value}" for key, value in fields.items())
            logger.log(level, "%s", f"{message} {text}".rstrip(), extra={"sagat": fields})
    except Exception:
        pass


async def resolve_token(token: TokenSource | Secret) -> str:
    """The token to use right now: the static one, or what the provider returns."""
    revealed: str | Callable[[], str | Awaitable[str]] = token.reveal() if isinstance(token, Secret) else token
    if isinstance(revealed, str):
        return revealed
    try:
        value = revealed()
        if inspect.isawaitable(value):
            value = await value
    except Exception as error:
        # Its own words may carry the credential it was fetching: say what failed, not what it said.
        raise ProviderFailure(
            "UNAUTHENTICATED", f"the token provider failed ({type(error).__name__})", reason="TOKEN_PROVIDER_FAILED"
        ) from None
    if not isinstance(value, str) or value == "":
        raise ProviderFailure("UNAUTHENTICATED", "the token provider returned no token", reason="TOKEN_PROVIDER_FAILED")
    return value


@dataclass(frozen=True)
class TlsOptions:
    """Trust roots and a client certificate (mTLS): PEM text or bytes; the system roots when ``ca`` is absent."""

    ca: str | bytes | None = field(default=None, repr=False)
    cert: str | bytes | None = field(default=None, repr=False)
    key: str | bytes | Secret | None = field(default=None, repr=False)
    server_name: str | None = None
    """The name the certificate is verified against when it is not the URL's host: verified, never skipped."""

    def __post_init__(self) -> None:
        if self.key is not None and not isinstance(self.key, Secret):
            object.__setattr__(self, "key", Secret(self.key))

    @staticmethod
    def from_files(
        ca: str | Path | None = None, cert: str | Path | None = None, key: str | Path | None = None
    ) -> TlsOptions:
        def read(path: str | Path | None) -> bytes | None:
            return Path(path).read_bytes() if path is not None else None

        return TlsOptions(ca=read(ca), cert=read(cert), key=read(key))

    def ssl_context(self) -> ssl.SSLContext:
        """The context for the REST client."""
        cadata = self.ca.decode("ascii") if isinstance(self.ca, bytes) else self.ca
        context = ssl.create_default_context(cadata=cadata)
        if self.cert is not None:
            with (
                tempfile.TemporaryDirectory() as scratch
            ):  # load_cert_chain wants files: 0700 directory, removed at once
                cert_path = Path(scratch) / "cert.pem"
                cert_path.write_bytes(self.cert.encode() if isinstance(self.cert, str) else self.cert)
                key_path = cert_path
                key = self.key_bytes()
                if key is not None:
                    key_path = Path(scratch) / "key.pem"
                    key_path.write_bytes(key)
                context.load_cert_chain(str(cert_path), str(key_path))
        return context

    @staticmethod
    def as_bytes(value: str | bytes | None) -> bytes | None:
        if value is None:
            return None
        return value.encode() if isinstance(value, str) else value

    def key_bytes(self) -> bytes | None:
        """The private key, revealed: only where it is handed to TLS."""
        if self.key is None:
            return None
        revealed = self.key.reveal() if isinstance(self.key, Secret) else self.key
        return self.as_bytes(revealed)


@dataclass(frozen=True)
class ClientConfig:
    """Configuration of ``SagatClient``. Durations are in seconds."""

    api_url: str
    """Base URL of the REST API including ``/v1``."""
    namespace: str
    """Slug of the namespace every call is made in."""
    token: TokenSource | Secret = field(repr=False)
    """A bearer token or a provider; held as a ``Secret`` (``config.token.reveal()`` returns it)."""
    timeout: float = 30.0
    """Deadline of one call (one attempt)."""
    retry: RetryPolicy = field(default_factory=lambda: DEFAULT_RETRY)
    user_agent: str | None = None
    """Appended to ``sagat-sdk-python/<version>``."""
    tls: TlsOptions | None = None
    transport: httpx.AsyncBaseTransport | None = None
    """The HTTP transport; ``httpx``'s own by default. The hook of the conformance suite and of tests."""
    trace_context: TraceContext | None = None
    """Called when a call has no explicit ``traceparent``."""
    logger: logging.Logger = LOG

    def __post_init__(self) -> None:
        if not isinstance(self.token, Secret):
            object.__setattr__(self, "token", Secret(self.token))


def is_loopback(host: str) -> bool:
    """True for ``localhost`` and the loopback addresses."""
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host.strip("[]")).is_loopback
    except ValueError:
        return False


def warn_if_plaintext(logger: logging.Logger, url: str, what: str) -> None:
    """Plain ``http://`` outside loopback is accepted but logs one warning (``sdks/SPEC.md`` section 1)."""
    parts = urlsplit(url)
    if parts.scheme == "http" and not is_loopback(parts.hostname or ""):
        log(
            logger,
            logging.WARNING,
            f"{what} is plain http outside loopback: credentials travel unencrypted",
            host=parts.hostname,
        )
