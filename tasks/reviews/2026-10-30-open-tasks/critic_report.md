# Stage 7 — Consolidated critic report

The three independent critic personas returned seven raw findings. They consolidate into **two Major actions and two Nits; no Blocker was established by this stage**. This is a review of the task dossier and linked specifications, not an implementation or release-acceptance verdict. Source tasks and the frozen dossier remain unchanged.

## CRIT-001 — Major — Reconcile distribution scope with the recorded owner decision

Affected entry: **OT-011 / FR-006-008**.

The preserved task at [tasks/todo.md:71](../../todo.md#L71) still requires registry publication and credentials. The [owner decision](../../../docs/owner-decisions.md#L49), the [packaging runbook](../../../docs/runbooks/sdk-packaging.md#L41) and later release records select GitHub Releases instead. The dossier overview repeats the old registry wording without identifying that conflict.

The next document change should separate the superseded registry obligation from any remaining verification of the authorized release artifacts. It should preserve the task ID and historical evidence, qualify the overview and reconcile the FR-006-008 traceability row. External registry credentials are not a current prerequisite under the recorded decision. This review does not establish that every distribution obligation is complete.

Closure evidence: consistent current-scope wording in the task/overview/traceability, references to the existing owner decision, and an explicit list of any remaining GitHub artifact/provenance/consumer checks. No new publishing channel or checkbox closure is inferred.

Raw findings: **DESIGN-001, STRUCT-002, STYLE7-002**.

## CRIT-002 — Major — Correct the human-results artifact before using it

Affected entries: **OT-010 and OT-012 / SC-008-001 and SC-012-001**. The numeric interaction threshold applies specifically to **SC-008-001**.

The [results template](../../../docs/validation/human-acceptance-results.template.md#L55) still says the diagnosis threshold awaits an owner decision and asks observers to record `N UNSET`. The [owner decision](../../../docs/owner-decisions.md#L40), [feature criterion](../../../specs/008-observability-operations/spec.md#L91) and [protocol](../../../docs/validation/human-acceptance.md#L11) already set **N = 10**, so diagnosis passes with **fewer than 10 counted interactions**.

Reconcile the template, the old task wording and the dossier's baseline annotation. Preserve `NOT RUN` until genuine sessions occur. Link the two task locations to a shared evidence index while retaining the distinct diagnosis and newcomer-onboarding criteria; do not treat them as two independent studies or apply the diagnosis threshold to onboarding.

Closure evidence: consistent threshold/comparison fields and a usable form, followed by actual protocol-compliant session records and independent verdicts. A corrected form alone does not close human acceptance.

Raw findings: **DESIGN-002, STRUCT-001**.

## CRIT-003 — Nit — Align pending acceptance wording with the sixth official SDK

Affected entries: **OT-003, OT-004, OT-007, OT-008**; T014-016, T014-017, T014-043 and T014-044.

The open primitive task/spec/plan wording still describes a five-SDK boundary. The [constitution](../../../.specify/memory/constitution.md#L50), [Rust feature](../../../specs/034-rust-sdk/spec.md#L24), later ledger and existing shared harness establish six SDKs. The design reviewer classifies this as a Nit because the governing boundary and harness are already present; this stage did not find a missing Rust implementation.

State the current feature-034 dependency and six-language requirement in forward-looking acceptance instructions. Keep historical five-language receipts dated and unchanged, and distinguish selected combined-tree scenarios from the final integrated suite.

Closure evidence: aligned current normative wording and explicit source/language/scenario selection for the required gate. This review does not rerun or accept that gate.

Raw finding: **DESIGN-003**.

## CRIT-004 — Nit — Bind the opening sentence to the canonical inventory snapshot

Affected scope: all **OT-001–OT-012**, dossier purpose at line 5.

Use the concrete scope already recorded by the inventory: 12 unchecked entries from `specs/*/tasks.md` and `tasks/todo.md` at the stated revision. The current phrase “every unchecked task entry in the current checkout” could also be read as including arbitrary checkboxes or newly generated review documents. No actual omitted source entry was found.

Suggested wording: “Review the 12 unchecked source entries recorded in `inventory.json`, drawn from `specs/*/tasks.md` and `tasks/todo.md` at the snapshot below.”

Closure evidence: a scoped wording correction in a later authorized revision, retaining all source locations and review exclusions.

Raw finding: **STYLE7-001**.

## Raw finding traceability

| Raw finding | Original severity | Consolidated action |
|---|---|---|
| DESIGN-001 | Major | CRIT-001 |
| DESIGN-002 | Major | CRIT-002 |
| DESIGN-003 | Nit | CRIT-003 |
| STRUCT-001 | Major | CRIT-002 |
| STRUCT-002 | Nit | CRIT-001 |
| STYLE7-001 | Nit | CRIT-004 |
| STYLE7-002 | Nit | CRIT-001 |

## Coverage and limits

All twelve source entries are preserved and accounted for. OT-001–OT-009 had no additional structural finding; their acknowledged implementation and evidence gaps remain open. The reviewers did not demand a second product specification or code-level detail in FeatureRange merely to satisfy an example template.

The structural reviewer found fourteen distinct unavailable historical destinations in sixteen linked occurrences. The feature validation report already identifies that archive as outside this checkout. Those unavailable artifacts cannot support independently verified closure; the finding does not require restoring the entire archive or imply that the corresponding implementation is absent. The [structural report](./critic_structural_report.md) lists the exact paths.

Read coverage is explicit in each complete reviewer report: [design](./critic_design_report.md), [structure](./critic_structural_report.md) and [style](./critic_style_report.md). Design and structure fully read 38 of 39 baseline files and selectively read the long validation history; style fully read 36 and selectively read three large histories. Their personas, assigned standards and full target were read. No inaccessible core input was reported. Uninspected history, sibling worktree contents, historical logs and external CI are not represented as verified.

No fixes were applied between stages. No runtime test, build, service action, publication or task-state change was performed by these reviewers.
