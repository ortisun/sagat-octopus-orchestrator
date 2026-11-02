# Stage 9 — Implementation readiness report

The aggregate checklist **needs refinement before it can serve as a zero-decision execution plan**. The existing plans nevertheless specify the next bounded capacity, upgrade and evidence-maintenance actions. Those next actions are conditionally executable; their existence does not complete the broad feature or release criteria.

Three fresh reviewers returned **14 raw findings: 12 Major, 2 Nit, no Blocker**. These consolidate into **6 Major actions and 1 Nit**. The original source tasks remain unchanged. No runtime defect, failed historical test run, production ceiling, publication acceptance or human result is established by this review.

## Review inputs and complete reports

- [Plan readiness](./readiness_plan_report.md): 2 Major, 1 Nit; conditional readiness for the next bounded actions.
- [Codebase correspondence](./readiness_codebase_report.md): 6 Major; remaining-change map and evidence corrections needed under the implementation-readiness standard.
- [Implementer-facing style](./readiness_style_report.md): 4 Major, 1 Nit; precision of scope, resource claims and historical results.

The apparent verdict difference concerns the unit of work. The plan reviewer evaluates the next bounded increments using linked proof plans. The code reviewer evaluates whether the aggregate preserved checkbox descriptions alone specify the remaining changes. Both support reusing existing implementation and plans, then publishing one explicit remaining-work map rather than recreating the architecture.

## IR-001 — Major — Reconcile the publication task with the approved channel

**OT-011 / FR-006-008.** Replace the active dependency on registry identities with reconciliation against the recorded GitHub Releases-only decision. Preserve the historical entry and identify actual outstanding immutable-asset, provenance and clean-consumer checks. Existing release/package tooling is present; no new publishing channel is authorized.

