"""The ``probe`` operation of the host: does this Iceberg connection work (feature 035: D035-150, T035-040, T035-045).

The server leases a probe for a connection it pinned (``sagat.lake.probe-input.v1``), the runner resolves the connection
and its secrets and writes the request (``request.parse_probe``), and the host runs three checks, in this order:

* ``CATALOG_AUTH``: build the catalog client, which authenticates (OAuth2 gets its token) and reads ``/v1/config``;
* ``WAREHOUSE``: the warehouse the connection names answers, with its first page of namespaces, and the configuration
  the catalog gave places it under the locations the connection declares;
* ``STORAGE_ACCESS``: the storage credentials reach each declared location: one ``HEAD`` of the location and, when that
  finds no object, one list of at most one key (``max-keys=1``), through the same S3 client the writes use. With
  credentials the catalog vends the check is ``SKIPPED``, because nothing is vended without a table, and a skip says
  *not confirmed*.

**A check is the answer.** A connection that does not work is a result: the check ``FAILED`` with a code of the closed
set, every check after it ``SKIPPED``, and the line is a ``receipt``, so the attempt succeeds. What the connection
cannot tell stays a ``failure`` of the line, as for a write: a secret that is missing
(``DATABASE_CREDENTIAL_UNAVAILABLE``), a request that is not one (``LAKE_HOST_PROTOCOL``), and anything nobody expected
(``WORKER_ERROR``, with the name of its class on standard error and never its message).

**The codes are the ones that exist** (the result invents none), and which side failed is said by the check:

====================  ======================================================================================
a catalog that        is ``DATABASE_AUTHENTICATION_FAILED`` (401, 403, an OAuth2 error), ``DATABASE_TLS_FAILED``
                      (a certificate nobody trusts), ``DATABASE_DESTINATION_DENIED`` (the relay refused the
                      tunnel), ``DATABASE_TIMEOUT`` (a request, or the whole check, ran out of time) or
                      ``CATALOG_UNAVAILABLE`` (anything else that stops it: down, 5xx, 429, no route, a body that is
                      not what the protocol says, a warehouse it does not know)
a storage that        is the same four, with ``STORAGE_UNAVAILABLE`` for the last
====================  ======================================================================================

``CHECK_CODES`` is the table of what each check may fail with, held equal to the server's (``ProbeCheck::allows``) by a
test: the server refuses a result that names another code. So the ``WAREHOUSE`` check, which is a second call to the
host the first one reached, never says it was a TLS failure or a refused destination; that host is unavailable.

**Each check has the time the lease gives it** (``checkTimeoutMs``): every request of the catalog client and of the S3
client is bounded by it, and so is the check as a whole, which the S3 client's own retries would otherwise stretch to a
few times that. A check that runs out is ``DATABASE_TIMEOUT``. PyIceberg and PyArrow block, so each check runs on a
thread and the event loop goes on telling the runner the host is alive; a thread left behind by a timeout is bounded by
the requests' own time limits and the runner kills the container once it has the result.

What the providers say is read here and never written: the message of an ``OSError`` of the S3 client is how its
failures are told apart (its ``curlCode``, its ``ACCESS_DENIED``), and it can carry a bucket, a key or a host, so only
the code of the closed set leaves.
"""

from __future__ import annotations

import asyncio
import re
from collections.abc import Awaitable, Callable, Mapping
from functools import partial
from typing import Any

import requests
from pydantic import ValidationError
from pyiceberg.exceptions import (
    AuthorizationExpiredError,
    ForbiddenError,
    NoSuchNamespaceError,
    OAuthError,
    RESTError,
    UnauthorizedError,
)
from pyiceberg.io.pyarrow import PyArrowFileIO

from .config_hygiene import LOCATION_NOT_ALLOWED, HygieneError
from .connection import OpenConnection, open_connection
from .failures import Failure
from .probe_codes import (
    AUTHENTICATION_FAILED,
    CATALOG_AUTH,
    CATALOG_UNAVAILABLE,
    CHECK_CODES,
    DESTINATION_DENIED,
    STORAGE_ACCESS,
    STORAGE_UNAVAILABLE,
    TIMEOUT,
    TLS_FAILED,
    WAREHOUSE,
)
from .request import PROBE_CHECKS, ProbeRequest, parse_probe
from .rest_catalog import LakeRestCatalog

