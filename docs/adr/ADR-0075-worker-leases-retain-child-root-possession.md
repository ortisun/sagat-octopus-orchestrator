# ADR-0075: Worker leases retain ChildRoot possession through commit

**Status:** Accepted (2026-11-04, by delegation) for the ChildRoot worker-lease guard only; production activation is NOT accepted (see "Activation gate")
**Decided by:** Owner for O705/O706 scope; accepted by delegate on 2026-11-04 after an architecture review and a PostgreSQL/transaction review (OD-2026-11-04-3). Reversible until the owner confirms that row; the status returns to Proposed if the owner declines
**Refines / supersedes:** Refines ADR-0030 §18 (adds the group header G after chain and own run, acquired without waiting). Supersedes nothing. The native-command lock position, the sweeper, the creator and SCOPED_PATH are decided by the lock-graph ADR (gap G721), not here
**Enforced by:** `capacity_worker_possession`, focused PostgreSQL worker-capacity tests,
`scripts/check-capacity-activation.py` and its negative fixtures. Not enforced by tooling yet: no
blocking wait on G, and the presence of the guard in both lease producers; the activation script is a
source-text dormancy and inventory check only. Production activation remains a separate acceptance gate

## Context

O705 chooses a scheduler-tick sweeper with an independent transaction per native capacity
operation. O706 requires possession only for granting new worker leases. Existing native
Vacate/Acquire/Finish take chain → own run → group NOWAIT → scheduler fence. Registry
uniqueness is group-scoped, so it does not prove one active group per chain.

## Decision

- The ChildRoot worker guard MUST be opt-in and disabled in production until every condition of
  the "Activation gate" holds. It MUST NOT enable publication of a wider workflow graph.
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
- A worker claim MUST NOT take a row lock on G unless an unlocked header read shows HELD by the
  claiming member. Vacant, Retired and non-holder refusals are decided without locking G. The
  grant path rereads the header and the member under `FOR UPDATE SKIP LOCKED`. A refusal from the
  unlocked read is safe (no lease is granted); only a grant needs the lock.
- A busy/nonholder group MUST yield no external lease and consume no attempt or admission
  quota. The normal transaction commit may retain preceding secret-refusal audits; it MUST
  NOT disguise a rollback as a successful commit.
- Membership creation MUST lock chains in ascending (tenant_id, chain_id) order, then runs, and
  MUST issue `SELECT ... FROM capacity_groups FOR UPDATE NOWAIT` before the first INSERT into
  `capacity_group_members`, because the foreign-key check of that insert waits unconditionally
  behind a worker's lock on G.
- Unsupported SCOPED_PATH registrations MUST fail closed while their coordination semantics
  remain pending. ChildRoot scope is not a general scoped-capacity implementation.

## Considered options

1. **Guard immediately before external admission** — preserves deterministic internal failure
   progress and keeps every lease within the group lock lifetime.
2. **Guard at transaction entry** — rejected: blocks synthetic input-limit failure progress,
   contrary to O706, and risks gating reports/internal scheduler callers.
3. **Guard in generic persistence with a poisoned commit** — rejected: admission side effects
   have already occurred and the commit contract would become misleading.
4. **Lock G first, then decide whether the member is the holder** (the code at acceptance) —
   rejected by the PostgreSQL review: when the group is vacant every member is a non-holder, so
   all member pollers contend with the native Acquire (`NOWAIT`) and can starve it. Conforming the
   code is activation condition (j); the guard is off, so nothing is affected today.

## Consequences

Registered claims add bounded indexed reads and, for a holder, retain G while their existing
admission checks run. Busy groups reduce throughput rather than creating cross-run lock cycles,
because nothing waits on G. G is held across blocking advisory-lock waits during activity admission,
so native commands see Busy and peer guards skip; that costs throughput only. Direct owner fixture
writes are not a supported concurrent creator.

