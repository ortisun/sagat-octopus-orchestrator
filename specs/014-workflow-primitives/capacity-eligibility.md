# Capacity eligibility after a policy reduction (B1c) — resolved by owner decision O701

Status: **group eligibility decided; dormant read-only discovery implemented, no production group caller.**
[Bounded discovery evidence](./capacity-discovery.md) covers B1d.1 only. The separate 0.3
[explicit zero tenant policy](./zero-iteration-policy.md) implements flat admission configuration
without enabling groups. The owner answered O701 on 2026-10-26: a reduction of
`iteration_concurrency` (including to 0) does **not** pause admitted work. The pause-based model that
was built first (commit `d917a54f`, withdrawn in the next commit) is therefore not needed and was
removed; it is kept only in history. This file records the decision, what it makes unnecessary and
what remains for B1d/B1e/C. Refs: T014-014..017, T014-042, ADR-0030 §§12–19, ADR-0037,
`data-model.md` ("A policy reduction stops new admissions above its new ceiling but does not kill
accepted work"), `docs/runbooks/workflow-primitives.md`, `docs/owner-decisions.md` (2026-10-26).

Ids come from the range **D701–D799, G701–G799, O701–O799** reserved for the nested-capacity lane
(highest ids on `develop`: D602, G526, O501); they are registered in `research.md`.

Design inputs that are not on `develop` (read them with `git show`): branch
`handoff/016-project-completion-20261024` @ `cc1830a2c19e`, directory `tasks/project-completion/`:
`capacity-group-lifecycle.md` (A0/A1), `capacity-lock-graph.md` (B0), `capacity-transaction-entry.md`
(B1a), `nested-capacity-review.md` (three rejected whole-protocol cycles) and `todo.md`.

## Decision

- **D701 (withdrawn)** — a drain lane for reserved drain above a bound. Moot: admitted work, compensation
  included, is never throttled by the tenant limit.
- **D702 (withdrawn)** — pausing admitted work while `enabled > limit`. Rejected by the owner (O701).
- **D703 (confirmed)** — a retained group counts against fresh admission until it settles, exactly as a
  flat iteration counts in `tenant_iteration_usage.active_iterations`. A group that is idle on a durable
  wait keeps its unit.
- **D704 — The only tenant-level capacity gate is fresh admission: `retained < limit`.** Zero admits
  nothing; a reduction below current use blocks further admission until use falls below the new value
  and never touches admitted work. This is what the flat code and the runbook already do, so the
  nested design adds **no new tenant counter, no enabled/parked state, no drain lane and no enable
  queue.** Execution inside one group is serialised by *possession* (A1, `capacity.rs`), not by the
  tenant limit.
- **D705 — A nested descendant joins an existing group; it never allocates one** (G706). Joining
  charges no tenant unit, so a suspended ancestor cannot starve its descendants of capacity (the
  ADR-0030 §14 deadlock) at any limit, including 1. Joining is still bounded by the lifetime
  `maxInstances` budget and by local scope slots, which are distinct from the tenant check.
  Only a root/top-level admission allocates a group, through the existing admission check.

Consequences the owner's answer implies (stated so they can be corrected): O702 — a tenant may run
above a reduced limit for as long as its admitted work lasts, as flat iterations do today;
O703 — at limit 0 accepted compensation/reconciliation continues, because admitted work continues;
O704 — a parked/idle group's unit counts against the quota (D703) and no serial drain lane exists.

## What this removes from the plan

`todo.md` B1c ("current forward eligibility versus reserved drain, reduction and policy 0") is closed
by D704. In B1d, the tenant-wide parts disappear (enable queue, cross-group fairness, "which group
parks", lane lease and hot tenant-row reads). Reserved drain still matters for the **lifetime** budget
(ADR-0030 §13 reserved derived-instance units survive exhaustion and reduction); that is domain
behaviour that already exists and is unrelated to the tenant concurrency limit.

## What remains (open obligations, none enforced yet)

- **G706 — Descendants join, they do not allocate.** Wiring must add a nested limit-1 test (ancestor
  retained and idle on a wait, descendant must still make progress) and a test that a descendant path
  never reaches the fresh-admission check.
- **G708 — A retained group blocks fresh admission.** At limit 1 a group idle on an event whose
  producer needs a fresh unit in an unrelated admission starves. This is today's flat behaviour.
- **G711 — Telemetry/audit.** The refusal at the fresh-admission check keeps its existing typed code;
  group join/vacate/acquire need a closed, bounded-cardinality vocabulary when wired.
- **Still B1d/B1e at group scope, now without any tenant queue:** how a quiescent holder proves it may
  `Vacate` under its own run fence; how a member with claimable work is discovered and `Acquire`s the
  vacancy from its own run's transaction, never holding two existing run locks (B0/ADR-0037 order:
  chain, own run, then the group header `FOR UPDATE NOWAIT`); cancellation policies per member;
  continuation while the member's chain is `CONTINUED`; stale possession epochs after restart; who
  creates groups and members (the runtime role has no INSERT on them today) and the audit of that.
  Fairness among members of one group is open and may be unnecessary (G713).

## Evidence

- The withdrawn model (`d917a54f`) was red first (13 failed against a stub; 3 more after review),
  17 green, eight named mutations killed, domain suite 561 passed, workspace Clippy/fmt/architecture/
  agent configuration green, `contract_developer_experience` 8 and `contract_workflow_primitives` 47
  passed. Two read-only reviews found no code defect; their architecture review is what exposed the
  conflict with the released reduction contract and led to O701. That evidence is history, not a
  delivered capability.
- At that removal checkpoint `crates/` was identical to `origin/develop`; the gates were re-run on that tree (see
  `validation-report.md`).
