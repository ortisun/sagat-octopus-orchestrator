"""The storage check of a probe against the real S3 client (feature 035: D035-150, T035-040, slice S5).

`test_probe_operation.py` swaps the listing for a double, so what it shows is the order and the rules of the checks.
These tests are what makes the double honest: `list_one_key` is PyArrow's S3 file system, the client the writes use, and
`storage_code` tells its failures apart by what the SDK under it says. The server here is a plain HTTP server on the
loopback that answers what an object store answers (and what it does when it is down, slow, or refuses), so a change of
PyArrow that rewords a message turns a test red here and not in a probe of a customer's storage.

The cases that cost seconds (the SDK retries a 5xx and a timeout) are one each.
"""

from __future__ import annotations

import threading
import time
from collections.abc import Generator, Mapping
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

import network_doubles
from sagat_lake.probe import list_one_key, storage_code
from sagat_lake.probe_codes import (
    AUTHENTICATION_FAILED,
    DESTINATION_DENIED,
    STORAGE_UNAVAILABLE,
    TIMEOUT,
    TLS_FAILED,
)

LOCATION = "s3://lake/warehouse/"


@dataclass
class ObjectStore:
    """What the double was asked, and how it answers."""

    endpoint: str = ""
    status: int = 200
    # What a missing object says for a `HEAD`; the listing after it is answered with `listing`.
    listing: bytes = b""
    delay: float = 0.0
    requests: list[str] = field(default_factory=list)


EMPTY_LISTING = (
    b"<?xml version='1.0'?><ListBucketResult><Name>lake</Name><Prefix>warehouse/</Prefix><KeyCount>0</KeyCount>"
    b"<MaxKeys>1</MaxKeys><IsTruncated>false</IsTruncated></ListBucketResult>"
)


@pytest.fixture
def store() -> Generator[ObjectStore]:
    state = ObjectStore()

    class Handler(BaseHTTPRequestHandler):
        def _answer(self) -> None:
            state.requests.append(f"{self.command} {self.path}")
            time.sleep(state.delay)
            body = state.listing if self.path.startswith("/lake?") and state.status == 200 else b""
            self.send_response(state.status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)

        do_GET = do_HEAD = _answer

        def log_message(self, format: str, *args: object) -> None:
            """Nothing is logged: a test reads what was asked."""
            del format, args

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    state.endpoint = f"http://127.0.0.1:{server.server_address[1]}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield state
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def properties(endpoint: str, **extra: str) -> dict[str, str]:
    """The storage properties of a connection with static credentials, as the host builds them."""
    return {
        "s3.endpoint": endpoint,
        "s3.region": "eu-west-1",
        "s3.access-key-id": "AKIA",
        "s3.secret-access-key": "secret",
        "s3.force-virtual-addressing": "false",
        "py-io-impl": "pyiceberg.io.pyarrow.PyArrowFileIO",
        **extra,
    }


def verdict(
    props: Mapping[str, str], *, location: str = LOCATION, seconds: float = 3.0
) -> tuple[str | None, str | None]:
    """The code the storage check would fail with, and the class of the error behind it (``None, None`` for a pass)."""
    try:
        list_one_key(props, location, seconds)
    except Exception as error:
        return storage_code(error), type(error).__name__
    return None, None


# --------------------------------------------------------------------------------------------------- what it asks
def test_an_object_at_the_location_is_one_head_and_no_listing(store: ObjectStore) -> None:
    assert verdict(properties(store.endpoint)) == (None, None)
    assert store.requests == ["HEAD /lake/warehouse"]


def test_a_location_with_no_object_is_one_head_and_one_listing_of_at_most_one_key(store: ObjectStore) -> None:
    store.status = 404

    assert verdict(properties(store.endpoint)) == (None, None)

    # nothing there is a pass: the credentials got to the location and were let ask. The listing is one key at most.
    assert store.requests == [
        "HEAD /lake/warehouse",
        "GET /lake?delimiter=%2F&list-type=2&max-keys=1&prefix=warehouse%2F",
    ]


def test_a_location_that_is_a_bucket_is_a_head_of_the_bucket(store: ObjectStore) -> None:
    assert verdict(properties(store.endpoint), location="s3://lake/") == (None, None)
    assert store.requests == ["HEAD /lake"]


