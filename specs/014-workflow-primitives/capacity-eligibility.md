# Capacity eligibility — forward work versus reserved drain (B1c)

Status: pure model implemented and reviewed twice (correctness: no defect in the code; architecture:
should-fix items below addressed); **not wired**; **D701/D702 are proposals pending owner question
O701** (below). No schema, scheduler caller, production counter or nesting is enabled by this
document. Refs: T014-014..017, T014-042, ADR-0030 §§12–19, ADR-0037, `data-model.md`.

Ids use the range **D701–D799, G701–G799, O701–O799**, reserved for the nested-capacity lane so they
cannot clash with the parallel `feature/014-v0.3-completion` session (highest ids on `develop`:
D602, G526, O501). They are registered in `research.md` ("Nested capacity eligibility").

Design inputs that are not on `develop` (read them with `git show`): branch
`handoff/016-project-completion-20261024` @ `cc1830a2c19e`, directory `tasks/project-completion/`:
`capacity-group-lifecycle.md` (A0/A1), `capacity-lock-graph.md` (B0), `capacity-transaction-entry.md`
(B1a), `nested-capacity-review.md` (three rejected whole-protocol cycles and the findings), and
`todo.md` (B1c = "current forward eligibility versus reserved drain, reduction and policy 0").

## The released contract this must not silently change

`data-model.md:98`: "A policy reduction stops new admissions above its new ceiling but does not
kill accepted work." `docs/runbooks/workflow-primitives.md:30`: reducing `iteration_concurrency`
"preserves admitted work and blocks further admission until use falls below the new value".
The flat behaviour is unchanged by this module: it is unwired. Any wiring must keep flat iterations
on exactly that contract unless the owner decides otherwise (O701).

## The model

`crates/domain/src/execution/capacity_eligibility.rs` consumes the effective `iteration_concurrency`
limit (a trusted input; zero is reachable because `quota_limits.max_value` may be 0) and a
constant-size tenant projection:

| Counter | Meaning |
| --- | --- |
| `retained` | Groups still holding a unit. Equals legacy flat `active_iterations`. |
| `enabled` | Groups in `Forward`: may take forward and reserved-drain work. |
| `draining` | At most one group in `Drain`: reserved-drain work only. |
| parked | `retained − enabled − draining`: retained, not executing, durable work unclaimable. |

| Operation | Allowed when | Effect |
| --- | --- | --- |
| `allocate_group` (fresh) | `retained < limit` | retained+1, enabled+1 |
| `enable` (parked → forward) | `enabled < limit` | enabled+1 |
| `enable_for_drain` (parked) | `enabled < limit` → `Forward`; else lane free → `Drain`; else `DrainLaneBusy` | enabled+1 or draining=1 |
| `park` | the counters hold a group in that enablement | enabled−1 or draining−1 |
| `retire` | the counters hold a group in that enablement | retained−1 and the matching decrement |
| `claim` forward work | group `Forward` and `enabled ≤ limit` | none (read-only) |
| `claim` reserved drain | group `Forward` or `Drain` | none |

Every operation also returns `WrongEnablement` for a stated enablement that does not fit
(`park(Parked)`, `enable(Forward)`) and `InconsistentCounters` when the counters hold no group in the
stated enablement, `claim` included. Counters are anonymous: they do not know *which* group.
Limit zero admits and forward-enables nothing; the drain lane still opens. A reduction changes no
counter. Executing groups are bounded by `max(enabled, limit) + 1`: the lane adds one beyond the
enabled ones and does not serialise drain among enabled groups. A lane holder is never promoted;
it parks and enables again.

"Reserved drain" here means work that consumes an already reserved compensation/reconciliation
identity (ADR-0030 §13, a lifetime `maxInstances` reservation). §13 says such obligations stay
drainable after exhaustion or reduction; it does not say they bypass the concurrency quota.
Exempting them from `iteration_concurrency` is an extension (D701).

## Decisions (proposed)

- **D701 — Reserved drain does not depend on the limit; above the bound it uses one drain lane per
  tenant.** Rejected: drain competing for ordinary capacity (policy zero would strand accepted
  compensation); unbounded drain groups (defeats the reduction). Cost: drain throughput above the
  bound is bounded by one extra executing group (G703). Pending O701/O703.
- **D702 — Proposed: a reduction revokes no enabled group but stops *new forward claims* and
  re-enabling while `enabled > limit`.** This pauses admitted work and is stricter than the released
  contract above. It comes from the rejected v3 draft, not from accepted text. Rejected: forced
  parking (needs a quiescence proof the model cannot make). No forced-preemption promise: an
  always-busy holder keeps the tenant over the bound. **Pending O701.**
- **D703 — Parked groups keep counting against fresh allocation.** Same as flat `active_iterations`
  (a flat iteration waiting on a wait holds its slot). Consequence G708.

**If the owner answers "no pause" (O701 option A), `enabled`, the drain lane, `OverBound` and the
re-enable gate are unnecessary**: B1c reduces to `retained < limit` for fresh admission, joins into an
existing group are unaffected, and parking/enable queue/fairness (B1d) largely disappear. This module
would then shrink to that one rule. It is kept here, tested, because the alternative needs it and the
choice is the owner's.

## Owner questions (the answers decide the shape of B1d)

- **O701 — After a reduction of `iteration_concurrency` (including to 0), may already admitted work be
  paused until simultaneous execution fits the new limit?** A (recommended: no pause, released
  contract): admitted work runs to completion, only new admissions stop; the tenant may exceed the
  new limit for a while. B (pause): the model above.
