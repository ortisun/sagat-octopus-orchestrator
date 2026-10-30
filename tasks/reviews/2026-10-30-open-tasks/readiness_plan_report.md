# Stage 9 — Implementation readiness: plan review

**Implementation-readiness verdict: Conditional.** An unfamiliar engineer can perform the next bounded capacity, upgrade and integrated-verification work using the existing proof plans. The inventory does not authorize completing general nesting, accepting production ceilings, publishing to registries or substituting automation for human evaluation.

There are **two Major findings, one Nit and no Blocker finding**. The remaining acceptance gates are not, by themselves, newly discovered implementation defects.

Review basis: `develop@888074e6237b2a1ce9c85f83ec5703b081c65dd6`; target DetailedRange 59–165, with FeatureRange 1–58 as context. This review made no changes and ran no tests. Test outcomes below are attributed to retained evidence, not independently reproduced results.

## READINESS-001 — Major — Registry publication contradicts the recorded distribution decision

- **Entries:** OT-011 / FR-006-008.
- **Locations:** `tasks/todo.md:71`; review target:155; `docs/traceability.md:194`.
- **Controlling evidence:** `docs/owner-decisions.md:49` chooses GitHub Releases only; `docs/owner-decisions.md:26` explicitly excludes adding registry publication. `docs/runbooks/sdk-packaging.md:41–55` says registry instructions are retained for a later decision and are not run. Its Rust section, lines 57–69, likewise includes `.crate` release assets without enabling crates.io.
- **Readiness problem:** The outstanding task directs an engineer to publish to registries once identities arrive, while the accepted release scope directs distribution through GitHub Releases. Identity information would not resolve that scope conflict. This is a contradictory next action, rather than proof that packages or release tooling are absent.
- **Concrete guidance:** Reconcile `tasks/todo.md` and the FR-006-008 traceability row with the existing decision. Separate verification of the approved GitHub distribution from deferred registry publication while retaining FR-006-008 and historical evidence. Reference `scripts/package-sdks.sh`, `scripts/package-rust-sdk.sh`, `docs/runbooks/sdk-packaging.md` and the release workflow as the executable path. A future registry publication requires a changed owner decision; do not request the same already-settled channel decision again merely to proceed with approved packaging verification.
- **Observable closure:** The active task names the approved channel, exact coordinated version/source, distributable hashes/provenance and clean-consumer checks. Any completed publication claim points to actual immutable assets and verification evidence. Registry publication remains explicitly deferred unless separately authorized.

## READINESS-002 — Major — The human evidence template reinstates an already-resolved acceptance threshold

- **Entries:** OT-010 and OT-012 / SC-008-001, SC-012-001.
- **Locations:** `tasks/todo.md:28`; review target:149; `docs/validation/human-acceptance-results.template.md:55–56,111`.
- **Controlling evidence:** `docs/owner-decisions.md:40` sets N = 10. `docs/validation/human-acceptance.md:10–13,104–106` requires fewer than 10 counted interactions: at most nine. The template instead instructs the observer to record `N = PENDING OWNER DECISION` and `NOT EVALUABLE — N UNSET`.
- **Readiness problem:** Following the mandated template produces an unevaluable SC-008-001 record despite an established criterion. OT-010 also incorrectly presents threshold agreement as remaining work. OT-010 and OT-012 refer to the same two criteria; they are not independent requests for duplicate studies.
- **Concrete guidance:** Correct the template and task wording to inherit the recorded threshold. Cross-reference OT-010/OT-012’s shared evidence obligation. Preserve the existing protocol’s participant eligibility, unaided operation, interaction counting and retention of failed/aborted/invalid sessions. SC-012-001 already requires one eligible newcomer to complete both TypeScript and C# runs unaided; the separate timing target is strictly under 15 minutes (`human-acceptance.md:197–208`).
- **Observable closure:** Protocol, template, task wording and specification use consistent criteria. Genuine participant records then identify the tested commit/environment, raw interaction counts, diagnosis, both onboarding run outcomes, elapsed time and validity decisions. Review proceeds under `human-acceptance.md:210–217`. Correcting the documents alone closes this finding, not either human acceptance criterion.

## READINESS-003 — Nit — Canonical entry summaries do not reliably route the next engineer to the current proof frontier

