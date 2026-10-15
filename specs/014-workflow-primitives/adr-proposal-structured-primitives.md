# ADR proposal — Structured durable instances and explicit continuation chains

**Status**: Proposed, pending acceptance and permanent ADR number (T014-002; the next free number
when this was written is ADR-0030).
**Date**: 2026-10-14. **Feature**: 014. **Supersedes**: none.

## Context

ADR-0003 selects declarative graph-native workflows; current definition validation rejects cycles
and current ordinary step identity is a definition node within a run. Runtime-sized collections
and repeated bodies need separate instances without changing a previously completed node's state.
Human decisions require an authorization operation beyond generic signals. Interactive state and
long-lived chains need explicit contracts without arbitrary code in the control plane.

## Proposed decision

1. Retain acyclic graphs; add closed acyclic body templates with bounded dynamic instance paths.
   Loops/fanout allocate distinct logical instances, leases, effects and results under shared budgets.
2. Use one instance-aware pure reducer and transactional store. All scheduling, quota accounting,
   timers/subscriptions, receipts and outbox remain inside the transition transaction. Do not invent
   a separate in-memory loop engine or separate compensation mechanism. Compensation keeps ADR-0005's
   reverse topological order with branch independence (FR-003-002): reverse topological within an
   iteration, no order between collection iterations, reverse iteration order between loop iterations.
3. Express Query/Update as bounded schema-checked declarative state operations. The trusted evaluator
   interprets the existing expression language; it never invokes user callbacks or external code.
   Only Updates change declared state, an Update schedules nothing, and its concurrency token is the
   state revision. Returning values through the API needs a separate value-disclosure decision that
   refines ADR-0028; this proposal does not make it.
4. Native approval binds policy, initiating identity and timer to an instance; reauthorize the person
   at decision time and let first committed decision/expiry/cancel win. Eligibility names subjects
   and built-in roles only (no custom roles, G39).
5. Continue-As-New atomically closes a SUCCEEDED/CONTINUED segment and starts a same-version successor,
   preserving chain deadline/quotas/lineage. Existing run waits remain segment waits; explicit chain
   waits and commands opt into chain addressing. Child parents await the final chain outcome.

The full semantics, bounds, schemas and failure behavior are in `spec.md`, `data-model.md` and
`contracts/README.md`. This proposal refines ADR-0003, ADR-0005, ADR-0007 and ADR-0015, and reuses
ADR-0012, ADR-0016, ADR-0017, ADR-0023, ADR-0027 and the closed `Error.code` enum (new meaning only
in `details.reason`). It does not relax the constitution, replace the transaction contract or allow
automatic version migration. If review finds a divergence
from any accepted ADR, assign a new ADR with explicit supersession and update its dependents first.

## Alternatives and consequences

Structural back-edges, unbounded iterators and language-level procedural replay change the model and
hide resource ownership; rejected for this increment. Expanding only at publication cannot support
runtime item counts. A dedicated service for each new primitive adds transactions/network failure
without independent measured need; stay within the modular monolith. Generic signals lack native
approval authorization. Arbitrary async Update handlers require a separate pending-handler protocol;
keep the first contract atomic and bounded. A new CONTINUED status would break older enum clients;
use an additive completion kind with explicit chain semantics and test old-client compatibility.

Costs include more instance rows/events, wider identity in worker/storage contracts, additional
indexes and chain-before-run locking. Benefits are inspectable checkpoints and one recovery model.
No empty mapper/module is required; new types carry independent instance, approval and chain meaning.

## Acceptance evidence required — pending, not supplied by this proposal

- Migration and replay with old active runs, root identities and every supported worker minor.
- Faults before/after each scope checkpoint, approval/update receipt and cross-run handoff commit;
  duplicate/lost outbox delivery and stale scheduler/worker fences.
- Compensation/reconciliation across failed parallel iterations, timeout/signal/approval races and
  child-parent chain completion/cancellation.
- Tenant/RBAC/secret-redaction tests, cross-run quota contention and five-language golden parity.
- Benchmark report with hardware/configuration, transaction duration, lock waits, journal/storage
  growth, memory and scheduler delay for the plan's three workloads; operational recovery/runbook.

T014-002 accepts the architecture direction before engine work. Release acceptance additionally
requires T014-040–044 evidence. The acceptance of a proposal must not be mistaken for proven runtime
guarantees, and checking this document into the repository does not accept it automatically.
