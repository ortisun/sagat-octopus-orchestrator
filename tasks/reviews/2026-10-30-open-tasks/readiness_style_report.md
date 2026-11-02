# Stage 9 — Implementation readiness: style review

Reviewed `tasks/reviews/2026-10-30-open-tasks/review_target.md:59–165`, with lines 1–58 as context. Mode: REVIEW. No additional style standard was assigned.

**Result: four Major findings and one Nit; no style Blocker.** These are corrections to implementer-facing wording and evidence attribution. They do not establish runtime defects, close tasks, authorize publication, or replace the preserved task IDs.

## Findings

### STYLE9-001 — Major — The human-evaluation wording reopens a settled threshold

**Affected scope:** `review_target.md:149`, OT-010, SC-008-001 / SC-012-001; original `tasks/todo.md:28`. Related OT-012 at `review_target.md:161` / `tasks/todo.md:72` and `docs/validation/human-acceptance-results.template.md:55–56,111`.

**Evidence:** OT-010 says “agree the numeric UX target.” However:

- `docs/owner-decisions.md:40` records the owner’s decision: N = 10, requiring at most nine counted interactions.
- `specs/008-observability-operations/spec.md:91` and `docs/validation/human-acceptance.md:10–13,105–106,126` apply that decision.
- The results template still says `N = PENDING OWNER DECISION` and defaults the comparison to `NOT EVALUABLE — N UNSET`.

**Editorial rationale:** The imperative presents a completed product decision as an outstanding prerequisite. The template then directs the evaluator to record an incorrect reason for withholding a verdict. This is a wording inconsistency with operational consequences, separate from the genuine absence of human evaluation.

**Concrete wording guidance:** Preserve the original entry, but append a dated clarification such as:

> The SC-008-001 threshold was settled on 2026-10-16: N = 10, so successful diagnosis requires at most nine counted interactions. Remaining work is genuine human evaluation under the existing protocol. Align the blank result template with this threshold before use; retain `NOT RUN` until a session occurs.

Identify OT-010 and OT-012 as two source locations tracking the same criterion pair. Do not imply that they require two separate evaluation programs. Keep SC-012-001 completion and its separately assessed onboarding-time target distinct.

**Readiness distinction:** Editorial reconciliation can resolve the apparent owner dependency. It cannot supply participant evidence.

### STYLE9-002 — Major — Registry identities are presented as the active publication prerequisite despite the settled distribution policy

**Affected scope:** `review_target.md:155`, OT-011, FR-006-008; original `tasks/todo.md:71`.

**Evidence:** The entry directs publication “under confirmed registry identities” and says it is “awaiting registry identity information.” In contrast:

- `docs/owner-decisions.md:49` selects GitHub Releases only; npm, PyPI, NuGet and Maven publication are deferred.
- `docs/owner-decisions.md:25` retains that channel for the later release authorization.
- `specs/006-sdk-multilanguage/research.md:88` records the same decision.
- `docs/runbooks/sdk-packaging.md`, “Releases go to GitHub Releases,” explicitly preserves registry instructions for a later decision.
- `docs/traceability.md:3,12` records GitHub Release distribution evidence. This review did not independently verify those assets.

**Editorial rationale:** “Awaiting identity information” describes an administrative blocker to already-authorized registry work. The governing decision instead excludes that channel for now. A reader could request credentials or prepare publication unnecessarily.

**Concrete wording guidance:** Retain the source entry and add:

> Historical registry-publication entry. The 2026-10-16 owner decision selects GitHub Releases only; external registry publication remains deferred. Reconcile FR-006-008 against the authorized distribution evidence and keep any future registry work conditional on an explicit channel decision.

Do not infer completion of FR-006-008 from this editorial correction or relabel local packaging evidence as registry publication.

**Readiness distinction:** This is reconciliation with an existing owner choice, not a new request for registry approval or identities.

### STYLE9-003 — Major — “Waits cost no worker, thread or connection” overstates the measured guarantee

**Affected scope:** `review_target.md:108`, OT-006, T014-042; original `specs/014-workflow-primitives/tasks.md:195`.

**Evidence:** The task itself correctly requires “zero dedicated workers.” Its evidence sentence drops that qualification: “waits cost no worker, thread or connection,” followed by four threads and eight connections.

The cited performance report is more precise:

