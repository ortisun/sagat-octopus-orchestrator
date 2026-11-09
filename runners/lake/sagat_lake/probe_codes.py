"""The codes a check of a probe may fail with (feature 035: D035-150), and nothing else.

These are not the codes of a ``failure`` line. A check that fails is a *result*: it travels in the ``receipt`` of the
line, and the server reads it back through its own codec and refuses a code the check may not fail with
(``ProbeCheck::allows``, ``crates/domain/src/database/lake_probe.rs``). A ``failure`` line is another thing, and the
runner reads only the codes of ``HOST_MAY_REPORT`` from it, so that a host cannot make a run look timed out or refused
(``crates/infra-lake/src/protocol.rs``).

That is why the table is a module of its own and the two tests that read every code the host writes as a literal, one
in Python (``test_host_protocol.py``) and one in Rust (``crates/infra-lake/tests/protocol.rs``), skip this file and no
other: a code written anywhere else in the host is still one a failure line could carry. In their place each side
holds this table to the server's, so a code cannot be added here without the domain saying the check may have it.
"""

from __future__ import annotations

from collections.abc import Mapping

from .config_hygiene import LOCATION_NOT_ALLOWED
from .request import PROBE_CHECKS

CATALOG_AUTH, WAREHOUSE, STORAGE_ACCESS = PROBE_CHECKS

AUTHENTICATION_FAILED = "DATABASE_AUTHENTICATION_FAILED"
TLS_FAILED = "DATABASE_TLS_FAILED"
DESTINATION_DENIED = "DATABASE_DESTINATION_DENIED"
TIMEOUT = "DATABASE_TIMEOUT"
CATALOG_UNAVAILABLE = "CATALOG_UNAVAILABLE"
STORAGE_UNAVAILABLE = "STORAGE_UNAVAILABLE"

# What each check may fail with: the server's table, read by a test of each side.
CHECK_CODES: Mapping[str, frozenset[str]] = {
    CATALOG_AUTH: frozenset({AUTHENTICATION_FAILED, TLS_FAILED, DESTINATION_DENIED, CATALOG_UNAVAILABLE, TIMEOUT}),
    WAREHOUSE: frozenset({AUTHENTICATION_FAILED, CATALOG_UNAVAILABLE, LOCATION_NOT_ALLOWED, TIMEOUT}),
    STORAGE_ACCESS: frozenset({AUTHENTICATION_FAILED, TLS_FAILED, DESTINATION_DENIED, STORAGE_UNAVAILABLE, TIMEOUT}),
}
