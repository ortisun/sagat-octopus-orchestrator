"""The conformance runner's ``awaitJournal`` reads the whole journal of a run: an event on a later page than the first is
found, and each page is asked for with the cursor the page before it gave."""

from __future__ import annotations

from collections.abc import Awaitable, Callable

from conformance.runner import journal_has
from sagat_octopus_orchestrator import JournalEvent, JournalPage


def two_pages() -> tuple[Callable[..., Awaitable[JournalPage]], list[str | None]]:
    asked: list[str | None] = []

    async def read(*, cursor: str | None) -> JournalPage:
        asked.append(cursor)
        if cursor == "second":
            started = {"sequence": 2, "eventType": "step.started", "stepId": "held", "attempt": 1}
            return JournalPage(items=(JournalEvent(started),), next_cursor=None)
        leased = {"sequence": 1, "eventType": "step.leased", "stepId": "held", "attempt": 1}
        return JournalPage(items=(JournalEvent(leased),), next_cursor="second")

    return read, asked


async def test_await_journal_finds_an_event_on_the_second_page_asked_for_with_the_cursor_of_the_first() -> None:
    read, asked = two_pages()
    assert await journal_has(read, "step.started", "held")
    assert asked == [None, "second"]


async def test_await_journal_reads_every_page_before_it_says_the_journal_has_no_such_event() -> None:
    read, asked = two_pages()
    assert not await journal_has(read, "step.completed", "held")
    assert asked == [None, "second"]