- `tests/performance/reports/workflow-primitives.md:16–20,87–103` describes absence of resources **owned per wait**, with shared process threads and connection pools.
- Lines 30–31 and 105–119 explicitly retain scheduler work.
- `specs/014-workflow-primitives/plan.md:80–83` defines acceptance as zero per-wait worker ownership, not zero resource consumption.

**Editorial rationale:** “Cost no” changes a resource-ownership claim into a zero-cost claim. The reported nonzero totals reinforce the ambiguity.

**Concrete wording guidance:**

> In the recorded 11,000-wait fixture, no wait retained a dedicated worker, thread or database connection. The process used four threads and an eight-connection pool; worker-session and wait-lease assertions were zero. Scheduler work and shared resource use remained measurable.

Keep the workload, observation limits and historical attribution. Do not generalize this fixture into a production-capacity guarantee.

### STYLE9-004 — Major — An obsolete performance checkpoint is written as the current implementation state

**Affected scope:** `review_target.md:108`, OT-006, T014-042; original `specs/014-workflow-primitives/tasks.md:195`.

**Evidence:** The evidence states that a 10,000-item scope “costs quadratic work,” cites the 1 h 54 min run and says the fix is “in progress on `feature/014-scale`.”

The linked material contains subsequent, separately scoped checkpoints:

- `tests/performance/reports/workflow-primitives.md:21` explicitly identifies the scheduler correction and labels the preceding measurements historical.
- `specs/014-workflow-primitives/research.md:1656–1663` records the scheduler cost tests after that correction.
- The performance report at lines 1323–1438 distinguishes owner-role diagnostics, failed resource-profile assertions, corrected runtime-role waits and later frozen flat-collection measurements.
- Lines 1437–1438 still retain nested progress, 100,000-instance and release gates.

**Editorial rationale:** Retaining historical failures is correct. Present-tense defect and branch-status language makes that history look like the next current implementation assignment. It obscures both completed incremental work and genuinely outstanding acceptance.

**Concrete wording guidance:** Introduce the existing paragraph as:

> Historical pre-scale checkpoint: the recorded 10,000-item run took 1 h 54 min and exposed G461/G468.

Then add a separately dated pointer:

> Subsequent scheduler and bounded-worker measurements are recorded in the report’s later checkpoints. They retain their own source identities and limitations; nested/low-quota progress, the 100,000-instance workload and integrated acceptance remain open.

Do not overwrite the old failures with later passes or claim the later measurements were rerun on the dossier’s checkout.

**Readiness distinction:** This identifies stale status framing, not a newly verified performance fix or accepted production ceiling.

### STYLE9-005 — Nit — Compressed result notation obscures counts and phase relationships

**Affected scope:** `review_target.md:90,92,94`, OT-004 / T014-017; lines 129,132–141, OT-009 / T033-007. Originals: `specs/014-workflow-primitives/tasks.md:71,73,75` and `specs/033-redis-streams/tasks.md:16,19–28`.

**Evidence:** Examples include `passes1`, `in8.42s`, `coordinator1/0/0`, `and2current`, `dated2026-10-27`, `plus28`, and `and111`. The linked released-profile READMEs explain that old producer/reader results are nested phases, not independent aggregate tests.

**Editorial rationale:** Missing word boundaries and unlabeled slash counts make the retained evidence harder to interpret. This is particularly avoidable where coordinator, nested phase and complete-command durations differ.

**Concrete wording guidance:** Use labeled counts and spaced units, for example:

> Coordinator: 1 passed, 0 failed, 0 ignored; 6.25 s. The nested released-client phase took 2.67 s and is not an additional aggregate test.

For Redis:

> 27 engine configuration cases, 28 Redis cases and 111 existing event regressions.

Preserve the distinctions between actual released-source execution, ordinary current fixtures, phase timings and command timings. The words identifying real released processes are meaningful evidence qualifications, not marketing fluff.

## Coverage of every inventory entry

