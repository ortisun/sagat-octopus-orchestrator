# Feature Specification: Explicit compensation failure and uncertain effects

**Feature Branch**: `feature/037-compensation-safety`
**Created**: 2026-11-01
**Status**: delivery 1 (US01–US03) approved on 2026-11-01; implementation locally validated. US04 remains open.
**Input**: Owner approved B02 S1-B and S2-B on 2026-11-01: human review by default,
explicit terminal compensation failure, and qualified exceptions for uncertain effects.

## Business context

Workflow owners need to know whether work stopped, an external effect was undone, or somebody
still needs to decide what happened. Payments, order execution and reusable integration steps
can all lose a response after a provider accepted a request. Neither cancellation nor a successful
request submission proves that the external effect is resolved.

The expected value is consistent, explainable recovery across explicit sagas, collections and
loops. This feature changes the recovery rules of features 003/014; it does not introduce another
workflow engine or a plugin marketplace. There are no known users; a fresh installation is approved.
No bank or venue capability, production cost or usage metric is assumed.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Preserve a decision owed to an operator (Priority: P1)

As an operator, I want cancellation and deadlines to preserve compensation review so that the
timing or structure of a workflow cannot silently discard a decision about an external effect.

**Why this priority**: current explicit and scoped workflows differ at this boundary.
**Independent Test**: fail a compensation under the default policy, then cancel or expire the run;
verify the same unresolved review survives before and after restart in each supported scope.

**Acceptance Scenarios**:

1. **Given** a compensation awaiting review, **When** cancellation or deadline occurs, **Then** it
   still awaits an authorized, audited decision; its obligation is not cancelled by that event.
2. **Given** cancellation already started compensation, **When** compensation exhausts its budget,
   **Then** the same review rule applies as when compensation started before cancellation.
3. **Given** two authorized decisions race with cancellation, **When** one decision commits,
   **Then** the other cannot overwrite it; the record identifies the decision and actor.
4. **Given** an authorized operator resolves a reviewed compensation as succeeded, **When** the
   resolution commits, **Then** the existing manual contract may yield `COMPENSATED`, retaining
   its human actor, justification and decision provenance; it is not provider-confirmed settlement.

### User Story 2 — Choose an honest terminal failure (Priority: P1)

As a workflow author, I want an explicit failure policy when active operator review is not the
desired recovery mechanism, without telling consumers that the effects were resolved.

**Why this priority**: the existing `fail` choice needs an unambiguous, approved meaning.
**Independent Test**: publish an explicit saga with `fail`, exhaust a compensation and inspect the
terminal outcome, retained failed/cancelled steps and independent compensations.

**Acceptance Scenarios**:

1. **Given** explicit `fail` in the published version, **When** compensation has a known permanent
   failure after its retry policy (or a known non-retryable failure),
   **Then** the run can end `COMPENSATION_FAILED`, retaining unresolved-effect evidence and no
   promise of an active review queue or automatic remediation after terminalization.
2. **Given** independent and dependent compensations, **When** one fails under that policy,
   **Then** independent compensations remain eligible and blocked dependents do not run out of order.
   The run becomes terminal only after every eligible independent compensation settles and blocked
   dependents are terminally suppressed; no active, runnable or review-waiting compensation remains.
3. **Given** no explicit choice, **When** compensation fails, **Then** human review remains the
   default; neither cancellation nor a deadline silently selects `fail`.
4. **Given** explicit `fail` but an accepted, pending or unknown compensation outcome, **When**
   its attempt budget expires, **Then** unknown-outcome handling takes precedence: consult/review
   or a qualified safe repeat; do not automatically terminalize it as `COMPENSATION_FAILED`.
5. **Given** an author considering `fail`, **When** they read its operational guidance, **Then** it
   explains how to locate retained failure evidence, who owns follow-up and which configured
   retention applies, without promising a live post-terminal queue or regulatory sufficiency.

### User Story 3 — Keep uncertain effects unresolved until a safe action is known (Priority: P1)

As an integration owner, I want a lost answer to trigger consultation or review, even when an
activity was not labelled high risk, so that a missing label cannot cause unsafe work.

**Why this priority**: ordinary failure fallthrough currently differs from the conservative rule.
**Independent Test**: lose an original-effect or compensation response for an unqualified integration;
verify no unauthorized repeat, defensive compensation or successful resolution follows.

**Acceptance Scenarios**:

