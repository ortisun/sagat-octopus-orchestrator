# Vacate and Acquire for ChildRoot capacity members (B1d.2 / B2) — proposal D706–D713

Status: **proposal, no code, awaiting bounded adversarial review.** Refs: T014-014..017/040/042,
ADR-0030 §§13–19, ADR-0037, A0/A1 (`capacity.rs`), B0 lock inventory, B1a entry decision,
`capacity-eligibility.md` D703–D705, `capacity-discovery.md` (B1d.1, read-only, another branch).
Ids: D706–D713, G714–G719, O705 (range D701–D799 / G701–G799 / O701–O799; highest used before: D705, G713, O704).

The pure transitions `CapacityGroup::vacate` and `acquire` exist and are tested. No transactional
adapter uses them. This document specifies the two native commands that do, for **ChildRoot members
only**, as one dormant slice with no production caller. Possession is not a permit: neither command
leases, releases, cancels or changes a run, a step or a chain.

## Decisions

- **D706 — Scope.** `member_kind='CHILD_ROOT'` only, matching B1d.1 discovery. `SCOPED_PATH` members have no
  chain; their authority comes from the original run and path steps, so they need their own slice (G716).
  Vacate and Acquire share one port module, one receipt table and one entry prefix; they differ only in the
  gate in D708/D709.
- **D707 — Entry and lock order are the Finish order (B1a).** Read the member (discovery only) → exact chain
  `FOR UPDATE` → exact head run `FOR UPDATE` → group header `FOR UPDATE NOWAIT` → re-read the member under the
  group → scheduler fence and database clock (`locked_scheduler_time`, sampled after the shard guard) →
  receipt lookup → CAS → gate → A1 transition → writes. One group, one run, never a second existing run;
  `55P03` is a distinct `Busy` that consumes the whole transaction; retry starts a fresh transaction.
