# Implementation Plan: Compensation safety

**Branch**: `feature/037-compensation-safety` (isolated implementation checkout) | **Date**: 2026-11-01 | **Spec**: [spec.md](spec.md)

**Status**: delivery-1 design and implementation approved by owner on 2026-11-01; delivery-1 implementation locally validated; US04 remains open.
Prepared on `develop@6b6f0fa4bb7b622c960e9d149a4b78785ad0647b`. Existing work is preserved;
the original checkout and its feature-014 pointer are preserved. The approved implementation runs in a separate feature-037 worktree; no commit or merge has been made.
The owner approved S1-B/S2-B and the B02 wording patch; approval of that direction is not evidence
that this new detailed design or runtime has passed validation.

## Summary

When a payment or another external action loses its response, the platform must keep asking what
happened instead of repeating or undoing it blindly. The people operating the integration must
still see and decide that unresolved work after cancellation or a restart. This reduces the risk
of duplicate effects and misleading “finished” outcomes.

Today, some ordinary activities retry after a lost response, a negative lookup may trigger another
attempt, and explicit sagas may cancel a waiting human decision while collection/loop compensation
keeps it. The new approach aligns those cases on retained review using the existing execution and
operator tools. Its main cost is more unresolved work for operators; it adds no service or broker.

Deliver the conservative recovery defaults first (US01–US03). Unknown original effects consult
or wait for an operator; unknown compensation outcomes wait for an operator. Cancellation and
deadlines preserve that obligation. Known compensation failure may use the already-published
explicit-saga `fail` policy, after independent work settles. Reuse the existing engine, journal,
review APIs, PostgreSQL transaction and worker failure wire.

US04 remains a separate evidence/design gate: no provider has been selected or qualified. The
first delivery enables no defensive compensation of unknown originals and no repeat of unknown
compensations. It also conservatively treats `not_applied` as review because the current wire
cannot establish final absence. This trades automation for safety; it does not remove the approved
eventual qualification capability. Feature 037 cannot be marked complete while US04/SC-037-004 is open.

## Technical Context

**Language/Version**: Rust 1.98.0, edition 2024 (`rust-toolchain.toml`, root `Cargo.toml`).
**Primary Dependencies**: existing Tokio, Axum/tonic and SQLx adapters; no new runtime dependency.
**Storage**: PostgreSQL journal, steps, outbox and audit. No new table or external service in delivery 1.
**Testing**: domain replay tests; real PostgreSQL integration/fault tests; REST/gRPC contracts; shared six-SDK conformance; Studio checks.
**Target Platform**: existing Linux container deployment and disposable repository test infrastructure.
**Project Type**: Rust modular monolith with six SDKs and React Studio.
**Performance Goals**: no new throughput/SLO promise. Waiting reviews must not consume worker leases or spin the scheduler.
**Constraints**: preserve exact execution-instance fencing, atomicity, least privilege, secret-free audit, immutable versions and independent-compensation ordering.
**Scale/Scope**: explicit sagas plus collection/loop paths, including nested/path-local execution and continuation boundaries. No known users or selected provider; fresh installation approved.

## Constitution Check

This checks the proposed design against the working Constitution 3.0.0 candidate, not runtime compliance.
The approved B02 text is applied locally; formal PR adoption of Constitution/ADR-0076 remains pending.

| Principle | Design response and required evidence |
|---|---|
| I durability | Keep commands/reducers pure; existing RunTx/PathExecutionTx persist state, next work, journal/outbox and required audit together. Crash/duplicate and rollback tests are required. |
| II boundaries | Change existing domain policy and adapters only; no new service, mapper layer or qualification framework. Architecture check remains mandatory. |
| III effects | Unknown precedes retry/cancel/fail; no exception is enabled without qualification. Explicit known failure drains independent compensation; human success retains provenance. US04 remains open. |
| IV compatibility | Fresh coordinated installation, unchanged wire shape but deliberately changed semantics. No mixed old/new engine deployment or in-place reinterpretation of live runs. All six clients tested. |
| V isolation | Existing permission and exact-instance checks remain; review/cancel races cannot grant SagaOverride or cross tenants. No new secret use. |
| VI evidence | Typed existing review/error surface plus proposed bounded failure code; journal/audit and bounded trace/metric reasons tested. No tenant or run metric labels. |
| VII delivery | PostgreSQL remains authority; no broker dependency added. Lost hints and duplicate reports cannot schedule a second decided action. |
| VIII tests | Red-first behavior tests, real PostgreSQL/runtime-role, fault, contract/client and operator demos precede delivery acceptance. Existing 94 tests are baseline only. |

