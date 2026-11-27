# ADR-0155: Trigger windows are identified by a random id, merged as sessions and started unfenced after their run; chain keys are plain digests that never leave the database

**Status:** Accepted (2026-11-27, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal with the fourth revision of design 042, accepted by the agent finishing design 042 under the
owner's delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-27-428. The owner may overrule it.
**Refines / supersedes:** supersedes ADR-0086 decisions 2 (keys as keyed hashes), 3 (the chain and key always locked),
6 (the window's identity) and 7 (the start holding the chain, key and window rows before the port), and, since the
approval reads of 2026-11-27, decision 4's clauses on elapsed deadlines and on `attempts = 0`, decision 10's caller as
starter, decision 11, and ADR-0086's Enforced-by line, Consequences and first revision of 2026-11-27; refines ADR-0153
decisions 1 (the statement that records the horizon) and 2 (the backlog rule and the cap); records deviations of
ADR-0050 decisions 6 and 7 and of ADR-0020 decision 3.
**Enforced by:** the fitness checks of Task 042-58 in `scripts/check-architecture.py` (the horizon statement reads
`now()`; no transaction holding a chain, usage or key row locks a `SEALED` window; no counter on `operator_chains`); the
slot tests of Tasks 042-13 and 042-22 in `tests/support/lock_order.rs`; the order-independence property tests of Task
042-15; the failpoint and two-replica tests of Task 042-27; the guarded-write test of Task 042-22. Until they exist:
review only.

## Context

The round-1 reviews of the third revision of design 042 (2026-11-27,
`docs/designs/042-stream-operators.reviews/2026-11-27/round-1/`) found that, as ADR-0086 and ADR-0153 wrote it:

- two dispatchers applying new keys to one chain deadlock, because both hold the chain row `FOR SHARE` and both update
  its counter;
- a session window that never extends an elapsed deadline lets the order in which events are applied, after a crash or
  between two replicas, split one burst into two runs;
- the pass could read "the deadline has passed" after taking its snapshot, so the horizon would not cover an admission
  that started just before the deadline;
- the backlog rule ignored events that had been attempted, and its cap equalled the dispatcher's claim lease, so a
  crashed apply's event could miss its window;
- the window identity used a key-row sequence that some chains do not have and that a purge resets;
- the start was to read a shard fence that the dispatcher, which starts windows sealed at once, does not hold, and to
  hold slot-3 rows before the port while also writing its window after the run;
- the keyed hash of keys protected nothing that a plain digest never shown does not, and its definer function was a
  security exception taken by a delegate.

## Decision

1. **Window identity.** A window is identified by a random `window_id` (a UUIDv7) drawn when it opens; the start's
   idempotency key is derived from it (and from the index after `flatMap`), and its fingerprint from the members'
   `(source, event_id)` in emit order. No key-row sequence and no key digest is part of it.
2. **Session windows merge.** An element joins every `OPEN` window of its key whose interval `[opened_at - gapMs,
   closes_at]` contains its `received_at`, whether or not that deadline has passed; windows it bridges merge into the
   earliest opened. Any change of a deadline clears the recorded horizon. Tumbling windows are aligned to the clock;
   count windows follow the order of application, the one order-dependent kind.
3. **Locks of the apply.** The chain row is taken `FOR SHARE` and never updated by an apply; a new key reserves a slot by
   a conditional update of the chain's own usage row; marks are bounded per key under the key lock the apply already
   holds; the held-bytes quota lock is slot 8b, last. A chain of only `filter`, `map` and `flatMap` takes no key row.
4. **The start.** The start is not fenced. It reads the window's state, held values and chain state in one statement;
   it calls the admission port, which takes its receipt (slot 2) and creates the run (slot 6); in the run's open
   transaction it updates its window `WHERE window_id = $1 AND state = 'SEALED'` and fails the transaction unless
   exactly one row changed. A replay sets `STARTED` in a short guarded transaction. A window of a removed trigger is
   refused `TRIGGER_REMOVED`.
5. **The horizon and the holds.** The statement that records the horizon compares the deadline with `now()`, the start
   of its transaction, never with `clock_timestamp()`. The backlog hold covers every `PENDING` inbox event of a source
   and type the chain matches, whatever its attempt count. The horizon wait and the backlog hold both end at
   `D + maxHoldMs`; `maxHoldMs` is 180 seconds by default and at least 90 (the claim lease of 60 seconds and a margin).
6. **Digests.** A key and a `distinct` value are stored only as a SHA-256 digest over the length-prefixed tenant, chain
   and canonical bytes of the value (as `start_key` builds its digest), never returned by an API, a log, a metric, a
   span, an audit record or an error; no key text is stored. No keyed hash, key table or definer function computes
   them. What links events by key (a window's key id and members, a delivery's window id, the reason `distinct`) is
   returned only under `reveal=pii` and audited; for a chain with a key, `distinct`, `skip` or `take`, so are its
   delivery outcomes, counts and windows, a delivery reading `WITHHELD` without the reveal grant. A database or backup
   reader can still recover a low-entropy value by trying values for as long as its row lives, and a publisher who sees
   the runs such a chain starts learns whether a value was seen: two stated residuals (design 042, Gaps 042-017 and
   042-018).

**Recorded deviation of ADR-0050 decision 7.** The start writes its window row, a slot-3 row for the apply and the seal,
after the run it creates (slot 6) and after 022's rows (slots 7 and 8). No cycle: a `SEALED` window is written only by
its starts and by the pass's short back-off, expiry and refusal transactions, which hold no other operator row; the
apply, the seal and a flush lock only `OPEN` windows, a discard `OPEN` or `REFUSED` ones, a retry `REFUSED` ones, so no
transaction that holds a slot-3 row locks a `SEALED` window, and duplicates of one start serialise first on the slot-2
receipt.

**Recorded deviation of ADR-0050 decision 6 and ADR-0020 decision 3 (2026-11-27, approval reads).** ADR-0050 decision
6 requires a pass to read the shard fence of the aggregate it acts on, and ADR-0020 decision 3 fences every action of
the scheduler. The start of a window, run by the pass or by the dispatcher, reads no fence: it is made safe by the
admission port's slot-2 receipt and by its guarded write of the window (decision 4), which ADR-0020 itself names as
what makes results correct, the fence being only a way to keep replicas from doing the same work. The seal stays
fenced.

**Why held bytes have a counter of their own (2026-11-27, approval reads).** A held-bytes column on the existing
content delta path was rejected: that path enforces one tenant ceiling at commit, while held bytes need a ceiling of
their own and the final refusal of design 042's FR-042-021 inside the apply's transaction, before commit. The held
flush runs the content flush first (design 042, Task 042-64), so the two usage rows are always taken in one order.

## Considered options

1. **This record** — no deadlock, one run per burst whatever the order of the applies, a start that a dispatcher and a
   pass can both run, and no security exception.
2. **Counters on the chain row taken `FOR UPDATE`** — no deadlock, but every element of a chain waits for the one before.
3. **A fenced start** — needs a shard the dispatcher does not hold, and a change to the port to carry the fence.
4. **Keeping the keyed hash** — a key table, an extension and a definer function the owner would have to accept as a
   security exception, for no protection the digest lacks.

## Consequences

New keys of one chain are serial on its usage row; applies that add held bytes are serial per tenant on the quota lock;
both are measured by Task 042-33. A long transaction delays a window by at most `maxHoldMs` past its deadline, then the
window seals and a later element is late, never lost. A window is purged seven days after it becomes final.

## Evidence

Design 042, Requirements FR-042-014 to FR-042-024 and FR-042-031 to FR-042-034, sections 4 to 6 and 10, fourth
revision (`docs/designs/042-stream-operators.md`); research Decisions 042-168 to 042-179.