- **D708 — Vacate gate (quiescence), structural.** The selected member is the holder (domain `NotHolder`
  otherwise) and is ACTIVE. Its chain's current head equals the observed head and is not final (a final
  head belongs to Finish, which already vacates). **No step of the head run is `LEASED` or `RUNNING`**, whether
  or not its lease has expired: expiry is reaped by the scheduler and this gate reads no clock. `READY`,
  `PENDING`, `WAITING` and `RETRY_WAIT` steps may remain (A0: "ready but gated work and durable waits may
  remain"). The probe is an indexed existence check under the already held run lock; every execution writer
  enters through chain then run, so the read is consistent without row locks on steps (the runtime role
  cannot take them).
- **D709 — Acquire gate (eligibility).** Group `VACANT` at the expected epoch; member ACTIVE; same head and
  not-final conditions. Head run status is movable: `Running`, `Waiting`, `Cancelling` or `Compensating`.
  `Paused` and `ManualReview` cannot move, so they are not eligible; terminal statuses go to Finish. And
  there is schedulable work: the run is `Cancelling`/`Compensating` (the scheduler still owes it a
  terminal transition), or a step is `READY`/`PENDING`, or a `RETRY_WAIT` or timer/event wake is due at the
  database time sampled under the shard guard. The domain match over `RunStatus` is exhaustive, with no
  wildcard arm. Eligibility grants no lease and picks no worker; the worker's own claim transaction must still
  check possession (separate slice, G717).
- **D710 — Writes are the header, the audit, the outbox and the receipt, and nothing else.** One UPDATE of
  `capacity_groups` (revision, epoch, possession, holder) under the held row and CAS on revision and epoch. The
  member row is not written (membership is unchanged by both commands), so no member trigger or reverse-FK
  update path is exercised beyond the deferred holder FK. No write to `runs`, `steps`, `execution_chains`, the
  journal or any quota counter. This is asserted by the tests, not only stated.
- **D711 — Receipt.** One append-only table `capacity_group_transfer_receipts` keyed
  `(tenant_id, group_id, operation_key)`, with `kind IN ('VACATE','ACQUIRE')`, the 32-byte fingerprint, the
  member/chain/head binding, the four expected counters, the resulting revision/epoch/possession/holder, time,
  audit id and outbox id. `UNIQUE(tenant_id, group_id, resulting_revision)` and CHECKs force
  `resulting_revision = expected_group_revision + 1` and `resulting_epoch = expected_group_epoch + 1`
  (both commands change possession) and tie possession to kind (VACATE → `VACANT` with no holder; ACQUIRE →
  `HELD` by the member). The fingerprint is domain-separated (`sagat.capacity.transfer.child-root.v1`, kind
  byte, fixed-width ids and big-endian counters; renewable scheduler authority excluded) and has an
  independently generated test vector. **The receipt is looked up before the gate**: a replay of an old Vacate
  must return its original result even after the member re-acquired and leased a step. The binding, scheduler
  fence and fingerprint conflict checks still run first.
- **D712 — Typed errors, telemetry, audit, events.** Closed errors: `Busy`, `Missing`, `BindingMismatch`,
  `StaleHead`, `StaleGroupRevision`, `StaleGroupEpoch`, `StaleChainRevision`, `StaleRunRevision`,
  `SchedulerFenceLost`, `NotHolder`, `Occupied`, `NotQuiescent`, `NotEligible`, `InactiveMember`, `ChainFinal`,
  `ContradictoryProjection`, `FingerprintConflict`, `CounterExhausted`, `Storage`,
  `CommitOutcomeUnknown { operation_key }`. One span `CAPACITY_TRANSFER_SPAN` with a closed `kind`
  (`vacate`|`acquire`) and the Finish outcome vocabulary; ids only, no payload or credential. New audit actions
  `CapacityMemberVacated` and `CapacityMemberAcquired` (every exhaustive table decided), new outbox hints
  `capacity.member.vacated` and `capacity.member.acquired` in the closed AsyncAPI vocabulary, new
  columns classified in `storage-classification.json`.
- **D713 — Commit uncertainty is the Finish contract.** Only server-confirmed constraint, serialization and
  deadlock aborts imply rollback; anything else after COMMIT dispatch is `CommitOutcomeUnknown` and is
  reconciled by re-sending the retained original command to the receipt. The caller must keep the command.

## Same-delivery footprint (taken from what Finish touched)

`crates/domain` (new `execution/capacity_transfer.rs`: the two pure gates; `audit.rs` and its test),
`crates/application/src/capacity_transfer.rs` (port, command, fingerprint, errors),
`crates/infra-postgres` (adapter, migration with grants and a down that refuses retained receipts,
`storage-classification.json`), `crates/telemetry/src/capacity.rs`, the AsyncAPI contract and baseline,
`runtime_privileges.rs`, `docs/traceability.md`, `validation-report.md`, and
`tests/integration/workflow-primitives/capacity_transfer*.rs`. It touches none of the files the other
lane owns (`execution/{mod,state}.rs`, `infra-postgres/src/execution*`, the main integration `main.rs`).

## Required evidence (each seen failing for the right reason first, runtime role and owner role)

1. Pure gates: table over every `RunStatus` × lease/work facts, plus a property test against an independent
   reference; the exhaustive match compiles without a wildcard.
2. Both commands, happy path: exact result, header and receipt, one audit, one outbox, no other row changed
   (row counts and `xmin` of runs, steps, chains and journal unchanged).
3. Vacate: refused with a `LEASED` step and with a `RUNNING` step, also when the lease has expired; allowed with
   `READY`, `PENDING`, `WAITING` and `RETRY_WAIT` steps; refused for a non-holder and for a final head.
4. Acquire: eligibility matrix by run status and step status including a due and a not-yet-due wake; refused
   when occupied, at a stale epoch, for a paused or manual-review run.
5. Replay returns the original result after the state has moved on; the same key with another command is
   `FingerprintConflict`; a receipt cannot be forged for a skipped revision.
6. Barriers, never sleeps: a held group row gives `Busy` and releases the chain and run guards (independent
   probes); two members racing to Acquire in both orders leave one winner, and the loser's retry sees `Occupied`;
   Vacate against Acquire; held deferred retained-usage row during a valid commit.
7. Confirmed abort versus lost acknowledgement, reconciled through the receipt, using the Finish commit-loss harness.
8. Cross-tenant, cross-namespace and member/chain mismatch are inert; an absent group is never "vacant".
9. Runtime-privilege audit passes for every statement; the down migration refuses with retained receipts.

## Open obligations

- **G714 — Index for the lease probe.** `steps_lease_expiry_idx` is keyed by expiry, not by run. Prove with
  `EXPLAIN` that the `LEASED`/`RUNNING` existence probe for one run does not scan; if it does, add a partial
  index on `(tenant_id, run_id)` for those two statuses (additive migration).
- **G715 — One runnable predicate.** "A wake is due" must reuse the scheduler's existing runnable
  definition, not a second copy that can drift. Add a check that fails if the two diverge.
- **G716 — `SCOPED_PATH` members.** Same two commands, different authority; separate slice.
- **G717 — Possession is not yet read by anyone.** Nothing gates a claim on possession, so this slice cannot
  change behaviour. The worker-claim and scheduler gating is the integration slice (C1).
- **G718 — Fairness and starvation.** Not claimed; no queue, no order among eligible members beyond who
  commits first.
- **G719 — The caller.** No scheduler or worker calls these commands. When one does, the Vacate trigger and the
  Acquire discovery (`capacity-discovery.md`) must compose without holding two existing run locks.
- **O705 (owner) — Who vacates, and when?** Possession cannot be taken from a holder (A0), so a holder
  that parks on a durable wait for a child must Vacate **in the transaction that makes it idle**, or the
  child can never acquire and the group stops (the ADR-0030 §14 deadlock, at any limit). Is vacate-on-idle
  the intended rule? It changes the scheduler's existing run transactions, which are not part of this slice.