No architectural exception is proposed. Full-feature conformity is pending US04 design and all runtime evidence.

## Design decisions

Decisions D1806–D1809 in [research](research.md) are proposed, not new approved product choices.

1. **Server classification.** Every leased `Activity` can affect an external system, including SDK,
   plugin, inline/script and managed-database handlers. Missing/false `highRisk`, language, binding
   and origin do not prove purity. Engine-owned expression/control transitions keep their existing
   internal semantics. Child workflow effects are resolved by their child lifecycle, not inferred
   pure. No user-selectable purity flag is added.
2. **Unknown wins.** Cover the complete uncertainty taxonomy, with source-specific handling: attempt ambiguity
   uses the existing lease/timeout/OUTCOME_UNKNOWN cases; output storage refusal and reconciliation failure
   use their own paths. `leaves_outcome_unknown` classifies retained recovery obligations; do not
   blindly treat every storage/reconciliation code as a fresh worker attempt failure. Evaluate uncertainty before cancellation, attempt exhaustion
   or `fail` in both root and scoped attempt paths. Forward activities with a declared reconciler
   use it; without one they review. Unknown derived compensation reviews immediately; no derived
   compensation-reconciler facility is invented. Lookup handlers may poll within existing budgets;
   failed/unknown lookup ends in review rather than recursive reconciliation.
3. **Negative lookup.** `Applied` means a final authoritative applied result under the adapter
   contract. `NotApplied` cannot prove that a late operation will never apply. In delivery 1 it
   concludes as review with proposed `RECONCILIATION_NOT_FINAL`, even during stop or with retry
   budget left. `Unknown` remains review. No new proto field or guessed protocol version is added.
   Automatic final-absence handling needs the later evidence/contract gate; authorized manual
   resolution remains available now.
4. **Recovery obligation.** Cancellation/drain must retain operator/reconciliation waits that
   represent unresolved effects and every compensation review. A shared predicate used by root
   stop/planning paths avoids ordinary cancel substituting for privileged effect resolution.
   Scoped draining must enforce the same condition under its exact path/fence. Other idle forward
   waits retain their normal cancellation behavior; do not freeze every timer or approval.
5. **Compensation eligibility.** Delivery 1 plans compensation only for succeeded effects, including
   explicitly human-resolved success with provenance. Unknown failed/cancelled entries cannot
   become defensive compensation through either planner. Reducers validate the same eligibility,
   not just planner output. A review request to retry **any unknown effect**, original or compensation, is refused with
   the existing typed conflict. Possessing SagaOverride does not establish what happened. First
   use existing authorized Resolve to record an outcome and reason; a failed outcome may end the
   step/run, so this proposal does not promise a new in-place retry-after-resolution operation.
   A later explicit new-run decision follows existing rerun rules and must not imply safe automatic
   repetition. Existing audited resolve/skip choices remain; skip never means successful undo.
   Known-failure retries remain available while forward work is allowed. This deliberately restricts
   the current unknown-original privileged Retry shortcut and is part of the requested design approval.
6. **Honest terminality.** Preserve the existing independent/dependent ordering. A known failed
   compensation under explicit `fail` does not end the run while independent compensation can
   run or any compensation awaits review. Suppress blocked dependents before terminalization.
   No post-terminal compensation queue is added, and cancel/deadline cannot turn uncertainty
   into `CANCELLED`, `PARTIAL` or `COMPENSATION_FAILED` merely to finish a run.
7. **Refusal evidence.** An authenticated state-conflict refusal rolls back the execution
   transaction and returns the existing typed conflict. It writes no success audit, idempotency
   success record, state transition or scheduling outbox item. Existing request tracing/typed error
   plus the durable unresolved history explain it. No new durable conflict-refusal audit is promised.
   Existing authorization-denial auditing and its failure policy remain unchanged. Accepted operator
   decisions still require their existing atomic audit; failure to commit that audit refuses the
   decision. This avoids inventing a second transaction or falsely recording `saga.step_retried`.
