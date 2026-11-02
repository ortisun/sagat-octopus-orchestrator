# Stage 5 — Initial Zoom Out

Stage 5 Initial Zoom Out review completed against checkout `888074e6237b2a1ce9c85f83ec5703b081c65dd6`.

The dossier provides sufficient context for review. Its underlying architecture remains appropriately grounded in the existing engine, PostgreSQL transactions, shared conformance harness and release tooling. Four Major documentation/readiness findings require correction. No Blocker was established under this rubric. This review does not establish runtime acceptance or authorize closing any checkbox.

## ZO-001 — Major — Registry publication is presented as required work despite the recorded distribution decision

**Affected:** OT-011, FR-006-008.
**Locations:** `tasks/reviews/2026-10-30-open-tasks/review_target.md:57` and `:155`; underlying `tasks/todo.md:71`, also `:110`.

The task treats registry identities and credentials as an outstanding dependency. The owner already selected **GitHub Releases only**, explicitly excluding npm, PyPI, NuGet and Maven publication: `docs/owner-decisions.md:49`. The later release authorization preserves that boundary at `docs/owner-decisions.md:26`. FR-006-008 requires versioned packages, compatibility information and examples; it does not independently require those registries (`specs/006-sdk-multilanguage/spec.md:73`).

This is an underlying backlog scope error copied into the dossier, not a missing publishing implementation. Acting on it would reopen a settled product decision and introduce unnecessary credential and publishing work.

**Simpler alternative and reuse:** Use the existing authorized distribution path. `docs/runbooks/sdk-packaging.md:43` identifies it; `.github/workflows/release.yml:160` already assembles checksums, attests assets and publishes through GitHub Releases. `scripts/package-sdks.sh:501` includes the existing Rust packaging helper. `docs/runbooks/sdk-packaging.md:69` expressly excludes crates.io publication.

**Observable closure recommendation:** Reconcile the task and overview with the recorded owner decision. Identify any remaining verification of authorized release assets and clean consumers, with exact version/source and evidence. Preserve external registry publication as conditional on a new owner decision, rather than awaiting credentials now. Do not close FR-006-008 solely from this review.

## ZO-002 — Major — Human acceptance still asks for a threshold already decided, and the results template prevents evaluating it

**Affected:** OT-010 and OT-012, SC-008-001 / SC-012-001.
**Locations:** `tasks/reviews/2026-10-30-open-tasks/review_target.md:149` and `:161`; underlying `tasks/todo.md:28` and `:72`; `docs/validation/human-acceptance-results.template.md:55` and `:111`.

OT-010 asks to agree the numeric UX target. The template instructs evaluators to report “N UNSET.” However, `docs/owner-decisions.md:40`, `specs/008-observability-operations/spec.md:91`, and `docs/validation/human-acceptance.md:11` already fix **N = 10**, meaning at most nine counted interactions.

The unresolved work is collecting and reviewing genuine human observations. Reopening the threshold is unnecessary; following the stale template would produce an unevaluable result even after a valid session. The duplicate task locations also describe the same acceptance work, rather than two separate studies.

**Simpler alternative and reuse:** Reuse the existing human protocol and results template, correcting the template’s threshold and verdict instructions. Both inventory entries can refer to one canonical evidence record while retaining their original task locations and both criteria.

**Observable closure recommendation:** Make the task, specification, protocol and template agree on the existing threshold; keep sessions `NOT RUN` until actual evidence exists. Record the real diagnosis counts and the distinct TypeScript/C# onboarding observations required by the protocol, including unsuccessful sessions and reviewer verdicts. Link both task entries to that evidence without double-counting participants or inventing results.

## ZO-003 — Major — Forward-looking collection acceptance still enumerates five SDKs after Rust became the sixth official SDK

**Affected:** OT-003, OT-004, OT-007 and OT-008; T014-016, T014-017, T014-043 and T014-044.
**Locations:** `tasks/reviews/2026-10-30-open-tasks/review_target.md:81`, `:88`, `:115` and `:122`; underlying `specs/014-workflow-primitives/tasks.md:67`, `:69` and `:197`; execution sequence also remains five-language at `specs/014-workflow-primitives/implementation-ledger.md:29`.

