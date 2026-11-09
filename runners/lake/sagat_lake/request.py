"""The request line of an ``icebergWrite`` or a ``probe``, checked field by field (feature 035: T035-057, D035-150).

The runner writes one JSON line; the host reads it before anything else. The runner is the platform's own, but the line
carries what people write (a table, a mapping, a ref) and a connection with its credentials, so each field is checked
for its type, its shape and its bounds. A field the host does not know, one that is missing and one that does not hold
what the contract says are the same thing, a break of the protocol, and a break is ``LAKE_HOST_PROTOCOL`` with no field
name and no value. So are the things that are not served yet: another mode, a Parquet input.

The paths of the inputs are resolved here, below ``<attempt dir>/in/``, so nothing downstream follows a name from the
request out of the attempt's directory. The connection is kept whole for the catalog loader and is never shown.

A ``probe`` is the lease of the server (``sagat.lake.probe-input.v1``: the connection by name and revision, the
checks in the order they run, the time one check may take) next to the pinned connection with its resolved secrets, in
a member of its own because the connection of the lease is a name and the connection of the request is a document. It
asks for exactly what the server leases, and nothing the host does not run: the three checks once each, in order.
"""

from __future__ import annotations

import datetime
import re
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .failures import LAKE_HOST_PROTOCOL, Failure
from .lookup import EFFECT_KEY
from .modes.append import AppendRequest, InputFile

PROTOCOL = "sagat.lake.v1"
OPERATION = "icebergWrite"
PROBE_OPERATION = "probe"
# The checks of a probe, in the order they run (``crates/domain/src/database/lake_probe.rs``, ``ProbeCheck::ALL``).
PROBE_CHECKS = ("CATALOG_AUTH", "WAREHOUSE", "STORAGE_ACCESS")

# What the platform puts in the summary of a commit (D035-016). PyIceberg refuses a key it owns, and the host does not
# let a request name any other.
SNAPSHOT_PROPERTIES = frozenset(
    {EFFECT_KEY, "sagat.run-id", "sagat.step-id", "sagat.workflow", "sagat.workflow-version"}
)

_TOP = frozenset(
    {
        "protocol",
        "operation",
        "effectKey",
        "firstAttemptStartedAt",
        "snapshotProperties",
        "connection",
        "relay",
        "table",
        "ref",
        "mode",
        "overwrite",
        "upsert",
        "inputs",
        "mapping",
        "ignoreUnknownFields",
        "limits",
        "allowEmpty",
        "priorAttemptMayHaveLeftFiles",
    }
)
_REQUIRED = _TOP - {
    "relay",
    "overwrite",
    "upsert",
    "mapping",
    "ignoreUnknownFields",
    "allowEmpty",
    "priorAttemptMayHaveLeftFiles",
}
_TIME = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,6})?Z")
_COUNT = re.compile(r"0|[1-9][0-9]{0,18}")
_FILE = re.compile(r"in/([A-Za-z0-9_-][A-Za-z0-9._-]{0,127})")
_ATTEMPT_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{0,59}")
_POINTER = re.compile(r"(/([^/~]|~[01])*)+")
# The limits of the domain (``crates/domain/src/database/iceberg_write.rs``): the host takes everything the platform
# publishes and nothing more. They are bytes, not characters, and a name has no control character.
_MAX_NAME = 255
_MAX_LEVELS = 8
_MAX_INPUTS = 32
_FORMATS = frozenset({"ndjson", "parquet"})
_MAX_MAPPINGS = 1024
_MAX_POINTER = 512
_MAX_DEPTH = 16
_MAX_COLUMN = 255
_MAX_LOCATIONS = 16
_MAX_PROPERTY = 1024
_MAX_KEY = 512
_MAX_RETRIES = 16
_INT64 = 2**63 - 1
# The name of a connection is the domain's slug: 3 to 63 of a-z, 0-9 and '-', a letter first, no '-' last or twice.
_SLUG = re.compile(r"[a-z][a-z0-9]*(-[a-z0-9]+)*")
_MIN_SLUG, _MAX_SLUG = 3, 63
_MAX_REVISION = 2**32 - 1
# The server leases 7 000 ms; the host takes a tenth of a second to a minute and nothing else.
_MIN_CHECK_MS, _MAX_CHECK_MS = 100, 60_000


