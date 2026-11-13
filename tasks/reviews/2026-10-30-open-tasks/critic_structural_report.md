# Stage 7 — Structural review

Structural review completed for FeatureRange 1–58. DetailedRange 59–165 was used as context. Findings: **1 Major, 1 Nit; no Blocker**.

## STRUCT-001 — Major — The linked human-evidence template contradicts the authoritative acceptance criterion

- **Location:** `tasks/reviews/2026-10-30-open-tasks/review_target.md:50`; `docs/validation/human-acceptance-results.template.md:55–56` and `:111`.
- **Affected entries:** OT-010 and OT-012; specifically their SC-008-001 requirement. Their shared SC-012-001 requirement is not affected by the interaction threshold.
- **Discrepancy:** The dossier presents the template and protocol together as baseline material without identifying that the template is obsolete. The template instructs an observer to use `N = PENDING OWNER DECISION` and `NOT EVALUABLE — N UNSET`. This makes a required follow-up artifact unusable as written.
- **Evidence:** `docs/owner-decisions.md:40` sets N = 10 and permits at most nine interactions. `specs/008-observability-operations/spec.md:91` and `docs/validation/human-acceptance.md:11–13` incorporate that decision. The protocol explicitly instructs observers to copy this template at lines 6–8.
- **Structural fix:** Annotate the baseline link with the template reconciliation dependency and identify the protocol’s existing threshold as authoritative. In the separate resolution change, replace the template’s unset-threshold instructions and comparison placeholder with N = 10 and the `< 10` comparison. Keep session and criterion outcomes `NOT RUN` until genuine human evidence exists. No new owner decision is needed.

## STRUCT-002 — Nit — The overview does not distinguish preserved registry task wording from current distribution scope

- **Location:** `tasks/reviews/2026-10-30-open-tasks/review_target.md:57`.
- **Affected entry:** OT-011 / FR-006-008, preserved from `tasks/todo.md:71`.
- **Discrepancy:** The high-level overview lists “public SDK registry publication” alongside current implementation and acceptance work. It does not tell a fresh reader that this category comes from an unreconciled source entry and conflicts with an already recorded owner decision.
- **Evidence:** `docs/owner-decisions.md:49` selects GitHub Releases only and explicitly excludes npm, PyPI, NuGet and Maven publication for now. The linked `docs/runbooks/sdk-packaging.md` and its referenced `docs/runbooks/release.md:3–5` retain that distribution policy. Preserving the original unchecked entry is appropriate; presenting its wording without a current-scope qualifier is the layout problem.
- **Structural fix:** Qualify the overview category as reconciliation of the preserved SDK-publication entry against the GitHub Releases decision, with links to OT-011 and the decision. Preserve its canonical ID and original source location. Do not create a new registry-identity dependency or infer authorization to publish.

## Structure map

| Scope | Structure | Assessment |
|---|---|---|
| Line 1 | H1: Open-task review dossier | One clear document title |
| Lines 3–5 | H2: Review purpose | Review objective and exclusions are explicit |
| Lines 7–15 | H2: Snapshot and scope | Commit, date, inventory, duplicate locations and worktree caveats are usable |
| Lines 17–53 | H2: System and review baseline | Consistent list of linked authorities and evidence; reading/evidence rules follow it |
| Lines 55–58 | H2: Open-work overview | Appropriate summary level; STRUCT-002 applies |
| Lines 59–162, context only | H2: Detailed Implementation Plan; H3: OT-001 through OT-012 | Sequential hierarchy and complete source-entry coverage |
| Lines 163–165, context only | H2: Review deliverables | Defines consolidation and separate resolution boundaries |

There are no skipped heading levels, malformed lists, unclosed fences or broken table syntax in the active range. All 51 direct local Markdown links in the complete dossier resolve to existing files. Twelve contextual source links use `#L…` fragments; their textual line references match the source entries. Renderer-specific navigation to those fragments was not exercised.

