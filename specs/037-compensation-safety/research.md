# Compensation safety — approval and research record

## Owner direction — 2026-11-01

The owner answered **“Sim aprovo”** to the B02 recommendation of S1-B and S2-B. The preceding
question explicitly limited approval to recording direction and preparing the specification and
dependent diffs. This is not approval of implementation, final wire shapes, provider qualification,
financial release, waiver of tests or adoption by PR.

- **S1-B:** human review by default; explicit published `fail` for an explicit saga. Terminal
  `COMPENSATION_FAILED` plus retained journal/step evidence is the platform's post-terminal tracking;
  no active obligation/review queue is promised. Cancellation/deadline preserves a pending review.
  Independent compensations continue. O381 is answered “hold for both”; O380 remains deferred.
- **S2-B:** unknown external effects use consultation/review by default, including missing/false
  `highRisk`. Defensive compensation needs a pinned, authorized qualification, authoritative provider
  capability evidence and adapter tests. It must tolerate absent/applied/late effects. Compensation
  success means business finality; unknown compensation repeat requires its own provider guarantee.
  Installing a plugin does not qualify it. Original unknown-effect retry has no automatic exception.
- Already approved: fresh installation/no legacy upgrade, SDK-owned integrations and reusable
  plugin steps. No bank/venue guarantees, usage statistics or regulatory retention periods supplied.

Canonical owner record: [owner decisions](../../docs/owner-decisions.md).
Old D63/D66 and O380/O381 records remain historical; additive notes link the approved direction.
No new globally shared D/G/O numbers are allocated by this preparation.

## Source evidence and differences from the target

| Current evidence | Consequence for feature 037 |
|---|---|
| `crates/domain/src/saga.rs` exposes default review and explicit `fail`; `execution/command.rs` applies the policy | Keep truthful terminal failure, amend the absolute Constitution III wording explicitly |
| `execution/sagas.rs::give_up` and compensation planner may abandon a review after an external stop; `execution/scoped_sagas.rs` holds it | Align explicit saga with the approved hold semantics; do not change scope policies |
| Both planners include unknown effects for defensive compensation | Existing `compensateWith` is not the future qualification gate |
| `command.rs` requests reconciliation for declared forward activities; ordinary unknown effects can fall through to retry | Cover both root/scoped failure paths and compensation ambiguity; default is no longer controlled by highRisk |
| `sdks/SPEC.md` §6.6 delegates idempotence/nonexistence safety to compensation handlers | Update contract to include finality, late effects and qualification; do not invent a derived compensation reconciler already present |
| `runs.rs` distinguishes ordinary operation and override permissions | Cancellation must not substitute for privileged explicit effect resolution |

Relevant history: [003 research](../003-sagas-compensations/research.md),
[014 research](../014-workflow-primitives/research.md), [ADR-0004](../../docs/adr/ADR-0004-external-effects-at-least-once.md).
Decision history is preserved; feature 037 is a target change, not a claim about prior behavior.

## Design work remaining before an implementation plan can close

These are technical design questions, not unanswered owner choices. Preserve the approved safe
defaults when proposing the answers; do not use them to invent a financial product requirement.

1. Define effect classification and evidence for pure computation. Arbitrary SDK code with unknown
   external capability cannot become pure because metadata is absent.
2. Define the smallest immutable integration-qualification reference, authorization and revocation
   behavior. Verify publication and use consistently without a new certification service.
3. Define accepted/pending/unknown compensation reporting and lookup/recovery. Prove finality and
   stable identity without falsely treating a generic handler as a platform reconciler.
4. Enumerate state/IR/SDK/Studio/persistence impacts; preserve exact-instance fencing, audit and
   atomic boundaries. Define ordering when current authorization changes after lookup or provider I/O.
5. Plan retention/follow-up, fresh-install cutover and rollback. Reverting code never undoes provider effects.

`abort_no_compensation` residual effects and presentation of manual-resolution provenance remain
separate product assessments. O380 policy fields are not introduced. No provider is qualified.

## Spec Kit and numbering provenance

Installed `.specify/init-options.json` pins Spec Kit 1.1.2. The repository template resolver selected
the spec template; its required scenarios/requirements/entities/success/assumptions structure is used.
The extension-hook file is absent. This preparation uses the template without executing the complete
`speckit-specify` command, switching branch or changing `.specify/feature.json` from feature 014.
Future commands must explicitly select feature 037 when work begins.