Sources: [owner decision](../../../docs/owner-decisions.md#L49), [packaging policy](../../../docs/runbooks/sdk-packaging.md#L41), [source task](../../todo.md#L71).

Raw findings: **READINESS-001, CODE-006, STYLE9-002**.

## IR-002 — Major — Correct the human-results template before using it

**OT-010 and OT-012 / SC-008-001 and SC-012-001.** SC-008-001 already uses **N = 10**, so successful diagnosis takes **at most nine counted interactions**. The old pending-decision wording and `N UNSET` result fields conflict with the protocol. Correct those instructions while preserving `NOT RUN` until genuine sessions occur. The separate newcomer criterion still requires the prescribed TypeScript/C# observations and timing evidence; N is not a participant count or its timing threshold.

Sources: [decision](../../../docs/owner-decisions.md#L40), [protocol](../../../docs/validation/human-acceptance.md#L10), [stale form](../../../docs/validation/human-acceptance-results.template.md#L55).

Raw findings: **READINESS-002, CODE-005, STYLE9-001**.

## IR-003 — Major — Publish one current remaining-change map

**OT-001–OT-009.** Map existing implementation to the still-required delta, marking each actual path and symbol Create/Modify/Delete or verification-only. Include the prerequisite increment, exact integrated or unmerged source, shared fixture, named negative/happy case, state oracle and closure condition. An absent proposed file is not an additional defect.

Use the existing bounded plans and [pinned handoff](../../../specs/014-workflow-primitives/capacity-eligibility.md#L17). The plan review identifies coordinator snapshot `ffe54ab41e0fafbaaae8c298ce3e87bee22aa89d`; its recorded decisions and tests are branch-scoped evidence, not integrated acceptance. Do not duplicate that work or reopen its recorded decisions merely because its files are absent from `develop`.

Raw findings: **CODE-001**; the current-frontier part of **READINESS-003**.

## IR-004 — Major — Replace the false universal fault-oracle statement with case-level evidence

**OT-005 / T014-040.** The named historical commit `7d7868ff` contains registered collection-source fault cases without the claimed `verify_run`/`assert_consistent` replay checks, including their shared fixture paths. They have other useful persisted-state assertions; the historical 87 passing results are not disproved.

Credit the actual scope/item/audit/status/aggregate assertions and list which cases also compare replay/accounting. Add further checks only where the complete acceptance matrix requires them. Do not require a particular helper merely for its name or describe the existing tests as having no persistence assertions.

Evidence: [complete code review](./readiness_codebase_report.md#code-002--major--the-universal-fault-replay-oracle-statement-is-false-at-its-named-historical-checkpoint), [parent source checks](./source_checks.json), [historical claim](../../../specs/014-workflow-primitives/validation-report.md#L1514).

Raw finding: **CODE-002**.

## IR-005 — Major — Separate historical performance, remaining workloads and resource ownership

**OT-006 / T014-042**, with traceability consequences for **OT-008**. Preserve the original G461/G468 failure as a dated baseline. Point to later bounded-path changes and frozen runtime-role results, then identify remaining current-source and nested/100k acceptance. Later flat/wait results do not accept production ceilings or prove actual parallel worker counts from logical width.

State **zero dedicated resources per wait**, rather than zero total resource cost: shared threads, connection pools and scheduler work remain. Record runtime and observer pool budgets and measurement limitations. This correction does not require redesigning the existing scheduler or rerunning a benchmark simply to edit the summary; completing the performance task still requires its actual prescribed workloads.

Sources: [task](../../../specs/014-workflow-primitives/tasks.md#L194), [resource definition](../../../tests/performance/reports/workflow-primitives.md#L16), [later measurements](../../../tests/performance/reports/workflow-primitives.md#L1393).

Raw findings: **CODE-003, STYLE9-003, STYLE9-004**; the performance-routing part of **READINESS-003**.

## IR-006 — Major — Make future closure instructions require six SDKs

**OT-003, OT-004, OT-007, OT-008.** Current forward-looking collection acceptance must include Rust and use the existing common harness/scenarios/goldens. The six-language infrastructure and Rust builders already exist. Preserve historical five-language receipts and distinguish filtered/focused six-language results from the full integrated gate.

Sources: [constitution](../../../.specify/memory/constitution.md#L50), [Rust requirement](../../../specs/034-rust-sdk/spec.md#L24), [shared registry](../../../tests/contract/sdk-conformance/harness.rs#L35).

Raw finding: **CODE-004**. The high-altitude stages classify the wording as Nit because the governing requirement and implementation already exist. This stage treats execution instructions that can understate the closure matrix as Major; it does not infer a missing Rust implementation.

## IR-007 — Nit — Expand compressed result notation without changing evidence

**OT-004 and OT-009.** Restore spaces, label passed/failed/ignored counts, and distinguish coordinator, nested released phase and total-command times. Preserve source revisions and all failures. A nested phase is not another aggregate test execution.

Raw finding: **STYLE9-005**.

## Raw-to-action traceability

| Raw finding | Severity | Action |
|---|---|---|
| READINESS-001 | Major | IR-001 |
| READINESS-002 | Major | IR-002 |
| READINESS-003 | Nit | IR-003 and IR-005 |
| CODE-001 | Major | IR-003 |
| CODE-002 | Major | IR-004 |
| CODE-003 | Major | IR-005 |
| CODE-004 | Major | IR-006 |
| CODE-005 | Major | IR-002 |
| CODE-006 | Major | IR-001 |
| STYLE9-001 | Major | IR-002 |
| STYLE9-002 | Major | IR-001 |
| STYLE9-003 | Major | IR-005 |
| STYLE9-004 | Major | IR-005 |
| STYLE9-005 | Nit | IR-007 |

## Per-entry readiness and next action

These statuses concern the next action, not task completion. All technical entries also inherit IR-003's remaining-change-map requirement.

| Entry | Status | Next action and dependency |
|---|---|---|
| OT-001 — T014-014 | Conditional | Execute the native Acquire/recovery and guarded-claim proofs specified by the current plans; retain nested limit-one, join, reservation and reduction-to-zero obligations until their real runtime path exists. |
| OT-002 — T014-015 | Conditional | Continue the existing possession/eligibility/sweep/caller sequence. Creator/member insertion, scoped path and retention/backfill obligations remain distinct activation prerequisites. |
| OT-003 — T014-016 | Conditional | Preserve existing captured-source/blob and builders; define the next bounded body/source delta and negative cases, then test it through shared six-language fixtures. |
| OT-004 — T014-017 | Conditional | Run source-bound released profiles and remaining authority/uncertain-COMMIT compositions. Do not recreate already documented bounded Applied quota/drain/compensation or cold-cutover work. |
| OT-005 — T014-040 | Conditional | Correct the oracle claim, then execute the complete applicable fault/security matrix on the integrated source with actual persistence oracles and runtime-role evidence. |
| OT-006 — T014-042 | Conditional | Use current flat/wait/tenant drivers and record real resources. Nested-100k measurement requires a supported nested path; existing narrower results remain historical. |
| OT-007 — T014-043 | Conditional | Execute the complete repository/CI, contract, upgrade, six-SDK, Compose and advisory gates on the actual integrated source. |
| OT-008 — T014-044 | Ready for evidence-maintenance work | Reconcile current clause-to-evidence references now. Final closure remains dependent on the applicable technical acceptance; preserve unsupported shapes and failed/missing evidence. |
| OT-009 — T033-007 | Conditional | Execute final release-tree messaging/Redis, upgrade and complete common-suite gates using existing cold-cutover fixtures. Selected combined-tree results remain partial evidence. |
| OT-010 — SC-008-001 / SC-012-001 | Human evidence | Correct the form and use the existing genuine-session protocol, sharing criterion evidence with OT-012. |
| OT-011 — FR-006-008 | Needs scope reconciliation | Use the existing owner decision and GitHub artifact/consumer checks; do not wait for identities to unlock deferred registry publication. |
| OT-012 — SC-008-001 / SC-012-001 | Human evidence | Collect and review real operator/newcomer records, preserving failed/aborted/invalid sessions and distinct criteria. |

## What this verdict does not establish

There is no demonstrated new production-code defect in these reviews. Actual bounded primitives, deterministic PostgreSQL fixtures, shared SDK machinery, released profiles and callback cutover exist. General nesting, capacity activation, production ceilings, the full final-tree gates and human acceptance remain open where their specified evidence is missing.

Reviewers did not execute those gates, inspect every historical transcript or verify remote CI/release assets. Full/selective reads, source ranges and pinned-branch limits remain in the three complete reports. The parent independently checked the historical counterexample through read-only Git source inspection. No canonical checkbox or source implementation was changed.