- **Entries:** Primarily OT-001–OT-008 / T014-014–017, T014-040, T014-042–044.
- **Locations:** `specs/014-workflow-primitives/tasks.md:51–58,71–75,195,253–267`; corresponding review target:82,90–94,108,115.
- **Evidence:** The phase introduction still describes Activity→Return as the public collection slice and broader compensation as outstanding. Later evidence records public control/timer/event increments and compensation. The Applied note leaves quota/drain/original-forward compensation open, while `implementation-ledger.md:1559–1576` and `release-upgrade-output-quota.md:78–81` close that bounded proof and identify the narrower remaining authority/uncertain-COMMIT compositions. The performance note directs work toward a 1 h 54 min scale problem, whereas `tests/performance/reports/workflow-primitives.md:1393–1438` records corrected 20,000-wait and three 10,000-item runtime-role executions, with explicit limits.
- **Readiness problem:** Historical evidence is correctly preserved, but several prominent task summaries resemble current next-work instructions. This can cause duplicate work or reopening settled questions. Existing ledgers and bounded plans mitigate the problem; it is not evidence that the architecture must be reinvented.
- **Concrete guidance:** Add concise current-frontier pointers beside the canonical entries, preserving the historical notes. Identify the remaining clause, current proof plan, pinned branch if unintegrated, next command/test family and closure condition. Resolve `tasks/project-completion/todo.md` through the pinned handoff documented in `capacity-eligibility.md:17–20`; its absence from this checkout is not evidence that the plan is missing.
- **Observable closure:** Starting from each affected task reaches the applicable current plan without treating old passing results as current-head acceptance or already-proven increments as missing implementation. Original task IDs and unchecked acceptance boundaries remain intact.

## Per-entry readiness matrix

“Ready” concerns the next action, not completion of the entire criterion. “Conditional” means the next action is specified but its prerequisite integration or proof remains outstanding.

| Entry and original task | Status | Exact next action and dependency |
|---|---|---|
| **OT-001 — T014-014** | **Conditional** | Execute and retain the native Acquire/recovery matrix described in `capacity-acquire.md:100–131`, followed by the selected integrated coordinator proofs. Reuse existing bounds, snapshots and budget tests; do not recreate them from the checkbox. Remaining nested evidence includes descendant progress at limit one, no fresh-admission check for joins, lifetime reservations and reduction-to-zero behavior (`capacity-eligibility.md:55–70`). These require actual creator/join/runtime wiring before general nesting acceptance. |
| **OT-002 — T014-015** | **Conditional** | Continue from existing reducer/admission/drain evidence into capacity possession and integration. The pinned coordinator plan orders lease guarding, exact eligibility evidence, bounded sweep progress and caller wiring. Domain possession policy and native operations already have implementation. Creator/membership, scoped-path and retention/backfill obligations remain distinct prerequisites; do not enable groups merely because the guard passes. |
| **OT-003 — T014-016** | **Conditional** | Preserve captured-source/blob and common-builder evidence. Complete the selected capacity/runtime increment before broadening public nesting. For the next broader source/body family, state its accepted graph, exact transaction and negative cases in the existing bounded proof-plan structure, then modify the relevant application/PostgreSQL primitive paths and shared conformance fixtures. Historical five-language outputs do not replace current six-SDK release evidence. |
| **OT-004 — T014-017** | **Conditional** | Run the real six released-source profiles against the chosen integrated source, then address the remaining authority/uncertain-COMMIT compositions identified by the current ledger. Actual Applied quota/drain/ordered compensation and dispatcher/callback cutover are already retained bounded evidence. Keep original released producer/reader identity, receipt/accounting assertions, both race orders and failure artifacts. General nesting and the full fault matrix remain separate. |
| **OT-005 — T014-040** | **Conditional** | Execute the named fault/security matrix on the integrated runtime slice, using `contracts/conformance.md`’s fault points and `validation-report.md:1496–1517`’s persistence assertions. Include normal and applicable `sagat_app` execution; retain failures, skipped cases and source identity. The historical 87/87 checkpoint is not evidence for every subsequently added path. |
| **OT-006 — T014-042** | **Conditional** | Reproduce and report the existing flat/wait/tenant workloads on the selected source when correctness prerequisites pass. Extend to the required 100,000 nested-instance workload only after a real supported nested path exists. Record hardware, configuration, queue and SQL measurements, resource budgets and failures. The existing 10k flat and 20k wait evidence is usable within its recorded limits; it neither disappears because the task is open nor accepts production ceilings. |
| **OT-007 — T014-043** | **Conditional** | Run the complete repository/CI checklist on the integrated commit, including corrected capacity/migration preflights, real upgrade profiles, contracts, SDK checks, Compose and advisories. Update operator/schema/API/quickstart guidance to the actually supported shapes and rollback limits. Current pending verification is explicit in `implementation-ledger.md:1654–1678` and the pinned coordinator plan:99–110. |
| **OT-008 — T014-044** | **Ready** | Reconcile the current clause-to-evidence index now, including the superseding bounded proofs and branch locations identified above. Files are already named: `docs/traceability.md`, feature014 tasks/validation and dependent documentation. Final criterion closure remains conditional on OT-001–007’s applicable evidence. Documentation work must preserve missing/failed evidence and capability restrictions. |
| **OT-009 — T033-007** | **Conditional** | Run final integrated release-tree gates after choosing the actual merge source. The callback cutover plan and test fixtures already exist; no new Redis design is required. Include real messaging/Redis regressions, six released-source profiles and the complete current six-language common suite. Update feature033 validation/traceability/roadmap dependencies with obtained results. The retained 139 regressions, five new scenarios in six SDKs and cutover checkpoint are partial combined-tree evidence, not final release acceptance. |
| **OT-010 — SC-008-001 / SC-012-001** | **Human evidence** | Apply READINESS-002’s document correction, then arrange genuine sessions under the existing protocol. Numeric targets are settled. Share the resulting criterion records with OT-012 rather than inventing a second evaluation requirement. |
| **OT-011 — FR-006-008** | **Needs refinement** | Apply READINESS-001 before acting on the publication wording. Approved GitHub package/consumer verification can proceed using existing tooling. Registry publication is deferred scope, not an action unlocked solely by receiving identities. |
| **OT-012 — SC-008-001 / SC-012-001** | **Human evidence** | Use the corrected template for actual operator/newcomer sessions, retain unsuccessful/invalid records, and obtain the protocol’s review of results. No simulation, automated timing result or task-checkbox edit substitutes for the required evidence. |

