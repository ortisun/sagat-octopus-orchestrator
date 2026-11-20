# Architecture review of ADR-0075 and gap G721 — 2026-11-04

Recorded by the assistant from the reply of the `architecture-reviewer` agent (read-only; its
session could not write files, so this is the reply, not a file it produced). Paths are relative to
the repository at `54570349` (`origin/develop@62cc6045` plus the reconciliation commit). The
reviewer read the documents and the code; nothing was run. The reviewer was given the question and
the files, not the assistant's leaning.

## Verdict

**(1) ACCEPT WITH NAMED AMENDMENTS.** The amendments are made before accepting, because an accepted
ADR is not edited in substance (`docs/adr/README.md`, rule 2). The design holds against the code:

- The lock order is chain, then own run, then the group header G, taken with `FOR UPDATE SKIP LOCKED`
  (`crates/infra-postgres/src/execution.rs:404-408`, `crates/infra-postgres/src/execution/worker_capacity.rs:111-113`).
- G is never waited for, so it cannot be part of a deadlock cycle.
- More than one ACTIVE membership, finished-only history and SCOPED_PATH members fail closed
  (`worker_capacity.rs:50-63,91-106`).
- The guard sits after the input preflight and before managed-database, activity and script
  admission, in both claim paths (`crates/application/src/worker.rs:1085-1142`,
  `crates/application/src/worker/scoped_claims.rs:184-223`).
- Tests cover G being held through the lease commit, reports ignoring a busy G, and the default
  being off (`tests/integration/workflow-primitives/capacity_worker_leases.rs:136,174,357`).

**(2) G721 is NOT satisfied. A separate ADR is required before any production caller.**

- ADR-0075 covers only the worker-lease caller.
- ADR-0037 says it excludes group eligibility and the full lock graph
  (`docs/adr/ADR-0037-stable-parent-identity-guards-for-child-handoff.md:58-60,81-82`).
- ADR-0038 says it does not accept the lock graph or a future group order
  (`docs/adr/ADR-0038-production-incidents-follow-execution-lock-entry.md:33-34`).
- The lock position of the native commands (chain, run, G `NOWAIT`) is only in the feature notes
  (`specs/014-workflow-primitives/capacity-vacate-acquire.md:44-46`).
- The ledger still lists the lock-position ADR as undecided.

Decision points for the new ADR:

1. Total lock order chain, own run, G, then shard, activity, script and quota, with the rule that
   nothing blocks on G.
2. Possession protocol: possession cannot be taken, Vacate and Acquire gates, epoch and revision
   compare-and-set, receipts are evidence rather than authority.
3. Caller boundaries: the D714 sweeper, Finish, no inline Vacate in report or step transactions, and
   no gating of scheduler transitions (O706).
4. Group and member creation, join and retire: who may do it, serialization on chain and run before
   G, one ACTIVE membership per chain as a database invariant, tenant-unit accounting.
5. SCOPED_PATH members, which already take G in `crates/infra-postgres/src/scoped_leaf_finish.rs:327`.
6. Runnable predicate and candidate selection (G715) and fairness (G718).
7. Rollout, rollback and receipt retention (G720).
8. Fitness checks and the closed telemetry vocabulary.

**(3) Accepting the ADR is separable from activating.** Dormancy rests on four facts: the default is
`false` (`crates/application/src/worker.rs:536`), the activation script bans the opt-in call outside
tests (`scripts/check-capacity-activation.py:27-28`), `sagat_app` has no INSERT on groups or members
(`crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql:174-177`), and the
guard returns true when a run has no membership (`worker_capacity.rs:75-77`). Mixed-version rollout
is a blocker for ACTIVATING only.

The gate may be lifted only when all of these hold:

- (a) The G721 ADR is accepted, with a CI check that no `capacity_groups` row lock lacks `NOWAIT` or
  `SKIP LOCKED`.
- (b) The D714 sweeper is wired with held-lock, restart and lost-hint tests
  (`capacity-coordinator.md:56-57`). Without it an idle holder never vacates and every other member
  starves.