All 48 local/cached-remote refs and the 11 existing worktrees were inspected on 2026-11-01:
maximum feature 036, maximum ADR 0075. This checkout reserves feature 037. ADR-0076 is proposed
in the separate adoption patch; recheck before applying it, because parallel work can advance.
No fetch was performed; this is not a global remote lock or claim of remote freshness.

## Validation limits

The earlier assessment ran the unchanged domain suites (60 sagas + 34 scoped compensation,
zero failed/ignored); [validation report](validation-report.md) preserves that provenance.
Those tests encode current behavior, including the differences this feature will change. They
are not passing acceptance tests for feature 037. New behavior requires red-first tests and the
appropriate persistence, authorization, fault, contract and SDK evidence.

## Draft precision after independent review

Unknown/accepted/pending compensation follows S2 consultation/review (or qualified safe repeat)
before S1 terminal-failure policy can apply. `fail` is for known permanent/non-retryable failure;
this is not permission to erase ambiguity after a retry budget.

Existing manual success resolution is preserved: source test
`an_operator_can_declare_a_compensation_done_outside_the_platform` permits `COMPENSATED`.
That is a human-declared workflow result with actor/reason retained, not provider-confirmed
financial settlement. No new terminal status, automatic qualification or manual UI redesign is
introduced. Handler/automatic success remains subject to the integration contract; qualification adds
finality evidence only for qualified exception paths.

Terminal failure retains the existing ordering rule: finish eligible independent compensations and
terminally suppress blocked dependents before making the run terminal; no compensation is scheduled
after terminalization. The S1-B follow-up/retention responsibility is expressed as operator guidance
(FR-037-020), not a new unapproved automatic admission gate or risk taxonomy. The owner-approved
direction asked the integration operator to define follow-up; it did not specify a platform gate.

## 2026-11-01 — Proposed delivery design (not implementation approval)

The questions above were assessed against root/scoped commands, reducers, planners, report refusal,
worker binding/publication and existing manual APIs. This section resolves the delivery-1 design;
it does not claim that the complete qualification design has been approved.

All 48 local/cached refs and 11 worktrees were checked for D/G/O identifiers before allocation;
maximum observed was 1805. No fetch or global reservation is claimed. D1806–D1809 are proposed here.

| ID | Proposed decision | Evidence and alternatives |
|---|---|---|
| D1806 | Deliver US01–US03 conservative defaults first; US04 remains an explicit evidence/design gate | No named provider, adapter release or authoritative guarantee exists in this assessment. Static authority can diverge across replicas; a PostgreSQL qualification registry is a later candidate, not infrastructure needed for denial now. |
| D1807 | Keep current wire; negative lookup reviews as RECONCILIATION_NOT_FINAL; unknown compensation never retries in delivery 1 | `execution/command.rs` currently maps NotApplied to RetryAt/Failed; `state_effects.rs` must allow NotApplied+Review. CompleteStep asserts finality; accepted/pending uses OUTCOME_UNKNOWN. No new pending RPC or compensation reconciler. |
| D1808 | Classify every leased Activity as external-capable; guard commands, reducers, planners and stop paths | `saga.rs` high_risk is author controlled. `sagas.rs` and `scoped_sagas.rs` admit unknown candidates today; `state.rs` must reject those plans as well. Internal control transitions remain internal; child effects retain child lifecycle handling. |
| D1809 | Fresh coordinated installation; no legacy replay or mixed server rollback | New NotApplied+Review and stricter compensation-plan validation change replay semantics even without SQL/proto shape changes. Preserve new data and fix forward after effects; no deletion or automatic replay authorized. |

Research disagreement resolved: one approach was to use `leaves_outcome_unknown` directly for all
attempt failures. The narrower source-aware approach is selected: keep attempt ambiguity distinct
from storage refusal and reconciliation outcomes, while covering every unknown code in obligation
preservation and tests. Root successful-output storage refusal becomes review; scoped atomic known
success remains known success. A broad code-string branch could otherwise discard proven success.
This resolution is proposed technical reasoning, not independent validation or a new product rule.

The initial planning draft preserved privileged retry of an unknown original and proposed a durable
refusal audit without defining its transaction. Independent review challenged both. The revised
proposal refuses direct Retry for every unknown original/compensation, even with SagaOverride;
existing outcome-bearing Resolve and compensation Skip remain. Known-failure retry is preserved.
No new in-place retry-after-resolution operation is added. This stricter interpretation of FR-037-009
is explicit in the design awaiting owner approval, not falsely recorded as an existing owner choice.

