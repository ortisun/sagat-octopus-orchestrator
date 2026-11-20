# Scoped event dispatcher checkpoint

Refs: T014-014–017/040, FR-014-004/005/014/015/017.

The existing dispatcher handles root waits with the legacy root path and scoped waits with
begin_current_path. A scoped event needs no external evaluation or scheduler clock decision:
the current bounded path is read under its own Run/chain lock, then the existing native receipt
port validates current pinned identity, subscription, current inbox admission and stop fences.
It returns the canonical document; no evaluator-provided document becomes the scoped value.
The exact pure satisfaction and one output, audit, journal, quota and receipt commit together.
Use the existing run-wide logical output counter to enforce MAX_RUN_OUTPUT_BYTES without loading
historical values. Quota/persistence/commit refusal rolls back the complete receipt transaction.
Pause permits the outcome and defers routing, as in the accepted native command.

Candidate pages include root and scoped paths using the existing complete database tuple.
Lookback uses the same exact receipt consumer identity as native delivery. A lookback candidate
must still match its complete current physical/logical identity and subscription binding;
a scan marker uses those same equality guards. Discovery/markers grant no outcome authority.
Legacy root consumers keep wait:{run}/{node}; scoped consumers use wait-instance:{run}/{step UUID}.
One failed consumer cannot suppress the remaining matching candidates. Failed lookback keeps
its scan pending. Cold duplicates find the receipt/outcome and retain no second value.

No public wait-body guard or worker capability changes; no new runtime grants.
The tests use internal immutable structured definitions after real worker → scheduler start.
Public entry admission, classified/public readers and actual released-v0.2 cutover remain
separate acceptance gates; no canonical task or general nested runtime is declared complete.


## Bounded lookback fairness and recovery

Unscanned discovery uses the complete global SQL tuple (tenant, run, path nodes, path indices,
node) and a strict-after cursor. One dispatcher retains only its last fetched candidate in a
short mutex, never across await; it advances before processing the page, even when a consumer
fails. Process at most scan_batch rows; an empty cursor page wraps once to the first page.
Restart resets fairness position, while all pending work remains in PostgreSQL. This is no
outcome authority or claim. Concurrent ticks may repeat discovery; native transaction identity
still decides the outcome. No fairness under infinite process crashes is promised.

A lookback scans only after a committed outcome or a query finding no matching admission.
Some(event) followed by native No/Refused remains pending: the selected earliest admission may
have been removed while entry waited. Do not hide the next retained matching admission.
After native receipt validation under the Run lock, propagate all pure settlement errors;
there is no remaining timeout/cancel race that could justify finalizing the event on such an
error. Failure keeps its lookback pending and direct ingress follows existing event retries.

The original oldest-first scan order was an adapter discovery detail, not a product ordering
guarantee. The cursor is process-local and only bounds starvation during ongoing ticks; it does
not store the sole durable work record. Global tenant identity is explicit in ORDER BY and
strict-after even though the current runs.id primary key is already globally unique.


## Index and deployment impact

Migration20261223000000 adds only steps_event_unscanned_path_idx with the same pending-event
predicate and complete tuple as keyset discovery. No columns, functions, privileges or retained
values change; the old updated_at scan index remains for previous readers. Rollback drops only
the new index. A normal CREATE INDEX takes its PostgreSQL table lock during the build and can
block concurrent writes; schedule this additive migration in the normal controlled migration
window on large steps tables. Its duration is not estimated from the tiny integration fixture.
The focused test rolls back/reapplies this one index with real admitted waits and proves exact
retained metadata, query results and replay survive. This is not a released-v0.2 upgrade test.