| Entry / task | Style disposition |
|---|---|
| OT-001 / T014-014 | No independent finding. Bounds and “descendant” terminology are supported by the clause table, which explicitly distinguishes iteration instances from gated nested scopes. |
| OT-002 / T014-015 | No independent finding. Charge-once and terminal/reconciled-drain wording describe defined semantics, not subjective quality claims. |
| OT-003 / T014-016 | No independent finding. Named fixtures and explicit broader-body exclusions support the bounded wording. Historical five-SDK evidence should retain its original count. |
| OT-004 / T014-017 | STYLE9-005. The dated compatibility increments remain distinct proofs; they do not establish full task acceptance. |
| OT-005 / T014-040 | No independent style finding. The named checkpoint, CI run and source revision provide historical attribution. This review did not verify the claim about every fault test’s assertions. |
| OT-006 / T014-042 | STYLE9-003 and STYLE9-004. Ownership wording and current-versus-historical status need correction. |
| OT-007 / T014-043 | No independent finding. The cited CI result is explicitly bound to a historical run/head. Earlier failures must remain recorded as failures. |
| OT-008 / T014-044 | No independent finding. “Fully proven” is governed by linked acceptance criteria and the explicit remaining gaps; it is not sufficient evidence by itself. |
| OT-009 / T033-007 | STYLE9-005. The paragraph correctly distinguishes focused combined-tree evidence from final release-tree gates. |
| OT-010 / SC-008-001, SC-012-001 | STYLE9-001. The numeric decision is settled; human evaluation remains outstanding. |
| OT-011 / FR-006-008 | STYLE9-002. Registry wording conflicts with the recorded distribution policy. |
| OT-012 / SC-008-001, SC-012-001 | No independent defect in the sentence itself. STYLE9-001 applies to its shared criterion tracking and referenced results template. |

## Read evidence

All paths below are relative to `/Users/marcelo/Documents/Projects/sagat-octopus-orchestrator`, except the plugin files.

**Read fully:**

- Plugin persona `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/agents/style.md:1–66` and integration documentation `docs/codex.md:1–117`.
- Dossier `review_target.md:1–165`, `inventory.json:1–266`, and static `review_context.json:1–212`.
- `.specify/memory/constitution.md:1–172`; `AGENTS.md:1–56`; `specs/AGENTS.md:1–10`; `SPECIFICATION-INDEX.md:1–45`.
- `docs/owner-decisions.md:1–79`; `docs/test-strategy.md:1–75`; `docs/runbooks/sdk-packaging.md:1–69`.
- Feature 014: `spec.md:1–693`, `plan.md:1–729`, `tasks.md:1–267`.
- Feature 033: `spec.md:1–53`, `plan.md:1–27`, `tasks.md:1–30`, `research.md:1–59`, `validation-report.md:1–99`, `released-callback-cutover.md:1–75`.
- Feature 006: `spec.md:1–158`, `plan.md:1–123`, `tasks.md:1–125`.
- Feature 008: `spec.md:1–144`, `plan.md:1–114`, `tasks.md:1–94`.
- Feature 012: `spec.md:1–128`, `plan.md:1–98`, `tasks.md:1–78`.
- `docs/validation/human-acceptance.md:1–217` and `human-acceptance-results.template.md:1–190`.
- Feature 014 evidence READMEs: `released-v02-20261027:1–37`, `released-v02-inflight-20261027:1–42`, `released-v02-applied-20261027:1–41`.
- `tasks/todo.md:1–121`.

**Read selectively:**

| File | Exact substantive ranges read |
|---|---|
| `docs/traceability.md` | 1–50, 180–210, 238–259, 311–332, 402–478, 1038–1094 |
| `docs/sdk-compatibility.md` | 1–59, 270–286, 434–568; heading search for navigation |
| `specs/014-workflow-primitives/research.md` | 1236–1324, 1342–1463, 1577–1793; all headings searched |
| `specs/014-workflow-primitives/validation-report.md` | 1464–1546, 4161–4250; heading search used to locate these checkpoints |
| `specs/006-sdk-multilanguage/research.md` | 80–94; heading/publication searches |
| `specs/008-observability-operations/research.md` | 144–158; heading/human-evaluation searches |
| `specs/012-developer-experience/research.md` | 148–169; heading/human-evaluation searches |
| Additional: `tests/performance/reports/workflow-primitives.md` | 1–35, 87–119, 1323–1438; all headings searched |

Thus, **32 of the 39 static reading-list documents were read fully and seven selectively**. The additional performance report was selective. Truncated initial outputs were supplemented for the full-read documents and substantive ranges listed above.

## Limits

No assigned document was inaccessible. Unlisted portions of the seven selective documents, raw execution transcripts/manifests, runtime implementation, CI services and sibling worktrees were not audited. Historical result summaries were checked for wording and consistency, not independently reproduced.

No prior review reports or live execution manifest were opened. No files were written, tests/builds run, services contacted, Git state changed, conversation/memory retrieved, or subagents spawned.
