"""The append mode of the host (feature 035: T035-049, T035-057, D035-030, D035-031, FR-035-018, FR-035-020).

The mode is the protocol of D035-030 around one ``Table.append``: load, check the table, search the history for the
effect key, write the files in a directory of this try, commit once, and decide from the answer. These tests run it
against a REST catalog over real HTTP with a real SQL catalog behind it (``counting_catalog.py``), so a conflict is a
real conflict, a lost response really lost, and the number of commit requests is counted.

What matters most is what is *not* done: a key that is already in the history is not written again, files that a landed
commit may reference are never deleted, and the files of a commit that was refused do not stay behind.
"""

from __future__ import annotations

import json
import pathlib
import threading
from collections.abc import Callable, Generator
from dataclasses import dataclass
from decimal import Decimal
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
import pyarrow.parquet as pq  # type: ignore[import-untyped]
import pytest
from pyiceberg.catalog.rest import RestCatalog
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.io.pyarrow import schema_to_pyarrow
from pyiceberg.partitioning import PartitionField, PartitionSpec
from pyiceberg.schema import Schema
from pyiceberg.table import Table
from pyiceberg.transforms import IdentityTransform
from pyiceberg.types import DecimalType, LongType, NestedField, StringType
from schema_check import problems

import counting_catalog
from counting_catalog import LOST, LOST_500, Catalog
from sagat_lake import attempt as attempt_module
from sagat_lake import batches as batches_module
from sagat_lake import records as records_module
from sagat_lake.attempt import Removal
from sagat_lake.failures import Failure
from sagat_lake.lookup import EFFECT_KEY
from sagat_lake.modes import append as append_mode
from sagat_lake.modes.append import AppendRequest, InputFile, append

NAME = f"{counting_catalog.NAMESPACE}.{counting_catalog.TABLE}"
SCHEMA = Schema(
    NestedField(1, "id", LongType(), required=True),
    NestedField(2, "name", StringType(), required=False),
    NestedField(3, "amount", DecimalType(12, 2), required=False),
)
KEY = "run-1:save-orders"
QUICK = {"commit.retry.min-wait-ms": "1", "commit.retry.max-wait-ms": "2"}


@dataclass
class Rig:
    sql: SqlCatalog
    rest: Catalog
    directory: pathlib.Path

    @property
    def warehouse(self) -> str:
        return f"file://{self.directory}/warehouse/"

    def load(self) -> Table:
        table: Table = RestCatalog("r", uri=self.rest.uri).load_table(NAME)
        return table

    def table(self) -> Table:
        table: Table = self.sql.load_table(NAME)
        return table

    def rows(self) -> list[dict[str, Any]]:
        return sorted(self.table().scan().to_arrow().to_pylist(), key=lambda row: row["id"])

    def data_files(self) -> list[str]:
        root = pathlib.Path(self.table().metadata.location.removeprefix("file://"))
        return sorted(str(path.relative_to(root)) for path in (root / "data").rglob("*.parquet"))

    def other_writer(self, number: int) -> None:
        """Another process commits to the table."""
        table = self.sql.load_table(NAME)
        row = {"id": 1_000 + number, "name": "other", "amount": Decimal("1.00")}
        table.append(pa.Table.from_pylist([row], schema=schema_to_pyarrow(table.schema())))


@pytest.fixture
def rig(tmp_path: pathlib.Path) -> Generator[Rig]:
    sql = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    sql.create_namespace(counting_catalog.NAMESPACE)
    sql.create_table(NAME, schema=SCHEMA)
    with counting_catalog.serve(sql) as rest:
        rest.properties = dict(QUICK)
        yield Rig(sql, rest, tmp_path)


def lines(*records: dict[str, Any]) -> bytes:
    return b"".join(json.dumps(record).encode() + b"\n" for record in records)


def request(rig: Rig, content: bytes | None = None, *, key: str = KEY, **changes: Any) -> AppendRequest:
    if content is None:
        content = lines({"id": 1, "name": "a", "amount": "10.50"}, {"id": 2, "name": "b"}, {"id": 3})
    inputs = changes.pop("inputs", None)
    if inputs is None:
        path = rig.directory / "inline.ndjson"
        path.write_bytes(content)
        inputs = [path]
    inputs = [item if isinstance(item, InputFile) else InputFile(item) for item in inputs]
    base: dict[str, Any] = {
        "effect_key": key,
        "first_attempt_started_at_ms": 0,
        "snapshot_properties": {EFFECT_KEY: key, "sagat.run-id": "run-1", "sagat.step-id": "save-orders"},
        "ref": "main",
        "attempt_id": "attempt-1",
        "inputs": inputs,
        "warehouse_locations": [rig.warehouse],
    }
    return AppendRequest(**{**base, **changes})