Older control-plane binaries (the only LeaseGrant producers are the root and scoped paths of
`WorkerService`) do not call the guard. Mixed versions are safe only while no ChildRoot membership
exists; the first membership MUST NOT be creatable until every replica runs the guard enabled.
Candidate selection is capacity-blind (G715), so non-holders can occupy the claim window. Both are
activation prerequisites. "One ACTIVE membership per chain" is today a writer convention plus the
fail-closed guard, not a database constraint; the cost of a violation is a halt, not corruption.

Indexes retain no new business data. Disabling the caller leaves existing leases reportable;
downgrade still obeys retained native-receipt guards. This ADR proves no latency or nesting SLO.

## Activation gate

The guard MUST NOT be enabled in production, and no membership may be created, until all hold:

- (a) The lock-graph ADR (G721) is accepted, with a CI check that no `capacity_groups` row lock lacks
  `NOWAIT` or `SKIP LOCKED`. ADR-0030, ADR-0037, ADR-0038 and this ADR do not accept the full graph.
- (b) The D714 scheduler-tick sweeper is wired with held-lock, restart and lost-hint tests. Without it
  an idle holder never vacates and every other member starves.
- (c) Group and member creation exists, with quota/backfill, tenant-unit accounting, receipt
  retention (G720) and recovery.
- (d) Candidate selection ignores non-holders (G715), so stuck non-holders cannot crowd out other runs.
- (e) Claim versus Acquire, Vacate and Finish races pass in both orders, with expiry and tenant
  cases, and the full gate is green at the integrated head. This includes one real-PostgreSQL
  barrier test, run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, in which holder and non-holder pollers race
  sweeper Vacate and Acquire, an unknown COMMIT is injected on the lease path by terminating the
  backend after COMMIT is sent, no group ever has more than one chain with a live LEASED or RUNNING
  step, and Acquire eventually succeeds.
- (f) Every control-plane replica runs a guard-capable build with the guard enabled before the first
  membership row can be created, and a mechanism enforces that order: activation is the first
  membership creation, gated by a database-recorded flag set only after every replica runs the
  guarded binary.
- (g) The activation script asserts that both lease producers call `worker_capacity_available`.
- (h) A runbook entry exists and the skipped-claim reason is observed
  (`worker_claim_skipped_total{reason}`).
- (i) A measured run replaces this ADR's disclaimer that it proves no latency or nesting SLO.
- (j) The worker claim conforms to the unlocked-header-first rule of the Decision, with a test that a
  vacant group's Acquire is not starved by member pollers.
- (k) A `UNIQUE (tenant_id, namespace_id, chain_id) WHERE member_kind='CHILD_ROOT' AND state='ACTIVE'`
  index exists before the creator ships (the duplicate-active fixture in
  `capacity_worker_leases.rs` is then reworked), or an ADR records that only the creator and the
  fail-closed guard enforce it.
- (l) The creator migration grants column-scoped INSERT on both tables to `sagat_app` before the
  creator code ships, and the runtime-privilege test is extended.

## Evidence

Owner answers: [owner decisions](../owner-decisions.md), O705/O706 and OD-2026-11-04-3. Pure policy
RED: six tests accepted invalid possession facts before enforcement. GREEN: 614 domain tests passed,
zero failures/ignored. First review found that entry guarding would block internal preflight; the
revised boundary passed the second bounded review.

Twelve focused PostgreSQL cases passed in CI40990030632 on `ec29e41b`; the released-source profile
job of that run failed, as [the coordinator ledger](../../specs/014-workflow-primitives/capacity-coordinator.md)
records. Both-order claim/transfer barriers and the full gate remain pending.

Reviews of 2026-11-04 (read-only, nothing run): the architecture review and the PostgreSQL review
found the lock order chain → own run → G in every path that touches a group, no group-first path, and
G never waited for, so G cannot be part of a deadlock cycle. Their opinions and the amendments they
required, all applied above, are in
[the delegated-decisions evidence](../../specs/014-workflow-primitives/evidence/delegated-decisions-20261104/README.md).