1. **Given** an unknown external outcome and no qualification, **When** the attempt ends or a scope
   drains, **Then** consult the provider when supported, otherwise preserve uncertainty for review.
   Missing or false `highRisk` does not bypass this rule.
2. **Given** a lookup answers “not found” while the original request may still finish, **When** the
   answer is processed, **Then** it does not by itself authorize repeat or declare resolution.
3. **Given** a compensation request was merely accepted or remains pending, **When** its handler
   reports progress, **Then** the effect remains unresolved and cannot yield `COMPENSATED`.
4. **Given** a lookup or human review is unresolved, **When** cancellation/deadline occurs,
   **Then** uncertainty remains; stopping is not evidence that an effect never happened.

### User Story 4 — Use a qualified recovery exception (Priority: P2)

As an integration owner, I want to reuse a recovery operation whose provider guarantees justify
a narrower exception, with the same conditions for SDK-owned integrations and deployable plugins.

**Why this priority**: conservative recovery is useful independently; exceptions require provider evidence.
**Independent Test**: authorize a versioned qualification for a controlled adapter, exercise every
declared failure scenario and reject absent, mismatched or unauthorized qualifications. A simulated
adapter does not qualify a real provider.

**Acceptance Scenarios**:

1. **Given** authoritative evidence plus adapter tests for a specific provider capability version,
   **When** an authorized author publishes an explicit qualified choice, **Then** only that
   qualified action is eligible; other actions and versions retain consultation/review defaults.
2. **Given** defensive compensation, **When** the original effect did happen, never happened or
   completes late, **Then** the declared contract prevents an unresolved late effect from being
   labelled compensated, including a late completion of the compensation itself.
3. **Given** an unknown compensation outcome, **When** a qualified repeat is selected, **Then** the
   provider guarantee makes repetition safe under the same stable effect identity. Without that
   guarantee, consult/review; this does not authorize blind repetition of the original effect.
4. **Given** missing or no-longer-authorized qualification, **When** an exception would be used,
   **Then** it is not dispatched; retain the effect and follow consultation/review rather than
   dropping the obligation. Published versions and their historical evidence remain unchanged.

### Edge Cases

- Cancel/deadline before versus after compensation starts; explicit, collection and loop scopes.
- Lookup, compensation, human decision and cancellation competing for the same effect.
- Original or compensating operation accepts, loses its response, then completes after lease loss.
- Duplicate/stale reports; restart; lost notification; unavailable audit or persistence.
- Partial compensation; dependent ordering; independent branches; exhausted forward capacity.
- Evidence is absent, out of scope, superseded or unauthorized; provider behavior changes.
- Same business identifier across tenants; cross-tenant qualification or operator access.
- Pure computation versus external-capable handler; an omitted risk label cannot assert purity.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-037-001**: Compensation failure MUST default to authorized, audited human review in explicit
  sagas, collections and loops.
- **FR-037-002**: An explicit saga's immutable published version MAY select `fail`; this MUST NOT
  be inferred from cancellation, deadline or an absent field. No new scope-level policy is added.
- **FR-037-003**: Under `fail`, terminal `COMPENSATION_FAILED` and retained step/history evidence
  MUST distinguish unresolved effects from success; no active post-terminal review queue is promised.
  `fail` applies only to known permanent failure after its retry policy or known non-retryable
  failure. Unknown, accepted or pending compensation outcomes MUST follow FR-037-016 first and
  MUST NOT automatically terminalize as failure when an attempt budget expires.
- **FR-037-004**: Cancellation/deadline MUST preserve an existing compensation review and MUST NOT
  change the failure policy because of event ordering. A terminal run MUST NOT silently reopen.
- **FR-037-005**: Independent compensations MUST remain eligible after a different compensation
  fails; dependencies MUST prevent out-of-order compensation. Terminal failure MUST wait until all
  eligible planned independent compensations settle and blocked dependents are terminally suppressed.
  Terminal runs MUST have no active, runnable or review-waiting compensation; no post-terminal
  compensation scheduling is authorized.
- **FR-037-006**: An unknown external-effect outcome MUST default to consultation/review in original
  and compensation work, including shutdown/drain paths; `highRisk` MUST NOT authorize an exception.
- **FR-037-007**: An unresolved lookup or manual decision about an unknown effect MUST survive
  cancellation/deadline unless an explicitly authorized resolution or qualified action settles it.
- **FR-037-008**: A negative lookup MUST authorize subsequent work only when the integration's
  result establishes the necessary finality; absence at a moment in time is insufficient.
