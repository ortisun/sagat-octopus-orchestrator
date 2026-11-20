"""Reading the records of an attempt from the NDJSON files the runner wrote (feature 035: T035-057, D035-033, D035-063).

The runner streams the inline records and the NDJSON blobs of a step into ``in/``; the host reads them one line at a
time, so that a file is never held whole in memory. What is read is not trusted: a line is a bounded amount of text, the
total is a bounded amount of bytes, an object does not repeat a key (the second would silently replace the first), a
number keeps the digits it was written with, and nothing that is wrong carries a value into the failure.
"""

from __future__ import annotations

import pathlib
from decimal import Decimal

import pytest

from sagat_lake.failures import Failure
from sagat_lake.records import Budget, read_ndjson

MiB = 1024 * 1024


def write(directory: pathlib.Path, content: bytes, name: str = "inline.ndjson") -> pathlib.Path:
    path = directory / name
    path.write_bytes(content)
    return path


def read(path: pathlib.Path, *, limit: int = 10 * MiB, line: int = MiB) -> list[object]:
    return list(read_ndjson(path, Budget(limit), max_line_bytes=line))


def refused(path: pathlib.Path, code: str, reason: str | None, **kwargs: int) -> Failure:
    with pytest.raises(Failure) as caught:
        read(path, **kwargs)
    failure = caught.value
    assert (failure.code, failure.reason, failure.column) == (code, reason, None)
    # nothing but the code: no value, no line, no position
    assert str(failure) == code
    return failure


# ---------------------------------------------------------------------------------------------------- what is read
def test_the_records_come_out_in_the_order_of_the_file(tmp_path: pathlib.Path) -> None:
    path = write(tmp_path, b'{"id": 1}\n{"id": 2}\n{"id": 3}\n')
    assert read(path) == [{"id": 1}, {"id": 2}, {"id": 3}]


def test_a_last_line_with_no_newline_is_a_record(tmp_path: pathlib.Path) -> None:
    assert read(write(tmp_path, b'{"id": 1}\n{"id": 2}')) == [{"id": 1}, {"id": 2}]


def test_an_empty_file_has_no_record(tmp_path: pathlib.Path) -> None:
    assert read(write(tmp_path, b"")) == []


def test_a_blank_line_is_not_a_record_and_a_windows_line_end_is_a_line_end(tmp_path: pathlib.Path) -> None:
    assert read(write(tmp_path, b'{"id": 1}\r\n\r\n   \n\t\n{"id": 2}\r\n')) == [{"id": 1}, {"id": 2}]


def test_a_number_keeps_the_digits_it_was_written_with(tmp_path: pathlib.Path) -> None:
    (record,) = read(write(tmp_path, b'{"price": 12.50, "n": 7, "big": 123456789012345678901234567890, "e": 1.5e3}'))
    assert isinstance(record, dict)
    assert record["price"] == Decimal("12.50")
    assert str(record["price"]) == "12.50"
    assert record["n"] == 7
    assert isinstance(record["n"], int)
    assert record["big"] == 123456789012345678901234567890
    assert str(record["e"]) == "1.5E+3"


def test_text_is_utf_8_and_escapes_are_decoded(tmp_path: pathlib.Path) -> None:
    (record,) = read(write(tmp_path, '{"s": "olá \\u00e9 \\ud83d\\ude00 😀"}'.encode()))
    assert record == {"s": "olá é 😀 😀"}


def test_not_a_number_and_the_infinities_reach_the_mapping_which_decides_for_each_column(
    tmp_path: pathlib.Path,
) -> None:
    (record,) = read(write(tmp_path, b'{"a": NaN, "b": Infinity, "c": -Infinity}'))
    assert isinstance(record, dict)
    assert str(record["a"]) == "nan"
    assert record["b"] == float("inf")
    assert record["c"] == float("-inf")


def test_nesting_up_to_a_sensible_depth_is_read(tmp_path: pathlib.Path) -> None:
    assert read(write(tmp_path, b'{"a": {"b": [{"c": [1, 2, {"d": null}]}]}}')) == [
        {"a": {"b": [{"c": [1, 2, {"d": None}]}]}}
    ]


