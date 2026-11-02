# Intermediate Zoom Out Report

**Mode:** REVIEW
**Target:** `tasks/reviews/2026-10-30-open-tasks/review_target.md`, lines 1–165
**Reviewed checkout:** `888074e6237b2a1ce9c85f83ec5703b081c65dd6`
**Result:** 0 Blockers, 2 Majors, 3 Nits. No prior finding is resolved.

The dossier provides enough context to understand its purpose: review twelve unchecked entries against their existing requirements and evidence. It does not need another feature specification, a replacement implementation plan, or new architecture.

The principal corrections concern stale instructions that would reopen settled owner decisions. Public registry publication is outside the approved distribution policy, and the human diagnosis threshold has already been selected. The remaining corrections concern current SDK wording, performance evidence summaries, and the exact scope of the snapshot.

The existing architecture supplies the relevant extension points: the shared six-language conformance harness, GitHub release packaging, PostgreSQL scope/frontier operations, worker authorization paths, and broker publisher interfaces. The review proposals should reuse these. Accepted requirements for nested scopes, low-quota behavior, crash recovery, tenant isolation, and cold transport cutover are not speculative complexity.

## Findings

### IZO-001 — Major: The distribution entry reopens a settled owner decision

**Affected scope:** OT-011 / FR-006-008; target `review_target.md:57`, `:151–155`; source `tasks/todo.md:71`. Related baseline inconsistencies include `docs/traceability.md:194` and `SPECIFICATION-INDEX.md:7`, `:36`.

**Evidence and rationale:** The dossier describes public SDK registry publication as remaining work and retains an entry waiting for registry identities. The governing owner decision specifies GitHub Releases for packages and binaries, checksums, and signatures, with no npm, PyPI, NuGet, or Maven publication at this stage (`docs/owner-decisions.md:49`). The packaging runbook preserves that boundary and makes registry instructions conditional on a later decision (`docs/runbooks/sdk-packaging.md:41–43`). Rust’s crate archive is also a GitHub asset; crates.io is not enabled (`:69`).

FR-006-008 requires versioned SDK packaging, runtime compatibility, and runnable examples; it does not require publication to those registries (`specs/006-sdk-multilanguage/spec.md:73`). The historical research itself records the subsequent GitHub-only decision (`research.md:87–88`).

Acting on the stale entry would create unnecessary credential, publishing, and operational work. It would change product scope rather than close an established packaging requirement.

**Simpler correction and reuse:** Preserve the original task wording as captured evidence, but identify it explicitly as superseded where the dossier describes actionable work. Reconcile currently operative pending text with the owner decision and the existing GitHub release process. Reuse `.github/workflows/release.yml:160–193` and `scripts/package-sdks.sh:485–515`; these already contain release artifact, checksum, provenance, and Rust packaging integration.

The documented 0.2.0 GitHub distribution evidence can support a reconciliation, but this review does not independently verify those remote artifacts.

**Observable closure:** The dossier and operative task references consistently state the approved GitHub distribution scope, identify any actual remaining artifact or evidence gap, and no longer make registry identities a prerequisite. Historical records remain historical. No registry publication or new owner decision is necessary to close this documentation finding.

### IZO-002 — Major: The human acceptance entry and results template ask for an already selected threshold

**Affected scope:** OT-010 and OT-012 / SC-008-001 and SC-012-001; target `review_target.md:145–149`, `:157–161`; source `tasks/todo.md:28`, `:72`; results template `docs/validation/human-acceptance-results.template.md:55–56`, `:111`.

**Evidence and rationale:** OT-010 still asks to agree a numeric UX target. The required results template says the threshold is pending an owner decision and consequently not evaluable. However, the owner has selected **N = 10**, meaning diagnosis must take fewer than ten interactions, or at most nine (`docs/owner-decisions.md:40`). The protocol and observability specification already contain that threshold (`docs/validation/human-acceptance.md:11–13`; `specs/008-observability-operations/spec.md:91`).

The protocol requires use of the results template (`human-acceptance.md:6–8`). The stale template therefore affects the actual evidence collection path, rather than merely an optional historical note.

The diagnosis interaction criterion and the newcomer onboarding criterion remain distinct. The latter concerns completing the TypeScript and C# workflow using the prescribed documentation and timing protocol (`specs/012-developer-experience/spec.md:84–88`). Selecting N does not establish either human result.