Operation = Callable[[Mapping[str, object]], Awaitable[dict[str, object]]]
# Lists at most one key under a location with the storage credentials of ``properties``, ``seconds`` to a request.
ListStorage = Callable[[Mapping[str, str], str, float], None]

PASSED, FAILED, SKIPPED = "PASSED", "FAILED", "SKIPPED"

# The statuses with which S3 says the credentials are the problem; the others it says are its own.
_AUTHENTICATION_STATUSES = frozenset({400, 401, 403})
# What libcurl, under the AWS SDK, says when TLS is what failed: the handshake, the peer's certificate (the two codes
# that mean it was not trusted, 51 before 7.62 and 60 after), a client certificate, a cipher, a CA file or an issuer.
_CURL_TLS = frozenset({35, 51, 58, 59, 60, 64, 77, 83, 90, 91})
_CURL_TIMEOUT = 28
_CURL_CODE = re.compile(r"curlCode: (\d+)")
_HTTP_STATUS = re.compile(r"HTTP status (\d+)")
_TUNNEL_REFUSED = "Tunnel connection failed: 403"
_MAX_CHAIN = 8


class _CheckFailed(Exception):
    """A check that ran and found the connection does not work: the code of the closed set it fails with."""

    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


class _WarehouseRefused(Exception):
    """The catalog answered, and what it said places the warehouse outside the locations the connection declares."""


def list_one_key(properties: Mapping[str, str], location: str, seconds: float) -> None:
    """Reaches ``location`` with the storage credentials in ``properties`` and lists at most one key under it.

    ``get_file_info`` of a path is one ``HEAD`` of the object of that name and, when there is none, one
    ``ListObjectsV2`` with ``max-keys=1`` and the path as the prefix (a bucket alone is a ``HeadBucket``), so it never
    lists more than a key. Nothing under the location is not a failure: the credentials got there. Every request has
    ``seconds``.
    """
    io = PyArrowFileIO({**properties, "s3.connect-timeout": str(seconds), "s3.request-timeout": str(seconds)})
    scheme, netloc, path = io.parse_location(location, io.properties)
    io.fs_by_scheme(scheme, netloc).get_file_info(path)


def probe_operation(*, list_storage: ListStorage = list_one_key) -> Operation:
    """The operation of a probe. ``list_storage`` is the one thing a test swaps; the catalog client is always real."""

    async def operation(document: Mapping[str, object]) -> dict[str, object]:
        request = parse_probe(document)
        connection = open_connection(request.connection)
        done: list[tuple[str, str, str | None]] = []
        await _run(request.check_timeout, connection, list_storage, done)
        # a check after the one that failed did not run
        done.extend((name, SKIPPED, None) for name in PROBE_CHECKS[len(done) :])
        return _result(request, done)

    return operation


async def _run(
    seconds: float, connection: OpenConnection, list_storage: ListStorage, done: list[tuple[str, str, str | None]]
) -> None:
    """The checks in order, each appended to ``done``, up to the first that fails."""
    try:
        catalog = await _within(seconds, partial(_authenticate, connection, seconds), _catalog_code)
    except _CheckFailed as failed:
        done.append((CATALOG_AUTH, FAILED, failed.code))
        return
    except _WarehouseRefused:
        # the catalog authenticated the client and gave its configuration: that is what the first check asks
        done += [(CATALOG_AUTH, PASSED, None), (WAREHOUSE, FAILED, LOCATION_NOT_ALLOWED)]
        return
    done.append((CATALOG_AUTH, PASSED, None))

    try:
        await _within(seconds, catalog.probe_namespaces, _warehouse_code)
    except _CheckFailed as failed:
        done.append((WAREHOUSE, FAILED, failed.code))
        return
    done.append((WAREHOUSE, PASSED, None))

    if connection.vended:
        # nothing is vended without a table: there is nothing to try the storage with, and that is not a pass
        done.append((STORAGE_ACCESS, SKIPPED, None))
        return
    properties = connection.hygiene.properties(
        {name: value for name, value in connection.properties.items() if name.startswith("s3.")}, {}
    )
    for location in connection.locations:
        try:
            await _within(seconds, partial(list_storage, properties, location, seconds), storage_code)
        except _CheckFailed as failed:
            done.append((STORAGE_ACCESS, FAILED, failed.code))
            return
    done.append((STORAGE_ACCESS, PASSED, None))


