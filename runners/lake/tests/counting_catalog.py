"""An Iceberg REST catalog that counts the commit requests it receives (feature 035, T035-019, G035-020).

It serves one real table over real HTTP on the loopback interface, and behind it is a real SQL catalog: a commit that
is not given an answer to inject is applied for real, with the requirements the client sent, so a conflict is a real
conflict (the requirement of the head fails). A test can inject the answer of the commit instead (a status, or a
connection closed without a reply) to see what the client does with it. Every commit request is counted, whatever the
answer.

The fixture lives next to the runtime, not in `tests/fixtures/`, because the selector of the CI treats that directory as
an input of every suite (D035-076).
"""

from __future__ import annotations

import json
import threading
import time
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.exceptions import CommitFailedException
from pyiceberg.table import CommitTableRequest

NAMESPACE = "sales"
TABLE = "orders"
RESET = "reset"
# The commit is applied to the real table and then the response is lost: the connection is closed, or a 500 is sent.
LOST = "lost"
LOST_500 = "lost-500"


@dataclass
class Catalog:
    """What a test sees of the server: where it is, what it was asked to commit, and what to answer."""

    uri: str
    sql: SqlCatalog
    # The commit requests received, whatever the answer: the path of each.
    commits: list[str] = field(default_factory=list)
    # ``None`` applies the commit to the real table; a status answers with it; ``RESET`` closes the connection;
    # ``LOST`` and ``LOST_500`` apply the commit and then lose the response.
    answer: int | str | None = None
    # Answers for the next commit requests, one each, in order; when it is empty ``answer`` is used.
    script: list[int | str | None] = field(default_factory=list)
    # Properties the catalog adds to the table it returns, as a table owner could.
    properties: dict[str, str] = field(default_factory=dict)
    # What a hostile or merely generous catalog adds: to /v1/config, to the config of a table, as vended credentials.
    config_defaults: dict[str, str] = field(default_factory=dict)
    config_overrides: dict[str, str] = field(default_factory=dict)
    table_config: dict[str, str] = field(default_factory=dict)
    storage_credentials: list[dict[str, Any]] = field(default_factory=list)
    # The headers of every request received, as a client sent them (names as the HTTP server reads them).
    headers: list[dict[str, str]] = field(default_factory=list)
    # Seconds every request waits before it is answered: a catalog that does not answer in time.
    delay: float = 0.0
    # A status every GET is answered with (config and table alike): a catalog that refuses or is down.
    get_status: int | None = None
    # A text and its replacement in the table the catalog returns: the same table, said to be somewhere else.
    rewrite: tuple[str, str] | None = None
    # The first page of namespaces (the probe's WAREHOUSE check): the paths asked for, a status it is answered with
    # instead of the page, and a body it is answered with instead (a catalog that says something that is not a page).
    namespace_requests: list[str] = field(default_factory=list)
    namespaces_status: int | None = None
    namespaces_body: dict[str, Any] | None = None
    # The `endpoints` of /v1/config: the operations the catalog says it has (`None` leaves the client's defaults).
    config_endpoints: list[str] | None = None
    # A page token the first page carries, as a catalog with more namespaces than one page does.
    next_page_token: str | None = None


def _error(status: int) -> dict[str, object]:
    return {"error": {"message": "injected", "type": "InjectedError", "code": status}}


def _handler(state: Catalog) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        def _reply(self, status: int, body: dict[str, Any]) -> None:
            payload = json.dumps(body).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def do_GET(self) -> None:
            state.headers.append(dict(self.headers.items()))
            time.sleep(state.delay)
            if state.get_status is not None:
                self._reply(state.get_status, _error(state.get_status))
                return
            path = self.path.split("?", 1)[0]
            if path == "/v1/config":
                config: dict[str, Any] = {"defaults": state.config_defaults, "overrides": state.config_overrides}
                if state.config_endpoints is not None:
                    config["endpoints"] = state.config_endpoints
                self._reply(200, config)
            elif path == "/v1/namespaces":
                state.namespace_requests.append(self.path)
                if state.namespaces_status is not None:
                    self._reply(state.namespaces_status, _error(state.namespaces_status))
                elif state.namespaces_body is not None:
                    self._reply(200, state.namespaces_body)
                else:
                    page: dict[str, Any] = {"namespaces": [[NAMESPACE]]}
                    if state.next_page_token is not None:
                        page["next-page-token"] = state.next_page_token
                    self._reply(200, page)
            elif path == f"/v1/namespaces/{NAMESPACE}/tables/{TABLE}":
                table = state.sql.load_table(f"{NAMESPACE}.{TABLE}")
                metadata = json.loads(table.metadata.model_dump_json(by_alias=True, exclude_none=True))
                metadata["properties"] = {**metadata.get("properties", {}), **state.properties}
                document = {
                    "metadata-location": table.metadata_location,
                    "metadata": metadata,
                    "config": state.table_config,
                    "storage-credentials": state.storage_credentials,
                }
                if state.rewrite is not None:
                    document = json.loads(json.dumps(document).replace(*state.rewrite))
                self._reply(200, document)
            else:
                self._reply(404, _error(404))

        def _apply(self, body: bytes) -> dict[str, Any] | None:
            """The commit applied to the real table: the response, or ``None`` when the requirements fail."""
            request = CommitTableRequest.model_validate_json(body)
            table = state.sql.load_table(f"{NAMESPACE}.{TABLE}")
            try:
                response = state.sql.commit_table(table, request.requirements, request.updates)
            except CommitFailedException:
                return None
            return dict(json.loads(response.model_dump_json(by_alias=True, exclude_none=True)))

        def do_POST(self) -> None:
            state.headers.append(dict(self.headers.items()))
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            state.commits.append(self.path)
            time.sleep(state.delay)
            answer = state.script.pop(0) if state.script else state.answer
            if answer == RESET:
                self.connection.close()
            elif answer in (LOST, LOST_500):
                self._apply(body)
                if answer == LOST:
                    self.connection.close()
                else:
                    self._reply(500, _error(500))
            elif isinstance(answer, int):
                self._reply(answer, _error(answer))
            else:
                applied = self._apply(body)
                if applied is None:
                    self._reply(409, _error(409))
                else:
                    self._reply(200, applied)

        def log_message(self, format: str, *args: object) -> None:
            """Nothing is logged: a test reads the counts, not the server's chatter."""
            del format, args

    return Handler


@contextmanager
def serve(sql: SqlCatalog) -> Generator[Catalog]:
    """A REST catalog in front of ``sql`` (which holds ``sales.orders``), stopped and closed when the block ends."""
    state = Catalog(uri="", sql=sql)
    server = ThreadingHTTPServer(("127.0.0.1", 0), _handler(state))
    state.uri = f"http://127.0.0.1:{server.server_address[1]}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield state
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