**Simpler correction and reuse:** Correct the existing template and pending wording to use the settled diagnosis threshold. Retain the existing protocol and results structure. Treat OT-010 and OT-012 as two recorded references to shared outstanding evidence, while preserving both acceptance criteria. Do not invent another study process or reopen threshold selection.

**Observable closure:** The task description, protocol, specification, and template agree on N = 10 and the maximum of nine diagnosis interactions. The template continues to show sessions as not run until genuine participant evidence exists. Correcting these documents closes this finding; it does not complete either human acceptance criterion.

### IZO-003 — Nit: Current conformance instructions retain obsolete five-SDK wording

**Affected scope:** OT-003, OT-004, OT-007, and OT-008; target `review_target.md:81`, `:88`, `:114–122`; T014-016, T014-017, T014-043, and T014-044. Relevant source instructions include `specs/014-workflow-primitives/spec.md:381–383` and `plan.md:200–202`.

**Evidence and rationale:** Some still-operative wording requires all five SDKs. The constitution requires six official SDKs (`.specify/memory/constitution.md:50–51`). Feature 034 explicitly requires the existing shared harness and distinguishes historical five-language evidence from new six-language results (`specs/034-rust-sdk/spec.md:24–30`).

Code inspection confirms that the common harness already enumerates six languages, including Rust (`tests/contract/sdk-conformance/harness.rs:35–42`). It also supports explicit language filtering (`:67–98`), so a filtered run cannot silently stand for the full matrix.

Historical five-language receipts are valid descriptions of their original executions. They should not be relabeled as six-language results. Likewise, selected new scenarios run across six languages are not evidence for the entire current matrix.

**Simpler correction and reuse:** Clarify current obligations as six-language obligations and retain the original language counts in historical receipts. Use the existing common harness and existing final integrated gate. Do not add a Rust-specific parallel conformance system or reopen a completed historical task solely to rewrite its receipt.

**Observable closure:** Operative acceptance instructions agree on the six-language scope; dated receipts retain their actual source revision, scenario selection, and language selection. Future acceptance evidence identifies those selections explicitly.

**Reconciliation of severity:** ZO-003 remains substantively open, but Major overstates the architectural consequence. The governing six-language requirement and reusable implementation already exist. I agree with CRIT-003’s Nit classification: the demonstrated defect is inconsistent current documentation, not a demonstrated absence of Rust conformance infrastructure.

### IZO-004 — Nit: The performance summary presents an older bottleneck as the current explanation

**Affected scope:** OT-006 and OT-008 / T014-042 and T014-044; target `review_target.md:103–108`, `:117–122`; source `specs/014-workflow-primitives/tasks.md:194–199`.

**Evidence and rationale:** The copied summary emphasizes the earlier quadratic G461 run, approximately 1 hour 54 minutes, and a fix still in progress. The linked performance history contains subsequent scheduler/frontier changes, corrected measurement procedures, runtime-role measurements, and later flat-width executions (`tests/performance/reports/workflow-primitives.md:1128–1438`). The linked research also records accepted bounded-path changes (`specs/014-workflow-primitives/research.md:1577–1793`).

Actual code provides reuse evidence: the PostgreSQL frontier store implements the relevant boundary (`crates/infra-postgres/src/scope_frontier.rs:276`); scoped claims use the current-path operation (`crates/application/src/worker/scoped_claims.rs:73`); scoped worker processing uses that same established path (`crates/application/src/worker/scoped.rs:19`). The scheduler deliberately retains whole-run work at specified scope transitions (`crates/scheduler/src/passes.rs:667–674`). Removing every whole-run operation would not follow from the evidence.

The later measurements still have material limits. The runtime flat-workload manifest identifies source `b1855cbe`, a copied binary, and a serial in-process driver. Logical width is not proof of that many external workers running concurrently. The waits record identifies source `60dfc824` and does not provide isolated PostgreSQL RSS. Neither is a result for the reviewed HEAD. Nested low-quota work, broader scale requirements, and final integrated acceptance remain open.

**Simpler correction and reuse:** Keep the old failure as history, then add a short current summary pointing to the later evidence and the remaining unproven requirements. Continue from the existing frontier and worker paths. Do not introduce caches, shards, new queues, or a new scheduling subsystem merely because an older summary still describes the original failure.

**Observable closure:** The dossier distinguishes the original failure, subsequent changes and measurements, and remaining acceptance gaps. It names the relevant source revisions and measurement limitations. Production ceilings remain unaccepted. Updating this summary does not require rerunning a benchmark; completing T014-042 still requires its existing evidence.

