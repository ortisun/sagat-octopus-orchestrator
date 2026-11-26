# Security review of design 045, round 3 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 7675358f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author; every finding and its severity kept, with the author's answer (applied in the fourth revision).

Counts: Blocker 0, Major 6, Nit 8

Round 2: Majors 4 and 5 and Nits 1 to 5 closed; Majors 1 and 2 partly closed; Major 3 and Nit 6 not closed.

## Major

1. **The hold does not check the process kinds**: a requirement recorded for control planes only would let level-1 and level-2
   versions publish while old engines serve. *Answer: accepted*: the hold returns true only when the requirement is recorded for both
   kinds, as `schedule_trigger_handover_complete` checks; a test with one kind.
2. **A separate port method cannot hold a lock until the publication commits** (`PgDefinitionStore::publish` opens and commits its own
   transaction). *Answer: accepted*: the store's publication takes the capabilities the level needs and calls the hold inside its own
   transaction, just before the commit; a refusal is a typed store error; a barrier test with a removal between the insert and the
   commit.
3. **The lock order and the floor check do not match the operator adapter** (`remove` takes the epoch, then `verify`, then the
   function). *Answer: accepted, with a simpler mechanism*: the hold reads the requirement's own row `FOR SHARE`, as
   `lease_withdrawal_required()` of Architecture decision 0098's build does; the floor is a rule of that build's replay-floor trigger,
   which runs under the removal's row lock and so sees a publication that held the row; no advisory lock and no replaced operator
   function.
4. **Republishing an older document is a silent no-op** (the digest reuses the level-0 row). *Answer: accepted*: when the stored row's
   level is below the computed one, publication refuses with `DOCUMENT_NEEDS_REPUBLISH`; the inventory tells the operator to edit the
   document; a test republishes the same bytes.
5. **How each loader gets the stored level is not designed.** *Answer: accepted*: the store returns the document with its level, and
   `parse_stored` and `DefinitionCache::get_or_parse` take the level; at level 0 the codec drops the keys without validating them; a
   test through every loader.
6. **Script nodes accept fields the sandbox runner cannot honour** (it registers protocol 1). *Answer: accepted*: `heartbeatFrom:
   "handler"` and a `heartbeatTimeoutMs` below 30,000 ms are refused on `script` and `bodyScript`; `scheduleToStartMs` is required for
   every level-2 field.

## Nit (each accepted)

1. The server's bound is two beats per second per lease (the 500 ms clamp), stated.
2. The paused flood joins Task 045-17; the pass does not filter paused runs (their start deadlines are null and their close cutoffs are
   acted on).
3. No advisory key any more (Major 3).
4. No operator function is replaced any more (Major 3).
5. A `SCHEDULE_TO_CLOSE_TIMEOUT` caused by a worker's delay is counted under the retry decisions only.
6. At level 0 the worker's delay is ignored and left out of the fingerprint.
7. `SECRET_IN_OUTPUT` joins the reserved codes.
8. The ad hoc insert of `workflow_versions` writes level 0; private documents cannot carry the fields.