- (c) Group and member creation exists, with one ACTIVE membership per chain enforced, plus
  quota/backfill, retention (G720) and recovery.
- (d) G715 candidate selection ignores non-holders. Today the candidate window of at most 64 runs,
  ordered by `ready_at`, is capacity-blind (`worker.rs:857`, `crates/infra-postgres/src/workers.rs:61-153`),
  so stuck non-holders can crowd out other runs.
- (e) Claim versus Acquire, Vacate and Finish races pass in both orders, with expiry and tenant
  cases, and the full gate is green at the integrated head (unticked at `capacity-coordinator.md:50-51`).
- (f) Every control-plane replica runs a guard-capable build with the guard enabled before the first
  membership row can be created. The creator is the activation switch, and a mechanism must enforce
  that order.
- (g) The activation script asserts that both lease producers call `worker_capacity_available`.
- (h) A runbook entry exists and the skipped-claim reason is observed
  (`worker_claim_skipped_total{reason}`, 7 closed values, `worker_capacity.rs:12-34`).
- (i) A measured run replaces the ADR's own "no latency or nesting SLO" disclaimer.

## Amendments to ADR-0075 (old text to new text)

1. Status line: `Proposed (2026-10-29; production activation remains gated)` becomes
   `Accepted 2026-11-04 on the owner's delegation, for the ChildRoot worker-lease guard only.
   Production activation is NOT accepted; see "Activation gate".`
2. `Refines / supersedes: ADR-0030; no accepted semantics superseded` becomes `Refines ADR-0030 §18
   (adds G after chain and own run, acquired without waiting). Supersedes nothing. Native-command
   lock position, sweeper, creator and SCOPED_PATH are decided by the G721 ADR.`
3. `Old workers bypass this new opt-in guard, so mixed-version activation is NOT safe. Deployment
   fencing and source inventory remain activation prerequisites.` becomes `Older control-plane
   binaries (the only LeaseGrant producers are WorkerService root and scoped) do not call the guard.
   Mixed versions are safe only while no ChildRoot membership exists; the first membership MUST NOT be
   creatable until every replica runs the guard enabled. Candidate selection is capacity-blind
   (G715), so non-holders can occupy the claim window. Both are activation prerequisites.`
4. Evidence line `PostgreSQL lease and race evidence remains pending` becomes `Twelve focused
   PostgreSQL cases passed in CI40990030632 on ec29e41b (capacity-coordinator.md:113-114). Both-order
   claim/transfer barriers and the full gate remain pending.` (The assistant adds that the same CI
   run's released-source profile job failed, as `capacity-coordinator.md` itself says.)
5. `Enforced by`: add `Not enforced by tooling yet: no blocking wait on G, and presence of the guard in
   both producers. The activation script is a source-text dormancy and inventory check only.`
6. Add `Accepted by: owner (delegated), recorded in docs/owner-decisions.md` and an "Activation gate"
   section holding conditions (a) to (i).

## Risks the reviewer accepts

1. Liveness, not safety: the guard on without the sweeper and G715 would starve peers. Accepted
   because it is dormant and gated by (b) and (d).
2. "One ACTIVE membership per chain" is only a writer convention. The guard fails closed on it, so
   the cost is halting, not corruption.
3. G is held across blocking advisory-lock waits (activity admission,
   `capacity_worker_leases.rs:357-398`). Native commands then see `Busy` and peer guards skip. That
   costs throughput only, because nothing waits on G.

## What would change the verdict

A barrier test, or a `pg_stat_database.deadlocks` delta, showing a lease granted to a non-holder, or
a cycle in which some transaction blocks on G (for example a foreign-key KEY SHARE insert or an
unqualified `FOR UPDATE`) while holding run, chain or activity locks next to a worker claim. A lease
producer outside `WorkerService` that bypasses the guard, found through
`specs/014-workflow-primitives/contracts/step-status-writers.json`, would do the same.
