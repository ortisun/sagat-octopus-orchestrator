# ADR-0075: Worker leases retain ChildRoot possession through commit

**Status:** Proposed (2026-10-29; production activation remains gated)
**Decided by:** Owner for O705/O706 scope; implementation proposal reviewed by agent
**Refines / supersedes:** ADR-0030; no accepted semantics superseded
**Enforced by:** `capacity_worker_possession`, focused PostgreSQL worker-capacity tests;
production activation remains a separate acceptance gate

## Context

O705 chooses a scheduler-tick sweeper with an independent transaction per native capacity
operation. O706 requires possession only for granting new worker leases. Existing native
Vacate/Acquire/Finish take chain → own run → group NOWAIT → scheduler fence. Registry
uniqueness is group-scoped, so it does not prove one active group per chain.

## Decision

- The ChildRoot worker guard MUST be opt-in and disabled in production until coordinator,
  creator, quota/backfill, retention and recovery acceptance is complete. It MUST NOT enable
  publication of a wider workflow graph.
- Both root and exact-path worker claims MUST check possession after deterministic input
  preflight and before managed-database, activity, compensation or script admission. Synthetic
  input-limit failures publish no task/token and MUST remain ungated. Existing reports,
  heartbeats and scheduler transitions MUST NOT require possession.
- The transaction MUST already retain chain and own-run guards. A bounded tenant/namespace/
  chain lookup MUST distinguish no history from FINISHED-only history and reject multiple
  ACTIVE memberships. A registered member MUST be the current chain head.
- An applicable group MUST be acquired with `FOR UPDATE SKIP LOCKED`, never a blocking group
  wait after holding a run. Its exact selected membership MUST be reread under that lock.
  The selected member MUST be ACTIVE and the current holder; the lock MUST survive through
  the lease commit. Possession is only one prerequisite, not complete execution eligibility.
- A busy/nonholder group MUST yield no external lease and consume no attempt or admission
  quota. The normal transaction commit may retain preceding secret-refusal audits; it MUST
  NOT disguise a rollback as a successful commit.
- Unsupported SCOPED_PATH registrations MUST fail closed while their coordination semantics
  remain pending. ChildRoot scope is not a general scoped-capacity implementation.

## Considered options

1. **Guard immediately before external admission** — preserves deterministic internal failure
   progress and keeps every lease within the group lock lifetime.
2. **Guard at transaction entry** — rejected: blocks synthetic input-limit failure progress,
   contrary to O706, and risks gating reports/internal scheduler callers.
3. **Guard in generic persistence with a poisoned commit** — rejected: admission side effects
   have already occurred and the commit contract would become misleading.

## Consequences

Registered claims add bounded indexed reads and retain G while their existing admission checks
run. Busy groups reduce throughput rather than creating cross-run lock cycles. Future membership
creation MUST serialize on the affected chain/run before G; direct owner fixture writes are not
a supported concurrent creator. Old workers bypass this new opt-in guard, so mixed-version
activation is NOT safe. Deployment fencing and source inventory remain activation prerequisites.
Indexes retain no new business data. Disabling the caller leaves existing leases reportable;
downgrade still obeys retained native-receipt guards. This ADR proves no latency or nesting SLO.

## Evidence

Owner answers: [owner decisions](../owner-decisions.md), O705/O706. Pure policy RED: six tests
accepted invalid possession facts before enforcement. GREEN: 614 domain tests passed, zero
failures/ignored. First review found that entry guarding would block internal preflight; the
revised boundary passed the second bounded review. PostgreSQL lease and race evidence remains
pending in [the coordinator ledger](../../specs/014-workflow-primitives/capacity-coordinator.md).