## Executable order and source boundaries

1. **Reconcile the two conflicting instructions and the task routing.** This is bounded documentation work in `tasks/todo.md`, the human-results template, `docs/traceability.md` and feature014’s current-frontier notes. No new feature specification is required.

2. **Select the exact capacity integration source before modifying shared code.** The snapshot coordinator is `feature/014-capacity-coordinator@ffe54ab41e0fafbaaae8c298ce3e87bee22aa89d`, based on the reviewed develop head. Its plan records O705/O706 as already answered: a scheduler-tick sweeper with separate native-operation transactions; possession gates new worker leases, not existing reports/heartbeats/internal progress. Do not treat those decisions as unanswered merely because their branch has not been integrated.

   The coordinator diff identifies concrete implementation surfaces: `crates/application/src/{execution.rs,worker.rs,worker/scoped_claims.rs}`, domain `execution/capacity.rs`, PostgreSQL `execution/{path.rs,worker_capacity.rs}` and `execution.rs`, the worker-membership index migration, and `capacity_worker_leases.rs` / `capacity_worker_preflight.rs`. Existing branch ownership notes matter because other worktrees touch shared files.

3. **Verify native and guarded-claim work before caller activation.** The current Acquire plan names 17 native cases, including held-lock contention in both orders, lost COMMIT acknowledgement, real deferred-FK abort, immutable replay after continuation/Finish, audit/outbox rollback and unchanged existing leases. The coordinator records focused passing evidence but still requires real six-profile and complete final CI after its schema-oracle correction. Review reports and lightweight source checks do not substitute for those executions.

4. **Complete the remaining bounded coordination designs in their existing order.** The coordinator explicitly separates shared exact worker eligibility from sampled candidate discovery, then bounded sweep discovery/native progress, then scheduler caller wiring. Further creator/member insertion, globally unambiguous membership, scoped paths, quota/backfill/retention and recovery acceptance remain activation prerequisites. Existing D704/D705 settle tenant reduction and descendant-join semantics. Those obligations do not justify a new tenant queue, pausing admitted work or speculative group activation.

5. **Run fault, upgrade, SDK and resource acceptance on the integrated source.** Use `.github/workflows/ci.yml` and repository fixtures. For changed behavior, preserve real failing-first tests and use locks/barriers for races. Do not combine unrelated historical successful runs into a final aggregate, bless unseen outputs, or count a cancelled/incomplete workflow as passing.