**Reconciliation of severity:** ZO-004’s correction remains justified, but I classify it as Nit here. The target already links the performance record, leaves the task unchecked, and does not accept production ceilings. No new optimization is actually proposed by the dossier. The immediate defect is a stale explanatory summary.

### IZO-005 — Nit: The purpose sentence is broader than the defined snapshot

**Affected scope:** All OT entries; target `review_target.md:5`, contrasted with `:10–15`.

**Evidence and rationale:** “Every unchecked task” in the current checkout can be read as an exhaustive repository-wide inventory. The dossier actually defines a fixed review snapshot: twelve unchecked entries from three files, identified through eighteen canonical task inventories, at a recorded revision. It also explains exclusions and the separate status of sibling worktrees.

The narrower scope is sufficient and sensible. The opening sentence should state it directly so the aggregate review does not acquire unrelated archive, checklist, or worktree obligations.

**Simpler correction and reuse:** Describe the review as the twelve captured unchecked entries from the identified canonical inventories at the recorded revision. Keep the existing inventory and exclusions.

**Observable closure:** The opening purpose agrees with the inventory’s revision, file set, and twelve-entry count. No inventory expansion or implementation work is implied.

## Reconciliation of earlier review actions

All earlier reports were read, including the complete linked critic persona reports. No target or baseline correction was applied between these stages.

| Earlier action | Intermediate disposition | Reason |
|---|---|---|
| ZO-001 — registry publication | Still open → IZO-001, Major | Contradicts the explicit GitHub-only decision. |
| ZO-002 — human threshold | Still open → IZO-002, Major | Both pending wording and the required results template remain stale. |
| ZO-003 — five versus six SDKs | Still open; justified disagreement on severity → IZO-003, Nit | Current wording needs correction, but the six-language authority and common harness already exist. |
| ZO-004 — performance summary | Still open; justified disagreement on severity → IZO-004, Nit | Later evidence changes the summary; the dossier nevertheless retains the acceptance gap and proposes no new optimization. |
| COMP-001 — distribution contradiction | Still open → IZO-001 | Comprehension and authority correction agree. |
| COMP-002 — human threshold contradiction | Still open → IZO-002 | The prescribed execution template still conflicts with the settled criterion. |
| CRIT-001 — distribution | Still open → IZO-001 | Includes DESIGN-001, STRUCT-002, and STYLE7-002. Reconcile authority without restoring an entire obsolete archive. |
| CRIT-002 — human acceptance | Still open → IZO-002 | Includes DESIGN-002 and STRUCT-001. Correct the existing evidence collection path. |
| CRIT-003 — SDK wording | Still open → IZO-003 | DESIGN-003’s Nit classification matches the demonstrated defect. |
| CRIT-004 — scope wording | Still open → IZO-005 | STYLE7-001 correctly distinguishes the canonical snapshot from an unrestricted repository audit. |

The absence of a performance finding in the comprehension or critic reports does not resolve ZO-004. Those reports applied different criteria. Conversely, a prior finding’s severity is not evidence that additional architecture or runtime work is required.

## Coverage of OT-001–OT-012