State-conflict refusal rolls back with no execution/outbox/idempotency-success/success-audit writes.
It is diagnosed by existing request tracing/typed conflict and retained review history. No new durable
conflict-refusal audit is promised. Existing authorization-denial auditing and accepted-decision atomic
audit remain unchanged. No new permission or automatic retention gate is added.

[Plan](plan.md), [data model](data-model.md), [contracts](contracts/README.md) and [tasks](tasks.md)
contain the reviewable proposal. US04 activation still needs provider evidence, exact authority and
revocation semantics, publication/use checks, identity and wire review. Those are not implementation
tasks disguised as an approved registry. No feature-037 acceptance test has run.

## 2026-11-01 — T037-028 test-location correction

`tests/integration/database-connections/consumers.rs` exercises ordinary worker secret delivery and
does not own the managed-database execution/result fixture. The real isolated connection,
`database-runner`, credential-delivery and result-validation setup is in
`tests/integration/database-connections/output.rs`; T037-028 is corrected to that file so its
cancel/deadline uncertainty tests exercise external SQL rather than an ordinary activity proxy.
This is a task-location correction only, not a new design decision or a claim that the provider
effect can be observed by the platform.

## 2026-11-01 — Delivery-1 verification scope corrections

Implementation exposed two additional enforcement locations within the approved contract.
T037-013 also covers `crates/infra-postgres/src/chain_progress.rs`: the locked database handoff
classifies blockers independently of the domain helper, so both must report retained recovery
obligations consistently. T037-025 also covers the existing component tests
`frontend/apps/studio/src/features/runs/operations.test.tsx` and real browser journey
`frontend/e2e/tests/us04-act.spec.ts`. This does not add a product capability or new UI flow.
T037-007's root command/reducer refusal cases live in `crates/domain/tests/sagas.rs`, with
known-retry controls in `operator_actions.rs`. Existing output-refusal domain coverage lives
in `execution.rs`; PostgreSQL coverage remains in the task's named integration modules.

The first PostgreSQL attempt could not connect to the documented Colima socket. Docker Desktop
is reachable at `unix:///Users/marcelo/.docker/run/docker.sock`; subsequent runs use the same
repository Testcontainers fixture there, without reusing another project's database.

Two new stopping tests initially treated reserve → submit as independent recovery. This was a
test mistake: reserve precedes unresolved submit in the same saga. The corrected oracle holds
the reverse plan until submit has an outcome; independent compensation coverage remains in
`terminal_failure_waits_for_independent_compensation`. The property oracle also retains an
existing ordinary operator hold inside an internally aborting saga; cancellation/deadline
continues to cancel ordinary held work. The generated minimal regression seed is preserved.

T037-027 also updates `tests/fault/sagas-compensations/crashes.rs`: the old crash-after-release
assertion demanded an automatic second compensation attempt after lease expiry, which delivery 1
forbids. It now proves one external call, retained review and outcome-bearing human resolution.
T037-026 checks undeclared fields at publication: draft creation intentionally preserves invalid
IR for editing and is not an execution admission boundary.
T037-029 includes `crates/application/tests/telemetry_derive.rs` for the actual review-span adapter
and the existing `tests/e2e/sagas-compensations/telemetry.rs` metrics gate.

The broad delivery-1 run also exposed legacy automatic-retry/cancellation expectations in
`tests/fault/workflow-primitives/scope_compensation.rs`,
`tests/integration/database-connections/cleanup_retention.rs`, and
`tests/integration/durable-execution/{concurrency,faults,scheduler}.rs`. Their fixture updates retain
stale-report fencing, crash/idempotency, quota-slot and credential-retention assertions while making
lease/timeout uncertainty wait for an explicit outcome-bearing resolution. Known-failure retry
controls remain unchanged. The separate `runtime_quotas.rs` failure is not part of this fixture
correction. This expands verification locations only; it adds no product behavior or test result.

## 2026-11-01 — T037-030 dependent-document synchronization

