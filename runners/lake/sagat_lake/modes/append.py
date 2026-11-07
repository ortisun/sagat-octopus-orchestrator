"""The append mode of the host (feature 035: T035-057, D035-030, D035-031, FR-035-018, FR-035-020).

The protocol of D035-030 around one ``Table.append``:

1. load the table and check that it can be written (``compat``);
2. search the history of the ref for the effect key: if it is there the step is done, nothing is written and the receipt
   is that commit's;
3. write the data files in a directory of *this try* and commit once, asserting the head that was read;
4. decide from the answer:

   * the commit landed: the receipt;
   * the commit was refused because the ref moved (a conflict): it did not land, so the files of the try are deleted,
     and the table is loaded again and searched again, within ``max_commit_retries``;
   * the commit may have landed (a 5xx, a lost connection, a timeout): nothing is deleted, because the commit may
     reference the files; the table is loaded again and searched again. If the key is there, that commit landed. If it
     is not, a new try asserts the head it reads, and since a request that is still in flight asserts the same head or
     an older one, at most one of them can land, and either carries the key.

Every try has a directory of its own (``<attempt id>-<n>``), so deleting the files of a refused try never touches the
files of a try that is still unknown. When the step ends committed, the directories of the unknown tries are cleaned of
every file that the landed commit does not reference. When it ends unknown they are left, and are the table owner's
(D035-031).

PyIceberg's own retry is off (``commit.pin_commit_once``), the table is not trusted to say how often to commit.
"""

from __future__ import annotations

import itertools
import re
import threading
from collections.abc import Callable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pyarrow as pa  # type: ignore[import-untyped]
from pyiceberg.exceptions import CommitFailedException

from ..attempt import pin_attempt_directory, remove_attempt_files
from ..batches import DEFAULT_BYTES, DEFAULT_ROWS, to_batches
from ..commit import pin_commit_once
from ..compat import check
from ..failures import Failure
from ..lookup import EFFECT_KEY, Found, lookup
from ..mapping import map_record
from ..parquet_input import read_parquet
from ..records import Budget, read_ndjson
from ..table_facts import facts_of, refs_of, snapshots_of

if TYPE_CHECKING:
    from pyiceberg.table import Table
    from pyiceberg.table.snapshots import Snapshot

COMMIT_CONFLICT = "ICEBERG_COMMIT_CONFLICT"
SCOPE_VIOLATION = "ICEBERG_SCOPE_VIOLATION"
STORAGE_UNAVAILABLE = "STORAGE_UNAVAILABLE"
OUTCOME_UNKNOWN = "OUTCOME_UNKNOWN"

_ATTEMPT_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{0,59}")
# How long the writers of PyIceberg are waited for before the files of a failed try are deleted.
_DRAIN_SECONDS = 120.0
# A count as the contract writes it: an unsigned integer in decimal, in a string.
_COUNT = re.compile(r"0|[1-9][0-9]{0,18}")


@dataclass(frozen=True)
class InputFile:
    """A file of the attempt directory that holds records, and the format they are in."""

    path: Path
    format: str = "ndjson"

    def __post_init__(self) -> None:
        if self.format not in ("ndjson", "parquet"):
            raise ValueError("an input is NDJSON or Parquet")


@dataclass(frozen=True)
class AppendRequest:
    """What the runner tells the host about one append (``contracts/lake-runtime.md``)."""

    effect_key: str
    first_attempt_started_at_ms: int
    snapshot_properties: Mapping[str, str]
    ref: str
    # Up to 60 characters: every try adds ``-<n>``, and a directory name is at most 64.
    attempt_id: str
    inputs: Sequence[InputFile]
    warehouse_locations: Sequence[str]
    mapping: Sequence[tuple[str, str]] = ()
    ignore_unknown_fields: bool = False
    allow_empty: bool = False
    max_metadata_bytes: int = 64 * 1024 * 1024
    max_decoded_bytes: int = 16 * 1024**3
    max_commit_retries: int = 4
    max_batch_rows: int = DEFAULT_ROWS
    max_batch_bytes: int = DEFAULT_BYTES
    prior_attempt_may_have_left_files: bool = False

    def __post_init__(self) -> None:
        if not _ATTEMPT_ID.fullmatch(self.attempt_id):
            raise ValueError("an attempt id is 1 to 60 letters, digits and hyphens")
        if self.max_commit_retries < 0:
            raise ValueError("the commit retries are not negative")


class _Outcome(Enum):
    COMMITTED = "committed"
    REFUSED = "refused"
    UNKNOWN = "unknown"
    EMPTY = "empty"


@dataclass
class _Counts:
    """What one try saw, and what it could not clean."""

    dropped: int = 0
    # Files that could not be deleted, or directories that could not be listed.
    left: int = 0


@dataclass
class _Try:
    outcome: _Outcome
    directory: str
    snapshot: Snapshot | None = None
    dropped: int = 0


