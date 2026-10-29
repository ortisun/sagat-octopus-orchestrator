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

- [x] Command/port contract and domain-separated fingerprint. Observe focused RED, then
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

## Evidence

The focused command RED ran in CI40862510897/job123409010726: one passed, three failed,
zero ignored. The canonical independent vector, binding/counter changes and changed run revision
all rejected the zero-fingerprint scaffold. The remaining jobs were cancelled after retaining
this failure. Local formatting, actionlint and diff checks passed. The local focused build was
interrupted before executing tests when two other builds were detected; it is not test evidence.

The encoder is now shared with Vacate without changing its prefix, byte0, field order or
big-endian counter representation; Acquire uses byte1. The next CI first executes both command
families, then the three native Acquire tests against an explicit missing-operation scaffold.
No native implementation or complete component acceptance is claimed by this checkpoint.

CI40863527219/job123412430206 then passed the shared command regression: eight passed,
zero failed (both operation vectors and existing capacity tests). Its three native tests
subsequently failed against the Missing stub in11.29s, including an explicit Missing-versus-Occupied
refusal mismatch. The other jobs were cancelled after retaining that native RED.

## Shared transaction implementation boundary (reviewed)

Vacate and Acquire will use one private PostgreSQL transfer implementation, so exact binding,
lock order, current scheduler fencing, receipt-before-head replay, four-counter CAS and COMMIT
classification cannot drift between copies. It receives the operation and existing identity,
expectations, scheduler authority and operation key directly; no mirrored command DTO is needed.
After replay and fresh-state validation, operation branches select their evidence and gate:
Vacate reads LEASED/RUNNING steps under the run lock before its structural gate; Acquire calls
the movable-status gate without a worker-lease or runnable-work probe. Branches also select the
group transition, result possession, audit action, outbox event and receipt kind.

Keep both existing operation-specific port/result names and their `vacated_at`/`acquired_at`
fields. A private immutable transition receipt projects into these results at the port boundary.
Preserve the existing Vacate error enum and the distinct Acquire enum. The private transaction
error bridge reuses existing Vacate errors for shared failure semantics and adds only the two
Acquire-specific Occupied/NotMovable cases. Explicit exhaustive projections preserve the public
port types and refusal meanings, including ContradictoryProjection for operation-inapplicable
domain refusals. This avoids a third mirrored full error enum. No SDK, HTTP or wire enum changes.

The existing Vacate fingerprint vector and all 18 native cases are required regression evidence,
alongside Acquire cases. There is no new migration, creator permission, scheduler caller or claim
guard in this extraction. Persisted fingerprints/receipts and existing Rust port paths, distinct
error types and result fields remain compatible. The shared encoder has a crate-private operation
enum; the native shared call receives the fingerprint computed by its typed public command.

## Native implementation checkpoint — 2026-10-29

The transaction, distinct typed error projections, acquired audit action, closed metrics/span
and additive `capacity.member.acquired` AsyncAPI message are implemented. A focused fresh-context
review of the shared engine against the prior Vacate adapter found no substantive transaction,
control-flow or SQL-binding issues. This review is not execution evidence.

The baseline diff contains only the new message shape, matching the existing reference fields;
no old message was changed. The normal non-bless contract suite must validate this addition in CI.
Native cases now include command refusals, finality, operation-key domain conflicts, movable status,
Busy rollback, outbox rollback, actual lost COMMIT acknowledgement (commit and abort), and OTLP/metrics.
These new cases are awaiting CI execution; no green native acceptance is claimed yet.

Lightweight validation: `cargo fmt --all -- --check`, `python3 scripts/check-architecture.py`
(24 crates, no cycles), `python3 scripts/check-agent-config.py` (`agent configuration ok`),
`actionlint` and `git diff --check` passed. Heavy tests remain in CI: the shared disk had only
8.5 GiB free when checked, and other sessions were actively building.