# ---------------------------------------------------------------------------------------------------- what is refused
@pytest.mark.parametrize(
    "line",
    [
        b"not json",
        b'{"a": 1',  # cut off
        b'{"a": 1} {"b": 2}',  # two records on one line
        b"{'a': 1}",
        b'{"a": 1,}',
        b'{"a": "x\ty"}',  # a raw tab inside a string
        b"[1, 2]",  # valid JSON, not an object
        b"5",
        b'"text"',
        b"null",
        b"true",
    ],
)
def test_a_line_that_is_not_a_json_object_is_a_type_mismatch_without_its_content(
    tmp_path: pathlib.Path, line: bytes
) -> None:
    refused(write(tmp_path, b'{"ok": 1}\n' + line + b"\n"), "ICEBERG_SCHEMA_MISMATCH", "TYPE")


@pytest.mark.parametrize(
    "content",
    [
        b'{"a": "\xff"}',  # not UTF-8
        b'{"a": "\xc3("}',
        b"\xef\xbb\xbf" + b'{"a": 1}',  # a byte order mark
        '{"a": 1}'.encode("utf-16"),
        '{"a": 1}'.encode("utf-32"),
    ],
)
def test_text_that_is_not_utf_8_is_a_type_mismatch(tmp_path: pathlib.Path, content: bytes) -> None:
    refused(write(tmp_path, content + b"\n"), "ICEBERG_SCHEMA_MISMATCH", "TYPE")


@pytest.mark.parametrize(
    "line",
    [
        b'{"a": 1, "a": 2}',
        b'{"a": {"b": 1, "b": 1}}',
        b'{"a": [{"b": 1, "b": 2}]}',
        b'{"a": 1, "b": 2, "a": 1}',
        b'{"\\u0061": 1, "a": 2}',  # the same key written two ways
    ],
)
def test_a_key_twice_in_one_object_is_refused_because_the_second_would_replace_the_first(
    tmp_path: pathlib.Path, line: bytes
) -> None:
    refused(write(tmp_path, line + b"\n"), "ICEBERG_SCHEMA_MISMATCH", "TYPE")


@pytest.mark.parametrize("blank", ["\u00a0", "\u2003", "\u3000", "\x0b", "\x0c", "\x1c"])
def test_only_the_four_blanks_of_json_are_blank(tmp_path: pathlib.Path, blank: str) -> None:
    # a line of any other kind of space is not an empty line: it is a line that is not a record
    refused(write(tmp_path, f"{blank}\n".encode()), "ICEBERG_SCHEMA_MISMATCH", "TYPE")
    refused(write(tmp_path, f'{{"a": 1}}{blank}\n'.encode()), "ICEBERG_SCHEMA_MISMATCH", "TYPE")


def test_a_key_in_two_different_objects_is_fine(tmp_path: pathlib.Path) -> None:
    assert read(write(tmp_path, b'{"a": {"a": 1}, "b": {"a": 2}}')) == [{"a": {"a": 1}, "b": {"a": 2}}]


def test_nesting_deeper_than_the_decoder_goes_is_refused_and_does_not_crash(tmp_path: pathlib.Path) -> None:
    deep = b'{"a": ' + b"[" * 200_000 + b"]" * 200_000 + b"}"
    refused(write(tmp_path, deep + b"\n"), "ICEBERG_SCHEMA_MISMATCH", "TYPE", line=MiB)


@pytest.mark.parametrize("literal", [b"9" * 5_000, b"-" + b"9" * 5_000])
def test_an_integer_of_more_digits_than_python_reads_is_an_overflow_not_a_crash(
    tmp_path: pathlib.Path, literal: bytes
) -> None:
    refused(write(tmp_path, b'{"n": ' + literal + b"}\n"), "ICEBERG_SCHEMA_MISMATCH", "OVERFLOW")


@pytest.mark.parametrize("literal", [b"1e999999999999999999999", b"-1e-999999999999999999999"])
def test_a_number_whose_exponent_a_decimal_cannot_hold_is_an_overflow_not_a_crash(
    tmp_path: pathlib.Path, literal: bytes
) -> None:
    refused(write(tmp_path, b'{"n": ' + literal + b"}\n"), "ICEBERG_SCHEMA_MISMATCH", "OVERFLOW")


