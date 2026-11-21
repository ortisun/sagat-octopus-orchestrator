"""A hostile Iceberg REST catalog for the tests of the lake host (feature 035, T035-018, D035-050).

It answers what a compromised or malicious catalog could say and the host must not believe: overrides of the FileIO
implementation, of the object-store endpoint, of the proxy and of the request signer, a region that would move the
endpoints, tables located at the cloud metadata service or at the platform's own database, and table properties that
redirect the data files outside the warehouse. It serves them over real HTTP on the loopback interface, so a test
parses the same JSON a client would receive.

The fixture lives next to the runtime, not in `tests/fixtures/`, because the selector of the CI treats that directory
as an input of every suite (D035-076); the Rust canaries that need it start it from this directory.
"""

from __future__ import annotations

import json
import threading
import urllib.request
from collections.abc import Generator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

WAREHOUSE = "s3://lake/warehouse"
METADATA_SERVICE = "169.254.169.254"
PLATFORM_DATABASE = "sagat-platform-db"

# Every key a hostile catalog sends that the host must drop, whatever the connection declares.
HOSTILE_KEYS = (
    "py-io-impl",
    "s3.endpoint",
    "s3.proxy-uri",
    "s3.region",
    "client.region",
    "s3.role-arn",
    "s3.signer",
    "s3.signer.uri",
    "s3.signer.endpoint",
)

_FILE_IO = "evil.module.FileIO"
_HOSTILE_VALUES = {
    "py-io-impl": _FILE_IO,
    "s3.endpoint": f"http://{METADATA_SERVICE}",
    "s3.proxy-uri": "http://attacker.example:3128",
    "s3.region": "attacker-region-1",
    "client.region": "attacker-region-1",
    "s3.role-arn": "arn:aws:iam::000000000000:role/attacker",
    "s3.signer": "evil.module.Signer",
    "s3.signer.uri": "http://attacker.example/sign",
    "s3.signer.endpoint": "v1/aws/s3/sign",
}


# Properties a table may honestly carry and the host lets PyIceberg see.
HONEST_PROPERTIES = {
    "write.format.default": "parquet",
    "commit.retry.min-wait-ms": "10",
    "schema.name-mapping.default": "[]",
    "gc.enabled": "true",
}


def config_response() -> dict[str, object]:
    """`GET /v1/config`: defaults and overrides that carry the hostile keys next to the two honest ones."""
    return {
        "defaults": {"warehouse": WAREHOUSE, "prefix": "tenant-a", **_HOSTILE_VALUES},
        "overrides": {"s3.endpoint": f"http://{METADATA_SERVICE}", "py-io-impl": _FILE_IO},
    }


def vended_credentials() -> dict[str, str]:
    """What a REST catalog vends for S3 with `X-Iceberg-Access-Delegation: vended-credentials`."""
    return {
        "s3.access-key-id": "vended-access-key",
        "s3.secret-access-key": "vended-secret-key",
        "s3.session-token": "vended-session-token",
    }


def table_response(
    location: str,
    *,
    config: dict[str, str] | None = None,
    properties: dict[str, str] | None = None,
) -> dict[str, object]:
    """`LoadTable`: a table at `location`, with the vended credentials and the hostile keys in its `config`."""
    return {
        "metadata-location": f"{location.rstrip('/')}/metadata/00000-hostile.metadata.json",
        "metadata": {"format-version": 2, "location": location, "properties": properties or {}},
        "config": config if config is not None else {**vended_credentials(), **_HOSTILE_VALUES},
    }


def table_responses() -> dict[str, dict[str, object]]:
    """The tables the server knows, by name."""
    return {
        "honest": table_response(f"{WAREHOUSE}/sales/orders"),
        "metadata-service": table_response(f"http://{METADATA_SERVICE}/latest/meta-data/"),
        "platform-database": table_response(f"postgresql://{PLATFORM_DATABASE}:5432/sagat"),
        # the keys of the catalog's own configuration, sent as properties of the table: PyIceberg merges a table's
        # properties over the catalog's when it builds the file IO
        "io-in-properties": table_response(
            f"{WAREHOUSE}/sales/orders",
            config=vended_credentials(),
            properties={**HONEST_PROPERTIES, **_HOSTILE_VALUES},
        ),
        "redirected-data": table_response(
            f"{WAREHOUSE}/sales/orders",
            properties={"write.data.path": "s3://attacker-bucket/stolen", "write.metadata.path": f"{WAREHOUSE}/m"},
        ),
    }


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        path = self.path.split("?", 1)[0]
        body: dict[str, object] | None = None
        if path == "/v1/config":
            body = config_response()
        elif path.startswith("/v1/namespaces/hostile/tables/"):
            body = table_responses().get(path.rsplit("/", 1)[1])
        if body is None:
            self.send_error(404)
            return
        payload = json.dumps(body).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, format: str, *args: object) -> None:
        """Nothing is logged: a test reads the responses, not the server's chatter."""
        del format, args


@contextmanager
def serve() -> Generator[str]:
    """The base URL of a hostile catalog on `127.0.0.1`, stopped and closed when the block ends."""
    server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def fetch(base_url: str, path: str) -> dict[str, object]:
    """One GET, parsed; the proxy variables of the environment are ignored so the loopback is reached directly."""
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(f"{base_url}{path}", timeout=5) as response:
        document: dict[str, object] = json.loads(response.read())
    return document
