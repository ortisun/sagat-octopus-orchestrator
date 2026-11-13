# ADR-0086: Operator chains on triggers are declared data, their state is rows on the database clock, and a fenced pass seals a window that starts one run by its identity

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-11-13; the owner asked for the operators in OD-2026-11-13-13 and put the trigger operators in a release after 0.3.0 in OD-2026-11-13-15, and has not accepted the mechanism)
**Refines / supersedes:** refines ADR-0012 (events pass through a durable inbox and a durable delivery table), ADR-0020 (sharded, fenced scheduler) and ADR-0032 (additive IR v1, one capability at a time); composes with ADR-0056 and the proposed ADR-0088 (one admission port; rate limits). Supersedes none.
**Enforced by:** proposed checks of design 042, slices S2 to S4: the fault, race, restart and capacity tests T042-27 to T042-32, the zero-cost budget test T042-19, and the composition test with feature 022, T042-3. None exists yet, and none is in release 0.3.0; until they exist, this rule is review only.

## Context

An admitted event starts one run per matching trigger (feature 004). A provider's burst is as many runs; deduplication
is by event id only; nothing is ordered (G74). The owner asked for reactive-stream operators (debounce, window, filter,
distinct, skip and others) and decided they apply to triggers as well as collections, in a release after 0.3.0
(design 042, O042-001 and O042-002).

The forces:

- **A deadline fires with no event to carry it.** A debounce ends when nothing arrives. Something has to look at the
  clock, durably, on whichever replica is alive.
- **Several clocks exist.** Each replica has one, a provider has one (`occurred_at`, optional), PostgreSQL has one. The
  platform already decides time on the database clock inside the deciding transaction.
- **Dispatch is at least once and unordered.** Two replicas may dispatch one admission; a slow dispatcher may still
  hold events received before a deadline.
- **A start must be idempotent.** The run that a burst starts must be one run after a crash between any two writes.
- **The dispatcher reads trigger rows without the IR.** A binary that predates a chain would act on the row as a plain
  trigger.
- **Feature 022 owns when a run may start** (ADR-0056, ADR-0088) and forbids locking a run after its rate rows.

Unknown: the rate one hot key sustains and the cost of the backlog check per tick (T042-33).

## Decision

1. **Declared data.** A trigger MAY declare `key` and `operators`: up to eight stages from the closed set of design 042
   in its canonical order, every predicate, key and projection an expression of the existing bounded language,
   evaluated by the engine. User code MUST NOT run in a chain. The fields are additive in IR v1 behind the capability
   `trigger-operators`.
2. **State is rows.** Everything a chain remembers (keys and their counters, `distinct` marks, windows and their
   members) MUST be PostgreSQL rows keyed by tenant, namespace, chain and key digest. Process memory, the broker and a
   replica's clock MUST NOT decide, remember or shorten anything. Element values are held as classified content and
   released when the window's run exists.
3. **One application per admission.** An event MUST be applied to a chain in the transaction that inserts its
   `event_deliveries` row for that trigger, under a lock on the chain's key row. The order of those commits is the
   order of the key's elements and MUST be recorded, never recomputed.
4. **Arrival time on the database clock.** An element's time is its `received_at`. `occurred_at` MUST NOT drive a
   timer. A window MUST NOT be sealed while the namespace's inbox holds a never-attempted pending event the trigger
   could match that was received at or before the window's deadline. An element applied after its window was sealed is
   late: it joins the next window, flagged and counted, and MUST NOT be dropped for being late.
5. **A fenced pass seals.** Windows are sealed by a scheduler pass on the shards its replica holds, in a transaction
   that reads the shard row as ADR-0020 prescribes and locks the key row. Sealing and applying are ordered by that
   lock; the first commit wins and the loser re-reads. A sealed window's membership MUST NOT change.
6. **A sealed window starts one run by its identity.** The idempotency scope is the trigger's; the key is derived from
   the window's recorded identity (namespace, chain, key digest, the instant it opened, its sequence) with a
   fingerprint of its membership. The run starts on the version whose chain gathered the window. A chain without a
   window keeps today's start key.
7. **Through the one admission port, chain rows first.** The start MUST use the admission port every run creator uses.
   The transaction locks the chain's key and window rows before the port takes a semaphore or a rate row, and MUST NOT
   touch a chain row afterwards (ADR-0088, decision 4). A run the port buffers exists, so the window is started; a full
   buffer leaves the window sealed for a retry under the same key.
8. **No operator paces a start.** A chain decides which events ask for a run and with what input. It MUST NOT delay,
   rate or limit an admitted run; that is `concurrency` and `rateLimit` of feature 022.
9. **Every ceiling has one outcome.** A full window seals or drops the newest, as declared. A new key beyond the
   chain's ceiling fails that event's delivery, which is retried and then dead-lettered. Nothing passes a chain
   unoperated because a ceiling was reached.
10. **The chain lives on the trigger row, and the fleet is upgraded first.** `trigger-operators` MUST NOT be enabled
    while a scheduler replica predates it. A replica that does would start a run per event: it over-delivers and loses
    nothing, which is why the chain is not kept where an old binary would not look.

## Considered options

1. **Rows, arrival time, a fenced sealing pass, identity-derived start** — the only option in which a restart, a
   second replica and a duplicate dispatch change nothing.
2. **State in the dispatcher's memory, with a broker timer** — lost on restart, wrong with two replicas; the broker is a
   hint (principle VII).
3. **Seal when the next event arrives** — no pass, and the last burst of a key never starts.
4. **Seal on the clock alone, without the backlog rule** — simpler, and the result depends on dispatcher lag: a burst
   is split when the dispatcher is slow.
5. **Windows on `occurred_at` with a watermark** — correct for out-of-order sources, and it needs a watermark per
   source and a policy for a provider with a wrong clock. Deferred by the owner (O042-004).
6. **Debounce as a policy of run admission** ("a new start withdraws the queued one") — covers "last wins" only, and
   makes a start request something that can be withdrawn, which feature 022's buffer does not allow.
7. **A stream resource of its own, outside the workflow version** — one chain could feed two workflows, and a
   workflow's behaviour would be defined in a second place that is not pinned.

## Consequences

- Good: "one run per burst per key" is a declaration that survives duplicate delivery, a crash and a restart; every
  held event is a row an operator can list, flush or discard.
- Bad: five tables and a pass to operate; one hot key is serial (a row lock per element); the dispatcher's transaction
  for a chained trigger does more; a slow dispatcher delays sealing instead of splitting bursts.
- Compatibility: triggers without a chain cost and behave as today, held by a budget test. A mixed fleet is
  unsupported for chained triggers until the instance registry can enforce decision 10 (G042-006).
- Operational cost: a lag metric and alert for the pass; purge of keys, marks and finished windows; held personal data
  counted in retained bytes.
- Rollback: disable the capability; publication refuses new chains, the pass keeps running until open windows drain,
  and no state is erased.
- Harder later: event-time windows, sliding windows and operators on `waitEvent` each need more state under the same
  rules; none is closed off.

## Evidence

None yet. Design: `docs/designs/042-stream-operators.md` (FR-042-006 to FR-042-018, FR-042-022, SC-042-001 to
SC-042-005, Technical Plan sections 4 to 8); registers: `specs/042-stream-operators/research.md` (D042-003 to D042-009,
D042-011, D042-014; G042-002, G042-005 to G042-007). Feature 022: `specs/022-flow-control-step-policies/spec.md`
(US07, FR-022-041, FR-022-042) and ADR-0088.