def test_the_records_before_a_bad_line_are_delivered_and_then_the_failure_comes(tmp_path: pathlib.Path) -> None:
    path = write(tmp_path, b'{"id": 1}\n{"id": 2}\nbroken\n{"id": 4}\n')
    records = read_ndjson(path, Budget(MiB))
    assert next(records) == {"id": 1}
    assert next(records) == {"id": 2}
    with pytest.raises(Failure):
        next(records)


# ------------------------------------------------------------------------------------------------------------- bounds
def test_a_line_at_the_limit_is_read_and_one_byte_over_is_refused(tmp_path: pathlib.Path) -> None:
    exactly = b'{"s": "' + b"x" * 20 + b'"}'  # 29 bytes, and 30 with its line end
    assert len(exactly) == 29
    assert read(write(tmp_path, exactly + b"\n"), line=30) == [{"s": "x" * 20}]
    assert read(write(tmp_path, exactly), line=29) == [{"s": "x" * 20}]
    refused(write(tmp_path, exactly + b"\n"), "ICEBERG_LIMIT_EXCEEDED", None, line=29)
    refused(write(tmp_path, exactly), "ICEBERG_LIMIT_EXCEEDED", None, line=28)


def test_a_line_with_no_end_is_never_read_whole(tmp_path: pathlib.Path) -> None:
    # two mebibytes with no newline, a limit of one: the reader stops after one mebibyte and one byte
    path = write(tmp_path, b'{"s": "' + b"x" * (2 * MiB) + b'"}')
    refused(path, "ICEBERG_LIMIT_EXCEEDED", None, line=MiB)


class _Spy:
    """A file that records how much each call to ``readline`` asked for."""

    def __init__(self, path: pathlib.Path, asked: list[int]) -> None:
        self._file = open(path, "rb")  # noqa: SIM115
        self._asked = asked

    def __enter__(self) -> _Spy:
        return self

    def __exit__(self, *exc: object) -> None:
        self._file.close()

    def readline(self, size: int = -1) -> bytes:
        self._asked.append(size)
        return self._file.readline(size)


def test_the_reader_never_asks_for_more_than_a_line_may_hold(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # `readline()` with no limit would read three mebibytes into memory before the length could be judged
    path = write(tmp_path, b'{"s": "' + b"x" * (3 * MiB) + b'"}')
    asked: list[int] = []
    monkeypatch.setattr(pathlib.Path, "open", lambda self, mode="r": _Spy(self, asked))
    with pytest.raises(Failure):
        list(read_ndjson(path, Budget(10 * MiB), max_line_bytes=MiB))
    assert asked
    assert all(0 < size <= MiB + 1 for size in asked)


def test_the_total_of_the_bytes_read_is_bounded_and_the_edge_is_inclusive(tmp_path: pathlib.Path) -> None:
    content = b'{"id": 1}\n{"id": 2}\n'
    path = write(tmp_path, content)
    assert read(path, limit=len(content)) == [{"id": 1}, {"id": 2}]
    refused(path, "ICEBERG_LIMIT_EXCEEDED", None, limit=len(content) - 1)


def test_blank_lines_and_line_ends_count_as_bytes(tmp_path: pathlib.Path) -> None:
    content = b'\n\n\n{"id": 1}\r\n'
    path = write(tmp_path, content)
    assert read(path, limit=len(content)) == [{"id": 1}]
    refused(path, "ICEBERG_LIMIT_EXCEEDED", None, limit=len(content) - 1)


def test_the_budget_is_shared_by_every_file_of_a_step(tmp_path: pathlib.Path) -> None:
    one = write(tmp_path, b'{"id": 1}\n', "blob-0.ndjson")
    two = write(tmp_path, b'{"id": 2}\n', "blob-1.ndjson")
    budget = Budget(15)
    assert list(read_ndjson(one, budget)) == [{"id": 1}]
    with pytest.raises(Failure) as caught:
        list(read_ndjson(two, budget))
    assert caught.value.code == "ICEBERG_LIMIT_EXCEEDED"
    assert budget.used > 15


def test_a_budget_of_zero_reads_nothing_and_a_file_that_is_empty_costs_nothing(tmp_path: pathlib.Path) -> None:
    assert read(write(tmp_path, b""), limit=0) == []
    refused(write(tmp_path, b"\n", "b.ndjson"), "ICEBERG_LIMIT_EXCEEDED", None, limit=0)


def test_a_budget_is_never_negative() -> None:
    with pytest.raises(ValueError, match="budget"):
        Budget(-1)
