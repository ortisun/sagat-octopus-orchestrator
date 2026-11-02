# Stage 7 — Design review

Stage 7 design review: **2 Major findings, 1 Nit, no Blocker identified.** The dossier needs scope and acceptance-document corrections before it guides execution. This review does not accept implementation, performance ceilings, task completion or release readiness.

Scope: `review_target.md:1–58`, with lines 59–165 and linked proposals used as context. Paths below are relative to `/Users/marcelo/Documents/Projects/sagat-octopus-orchestrator` unless stated otherwise.

## DESIGN-001 — Major — SDK publication work contradicts the accepted distribution channel

Affected scope: FeatureRange overview, `tasks/reviews/2026-10-30-open-tasks/review_target.md:57`; underlying `tasks/todo.md:71` and `docs/traceability.md:194`. **OT-011; FR-006-008; G123.** The copied task appears at target line 155.

Evidence:

- The overview identifies “public SDK registry publication” as open work. OT-011 requires publication under registry identities and treats their absence as a blocker.
- `docs/owner-decisions.md:49` records **GitHub Releases only**, explicitly excluding npm, PyPI, NuGet and Maven for now. Line 26 retains that channel for the coordinated release.
- `docs/runbooks/sdk-packaging.md:43` says registry instructions are retained for a later decision and are not run.
- `specs/006-sdk-multilanguage/research.md:88` records the same decision.
- `docs/releases/0.2.0.md:91` onward documents GitHub release assets; `docs/traceability.md:3` and line 12 report their publication and verification. Its FR-006-008 row at line 194 nevertheless still presents registry credentials as release prerequisites.

The task creates an unauthorized scope dependency: an implementer could seek registry credentials or publish to additional channels to satisfy an obsolete obligation. It also obscures the actual remaining artifact-verification boundary. FR-006-008 requires published versioned packages; it does not override the owner’s chosen channel.

Guidance and observable closure: reconcile the overview, OT-011’s source task and FR-006-008 traceability with the recorded GitHub Releases decision. Identify any remaining immutable-artifact, provenance, retention or clean-consumer evidence for that channel explicitly. Classify registry publication as deferred scope rather than a current credential blocker. Preserve historical evidence and the OT-011 mapping; do not infer completion from this review or reopen the owner’s channel decision.

## DESIGN-002 — Major — The human-evaluation record instructs observers to disregard an already-set acceptance threshold

Affected scope: baseline links at `review_target.md:50–51`; underlying `tasks/todo.md:28` and `docs/validation/human-acceptance-results.template.md:55–56,111`. **OT-010 and OT-012; SC-008-001 and SC-012-001.** Copied task locations are target lines 149 and 161.

Evidence:

- OT-010 still requires agreement on a numeric UX target.
- The session template says `N = PENDING OWNER DECISION` and instructs observers to record `NOT EVALUABLE — N UNSET`; its outcome table repeats that state.
- `docs/owner-decisions.md:40`, `specs/008-observability-operations/spec.md:91` and `docs/validation/human-acceptance.md:11–13` already specify **N = 10**, requiring **at most 9 counted interactions**.
- The protocol explicitly directs every participant/session to use this template (`human-acceptance.md:6–8`).

This is an actionable acceptance-design conflict, not merely an old historical note. An observer following the required form receives different evaluation instructions from the protocol and specification. The duplicate task locations also appear to represent the same remaining human-evidence obligation, rather than two independent product requirements.

Guidance and observable closure: align the task and template with the existing threshold, including the interaction-comparison field. Retain `NOT RUN` until genuine sessions occur; preserve the protocol’s eligibility, assistance, invalid-session and evidence-review rules. Map both OT aliases to the shared obligation while retaining both source locations. Do not apply the diagnosis interaction threshold to SC-012-001, invent results, or request a new threshold decision.

## DESIGN-003 — Nit — Pending primitive tasks still describe a five-SDK acceptance boundary

Affected scope: linked specifications/plans at `review_target.md:28–30`; underlying `specs/014-workflow-primitives/tasks.md:67,69`, `spec.md:381–383`, `plan.md:200–202` and `implementation-ledger.md:28–29`. **OT-003, OT-004, OT-007 and OT-008; T014-016, T014-017, T014-043, T014-044; FR-014-016; FR-034-005/007.**

Evidence:

- Pending collection work enumerates five SDK directories, requires “all-five” conformance, and uses a specification clause that still says all five SDKs must pass before advertisement.
- `docs/owner-decisions.md:64–69` adds the official Rust SDK to release 0.3.
- Constitution IV (`.specify/memory/constitution.md:50–51`) now specifies six SDKs.
- `specs/034-rust-sdk/spec.md:24–30` requires the shared harness and distinguishes historical five-language evidence from new six-language acceptance.
- The later implementation ledger already acknowledges the integrated dependency at lines 1579–1603. Code inspection confirms six entries, including Rust, in `tests/contract/sdk-conformance/harness.rs:35–42`. Therefore this is **not evidence of a missing Rust implementation or absent shared harness**.