8. **Handler finality.** Existing CompleteStep is a final-result assertion. An accepted/pending
   provider response must instead produce `OUTCOME_UNKNOWN`; `retryable=true` cannot override
   uncertainty. SDK errors retain that code. The server cannot inspect a provider's truth or
   prevent a dishonest handler from claiming finality. Conformance proves protocol behavior,
   not a bank/venue guarantee. Human resolution retains actor/reason and does not certify a provider.

See [data model](data-model.md) and [contract delta](contracts/README.md) for transition tables.

## Project Structure

```text
specs/037-compensation-safety/
  spec.md                 approved direction; detailed delivery pending
  research.md             evidence and proposed technical decisions
  plan.md                 this design and scope split
  data-model.md           existing entities and transition invariants
  contracts/README.md     semantic delta and client obligations
  quickstart.md           proposed validation commands and demonstrations
  tasks.md                unchecked delivery-1 tasks plus US04 evidence gate
  analysis.md             requirement-to-task coverage and open gates
  validation-report.md    actual results and limitations
```

Implementation stays in `crates/domain/src/execution/{command,state_effects,sagas,scoped_sagas,
plan,state,path_state,event}.rs`, existing application worker/run services and PostgreSQL execution
adapters. Tests stay with sagas/workflow-primitives and SDK conformance. Paths in tasks identify
smaller ownership units. No parallel engine or governance tooling is introduced.

## Delivery sequence and acceptance

1. **Foundation + US03**: classify uncertainty, fail closed on negative lookup, remove defensive
   candidates and reject direct retry of every unknown effect in commands and reducers. First checkpoint:
   root and exact-path domain replay with refusal and retained review.
2. **US01**: preserve recovery obligations across stop/deadline, path drain and restart. Prove
   current authority, race winners, required audit and human provenance on real PostgreSQL.
3. **US02**: prove known-failure `fail`, independent progress and terminal quiescence. Preserve
   quotas and distinguish skipped/failed compensation in existing views.
4. **Delivery-1 convergence**: six-client contract tests, fault tests, operator demonstration,
   documentation, architecture/CI gates and fresh-install cutover. No provider qualification claim.
5. **US04 gate**: select one provider/capability and immutable adapter artifact, obtain authoritative
   guarantees and adapter evidence, then review exact qualification identity/authority/revocation
   and worker protocol. Generate implementation tasks only after that design is approved.

US03 precedes US01 because preserving review is ineffective if the failure path never creates it.
Each test/implementation pair is one reviewable change; do not merge knowingly failing intermediate
states. Shared domain files are sequential. Client harness additions can be parallel after the
contract checkpoint. All unchecked tasks mean work not yet performed.

## Effort, resources and timing

This is a substantial engine-safety change, not a documentation-only runtime patch. The proposed
work has 32 delivery tasks and two later evidence/design tasks. One engine implementer plus an
independent reviewer can execute the sequential core; client harness work may be split after its
shared contract stabilizes. No extra production service or database is added. Verification needs
local Docker/PostgreSQL fixtures and all six SDK toolchains, with at most two heavy builds and
private targets. Retained reviews increase storage and operator work; no usage data supports a
numerical cost estimate yet.

A rough, low-confidence planning estimate is **3–6 working weeks** for one dedicated engineer,
with an independent reviewer available at the three checkpoints and all six toolchains available.
This is an estimate, not measured throughput or a release commitment; it excludes owner/provider
waiting time. Engine/replay/authority work and cross-language/fault validation dominate the range.
Re-estimate after the first root/path domain checkpoint using actual turnaround. US04 has no
credible duration until a named provider supplies evidence; it is excluded from this estimate.

For retained reviews, use a parameterized cost model rather than an invented adoption forecast:
`retained bytes ≈ unresolved reviews × measured bytes per review + shared/index overhead`.
Backlog grows when incoming unresolved cases exceed the operator's observed resolution rate.
Measure those rates in an operator rehearsal; do not assert a handling time from code. Existing
run-journal and audit retention are explicitly not built (`docs/runbooks/execution-engine.md`,
known gaps G28/G182), so the runbook must state that absence and describe operator-owned backup,
export and responsibility transfer instead of naming a nonexistent automatic retention control.

