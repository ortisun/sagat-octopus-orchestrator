"""The search for the effect key before every commit (feature 035: T035-049, D035-030, FR-035-018, FR-035-019).

The host walks the parent snapshots of the target ref from its head, looking for the effect key in the summary of
each. The walk is *conclusive* when it reaches a snapshot older than the first attempt's start minus five minutes
(the margin for the catalog's clock) or the first snapshot of the table, along an unbroken chain; otherwise it is
*inconclusive*, and the outcome is unknown: the step never assumes absence.
"""

from __future__ import annotations

import pytest

from sagat_lake.failures import Failure
from sagat_lake.lookup import MAX_SNAPSHOTS, Absent, Found, Inconclusive, Snapshot, lookup, search

KEY = "8f1c:save-orders"
MARGIN = 5 * 60 * 1000
START = 1_000_000_000_000  # the first attempt's start, in milliseconds


def snap(snapshot_id: int, parent: int | None, at: int, key: str | None = None) -> Snapshot:
    summary = {"sagat.effect-key": key} if key is not None else {}
    return Snapshot(snapshot_id=snapshot_id, parent_id=parent, timestamp_ms=at, summary=summary)


def table(*snapshots: Snapshot) -> dict[int, Snapshot]:
    return {s.snapshot_id: s for s in snapshots}


# -------------------------------------------------------------------------------------------------------------- found
def test_the_key_at_the_head_is_found() -> None:
    snapshots = table(snap(1, None, START - 10 * MARGIN), snap(2, 1, START + 10, KEY))
    assert search(KEY, head=2, snapshots=snapshots, first_attempt_started_at_ms=START) == Found(snapshot_id=2)


def test_the_key_deeper_in_the_chain_is_found() -> None:
    snapshots = table(
        snap(1, None, START - 10 * MARGIN),
        snap(2, 1, START + 10, KEY),
        snap(3, 2, START + 20),
        snap(4, 3, START + 30),
    )
    assert search(KEY, head=4, snapshots=snapshots, first_attempt_started_at_ms=START) == Found(snapshot_id=2)


def test_another_effect_key_or_a_part_of_ours_is_not_ours() -> None:
    snapshots = table(
        snap(1, None, START - 10 * MARGIN),
        snap(2, 1, START + 10, KEY + "x"),
        snap(3, 2, START + 20, KEY[:-1]),
        snap(4, 3, START + 30, "other-key"),
    )
    assert search(KEY, head=4, snapshots=snapshots, first_attempt_started_at_ms=START) == Absent()


# ---------------------------------------------------------------------------------------------- absent and conclusive
def test_a_walk_that_reaches_a_snapshot_older_than_the_start_minus_the_margin_is_conclusive() -> None:
    snapshots = table(
        snap(1, None, START - 10 * MARGIN),
        snap(2, 1, START - MARGIN - 1),  # older than the start minus five minutes
        snap(3, 2, START + 10),
    )
    assert search(KEY, head=3, snapshots=snapshots, first_attempt_started_at_ms=START) == Absent()


def test_the_walk_stops_at_the_old_snapshot_and_does_not_need_what_is_before_it() -> None:
    # The parent of snapshot 2 has expired. That does not matter: 2 is already older than the window.
    snapshots = table(snap(2, 1, START - MARGIN - 1), snap(3, 2, START + 10))
    assert search(KEY, head=3, snapshots=snapshots, first_attempt_started_at_ms=START) == Absent()


def test_the_old_snapshot_itself_is_checked_for_the_key() -> None:
    snapshots = table(snap(2, None, START - MARGIN - 1, KEY), snap(3, 2, START + 10))
    assert search(KEY, head=3, snapshots=snapshots, first_attempt_started_at_ms=START) == Found(snapshot_id=2)


def test_a_table_with_no_commit_on_the_ref_has_nothing_to_find() -> None:
    assert search(KEY, head=None, snapshots={}, first_attempt_started_at_ms=START) == Absent()