The current task wording and newer release dependency describe different acceptance boundaries. Severity is Nit because the constitution, feature 034, later ledger and existing harness establish the correct boundary.

Guidance and observable closure: add an explicit current feature-034 dependency to the pending feature-014 acceptance instructions and align the current normative wording. Preserve dated five-language receipts as historical evidence. Keep the full integrated six-language release gate separate from the selected five-scenario/six-language checkpoint; do not relabel either checkpoint as full acceptance.

## Design assessment and task coverage

The wrapper is an aggregate review dossier, not a new feature proposal. Its purpose, snapshot and review limitations are sufficiently clear. A second product specification or additional implementation detail in FeatureRange is unnecessary.

The linked primitive design addresses alternatives and consequences: bounded declarative scopes, shared budgets, explicit transaction boundaries, reconciliation, compensation ordering and guarded capability activation. Its capacity work explicitly distinguishes dormant storage from usable nested execution. The documented Acquire/sweeper, possession-gating, retention, fairness and lock-graph obligations remain important activation dependencies; their acknowledged incompleteness is not itself a newly discovered defect.

Feature 033 preserves PostgreSQL authority, treats Redis as an optional transport, records duplicate and uncertain-acknowledgement semantics, distinguishes internal hint trimming from customer callback retention, and requires coordinated activation. ADR-0069 and the plan explain the architectural choice. Source searches corroborate reuse of the existing publication and delivery ports. I found no supported reason to propose a separate orchestration or delivery subsystem.

| Entry | Original identifier | Review result |
|---|---|---|
| OT-001 | T014-014 | No additional design finding. Low-quota descendant progress and reservations remain required evidence. Existing bounded cases do not establish nested acceptance. |
| OT-002 | T014-015 | No additional design finding. Accounting, ordered results and reconciled drain are specified; dormant capacity mechanisms do not establish broader runtime readiness. |
| OT-003 | T014-016 | DESIGN-003. Source capture and broader-body boundaries remain explicitly gated. |
| OT-004 | T014-017 | DESIGN-003. Released READY, FORWARD and Applied profiles are separate bounded proofs; later cutover/quota evidence does not replace the complete fault matrix. |
| OT-005 | T014-040 | No additional design finding. Historical 87-test results are not final current-tree acceptance. |
| OT-006 | T014-042 | No additional design finding. Specified large workloads, tenant bounds and production ceilings remain unaccepted. Historical diagnostics are not substituted for them. |
| OT-007 | T014-043 | DESIGN-003. Integrated release gates remain required; the latest inspected ledger records corrected CI still pending. |
| OT-008 | T014-044 | DESIGN-003. Requirement-to-evidence updates must preserve the distinction between demonstrated subsets and remaining acceptance. |
| OT-009 | T033-007 | No additional design finding. Cold-cutover and selected combined-tree evidence are present; full release-tree acceptance remains separate. |
| OT-010 | SC-008-001 / SC-012-001 | DESIGN-002; shares the human-evidence obligation with OT-012. |
| OT-011 | FR-006-008 | DESIGN-001. |
| OT-012 | SC-008-001 / SC-012-001 | DESIGN-002; preserve this source alias when reconciling the duplicate obligation. |

## Read evidence

The following control documents were read fully:

- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/agents/design_critic.md`, lines 1–63.
- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/resources/document_standards/feature-spec-standard.md`, lines 1–36.
- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/docs/codex.md`, lines 1–117.
- `tasks/reviews/2026-10-30-open-tasks/review_target.md`, lines 1–165.
- `tasks/reviews/2026-10-30-open-tasks/review_context.json`, lines 1–212.

Baseline reading-list coverage: **38 files read fully; one long validation history read selectively**, following the supplied reading-policy refinement.

| Baseline path | Read coverage |
|---|---|
| `tasks/reviews/2026-10-30-open-tasks/inventory.json` | Full, 1–266 |
| `.specify/memory/constitution.md` | Full, 1–172 |
| `AGENTS.md` | Full, 1–56 |
| `specs/AGENTS.md` | Full, 1–10 |
| `SPECIFICATION-INDEX.md` | Full, 1–45 |
| `docs/traceability.md` | Full, 1–1094 |
| `docs/owner-decisions.md` | Full, 1–79 |
| `docs/test-strategy.md` | Full, 1–75 |
| `docs/sdk-compatibility.md` | Full, 1–568 |
| `docs/runbooks/sdk-packaging.md` | Full, 1–69 |
| `specs/014-workflow-primitives/spec.md` | Full, 1–693 |
| `specs/014-workflow-primitives/plan.md` | Full, 1–729 |
| `specs/014-workflow-primitives/tasks.md` | Full, 1–267 |
| `specs/014-workflow-primitives/research.md` | Full, 1–1793 |
| `specs/014-workflow-primitives/validation-report.md` | All headings; sections 1464–1582, 1800–1819, 3873–3970, 4161–4309 |
| `specs/033-redis-streams/spec.md` | Full, 1–53 |
| `specs/033-redis-streams/plan.md` | Full, 1–27 |
| `specs/033-redis-streams/tasks.md` | Full, 1–30 |
| `specs/033-redis-streams/research.md` | Full, 1–59 |
| `specs/033-redis-streams/validation-report.md` | Full, 1–99 |
| `specs/006-sdk-multilanguage/spec.md` | Full, 1–158 |
| `specs/006-sdk-multilanguage/plan.md` | Full, 1–123 |
| `specs/006-sdk-multilanguage/tasks.md` | Full, 1–125 |
| `specs/006-sdk-multilanguage/research.md` | Full, 1–184 |
| `specs/008-observability-operations/spec.md` | Full, 1–144 |
| `specs/008-observability-operations/plan.md` | Full, 1–114 |
| `specs/008-observability-operations/tasks.md` | Full, 1–94 |
| `specs/008-observability-operations/research.md` | Full, 1–210 |
| `specs/012-developer-experience/spec.md` | Full, 1–128 |
| `specs/012-developer-experience/plan.md` | Full, 1–98 |
| `specs/012-developer-experience/tasks.md` | Full, 1–78 |
| `specs/012-developer-experience/research.md` | Full, 1–213 |
| `docs/validation/human-acceptance-results.template.md` | Full, 1–190 |
| `docs/validation/human-acceptance.md` | Full, 1–217 |
| `specs/014-workflow-primitives/evidence/released-v02-20261027/README.md` | Full, 1–37 |
| `specs/014-workflow-primitives/evidence/released-v02-inflight-20261027/README.md` | Full, 1–42 |
| `specs/014-workflow-primitives/evidence/released-v02-applied-20261027/README.md` | Full, 1–41 |
| `specs/033-redis-streams/released-callback-cutover.md` | Full, 1–75 |
| `tasks/todo.md` | Full, 1–121 |

Additional references read:

- `docs/adr/ADR-0069-redis-streams-share-outbox-and-delivery-authority.md`, full 1–35.
- `docs/releases/0.2.0.md`, full 1–101.
- `specs/014-workflow-primitives/capacity-vacate-acquire.md`, full 1–228.
- `specs/014-workflow-primitives/implementation-ledger.md`: all headings and sections 1–80, 1518–1678.
- `specs/034-rust-sdk/spec.md`, full 1–36; `plan.md`, full 1–19; `tasks.md`, full 1–43.
- `docs/adr/ADR-0070-rust-sdk-shares-the-canonical-sdk-contract.md`, full 1–17.
- `tests/contract/sdk-conformance/README.md`, lines 1–17, plus targeted search matches.
- Code sections: `crates/infra-connectors/src/redis.rs:150–215`; `publish.rs:58–109`; `crates/application/src/maintenance.rs:245–269`; `crates/scheduler/src/outbox.rs:85–180`; `deliveries.rs:123–145`; `tests/contract/sdk-conformance/harness.rs:35–100`.

Searches used `rg` for publication/channel decisions, UX thresholds and criteria, SDK language/dependency wording, document headings, and concrete reuse symbols. The reuse search found `RedisPublisher` implementing `EventPublisher`, `OutboxDispatcher` parameterized by that port, Redis branching in the existing `BrokerPublisher`, and the existing `DeliveryWorker` accepting that publisher. SDK searches located the six-language registry and its explicit selection filters. Capacity searches located the existing Vacate adapter and shared Finish identity types; no missing-implementation finding is based on a proposed filename.

## Limitations

No required baseline file was inaccessible. The unselected historical validation-report lines and unselected implementation-ledger history were not read; no full-history audit is claimed. Large reads that truncated were recovered in smaller reads where full coverage is reported. Exploratory guesses at several paths were absent; canonical paths were subsequently located and used.

No tests, builds, network requests, services, writes, git mutations, prior review reports, live execution manifest, conversation retrieval or memory retrieval were used. Sibling worktree contents were not independently audited. Recorded test counts, release verification, hashes and CI results remain documentary claims, not results independently reproduced by this review.
