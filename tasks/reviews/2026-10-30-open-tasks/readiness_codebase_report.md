# Stage 9 — Implementation readiness: codebase review

The target is **not implementation-ready** under the supplied standard. I found **six Major findings, no Blockers and no Nits**. The main gaps are an unspecified remaining implementation delta, inaccurate evidence wording, and tasks that still describe superseded scope.

This was strictly read-only. I ran no builds, tests, services, network requests or Git mutations, and wrote no files. Historical test totals below remain documentary evidence, not results obtained by this review.

## CODE-001 — Major — Classify the remaining changes once, against the current implementation

Affected scope: `review_target.md:67–143`; OT-001–OT-009; T014-014–017, T014-040, T014-042–044, T033-007.

The task descriptions do not mark files Create/Modify/Delete or identify the remaining changes after their accumulated evidence. Several instructions now name substantial existing implementations: typed collection state, transactional entry/admission, blob membership, scoped worker claims, public control/timer/event bodies, SDK builders and Redis transport. A fresh implementer must infer which accepted behavior to preserve and which absent combinations to implement.

For example, OT-001 proposes `tests/integration/workflow-primitives/collections.rs`, while current tests are registered through `tests/integration/workflow-primitives/main.rs` and distributed across `primitive_transactions`, `collections_http`, collection budgets, sources and related modules. The proposed file’s absence is **not** an independent defect.

Correction: preserve the inventory’s original wording, but provide one current remaining-work map with Create/Modify/Delete, exact symbols, prerequisite increments, existing shared fixtures and named acceptance cases. Reuse the existing transaction ports and test harnesses. Distinguish dormant capacity components from their future production callers. This is one classification/readiness finding, not a separate finding for each proposed path.

## CODE-002 — Major — The universal fault replay-oracle statement is false at its named historical checkpoint

Affected scope: `review_target.md:101`; OT-005/T014-040; `specs/014-workflow-primitives/validation-report.md:1514–1517`.

The evidence says every fault test ends with `verify_run`/`assert_consistent`, meaning persisted projection is compared with journal replay. I checked the local historical commit **7d7868ff**, not merely tests introduced afterward. Its fault entrypoint registers `collection_sources`. That snapshot’s `tests/fault/workflow-primitives/collection_sources.rs:61–108`, `136–150`, `186–213` and `216–234` contain counterexamples. Its shared `tests/support/blob_collections.rs:177–260` reads status, counts and aggregate and drives completion; it does not add a replay verification. The inspected historical fixture has no implicit teardown verifier.

These tests do contain valuable independent persistence assertions: scope/item/audit counts, rollback manifest absence, status and aggregate output. The finding is **not** “no persistence assertions,” nor a requirement to use one particular helper.

Correction: replace the universal statement with a case-to-oracle inventory that credits equivalent independent assertions. Add any replay/accounting checks required for the remaining complete matrix, retaining original results and temporal scope. The recorded 87 passes are not disproved. Major is appropriate because this is a false historical evidence description, not a demonstrated current runtime failure.

## CODE-003 — Major — OT-006’s current remediation description is behind the available implementation and measurements

Affected scope: `review_target.md:108`; OT-006/T014-042.

“Fix in progress on `feature/014-scale`” and the original G461/G468 costs are insufficient as the current starting point. `crates/scheduler/src/passes.rs:1892–1917` and `2942–2964` already select bounded collection/loop frontiers with explicit whole-run fallback. `crates/application/src/worker/scoped_claims.rs:59–140` uses exact-path transactions.

The referenced performance report itself records subsequent work: `tests/performance/reports/workflow-primitives.md:1128–1258` documents the scale increment; `1393–1407` documents 20,000 durable waits under the runtime role; `1410–1438` documents 10,000-item flat collections at widths 1/16/256 with explicit limitations. Those are narrower historical measurements, not production-ceiling acceptance.

The plan still requires nested execution reaching 100,000 instances (`specs/014-workflow-primitives/plan.md:80–83`). The inspected performance target contains flat collections, sequential loops, waits and contention; I found no registered nested-100k workload. `collections.rs:425–438` defaults to only 100/1,000 items unless configured.