The four primary T037-030 documents remain `docs/{architecture,runbooks}/sagas.md`, `sdks/SPEC.md`
and this feature's `quickstart.md`. Consistency review found live lease/recovery and delivery-status
claims in coordinated dependants that would otherwise contradict those documents. The same
documentation delivery therefore also updates `SPECIFICATION-INDEX.md`,
`docs/architecture/{execution-semantics,worker-runtime-protocol}.md`,
`docs/runbooks/execution-engine.md`, `docs/{traceability,roadmap}.md`,
`docs/engineering/architecture-principles.md`, `docs/adr/{README,ADR-0076-compensation-failure-and-uncertain-effects}.md`,
and the later recovery notices in `specs/{003-sagas-compensations,014-workflow-primitives}/{spec,research}.md`.
The dated 003/014 decision bodies and ADR-0076 proposal context remain unchanged; only later status
notices and live current-contract wording are synchronized. This adds no architectural decision,
test result or qualification claim, and does not change T037-030 acceptance.


### Delivery-1 review correction: quota source and lookup drain

R1 reproduced a pending reconciliation after stop and an unconditional scoped content-quota
fallback that discarded the original report. T037-009 additionally touches
`crates/application/src/worker/scoped_reports.rs`: reacquire current authority/receipt/fence,
preserve the first stop, then retain scoped known success or the original uncertainty in one
transaction without output or a forward/error route. The unknown quota test injects the precise
PostgreSQL quota constraint once; ordinary unknown review itself allocates no retained payload.
Consequently the root expired-lease-at-full-storage control expects immediate payload-free
review, no child and no artificial quota stop. T037-012 releases declared recovery lookups while
draining; it does not reopen ordinary forward work. Evidence remains in the validation report.


T037-019 real-server prerequisite exposed a TypeScript conformance-runner defect: build-mode
stdout was cut at 512 bytes because `process.exit(0)` ran before the pipe write drained.
The shared scenario failed parsing its IR before booting the server. The runner now awaits its
stdout callback before exiting; it does not change SDK execution semantics or scenario output.
The failed first run is retained as `sdk-compensation-safety-first.txt`.

The T037-009 source-aware tenant quota correction also uses the existing
`crates/application/src/worker/scoped_output_refusal.rs` full-run refusal module.
Full-run acquisition is restricted to the exceptional rejected-content path; normal reports
remain bounded-path operations. Secret scan revisions are rechecked without rescanning.

R2 correctly rejected the first quota correction: root `stop_for_storage_quota` settles
an open structured owner and produces `UnsupportedStructuredNode`. The existing failed
regression confirmed the fallback returned Internal. T037-009 therefore additionally
exports `prepare_scoped_content_refusal` from domain execution: validate the exact live
scoped lease/ancestry, prepare only the first root stop, then persist the source outcome
in that same transaction. It reuses the stop preparation of logical-output refusal;
structured scheduler owners still perform drain. No new storage or scheduling framework.

The quota matrix's empty-route expectation was corrected after reading `plan_addressed`:
a stopped declared forward node deliberately remains unrouted, whereas derived completions
retain an empty route. Known success must be recorded without either forward or error
routing; the matrix now asserts zero routed events. Its one-shot constraint fault grants
USAGE only on the disposable test sequence to sagat_app, not on production objects.

T037-025 real Chromium run passed the existing two journeys and reached both new human
resolution outcomes. Their final assertion incorrectly equated journal actor `user:alice`
with audit subject `alice`. The corrected test asserts both exact representations and
unchanged actor/reason/resource provenance; it changes no production UI or audit behavior.

T037-008/010 scoped REST refusal reached the correct domain guard but returned HTTP 500:
`runs::execution_error` omitted `IllegalInstanceTransition` from the existing StateConflict
mapping. The collection negative-lookup test proved 500 instead of 409. The adapter now maps
that scoped refusal alongside its root counterpart; no authorization or domain rule changes.

The second Chromium run exposed a stale-view test race: API-only checks observed cancellation
before the browser projection refreshed, then Resolve correctly returned REVISION_CONFLICT.
The journey now explicitly uses the existing Refresh control and waits for its graph response
before opening the decision. Production optimistic concurrency remains unchanged.

## 2026-11-01 — Resumed defect verification (T037-026/027/031)

The REST negative case wrongly assumed unknown node properties are rejected. The canonical
workflow schema's nodeBase explicitly permits additionalProperties; delivery 1 does not change
that compatibility contract. The corrected case publishes the extension then proves that
OUTCOME_UNKNOWN with retryable=true still waits for review, leases no repeat/undo, and refuses
manual Retry. No qualification/purity exception is enabled.

