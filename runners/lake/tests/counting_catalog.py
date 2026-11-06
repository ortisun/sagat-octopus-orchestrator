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


@dataclass
class Catalog:
    """What a test sees of the server: where it is, what it was asked to commit, and what to answer."""

    uri: str
    sql: SqlCatalog
    # The commit requests received, whatever the answer: the path of each.
    commits: list[str] = field(default_factory=list)
    # ``None`` applies the commit to the real table; a status answers with it; ``RESET`` closes the connection.
    answer: int | str | None = None
    # Properties the catalog adds to the table it returns, as a table owner could.
    properties: dict[str, str] = field(default_factory=dict)


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
            path = self.path.split("?", 1)[0]
            if path == "/v1/config":
                self._reply(200, {"defaults": {}, "overrides": {}})
            elif path == f"/v1/namespaces/{NAMESPACE}/tables/{TABLE}":
                table = state.sql.load_table(f"{NAMESPACE}.{TABLE}")
                metadata = json.loads(table.metadata.model_dump_json(by_alias=True, exclude_none=True))
                metadata["properties"] = {**metadata.get("properties", {}), **state.properties}
                self._reply(200, {"metadata-location": table.metadata_location, "metadata": metadata, "config": {}})
            else:
                self._reply(404, _error(404))

        def do_POST(self) -> None:
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            state.commits.append(self.path)
            if state.answer == RESET:
                self.connection.close()
            elif isinstance(state.answer, int):
                self._reply(state.answer, _error(state.answer))
            else:
                request = CommitTableRequest.model_validate_json(body)
                table = state.sql.load_table(f"{NAMESPACE}.{TABLE}")
                try:
                    response = state.sql.commit_table(table, request.requirements, request.updates)
                except CommitFailedException:
                    self._reply(409, _error(409))
                else:
                    self._reply(200, json.loads(response.model_dump_json(by_alias=True, exclude_none=True)))

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