Correction: retain old numbers as dated baselines, link the newer evidence and specify the remaining workloads, prerequisites, driver changes and exact commands. Record the real runtime/observer pool budgets and logical-versus-parallel concurrency limits. Keep nested/resource acceptance open; do not restart already implemented scale work or infer general nesting from flat measurements.

## CODE-004 — Major — Future collection acceptance still specifies only five SDKs

Affected scope: `review_target.md:81` and `88`; OT-003/T014-016 and OT-004/T014-017; consequential release/traceability work OT-007–008.

The five-language historical results are legitimate historical claims. The unfinished implementation and conformance instructions, however, still enumerate only TypeScript, Go, Python, C# and Java and require “all-five” acceptance.

The official Rust SDK is an approved current dependency (`docs/owner-decisions.md:64–69`; `specs/034-rust-sdk/spec.md:24–30`). Its existing collection builders are in `sdks/rust/src/workflow/primitives.rs:273–286`. The shared harness enumerates six languages at `tests/contract/sdk-conformance/harness.rs:35–42`; CI runs the six-language suites at `.github/workflows/ci.yml:217–225`.

Correction: make the current remaining collection/runtime/fault gate use all six official SDKs and the shared scenarios/goldens. Reuse Rust’s existing builders and harness. Preserve historical five-language evidence unchanged and distinguish focused six-language runs from complete release-tree acceptance.

## CODE-005 — Major — The human acceptance template contradicts the already selected UX threshold

Affected scope: `review_target.md:149` and `161`; OT-010 and OT-012; SC-008-001/SC-012-001; `tasks/todo.md:28,72`.

OT-010 still asks to agree the numeric UX target. `docs/owner-decisions.md:40` already selects **N = 10**, and `docs/validation/human-acceptance.md:10–13` correctly requires at most nine counted interactions. But the shared results template at `docs/validation/human-acceptance-results.template.md:55–56,111` directs the observer to use “PENDING OWNER DECISION” and “NOT EVALUABLE — N UNSET.”

Correction: apply the recorded threshold to the task and template, and map OT-010/012 to one shared human-evaluation evidence stream. Preserve distinct diagnosis and onboarding criteria, including the onboarding timing criterion. Keep results NOT RUN until genuine sessions occur and are reviewed. No new owner decision or simulated acceptance is needed.

## CODE-006 — Major — OT-011 waits for registry identities for a distribution channel the owner deferred

Affected scope: `review_target.md:155`; OT-011/FR-006-008; `tasks/todo.md:71`.

The task requires publication under registry identities and describes missing registry identity information as its blocker. The recorded decision is **GitHub Releases only**, excluding npm/PyPI/NuGet/Maven publication for now (`docs/owner-decisions.md:26,49`). This is also explicit in `specs/006-sdk-multilanguage/research.md:88` and `docs/runbooks/sdk-packaging.md:41–55`.

The implementation matches that channel: `.github/workflows/release.yml:159–193` collects assets/checksums, attests provenance and publishes a verified tag’s GitHub Release. Rust packaging is included as a candidate, with crates.io disabled (`docs/runbooks/sdk-packaging.md:57–69`).

Correction: reconcile the task with the approved GitHub distribution channel and identify any remaining immutable-version, provenance and clean-consumer evidence for the applicable release. Keep registry publication conditional on a later scope decision. This review did not verify remote assets and does not declare publication acceptance complete.

## Per-entry coverage