def append(load_table: Callable[[], Table], request: AppendRequest) -> dict[str, Any]:
    """Appends the records of ``request`` once, or finds that they are in the table already. Returns the receipt.

    ``load_table`` loads the table afresh from the catalog on every call.
    """
    counts = _Counts()
    unsettled: list[tuple[str, str]] = []  # (directory id, directory) of the tries whose outcome is unknown
    tries = 0
    while True:
        table = load_table()
        check(
            facts_of(table, warehouse_locations=request.warehouse_locations),
            ref=request.ref,
            warehouse_locations=request.warehouse_locations,
            max_metadata_bytes=request.max_metadata_bytes,
        )
        found = lookup(
            request.effect_key,
            ref=request.ref,
            refs=refs_of(table),
            snapshots=snapshots_of(table),
            first_attempt_started_at_ms=request.first_attempt_started_at_ms,
        )
        if isinstance(found, Found):
            snapshot = table.snapshot_by_id(found.snapshot_id)
            assert snapshot is not None  # the lookup read it from this table
            _settle(table, snapshot, unsettled, request, counts)
            return _receipt(table, snapshot, request, found=True, dropped=0, left=counts.left)

        directory_id = f"{request.attempt_id}-{tries}"
        attempt = _try(table, request, directory_id, counts)
        if attempt.outcome is _Outcome.COMMITTED:
            assert attempt.snapshot is not None
            _settle(table, attempt.snapshot, unsettled, request, counts)
            return _receipt(table, attempt.snapshot, request, found=False, dropped=attempt.dropped, left=counts.left)
        if attempt.outcome is _Outcome.EMPTY:
            if not request.allow_empty:
                raise Failure(SCOPE_VIOLATION)
            return _empty_receipt(request)
        tries += 1
        if attempt.outcome is _Outcome.UNKNOWN:
            unsettled.append((directory_id, attempt.directory))
            if tries > request.max_commit_retries:
                raise Failure(OUTCOME_UNKNOWN)
        elif tries > request.max_commit_retries:
            raise Failure(COMMIT_CONFLICT)


def _try(table: Table, request: AppendRequest, directory_id: str, counts: _Counts) -> _Try:
    """One try: files in the directory of the try, one commit, and what became of it."""
    pin_commit_once(table)
    directory = pin_attempt_directory(table, directory_id)
    tried = _Try(_Outcome.UNKNOWN, directory)
    with _watching_commits(table) as sent:
        try:
            data = _data(table, request, tried)
            if data is None:
                return _Try(_Outcome.EMPTY, directory)
            properties = {**request.snapshot_properties, EFFECT_KEY: request.effect_key}
            table.append(data, snapshot_properties=properties, branch=request.ref)
        except CommitFailedException:
            # the catalog says the commit did not land: its files are nobody's
            _discard(table, directory, directory_id, request, counts)
            return _Try(_Outcome.REFUSED, directory)
        except Failure:
            _discard(table, directory, directory_id, request, counts)
            raise
        except Exception as exc:
            if sent[0]:
                # a commit was sent and nothing says whether it landed: the files may be referenced, so they stay
                return _Try(_Outcome.UNKNOWN, directory)
            _discard(table, directory, directory_id, request, counts)
            if isinstance(exc, OSError):
                raise Failure(STORAGE_UNAVAILABLE) from exc
            raise
    snapshot = table.snapshot_by_name(request.ref)
    assert snapshot is not None  # the commit that just landed made it
    tried.outcome = _Outcome.COMMITTED
    tried.snapshot = snapshot
    return tried


def _records(file: InputFile, budget: Budget) -> Iterator[dict[str, Any]]:
    return read_parquet(file.path, budget) if file.format == "parquet" else read_ndjson(file.path, budget)


def _data(table: Table, request: AppendRequest, tried: _Try) -> pa.Table | pa.RecordBatchReader | None:
    """The records as Arrow, or ``None`` when there are none. The files are read here, so a bad record ends here."""
    schema = table.schema()
    budget = Budget(request.max_decoded_bytes)

    def rows() -> Iterator[dict[str, Any]]:
        for source in request.inputs:
            for record in _records(source, budget):
                row, dropped = map_record(
                    schema, record, mapping=request.mapping, ignore_unknown=request.ignore_unknown_fields
                )
                tried.dropped += dropped
                yield row

    batches = to_batches(
        schema,
        rows(),
        meter=lambda: budget.used,
        max_rows=request.max_batch_rows,
        max_bytes=request.max_batch_bytes,
    )
    first = next(batches, None)
    if first is None:
        return None
    stream = itertools.chain([first], batches)
    if table.spec().is_unpartitioned():
        # PyIceberg 0.12 streams only into an unpartitioned table (G035-045)
        return pa.RecordBatchReader.from_batches(first.schema, stream)
    return pa.Table.from_batches(list(stream), schema=first.schema)