- **FR-037-009**: Repeating an original effect with unknown outcome MUST require consultation that
  safely settles its outcome or an explicit authorized human resolution; this feature grants no
  automatic original-effect retry exception.
- **FR-037-010**: Defensive compensation MUST require an explicit qualified choice pinned by the
  workflow version and stable original-effect identity; declaring `compensateWith` alone is insufficient.
- **FR-037-011**: A qualifying contract MUST cover repeated compensation, an applied original effect,
  an effect that never occurred, and late completion of both original and compensation operations.
- **FR-037-012**: Qualification MUST include authoritative provider capability evidence, pinned
  provider/capability version, adapter tests and an accountable authorized approver. Author assertions
  or simulated tests alone MUST NOT establish a third-party guarantee.
- **FR-037-013**: Qualification use MUST validate presence, integrity, pinned identity, scope and
  current authorization. The platform MUST NOT claim it establishes the truth of provider guarantees.
- **FR-037-014**: Missing/invalid/unauthorized qualification MUST refuse the exception and preserve
  the effect for consultation/review. No automatic fallback to blind repeat or compensation is allowed.
- **FR-037-015**: Automatic/handler compensation success MUST correspond to authoritative final
  business outcome under the integration contract; accepted/pending is unresolved and MUST NOT
  automatically yield `COMPENSATED`. Existing authorized, audited manual success resolution remains
  permitted and MAY yield `COMPENSATED`, with human provenance retained. It MUST NOT be presented
  as provider confirmation or financial settlement, or qualify an adapter for future exceptions.
- **FR-037-016**: Unknown compensation outcomes MUST be consulted/reviewed, except a qualified
  repeat demonstrably safe under the provider guarantee and the same stable compensation identity.
- **FR-037-017**: SDK-owned handlers and reusable plugins MUST satisfy the same qualification rules;
  deployment, language or plugin origin MUST NOT confer qualification.
- **FR-037-018**: Competing lookup, exception and operator decisions MUST have one durable winning
  transition; duplicate/stale reports MUST NOT change the accepted outcome or repeat a decided action.
- **FR-037-019**: Operator views MUST expose the retained terminal/step outcome and resolution
  evidence, including the existing human actor/justification when manually resolved, without claiming
  financial settlement or provider confirmation from workflow termination alone. No new terminal
  status or redesign of manual-resolution presentation is required by this feature.
- **FR-037-020**: Operational guidance for `fail` MUST identify how to locate retained terminal/step
  evidence, assign follow-up to the integration operator and identify the applicable configured
  retention. It MUST state that the operator determines retention adequacy or external responsibility
  transfer for their use case; the platform promises neither an active post-terminal queue nor
  regulatory sufficiency. This is a documentation obligation, not a new automatic activation gate
  or a new category of “critical integration” in the product.
- **FR-037-021**: An external effect means an observable change outside the workflow's durable state.
  Purity MUST NOT be inferred from missing effect metadata or `highRisk: false`. Where an external
  outcome cannot be ruled out, the unknown-effect default applies. Pure computation remains outside
  the provider-reconciliation requirement; classification/enforcement evidence belongs in the plan.

### Non-functional Requirements

- **NFR-037-001**: State, next work, journal, notification intent and required audit MUST obey the
  existing atomic boundaries; a failed required commit MUST NOT authorize an external exception.
- **NFR-037-002**: Tenant isolation, least privilege, exact effect authority and secret-free audit
  MUST remain intact, including qualification and manual resolution.
- **NFR-037-003**: Decisions, unresolved outcomes and refusals MUST be diagnosable through typed
  errors, bounded metrics, traces and authorized audit/history; no sensitive payloads or high-cardinality labels.
- **NFR-037-004**: Restart and replay MUST preserve effect identity, accepted decisions, qualification
  references and unresolved state. No exactly-once external execution guarantee is introduced.
- **NFR-037-005**: Recovery MUST respect admission/resource limits without using forward exhaustion
  to erase a previously admitted compensation obligation. No unsupported throughput/SLO is promised.
- **NFR-037-006**: Definitions and in-flight identities remain immutable. Committed journals keep
  their meaning on replay, so an in-place upgrade from v0.2 continues in-flight runs (owner decision
  2026-11-01, D1810). Rollback MUST NOT replay an external effect or reinterpret a live version
  silently.

### Key Entities