# --------------------------------------------------------------------------------------------- what it tells apart
@pytest.mark.parametrize("status", [400, 401, 403])
def test_a_store_that_refuses_the_credentials_is_an_authentication_failure(store: ObjectStore, status: int) -> None:
    store.status = status
    code, _ = verdict(properties(store.endpoint))
    assert code == AUTHENTICATION_FAILED
    assert len(store.requests) == 1


def test_a_store_that_cannot_serve_is_unavailable(store: ObjectStore) -> None:
    store.status = 503
    code, _ = verdict(properties(store.endpoint))
    assert code == STORAGE_UNAVAILABLE


def test_a_store_that_redirects_is_the_connections_mistake_and_is_unavailable(store: ObjectStore) -> None:
    # a bucket in another region answers 301: the client is not sent where it would have to go
    store.status = 301
    code, _ = verdict(properties(store.endpoint))
    assert code == STORAGE_UNAVAILABLE


def test_a_store_nobody_listens_at_is_unavailable() -> None:
    code, _ = verdict(properties("http://127.0.0.1:9"), seconds=1.0)
    assert code == STORAGE_UNAVAILABLE


def test_a_store_that_does_not_answer_in_time_is_a_timeout(store: ObjectStore) -> None:
    store.delay = 3.0
    started = time.monotonic()
    code, _ = verdict(properties(store.endpoint), seconds=0.3)
    assert code == TIMEOUT
    # the SDK retries: that is why the check as a whole is bounded too (test_probe_operation)
    assert time.monotonic() - started < 15


def test_a_store_with_a_certificate_nobody_trusts_is_a_tls_failure(tmp_path: Path) -> None:
    with network_doubles.untrusted_tls_server(tmp_path) as port:
        code, _ = verdict(properties(f"https://127.0.0.1:{port}"))
    assert code == TLS_FAILED


def test_a_store_the_relay_refuses_is_a_destination_denial() -> None:
    with network_doubles.refusing_proxy() as port:
        code, _ = verdict(properties("https://s3.example.test", **{"s3.proxy-uri": f"http://127.0.0.1:{port}"}))
    assert code == DESTINATION_DENIED


# ---------------------------------------------------------------------------------------------------- the mapping
@pytest.mark.parametrize(
    ("error", "code"),
    [
        (TimeoutError(), TIMEOUT),
        (OSError("AWS Error ACCESS_DENIED during HeadObject operation: No response body."), AUTHENTICATION_FAILED),
        (OSError("AWS Error UNKNOWN (HTTP status 400) during HeadObject operation"), AUTHENTICATION_FAILED),
        (OSError("AWS Error UNKNOWN (HTTP status 500) during HeadObject operation"), STORAGE_UNAVAILABLE),
        (OSError("AWS Error UNKNOWN (HTTP status 429) during HeadObject operation"), STORAGE_UNAVAILABLE),
        (
            OSError("AWS Error NETWORK_CONNECTION during HeadObject operation: curlCode: 7, Could not connect"),
            STORAGE_UNAVAILABLE,
        ),
        (
            OSError("AWS Error NETWORK_CONNECTION during HeadObject operation: curlCode: 6, Could not resolve"),
            STORAGE_UNAVAILABLE,
        ),
        (
            OSError("AWS Error NETWORK_CONNECTION during HeadObject operation: curlCode: 28, Timeout was reached"),
            TIMEOUT,
        ),
        (
            OSError("AWS Error NETWORK_CONNECTION during HeadObject operation: curlCode: 60, SSL peer certificate"),
            TLS_FAILED,
        ),
        (
            OSError("AWS Error NETWORK_CONNECTION during HeadObject operation: curlCode: 35, SSL connect error"),
            TLS_FAILED,
        ),
        (
            OSError("curlCode: 7, Could not connect to server; Details: CONNECT tunnel failed, response 403"),
            DESTINATION_DENIED,
        ),
        (OSError("something no one has seen"), STORAGE_UNAVAILABLE),
    ],
)
def test_the_messages_of_the_s3_client_come_to_one_code_each(error: Exception, code: str) -> None:
    assert storage_code(error) == code


@pytest.mark.parametrize("error", [RuntimeError("x"), ValueError("x"), KeyError("x")])
def test_an_error_that_is_not_the_stores_is_nobodys_to_guess_at(error: Exception) -> None:
    assert storage_code(error) is None