# ---------------------------------------- reaching the first snapshot
def test_reaching_the_first_snapshot_is_conclusive_even_when_it_is_recent() -> None:
    # The whole history is newer than the window, and it starts here: nothing before it can hold the key.
    snapshots = table(snap(1, None, START + 1), snap(2, 1, START + 2))
    assert search(KEY, head=2, snapshots=snapshots, first_attempt_started_at_ms=START) == Absent()


# --------------------------------------------------------------------------------------------- the five-minute margin
def test_a_snapshot_exactly_at_the_margin_is_not_older_than_it() -> None:
    # at start - margin: not older, so the walk goes on to its parent, which is missing: inconclusive
    snapshots = table(snap(2, 1, START - MARGIN), snap(3, 2, START + 10))
    assert search(KEY, head=3, snapshots=snapshots, first_attempt_started_at_ms=START) == Inconclusive()


def test_a_snapshot_one_millisecond_older_than_the_margin_is_conclusive() -> None:
    snapshots = table(snap(2, 1, START - MARGIN - 1), snap(3, 2, START + 10))
    assert search(KEY, head=3, snapshots=snapshots, first_attempt_started_at_ms=START) == Absent()


def test_a_commit_the_catalog_stamped_a_little_before_our_start_is_still_looked_at() -> None:
    # Our clock and the catalog's differ by up to five minutes: this commit may be ours.
    snapshots = table(snap(1, None, START - MARGIN + 60_000, KEY), snap(2, 1, START + 10))
    assert search(KEY, head=2, snapshots=snapshots, first_attempt_started_at_ms=START) == Found(snapshot_id=1)


# --------------------------------------------------------------------------------------------------------- inconclusive
def test_a_broken_chain_is_inconclusive() -> None:
    # snapshot 2 has a parent (1) that the table no longer holds, and 2 is still inside the window
    snapshots = table(snap(2, 1, START + 5), snap(3, 2, START + 10))
    assert search(KEY, head=3, snapshots=snapshots, first_attempt_started_at_ms=START) == Inconclusive()


def test_a_head_the_table_does_not_hold_is_inconclusive() -> None:
    assert search(KEY, head=9, snapshots={}, first_attempt_started_at_ms=START) == Inconclusive()


def test_more_snapshots_than_the_bound_are_inconclusive() -> None:
    # MAX_SNAPSHOTS + 1 recent snapshots in a chain that starts at snapshot 1
    snapshots = table(snap(1, None, START + 1), *(snap(n, n - 1, START + n) for n in range(2, MAX_SNAPSHOTS + 2)))
    assert search(KEY, head=MAX_SNAPSHOTS + 1, snapshots=snapshots, first_attempt_started_at_ms=START) == Inconclusive()


def test_exactly_the_bound_is_still_decided() -> None:
    snapshots = table(snap(1, None, START + 1), *(snap(n, n - 1, START + n) for n in range(2, MAX_SNAPSHOTS + 1)))
    assert search(KEY, head=MAX_SNAPSHOTS, snapshots=snapshots, first_attempt_started_at_ms=START) == Absent()


def test_the_key_within_the_bound_is_found_even_when_the_history_is_long() -> None:
    snapshots = table(
        snap(1, None, START + 1), snap(2, 1, START + 2, KEY), *(snap(n, n - 1, START + n) for n in range(3, 50))
    )
    assert search(KEY, head=49, snapshots=snapshots, first_attempt_started_at_ms=START) == Found(snapshot_id=2)


# ---------------------------------------------------------------------------------------------------- the whole lookup
def refs(**heads: int | None) -> dict[str, int | None]:
    return heads


def test_a_lookup_that_finds_the_key_on_the_target_ref_reports_it() -> None:
    snapshots = table(snap(1, None, START - 10 * MARGIN), snap(2, 1, START + 10, KEY))
    outcome = lookup(KEY, ref="main", refs=refs(main=2), snapshots=snapshots, first_attempt_started_at_ms=START)
    assert outcome == Found(snapshot_id=2)


