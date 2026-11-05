"""The failures the host reports (feature 035, `contracts/errors.md`, `crates/domain/src/database/failure.rs`).

A failure is a code of the closed set the runner knows and, for a schema mismatch, the column and a reason. It has no
free text: the runner refuses a result line that has any, and a value that could travel in a message never leaves the
host. The constructor refuses what the runner would, so a bug cannot produce a line that fails the whole attempt as a
protocol break.
"""

from __future__ import annotations

import re

# The codes the skeleton itself raises. The others arrive with the operations that raise them, each in the one table of
# `failure.rs`, which a test of this directory reads.
LAKE_HOST_PROTOCOL = "LAKE_HOST_PROTOCOL"
WORKER_ERROR = "WORKER_ERROR"

_CODE = re.compile(r"[A-Z][A-Z0-9_]{0,63}")
_REASON = re.compile(r"[A-Z0-9_]{1,48}")
_MAX_COLUMN = 128


class Failure(Exception):
    """An operation that ended without a receipt, for a reason the contract names."""

    def __init__(self, code: str, *, column: str | None = None, reason: str | None = None) -> None:
        if not _CODE.fullmatch(code):
            raise ValueError("a failure has a code of capital letters, digits and underscores")
        if column is not None and (not 1 <= len(column) <= _MAX_COLUMN or not column.isprintable()):
            raise ValueError("a failure names a column of 1 to 128 printable characters")
        if reason is not None and not _REASON.fullmatch(reason):
            raise ValueError("a failure has a reason of 1 to 48 capital letters, digits and underscores")
        # the message is the code: nothing that could carry a value
        super().__init__(code)
        self.code = code
        self.column = column
        self.reason = reason

    def as_member(self) -> dict[str, str]:
        """The `failure` member of the result line."""
        member = {"code": self.code}
        if self.column is not None:
            member["column"] = self.column
        if self.reason is not None:
            member["reason"] = self.reason
        return member
