"""The ``icebergWrite`` operation of the host (feature 035: T035-057, G035-047, D035-106).

It is the join of what the other modules do, in the order the protocol asks: parse the request line
(``request.parse_write``), open the catalog of the pinned connection (``connection.open_connection``), and append the
records of ``in/`` (``modes.append.append``), answering with the receipt. Everything it can say about a failure is a
code of the closed set.

PyIceberg blocks, and the host must go on telling the runner that it is alive, so the blocking work runs on a thread and
the event loop keeps writing heartbeats.

What PyIceberg and the catalog raise is mapped to a code at this one place:

* a catalog that refuses the client (401, 403, an OAuth2 error) is ``DATABASE_AUTHENTICATION_FAILED``;
* a catalog that is down, slow or overloaded (a connection that fails, a time limit, 5xx, 429) is
  ``CATALOG_UNAVAILABLE``;
* a table or a namespace the catalog does not have is ``ICEBERG_TABLE_NOT_FOUND``;
* a location the hygiene refuses carries the hygiene's own code;
* a storage that cannot be read or written is ``STORAGE_UNAVAILABLE``;
* anything else is not guessed at: the host answers ``WORKER_ERROR`` and writes the name of the class to standard error,
  never its message, which could carry a credential.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable, Mapping
from pathlib import Path
from typing import Any

import requests
from pyiceberg.catalog import Catalog
from pyiceberg.exceptions import (
    AuthorizationExpiredError,
    ForbiddenError,
    NoSuchIdentifierError,
    NoSuchNamespaceError,
    NoSuchTableError,
    OAuthError,
    ServerError,
    ServiceUnavailableError,
    TooManyRequestsError,
    UnauthorizedError,
)

from .config_hygiene import HygieneError
from .connection import open_connection
from .failures import Failure
from .modes.append import append
from .request import parse_write

Operation = Callable[[Mapping[str, object]], Awaitable[dict[str, object]]]
# Opens the catalog of a connection (the document of the request) with a time limit for each request it makes.
CatalogOpener = Callable[[Any, float], Catalog]

# How long one request to the catalog may take. The deadline of the attempt is the runner's, which kills the container.
CATALOG_TIMEOUT = 30.0

TABLE_NOT_FOUND = "ICEBERG_TABLE_NOT_FOUND"
AUTHENTICATION_FAILED = "DATABASE_AUTHENTICATION_FAILED"
CATALOG_UNAVAILABLE = "CATALOG_UNAVAILABLE"
STORAGE_UNAVAILABLE = "STORAGE_UNAVAILABLE"


def open_lake_catalog(connection: Any, request_timeout: float) -> Catalog:
    """The catalog of the pinned connection: the one that believes nothing its server says."""
    return open_connection(connection).catalog(request_timeout=request_timeout)


def iceberg_write(
    attempt_dir: Path, *, request_timeout: float = CATALOG_TIMEOUT, open_catalog: CatalogOpener = open_lake_catalog
) -> Operation:
    """The operation for the attempt whose directory is ``attempt_dir``."""

    async def operation(document: Mapping[str, object]) -> dict[str, object]:
        request = parse_write(document, attempt_dir=attempt_dir)
        identifier = (*request.namespace, request.table)

        def work() -> dict[str, object]:
            catalog = open_catalog(request.connection, request_timeout)
            return append(lambda: catalog.load_table(identifier), request.append)

        try:
            return await asyncio.to_thread(work)
        except Failure:
            raise
        except Exception as error:
            mapped = _failure(error)
            if mapped is None:
                raise
            raise mapped from None

    return operation


def operations_for(attempt_dir: Path) -> dict[str, Operation]:
    """The operations the host serves for an attempt."""
    return {"icebergWrite": iceberg_write(attempt_dir)}


def _failure(error: Exception) -> Failure | None:
    """The failure of the closed set that ``error`` comes to, or ``None`` for one nobody expected."""
    if isinstance(error, HygieneError):
        return Failure(error.code)
    if isinstance(error, NoSuchTableError | NoSuchNamespaceError | NoSuchIdentifierError):
        return Failure(TABLE_NOT_FOUND)
    if isinstance(error, UnauthorizedError | ForbiddenError | OAuthError | AuthorizationExpiredError):
        return Failure(AUTHENTICATION_FAILED)
    # requests' errors are OSErrors too: the catalog is asked about first
    if isinstance(error, requests.RequestException | ServerError | ServiceUnavailableError | TooManyRequestsError):
        return Failure(CATALOG_UNAVAILABLE)
    if isinstance(error, OSError):
        return Failure(STORAGE_UNAVAILABLE)
    return None
