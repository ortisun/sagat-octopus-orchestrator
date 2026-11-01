# ADR-0076: Compensation failure and uncertain effects require explicit recovery authority

**Status:** PROPOSED — product direction approved 2026-11-01; delivery-1 implementation in verification
**Decided by:** Owner approved S1-B/S2-B; this wording is an agent-prepared adoption proposal
**Refines / supersedes:** Refines ADR-0004, ADR-0005, ADR-0010 and ADR-0030; no complete supersession
**Enforced by:** Delivery-1 implementation and tests are in verification. Conformity requires recorded feature-037 acceptance; US04 remains open and no provider is qualified.

## Context

The Constitution's mandatory human intervention conflicts with explicit saga `fail` and with
root cancellation abandoning some pending compensation reviews. Collections/loops retain their
review. Both planners defensively compensate unknown effects without the qualification described
below. Existing ambiguity handling can retry ordinary forward work without lookup, while a
compensation has no platform-derived reconciler. A stable key alone does not prove provider safety.

The owner approved the product direction, not runtime correctness or a financial release.
[Feature 037](../../specs/037-compensation-safety/spec.md) defines the target requirements and
[research](../../specs/037-compensation-safety/research.md) records approval and design dependencies.

## Decision

1. Compensation failure MUST default to authorized, audited human review. Cancellation/deadline
   MUST preserve review across explicit sagas and scopes; O381 is answered accordingly. No new
   scope policy fields are introduced; O380 remains deferred.
2. Explicit published `fail` MAY end an explicit saga with `COMPENSATION_FAILED`. The platform
   retains terminal/step/history evidence, not a promised active post-terminal review queue.
   This terminal policy is for known permanent/non-retryable failure; it MUST NOT terminalize an
   unknown/accepted/pending compensation instead of consultation/review or a qualified safe repeat.
   It MUST NOT call unresolved effects compensated. Independent compensations remain eligible;
   failed dependencies MUST prevent out-of-order execution. Terminal failure MUST wait until all
   eligible independent planned compensations settle and blocked dependents are terminally suppressed;
   a terminal run MUST have no active, runnable or review-waiting compensation.
3. Unknown external effects MUST default to consultation/review regardless of risk labels. Unknown
   original work MUST NOT repeat without consultation or an authorized human resolution. Stopping
   the workflow MUST NOT establish that an effect never happened.
4. Defensive compensation and repeat of an unknown compensation outcome require an explicit,
   version-pinned, authorized qualification. It MUST include authoritative provider capability
   evidence, accountable approver and adapter tests for duplicate, absent, applied, late and
   timeout-after-acceptance outcomes. Tests and an author's document alone are insufficient.
5. Automatic/handler compensation success MUST mean authoritative final business outcome.
   Accepted/pending/unknown is not success. Existing authorized audited human success resolution
   MAY yield `COMPENSATED` with retained human provenance; it MUST NOT be presented as provider
   confirmation/financial settlement or qualify an adapter for later exceptions. Original and compensating operations completing late MUST both be covered.
   An adapter unable to represent unresolved results safely MUST NOT be qualified.
6. Qualification checks MUST validate reference presence, integrity, pinning, scope and current
   authority; they do not establish truth of provider guarantees. Invalid qualification MUST
   preserve consultation/review. SDK integrations and plugins have identical obligations.
7. Current atomicity, tenant boundary, fencing, retained evidence and immutable versions MUST remain.
   Exact contract, effect classification, publication/use authorization and revocation design remain
   required before implementation approval; this decision invents no already-implemented gate.

## Considered options

1. **Defaults plus qualified explicit exceptions** — approved direction: keeps simple safe defaults
   and reusable recovery where evidence justifies it, without requiring another service.
2. **Intervention/consultation always** — simpler, but removes the existing intentional terminal-failure
   choice and cannot exploit a provider's safe definitive cancellation. Remains the fallback when
   an exception cannot be qualified.
3. **Ratify current behavior unchanged** — fewer changes, but preserves timing/scope differences and
   unsupported provider assumptions. Rejected as the target.

## Consequences

Explicit-saga cancellation behavior changes. Ordinary unknown effects may now wait rather than
retry; operator load and retained state can increase. Terminal `fail` transfers follow-up to the
operator's use of retained evidence. Operational guidance must identify evidence lookup, operator
responsibility and configured retention; operators determine adequacy/external ownership for their
use case. This is not a new automatic activation gate or critical-integration product taxonomy. No throughput or cost measurements exist.
No new broker/database/certification service is proposed.

Fresh installation/no legacy upgrade is approved, but cutover cannot reinterpret live definitions
or replay effects. Plans must enumerate schema/protocol/client impacts and rollback boundaries.
Code rollback cannot undo provider effects. Keep previous ADR rationale and dated research evidence;
this is a refinement of safety conditions, not a rewrite of historical implementation claims.

## Evidence

[Baseline report](../../specs/037-compensation-safety/validation-report.md): 60 saga and 34 scoped
compensation domain tests passed on 2026-11-01, before this target was implemented. No provider,
SQL/API/SDK/fault evidence or performance benchmark was produced. Named characterization tests
include cancellation giving up on root review, scoped hold, defensive compensation and explicit fail.
Those passing tests do not enforce this new decision; required acceptance is in feature 037.

## Delivery-1 implementation note — 2026-11-01

The approved conservative slice implements the no-exception defaults in the feature branch and is
now in verification. It preserves review on stop/deadline, refuses direct retry of an unknown
original or compensation, and requires an outcome-bearing human resolution with retained
provenance. That resolution is terminal for the affected outcome; delivery 1 promises no in-place
retry after resolution. Qualification machinery remains absent and US04 remains open. This note is
implementation status, not acceptance or provider proof; the original proposal context and dated
baseline evidence above remain unchanged.