| Entry and canonical task | High-altitude assessment |
|---|---|
| **OT-001 — T014-014**, `specs/014-workflow-primitives/tasks.md:63` | No additional finding. Collection, nested-scope, low-quota, and reservation behavior are explicit requirements. Their remaining tests are justified; they are not optional complexity to remove. |
| **OT-002 — T014-015**, `tasks.md:65` | No additional finding. Typed admission, ordered aggregation, fail-fast drain, logical charging, and slot release fit the existing domain and transactional boundaries. Durable obligations must not be confused with active execution capacity. |
| **OT-003 — T014-016**, `tasks.md:67` | IZO-003 applies. Preserve the required collection wiring and builders; clarify the current six-language obligation using the existing harness. |
| **OT-004 — T014-017**, `tasks.md:69` | IZO-003 applies. Existing released-version profiles are bounded evidence. They do not complete the full crash, duplicate, stale-fence, broker-loss, cancellation, and compensation matrix. |
| **OT-005 — T014-040**, `tasks.md:189` | No additional finding. The fault, tenant, and secret matrix is justified by the platform guarantees. The recorded 87-test result is historical evidence, not acceptance of the current integrated tree. |
| **OT-006 — T014-042**, `tasks.md:194–195` | IZO-004 applies. Preserve the original workload and resource obligations; update the summary of what has already been investigated and what remains unproven. |
| **OT-007 — T014-043**, `tasks.md:196–197` | IZO-003 applies. Reuse the existing integrated checks and release process. Earlier CI results cannot substitute for the eventual final-tree gate. |
| **OT-008 — T014-044**, `tasks.md:198–199` | IZO-003 and IZO-004 apply. Traceability must distinguish current obligations from historical language counts, performance measurements, and unresolved ceilings. |
| **OT-009 — T033-007**, `specs/033-redis-streams/tasks.md:15` | No additional finding. Redis fits the existing publisher boundaries and remains optional transport. PostgreSQL authority, stable identifiers, and cold-cutover requirements remain necessary. Selected released profiles and selected six-SDK scenarios do not establish final integrated release acceptance. |
| **OT-010 — SC-008-001 / SC-012-001**, `tasks/todo.md:28` | IZO-002 applies. Threshold selection is settled; genuine human evidence remains outstanding. |
| **OT-011 — FR-006-008**, `tasks/todo.md:71` | IZO-001 applies. Reconcile against GitHub distribution and existing packaging evidence rather than arranging registry publication. |
| **OT-012 — SC-008-001 / SC-012-001**, `tasks/todo.md:72` | IZO-002 applies. Preserve this source location while linking shared evidence with OT-010. Both distinct acceptance criteria still need genuine results. |

IZO-005 applies to the framing of all twelve entries.

## Architectural fit and reuse assessment

The target’s detailed task wording is appropriate for an evidence dossier. It records existing implementation obligations rather than introducing helper-level pseudocode into a new architectural proposal. I found no additional altitude violation requiring that detail to be removed.

Code searches and focused reads confirmed these reuse paths:

| Concern | Existing implementation inspected | Implication |
|---|---|---|
| SDK conformance | `tests/contract/sdk-conformance/harness.rs:1–110` | One common six-language harness exists. Preserve shared semantics and record filtering explicitly. |
| Distribution | `.github/workflows/release.yml:130–205`; `scripts/package-sdks.sh:485–515` | Existing GitHub release and packaging machinery addresses the approved delivery model. |
| Bounded scope work | `crates/infra-postgres/src/scope_frontier.rs:260–330`; `crates/application/src/worker/scoped_claims.rs:45–115`; `crates/application/src/worker/scoped.rs:1–125` | Continue through existing transactional and authorization paths; a new scheduling subsystem is not justified by the stale summary. |
| Scope-transition oracle | `crates/scheduler/src/passes.rs:650–721` | Some whole-run work is deliberate at bounded lifecycle points. “Eliminate all full-run reads” would be an unjustified redesign. |
| Redis publishing | `crates/infra-connectors/src/redis.rs:150–205`; `crates/infra-connectors/src/publish.rs:55–110` | Redis already implements established publishing interfaces. |
| Durable dispatch | `crates/scheduler/src/outbox.rs:60–115`; `crates/scheduler/src/deliveries.rs:80–145` | Existing outbox and delivery abstractions supply the architectural integration point. A second orchestration or delivery framework is unnecessary. |

These reads establish the presence and intended role of reusable components. They do not establish runtime correctness, race safety, or current release readiness.

The remaining accepted feature obligations should not be removed as “YAGNI.” The owner explicitly retained the feature 014 work and approved the Rust and Redis dependencies. Similarly, a correction to the review dossier should not grow into a demand for a new ADR, new product specification, registry rollout, or new human study framework.

## Read and inspection evidence

### Required review material

Read fully:

- Installed `agents/zoom_out.md`, `resources/document_standards/zoom-out-standard.md`, and `docs/codex.md`.
- Target `review_target.md:1–165`, including both designated ranges.
- `review_context.json`.
- `zoom_out_report.md`, `comprehension_report.md`, `critic_report.md`.
- Complete linked `critic_design_report.md`, `critic_structural_report.md`, and `critic_style_report.md`.

### Static baseline: 32 of 39 files read fully