Historical five-language receipts remain valid evidence for their recorded boundaries. The problem is that the open implementation/acceptance wording still limits builders and WP-03/04 verification to five languages. Constitution IV now requires the same semantics and conformance suite across six (`.specify/memory/constitution.md:50`). The approved Rust dependency explicitly preserves historical evidence while requiring new six-language acceptance (`specs/034-rust-sdk/spec.md:24` and `:29`).

The dossier mentions six-language evidence under Redis, but that does not amend the collection tasks or turn five selected scenarios into the complete integrated acceptance gate. The ledger explicitly distinguishes those results at `implementation-ledger.md:1592`.

**Simpler alternative and reuse:** Extend the acceptance wording to the existing common harness, rather than create another Rust-specific collection suite. Actual checkout inspection confirms `tests/contract/sdk-conformance/harness.rs:35` registers all six languages; the same function permits narrowing through `SAGAT_SDK_LANGS`, so selected runs must remain identifiable.

**Observable closure recommendation:** State six-language coverage for newly accepted collection behavior and the final integrated suite. Keep earlier five-language receipts unchanged and dated. Require a result identifying the exact current source, registered scenarios, all six selected languages and unchanged reviewed goldens; selected or historical results must not substitute for that gate.

## ZO-004 — Major — The performance task’s current problem statement points to a superseded optimization effort

**Affected:** OT-006 and OT-008; T014-042 and T014-044.
**Locations:** `tasks/reviews/2026-10-30-open-tasks/review_target.md:108` and `:122`; underlying `specs/014-workflow-primitives/tasks.md:195` and `:199`.

The adjacent evidence still describes G461’s quadratic 1 h 54 min collection run and says the fix is in progress on `feature/014-scale`. Later retained evidence and current source show that this is no longer an adequate description of the remaining work. The performance report records runtime-role flat 10,000-item runs at all three widths, and a 20,000-wait workload (`tests/performance/reports/workflow-primitives.md:1393` and `:1410`). Their manifests retain source identities and limitations.

Actual checkout searches also found the existing frontier and bounded worker paths: `crates/infra-postgres/src/scope_frontier.rs:276`, `crates/application/src/worker/scoped_claims.rs:73`, and `crates/application/src/worker/scoped.rs:19`. The scheduler deliberately retains a whole-run oracle for transitions that need it (`crates/scheduler/src/passes.rs:667`).

This does **not** establish current release capacity. The later measurements are historical, workload-specific results, and the report explicitly leaves nested/low-quota progress, 100,000 instances and release gates open. The readiness problem is directing another implementer toward an obsolete root cause without separating existing machinery, historical measurements and remaining acceptance.

**Simpler alternative and reuse:** Reuse the existing frontier/path implementation and corrected measurement driver. Add a concise current residual-work reference to the task and dossier; retain the original failure as history. No new optimization architecture is justified merely by the old timing.

**Observable closure recommendation:** Identify which specified workloads still lack evidence on the integrated release source, including nested capacity and resource observations. Record fresh source-bound commands, results, failures and measurement limitations for those gates. Update traceability to distinguish historical G461 evidence from present acceptance, without accepting ceilings or reducing the approved workloads.

## Task coverage

| Entry | Result under this rubric |
|---|---|
| OT-001 — T014-014 | No additional finding. Low-quota descendant and reservation tests address accepted requirements; their absence remains explicit. |
| OT-002 — T014-015 | No additional finding. Typed collection behavior fits the existing domain; the linked context distinguishes implemented subsets from general nesting. |
| OT-003 — T014-016 | ZO-003. |
| OT-004 — T014-017 | ZO-003. Historical upgrade results do not establish complete current acceptance. |
| OT-005 — T014-040 | No additional finding. The complete fault/tenant/secret matrix is justified and reuses existing fixtures. |
| OT-006 — T014-042 | ZO-004. |
| OT-007 — T014-043 | ZO-003. Final integrated release gates remain necessary. |
| OT-008 — T014-044 | ZO-003 and ZO-004. |
| OT-009 — T033-007 | No additional finding. Redis reuses the existing publisher/outbox boundary, preserves PostgreSQL authority and has an explicit final integration gate. |
| OT-010 — human acceptance | ZO-002. |
| OT-011 — distribution | ZO-001. |
| OT-012 — human acceptance | ZO-002; same acceptance work as OT-010, with both source locations preserved. |

