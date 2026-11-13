# ADR-0086: Operator chains on triggers are declared data, their state is rows on the database clock, and a fenced pass seals a window that starts one run by its identity

**Status:** Accepted (2026-11-13, OD-2026-11-13-17: the owner's "Aceito tudo"); revised the same day after review, before any code (see Revision)
**Decided by:** owner, on the agent's proposal (OD-2026-11-13-17, accepting the text of commit df83eed9); the revision of 2026-11-13 is the agent's, answering the architecture, security and correctness reviews, and is put to the owner with the design (T042-44 before S2)
**Refines / supersedes:** refines ADR-0012 (events pass through a durable inbox and a durable delivery table), ADR-0020 (sharded, fenced scheduler) and ADR-0032 (additive IR v1, one capability at a time); composes with ADR-0056 and ADR-0088 (one admission port; rate limits). Supersedes none.
**Enforced by:** proposed checks of design 042, slices S2 to S4: the fault, race, restart and capacity tests T042-27 to T042-32, the zero-cost budget test T042-19, the HMAC test T042-53 and the composition test with feature 022, T042-3. None exists yet, and none is in release 0.3.0; until they exist, this rule is review only.

## Context

An admitted event starts one run per matching trigger (feature 004). A provider's burst is as many runs; deduplication
is by event id only; nothing is ordered (G74). The owner asked for reactive-stream operators and decided they apply to
triggers as well as collections, in a release after 0.3.0 (design 042, O042-001 and O042-002).

The forces:

- **A deadline fires with no event to carry it.** Something has to look at the clock, durably.
- **Several clocks exist.** The platform decides time on the database clock inside the deciding transaction. An event's
  `received_at` is `clock_timestamp()` at insert, but the event becomes visible only when its transaction commits, later.
- **Dispatch is at least once and unordered.** Two replicas may dispatch one admission; a slow dispatcher may still
  hold events received before a deadline; claiming an event does not increase its `attempts`.
- **A start must be idempotent**, and feature 022 owns when a run may start (ADR-0056, ADR-0088), with its own lock
  order (ADR-0088, decision 4).
- **Held values are personal data** (feature 016) and must be deletable; the `payloads` table is append-only.
- **A key is often short** (an order number); a plain digest of it can be reversed by trying values.
- **The dispatcher reads trigger rows without the IR.**

Unknown: the rate one hot key sustains and the cost of the backlog check per tick (T042-33).

## Decision

1. **Declared data.** A trigger MAY declare `key` and `operators`: up to eight stages from the closed set of design 042
   in its canonical order, every predicate, key and projection an expression of the existing bounded language,
   evaluated by the engine. User code MUST NOT run in a chain. The fields are additive in IR v1; the deployment switch
   `trigger-operators` is checked by the definition service at publication and never makes a stored definition
   unreadable.
2. **State is rows; values are deletable; keys are keyed hashes.** Everything a chain remembers MUST be PostgreSQL rows
   keyed by tenant, namespace, chain and HMAC-SHA-256 of the key under a per-tenant key that never leaves the database.
   `distinct` values are stored the same way. Element values MUST be held in a classified, quota-counted column of the
   chain's own table, never in `payloads`, and MUST be deleted in the transaction that starts or discards the window.
   The API names keys and windows by random surrogate ids, never by a digest. Process memory, the broker and a
   replica's clock MUST NOT decide or remember anything.
3. **One application per admission, recorded once.** An event MUST be applied in one transaction that locks the chain
   row, then the key row when the chain has a keyed or windowed stage (a chain of only stateless stages takes none),
   applies the stages, and inserts the `event_deliveries` row with its final outcome; a conflict on that insert rolls
   the transaction back. A delivery row is never updated and is deleted with its inbox row.
4. **Arrival time on the database clock, with a grace and a cap.** An element's time is its `received_at`; `occurred_at`
   MUST NOT drive a timer. A key MAY have several open windows; an element joins the one covering its `received_at`, and
   an elapsed deadline is never extended. A window with deadline `D` MUST NOT be sealed before `D + admissionGraceMs`
   (at least the admission statement timeout), nor, until `D + maxHoldMs`, while the tenant and namespace hold a
   pending event with `attempts = 0` the trigger could match that was received at or before `D`. `emit: first`, `last`
   and `all` follow the order `(received_at, source, event_id)`. A late element joins an open window or opens one,
   flagged; it MUST NOT be dropped for being late.
5. **A fenced pass seals.** Windows are sealed by a scheduler pass on the shards its replica holds, reading the shard
   row as ADR-0020 prescribes and locking chain, key and window rows in that order. The first commit wins and the loser
   re-reads. A sealed window's membership MUST NOT change.
6. **A sealed window starts one run by its identity.** The idempotency key is derived from the window's recorded identity
   (namespace, chain, key HMAC, the instant it opened, its sequence) with a fingerprint of its membership; the run
   starts on the version whose chain gathered the window.
7. **Through the one admission port, chain rows first.** The start MUST use the port every run creator uses, holding
   the chain, key and window rows before the port takes a semaphore or a rate row, and MUST NOT touch a chain row
   afterwards. When the port creates the run, buffered or not, the window is `STARTED`; a full buffer leaves it
   `SEALED` for a retry under the same key.
8. **No operator paces a start.** That is `concurrency` and `rateLimit` of feature 022.
9. **Every ceiling has one outcome, and none holds the backlog.** A full window seals or drops the newest, as declared.
   A new key beyond `maxKeys`, or a mark beyond `maxMarks`, ends that event's delivery to that trigger at once with
   `FAILED`/`KEY_LIMIT`, audited once per chain and minute with a count, retryable by an operator; it MUST NOT leave the
   event pending. Marks do not hold a key slot. Held bytes are bounded per tenant by a quota of their own.
10. **Manual acts are run acts.** Flushing or retrying a window needs `run:start` and `run:operate`; discarding needs
    `run:operate`; the caller is recorded as the starter of the run. Every such act is audited.
11. **The chain lives on the trigger row, and the fleet is upgraded first.** `trigger-operators` MUST NOT be enabled
    while a scheduler replica predates it; such a replica would over-deliver, never lose.

## Considered options

1. **Rows, arrival time with a grace and a cap, a fenced sealing pass, identity-derived start** — the only option in
   which a restart, a second replica, a duplicate dispatch and commit latency change nothing.
2. **State in memory with a broker timer** — lost on restart, wrong with two replicas.
3. **Seal when the next event arrives** — the last burst of a key never starts.
4. **Seal on the clock alone** — a burst is split by dispatcher lag or by a commit that lands after the deadline.
5. **An uncapped backlog rule** (the accepted first text) — an event claimed and never attempted holds windows for ever.
6. **Held values in `payloads`** (the accepted first text) — append-only; they could never be deleted.
7. **Plain digests shown in the API** (the accepted first text) — reversible for short keys.
8. **`QUOTA_EXCEEDED` with the dispatcher's retry at `maxKeys`** (the accepted first text) — keeps the event pending,
   which holds every window of the namespace behind the backlog rule.
9. **Windows on `occurred_at` with a watermark** — deferred by the owner (O042-004).
10. **A stream resource outside the workflow version** — a second, unpinned place where behaviour is defined.

## Consequences

- Good: "one run per burst per key" survives duplicate delivery, a crash, a restart and commit latency; every held event
  is a row an operator can list, flush or discard; held values leave when they are used.
- Bad: five tables, a pass and a per-tenant HMAC key to operate; one hot key is serial; sealing waits at least the
  admission grace after a deadline; a stuck dispatcher delays sealing by up to `maxHoldMs`.
- Compatibility: triggers without a chain cost and behave as today. A mixed fleet is unsupported for chained triggers.
- Rollback: switch publication off; stored definitions stay readable, the pass runs until open windows drain.
- Feature 022: its D022-018 says a buffered window "stays sealed"; this ADR says `STARTED`; the 022 text is to be
  aligned (design 042, G042-011).

## Revision (2026-11-13, after review, before any code)

The owner accepted the text of commit df83eed9. The architecture, security and correctness reviews of commit 540154c8
found that held values were meant for the append-only `payloads` table, digests of short keys were to be shown, a key
ceiling would hold events pending for a day and with them every window of the namespace, the backlog rule had no cap
and was not scoped, a commit landing after a deadline could split a burst, `emit: last` depended on commit order, the
delivery row was written twice, and window management sat under a configuration permission. Decisions 2, 3, 4, 7 and 9
were rewritten and decision 10 added. The chosen option is unchanged. The refinements of O042-008 and O042-009 and the
new numbers (`maxMarks`, the held-bytes quota, `maxHoldMs`, `admissionGraceMs`) are put to the owner before S2 (T042-44).

## Evidence

None yet. Design: `docs/designs/042-stream-operators.md` (FR-042-013 to FR-042-024, FR-042-029, FR-042-030, SC-042-001,
SC-042-002, SC-042-006, SC-042-007, Technical Plan sections 4 to 8); registers: `specs/042-stream-operators/research.md`
(D042-003 to D042-009, D042-011, D042-014, D042-029 to D042-037; G042-002, G042-005 to G042-007, G042-011). Feature
022: `specs/022-flow-control-step-policies/spec.md` (US07, FR-022-041, FR-022-042), its D022-018, and ADR-0088.
