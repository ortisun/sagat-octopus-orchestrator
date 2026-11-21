"""The commit the host makes: once (feature 035: T035-034, D035-030 step 5, FR-035-018, G035-020).

The host searches the history of the table for the effect key before it commits, and again before it tries a commit
a second time. PyIceberg 0.12 would defeat that search: when the catalog refuses a commit because the ref moved, it
refreshes the table, rebuilds the snapshot on top of the new head and commits again, up to four times, without the host
having looked at the new history. How many times is read from ``commit.retry.num-retries`` among the properties of the
table the catalog returned, so the owner of the table chooses it, not the host.

The host therefore sets that property to ``0`` on the table object it loaded, in memory, before it appends. It is never
written to the table. A refused commit then raises ``CommitFailedException`` and the host searches again.
``tests/test_commit.py`` counts the commit requests of a REST catalog for every answer to prove it.

One repeat remains that this cannot remove, and it is safe: PyIceberg's REST client asks again once after a 401, with a
refreshed token, and the catalog applied nothing to a request it did not authorise.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from pyiceberg.table import Table

COMMIT_RETRIES = "commit.retry.num-retries"


def pin_commit_once(table: Table) -> None:
    """Makes PyIceberg send the commit of ``table`` once, whatever the table's own properties say.

    Only the object in memory changes. The properties the table keeps in the catalog are not touched, and the others
    of the loaded table are kept.
    """
    table.metadata = table.metadata.model_copy(
        update={"properties": {**table.metadata.properties, COMMIT_RETRIES: "0"}}
    )