| Files | Full ranges |
|---|---|
| `inventory.json` | 1–266 |
| `.specify/memory/constitution.md`; root `AGENTS.md`; `specs/AGENTS.md`; `SPECIFICATION-INDEX.md` | 1–172; 1–56; 1–10; 1–45 |
| `docs/owner-decisions.md`; `docs/test-strategy.md`; `docs/runbooks/sdk-packaging.md` | 1–79; 1–75; 1–69 |
| Feature 014 `spec.md`, `plan.md`, `tasks.md` | 1–693; 1–729; 1–267 |
| Feature 033 `spec.md`, `plan.md`, `tasks.md`, `research.md`, `validation-report.md`, `released-callback-cutover.md` | 1–53; 1–27; 1–30; 1–59; 1–99; 1–75 |
| Feature 006 `spec.md`, `plan.md`, `tasks.md` | 1–158; 1–123; 1–125 |
| Feature 008 `spec.md`, `plan.md`, `tasks.md` | 1–144; 1–114; 1–94 |
| Feature 012 `spec.md`, `plan.md`, `tasks.md` | 1–128; 1–98; 1–78 |
| Human results template and acceptance protocol | 1–190; 1–217 |
| Feature 014 released-v02, released-v02-inflight, and released-v02-applied evidence READMEs | 1–37; 1–42; 1–41 |
| `tasks/todo.md` | 1–121 |

All three source files containing the twelve open entries were read fully. The applicable feature specifications and plans were also read fully.

### Seven baseline files read selectively

Headings and relevant search matches were inspected in addition to these body ranges:

| File | Body ranges read |
|---|---|
| `docs/traceability.md` | 1–32, 181–204, 238–259, 311–332, 382–433, 1005–1094 |
| `docs/sdk-compatibility.md` | 1–118, 434–568 |
| `specs/014-workflow-primitives/research.md` | 1–213, 1244–1298, 1342–1463, 1577–1793 |
| `specs/014-workflow-primitives/validation-report.md` | 1–13, 1462–1546, 1800–1819, 4161–4309 |
| `specs/006-sdk-multilanguage/research.md` | 75–110 |
| `specs/008-observability-operations/research.md` | 144–185 |
| `specs/012-developer-experience/research.md` | 130–180 |

Unlisted body sections were not fully reviewed. An attempted broader read of feature 006 research was truncated and is not counted as a full read.

### Additional linked material

Read fully:

- Feature 034 `spec.md`, `plan.md`, and `tasks.md`.
- `docs/runbooks/release.md:1–52`.
- `docs/releases/0.2.0.md:1–101`.
- ADR 0069 and ADR 0070.
- Feature 014 `evidence/sdk64-20261027/README.md`.
- `tests/performance/reports/raw/workflow-primitives/20261017-runtime-flat/manifest.json`.
- `tests/performance/reports/raw/workflow-primitives/20261017-waits-runtime/result.json`.

Read selectively:

- Root `README.md:1–95`.
- Feature 014 `implementation-ledger.md:1579–1678`, plus relevant headings and search matches.
- `tests/performance/reports/workflow-primitives.md:1–85`, `:1128–1438`, plus headings.
- The implementation ranges listed in the reuse table.

Searches covered owner distribution decisions, packaging and release paths, human thresholds and result states, five/six-language conformance instructions, performance gaps and later decisions, frontier/current-path implementations, and Redis publisher/outbox integration.

### Snapshot verification

Read-only verification returned:

- `git rev-parse HEAD`: `888074e6237b2a1ce9c85f83ec5703b081c65dd6`.
- SHA-256 comparison of the target plus all 39 baseline files: **40 checked, zero mismatches**.
- `tasks/project-completion` does not exist in this checkout.

This confirms that the supplied static target and baseline have not incorporated prior mitigations. The missing archive is a documentation-link limitation; restoring the entire archive is not a necessary architectural remedy. I did not independently re-enumerate the structural reviewer’s reported count of fourteen missing destinations and sixteen references.

## Limits of this review

No files were written, tests or builds run, services started, network resources accessed, git state changed, memory retrieved, or subagents launched.

Historical CI, package, SDK, benchmark, and released-version results remain documentary evidence tied to their recorded revisions and conditions. They were not rerun or remotely verified. The current code reads establish available reuse, not proven integrated acceptance.

Sibling worktree contents were not inspected. Their existence does not make their work part of this reviewed checkout or establish release acceptance. Large histories were inspected only within the stated ranges; this report does not claim a full audit of every historical receipt or transitive link.

The document corrections above are separable from completing the twelve underlying entries. None authorizes a task checkbox change, a new publication target, acceptance of production ceilings, or a claim of completed human validation.
