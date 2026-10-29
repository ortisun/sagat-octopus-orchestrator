# Dormant native ChildRoot Acquire

Scope: the Acquire half of D706–D713/D715, after Vacate merge `06b95c7f`.
Refs: T014-014–017, T014-040, T014-042–044, FR-014-014–015.

The owner asked to continue open tasks on 2026-10-29 while preserving other sessions.
This work owns `feature/014-capacity-acquire` in the dedicated
`sagat-octopus-capacity-acquire` worktree, cut from `develop@06b95c7f`.
The schedules, Iceberg, elastic-scaling, legacy bounded-execution and classification
worktrees are read-only inputs and will not be changed. Registration lines in shared
module indexes remain small additive edits; no new migration or manifest change is planned.

## Contract

Acquire changes one vacant group's possession to its selected active ChildRoot member.
It proves exact tenant/namespace/member/chain/head identity, scheduler authority and four
CAS counters under chain → own run → group NOWAIT. Receipt replay precedes current-head
and fresh-state checks. The current non-final head must have a movable status; possession
does not prove runnable work or grant a worker lease. The only writes are group header,
audit, outbox and immutable transfer receipt, reusing migration 26.

Same-key reconciliation retains the complete command and original receipt after later
Vacate/continuation. A current fence remains required. COMMIT dispatch uncertainty is
reported with the original operation key and uses the shared confirmed-abort classifier.

O705/O706, runnable predicate, sweeper, production callers, claim guards, nested publication,
creation/join and receipt retention stay open. This slice does not choose or activate them.

## Ordered implementation

- [ ] Command/port contract and domain-separated fingerprint. Observe focused RED, then
  share the fixed-width encoder with Vacate and verify both existing and new vectors.
- [ ] Native transaction and typed refusals using the existing pure Acquire gate. Observe
  native RED before implementation; preserve Vacate's public contract and lock order.
- [ ] Audit, closed telemetry and reference-only AsyncAPI hint; reuse existing receipt grants.
- [ ] Native evidence for apply/replay/CAS/fence/tenant/binding/finality/movability, rollback,
  two-member contention in both orders, released guards after Busy and uncertain COMMIT.
- [ ] Check focused runtime/owner gates, required integrated CI and source-bound upgrade
  profiles, then record actual evidence and a ready PR. Canonical task closure requires
  the complete original acceptance, not just this component.

## Resource and coordination boundaries

Use a private target outside the repository, one Cargo build job for focused checks, and
at most two heavy builds across the shared machine. Stop local builds below 12 GiB free.
Heavy workspace/release validation stays in CI. Never clean or delete another session's
target, container, volume, branch or worktree. Recheck live worktree status before shared
registration edits and before integration.