The crash fixture unconditionally aged next_attempt_at after an unknown compensation entered
review. This manufactured a projection/journal mismatch; remove the obsolete retry timestamp
mutation and retain the no-second-call, human-provenance and replay consistency assertions.
The initial composed failures are retained in evidence/resume-sagas-contract-fault.txt.

T037-027 adds the existing `tests/fault/sagas-compensations/chaos.rs` to its traced locations.
The resumed suite reproduced its 120-second failure: it counted MANUAL_REVIEW as unfinished
worker work and required every run to auto-settle. The corrected oracle allows a retained
unknown-effect operator obligation, verifies replay and no terminal event for that run, and
asserts every synthetic provider call is the first call for its effect key. Settled runs
retain their prior success/compensation expectations. No engine retry exception is introduced.

T037-031 also fixes an unused domain-test import, replaces the new 13-column quota-evidence
tuple mapper with the existing named evidence struct using SQLx FromRow, and replaces a test
unwrap with a diagnostic expect. The preceding Clippy failures remain in the evidence directory.

The dependency audit still reports GHSA-g3hv-75wg-m249 (14 moderate transitive findings).
The upstream advisory lists minimatch 11.0.1 as fixed. Installed @redocly/openapi-core 1.x and
eslint-plugin-jsx-a11y 6.10.2 call the older callable CommonJS default; minimatch 11 exposes
named exports. The latest published jsx-a11y is still 6.10.2 with minimatch ^3.1.2, and the
latest openapi-typescript 7.13.0 depends on @redocly/openapi-core ^1.34.6. A blanket override
would violate those consumers' interface. No lockfile override, audit suppression, forced
downgrade or invented patched package was added. Gate remains open pending a compatible
upstream fix or separately reviewed tooling migration. Authoritative advisory:
https://github.com/advisories/GHSA-g3hv-75wg-m249 .


## Compatible minimatch resolution and final review findings — 2026-11-01

The preceding advisory blocker is superseded by the owner's instruction to resolve it while
preserving the current tools. A private local import bridge delegates all matching to unmodified
`minimatch-upstream: npm:minimatch@11.0.1`; it exposes both legacy callable CommonJS/default and
current named imports. The build identity `minimatch@11.0.1+sagat.1` identifies repository code,
not a claimed upstream distribution. Six scoped npm overrides select this workspace without
changing the other tools' versions. Global `file:` overrides produced broken nested links under
Node 22/npm10; the final canonical workspace link passes a clean filtered Docker install and
11 regressions, which now run in the Dockerfile as well as the root test command. Audit is zero.
The bridge and removal trigger are documented in
[its README](../../frontend/tools/minimatch-compat/README.md); no install-time mutation or
advisory suppression was added. Earlier failed dependency experiments remain evidence, not passes.

Runtime review found two completion gaps within the existing approved contract: a scoped
known-failure manual retry ignored stop/outer drain, and refusal of a successful output under the
logical run ceiling could be treated as known failure. Both now have observed RED/GREEN tests
and source-local fixes. Internal pre-execution limits keep `RUN_DATA_LIMIT`; flat collection
atomic known-success recovery is retained. An attempted nested-output integration fixture was
outside the public workflow subset and never reached report handling; it was removed without
expanding that subset, and its failed evidence is retained. Nested full/path retry is covered.

T037-029 now explains the exact observed list statement with the runtime role. The initial
common-prefix lookup also selected `get`; selecting uniquely by the list-specific ORDER/LIMIT
clause fixed that measurement defect. The final bounded experiment passed at all three scales;
it measures ordinary unknown reviews only and does not certify production capacity.


Three previously unchanged saga E2E scenarios still expected automatic repeat after negative
lookup or an expired compensation lease. Their observed failures are recorded in the correction
summary; they now require an explicit human decision and assert no second lease. A same-token
review-route test covers role demotion and membership deletion without changing production auth.

Compatibility boundary confirmed by the owner: feature-009 D47/D57 explicitly treats ordinary
secret-bearing worker output as final `SECRET_IN_OUTPUT` failure, while feature-037 task009
specifically addresses storage refusal. The broader plan wording about output/report rejection
must not be read as proof that every rejection path now retains recovery. The owner selected “Manter a regra da 009 e registrar a limitação nesta entrega”. Root/scoped
ordinary secret-scan behavior therefore remains unchanged; a source-aware cross-feature change
is deferred. No secret payload persistence is proposed. See docs/owner-decisions.md.
