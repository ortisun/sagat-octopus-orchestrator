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
from pyarrow import ArrowInvalid, ArrowNotImplementedError  # type: ignore[import-untyped]

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


EMPTY_LISTING = (
    b"<?xml version='1.0'?><ListBucketResult><Name>lake</Name><Prefix>warehouse/</Prefix><KeyCount>0</KeyCount>"
    b"<MaxKeys>1</MaxKeys><IsTruncated>false</IsTruncated></ListBucketResult>"
)
NO_SUCH_BUCKET = (
    b"<?xml version='1.0'?><Error><Code>NoSuchBucket</Code><Message>The bucket does not exist</Message></Error>"
)


@dataclass
class ObjectStore:
    """What the double was asked, and how it answers: the `HEAD` of the object at the location (``status``), the
    listing under it (``listing_status`` and ``listing``) and the `HEAD` of the bucket itself (``bucket_status``)."""

    endpoint: str = ""
    status: int = 200
    listing_status: int = 200
    listing: bytes = EMPTY_LISTING
    bucket_status: int = 200
    delay: float = 0.0
    requests: list[str] = field(default_factory=list)


@pytest.fixture
def store() -> Generator[ObjectStore]:
    state = ObjectStore()

    class Handler(BaseHTTPRequestHandler):
        def _answer(self) -> None:
            state.requests.append(f"{self.command} {self.path}")
            time.sleep(state.delay)
            if self.path == "/lake":
                status, body = state.bucket_status, b""
            elif self.path.startswith("/lake?"):
                status, body = state.listing_status, state.listing
            else:
                status, body = state.status, b""
            self.send_response(status)
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


def test_a_location_with_nothing_under_it_in_a_bucket_that_exists_is_a_pass(store: ObjectStore) -> None:
    store.status = 404  # no object of that name, and the listing is a real, empty one

    assert verdict(properties(store.endpoint)) == (None, None)

    # nothing there is a pass: the credentials got to the location and were let ask. The listing is one key at most,
    # and the bucket is asked about last, because "nothing there" is what a bucket that does not exist says too.
    assert store.requests == [
        "HEAD /lake/warehouse",
        "GET /lake?delimiter=%2F&list-type=2&max-keys=1&prefix=warehouse%2F",
        "HEAD /lake",
    ]


def test_a_bucket_that_does_not_exist_is_not_a_pass(store: ObjectStore) -> None:
    # the client says "not found" for the object, for the listing (NoSuchBucket) and for the bucket: that is the client
    # being told there is no such place, which is not credentials that reach the location
    store.status = 404
    store.listing_status = 404
    store.listing = NO_SUCH_BUCKET
    store.bucket_status = 404

    code, error = verdict(properties(store.endpoint))

    assert (code, error) == (STORAGE_UNAVAILABLE, "FileNotFoundError")


def test_a_bucket_location_that_does_not_exist_is_not_a_pass(store: ObjectStore) -> None:
    store.bucket_status = 404
    code, _ = verdict(properties(store.endpoint), location="s3://lake/")
    assert code == STORAGE_UNAVAILABLE


def test_credentials_scoped_to_the_prefix_pass_when_the_bucket_itself_may_not_be_asked_about(
    store: ObjectStore,
) -> None:
    # the listing under the prefix was allowed and found nothing; the bucket answers 403 because the credentials have no
    # right to it, and that is not a reason to say the location is not there
    store.status = 404
    store.bucket_status = 403

    assert verdict(properties(store.endpoint)) == (None, None)
    assert store.requests[-1] == "HEAD /lake"


def test_a_location_that_is_a_bucket_is_a_head_of_the_bucket(store: ObjectStore) -> None:
    assert verdict(properties(store.endpoint), location="s3://lake/") == (None, None)
    assert store.requests == ["HEAD /lake"]


def test_a_location_the_client_refuses_is_a_failure_of_the_storage_check_and_not_a_crash() -> None:
    # nothing in the domain or in the host forbids an empty path segment; the S3 client does
    code, error = verdict(properties("http://127.0.0.1:9"), location="s3://lake//x/", seconds=1.0)
    assert (code, error) == (STORAGE_UNAVAILABLE, "ArrowInvalid")


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
        # what a store writes in its own words is data: it is read by what it says and never trusted to be small
        (OSError("curlCode: " + "9" * 5000), STORAGE_UNAVAILABLE),
        (OSError("AWS Error UNKNOWN (HTTP status " + "4" * 5000 + ") during HeadObject"), STORAGE_UNAVAILABLE),
        (OSError("AWS Error UNKNOWN (HTTP status 4000) during HeadObject operation"), STORAGE_UNAVAILABLE),
        (OSError("curlCode: 2800, Timeout was reached"), STORAGE_UNAVAILABLE),
        (ArrowInvalid("Empty path component in path lake//x"), STORAGE_UNAVAILABLE),
        (ArrowNotImplementedError("no such thing"), STORAGE_UNAVAILABLE),
        (FileNotFoundError("the bucket is not there"), STORAGE_UNAVAILABLE),
    ],
)
def test_the_messages_of_the_s3_client_come_to_one_code_each(error: Exception, code: str) -> None:
    assert storage_code(error) == code


@pytest.mark.parametrize("error", [RuntimeError("x"), ValueError("x"), KeyError("x")])
def test_an_error_that_is_not_the_stores_is_nobodys_to_guess_at(error: Exception) -> None:
    assert storage_code(error) is None