def _authenticate(connection: OpenConnection, seconds: float) -> LakeRestCatalog:
    """The catalog client of the connection: it authenticates and reads the configuration as it is built."""
    try:
        return connection.catalog(request_timeout=seconds)
    except HygieneError as error:
        if error.code == LOCATION_NOT_ALLOWED:
            raise _WarehouseRefused from None
        raise Failure(error.code) from None


async def _within[T](seconds: float, work: Callable[[], T], code_of: Callable[[Exception], str | None]) -> T:
    """``work`` on a thread, in at most ``seconds``; a ``_CheckFailed`` with the code ``code_of`` gives its error.

    An error ``code_of`` does not know is nobody's to guess at: it goes on, and the host answers ``WORKER_ERROR``.
    """
    try:
        async with asyncio.timeout(seconds):
            return await asyncio.to_thread(work)
    except TimeoutError:
        raise _CheckFailed(TIMEOUT) from None
    except (Failure, _WarehouseRefused):
        raise
    except Exception as error:
        code = code_of(error)
        if code is None:
            raise
        raise _CheckFailed(code) from None


def _catalog_code(error: Exception) -> str | None:
    """The code of the closed set a failure of the catalog client comes to, or ``None`` for one nobody expected."""
    if isinstance(error, UnauthorizedError | ForbiddenError | OAuthError | AuthorizationExpiredError):
        return AUTHENTICATION_FAILED
    if isinstance(error, requests.exceptions.SSLError):
        return TLS_FAILED
    if isinstance(error, requests.exceptions.ProxyError):
        return DESTINATION_DENIED if _tunnel_refused(error) else CATALOG_UNAVAILABLE
    if isinstance(error, requests.exceptions.Timeout):
        return TIMEOUT
    # a 5xx, a 429, a 404 of a warehouse the catalog does not know, no route, a body that is not what the protocol says
    if isinstance(error, requests.RequestException | RESTError | NoSuchNamespaceError | ValidationError):
        return CATALOG_UNAVAILABLE
    if isinstance(error, NotImplementedError):
        return CATALOG_UNAVAILABLE
    return None


def _warehouse_code(error: Exception) -> str | None:
    """As ``_catalog_code``, but only with what the ``WAREHOUSE`` check may say: the host the first check reached that
    now fails a different way is a host that is unavailable."""
    code = _catalog_code(error)
    if code is None or code in CHECK_CODES[WAREHOUSE]:
        return code
    return CATALOG_UNAVAILABLE


def _tunnel_refused(error: BaseException) -> bool:
    """Whether the proxy, which is the relay, answered the ``CONNECT`` with 403: it refuses what the connection does not
    declare. An unreachable proxy is another thing."""
    current: BaseException | None = error
    for _ in range(_MAX_CHAIN):
        if current is None:
            return False
        if _TUNNEL_REFUSED in str(current):
            return True
        current = current.__cause__ or current.__context__
    return False


def storage_code(error: Exception) -> str | None:
    """The code of the closed set a failure of the S3 client comes to, or ``None`` for one nobody expected.

    PyArrow raises an ``OSError`` whose message is the SDK's: ``AWS Error ACCESS_DENIED``, ``curlCode: 60``,
    ``HTTP status 503``. That text is what tells them apart, so it is read here and never leaves.
    """
    if isinstance(error, TimeoutError):
        return TIMEOUT
    if not isinstance(error, OSError):
        return None
    text = str(error)
    if "CONNECT tunnel failed, response 403" in text:
        return DESTINATION_DENIED
    if curl := _CURL_CODE.search(text):
        if int(curl.group(1)) == _CURL_TIMEOUT:
            return TIMEOUT
        if int(curl.group(1)) in _CURL_TLS:
            return TLS_FAILED
    status = _HTTP_STATUS.search(text)
    if "ACCESS_DENIED" in text or (status is not None and int(status.group(1)) in _AUTHENTICATION_STATUSES):
        return AUTHENTICATION_FAILED
    return STORAGE_UNAVAILABLE


def _result(request: ProbeRequest, done: list[tuple[str, str, str | None]]) -> dict[str, Any]:
    """The typed result the server reads back: the connection's own name and revision, echoed, and each check."""
    return {
        "kind": "probe",
        "connection": {"name": request.name, "revision": request.revision},
        "checks": [
            {"check": check, "outcome": outcome, **({"code": code} if code is not None else {})}
            for check, outcome, code in done
        ],
    }