def run(rig: Rig, req: AppendRequest, load: Callable[[], Table] | None = None) -> dict[str, Any]:
    return append(load or rig.load, req)


def failure_of(rig: Rig, req: AppendRequest, load: Callable[[], Table] | None = None) -> Failure:
    with pytest.raises(Failure) as caught:
        run(rig, req, load)
    return caught.value


# ------------------------------------------------------------ the happy path
def test_a_new_effect_key_commits_the_records_once_and_the_receipt_says_what_landed(rig: Rig) -> None:
    receipt = run(rig, request(rig))

    table = rig.table()
    snapshot = table.current_snapshot()
    assert snapshot is not None
    assert receipt == {
        "kind": "iceberg",
        "outcome": "COMMITTED",
        "effectKey": KEY,
        "foundByLookup": False,
        "mode": "append",
        "ref": "main",
        "snapshotId": str(snapshot.snapshot_id),
        "sequenceNumber": str(snapshot.sequence_number),
        "schemaId": table.schema().schema_id,
        "formatVersion": table.metadata.format_version,
        "rowLevelMode": "NONE",
        "rowsAdded": "3",
        "dataFilesAdded": "1",
        "bytesWritten": receipt["bytesWritten"],
    }
    assert int(receipt["bytesWritten"]) > 0
    assert len(rig.rest.commits) == 1
    assert [row["id"] for row in rig.rows()] == [1, 2, 3]
    assert rig.rows()[0]["amount"] == Decimal("10.50")


def test_every_snapshot_of_the_commit_carries_the_snapshot_properties(rig: Rig) -> None:
    run(rig, request(rig))
    snapshot = rig.table().current_snapshot()
    assert snapshot is not None
    assert snapshot.summary is not None
    properties = snapshot.summary.additional_properties
    assert properties[EFFECT_KEY] == KEY
    assert properties["sagat.run-id"] == "run-1"
    assert properties["sagat.step-id"] == "save-orders"


def test_the_effect_key_of_the_request_is_the_one_in_the_snapshot_whatever_the_properties_say(rig: Rig) -> None:
    run(rig, request(rig, snapshot_properties={EFFECT_KEY: "another", "sagat.run-id": "run-1"}))
    snapshot = rig.table().current_snapshot()
    assert snapshot is not None
    assert snapshot.summary is not None
    assert snapshot.summary.additional_properties[EFFECT_KEY] == KEY


def test_the_files_of_the_commit_are_in_a_directory_of_the_attempt(rig: Rig) -> None:
    run(rig, request(rig, attempt_id="attempt-xyz"))
    assert [name for name in rig.data_files() if "/attempt-xyz-0/" in "/" + name] == rig.data_files()
    assert len(rig.data_files()) == 1


def test_the_stored_table_never_has_the_pins(rig: Rig) -> None:
    run(rig, request(rig))
    properties = rig.table().metadata.properties
    assert "write.data.path" not in properties
    assert "commit.retry.num-retries" not in properties


def test_records_from_several_files_end_in_one_commit(rig: Rig) -> None:
    first = rig.directory / "blob-0.ndjson"
    second = rig.directory / "blob-1.ndjson"
    first.write_bytes(lines({"id": 1}, {"id": 2}))
    second.write_bytes(lines({"id": 3}))
    receipt = run(rig, request(rig, inputs=[first, second]))
    assert receipt["rowsAdded"] == "3"
    assert len(rig.rest.commits) == 1
    assert [row["id"] for row in rig.rows()] == [1, 2, 3]


def test_a_mapping_selects_the_fields_by_pointer(rig: Rig) -> None:
    content = lines({"order": {"n": 7}, "who": "ana", "extra": 1})
    receipt = run(rig, request(rig, content, mapping=[("/order/n", "id"), ("/who", "name")]))
    assert receipt["rowsAdded"] == "1"
    assert rig.rows()[0]["id"] == 7
    assert rig.rows()[0]["name"] == "ana"


