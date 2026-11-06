"""The search for the effect key before every commit (feature 035: D035-030, FR-035-018, FR-035-019).

Every commit of an attempt carries the effect key in the summary of its snapshot. Before it writes, and before every
retry, the host walks the parent snapshots of the target ref from its head looking for that key. If it is there, the
step is already done and nothing is written. The walk is *conclusive* when it reaches a snapshot older than the
first attempt's start minus five minutes (a margin for the catalog's clock, which stamps the snapshots) or the first
snapshot of the table, along an unbroken chain. Otherwise it is *inconclusive*: expiry broke the chain, or the
history is longer than the bound. Then the outcome is unknown, and the step never assumes absence.

The functions here are pure: they read a mapping of snapshots, so they are tested without a catalog.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass

from .failures import Failure

EFFECT_KEY = "sagat.effect-key"
# The catalog stamps the snapshots with its own clock. A commit of ours can carry a time up to this long before our
# start.
CLOCK_MARGIN_MS = 5 * 60 * 1000
# The most snapshots one walk looks at; a longer history is inconclusive rather than walked without end.
MAX_SNAPSHOTS = 10_000
OUTCOME_UNKNOWN = "OUTCOME_UNKNOWN"


@dataclass(frozen=True)
class Snapshot:
    """What the walk needs of a snapshot: where it comes from, when the catalog made it and its summary."""

    snapshot_id: int
    parent_id: int | None
    timestamp_ms: int
    summary: Mapping[str, str]


@dataclass(frozen=True)
class Found:
    """The commit of the step is in the history of the ref."""

    snapshot_id: int


@dataclass(frozen=True)
class Absent:
    """The history covers the time since the first attempt began, and the key is not in it."""


@dataclass(frozen=True)
class Inconclusive:
    """The history does not cover the time since the first attempt began, so nothing can be said."""


def search(
    effect_key: str,
    *,
    head: int | None,
    snapshots: Mapping[int, Snapshot],
    first_attempt_started_at_ms: int,
    max_snapshots: int = MAX_SNAPSHOTS,
) -> Found | Absent | Inconclusive:
    """Walks from ``head`` towards the first snapshot, looking for ``effect_key``."""
    if head is None:
        # The ref has no commit at all: there is no history in which the key could be.
        return Absent()
    window_start = first_attempt_started_at_ms - CLOCK_MARGIN_MS
    current = snapshots.get(head)
    looked_at = 0
    while current is not None:
        looked_at += 1
        if looked_at > max_snapshots:
            return Inconclusive()
        if current.summary.get(EFFECT_KEY) == effect_key:
            return Found(snapshot_id=current.snapshot_id)
        if current.timestamp_ms < window_start:
            # Older than anything the first attempt could have committed: the history covers the window.
            return Absent()
        if current.parent_id is None:
            # The first snapshot of the table: nothing before it.
            return Absent()
        current = snapshots.get(current.parent_id)
    # A parent the table no longer holds, inside the window: expiry broke the chain.
    return Inconclusive()


def lookup(
    effect_key: str,
    *,
    ref: str,
    refs: Mapping[str, int | None],
    snapshots: Mapping[int, Snapshot],
    first_attempt_started_at_ms: int,
) -> Found | Absent:
    """The result of the search on the target ``ref``, or the failure for review.

    A key found on the target ref is found. A key found only on another ref is an inconsistency, and so is a search that
    cannot decide: both are ``OUTCOME_UNKNOWN``, which goes to review and is not repeated (FR-035-019).
    """
    on_target = search(
        effect_key,
        head=refs.get(ref),
        snapshots=snapshots,
        first_attempt_started_at_ms=first_attempt_started_at_ms,
    )
    if isinstance(on_target, Found):
        return on_target
    for other, head in refs.items():
        if other == ref:
            continue
        elsewhere = search(
            effect_key, head=head, snapshots=snapshots, first_attempt_started_at_ms=first_attempt_started_at_ms
        )
        if isinstance(elsewhere, Found):
            raise Failure(OUTCOME_UNKNOWN)
    if isinstance(on_target, Inconclusive):
        raise Failure(OUTCOME_UNKNOWN)
    return on_target