@dataclass(frozen=True)
class WriteRequest:
    """What the operation needs of the request: where to write, how, and the connection to reach it."""

    namespace: tuple[str, ...]
    table: str
    append: AppendRequest
    deadline_at_ms: int
    # The pinned connection with its resolved credentials. It is never part of a representation.
    connection: Mapping[str, Any] = field(repr=False)


def _break() -> Failure:
    return Failure(LAKE_HOST_PROTOCOL)


def parse_write(document: object, *, attempt_dir: Path) -> WriteRequest:
    """The request of an ``icebergWrite``, or ``Failure(LAKE_HOST_PROTOCOL)``."""
    if not isinstance(document, dict) or not _REQUIRED <= document.keys() <= _TOP:
        raise _break()
    if document["protocol"] != PROTOCOL or document["operation"] != OPERATION:
        raise _break()
    # only append is served; the sections of the other modes must be absent or null
    if document["mode"] != "append" or document.get("overwrite") is not None or document.get("upsert") is not None:
        raise _break()
    attempt_id = attempt_dir.name
    if not _ATTEMPT_ID.fullmatch(attempt_id):
        raise _break()

    effect_key = _text(document["effectKey"], _MAX_KEY)
    limits = _object(document["limits"], {"maxCommitRetries", "maxDecodedBytes", "maxMetadataBytes", "deadlineAt"})
    namespace, table = _table(document["table"])
    connection = _connection(document["connection"])
    append = AppendRequest(
        effect_key=effect_key,
        first_attempt_started_at_ms=_millis(document["firstAttemptStartedAt"]),
        snapshot_properties=_snapshot_properties(document["snapshotProperties"]),
        ref=_text(document["ref"], _MAX_NAME),
        attempt_id=attempt_id,
        inputs=_inputs(document["inputs"], attempt_dir),
        warehouse_locations=tuple(connection["warehouseLocations"]),
        mapping=_mapping(document.get("mapping", [])),
        ignore_unknown_fields=_flag(document.get("ignoreUnknownFields", False)),
        allow_empty=_flag(document.get("allowEmpty", False)),
        max_metadata_bytes=_count(limits["maxMetadataBytes"]),
        max_decoded_bytes=_count(limits["maxDecodedBytes"]),
        max_commit_retries=_retries(limits["maxCommitRetries"]),
        prior_attempt_may_have_left_files=_flag(document.get("priorAttemptMayHaveLeftFiles", False)),
    )
    return WriteRequest(
        namespace=namespace,
        table=table,
        append=append,
        deadline_at_ms=_millis(limits["deadlineAt"]),
        connection=connection,
    )


@dataclass(frozen=True)
class ProbeRequest:
    """What a probe needs of the request: whose probe it is (echoed in the result), how long a check may take, and the
    connection to reach."""

    name: str
    revision: int
    check_timeout: float
    # The pinned connection with its resolved credentials. It is never part of a representation.
    connection: Mapping[str, Any] = field(repr=False)


def parse_probe(document: object) -> ProbeRequest:
    """The request of a ``probe``, or ``Failure(LAKE_HOST_PROTOCOL)``."""
    keys = {"protocol", "operation", "connection", "relay", "probe"}
    if not isinstance(document, dict) or not keys - {"relay"} <= document.keys() <= keys:
        raise _break()
    if document["protocol"] != PROTOCOL or document["operation"] != PROBE_OPERATION:
        raise _break()
    probe = _object(document["probe"], {"connection", "checks", "checkTimeoutMs"})
    if probe["checks"] != list(PROBE_CHECKS):
        raise _break()
    identity = _object(probe["connection"], {"name", "revision"})
    name, revision = identity["name"], identity["revision"]
    if not isinstance(name, str) or not _MIN_SLUG <= len(name) <= _MAX_SLUG or not _SLUG.fullmatch(name):
        raise _break()
    if isinstance(revision, bool) or not isinstance(revision, int) or not 1 <= revision <= _MAX_REVISION:
        raise _break()
    milliseconds = probe["checkTimeoutMs"]
    if (
        isinstance(milliseconds, bool)
        or not isinstance(milliseconds, int)
        or not _MIN_CHECK_MS <= milliseconds <= _MAX_CHECK_MS
    ):
        raise _break()
    return ProbeRequest(
        name=name,
        revision=revision,
        check_timeout=milliseconds / 1000,
        connection=_connection(document["connection"]),
    )


def _text(value: object, limit: int) -> str:
    """Text of 1 to ``limit`` bytes without a control character, as the domain's ``plain``."""
    if not isinstance(value, str) or not value:
        raise _break()
    try:
        size = len(value.encode("utf-8"))
    except UnicodeEncodeError:
        raise _break() from None
    if size > limit or any(unicodedata.category(character) == "Cc" for character in value):
        raise _break()
    return value