The delivery exam measures isolated backlogs of 1, 100 and 1,000 synthetic unresolved cases on a
stated fixture/hardware profile: database bytes (including indexes), scheduler wakeups/queries,
active leases, and review-list latency/query plan. These are experimental sizes, not production
capacity claims. Stop the exam and investigate if review holds worker capacity, causes a busy loop,
erases evidence under quota, or exceeds the test machine's agreed disk/resource budget. Record
latency and storage results without inventing an SLO. Production/operator budgets must be selected
from measured results; this is validation and operating guidance, not a new automatic admission or
retention gate in the product.

## Qualification gate: alternatives, not a speculative implementation

| Alternative | Benefit | Cost/risk | Disposition |
|---|---|---|---|
| Deny all exceptions for delivery 1 | Smaller scope than qualification machinery; safe defaults | More manual review; US04 remains incomplete | Proposed now |
| Static deployment qualification config | Simple initial loading | Replica divergence, revocation/restart races, no atomic authorization/audit | Reject as runtime authority |
| PostgreSQL immutable qualification + current scoped authority | Publication/use can share durable truth | New identity, permissions, wire and revocation design without a real adapter | Candidate after evidence gate |

The later design must bind provider/capability version, authoritative evidence digest, adapter
artifact, exact activity/task kind, stable effect identity, approver, tenant/namespace and permitted
action. No original-effect automatic-repeat action. Check authority at publication and actual use;
revocation cannot recall provider I/O. Preserve factual reports from already-authorized work without
granting another attempt. Existing subject binding is not binary attestation. Four-eyes approval,
provider trust roots, retention and protocol negotiation remain explicit design questions, not
silently added requirements. No table, API, permission or use-begin RPC is approved here.

## Risks, operation and migration

| Risk | Impact | Mitigation / acceptance gate |
|---|---|---|
| More reviews and long-lived retained state | Operator effort and database retention grow; no workload measurement available | Existing durable queries, bounded metrics and runbook ownership; measure representative backlog without inventing capacity/SLO |
| Generic worker failure falsely implies no effect | Unsafe retry despite new policy | Contract states known failure versus uncertainty; all SDKs preserve reserved code; real adapter acceptance remains separate |
| Output storage rejection after provider effect | Success lost then repeated | Exercise storage refusal, quota, lease loss and reconciliation failure through shared uncertainty rules; feature-009 secret rejection remains a recorded delivery-1 limitation |
| Root/path divergence or continuation escape | Review disappears or effect repeats | Same semantic matrix across root, scoped/path-local and continuation; held-lock races and replay |
| Old engine reads new journal semantics | Review lost or reducer rejects new failure/result pairing | Fresh coordinated install; do not roll back into live 037 data or mix engine versions |
| Provider acceptance mistaken for finality | False compensated result | CompleteStep contract, pending-as-unknown cases, truthful manual provenance; no provider guarantee inferred |

Fresh installation is authorized, deletion of existing developer data is not. Use a new isolated
installation and pinned matching server/SDK artifacts; do not run older engines against its data.
Before exposure, rollback removes only this unexercised build/deployment. After any effect has been
sent, pause new admission, preserve the database/journal and operator ownership, and fix forward
or perform an explicitly reviewed reconciliation. Git revert cannot undo provider effects. No
migration of legacy live runs, automatic restart replay or re-triggering original requests is planned.

Follow-up under `fail` belongs to the integration operator. The runbook must point to retained
steps/history, identify actual retention controls or their absence, and explain operator-owned
retention and external responsibility transfer where needed.
No automatic critical-integration activation gate, active post-terminal review queue or regulatory
retention promise is introduced.

## Complexity Tracking

No constitutional exception or new infrastructure is requested. US04 remains visibly incomplete;
its absence is a delivery boundary, not a claim that a mock or registry proves provider safety.


### Owner-confirmed secret-output boundary (2026-11-01)

The owner retained feature-009's final `SECRET_IN_OUTPUT` contract for delivery 1. The successful
report-refusal work above covers storage ceilings/quota, not every rejection source. Ordinary
secret-bearing completion can still fail finally under 009 rather than create recovery review;
there is no universal output-rejection preservation guarantee. Existing managed-database handling
and secret suppression remain. A later source-aware change requires an approved cross-feature
contract update; see [owner decisions](../../docs/owner-decisions.md).