6. **Close evidence last.** The closure record needs the requirement, named test/command, exact source/configuration, actual outcome, inspected persistence, contract, telemetry/audit and demonstration. Genuine human evidence remains a separate stream. Production ceilings and final release acceptance remain unapproved until their specified evidence and owner-controlled scope decisions are satisfied.

## Important non-findings

- An unchecked T014 task is not proof that its described collection implementation is absent.
- Actual released Applied quota/drain/compensation and scoped dispatcher/callback cutover proofs exist. Their remaining gates must be named narrowly.
- The latest recorded flat and wait measurements supersede the old “scale fix in progress” routing within their stated scope; they do not prove nested scale or an unmeasured latency SLO.
- The coordinator’s unintegrated implementation/proof is not missing work to recreate on develop.
- The absent checkout path `tasks/project-completion/` is recoverable through the documented pinned handoff. It was not classified as missing evidence solely from the failed local-path lookup.
- No fresh production-code defect was established by this readiness review.

## Read evidence and limits

Full reads:

- EGM `agents/implementation_readiness.md`, `resources/document_standards/implementation-readiness-standard.md` and `docs/codex.md`.
- Review target 1–165, `inventory.json` 1–266 and static `review_context.json` 1–212.
- Constitution; root/specs AGENTS; specification index; `docs/owner-decisions.md`; `docs/test-strategy.md`; `docs/runbooks/sdk-packaging.md`; `tasks/todo.md`.
- Applicable **spec.md, plan.md and tasks.md** for features **006, 008, 012, 014 and 033**.
- Feature033 research, validation and released-callback-cutover.
- Human acceptance protocol and results template.
- Feature014 capacity Acquire, Vacate/Acquire, eligibility and discovery plans; three released-upgrade profile plans; released output-quota plan; the three original released-v02 evidence READMEs.
- Pinned coordinator plan and ADR-0075 at `ffe54ab…`.
- Current PostgreSQL `capacity_acquire.rs`.

Selective reads, explicitly not full-history reviews:

| Material | Inspected extent |
|---|---|
| Feature014 `implementation-ledger.md` | Heading inventory; lines 1–80 and 1518–1678. |
| Feature014 `research.md` | Heading inventory; relevant defaults, clause evidence and current capacity sections, including 153–213, 1244–1298, 1388–1463 and the current tail within 1673–1793. |
| Feature014 `validation-report.md` | Heading inventory; release-gate matrix 1464–1546 and current migration/Vacate corrections 4251–4309. The intervening history was not fully read. |
| Feature014 conformance contract | Fault matrix and surrounding early contract content; later scoped completion/compatibility sections through 258. Not represented as a full independent audit of every contract clause. |
| Performance report | Heading inventory; opening/workload material, 919–955 and 1323–1438. Performance source function/parameter searches were selective; the entire harness was not reviewed. |
| `docs/sdk-compatibility.md` | Heading/keyword inventory and current/version evidence including 1–59, 118–154, 434–456 and 517–568. |
| `docs/traceability.md` | Relevant 006/008/012/014 and recent 033/034 rows, especially 181–194, 238–259, 402–433 and 1038 onward; not the complete file. |
| Feature006/008/012 research | Publication/human-criterion sections: 006:80–118, 008:144–210 and 012:148–213. |
| Pinned project-completion handoff | Relevant initial/open-work and closing sections of `todo.md` at `cc1830a2…`; located through `capacity-eligibility.md:17–20`. Not every historical proof document was read. |
| Source/configuration | CI:24–119 and 205–228; capacity test inventories; coordinator changed-file inventory; read-only branch/worktree metadata. Data-model, runbook and additional contract headings were inspected selectively. |

Missing/deferred reads and verification limits:

- I did not fully read all long research, validation, traceability, ledger histories or every transitive evidence artifact. No claim of a complete “all 39 documents, every line” read is made.
- I did not independently inspect every raw transcript/hash artifact behind retained test reports, run any test/build, query CI or registries, or verify a published release externally.
- Live sibling branches had advanced beyond some inventory pins. Conclusions about branch work use the specified snapshot commits, not an inferred latest integrated state.
- The narrower missing evidence is real native/current-head acceptance, remaining runtime compositions, nested/resource acceptance and genuine human sessions. Those are distinct from the two contradictory instructions and from unintegrated branch work.
- No prior EGM report or live execution manifest was consulted.