def _flag(value: object) -> bool:
    if not isinstance(value, bool):
        raise _break()
    return value


def _object(value: object, keys: set[str], *, required: set[str] | None = None) -> dict[str, Any]:
    """An object with exactly these keys, or with these at most and ``required`` at least."""
    needed = keys if required is None else required
    if not isinstance(value, dict) or not needed <= value.keys() <= keys:
        raise _break()
    return value


def _millis(value: object) -> int:
    if not isinstance(value, str) or not _TIME.fullmatch(value):
        raise _break()
    try:
        moment = datetime.datetime.fromisoformat(value.removesuffix("Z") + "+00:00")
    except ValueError:
        raise _break() from None
    return int(moment.timestamp()) * 1000 + moment.microsecond // 1000


def _count(value: object) -> int:
    if not isinstance(value, str) or not _COUNT.fullmatch(value) or int(value) > _INT64:
        raise _break()
    return int(value)


def _retries(value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= _MAX_RETRIES:
        raise _break()
    return value


def _table(value: object) -> tuple[tuple[str, ...], str]:
    table = _object(value, {"namespace", "name"})
    levels = table["namespace"]
    if not isinstance(levels, list) or not 1 <= len(levels) <= _MAX_LEVELS:
        raise _break()
    return tuple(_name(level) for level in levels), _name(table["name"])


def _name(value: object) -> str:
    """A level of a namespace or a table name. Dot segments are refused: the HTTP client removes them from the path of
    a request (RFC 3986 section 5.2.4), so ``..`` would send the catalog request to another endpoint than the one
    meant."""
    text = _text(value, _MAX_NAME)
    if text in {".", ".."}:
        raise _break()
    return text


def _connection(value: object) -> dict[str, Any]:
    connection = _object(value, {"catalog", "storage", "warehouseLocations", "credentials"})
    if not isinstance(connection["catalog"], dict) or not isinstance(connection["storage"], dict):
        raise _break()
    locations = connection["warehouseLocations"]
    if not isinstance(locations, list) or not 1 <= len(locations) <= _MAX_LOCATIONS:
        raise _break()
    for location in locations:
        _text(location, 2048)
    # the secret documents of the data model, each text to text: the catalog's and the storage's
    credentials = _object(connection["credentials"], {"catalog", "storage"}, required=set())
    for document in credentials.values():
        if not isinstance(document, dict) or not all(
            isinstance(name, str) and isinstance(secret, str) for name, secret in document.items()
        ):
            raise _break()
    return connection


def _snapshot_properties(value: object) -> dict[str, str]:
    if not isinstance(value, dict):
        raise _break()
    for name, text in value.items():
        if name not in SNAPSHOT_PROPERTIES:
            raise _break()
        _text(text, _MAX_PROPERTY)
    return dict(value)


def _inputs(value: object, attempt_dir: Path) -> tuple[InputFile, ...]:
    if not isinstance(value, list) or not 1 <= len(value) <= _MAX_INPUTS:
        raise _break()
    files = []
    for entry in value:
        item = _object(entry, {"file", "format"})
        match = _FILE.fullmatch(item["file"]) if isinstance(item["file"], str) else None
        if match is None or not isinstance(item["format"], str) or item["format"] not in _FORMATS:
            raise _break()
        files.append(InputFile(attempt_dir / "in" / match.group(1), item["format"]))
    return tuple(files)


def _mapping(value: object) -> tuple[tuple[str, str], ...]:
    if not isinstance(value, list) or len(value) > _MAX_MAPPINGS:
        raise _break()
    pairs = []
    for entry in value:
        item = _object(entry, {"field", "column"})
        pointer, column = item["field"], item["column"]
        if not isinstance(pointer, str) or not _pointer(pointer):
            raise _break()
        pairs.append((pointer, _text(column, _MAX_COLUMN)))
    return tuple(pairs)


def _pointer(pointer: str) -> bool:
    """A JSON pointer of at most 512 bytes and 16 segments, with only the escapes ``~0`` and ``~1``."""
    try:
        size = len(pointer.encode("utf-8"))
    except UnicodeEncodeError:
        return False
    return size <= _MAX_POINTER and _POINTER.fullmatch(pointer) is not None and pointer.count("/") <= _MAX_DEPTH