## Read evidence

- Fully read the EGM persona `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/agents/zoom_out.md`, the complete `resources/document_standards/zoom-out-standard.md`, and plugin `docs/codex.md`.
- Fully read `tasks/reviews/2026-10-30-open-tasks/review_target.md` lines 1–165, its `inventory.json`, and its `execution_manifest.json`.
- Fully read all 39 manifest baseline files. Grouped by their exact parent directories:
  - Repository root: `AGENTS.md`, `SPECIFICATION-INDEX.md`.
  - `.specify/memory/`: `constitution.md`.
  - `specs/`: `AGENTS.md`.
  - `docs/`: `traceability.md`, `owner-decisions.md`, `test-strategy.md`, `sdk-compatibility.md`.
  - `docs/runbooks/`: `sdk-packaging.md`.
  - `specs/014-workflow-primitives/`: `spec.md`, `plan.md`, `tasks.md`, `research.md`, `validation-report.md`.
  - `specs/033-redis-streams/`: `spec.md`, `plan.md`, `tasks.md`, `research.md`, `validation-report.md`, `released-callback-cutover.md`.
  - `specs/006-sdk-multilanguage/`: `spec.md`, `plan.md`, `tasks.md`, `research.md`.
  - `specs/008-observability-operations/`: `spec.md`, `plan.md`, `tasks.md`, `research.md`.
  - `specs/012-developer-experience/`: `spec.md`, `plan.md`, `tasks.md`, `research.md`.
  - `docs/validation/`: `human-acceptance-results.template.md`, `human-acceptance.md`.
  - `specs/014-workflow-primitives/evidence/`: `released-v02-20261027/README.md`, `released-v02-inflight-20261027/README.md`, `released-v02-applied-20261027/README.md`.
  - `tasks/`: `todo.md`.
  - `tasks/reviews/2026-10-30-open-tasks/`: `inventory.json`.
- Additional relevant reads:
  - Full `specs/034-rust-sdk/spec.md`, `plan.md`, and `tasks.md`.
  - Full `docs/runbooks/release.md`, `docs/releases/0.2.0.md`, and `specs/014-workflow-primitives/evidence/sdk64-20261027/README.md`.
  - `specs/014-workflow-primitives/implementation-ledger.md` lines 1–81 and 1518–1678, plus heading/reference searches.
  - `tests/performance/reports/workflow-primitives.md` lines 1323–1438, plus targeted searches; full `raw/workflow-primitives/20261017-runtime-flat/manifest.json` and `20261017-waits-runtime/result.json` beneath that report directory.
  - Relevant source sections in the conformance harness/main target, bounded worker paths, scheduler, and release workflow.

## Searches and limitations

- Used read-only `rg`/`rg --files` searches for distribution decisions and implementations, human thresholds/templates, SDK language registration, frontier/path reuse, performance evidence and current ledger references. Initial incorrect conformance-directory searches were corrected using the actual file inventory.
- `git rev-parse HEAD` confirmed the dossier’s checkout revision.
- No mandatory baseline file was missing. The historical `tasks/project-completion/` evidence tree is absent from this checkout, as the validation report itself explains. Its inaccessible receipts were not treated as independently verified acceptance.
- Did not inspect other worktrees, retrieve conversation or memory, contact services, use the network, run tests/builds, or modify files or Git state. Source inspection proves the existence of reusable machinery; retained test reports remain claims at their recorded revisions.
- No recommendation here changes approved product scope, enables a primitive, accepts performance ceilings, publishes artifacts or closes a task.
