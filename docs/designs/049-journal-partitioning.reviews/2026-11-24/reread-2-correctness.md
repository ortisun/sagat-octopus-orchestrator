# Second correctness re-read of design 049 (2026-11-24)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: c095c5c5e36495cbe5a43ae3933c4220c7ef4718
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The third read of
correctness, of the same text as `reread-architecture.md`; Blocker 1, Major 5 and Nits 4, 5, 6, 10 and 11 were answered
in `4e3650fe` before this report arrived, the others after it.

## Blocker

1. The reference triggers do not lock the row they check, so a concurrent purge can delete a payload a live run still
   references (`ON CONFLICT DO NOTHING` reuse of a shared legacy digest). Fix: `FOR KEY SHARE` in the trigger; the purge
   locks candidates, re-checks in a fresh statement, deletes; a two-session test.

## Majors

1. Step 5 fails: the parent's `run_events_instance_idx` collides with the same name on the renamed table.
2. Many payload read sites are not in the plan (`payload_disclosure.rs`, `payload_inspection.rs`, `maintenance.rs`,
   `execution/path_read.rs`, `execution/resets.rs`, `scope_frontier.rs`, `ad_hoc.rs`, `database_results.rs`,
   `collection_outputs.rs`, `primitives.rs`), and a dated run read through a `LEFT JOIN payloads` silently gets no
   content. Fix: one lookup that branches on the bucket; the static check refuses `payloads` filtered by a run.
3. "23514 with no constraint name" also matches PL/pgSQL guards that raise 23514 (`execution_chain_identity_guard` and
   others). Fix: also require the error's table to be a partitioned journal table; give the switch trigger its own code.
4. Run creations in flight at the switch can be refused. Fix: one switch row read `FOR SHARE` by creators and updated by
   the switch; a barrier test straddling the switch.
5. A run-owned row can carry the legacy default while its run is dated, and nothing refuses it.
6. "A purged run refuses any later append" has no database guard, and the check after the detach cannot see such an
   append. Fix: the bucket trigger refuses rows of a run with a stub; the check also compares the archived last sequence.
7. Retiring the legacy bucket leaves every legacy payload in `payloads`, charged forever.
8. A retirement stopped after step 2 leaves runs stubbed with their content in place and charged; erasure must never use
   the mark-only form.
9. A per-run purge during a pending detach deletes nothing and reports success.
10. Resuming after a crash between a detach and its record fails; derive each table's state from the catalog.
11. Migration step 1 can hold several hot-table locks at once, against the hot path's order.
12. Dropping a month conflicts with decision D298 (certificates and their sources kept without expiry) and the
    unconditional delete guards; the two certificate digests may not be payload digests.
13. The order of the bucket lock and the run lock is unspecified; retirement and a per-run purge can deadlock.

## Nits

1. The months-ahead alert fires on the first of every month; create three ahead or alert below one.
2. Only the statement-level truncate trigger is created per partition; duplicated row triggers would double-charge.
3. The bucket and switch triggers must be enabled always.
4. Architecture decision 0132's order of retirement, capability name, design number and column count differ from the
   design.
5. The count of 11 referencing tables does not add up.
6. The budget hook misses deferred triggers.
7. "Step 5 builds no index" needs an oracle (index identity before and after).
8. The 120 s bound assumes every older transaction stops within 60 s; migration steps must lift their timeouts.
9. One pending detach per parent: take a global retirement lock.
10. Partial indexes lose heap-only updates for every updated indexed column.
11. Release usage rows in tenant order and sum `octet_length(content)` as the delta trigger does.

Counts: Blocker 1, Major 13, Nit 11