def test_unknown_fields_are_counted_in_the_receipt_when_the_step_ignores_them(rig: Rig) -> None:
    receipt = run(rig, request(rig, lines({"id": 1, "x": 1, "y": 2}, {"id": 2, "z": 3}), ignore_unknown_fields=True))
    assert receipt["ignoredFields"] == "3"
    assert [row["id"] for row in rig.rows()] == [1, 2]


def test_a_receipt_without_ignored_fields_does_not_mention_them(rig: Rig) -> None:
    assert "ignoredFields" not in run(rig, request(rig))


def test_the_flag_of_a_prior_attempt_that_may_have_left_files_is_in_the_receipt(rig: Rig) -> None:
    receipt = run(rig, request(rig, prior_attempt_may_have_left_files=True))
    assert receipt["priorAttemptMayHaveLeftFiles"] is True
    other = run(rig, request(rig, key="run-2:other"))
    assert "priorAttemptMayHaveLeftFiles" not in other


# ------------------------------------------------------------ the key is found already
def test_a_key_that_is_already_in_the_history_is_not_written_again(rig: Rig) -> None:
    first = run(rig, request(rig))
    files = rig.data_files()
    commits = len(rig.rest.commits)

    again = run(rig, request(rig, attempt_id="attempt-2"))

    assert again["foundByLookup"] is True
    assert again["outcome"] == "COMMITTED"
    assert again["snapshotId"] == first["snapshotId"]
    assert again["rowsAdded"] == first["rowsAdded"]
    assert rig.data_files() == files
    assert len(rig.rest.commits) == commits
    assert len(rig.rows()) == 3


def test_a_key_found_deeper_in_the_history_is_found(rig: Rig) -> None:
    first = run(rig, request(rig))
    for number in range(3):
        rig.other_writer(number)
    again = run(rig, request(rig, attempt_id="attempt-2"))
    assert again["foundByLookup"] is True
    assert again["snapshotId"] == first["snapshotId"]
    assert len(rig.rows()) == 3 + 3


def test_a_different_key_commits_again(rig: Rig) -> None:
    run(rig, request(rig))
    second = run(rig, request(rig, key="run-1:other-step", attempt_id="attempt-2"))
    assert second["foundByLookup"] is False
    assert len(rig.rows()) == 6


# ------------------------------------------------------------ a commit that is refused (a race)
def racing(rig: Rig, times: int) -> Callable[[], Table]:
    """A load that is followed by another writer's commit: the table it returns is already stale."""
    state = {"loads": 0}

    def load() -> Table:
        table = rig.load()
        if state["loads"] < times:
            rig.other_writer(state["loads"])
        state["loads"] += 1
        return table

    return load


def test_a_commit_that_lost_a_race_is_tried_again_on_the_new_head_and_leaves_nothing_behind(rig: Rig) -> None:
    receipt = run(rig, request(rig), racing(rig, 1))

    assert receipt["outcome"] == "COMMITTED"
    assert receipt["foundByLookup"] is False
    assert len(rig.rest.commits) == 2
    ids = [row["id"] for row in rig.rows()]
    assert ids == [1, 2, 3, 1_000]
    # one data file of the other writer and one of ours: the files of the refused try were deleted
    assert len(rig.data_files()) == 2
    assert not [name for name in rig.data_files() if "attempt-1-0" in name]
    assert [name for name in rig.data_files() if "attempt-1-1" in name]


def test_conflicts_beyond_the_bound_end_in_a_conflict_with_nothing_left_behind(rig: Rig) -> None:
    failure = failure_of(rig, request(rig, max_commit_retries=2), racing(rig, 10))

    assert (failure.code, failure.reason, failure.column) == ("ICEBERG_COMMIT_CONFLICT", None, None)
    assert len(rig.rest.commits) == 3  # the first try and two more
    assert [row["id"] for row in rig.rows()] == [1_000, 1_001, 1_002]
    assert not [name for name in rig.data_files() if "attempt-1" in name]


def test_with_no_retries_the_first_conflict_is_the_answer(rig: Rig) -> None:
    failure = failure_of(rig, request(rig, max_commit_retries=0), racing(rig, 5))
    assert failure.code == "ICEBERG_COMMIT_CONFLICT"
    assert len(rig.rest.commits) == 1


# ------------------------------------------------------------ a commit whose outcome is unknown
def test_a_commit_that_landed_but_whose_answer_was_lost_is_found_and_not_written_again(rig: Rig) -> None:
    rig.rest.script = [LOST]

    receipt = run(rig, request(rig))

    assert receipt["foundByLookup"] is True
    assert receipt["outcome"] == "COMMITTED"
    assert receipt["rowsAdded"] == "3"
    assert len(rig.rest.commits) == 1
    assert [row["id"] for row in rig.rows()] == [1, 2, 3]
    assert len(rig.data_files()) == 1  # the file the landed commit references is still there


