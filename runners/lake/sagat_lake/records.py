"""The records of an attempt, read from the NDJSON files the runner wrote (feature 035: T035-057, D035-033, D035-063).

The runner streams the inline records and the NDJSON blobs of a step into ``in/``; the host reads them one line at
a time, so a file is never held whole in memory. What it reads is not trusted:

* a line is a bounded number of bytes and the files of a step share a bounded budget, so the cost of decoding is the
  runner's to decide and not the data's (D035-063);
* the text is UTF-8 and nothing else, and a line is one JSON object;
* an object does not repeat a key, because the second would silently replace the first;
* a number keeps the digits it was written with (``12.50`` is a ``Decimal``), for the mapping to judge;
* nothing that is wrong puts a value, a line or a position into the failure, which is a code and a reason (the contract
  has no code for a malformed line: it is ``ICEBERG_SCHEMA_MISMATCH``, ``TYPE``, and one too large to read is
  ``OVERFLOW``).
"""

from __future__ import annotations

import json
from collections.abc import Iterator
from decimal import Decimal
from pathlib import Path
from typing import Any

from .compat import LIMIT_EXCEEDED
from .failures import Failure
from .mapping import SCHEMA_MISMATCH

# The most bytes of one line, its terminator included. A record the platform writes is far smaller than this.
MAX_LINE_BYTES = 16 * 1024 * 1024
_BLANK = " \t\r\n"


class Budget:
    """The bytes a step may read in all its files; going over fails with ``ICEBERG_LIMIT_EXCEEDED``."""

    def __init__(self, limit: int) -> None:
        if limit < 0:
            raise ValueError("a budget is not negative")
        self.limit = limit
        self.used = 0

    def spend(self, size: int) -> None:
        self.used += size
        if self.used > self.limit:
            raise Failure(LIMIT_EXCEEDED)


class _RepeatedKey(Exception):
    """An object that holds the same key twice."""


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = dict(pairs)
    if len(result) != len(pairs):
        raise _RepeatedKey
    return result


def read_ndjson(path: Path, budget: Budget, *, max_line_bytes: int = MAX_LINE_BYTES) -> Iterator[dict[str, Any]]:
    """The records of ``path``, one per line, in order. A blank line is not a record."""
    with path.open("rb") as stream:
        while True:
            # One byte more than the limit is enough to tell a line that is too long, and a line with no end is never
            # read whole.
            line = stream.readline(max_line_bytes + 1)
            if not line:
                return
            budget.spend(len(line))
            if len(line) > max_line_bytes:
                raise Failure(LIMIT_EXCEEDED)
            yield from _record(line)


def _record(line: bytes) -> Iterator[dict[str, Any]]:
    try:
        text = line.decode("utf-8").strip(_BLANK)
    except UnicodeDecodeError:
        raise Failure(SCHEMA_MISMATCH, reason="TYPE") from None
    if not text:
        return
    try:
        value = json.loads(text, parse_float=Decimal, object_pairs_hook=_object)
    except (json.JSONDecodeError, _RepeatedKey, RecursionError):
        raise Failure(SCHEMA_MISMATCH, reason="TYPE") from None
    except (ValueError, ArithmeticError):
        # an integer of more than 4300 digits, or an exponent a Decimal cannot hold: outside every column
        raise Failure(SCHEMA_MISMATCH, reason="OVERFLOW") from None
    if not isinstance(value, dict):
        raise Failure(SCHEMA_MISMATCH, reason="TYPE")
    yield value
