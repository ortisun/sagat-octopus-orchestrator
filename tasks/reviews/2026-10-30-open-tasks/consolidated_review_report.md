# Consolidated EGM review — open tasks

**Review complete.** Five stages and nine fresh-context reviewers analyzed the **12 captured unchecked entries** in three source task files at `develop@888074e6237b2a1ce9c85f83ec5703b081c65dd6`. The 18 canonical inventories and 39 linked baseline files remain unchanged.

**Result: 8 distinct actions — 6 Major, 2 Nit, 0 Blocker — consolidated from 32 raw findings.** All eight actions remain open. Review completion does not complete the source tasks or establish runtime/release acceptance.

The existing bounded plans support the next technical increments conditionally. The preserved aggregate checklist needs a current remaining-change map and evidence/scope corrections before it can serve as an unambiguous implementation assignment. Genuine human evaluation and final integrated acceptance remain separate work.

## Prioritized actions

The identifiers below are review-local aliases; canonical T/FR/SC identifiers and original checkbox states are preserved.

| Action | Priority | Affected entries | Required correction |
|---|---|---|---|
| REV-001 | Major | OT-011 | Reconcile registry publication with the recorded GitHub Releases-only decision. |
| REV-002 | Major | OT-010, OT-012 | Apply the settled diagnosis threshold to the task/form; keep genuine human results pending. |
| REV-003 | Major | OT-001–OT-009 | Publish the actual remaining delta, file operations, source pins, prerequisites, tests and oracles. |
| REV-004 | Major | OT-005 | Correct the false universal fault replay-oracle description at the cited checkpoint. |
| REV-005 | Major | OT-006, OT-008 | Separate historical scale work, current remaining workloads and per-wait resource ownership. |
| REV-006 | Major | OT-003, OT-004, OT-007, OT-008 | Make new acceptance instructions explicitly cover all six SDKs. |
| REV-007 | Nit | All entries | Bind the purpose sentence to the canonical inventory snapshot. |
| REV-008 | Nit | OT-004, OT-009 | Expand compressed counts and distinguish nested/coordinator/command times. |

### REV-001 — Distribution scope