def test_the_same_with_an_answer_of_500_instead_of_a_closed_connection(rig: Rig) -> None:
    rig.rest.script = [LOST_500]
    receipt = run(rig, request(rig))
    assert receipt["foundByLookup"] is True
    assert len(rig.rows()) == 3
    assert len(rig.data_files()) == 1


def test_a_commit_whose_answer_was_500_and_that_did_not_land_is_tried_again(rig: Rig) -> None:
    rig.rest.script = [500]

    receipt = run(rig, request(rig))

    assert receipt["foundByLookup"] is False
    assert len(rig.rest.commits) == 2
    assert [row["id"] for row in rig.rows()] == [1, 2, 3]
    # the files of the first try were not deleted when its outcome was unknown, and are once the second has committed
    assert len(rig.data_files()) == 1
    assert [name for name in rig.data_files() if "attempt-1-1" in name]


def test_a_closed_connection_that_did_not_land_is_tried_again_too(rig: Rig) -> None:
    rig.rest.script = ["reset"]
    receipt = run(rig, request(rig))
    assert receipt["foundByLookup"] is False
    assert len(rig.rest.commits) == 2
    assert len(rig.rows()) == 3
    assert len(rig.data_files()) == 1


def test_an_outcome_that_stays_unknown_ends_unknown_and_deletes_nothing(rig: Rig) -> None:
    rig.rest.answer = 500
    failure = failure_of(rig, request(rig, max_commit_retries=2))
    assert failure.code == "OUTCOME_UNKNOWN"
    assert len(rig.rest.commits) == 3
    # none of the three tries may be deleted: any of them could have landed
    assert len(rig.data_files()) == 3
    assert rig.rows() == []


def test_the_files_of_an_unknown_try_that_the_landed_commit_does_not_reference_are_deleted(rig: Rig) -> None:
    # the first answer is a 500 that did not land and the second one is lost after the commit did: the files of the
    # first try are referenced by nothing, the files of the second by the commit that landed
    rig.rest.script = [500, LOST]

    receipt = run(rig, request(rig))

    assert receipt["foundByLookup"] is True
    assert len(rig.rest.commits) == 2
    assert len(rig.data_files()) == 1
    assert [name for name in rig.data_files() if "attempt-1-1" in name]
    assert [row["id"] for row in rig.rows()] == [1, 2, 3]


# ------------------------------------------------------------ nothing to commit
def test_no_records_is_a_scope_violation_unless_the_step_allows_it(rig: Rig) -> None:
    failure = failure_of(rig, request(rig, b""))
    assert failure.code == "ICEBERG_SCOPE_VIOLATION"
    assert rig.rest.commits == []
    assert rig.data_files() == []


def test_no_records_with_allow_empty_is_a_receipt_with_no_commit(rig: Rig) -> None:
    receipt = run(rig, request(rig, b"\n\n", allow_empty=True))
    assert receipt["outcome"] == "EMPTY_NO_COMMIT"
    assert receipt["foundByLookup"] is False
    assert "snapshotId" not in receipt
    assert receipt["rowsAdded"] == "0"
    assert rig.rest.commits == []
    assert rig.table().current_snapshot() is None


# ------------------------------------------------------------ what is refused before any file
def test_a_record_that_does_not_map_is_refused_with_its_column_and_leaves_no_file(rig: Rig) -> None:
    failure = failure_of(rig, request(rig, lines({"id": 1}, {"id": "x"})))
    assert (failure.code, failure.reason, failure.column) == ("ICEBERG_SCHEMA_MISMATCH", "TYPE", "id")
    assert rig.rest.commits == []
    assert rig.data_files() == []
    assert rig.rows() == []