@contextmanager
def _watching_commits(table: Table) -> Iterator[list[bool]]:
    """Tells whether a commit request was sent: ``sent[0]`` is true once ``commit_table`` of the catalog was called."""
    catalog = table.catalog
    original = catalog.commit_table
    sent = [False]

    def spy(*args: Any, **kwargs: Any) -> Any:
        sent[0] = True
        return original(*args, **kwargs)

    setattr(catalog, "commit_table", spy)  # noqa: B010
    try:
        yield sent
    finally:
        delattr(catalog, "commit_table")


def _quiesced() -> bool:
    """Waits until every thread of PyIceberg's writer pool is idle; ``False`` if they are not within the time.

    PyIceberg submits every write of a stream to a pool of threads (``ThreadPoolExecutor.map`` consumes the whole
    iterable before it returns anything), and when the reader fails it raises with writes still running. A deletion made
    at once would miss the files they are about to create. One task per thread waits for all the others at a barrier,
    so none returns before every thread has finished what it had been given.
    """
    from concurrent.futures import TimeoutError as FutureTimeout

    from pyiceberg.utils.concurrent import ExecutorFactory

    executor = ExecutorFactory.get_or_create()
    workers = getattr(executor, "_max_workers", None)
    if not isinstance(workers, int) or workers < 1:
        return False
    barrier = threading.Barrier(workers)
    try:
        futures = [executor.submit(barrier.wait, _DRAIN_SECONDS) for _ in range(workers)]
        for future in futures:
            future.result(_DRAIN_SECONDS + 5)
    except (threading.BrokenBarrierError, FutureTimeout, RuntimeError):
        barrier.abort()
        return False
    return True


def _discard(table: Table, directory: str, directory_id: str, request: AppendRequest, counts: _Counts) -> None:
    """Deletes every file of a try that did not commit. Whatever cannot be deleted is counted, never raised.

    The writers are waited for first. If they cannot be, nothing is deleted: a file that is deleted and then written
    again is worse than one that stays.
    """
    if not _quiesced():
        counts.left += 1
        return
    try:
        removal = remove_attempt_files(table, directory, directory_id, warehouse_locations=request.warehouse_locations)
    except (OSError, Failure):
        counts.left += 1
    else:
        counts.left += removal.failed


def _settle(
    table: Table, snapshot: Snapshot, unsettled: list[tuple[str, str]], request: AppendRequest, counts: _Counts
) -> None:
    """Cleans the directories of the unknown tries, now that the commit that landed is known: whatever it does not
    reference is deleted, and whatever it does is kept."""
    if not unsettled:
        return
    io = table.io
    keep = {
        entry.data_file.file_path
        for manifest in snapshot.manifests(io)
        for entry in manifest.fetch_manifest_entry(io, discard_deleted=True)
    }
    for directory_id, directory in unsettled:
        try:
            removal = remove_attempt_files(
                table, directory, directory_id, warehouse_locations=request.warehouse_locations, keep=keep
            )
        except (OSError, Failure):
            counts.left += 1
        else:
            counts.left += removal.failed


def _count(value: object) -> str:
    return value if isinstance(value, str) and _COUNT.fullmatch(value) else "0"


def _receipt(
    table: Table, snapshot: Snapshot, request: AppendRequest, *, found: bool, dropped: int, left: int
) -> dict[str, Any]:
    summary = snapshot.summary.additional_properties if snapshot.summary else {}
    receipt: dict[str, Any] = {
        "kind": "iceberg",
        "outcome": "COMMITTED",
        "effectKey": request.effect_key,
        "foundByLookup": found,
        "mode": "append",
        "ref": request.ref,
        "snapshotId": str(snapshot.snapshot_id),
        "sequenceNumber": _count(str(snapshot.sequence_number)),
        "schemaId": snapshot.schema_id if snapshot.schema_id is not None else table.schema().schema_id,
        "formatVersion": table.metadata.format_version,
        "rowLevelMode": "NONE",
        "rowsAdded": _count(summary.get("added-records")),
        "dataFilesAdded": _count(summary.get("added-data-files")),
        "bytesWritten": _count(summary.get("added-files-size")),
    }
    return _with_optional(receipt, request, dropped=dropped, left=left)


def _empty_receipt(request: AppendRequest) -> dict[str, Any]:
    receipt: dict[str, Any] = {
        "kind": "iceberg",
        "outcome": "EMPTY_NO_COMMIT",
        "effectKey": request.effect_key,
        "foundByLookup": False,
        "mode": "append",
        "ref": request.ref,
        "rowsAdded": "0",
    }
    return _with_optional(receipt, request, dropped=0, left=0)


def _with_optional(receipt: dict[str, Any], request: AppendRequest, *, dropped: int, left: int) -> dict[str, Any]:
    if dropped:
        receipt["ignoredFields"] = str(dropped)
    if left:
        receipt["uncommittedFiles"] = str(left)
    if request.prior_attempt_may_have_left_files:
        receipt["priorAttemptMayHaveLeftFiles"] = True
    return receipt