def test_a_lookup_that_is_conclusive_and_finds_nothing_says_absent() -> None:
    snapshots = table(snap(1, None, START - 10 * MARGIN), snap(2, 1, START + 10))
    outcome = lookup(KEY, ref="main", refs=refs(main=2), snapshots=snapshots, first_attempt_started_at_ms=START)
    assert outcome == Absent()


def test_an_inconclusive_lookup_is_an_unknown_outcome_and_never_an_absence() -> None:
    snapshots = table(snap(3, 2, START + 10))
    with pytest.raises(Failure) as caught:
        lookup(KEY, ref="main", refs=refs(main=3), snapshots=snapshots, first_attempt_started_at_ms=START)
    assert caught.value.code == "OUTCOME_UNKNOWN"


def test_the_key_on_another_ref_is_a_failure_for_review_and_not_a_find() -> None:
    snapshots = table(
        snap(1, None, START - 10 * MARGIN),
        snap(2, 1, START + 10),
        snap(3, 1, START + 20, KEY),  # on the branch `audit`
    )
    with pytest.raises(Failure) as caught:
        lookup(KEY, ref="main", refs=refs(main=2, audit=3), snapshots=snapshots, first_attempt_started_at_ms=START)
    assert caught.value.code == "OUTCOME_UNKNOWN"


def test_the_key_on_the_target_ref_wins_over_the_same_commit_being_visible_from_another_ref() -> None:
    # `audit` was branched from main after our commit: the commit is in both histories, and main is where we wanted it.
    snapshots = table(snap(1, None, START - 10 * MARGIN), snap(2, 1, START + 10, KEY), snap(3, 2, START + 20))
    outcome = lookup(
        KEY, ref="main", refs=refs(main=2, audit=3), snapshots=snapshots, first_attempt_started_at_ms=START
    )
    assert outcome == Found(snapshot_id=2)


def test_a_lookup_does_not_look_at_a_ref_that_does_not_matter_when_the_target_is_empty() -> None:
    snapshots = table(snap(1, None, START - 10 * MARGIN))
    outcome = lookup(
        KEY, ref="main", refs=refs(main=None, audit=1), snapshots=snapshots, first_attempt_started_at_ms=START
    )
    assert outcome == Absent()


def test_the_failure_for_review_carries_no_text_of_its_own() -> None:
    snapshots = table(snap(3, 2, START + 10))
    with pytest.raises(Failure) as caught:
        lookup(KEY, ref="main", refs=refs(main=3), snapshots=snapshots, first_attempt_started_at_ms=START)
    assert caught.value.as_member() == {"code": "OUTCOME_UNKNOWN"}


def test_the_target_ref_is_looked_up_by_its_name_whatever_its_place_among_the_refs() -> None:
    snapshots = table(snap(1, None, START - 10 * MARGIN), snap(2, 1, START + 10, KEY), snap(3, 1, START + 20))
    outcome = lookup(
        KEY, ref="main", refs=refs(audit=3, main=2), snapshots=snapshots, first_attempt_started_at_ms=START
    )
    assert outcome == Found(snapshot_id=2)
    with pytest.raises(Failure):
        # the same key, on `audit` only, with `main` listed second
        lookup(KEY, ref="main", refs=refs(audit=2, main=3), snapshots=snapshots, first_attempt_started_at_ms=START)


def test_a_lookup_is_conclusive_by_the_window_when_the_history_before_it_was_trimmed() -> None:
    # Snapshot 1 expired. The oldest one left is already older than the first attempt's start minus the margin.
    snapshots = table(snap(2, 1, START - MARGIN - 1), snap(3, 2, START + 10))
    outcome = lookup(KEY, ref="main", refs=refs(main=3), snapshots=snapshots, first_attempt_started_at_ms=START)
    assert outcome == Absent()