def test_a_failure_after_files_were_written_deletes_them(rig: Rig, monkeypatch: pytest.MonkeyPatch) -> None:
    # more rows than a batch holds, and a target file size so small that every batch is a file as soon as it arrives, so
    # that files already exist when the bad record is read
    rig.rest.properties["write.target-file-size-bytes"] = "1"
    written: list[str] = []
    from pyiceberg.io.pyarrow import PyArrowFileIO

    real = PyArrowFileIO.new_output

    def spy(self: PyArrowFileIO, location: str) -> Any:
        written.append(location)
        return real(self, location)

    monkeypatch.setattr(PyArrowFileIO, "new_output", spy)
    good = [{"id": number} for number in range(2_500)]

    failure = failure_of(rig, request(rig, lines(*good, {"id": "bad"}), max_batch_rows=1_000))

    assert failure.code == "ICEBERG_SCHEMA_MISMATCH"
    assert [location for location in written if location.endswith(".parquet")], (
        "no data file existed before the failure"
    )
    assert rig.rest.commits == []
    assert rig.data_files() == []


def test_a_budget_that_is_too_small_is_a_limit_exceeded_and_leaves_no_file(rig: Rig) -> None:
    failure = failure_of(rig, request(rig, max_decoded_bytes=20))
    assert failure.code == "ICEBERG_LIMIT_EXCEEDED"
    assert rig.rest.commits == []
    assert rig.data_files() == []


def test_a_table_the_host_cannot_write_is_refused_before_any_file(rig: Rig) -> None:
    # a table whose files are not Parquet: the host writes Parquet only
    rig.sql.create_table("sales.orc", schema=SCHEMA, properties={"write.format.default": "orc"})

    def load() -> Table:
        table: Table = RestCatalog("r", uri=rig.rest.uri).load_table(NAME)
        table.metadata = rig.sql.load_table("sales.orc").metadata
        return table

    failure = failure_of(rig, request(rig), load)
    assert (failure.code, failure.reason) == ("ICEBERG_UNSUPPORTED_TABLE", "FILE_FORMAT")
    assert rig.rest.commits == []
    assert rig.data_files() == []


def test_a_ref_the_table_does_not_have_is_a_table_not_found(rig: Rig) -> None:
    failure = failure_of(rig, request(rig, ref="release"))
    assert failure.code == "ICEBERG_TABLE_NOT_FOUND"
    assert rig.rest.commits == []