| Entry | Task / criterion | Source mapping and review disposition |
|---|---|---|
| OT-001 | T014-014 | Typed identity bounds, existing collection budget/contention tests and registered integration modules exist. Nested low-quota acceptance remains pending. CODE-001. |
| OT-002 | T014-015 | `execution/instance.rs`, `execution/state.rs`, collection reducer/event families and transactional admission/drain implementation exist. Ordered results use index-keyed state; accepted work and drain reservations are distinct from fresh admission. No additional demonstrated contradiction. CODE-001. |
| OT-003 | T014-016 | Application collection-source preparation, PostgreSQL entry/admission, source-blob FK and scoped claims exist. Current publication remains bounded. CODE-001 and CODE-004. |
| OT-004 | T014-017 | Real fault helpers, compensation modules and released-source profiles exist. Historical active-data/FORWARD/Applied increments retain explicit boundaries; later output-quota and dispatcher/callback cutover work is also present. CODE-001 and CODE-004. |
| OT-005 | T014-040 | Configured fault target and real database barriers exist. Historical universal replay statement has a same-commit counterexample. CODE-001 and CODE-002. |
| OT-006 | T014-042 | Configured performance target, explicit ignored benchmarks and later runtime-role measurements exist. Nested-100k driver/acceptance not found in inspected target. CODE-001 and CODE-003. |
| OT-007 | T014-043 | Current CI contains native/static/workspace, dedicated release-upgrade and six-SDK gates. The old 12-check run is not a current result. No independent false-pass finding. CODE-001; CODE-004 affects final SDK scope. |
| OT-008 | T014-044 | Traceability and current implementation ledger exist, with bounded increments and explicitly open broad criteria. Current publication guards support restricted shapes only. CODE-001; CODE-004 affects updated acceptance mapping. |
| OT-009 | T033-007 | Redis implementation, configured integration target and genuine released callback cold-cutover coordinator exist. Final release-tree gates remain explicitly open. No missing-proof finding. CODE-001. |
| OT-010 | SC-008-001 / SC-012-001 | Shared protocol exists and N is decided; template is stale. CODE-005. |
| OT-011 | FR-006-008 | Packaging/consumer/release workflow exists; registry waiting state conflicts with approved channel. CODE-006. |
| OT-012 | SC-008-001 / SC-012-001 | Same genuine human evidence obligation as OT-010, not another independent study completion. CODE-005. |

## Observed implementation boundaries and reuse

- `crates/infra-ir/src/convert.rs:641–832` admits a bounded root collection graph with local activity/control/timer/event nodes and Return. It does not enable arbitrary nested collections, child/approval bodies or unrestricted effect/error combinations.
- `crates/infra-postgres/src/primitives.rs:603–640,1266–1320,1518–1764,1775–1910,2000–2114` contains applicable chain/own-run locking, current scheduler fencing, revision checks, captured membership, policy/usage locking and atomic journal/projection/outbox/audit updates. The application collection-source layer validates source ownership, canonical item capture and byte/item limits. Migration `20261121000000_collection_blob_source.up.sql` protects the retained source with a composite tenant/namespace FK and RESTRICT deletion.
- Production engine setup enables scoped worker storage and supplies the collection item source. Scoped claims require supported storage and sufficient negotiated protocol, then check exact path, namespace, binding, session room and current authorization. Root and scoped work are not interchangeable.
- Capacity Acquire/Vacate share `crates/infra-postgres/src/capacity_transfer.rs`. The inspected transaction uses chain → own run → group NOWAIT, rechecks membership and scheduler authority, handles immutable receipt replay, and distinguishes confirmed abort from uncertain COMMIT outcome. Acquire does not establish runnable work or issue leases. Production scheduler tick has no capacity discovery/transfer pass (`passes.rs:610–640`), and searches found no production caller for the inspected ports. General nesting is therefore not established.
- Capacity design inputs are not missing merely because `tasks/project-completion/` is absent on this checkout. `capacity-eligibility.md:17–20` explicitly pins them to `cc1830a2c19e`; the lifecycle, lock inventory and transaction-entry documents resolve there. Their historical source inventory is not proof that unmerged sibling implementation is integrated.
- Redis internal transport is explicitly selected by `SAGAT_EVENT_TRANSPORT` (`apps/execution-engine/src/settings.rs:664–726`). Callback admission separately checks its rollout flag, namespace egress and TLS for credentials (`crates/application/src/outbound.rs:741–785`). The existing publisher/delivery infrastructure is reused.
- The callback cutover is concrete: `tests/integration/workflow-primitives/release_upgrade_callback_cutover.rs:109–149` revokes old login, terminates authenticated sessions, joins the actual old process, then migrates/enables Redis and checks old metadata, delivery identity/body and replay. It explicitly claims cold cutover, not mixed-version safety.