The feature-spec standard does not mandate the example heading names. This dossier proposes a review of existing approved specifications, rather than a new architecture. Its purpose, scope, exclusions, evidence limitations and links to the existing specifications/plans/research satisfy the applicable structural requirements. A new architecture section, repeated alternatives analysis, implementation details in FeatureRange, or removal of useful linked context is not required.

## Scope and task coverage

| Dossier entries | Canonical work | Coverage |
|---|---|---|
| OT-001–OT-004 | T014-014–T014-017 | All four source entries and their evidence annotations preserved |
| OT-005 | T014-040 | Fault/security acceptance entry preserved |
| OT-006 | T014-042 | Performance acceptance entry preserved |
| OT-007 | T014-043 | Integrated release-gate entry preserved |
| OT-008 | T014-044 | Traceability entry preserved |
| OT-009 | T033-007 | Redis integrated validation entry and adjacent evidence preserved |
| OT-010 and OT-012 | SC-008-001 / SC-012-001 | Both source locations preserved; they share canonical criteria and do not establish two independent studies |
| OT-011 | FR-006-008 | Original publication entry preserved; STRUCT-002 applies |

The three original task files contain exactly the twelve unchecked locations represented by OT-001–OT-012. No missing OT, duplicate OT label, or canonical-ID renumbering was found. No additional structural finding was identified for OT-001–OT-009. Partial and historical evidence is explicitly distinguished from completed acceptance; the task count does not establish that twelve independent implementation efforts remain.

## Read evidence

Repository-relative paths below are rooted at `/Users/marcelo/Documents/Projects/sagat-octopus-orchestrator`.

Fully read outside the 39-file baseline:

- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/agents/structural.md`
- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/resources/document_standards/feature-spec-standard.md`
- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/docs/codex.md`
- `tasks/reviews/2026-10-30-open-tasks/review_target.md`, lines 1–165
- `tasks/reviews/2026-10-30-open-tasks/review_context.json`, complete static context

Fully read baseline files, 38 of 39:

| Paths | Full ranges |
|---|---|
| `tasks/reviews/2026-10-30-open-tasks/inventory.json` | 1–266 |
| `.specify/memory/constitution.md` | 1–172 |
| `AGENTS.md`; `specs/AGENTS.md` | 1–56; 1–10 |
| `SPECIFICATION-INDEX.md` | 1–45 |
| `docs/traceability.md` | 1–1094 |
| `docs/owner-decisions.md`; `docs/test-strategy.md` | 1–79; 1–75 |
| `docs/sdk-compatibility.md`; `docs/runbooks/sdk-packaging.md` | 1–568; 1–69 |
| `specs/014-workflow-primitives/spec.md`, `plan.md`, `tasks.md`, `research.md` | 1–693; 1–729; 1–267; 1–1793 |
| `specs/033-redis-streams/spec.md`, `plan.md`, `tasks.md`, `research.md`, `validation-report.md` | 1–53; 1–27; 1–30; 1–59; 1–99 |
| `specs/006-sdk-multilanguage/spec.md`, `plan.md`, `tasks.md`, `research.md` | 1–158; 1–123; 1–125; 1–184 |
| `specs/008-observability-operations/spec.md`, `plan.md`, `tasks.md`, `research.md` | 1–144; 1–114; 1–94; 1–210 |
| `specs/012-developer-experience/spec.md`, `plan.md`, `tasks.md`, `research.md` | 1–128; 1–98; 1–78; 1–213 |
| `docs/validation/human-acceptance-results.template.md`; `docs/validation/human-acceptance.md` | 1–190; 1–217 |
| `specs/014-workflow-primitives/evidence/released-v02-20261027/README.md` | 1–37 |
| `specs/014-workflow-primitives/evidence/released-v02-inflight-20261027/README.md` | 1–42 |
| `specs/014-workflow-primitives/evidence/released-v02-applied-20261027/README.md` | 1–41 |
| `specs/033-redis-streams/released-callback-cutover.md` | 1–75 |
| `tasks/todo.md` | 1–121 |

Selectively read under the refined Stage7 reading policy:

- `specs/014-workflow-primitives/validation-report.md`: complete heading inventory; body lines 1–650, 1462–1546 and 3873–4309. This covers the evidence-availability warning, historical release/fault gates, current owner-decision boundary, recent public/released compatibility evidence and unresolved CI corrections. The intervening historical body sections were not fully read.

Additional explicit references inspected:

- `specs/033-redis-streams/evidence/loopback-fixtures-20261028/README.md`: fully read.
- `docs/runbooks/release.md`: fully read.
- `specs/033-redis-streams/evidence/combined-v03-20261027/artifacts.json`: partial textual read plus complete structural enumeration of its 114 artifact paths; all listed files exist. This was not runtime or artifact-content verification.
- Local Markdown destinations throughout the baseline were checked for existence: 185 references inspected, with the sixteen missing occurrences listed below.

A read-only SHA-256 comparison found that the dossier and all 39 baseline files still match the static context: 40 files checked, no missing or changed baseline file.

## Missing reads and limitations

No required baseline file was missing or inaccessible. Fourteen distinct referenced paths were unavailable in this checkout, appearing in sixteen local Markdown links:

| Referencing file and line | Missing repository-relative destination |
|---|---|
| `docs/traceability.md:362` | `tasks/project-completion/integration-notes.md` |
| `docs/sdk-compatibility.md:320` | `tasks/project-completion/evidence/sdk58-current-20261018/reproduce.md` |
| `docs/sdk-compatibility.md:394` | `tasks/project-completion/evidence/sdk-switch-case-boundary-20261018/reproduce.md` |
| `docs/sdk-compatibility.md:405` | `tasks/project-completion/evidence/sdk-expression-limits-20261018/reproduce.md` |
| `docs/sdk-compatibility.md:423` | `tasks/project-completion/evidence/sdk59-current-20261018/reproduce.md` |
| `docs/sdk-compatibility.md:431` | `tasks/project-completion/evidence/sdk-public-gaps-20261018/reproduce.md` |
| `docs/sdk-compatibility.md:450`; `specs/014-workflow-primitives/validation-report.md:2998` | `tasks/project-completion/evidence/sdk60-current-20261018/reproduce.md` |
| `specs/014-workflow-primitives/spec.md:563`; `specs/014-workflow-primitives/plan.md:526` | `tasks/project-completion/leaf-child-source-preservation.md` |
| `specs/014-workflow-primitives/validation-report.md:1866` | `tasks/project-completion/evidence/scoped-blob-20261017/manifest.json` |
| `specs/014-workflow-primitives/validation-report.md:1896` | `tasks/project-completion/evidence/capacity-a1-20261018/manifest.json` |
| `specs/014-workflow-primitives/validation-report.md:1931` | `tasks/project-completion/evidence/child-root-fk-20261018/manifest.json` |
| `specs/014-workflow-primitives/validation-report.md:3293` | `tasks/project-completion/evidence/collection-result-composition-20261018/reproduce.md` |
| `specs/014-workflow-primitives/validation-report.md:3300` | `tasks/project-completion/evidence/collection-discovery-integration-20261018/reproduce.md` |
| `specs/014-workflow-primitives/validation-report.md:3308` | `tasks/project-completion/evidence/collection-report-replay-20261018/reproduce.md` |

`specs/014-workflow-primitives/validation-report.md:3–7` explicitly explains that this historical tree is outside the repository and that claims supported only by it remain historical until rerun. These missing references limit evidence verification, especially for OT-001–OT-008; they do not justify silently accepting the claims or requiring restoration of the entire archived tree. No archived branch/history was opened.

No builds, runtime tests, external links, services, prior reviews or live execution manifest were accessed. Historical test counts remain unverified claims. No files were written and no task, owner decision or canonical identifier was changed.