def test_files_that_fail_to_be_written_are_a_storage_failure_and_leave_nothing(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    def refuse(self: object, location: str) -> object:
        raise OSError("storage is down")

    monkeypatch.setattr("pyiceberg.io.pyarrow.PyArrowFileIO.new_output", refuse)
    failure = failure_of(rig, request(rig))
    assert failure.code == "STORAGE_UNAVAILABLE"
    assert rig.rest.commits == []


# ------------------------------------------------------------ a partitioned table
def test_a_table_partitioned_by_identity_is_written_from_a_whole_table_and_commits_once(rig: Rig) -> None:
    spec = PartitionSpec(PartitionField(source_id=2, field_id=1000, transform=IdentityTransform(), name="name"))
    rig.sql.drop_table(NAME)
    rig.sql.create_table(NAME, schema=SCHEMA, partition_spec=spec)

    receipt = run(rig, request(rig, lines({"id": 1, "name": "a"}, {"id": 2, "name": "b"}, {"id": 3, "name": "a"})))

    assert receipt["rowsAdded"] == "3"
    assert receipt["dataFilesAdded"] == "2"
    assert len(rig.rest.commits) == 1
    assert [row["id"] for row in rig.rows()] == [1, 2, 3]


# ------------------------------------------------------------ waiting for the writers before deleting what they wrote
def test_the_writers_of_pyiceberg_are_waited_for_before_their_files_are_deleted() -> None:
    # PyIceberg submits every write of a stream to a pool of threads and, when the reader fails, raises while writes are
    # still running: a deletion made at once misses the files they are about to create. `_quiesced` returns when every
    # thread of the pool is idle.
    from pyiceberg.utils.concurrent import ExecutorFactory

    executor = ExecutorFactory.get_or_create()
    started = threading.Event()
    release = threading.Event()
    finished: list[bool] = []

    def slow_write() -> None:
        started.set()
        release.wait(5)
        finished.append(True)

    future = executor.submit(slow_write)
    assert started.wait(5)
    threading.Timer(0.2, release.set).start()

    assert append_mode._quiesced() is True
    assert finished == [True]
    future.result(5)


def test_writers_that_do_not_finish_are_not_waited_for_forever(monkeypatch: pytest.MonkeyPatch) -> None:
    from pyiceberg.utils.concurrent import ExecutorFactory

    executor = ExecutorFactory.get_or_create()
    started = threading.Event()
    release = threading.Event()

    def stuck() -> None:
        started.set()
        release.wait(30)

    future = executor.submit(stuck)
    assert started.wait(5)
    monkeypatch.setattr(append_mode, "_DRAIN_SECONDS", 0.3)
    try:
        assert append_mode._quiesced() is False
    finally:
        release.set()
        future.result(5)


def test_nothing_is_deleted_when_the_writers_cannot_be_waited_for(rig: Rig, monkeypatch: pytest.MonkeyPatch) -> None:
    deleted_from: list[str] = []
    real = attempt_module.remove_attempt_files

    def spy(*args: Any, **kwargs: Any) -> Removal:
        deleted_from.append(args[1])
        return real(*args, **kwargs)

    monkeypatch.setattr("sagat_lake.modes.append.remove_attempt_files", spy)

    monkeypatch.setattr(append_mode, "_quiesced", lambda: False)
    failure_of(rig, request(rig, lines({"id": "bad"})))
    assert deleted_from == []

    monkeypatch.setattr(append_mode, "_quiesced", lambda: True)
    failure_of(rig, request(rig, lines({"id": "bad"}), attempt_id="attempt-2"))
    assert len(deleted_from) == 1


# ------------------------------------------------ the request, a storage error among files, a branch, leftovers
def test_an_attempt_id_is_at_most_sixty_characters_so_that_every_try_can_add_its_number(rig: Rig) -> None:
    request(rig, attempt_id="a" * 60)
    with pytest.raises(ValueError, match="attempt id"):
        request(rig, attempt_id="a" * 61)
    with pytest.raises(ValueError, match="attempt id"):
        request(rig, attempt_id="")


def test_the_commit_retries_are_not_negative(rig: Rig) -> None:
    request(rig, max_commit_retries=0)
    with pytest.raises(ValueError, match="retries"):
        request(rig, max_commit_retries=-1)


def test_a_storage_error_after_some_files_were_written_deletes_them(rig: Rig, monkeypatch: pytest.MonkeyPatch) -> None:
    rig.rest.properties["write.target-file-size-bytes"] = "1"
    from pyiceberg.io.pyarrow import PyArrowFileIO

    real = PyArrowFileIO.new_output
    seen = {"files": 0}

    def flaky(self: PyArrowFileIO, location: str) -> Any:
        if location.endswith(".parquet"):
            seen["files"] += 1
            if seen["files"] == 2:
                raise OSError("storage is down")
        return real(self, location)

    monkeypatch.setattr(PyArrowFileIO, "new_output", flaky)
    good = [{"id": number} for number in range(2_500)]

    failure = failure_of(rig, request(rig, lines(*good), max_batch_rows=1_000))

    assert failure.code == "STORAGE_UNAVAILABLE"
    assert seen["files"] >= 2
    assert rig.rest.commits == []
    assert rig.data_files() == []


def test_a_branch_is_written_and_main_is_left_alone(rig: Rig) -> None:
    rig.other_writer(0)
    table = rig.table()
    head = table.current_snapshot()
    assert head is not None
    with table.manage_snapshots() as manage:
        manage.create_branch(head.snapshot_id, "audit")

    receipt = run(rig, request(rig, ref="audit"))

    table = rig.table()
    audit, main = table.snapshot_by_name("audit"), table.snapshot_by_name("main")
    assert audit is not None
    assert main is not None
    assert str(audit.snapshot_id) == receipt["snapshotId"]
    assert main.snapshot_id == head.snapshot_id
    assert receipt["ref"] == "audit"
    again = run(rig, request(rig, ref="audit", attempt_id="attempt-2"))
    assert again["foundByLookup"] is True


def test_files_that_could_not_be_deleted_are_in_the_receipt_after_a_refused_try(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("sagat_lake.modes.append.remove_attempt_files", lambda *a, **k: Removal(deleted=0, failed=2))
    receipt = run(rig, request(rig), racing(rig, 1))
    assert receipt["uncommittedFiles"] == "2"


def test_a_directory_that_could_not_be_listed_is_in_the_receipt_too(rig: Rig, monkeypatch: pytest.MonkeyPatch) -> None:
    def refuse(*args: Any, **kwargs: Any) -> Removal:
        raise OSError("cannot list")

    monkeypatch.setattr("sagat_lake.modes.append.remove_attempt_files", refuse)
    receipt = run(rig, request(rig), racing(rig, 1))
    assert receipt["uncommittedFiles"] == "1"


def test_files_that_could_not_be_deleted_are_in_the_receipt_after_an_unknown_try(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("sagat_lake.modes.append.remove_attempt_files", lambda *a, **k: Removal(deleted=0, failed=3))
    rig.rest.script = [500]
    receipt = run(rig, request(rig))
    assert receipt["uncommittedFiles"] == "3"
    other = run(rig, request(rig, key="run-2:other", attempt_id="attempt-2"))
    assert "uncommittedFiles" not in other


def test_the_batch_bounds_and_the_meter_of_the_request_reach_the_batches(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    seen: dict[str, Any] = {}
    real = batches_module.to_batches

    def spy(schema: Schema, rows: Any, **kwargs: Any) -> Any:
        seen.update(kwargs)
        return real(schema, rows, **kwargs)

    monkeypatch.setattr("sagat_lake.modes.append.to_batches", spy)

    run(rig, request(rig, max_batch_rows=7, max_batch_bytes=12_345))

    assert (seen["max_rows"], seen["max_bytes"]) == (7, 12_345)
    assert seen["meter"]() > 0  # the meter reads what the reader has spent on the records


def test_the_decoded_bytes_of_the_request_are_the_budget_of_the_reader(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    limits: list[int] = []
    real = records_module.Budget

    def spy(limit: int) -> Any:
        limits.append(limit)
        return real(limit)

    monkeypatch.setattr("sagat_lake.modes.append.Budget", spy)
    run(rig, request(rig, max_decoded_bytes=123_456))
    assert limits == [123_456]


@pytest.mark.parametrize("junk", ["abc", "1x", "007", "-1", "1" * 20, "", " 5", "5\n", "5.0"])
def test_a_count_in_a_snapshot_written_by_someone_else_is_checked_before_it_reaches_a_receipt(
    rig: Rig, junk: str
) -> None:
    table = rig.table()
    row = {"id": 1_000, "name": "other", "amount": Decimal("1.00")}
    table.append(
        pa.Table.from_pylist([row], schema=schema_to_pyarrow(table.schema())), snapshot_properties={EFFECT_KEY: KEY}
    )
    # PyIceberg takes no count from the caller, so the summary is rewritten in the metadata file, as another engine
    # could write it
    path = pathlib.Path(rig.table().metadata_location.removeprefix("file://"))
    document = json.loads(path.read_text())
    document["snapshots"][-1]["summary"].update(
        {"added-records": junk, "added-data-files": junk, "added-files-size": junk}
    )
    path.write_text(json.dumps(document))

    receipt = run(rig, request(rig))

    assert receipt["foundByLookup"] is True
    assert (receipt["rowsAdded"], receipt["dataFilesAdded"], receipt["bytesWritten"]) == ("0", "0", "0")


def test_a_count_written_by_someone_else_in_the_contracts_own_form_is_kept(rig: Rig) -> None:
    table = rig.table()
    row = {"id": 1_000, "name": "other", "amount": Decimal("1.00")}
    table.append(
        pa.Table.from_pylist([row], schema=schema_to_pyarrow(table.schema())), snapshot_properties={EFFECT_KEY: KEY}
    )
    path = pathlib.Path(rig.table().metadata_location.removeprefix("file://"))
    document = json.loads(path.read_text())
    document["snapshots"][-1]["summary"].update({"added-records": "9223372036854775807", "added-data-files": "0"})
    path.write_text(json.dumps(document))

    receipt = run(rig, request(rig))

    assert (receipt["rowsAdded"], receipt["dataFilesAdded"]) == ("9223372036854775807", "0")


# ---------------------------------------------------------------------------------------------- a Parquet input
def parquet_blob(rig: Rig, rows: list[dict[str, Any]], name: str = "blob-0.parquet") -> pathlib.Path:
    path = rig.directory / name
    pq.write_table(pa.Table.from_pylist(rows), path)
    return path


def test_a_parquet_blob_and_an_ndjson_file_end_in_one_commit(rig: Rig) -> None:
    blob = parquet_blob(
        rig, [{"id": 10, "name": "p", "amount": Decimal("5.25")}, {"id": 11, "name": None, "amount": None}]
    )
    inline = rig.directory / "inline.ndjson"
    inline.write_bytes(lines({"id": 1, "name": "a"}))

    receipt = run(rig, request(rig, inputs=[InputFile(blob, "parquet"), InputFile(inline)]))

    assert receipt["rowsAdded"] == "3"
    assert len(rig.rest.commits) == 1
    assert [row["id"] for row in rig.rows()] == [1, 10, 11]
    assert rig.rows()[1]["amount"] == Decimal("5.25")


def test_a_parquet_cell_that_does_not_fit_the_table_is_a_mismatch_with_its_column(rig: Rig) -> None:
    blob = parquet_blob(rig, [{"id": 1, "name": "a", "amount": Decimal("1.234")}])
    failure = failure_of(rig, request(rig, inputs=[InputFile(blob, "parquet")]))
    assert (failure.code, failure.reason, failure.column) == ("ICEBERG_SCHEMA_MISMATCH", "SCALE_OVERFLOW", "amount")
    assert rig.rest.commits == []
    assert rig.data_files() == []


def test_a_file_that_is_not_parquet_is_a_mismatch_and_nothing_is_written(rig: Rig) -> None:
    path = rig.directory / "blob-0.parquet"
    path.write_bytes(b"id,name\n1,a\n")
    failure = failure_of(rig, request(rig, inputs=[InputFile(path, "parquet")]))
    assert (failure.code, failure.reason, failure.column) == ("ICEBERG_SCHEMA_MISMATCH", "TYPE", None)
    assert rig.data_files() == []


def test_the_decoded_budget_is_spent_by_both_formats_together(rig: Rig) -> None:
    blob = parquet_blob(rig, [{"id": number, "name": "x" * 100} for number in range(50)])
    size = pq.ParquetFile(blob).metadata.row_group(0).total_byte_size
    failure = failure_of(rig, request(rig, inputs=[InputFile(blob, "parquet")], max_decoded_bytes=size))
    assert failure.code == "ICEBERG_LIMIT_EXCEEDED"
    assert rig.data_files() == []


# --------------------------------------------------------------------------------------- the schema of the receipts
RECEIPTS = json.loads(
    (
        pathlib.Path(__file__).resolve().parents[3] / "specs/035-iceberg-write-steps/contracts/receipts.schema.json"
    ).read_text()
)


def conforms(receipt: dict[str, Any]) -> list[str]:
    return problems(receipt, RECEIPTS["$defs"]["iceberg"], RECEIPTS["$defs"])


def test_the_checker_catches_what_the_schema_forbids() -> None:
    good = {
        "kind": "iceberg",
        "outcome": "COMMITTED",
        "effectKey": "k",
        "foundByLookup": False,
        "mode": "append",
        "ref": "main",
    }
    assert conforms(good) == []
    assert conforms({**good, "extra": 1})
    assert conforms({**good, "rowsAdded": "007"})
    assert conforms({**good, "rowsAdded": 7})
    assert conforms({**good, "outcome": "DONE"})
    assert conforms({k: v for k, v in good.items() if k != "ref"})
    assert conforms({**good, "formatVersion": 3})
    assert conforms({**good, "snapshotId": "12ab"})


def test_every_receipt_the_host_writes_conforms_to_the_schema_the_runner_rebuilds_it_from(
    rig: Rig, monkeypatch: pytest.MonkeyPatch
) -> None:
    receipts = []
    # committed, with every optional member the host can add
    records = lines({"id": 1, "x": 1}, {"id": 2, "y": 2})
    receipts.append(run(rig, request(rig, records, ignore_unknown_fields=True, prior_attempt_may_have_left_files=True)))
    # found by lookup
    receipts.append(run(rig, request(rig, records, ignore_unknown_fields=True, attempt_id="attempt-2")))
    # nothing to commit
    receipts.append(run(rig, request(rig, b"", key="run-2:empty", allow_empty=True)))
    # a refused try whose files could not all be deleted
    monkeypatch.setattr("sagat_lake.modes.append.remove_attempt_files", lambda *a, **k: Removal(deleted=0, failed=2))
    receipts.append(run(rig, request(rig, lines({"id": 9}), key="run-3:raced"), racing(rig, 1)))

    assert [receipt["outcome"] for receipt in receipts] == ["COMMITTED", "COMMITTED", "EMPTY_NO_COMMIT", "COMMITTED"]
    assert [receipt["foundByLookup"] for receipt in receipts] == [False, True, False, False]
    members = set().union(*(set(receipt) for receipt in receipts))
    assert {
        "ignoredFields",
        "priorAttemptMayHaveLeftFiles",
        "uncommittedFiles",
        "snapshotId",
        "bytesWritten",
    } <= members
    for receipt in receipts:
        assert conforms(receipt) == [], receipt