- **Effect identity**: stable identity connecting original action, its result and compensation.
- **Compensation policy**: published review/default or explicit terminal-failure choice.
- **Unresolved outcome**: evidence that an external effect may exist but its final outcome is unknown.
- **Integration qualification**: scoped, versioned provider evidence, adapter evidence and approver
  authorizing a particular exception; not a marketplace or certification service.
- **Resolution evidence**: retained provider or operator decision with provenance, outcome and authority.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-037-001**: All cancellation/deadline-before/after-failure cases in explicit, collection and
  loop workflows preserve the same human-review obligation; restart reproduces each decision.
- **SC-037-002**: Every explicit `fail` acceptance case ends with truthful terminal/step evidence;
  no failed/dependent-unperformed compensation is presented as successfully undone, and no
  compensation remains active/runnable/waiting when the run becomes terminal.
- **SC-037-003**: Every tested unknown external outcome lacking qualification is held for
  consultation/review, including absent/false risk labels and a cancellation during uncertainty.
- **SC-037-004**: Every qualified adapter passes duplicate, absent-effect, applied-effect,
  late-completion, timeout-after-acceptance and finality scenarios for the pinned capability;
  unsupported cases prevent qualification. Real providers additionally require authoritative evidence.
- **SC-037-005**: Every race/duplicate/restart acceptance case yields one accepted decision with
  consistent retained evidence; cross-tenant and unauthorized exception attempts disclose no effect data.
- **SC-037-006**: Across the supported clients, operators can distinguish unresolved, failed,
  partial and compensated outcomes from retained evidence in the happy/failure demonstrations.
- **SC-037-007**: Each requirement has named automated acceptance evidence and its contract/doc
  trace before delivery is marked complete; existing characterization tests alone close no new criterion.

## Assumptions

- Owner direction S1-B/S2-B is approved; detailed design, adoption patch and implementation remain separate.
- Installed Spec Kit 1.1.2 templates shaped this specification; the isolated implementation checkout points to 037. The original develop checkout remains untouched.
- Feature 003/014 behavior is the baseline. Six SDKs share semantics; no new infrastructure is assumed.
- No provider is qualified by this specification. If qualification cannot be demonstrated, consultation/review remains available.
- Fresh installation/no legacy upgrade is already approved. Runtime and schema cutover still require a design.

## Dependencies, risks and migration boundaries

The Constitution III amendment and proposed ADR-0076 must be reviewed together with the dependent
003/014 and SDK/runbook text before implementation is considered conforming. Current ADR-0004,
ADR-0005, ADR-0010 and ADR-0030 remain historical constraints until explicit adoption records the
refinement. They must not be silently rewritten as if this behavior had always been approved.

Long-lived review retains state and operational responsibility; qualification can become stale;
the platform cannot prove a provider contract from source code. The plan must define qualification
binding/revocation and effect classification, contract/SDK impacts, resource/retention impact and a
safe fresh-install cutover. These are design dependencies, not invented existing capabilities.

Out of scope: new scope policy fields (O380), redesign of `abort_no_compensation`, a new terminal
status solely for manual provenance, a provider certification service, financial release approval,
deploying integrations, exactly-once effects and changing published workflow versions in place.

## Requirement acceptance mapping

| Requirements | User scenarios | Success criteria |
|---|---|---|
| FR-037-001,004,007 | US01-1..4, US03-4 | SC-037-001,003,005 |
| FR-037-002,003,005,019,020 | US02-1..5 | SC-037-002,006 |
| FR-037-006,008,009,021 | US03-1..4 | SC-037-003 |
| FR-037-010..014,017 | US04-1,2,4 | SC-037-004,005 |
| FR-037-015,016 | US01-4, US03-3, US04-2,3 | SC-037-003,004 |
| FR-037-018; NFR-037-001..006 | US01-3, US04-2..4; edge cases | SC-037-005,007 |

Named tests and final contracts belong in the subsequent plan/tasks/analysis; no implementation
tasks are marked complete here. See [research and approval record](research.md).


## Delivery-1 compatibility limitation — owner confirmed 2026-11-01

Feature-009's ordinary `SECRET_IN_OUTPUT` behavior remains final failure. Delivery 1 preserves
recovery for successful-result storage refusal; it does not guarantee recovery review for every
rejected report. A secret-bearing completion may still follow the existing failure/onError path,
without retaining the prohibited payload. Existing managed-database protections are unchanged.
The owner explicitly selected this boundary; see [decision](../../docs/owner-decisions.md).
Provider qualification remains outside delivery 1 and cannot be inferred from this exception.