- **O702 — May a tenant execute above its configured limit, and by how much?** A: by the admitted
  work that survives a reduction. B: by that plus one drain lane.
- **O703 — At limit 0, must accepted compensation/reconciliation keep running?** D701 says yes.
- **O704 — Is a parked group's retained unit counted against the tenant quota (D703), and is serial
  drain above the bound acceptable?** D703 preserves today's flat semantics.

## Obligations on the adapter (not provable in the pure model; none is wired or enforced yet)

- **G701 — A holder with no claimable work must park or retire.** This covers a holder suspended only
  on durable waits (otherwise at `limit = 1` a holder waiting for an event only a parked group can
  produce deadlocks), a holder refused `OverBound`, and a lane holder whose reserved drain is finished
  but has only forward work left (otherwise the lane is held forever and every other drain strands).
  `park` is never limited by the limit; whether it is *used* is the adapter's duty (B1d/B1e).
- **G702 — Which group parks, the tenant enable queue and fairness** belong to B1d. A queue must
  not invert the lock order (G704).
- **G703 — Drain throughput is unmeasured.** C2's 100,000-instance workload must measure it at policy
  zero before any capacity claim; no ceiling is accepted here.
- **G704 — Counters are a projection with a lock position.** They are updated with the group's
  enablement in one transaction and reconciled against actual group rows after crash/restart; old
  flat writers are fenced (C0). Lock position: B0 (`capacity-lock-graph.md`) places
  `tenant_iteration_usage` after chain, run and scheduler guards and a future group lock before the
  run guards, so a "tenant guard then group" step (a queue scan) would invert it and needs its own
  review. Possession (`capacity.rs`: Vacant/Held/Retired) and enablement are separate axes: a
  retired group must also be retired in the counters. Where enablement is stored is a C decision; the
  `CapacityGroup` header is deliberately unchanged.
- **G705 — Work class is derived, not asserted.** `CapacityWork::ReservedDrain` comes from the
  instance's consumed-reservation state, never a caller flag (B1b).
- **G706 — Only root/top-level admissions call `allocate_group`; nested descendants join an existing
  group.** A descendant allocating a fresh group at `limit = 1` while its ancestor is retained is the
  ADR-0030 §14 deadlock (suspended ancestors consuming the capacity descendants need). Wiring needs a
  nested limit-1 test.
- **G707 — No lane→forward promotion.** A lane holder that becomes eligible when the limit rises must
  park and `enable`.
- **G708 — A parked group blocks fresh admission** (D703), as in flat: at limit 1 a parked waiter
  whose producer is another iteration needing a fresh unit starves. Nested joins avoid it inside one
  group; across unrelated admissions it is today's flat behaviour.
- **G709 — Wake-ups.** `park`/`retire` free capacity and return nothing about who can now enable; the
  adapter must scan the enable queue in the same transaction or the wake is lost.
- **G710 — Hot row.** Every forward claim reads the tenant counters; today the tenant row is touched
  only at admission and release. Measure contention in C2.
- **G711 — Telemetry.** `NoRoom/OverBound/DrainLaneBusy/…` need a closed, bounded-cardinality
  vocabulary (and audit semantics) when wired.
- **G712 — The lane has no lease.** A lane holder that never parks starves other drains.

None of G701–G712 has a failing check yet (AGENTS rule 21): they are open gaps in `research.md` and
each wiring task must add its test before ticking anything.

## What the tests cover

Boundary scenarios derived from the accepted text: capacity 2→1 (both holders survive, forward claims
refused until one parks, the parked group refused while over the bound), policy zero (no allocation,
forward enable or forward claim; a parked group drains through the lane; the next group takes the
lane after it parks or retires), ordinary room before the lane, the lane adding exactly one executing
group, no lane promotion, claims and transitions on counters that hold no such group (refused without
change), `restore` rejecting a wrapping sum, and operations from the `(u64::MAX, u64::MAX−1, 1)`
projection. The property test is a *reference model* (a recomputed multiset of enablements, arbitrary
restored starting states, arbitrary and false stated enablements, limits 0–4): it checks bookkeeping,
refusal-without-change and the invariants; it re-uses the rules' predicates, so the **policy** is
pinned by the scenario tests, not by the oracle.

Not covered (pending B1d/B1e/B2/C): offers and recalls, stale transfer epochs, cancellation policies,
continuation while parked, mixed-version fencing, the real lock graph, any storage.

## Evidence

- Red first: the same suite against a refuse-everything stub: 13 failed, 0 passed. The review-driven
  additions (counter-checked `claim`, extreme projection, reference model from restored states) were
  red first too: 3 failed before the `claim` change.
- Green: 17 passed. Eight manual mutations, each killed by at least two tests: enable room `>=`→`>`;
  `OverBound` `>`→`>=`; lane not released on `park(Drain)`; allocation bound `>=`→`>`; lane always
  granted; `claim` without the counter check; drain ignoring ordinary room; `retire(Parked)` without
  a parked group.
- `cargo test -p sagat-domain --locked`: 561 passed, 0 failed, 0 ignored across 40 targets
  (544 baseline + 17 new). `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, architecture (24 crates, no cycles) and agent configuration pass.
- Reviews (read-only agents given the code and the contract, not this conclusion): correctness — no
  confirmed defect; found the `claim` doc/code mismatch, the "limit zero" wording, the lane-holder and
  wake-up gaps (G701, G709) and the oracle-independence caveat. Architecture — found the tension with
  the released reduction contract (O701), the missing descendants-join obligation (G706), the lock
  position (G704), the "serialised" overstatement, the two senses of "reserved" and the unregistered
  ids. All are addressed above or recorded as open.
