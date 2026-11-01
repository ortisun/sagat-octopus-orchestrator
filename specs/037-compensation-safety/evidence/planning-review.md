# EGM Design Critic — feature 037 plan

Date: 2026-11-01. Scope: selected Design Critique, not the full five-phase EGM protocol or a
runtime/provider validation. Reviewer: fresh-context `/root/saga_plan_goldfish` (`fork_turns: none`).
Installed persona: EGM 0.1.0 `agents/design_critic.md`; target standard:
`resources/document_standards/heilmeier-standard.md`. The reviewer confirmed reading both complete
files, all planning artifacts/spec/Constitution/ADR-0076 and relevant implementation/test evidence.
No edits, builds or tests were performed by the reviewer.

## R1 findings (reviewer report, consolidated faithfully)

| ID | Severity / confidence | Finding and evidence | Counterargument considered | Proposed response |
|---|---|---|---|---|
| P1 | Blocker / high | FR-037-009 requires resolution before original repeat, but plan/tasks preserved privileged unknown-original Retry. `crates/application/src/runs.rs:1361` upgrades authority then emits StepRetryRequested, not outcome-bearing StepResolved. | An audited retry could be interpreted as the human decision, but the documents distinguish retry from resolve. | Revised proposal explicitly refuses direct Retry for any unknown original/compensation, even with SagaOverride; existing Resolve records outcome first. No new in-place retry-after-resolution. Owner design approval required. |
| P2 | Blocker / high | Contract/task promised durable refusal audit, but current review transaction writes a success action only after a successful domain command; refusal rolls it back. `runs.rs:1383` and `domain/src/audit.rs` have no proposed refusal action/path/idempotency contract. | Typed conflict/trace could suffice, but contradicted the draft's durable-audit promise. | Remove the unapproved new audit promise. State conflict writes no execution/outbox/success-audit/idempotency success; existing tracing/typed error and retained review history diagnose it. Existing permission-denial audits and mandatory accepted-decision audit unchanged. |
| P3 | Major / high | Retained-review storage/operator cost had no model or measurement exam. Existing journal/audit retention is not built (execution-engine runbook, G28/G182). | No users means a production forecast would be fabricated; parameterized measurement remains possible. | Add bytes-per-review/backlog model, synthetic 1/100/1,000-review experiment and stop criteria. Explicitly state retention gaps and operator-owned backup/export; no invented control or automatic retention gate. |
| P4 | Major / high | Dependency order and checkpoints did not answer elapsed time/staffing under Heilmeier. | Task order is useful but not duration. | Add low-confidence conditional 3–6 working-week planning estimate with one dedicated engineer and checkpoint reviewer, six toolchains available, excluding owner/provider waits; re-estimate after first checkpoint. No release commitment or observed throughput claim. |
| P5 | Major / medium-high | Summary used implementation terms; current versus target behavior required reconstructing research/code. | Linked business spec was clear but did not meet self-contained proposal standard. | Add plain-language objective and compact current-limit/new-approach paragraphs. |

R1 conclusion: concrete enough for human design review, **not ready for implementation approval**
until P1/P2 are decided; cost/timeline/objective did not yet pass Heilmeier. The reviewer confirmed
that US04 evidence/authority/protocol gaps are properly separated and do not block a delivery that
enables no qualified exceptions. T037-034 addressed the old negative-lookup scenario migration.

## Disagreements and decision limits

The initial proposed preservation of privileged original retry was challenged, not treated as proof
because it exists today. The revised plan chooses the conservative interpretation for owner review.
This changes the current shortcut and must be visible in the implementation approval. It is not a
claim that the owner previously chose it. Known-failure retry and audited outcome resolution remain.

A dedicated durable conflict-refusal event was rejected as unnecessary new machinery in this slice.
This narrows a draft promise, not an approved audit protection: accepted manual decisions still
require their audit, and authorization-denial auditing retains the current policy.

US04 qualification, actual provider finality, production costs and all runtime acceptance remain open.
Agreement between reviewers is not proof. Actual implementation must pass the planned tests.

## R2 closure

The reviewer reread the revised plan, model, contracts, tasks, research, spec/coverage/quickstart
and relevant current review/audit source. It reported **no unresolved or new blockers for human
design review**. P1–P5 were resolved in the proposal: unknown-original resolution was explicit,
conflict rollback matched the current transaction shape, costs/retention/measurement were specified,
timing was a qualified estimate, and the objective/current behavior were self-contained.

Two non-blocking wording nits remained: the delivery-sequence summary still named only unknown
compensation retry, and coverage said “retention configuration.” Both were corrected to “every
unknown effect” and “controls or their absence” after R2. No semantic change or runtime fix was made.

R2 conclusion: ready for owner design approval, **not runtime-validated or implementation-complete**.
US04 evidence, exact qualification authority/protocol design, implementation and SC-037-004 remain open.
This selected review does not prove broader reset completion or successful financial integration.
