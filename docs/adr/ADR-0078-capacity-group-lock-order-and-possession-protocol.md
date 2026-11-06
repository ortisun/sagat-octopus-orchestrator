# ADR-0078: The ChildRoot capacity group header is a lock nobody waits for, and possession moves only through fenced commands

**Status:** Proposed (2026-11-05). Accepting it closes gap G721 as the gap register words it (the possession protocol, the lock position of the group header, and the sweeper's protocol but not its coverage). It does **not** close the items under "Open", which OD-2026-11-04-4 also lists; each needs its own decision before activation
**Decided by:** Agent draft, revised three times after an architecture review and a PostgreSQL/transaction review of 2026-11-05. Both rejected the first draft; on the second pass both rejected the revision for two wrong rules the author had introduced (an operation-key derivation and the handling of ADR-0075's amendment); on the third pass both found no Blocker or Major. "Evidence" gives the detail and what no review covers. The sweeper (O705) and the lease-only possession gate (O706) were decided by the owner; the rules below consolidate decisions D704 to D714 of feature 014. A person accepts
**Refines / supersedes:** Refines ADR-0030 §§18–19, ADR-0037 (§2, §6), ADR-0038 and ADR-0075 by fixing where the capacity group header G sits and how it is locked. Supersedes nothing. It records that the sentence of ADR-0075 telling a membership creator to lock chains in ascending order and then runs was withdrawn before ADR-0075 reached `develop`, because it was never reconciled with ADR-0030 §19 and ADR-0037 §6 (owner-decisions row OD-2026-11-05-1)
**Enforced by:** Partly built.
Built: real-PostgreSQL tests of the native commands and of the worker guard; `scripts/check-capacity-activation.py`, which bans any production caller and the opt-in guard outside tests (so the sweeper-only rule is not yet an allowlist); the step-status-writer inventory `specs/014-workflow-primitives/contracts/step-status-writers.json`, on which the soundness of the Vacate gate depends; the runtime-privilege test.
Not built, required by the Activation gate of ADR-0075: an extension of the script (a file allowlist for the three commands, and every `FROM capacity_groups ... FOR` string carrying `NOWAIT` or `SKIP LOCKED`); a real-PostgreSQL test per writer of a group or member row, run with G held on another connection, expecting `Busy` or a skip and no wait (a text check cannot see a plain `UPDATE` or a foreign-key wait); claim-versus-command barriers in both orders; the starvation test of ADR-0075 condition (j).
Until then this ADR is enforced by review only

## Context

Feature 014 lets a nested ChildRoot share a tenant capacity unit with its ancestor chain instead of
allocating a second one (D705). The shared unit is a **capacity group**: a header row G with a
possession state (`VACANT`, `HELD`, or terminal `RETIRED` once its last active member has left), a holder
member, a revision and an epoch, plus one member row per ChildRoot. Only the holder may be granted new
worker leases (O706).

The pieces exist and are dormant. Native `Vacate`, `Acquire` and `Finish` commands move possession.
Bounded discovery reads candidates. A worker guard checks possession before granting a lease and is off
by default (ADR-0075). No production code calls the commands, nothing creates groups or members, and the
runtime role `sagat_app` cannot insert them.

What is accepted so far is partial. ADR-0030 §18 fixes "chain before its current run" and §19 the
parent/child link order. ADR-0037 permits one stable parent-identity guard. ADR-0075 adds G to the worker
lease path. ADR-0037 and ADR-0038 both state that they do **not** accept the lock graph or a future group
order, and the order of the native commands lives only in the feature notes (gap G721). Registry
uniqueness is group-scoped, so the database does not today prove one active group per chain.

The worker guard **as built** does not meet rule 2: it locks G with `SKIP LOCKED` before it reads the
holder, so a non-holder poll holds G to the end of its transaction and makes a native command return
`Busy` (ADR-0075 condition (j)). It is off by default and activation stays blocked until it conforms.

The risk this ADR addresses is a lock cycle or a double holder when four kinds of transaction meet on one
group: a worker claim, a native command, a membership creator and the scheduler.

## Decision

### 1. G is a leaf lock

- A transaction locks G only in the ways of rule 2, none of which waits. Nothing waits for G, so no
  wait-for edge ends at G and G cannot be a link of a lock cycle. Holding G longer makes other
  transactions wait longer for *other* locks; it adds no cycle.
- The orders that already bind are not restated or changed: ADR-0030 §18 (chain before its current run;
  for a creation the tree-admission advisory lock comes first, for a handoff it comes after the chain, the
  parent identity guard, the head, the fence and the link, ADR-0037 §2), and ADR-0030 §19 with ADR-0037 §6
  (never two existing run mutation locks, no parent/child cross-locking).
- G sits after the chain and after the run the command names, never before. A native command takes the
  chain, the run it names, G `NOWAIT`, then the shard `FOR SHARE` and the database clock. A worker claim
  takes the chain and the run, the other locks of its claim path, then G, then per-instance admissions;
  its complete sequence is open (see "Open").
- A transaction MUST NOT acquire a chain or run lock it does not already hold after G.
- Receipt foreign keys are checked at INSERT and take `KEY SHARE` on the member row, which the command does
  not lock; the holder foreign key is deferred to COMMIT. A `KEY SHARE` on a member conflicts only with a
  state change of that member, and every such writer (`Finish`, the scoped leaf finish) takes G first, so
  it cannot run beside a transaction that holds G.
- The chain, run and shard locks inside a command are blocking waits bounded by the configured statement
  timeout (5 s by default). G is the only lock no command waits for.

### 2. How G is locked

- Native `Vacate`, `Acquire` and `Finish` MUST lock G with `FOR UPDATE NOWAIT`. SQLSTATE `55P03` is the
  typed `Busy` error, consumes the whole transaction, and a retry starts fresh (D707).
- A worker claim MUST read the header without a lock and MUST lock G only when that read shows the
  claiming member as holder, with `FOR UPDATE SKIP LOCKED`, rereading the header and the member under the
  lock; the lock MUST survive to the lease commit (ADR-0075). A stale unlocked read can only cause a
  spurious refusal: the header is reread under the lock, and each READ COMMITTED statement takes a fresh
  snapshot.
- A membership creator MUST take G `FOR UPDATE NOWAIT` before the first insert into
  `capacity_group_members`, because the foreign-key check of that insert waits unconditionally behind a
  worker's lock on G (ADR-0075).
- Other code MUST NOT lock a `capacity_groups` row without `NOWAIT` or `SKIP LOCKED`. Any new writer of a
  group or member row MUST be checked against rule 1.

### 3. Possession moves only through three fenced commands

- Possession cannot be taken. It moves only by `Vacate` (holder to `VACANT`), `Acquire` (`VACANT` to `HELD`
  for an ACTIVE member) and `Finish` (a FINAL head releases and retires its member, and the group when it
  has no active member left).
- Each command is one transaction that compares and sets the group revision and epoch and compares the
  chain and run revisions. `Vacate` and `Acquire` write only the header, one audit row, one outbox row and
  one transfer receipt (D710, D711). `Finish` also retires the member and writes a finish receipt. None
  writes a run, step, chain, journal or quota row.
- `Vacate` and `Acquire` lock the run they name after the chain and then compare it with the chain's
  current head (`StaleHead`); `Finish` compares first and then locks.
- `Vacate` is gated by quiescence: the selected member is the holder and ACTIVE, the head equals the
  observed head and is not final, and no step of the head run is `LEASED` or `RUNNING`, expired or not.
  The gate reads no clock (D708). `Acquire` requires `VACANT` at the expected epoch, an ACTIVE member, the
  same head, not final, and a run status that can move (D709). It does not assert that the member has work.
- A receipt, keyed `(tenant_id, group_id, operation_key)`, is evidence of a completed transition and an
  idempotency key, never authority: the header is the authority (D711). A replay needs a valid binding and
  a current scheduler fence; a lost fence fails closed (D707).
- Commit uncertainty follows the `Finish` contract with one shared classifier: only a server-confirmed
  constraint, serialization or deadlock abort implies rollback; anything after the COMMIT is dispatched is
  `CommitOutcomeUnknown` and is reconciled by re-sending the retained command (D713).

### 4. Callers

- The only caller of `Vacate`, `Acquire` and `Finish` is the scheduler-tick sweeper (D714, O705):
  level-triggered, bounded pages, one independent transaction per operation, one run lock at a time,
  idempotent. It calls `Finish`, not `Vacate`, for a holder whose head is FINAL and not yet finished. No
  production caller of any of the three exists today.
- The sweeper MUST hold no transaction and no lock, on any connection of the same task, while it calls a
  command. A command opens its own connection, so a lock held by its caller is invisible to PostgreSQL and
  the command would wait for it until the statement timeout.
- The sweeper MUST derive each operation key from the whole tuple the command fingerprints (kind, group,
  member, head run and the four expected counters), so that two different transitions never share a key.
  An epoch alone is not enough: `Finish` of a non-holder that is not the last member leaves the epoch
  unchanged, so two such finishes in one epoch would share a key and the second would fail with
  `FingerprintConflict`. A `FingerprintConflict` from a derived key MUST be treated as "rediscover",
  never as success. A restarted sweeper rediscovers from the state, and a transition that already
  committed no longer qualifies; replay by key is for a retained command (D713).
- Its discovery pages ACTIVE members and filters holders afterwards; the index on `possession='HELD'` that
  D714 assumes does not exist and is a prerequisite.
- A command MUST NOT run inside a worker report, a step transaction or any transaction that already holds a
  run lock beyond its own entry locks. This rule cannot be checked while there is no caller; it is
  unproven until the sweeper exists and a barrier test covers it.
- Possession gates worker lease grants only (O706). It MUST NOT gate reports, heartbeats or scheduler
  transitions (timer fire, retry release, control nodes, child outcomes).

### 5. Membership

- A nested descendant joins an existing group and never allocates one (D705). The only tenant-level
  capacity gate is fresh admission `retained < limit` (D704).
- A database unique index on (tenant_id, namespace_id, chain_id) where the member kind is `CHILD_ROOT` and
  the state is `ACTIVE` MUST exist before any creator ships. This ADR takes the index, not the alternative
  that condition (k) of ADR-0075 allows; until then the chain lock and the fail-closed guard are the only
  enforcement.
- `SCOPED_PATH` members are out of scope. They MUST fail closed for the ChildRoot commands
  (`BindingMismatch`) and keep their own commands and receipt table (D706, G716). The scoped leaf finish
  already takes G and updates member rows, always taking G first; it MUST follow rule 2.

### 6. Eligibility, observation and retention

- A single shared runnable predicate, one SQL fragment used by the scheduler and by `Acquire`, with a
  differential test, MUST exist before `Acquire` may assert that a member has work or before candidate
  selection may skip non-holders (G715). Fairness among members is not claimed (G718).
- Capacity telemetry labels MUST come from the closed sets of D712 (operation, outcome, refusal class) and
  of `worker_claim_skipped_total{reason}`; adding a value needs a test and a documentation change. D712
  calls the refusal class "about six values" and no code enumerates it yet; enumerating it with a test is
  part of the Activation gate of ADR-0075 (h). The sweeper MUST export a HELD-age signal.
- After the first membership or receipt exists, binaries without the guard enabled (a guard-capable binary
  with the flag off included) MUST NOT run and the schema MUST NOT be rolled back: the down migration of
  the transfer receipts refuses once a receipt exists (SQLSTATE `55000`). Activation is gated by the database-recorded flag of ADR-0075 condition (f).
- Receipt growth is bounded only by the group revision. A per-group ceiling or a compaction path MUST be
  decided before activation (G720).

## Considered options

1. **G as a leaf lock never waited for, three fenced commands, a level-triggered sweeper** — keeps every
   accepted order, makes G unable to be part of a deadlock cycle, and keeps every transition idempotent and
   recoverable by re-sending.
2. **Group first, then chain and run** — rejected: contradicts ADR-0030 §18 and every existing path, and
   would force the creation path to invert the order.
3. **A blocking wait on G, with or without a timeout** — rejected: ADR-0075 forbids a blocking group wait
   after holding a run; a waiter holding run A and waiting for G held by a transaction that needs run A is
   a cycle.
4. **Inline `Vacate` when a holder becomes idle, inside the worker or step transaction** — rejected by the
   owner's approval of D714 (O705): it would put a command that locks G and a second run inside a
   transaction that already holds one; the sweeper costs at most one tick of latency.
5. **Gate scheduler-owned transitions on possession too** — rejected by the owner (O706): a larger change
   that gates the scheduler's own transactions.
6. **One ACTIVE membership per chain by convention only (the writer and the fail-closed guard)** — rejected
   for activation: a violation halts the chain instead of being refused at write time; the index costs one
   write on membership creation.

## Consequences

Good: no transaction waits on G, so no lock cycle can include it. No lease is **granted** without G by a
guarded binary. A lost acknowledgement of a lease COMMIT is safe: the backend keeps G until the server
resolves it and no token is returned; the orphan `LEASED` step keeps its counted attempt and only delays
`Vacate` until the lease expires and the reaper clears it (the step-status-writer inventory covers that
writer). A lost
acknowledgement of a command is recovered by re-sending, not by guessing.

What this does not stop, recorded and not hidden:

- A worker whose lease was reaped may still run as a zombie, and an activity that blocks inside a worker
  waiting on a descendant keeps `Vacate` refused indefinitely (G723). The gate cannot see either.
- Work leased before a member joined a held group has no possession; the creator must prove the joining
  chain has no `LEASED` or `RUNNING` step.
- A binary with the guard off, or an older binary, leases without looking at the group (ADR-0075).
- A busy group reduces throughput instead of blocking. A holder that cannot progress waits up to one
  sweeper tick to be vacated (D714).
- Two receipts, two audit rows and two outbox rows per vacate/acquire cycle accumulate (G720).

### Open and not decided here

Each is a pending decision, not a guarantee.

- **The creator.** Who creates groups and members, through which API, with which audit, and how the runtime
  role is granted insert (B1e, G722). The chain and run locks it holds and their order are undecided and
  MUST NOT contradict ADR-0030 §19 and ADR-0037 §6; they must also be reconciled with the parent-first
  order of child creation (`FOR NO KEY UPDATE` on the parent, then the tree advisory lock).
- **Sweeper coverage.** Confirmed in the code (`crates/domain/src/execution/capacity.rs`,
  `crates/infra-postgres/src/capacity_finish.rs`): `Finish` accepts any ACTIVE member with a FINAL head,
  holder or not, and a non-holder that is not the last member leaves the epoch unchanged. The sweeper of
  D714 pages `HELD` groups with a quiescent holder, so a non-holder member with a FINAL head, or a vacant
  group whose members are all FINAL, would never be finished or retired, and the group's unit would count
  against fresh admission until it settles (D703). What stays open is the sweeper's selection; it MUST
  cover these cases before activation.
- **The worker's complete lock sequence.** Read in the code by the PostgreSQL review, not by the author,
  and not yet written down in a checked artifact. Before G: the drain advisory lock (shared), the binding
  advisory lock (scoped path), a blocking `worker_sessions` lock and secret advisory locks
  (`crates/application/src/worker.rs`, `crates/infra-postgres/src/execution.rs`). After G: the activity-quota
  advisory lock (exclusive), the breaker, and the database and script locks (`quotas.rs`,
  `execution/database.rs`). G is re-taken per instance inside one run's transaction, which takes no new
  chain or run lock. The list MUST be written down where a check can read it and verified against rule 1,
  including ADR-0038's binding guard.
- **`SCOPED_PATH` commands** (G716), **the receipt ceiling** (G720) and **fairness** (G718).

## Evidence

Native tests of Vacate, Acquire, Finish, discovery and the worker guard exist and are described in
[the vacate/acquire notes](../../specs/014-workflow-primitives/capacity-vacate-acquire.md) and
[the coordinator ledger](../../specs/014-workflow-primitives/capacity-coordinator.md). Twelve focused
PostgreSQL cases passed in CI40990030632 on `ec29e41b`; the released-source profile job of that run failed,
as the coordinator ledger records. The claim-versus-command barriers in both orders, an unknown-COMMIT
injection on the lease path and the full gate at the integrated head are pending.

Reviews, read-only, nothing run. On 2026-11-04 an architecture review and a PostgreSQL review found the
order chain → own run → G in every path that touches a group, no group-first path and G never waited for
([the delegated-decisions evidence](../../specs/014-workflow-primitives/evidence/delegated-decisions-20261104/README.md)).
On 2026-11-05 the same two roles reviewed the first draft of this text and rejected it: the architecture
review for a rule that accepted handoff code already breaks (the tree-admission lock comes after the chain,
confirmed by the author in `chain_progress.rs`) and for factual errors against the schema and the code; the
PostgreSQL review for a "total order" claim that is false as written and a wrong foreign-key argument. Both
confirmed that the core design holds: G is a leaf, and chain → run → G is respected by every command and by
the worker claim.

Second pass: both closed most findings and independently found the same new error, an operation key of
group, epoch and kind that collides for `Finish` (confirmed by the author in
`crates/domain/src/execution/capacity.rs`); the architecture review added that ADR-0075 had been edited
after its acceptance, which is addressed by OD-2026-11-05-1 and by the fact that the acceptance sat in a
pull request not yet merged. Third pass: both answered that a person may accept this text, with no
Blocker or Major left; the remaining points were minor and are applied (provenance wording, the
`ADR-0075` provenance note, the README index entry, one overlong line). The PostgreSQL review did not
re-verify the handoff order of rule 1; the architecture review and the author did, in `chain_progress.rs`.

What no review covers: nothing was run. The creator, the sweeper's coverage, the worker's complete lock
sequence as a checked artifact, rule 4 and the per-writer tests are pending and gated by the Activation
gate of ADR-0075. Accepting this ADR does not close any of them.

The author re-read in the code: the tree-admission lock after the chain lock in `chain_progress.rs`; the
refusal of the transfer-receipt down migration once a receipt exists; the absence of an index on
`possession='HELD'`. Owner answers: O705 and O706 in [owner decisions](../owner-decisions.md).