The shared fixtures use real PostgreSQL, migration setup and optional least-privilege runtime pools. `tests/support/scope_flow.rs:715–815` implements held-run locks, observable database waiters and controlled future interruption. `tests/support/brokers.rs:359–459` starts owned Redis with pinned image/AOF and verifies actual loopback bindings. The common SDK harness selects from one six-language registry and shared scenarios/goldens; environment filters can narrow a run and must therefore be recorded.

## Reference and command validation

The target’s recorded SHA-256 matches, as do all 39 static reading-list hashes. All 12 inventory source-task lines match their originals. Direct local Markdown paths in the target resolve; Markdown anchor semantics were not validated. The reviewed checkout HEAD matched `888074e6237b2a1ce9c85f83ec5703b081c65dd6`.

`tests/Cargo.toml:107–124,356–357,384–385` registers `sdk_conformance`, `contract_workflow_primitives`, `integration_workflow_primitives`, `fault_workflow_primitives`, `e2e_workflow_primitives`, `perf_workflow_primitives` and `integration_redis_streams`. Current CI explicitly runs the ignored SDK and released-upgrade profiles; a normal workspace run alone does not execute those gates. The current upgrade runner is `scripts/test-release-upgrade-profile.py`; older evidence READMEs retain their historical versioned command, consistent with the recorded source-naming decision.

## Read inventory and limits

Full reads included:

- EGM persona, implementation-readiness standard and `docs/codex.md`; target, inventory and static review context.
- Constitution, repository/spec instructions, specification index, owner decisions, test strategy and `tasks/todo.md`.
- Complete `spec.md`, `plan.md`, `tasks.md` for features 006, 008, 012, 014, 033 and 034.
- Feature 033 research, validation and released-callback-cutover plan; SDK packaging runbook; human protocol and results template.
- Feature 014 capacity eligibility, discovery, Vacate/Acquire and Acquire plans; scoped-event-publication plan; all three directly linked released-v0.2 evidence READMEs.
- Pinned `cc1830a2c19e` capacity lifecycle, lock-graph and transaction-entry documents, respectively 93, 156 and 86 lines.
- Principal complete source reads: fault/integration target entrypoints, collection-source application implementation, source-blob migration, Redis connector and Redis integration entrypoint, capacity Acquire adapter, bounded test helper, SDK-conformance entrypoint and Rust package helper. Historical `7d7868ff` fault collection-source file was read completely.

Selective historical/document reads were:

- Feature 014 `research.md:1244–1298,1670–1793`.
- Feature 014 `validation-report.md:1464–1545,4229–4309`.
- Feature 014 `implementation-ledger.md:1518–1678`.
- Performance report `1–110,1128–1258,1323–1438`.
- `docs/traceability.md:402–490`; `docs/sdk-compatibility.md:1–110,524–568`.
- Feature 006 research `80–118`; feature 008 research `144–210`; feature 012 research `148–213`.
- Headings/searches supplemented these ranges. Older histories and raw result archives were not exhaustively audited.

Principal selective source reads included the PostgreSQL primitive ranges above; domain instance `1–145` and collection state `680–790`; IR conversion `641–832`; scheduler tick/frontier ranges above; scoped claims `1–140`; capacity transfer `1–360`; engine setup `80–105,176–261`; settings/outbound/publisher ranges cited above; support database `1–265,370–EOF`, engine `129–225`, scope flow `395–450,710–830`, blob collections `175–260`, brokers `350–459`; fault collections `1–190,555–615`; conformance harness `30–95,1445–1525,1590–1685`; callback coordinator `1–153`; profile runner `15–45,225–272`; current CI `20–105,180–230`; release workflow `150–193`. Other referenced builder/test symbols were located by code search rather than exhaustively reviewing every unit test.

No prior review reports or live execution manifest were consulted. No remote CI, release assets, full raw evidence archive, actual human session or current test result was independently verified. Pending acceptance remains pending.