The active entry at [tasks/todo.md:71](../../todo.md#L71) treats registry identities as a prerequisite. The [owner decision](../../../docs/owner-decisions.md#L49) selects GitHub Releases only; [packaging instructions](../../../docs/runbooks/sdk-packaging.md#L41) defer external registries. Reconcile the task, requirement traceability, overview and any other operative pending wording with that existing decision. Identify any actual remaining immutable-asset, checksum/provenance and clean-consumer evidence for the approved channel. Existing package/release machinery should be reused.

This correction neither authorizes registry publication nor independently verifies remote release assets. Source FR-006-008 and historical evidence remain traceable.

### REV-002 — Human evidence and its form

SC-008-001 already uses **N = 10**, so diagnosis must take **fewer than 10 counted interactions: at most nine**. N is not a sample-size requirement or the onboarding timing target. The [required result form](../../../docs/validation/human-acceptance-results.template.md#L55) contradicts the [protocol](../../../docs/validation/human-acceptance.md#L10) by declaring N unset.

Correct the form and obsolete threshold-agreement wording, then collect genuine protocol-compliant records. OT-010 and OT-012 are two source locations for the same criterion pair; use a shared evidence index, preserving distinct diagnosis and TypeScript/C# onboarding observations. Keep `NOT RUN` until sessions occur and retain unsuccessful/aborted/invalid records. A corrected template alone does not satisfy human acceptance.

### REV-003 — Remaining implementation delta and routing

The checkbox descriptions accumulate substantial implemented behavior and historical results without one explicit remaining-change map. Preserve existing collection/source/worker/Redis machinery and distinguish it from required new combinations. For each bounded increment, record:

| Required field | Evidence needed |
|---|---|
| Existing behavior and remaining clause | Current implemented surface, retained proof, exact unresolved obligation |
| File operation | Create/Modify/Delete or verification-only, with actual path and symbol |
| Source/ownership | Integrated commit or pinned unmerged snapshot, dependency and worktree ownership |
| Test and oracle | Named happy/negative/race case, shared fixture and persisted-state assertions |
| Command and closure | Exact invocation/configuration, observed result, contract/telemetry/audit/demo and closure boundary |

An absent proposed file is not itself a defect. Existing capacity inputs resolve through the [documented pinned handoff](../../../specs/014-workflow-primitives/capacity-eligibility.md#L17). The readiness reviewer inspected coordinator snapshot `ffe54ab41e0fafbaaae8c298ce3e87bee22aa89d`; its decisions and focused results are branch-scoped evidence. They must not be recreated merely because they are unmerged, or treated as already integrated. See the [complete readiness plan](./readiness_plan_report.md) for the current sequence and limits.

### REV-004 — Fault evidence must name its actual oracles

The claim that every fault test ends with `verify_run`/`assert_consistent` is false even at the cited historical commit **7d7868ff**. Its registered collection-source cases and shared fixture files contain counterexamples. They have other meaningful persisted-state assertions, including scope/item/audit counts, rollback absence, status and aggregate checks.

Replace the universal statement with a case-to-oracle inventory, credit equivalent assertions, and add only the replay/accounting coverage actually required by the remaining complete matrix. **The recorded 87 passing tests are not disproved**, and no runtime failure was established. The parent independently checked the historical source; [source hashes and checks](./source_checks.json) and the [codebase finding](./readiness_codebase_report.md#code-002--major--the-universal-fault-replay-oracle-statement-is-false-at-its-named-historical-checkpoint) retain that evidence.

### REV-005 — Performance evidence must retain time and measurement boundaries

Date the original G461/G468 failure, then point to later bounded-path changes and frozen flat/wait results. The [later performance record](../../../tests/performance/reports/workflow-primitives.md#L1393) does not establish current-head release capacity, nested 100k acceptance or a latency SLO. Logical width is not proof of simultaneous external-worker execution; sampled resources and unisolated PostgreSQL RSS retain their stated limits.

Use **zero dedicated worker/thread/connection per wait**, not zero total cost. Shared process threads, pools and scheduler work remain. Identify the remaining workload, supported-runtime prerequisites, driver/configuration and resource observations. Keep production ceilings unaccepted. Reuse the existing frontier and scoped-worker paths; an old summary does not justify a new scheduler, queue or cache architecture.

### REV-006 — New closure instructions must use six SDKs

The unfinished collection instructions still enumerate five languages. The [constitution](../../../.specify/memory/constitution.md#L50), [Rust requirement](../../../specs/034-rust-sdk/spec.md#L24), [shared harness](../../../tests/contract/sdk-conformance/harness.rs#L35) and CI already provide six-language authority/infrastructure.

Align operative acceptance wording and use the existing shared scenarios and reviewed goldens. Record source, scenario and language filters; a focused six-language run is not the entire integrated matrix. Preserve historical five-language receipts exactly as historical results. This is not a missing-Rust-implementation finding.

### REV-007 and REV-008 — Precision without changing meaning

Describe the corpus as the 12 captured unchecked entries from `specs/*/tasks.md` and `tasks/todo.md` at the recorded revision, rather than every checkbox in an evolving repository. No omitted canonical source entry was found.

Add ordinary spacing and label passed/failed/ignored counts and phase times. Preserve all numbers, source identities and failure records; a nested released phase is not an additional aggregate test.

## Readiness of every source entry

These are statuses of the **next action**, not completion verdicts. All technical entries inherit REV-003's remaining-delta requirement.

| Entry | Canonical identifier | Status | Next action and closure boundary |
|---|---|---|---|
| OT-001 | T014-014 | Conditional | Follow native Acquire/guarded-claim proofs and retain nested limit-one, join, reservation and reduction-to-zero evidence until real runtime wiring exists. |
| OT-002 | T014-015 | Conditional | Continue existing possession/eligibility/sweep/caller plans. Creator/member, scoped-path and retention/backfill obligations remain activation prerequisites. |
| OT-003 | T014-016 | Conditional | Preserve captured-source/blob and existing builders; specify the next bounded body/source delta and negative cases, using shared six-SDK fixtures. |
| OT-004 | T014-017 | Conditional | Verify the source-bound released profiles and remaining authority/uncertain-COMMIT compositions. Preserve already documented Applied quota/drain/compensation and cold-cutover proofs. |
| OT-005 | T014-040 | Conditional | Correct the oracle description, then run the applicable full fault/security matrix with real persistence evidence and runtime-role coverage. |
| OT-006 | T014-042 | Conditional | Use existing flat/wait/tenant drivers on the selected source; nested-100k execution requires the corresponding supported runtime. Record resource and environment limits. |
| OT-007 | T014-043 | Conditional | Run complete integrated repository/CI, contracts, upgrades, six-SDK, Compose and advisory gates; update operator/rollback guidance to proven shapes. |
| OT-008 | T014-044 | Ready for evidence maintenance | Update current clause-to-evidence routing now; final acceptance remains conditional on the applicable technical proofs. |
| OT-009 | T033-007 | Conditional | Execute final release-tree messaging/Redis, released-profile and common-suite gates using existing cutover fixtures. Selected combined-tree evidence stays partial. |
| OT-010 | SC-008-001 / SC-012-001 | Human evidence | Correct the form and conduct the existing genuine-session protocol, sharing criterion evidence with OT-012. |
| OT-011 | FR-006-008 | Needs scope reconciliation | Resolve the GitHub/registry contradiction, then verify any outstanding approved artifact/consumer evidence. |
| OT-012 | SC-008-001 / SC-012-001 | Human evidence | Collect and review actual operator/newcomer records, retaining unsuccessful and invalid outcomes. |

## Recommended execution order

1. Reconcile the publication and human-result instructions and the false oracle statement. Produce the current remaining-change map and accurate evidence summaries; do not close source criteria from these edits alone.
2. Follow the existing bounded capacity/proof dependencies and coordinate source/worktree ownership before shared-code changes. Dormant ports, isolated proofs and unmerged branch results do not authorize group activation or general nesting.
3. Run the complete applicable fault, compatibility, six-SDK and resource gates on the actual integrated source. Preserve initial failures, ignored/selected cases, environment and source identity. Existing historical reports are not a combined new pass.
4. Collect genuine human evidence through the corrected existing protocol. Update requirement-to-test/contract/telemetry/audit/demo links last, closing only criteria whose evidence actually exists.

## Severity reconciliation

The earlier stages disagree on the priority of SDK wording and the old performance narrative. Their original severities remain in the traceability table and complete reports.

- REV-006 is Major here because Stage9 identifies forward execution/closure instructions that can understate the required matrix. The existing six-language harness remains credited; no infrastructure absence is inferred. Design and intermediate zoom-out classified the wording alone as Nit.
- REV-005 is Major for the combined readiness action: Stage9 adds the overstrong quantitative resource wording and remaining-work guidance. Intermediate zoom-out treated only the old linked summary as Nit. Neither classification establishes a current runtime performance defect or accepts ceilings.
- The false replay statement is Major evidence correction, not a Blocker claim that historical tests failed or production is broken.

## Raw finding traceability

Each raw finding is mapped below; aggregate CRIT/IR IDs are aliases and are not counted again as raw findings.

| Raw finding | Stage | Original severity | Consolidated action | Complete report |
|---|---:|---|---|---|
| ZO-001 | 5 | Major | REV-001 | [Initial zoom out](./zoom_out_report.md) |
| ZO-002 | 5 | Major | REV-002 | [Initial zoom out](./zoom_out_report.md) |
| ZO-003 | 5 | Major | REV-006 | [Initial zoom out](./zoom_out_report.md) |
| ZO-004 | 5 | Major | REV-005 | [Initial zoom out](./zoom_out_report.md) |
| COMP-001 | 6 | Major | REV-001 | [Comprehension](./comprehension_report.md) |
| COMP-002 | 6 | Major | REV-002 | [Comprehension](./comprehension_report.md) |
| DESIGN-001 | 7 | Major | REV-001 | [Design](./critic_design_report.md) |
| DESIGN-002 | 7 | Major | REV-002 | [Design](./critic_design_report.md) |
| DESIGN-003 | 7 | Nit | REV-006 | [Design](./critic_design_report.md) |
| STRUCT-001 | 7 | Major | REV-002 | [Structure](./critic_structural_report.md) |
| STRUCT-002 | 7 | Nit | REV-001 | [Structure](./critic_structural_report.md) |
| STYLE7-001 | 7 | Nit | REV-007 | [Feature style](./critic_style_report.md) |
| STYLE7-002 | 7 | Nit | REV-001 | [Feature style](./critic_style_report.md) |
| IZO-001 | 8 | Major | REV-001 | [Intermediate zoom out](./intermediate_zoom_out_report.md) |
| IZO-002 | 8 | Major | REV-002 | [Intermediate zoom out](./intermediate_zoom_out_report.md) |
| IZO-003 | 8 | Nit | REV-006 | [Intermediate zoom out](./intermediate_zoom_out_report.md) |
| IZO-004 | 8 | Nit | REV-005 | [Intermediate zoom out](./intermediate_zoom_out_report.md) |
| IZO-005 | 8 | Nit | REV-007 | [Intermediate zoom out](./intermediate_zoom_out_report.md) |
| READINESS-001 | 9 | Major | REV-001 | [Plan readiness](./readiness_plan_report.md) |
| READINESS-002 | 9 | Major | REV-002 | [Plan readiness](./readiness_plan_report.md) |
| READINESS-003 | 9 | Nit | REV-003, REV-005 | [Plan readiness](./readiness_plan_report.md) |
| CODE-001 | 9 | Major | REV-003 | [Codebase](./readiness_codebase_report.md) |
| CODE-002 | 9 | Major | REV-004 | [Codebase](./readiness_codebase_report.md) |
| CODE-003 | 9 | Major | REV-005 | [Codebase](./readiness_codebase_report.md) |
| CODE-004 | 9 | Major | REV-006 | [Codebase](./readiness_codebase_report.md) |
| CODE-005 | 9 | Major | REV-002 | [Codebase](./readiness_codebase_report.md) |
| CODE-006 | 9 | Major | REV-001 | [Codebase](./readiness_codebase_report.md) |
| STYLE9-001 | 9 | Major | REV-002 | [Detailed style](./readiness_style_report.md) |
| STYLE9-002 | 9 | Major | REV-001 | [Detailed style](./readiness_style_report.md) |
| STYLE9-003 | 9 | Major | REV-005 | [Detailed style](./readiness_style_report.md) |
| STYLE9-004 | 9 | Major | REV-005 | [Detailed style](./readiness_style_report.md) |
| STYLE9-005 | 9 | Nit | REV-008 | [Detailed style](./readiness_style_report.md) |

Aggregate aliases: CRIT-001→REV-001; CRIT-002→REV-002; CRIT-003→REV-006; CRIT-004→REV-007. IR-001→REV-001; IR-002→REV-002; IR-003→REV-003; IR-004→REV-004; IR-005→REV-005; IR-006→REV-006; IR-007→REV-008. READINESS-003 contributes to both the delta map and performance routing without becoming two raw findings.

Machine-readable actions and mappings: [consolidated_findings.json](./consolidated_findings.json).

## Stage artifacts and verification limits

| Stage | Completed artifact |
|---|---|
| 5 — Initial Zoom Out | [zoom_out_report.md](./zoom_out_report.md) |
| 6 — Comprehension | [comprehension_report.md](./comprehension_report.md), including both original paraphrases |
| 7 — Critic Review | [critic_report.md](./critic_report.md), with complete design/structure/style reports |
| 8 — Intermediate Zoom Out | [intermediate_zoom_out_report.md](./intermediate_zoom_out_report.md) |
| 9 — Implementation Readiness | [implementation_readiness_report.md](./implementation_readiness_report.md), with three complete persona reports |

All nine reviewers used fresh conversation contexts with GPT-6 Astra at xhigh reasoning. Prior findings were supplied only to the intermediate reconciliation stage. Personas, assigned standards and target were read; each report records full/selective context and uninspected ranges. Large historical records were not exhaustively read by every rubric, and no full-source security or runtime audit is claimed.

The structural review lists fourteen distinct historical destinations unavailable on this checkout. Some capacity design inputs resolve through their explicit pinned Git handoff; that does not recover or independently verify every old raw execution archive. Missing archived receipts and unmerged code are kept separate from actual current acceptance.

Original task files, the frozen dossier and baseline hashes remain unchanged. No runtime test/build, CI query, remote release verification, publication or human session was performed by these reviews. The parent independently checked the cited historical source counterexample. [Artifact validation](./artifact_validation.json), [execution provenance](./execution_manifest.json), [input hashes](./review_context.json) and [original inventory](./inventory.json) retain the verification scope.

The earlier README bootstrap is a separate completed documentation change: 203 contracts, 390 local links, 5,258 cited source paths and 132 Cargo command targets passed local checks; architecture (24 workspace crates), agent configuration, vendor provenance (84 upstream files) and whitespace checks passed. It supplies no new runtime acceptance for these tasks.
