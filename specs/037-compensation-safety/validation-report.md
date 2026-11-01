# Validation report — compensation safety

## Current delivery-1 verification status (2026-11-01)

Delivery 1 (US01–US03) is implemented and locally validated: all 32 delivery tasks are closed
with evidence. The final independent SDK E2E/fault targets passed 6 and 9 tests; all static gates
passed. There is no remaining local validation blocker in this approved delivery scope. The minimatch blocker is resolved: the private import bridge
uses pinned upstream 11.0.1, all six locked tool consumers pass, clean host and Node22 Docker
installation/build pass, and npm audit reports zero vulnerabilities. No toolchain-wide upgrade
or matcher implementation was introduced.

The owner retained ordinary feature-009 `SECRET_IN_OUTPUT` as final failure. Successful-result
storage refusal preserves recovery as specified by 037; no universal secret-output recovery,
qualified external provider, US04 completion, production release or live-data upgrade is claimed.

The first broad workspace command exited 101. Its original failures are retained in
`evidence/d1-final-workspace.txt`; corrected targets are verified separately below. The duplicate
primitives process with the diagnosed host logging conflict was explicitly replaced by the clean
full target, not counted as a pass. The clean primitives run had 749 passed, one fixture failure
and 22 explicit ignores; the corrected eight-test blob module then passed independently. The
parent also passed all 90 tests in the other changed primitives modules. Do not describe either
initial broad command as an exit-zero result.

Main completed evidence: Studio 1,194 tests and 11 tooling checks; 38 browser journeys (2 optional
performance cases skipped); database/OpenBao and onboarding browser journeys; four shared
compensation scenarios in all six SDKs; SDK per-language checks; contracts; runtime-role durable,
saga and managed-database integration; fault recovery, audit, authority and output-limit controls;
review-backlog measurement; compose smoke; audits and architecture checks. Exact commands,
results and the final retest ledger follow in the dated checkpoints.

This working tree is based on `6b6f0fa4bb7b622c960e9d149a4b78785ad0647b`;
no commit, merge or deployment is performed by this validation. The final changed-source snapshot
is recorded in `evidence/d1-source-tree.json` (base commit plus path/content hashes, excluding evidence).


## 2026-11-01 — Baseline characterization, before feature 037

Command executed during the B02 assessment:

```sh
CARGO_TARGET_DIR=<private temporary directory> CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 \
  cargo test --locked --offline -p sagat-domain --test sagas --test scoped_compensation
```

Result: exit 0; `sagas`: 60 passed, 0 failed, 0 ignored; `scoped_compensation`: 34 passed,
0 failed, 0 ignored. Toolchain pinned to Rust 1.98.0. The private build target was removed.
The run used develop@6b6f0fa4 with the approved B01 documentation changes; production code was
unchanged. The log is retained under [evidence](evidence/baseline-domain-tests.txt).

This was not rerun to validate the new specification: it characterizes existing behavior. It does
not close any new FR-037/SC-037 criterion. No PostgreSQL, HTTP/gRPC, SDK, distributed fault,
provider or financial integration validation ran in that assessment.

## Specification preparation

The specification and dependent proposal require review and static consistency checks. Results
will be appended after checking the composed artifacts. No implementation acceptance is claimed.

## 2026-11-01 — Specification and proposed normative patch

Independent EGM Design Critic review plus focused R2/R3 closure found the specification ready
for planning. All 12 requirement-quality checklist items were supported; those marks are not
implementation completion. Fixes clarified unknown-outcome precedence, human/provider provenance,
terminal quiescence, explicit-saga-only fail and operator guidance without a new admission gate.

Static validation passed: `git diff --check`; `python3 scripts/check-agent-config.py` returned
`agent configuration ok` in the real tree and in an isolated copy with the proposed 14-file patch.
The patch applied/reversed successfully; candidates matched bytes, new local links resolved,
34 FR/NFR/SC IDs were unique, and the other seven constitutional principles were unchanged.
The normative patch remains external and unapplied. These static checks do not prove runtime
behavior, provider guarantees or constitutional adoption. No runtime tests were rerun for this
documentary preparation. The active feature pointer remains 014; explicitly select 037 for planning.

## 2026-11-01 — B02 applied; delivery planning in progress

The earlier “external and unapplied” statement describes the preceding review snapshot. After the
owner's subsequent approval, the exact 14-file B02 wording patch was applied to this working tree
and verified against its candidates. Formal PR adoption remains pending; no runtime behavior was
changed by that patch.

Spec Kit's setup-plan/setup-tasks helpers were invoked with explicit
`SPECIFY_FEATURE_DIRECTORY=specs/037-compensation-safety` and `SPECIFY_FEATURE_NO_PERSIST=1`.
The installed 1.1.2 templates supplied plan/tasks structure. The extension-hook file is absent.
The feature pointer remains 014 and Git remains develop at 6b6f0fa4bb7b622c960e9d149a4b78785ad0647b.
No commit, merge, push or runtime test was performed for planning.

Initial planning static checks passed: 34 requirement IDs covered, task IDs unique and unchecked,
dependencies ordered, local linked files present, balanced fences and no template placeholders.
All existing task file paths were checked; the three new conformance fixture paths are explicitly
future artifacts. `git diff --check` produced no diagnostics; `python3 scripts/check-agent-config.py`
returned `agent configuration ok`. Baseline hashes found no changes outside the listed planning
and navigation records. Independent plan review and final static snapshot are recorded below when complete.

## 2026-11-01 — Planning package ready for owner review

[Independent EGM Design Critic R1/R2](evidence/planning-review.md) raised and resolved two blockers
and three major documentation/design findings. The revised proposal explicitly refuses direct Retry
of unknown original effects as well as unknown compensation; outcome-bearing human resolution is
distinct. It also uses the existing typed conflict/rollback path without inventing a durable
conflict-refusal audit. These are proposed design choices awaiting owner approval, not runtime fixes.
R2 found no remaining blocker for human design review. Two wording nits were then corrected.

[Static check output](evidence/planning-checks.txt): git diff --check exit 0;
check-agent-config.py exit 0 (`agent configuration ok`); planning validator exit 0. All 34 requirement
IDs have task/gate coverage; all 34 tasks remain unchecked (32 delivery-1 tasks, 2 US04 evidence/design
gates). The comparison covered 3,520 baseline files with no out-of-scope existing-file modification.
No runtime/source/config/active-feature-pointer change, build, commit or deployment occurred here.

Detailed design approval is the next gate. US04, provider qualification, feature-037 acceptance and
the overall engineering reset remain incomplete. The 94 earlier domain passes still describe the
old implementation and close no feature-037 criterion.

## 2026-11-01 — Approved implementation begins

T037-001: git worktree add -b feature/037-compensation-safety ../sagat-octopus-compensation-safety develop
completed at 6b6f0fa4bb7b622c960e9d149a4b78785ad0647b. 51 changed/untracked files from the reviewed
develop checkout were copied and SHA-256 verified. No original checkout file changed. Active pointer
is 037 only in the implementation worktree. Disk check reported 106 GiB available; no heavy build
was running. Private CARGO_TARGET_DIR and implementation evidence directory recorded in
/tmp/sagat-037-current.json. No commit or merge performed.

T037-002: approved recovery semantic delta synchronized into SDK/architecture docs and linked
feature model/contracts; git diff --check exit 0. No wire/property/stub added.

## 2026-11-01 — Implementation checkpoints (not final acceptance)

All commands below use the feature worktree at base `6b6f0fa4`, private target,
`CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0` and
`RUST_TEST_THREADS=2`. PostgreSQL commands use `SAGAT_TEST_AS_RUNTIME_ROLE=1` and the
Docker Desktop socket; the repository starts its own disposable container. No real provider
was contacted. No commit, merge or deployment has occurred.

- T037-003/005/007/011 red evidence: `root-red.txt`, `negative-red.txt`, `policy-red.txt`,
  `scoped-baseline-red.txt`. They show assertions against unsafe retry, candidate selection
  and erased review. The scoped baseline was reproduced by temporarily restoring only the
  three root-owned production files to base and restoring the working copies immediately;
  this is recorded separately from the subagent's test authorship.
- T037-004/006/011/012/015/016: `cargo test --locked --offline -p sagat-domain -p sagat-telemetry
  --no-fail-fast` passed **700 tests**, zero failures/ignored, including all domain saga,
  scoped, full/path parity, property and continuation tests.
  [Full output](evidence/domain-telemetry-final.txt). This does not prove PostgreSQL handoff
  classification or the pending source-aware application telemetry test.
- T037-007/008: the new operator test first failed with `Forbidden { permission: SagaOverride }`.
  After removing the obsolete privilege shortcut, the **80 integration_sagas_compensations
  tests passed**, including operator/admin refusal with unchanged durable effects, human
  provenance, held-lock decision/cancel race, negative lookup, and review through stopping.
  One explicitly ignored scale measurement has not run yet.
- Combined command: `cargo test --locked --offline -p sagat-tests
  --test integration_sagas_compensations --test contract_rest_sagas_compensations
  --test contract_grpc_sagas_compensations --test fault_sagas_compensations --no-fail-fast`.
  [Output](evidence/sagas-contract-fault.txt): gRPC **7 passed**; REST **3 passed/1 failed**
  because the new field-rejection test incorrectly treated draft creation as publication;
  fault suite **12 passed/2 failed** on obsolete automatic compensation retry expectations.
  Their test corrections are pending rerun; the combined gate is **not passed**.
- T037-025 intermediate: focused guard tests first failed **4/24**; after the guard/copy fix
  and correction of obsolete component expectations, `npm test` passed **1,194 tests in
  87 files**. `npm run lint` and `npm run build` passed. Real-browser journeys remain pending.
  [Suite](evidence/studio-suite-green.txt), [lint](evidence/studio-lint.txt),
  [build](evidence/studio-build.txt). The initial unused optional parameter lint failure was
  corrected by removing the obsolete argument from the dialog and guard signature.

Task checkmarks above close only the named checkpoint evidence. T037-031, all provider
qualification work and the overall engineering reset remain open. Source/test-location
corrections and fixture errors are recorded in research.md; no passing result is inferred
from a subagent report.


## 2026-11-01 — Independent runtime review, correction checkpoint

The fresh-context runtime reviewer identified actionable defects in stopped reconciliation
release, scoped tenant-content fallback, worker uncertainty classification and derived-node
compensation-plan validation. These are being reproduced and corrected; this is not an EGM
design review or a delivery pass. A cross-model CLI review was offered as optional and was
not executed.

- Stopped root and scoped lookup tests failed with Pending instead of Ready. After planner
  corrections, both focused tests passed (1 each), with RED/GREEN logs under evidence/.
- `cargo test -p sagat-application --test telemetry_derive --locked`: **21 passed**.
  The new review telemetry test first failed because `sagat.failure.code` was absent.
  The fix keeps the source verdict `sagat.cause.code=not_applied` and adds the typed
  review reason; it invents neither an attempt span nor a provider verdict.
- Original-worktree integrity rechecked: all **51** preserved files match their saved hashes.
- Studio `npm run check:api` passed. `npm audit --json` failed with **14 moderate package
  findings**, all associated with the same minimatch advisory through lint/schema tooling.
  [GHSA-g3hv-75wg-m249](https://github.com/advisories/GHSA-g3hv-75wg-m249) describes
  attacker-controlled glob amplification and lists 11.0.1 as patched. No dependency or
  lockfile changed in this feature. The audit gate is not passed; production reachability
  has not been established and the tool's suggested major downgrades were not applied.
- First TypeScript shared-scenario run failed before server execution because the build
  runner exited before its stdout pipe drained (512-byte truncated JSON). Awaiting the
  write is under verification; this failed run closes no SDK conformance criterion.

The broad PostgreSQL run exposed obsolete automatic retry expectations as well as the
new quota controls. Its failures remain evidence until the corrected cases and composed
tree pass. No full feature, provider qualification, reset completion or merge is claimed.

## 2026-11-01 — Resumed defect closure

Work remains in `feature/037-compensation-safety` at base `6b6f0fa4`; all pre-existing
changes were preserved. No commit, merge or deployment. This is defect verification,
not full delivery-1 or US04 acceptance. The original report missed later temporary
evidence; this checkpoint reruns the relevant gates instead of relying on those claims.

Environment: macOS arm64; repository-owned disposable PostgreSQL fixtures through
Docker Desktop; `SAGAT_TEST_AS_RUNTIME_ROLE=1`, `RUST_TEST_THREADS=2`,
`CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`.
Reused the feature's existing private external build directories (no unrelated target
was removed). Backend CARGO_TARGET_DIR ends in `sagat-037-target-q0fwz2gu`; SDK
SAGAT_RUST_SDK_TARGET_DIR ends in `sagat-037-rust-sdk-target-5xdnZU5Z`.
Free disk was checked before builds: 108 GiB, later 107 GiB.

Commands below ran with `--locked --offline`; each evidence file contains actual output.

| Command (cargo test unless stated) | Result | Evidence |
|---|---|---|
| `-p sagat-domain -p sagat-application -p sagat-telemetry --no-fail-fast` | 964 passed, 0 failed | [domain/application](evidence/resume-domain-application.txt) |
| `-p sagat-tests --test integration_sagas_compensations` | 81 passed, 1 explicitly ignored scale measurement | [initial composed run](evidence/resume-sagas-contract-fault.txt) |
| `-p sagat-tests --test contract_grpc_sagas_compensations` | 9 passed | [initial composed run](evidence/resume-sagas-contract-fault.txt) |
| `-p sagat-tests --test contract_rest_sagas_compensations` | 4 passed | [REST final](evidence/resume-rest-final.txt) |
| `-p sagat-tests --test fault_sagas_compensations` | 14 passed | [fault final](evidence/resume-fault-final.txt) |
| `-p sagat-tests --test integration_workflow_primitives -- chains::` | 22 passed | [chains](evidence/resume-chains.txt) |
| `-p sagat-tests --test integration_workflow_primitives -- scoped_worker::` | 20 passed | [scoped workers](evidence/resume-scoped-worker.txt) |
| `-p sagat-tests --test integration_workflow_primitives -- scope_compensation_http::` | 21 passed | [scoped HTTP](evidence/resume-scoped-http.txt) |
| `-p sagat-tests --test integration_durable_execution -- runtime_quotas::` | 10 passed | [quotas](evidence/resume-quotas.txt) |
| `-p sagat-tests --test integration_workflow_primitives scoped_content_quota_fallback` | 1 passed after named SQL row cleanup | [final quota row](evidence/resume-quota-row-final.txt) |
| `SAGAT_STUDIO_GREP=T037 cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture` | 2 real Chromium journeys passed; Studio build passed | [browser](evidence/resume-browser.txt) |
| `python3 scripts/check-architecture.py` | passed, 24 crates | [architecture](evidence/resume-architecture.txt) |
| `python3 scripts/check-agent-config.py` | passed | [agent config](evidence/resume-agent-config.txt) |

Initial failures are preserved. REST incorrectly expected node extensions to be rejected
although the canonical schema permits them; it now proves those fields cannot activate
a repeat/undo exception on the published runtime. Its intermediate assertion confused
the worker's opaque step UUID with the authored node; corrected to activity identity.
The crash fixture incorrectly manufactured next_attempt_at for a review-waiting step,
creating a projection mismatch. The chaos test incorrectly waited for review to become
terminal automatically; it now verifies durable unknown-effect review and rejects every
duplicate synthetic provider call. See research.md for the T037-027 location amendment.

Clippy also exposed an unused import, a large quota-evidence tuple and an unwrap in
a new fault-test helper; these were corrected without changing runtime semantics.
The first SDK rerun lacked the required separate Rust target variable and failed during
prepare; this is retained in evidence/resume-sdk-six.txt and is not a conformance result.

Open blocker: `npm audit --json` still reports 14 moderate transitive minimatch findings
([audit](evidence/resume-audit-before.json)). The available patched major is incompatible
with callable CommonJS consumers still required by the current tooling. No audit exception,
forced override or downgrade was introduced. The overall gate remains unpassed pending
a compatible upstream fix or reviewed tooling migration. Full workspace tests, all broader
CI/compose/security/SDK regression gates and US04 are not certified by this checkpoint.

Final checks in this resumed run:

- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: exit 0
  ([output](evidence/resume-clippy-verified.txt)); preceding failures are retained separately.
- `SAGAT_SDK_SCENARIO=compensation-safety cargo test --locked --offline -p sagat-tests
  --test sdk_conformance -- --ignored --nocapture`, with the separate Rust target variable:
  all six languages passed, no bless, 63.06 seconds
  ([output](evidence/resume-sdk-six-final.txt)). This verifies the TypeScript pipe-write fix
  in the real cross-language harness. No golden was changed in this resumed run.
- `cargo fmt --check` and `git diff --check`: exit 0.

T037-013 is closed with current domain and locked-handoff evidence. Broader tasks remain
unchecked where a checkpoint does not prove their whole acceptance matrix.


## Delivery-1 closure run — 2026-11-01 (in progress)

The previous minimatch blocker is resolved without upgrading the locked tooling, suppressing
advisories or running dependency lifecycle scripts. The local workspace `minimatch@11.0.1+sagat.1`
is only an import bridge; matching delegates to the integrity-pinned upstream alias at 11.0.1.
Six consumer-scoped overrides produce one canonical link. A global relative `file:` override
passed the host install but produced broken nested links in Docker; that failed attempt remains
in `evidence/d1-minimatch-node22.txt` and is **not** passing evidence. The final Dockerfile runs
all compatibility regressions immediately after its Node 22 workspace-filtered `npm ci`.

Verified results on the final dependency tree:

- Clean host install, all six consumer resolutions, bounded amplification regression, ESM/CJS,
  actual JSX accessibility helper and Redocly header matching: 11 tests passed; audit 0 findings
  (`evidence/d1-minimatch-workspace-host.txt`).
- Exact Studio Docker stage: filtered installation, 11 compatibility tests and build passed
  (`evidence/d1-minimatch-workspace-studio-image.txt`). Parent independently ran the image's
  `npm run test:tooling`: 11 passed (`evidence/d1-minimatch-docker-independent.txt`).
- Final frontend audit, lint, Studio/e2e type checks, tests, build and API-drift checks all exit 0
  (`evidence/d1-frontend-final-checks.json`). Tests: 11 tooling + 1,194 Studio.
- All canonical contracts passed: OpenAPI lint, Protobuf compilation, AsyncAPI validation and
  the payment workflow JSON Schema validation (`evidence/d1-contract-checks.json`).
- SDK TypeScript, Go race/vet/format, .NET test/format, Java verify and Rust fmt/clippy/tests
  passed (`evidence/d1-sdk-checks.json`). Python initially lacked test dependencies, so those
  initial entries are failures, not passes. Installing the repository's hashed dev lock then
  passed 264 tests, mypy and ruff (`evidence/d1-python-final.json`).
- Architecture, capacity activation, upgrade boundary, vendor integrity and agent configuration
  passed, as did both Cargo advisory audits under the existing policy
  (`evidence/d1-static-checks.json`). No advisory exception was added or widened.
- The script test suite initially blocked in its pipe fixture before the bounded reader could
  start (captured `d1-diagnostics-pipe-red.txt`). Concurrent fixture writing removes the platform
  pipe-capacity assumption; final result: 76 tests, zero failures, one explicitly opt-in Docker
  diagnostic skipped (`evidence/d1-static-final-tests.txt`). Independent review confirmed the
  reader-close/kill/join path preserves the deadline and return-code assertions.

### Review backlog observation (T037-029)

`SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked --offline -p sagat-tests --test
integration_sagas_compensations ordinary_unknown_reviews_remain_idle_at_one_one_hundred_and_one_thousand
-- --ignored --nocapture`: passed in 28.38 seconds (`evidence/d1-review-query-binding.txt`).
EXPLAIN uses the actual normalized list statement, selected uniquely by its list-specific clause,
with the execution role and the same 60-second observation timeout. Earlier compile and query
selection failures are retained separately; selecting a get statement by the common prefix was
fixed, not retried unchanged.

Fixture: macOS arm64 Mac15,9, 16 CPUs, 48 GiB host RAM, Docker 16 CPUs/~7.75 GiB;
PostgreSQL 16.15, disposable repository fixture. Host load averages at entry: 4.90/9.23/11.42;
94 GiB host disk free (`evidence/d1-review-host.txt`). Guard: 1,000 reviews maximum, 600 seconds
setup per scale, 60 seconds per observation, 1 GiB measured relations, 20 GiB minimum free host
disk before launch; two build jobs and two Rust test threads. No budget was exceeded.

| Synthetic reviews | Table bytes | Index bytes | First-page latency | Idle SQL calls | Worker leases |
|---:|---:|---:|---:|---:|---:|
| 1 | 90,112 | 737,280 | 1.84 ms | 16 | 0 |
| 100 | 483,328 | 851,968 | 2.11 ms | 16 | 0 |
| 1,000 | 2,678,784 | 3,342,336 | 2.25 ms | 16 | 0 |

At every scale one scheduler tick plus one poll created no work, retry, lease, journal event or
ready hint. Plans used backward indexed run lookup and indexed chain lookup. These are single
observations of ordinary unknown forward reviews, not percentiles, a production SLO or a claim
about every scoped recovery shape. Broader stop/scope invariants are exercised separately.

Full final workspace, six-language regression scenarios, browser and compose checks are still
running or pending at this checkpoint. This section does not close T037-031 or US04.


### Additional final-tree gates completed

- Four shared scenarios (`compensation-safety`, `saga-compensation`, `collection-compensation`,
  `loop-compensation`) passed in **all six SDKs**, without bless or golden edits in this closure run
  (`evidence/d1-six-checks.json`, corresponding `d1-six-*.txt`).
- Full Studio journey harness, with the real sandbox enabled: **38 Chromium journeys passed**;
  two explicitly opt-in performance benchmarks skipped (`evidence/d1-browser-all.txt`).
- `scripts/smoke-compose.sh` with the built CLI and `SAGAT_SMOKE_BROWSER=1`: passed health,
  migrations, runtime-role checks, CLI/SDK execution and the real onboarding browser journey
  (`evidence/d1-smoke-checks.json`, `d1-compose-smoke.txt`). Its own disposable project cleaned up.
- Saga E2E: **21 passed**, including current-token permission demotion and membership deletion,
  uncertain outcome refusal/resolution, human provenance and telemetry
  (`evidence/d1-saga-e2e-final.txt`).
- Parent independently reran durable recovery E2E after updating three old crash/retry oracles:
  **16 passed** (`evidence/d1-parent-durable-e2e.txt`); parent also reran the two corrected gRPC
  cases (`evidence/d1-parent-grpc-checks.json`) and the CLI negative-lookup case, **1 passed**
  (`evidence/d1-parent-dx-e2e.txt`). REDs remain in the first workspace log.
- The first workspace run exposed missing local CI prerequisites for the C# sandbox case: both
  repository-pinned .NET images were absent, so capability discovery did not claim C# work.
  Building the exact images from the CI Dockerfiles succeeded (`evidence/d1-dotnet-images.json`);
  the unchanged scenario then passed (`evidence/d1-dotnet-runtime-final.txt`). This is environment
  preparation, not a silently retried flaky assertion or a runtime-policy change.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` passed at the
  checkpoint before the later test-only oracle updates (`evidence/d1-final-clippy.txt`). A final
  check after those edits is still required. `cargo fmt --check` passed (`evidence/d1-fmt.txt`).

The broad workspace run is still in progress and retains failures from the obsolete oracles
already corrected above. It must not be represented as an exit-zero command. Complete its
remaining tests and record the final retest results before closing the integrated gate.


### Owner-confirmed limit

On 2026-11-01 the owner chose to keep the feature-009 final `SECRET_IN_OUTPUT` rule in this
first delivery. Successful-result storage refusal is covered; ordinary secret-bearing output
rejection is not claimed to preserve recovery review. Secret suppression and existing managed
SQL protections remain. The decision is recorded in `docs/owner-decisions.md` and the spec/plan.


The broad run inherited `RUST_LOG=warn` from the host. Some engine fixtures then install a global
logger before the tracing collector, whose explicit installation correctly refuses a second
global dispatcher. The acquisition observation test reproduces that failure unchanged, and passes
unchanged with `env -u RUST_LOG` (`d1-primitives-acquire-diagnostic.txt` /
`d1-primitives-acquire-no-log.txt`). Final collector gates unset that host-only variable; no tracing
or evidence assertion was weakened. The interrupted primitives run is retained as
`d1-workflow-primitives-rust-log.txt`: its shutdown triggered Testcontainers cleanup and subsequent
EOF/assertion failures while the harness was still exiting, so that tail is not a valid defect
sample. The clean full primitives run is the authoritative result.


### Independent retest checkpoint — runtime oracles

The five corrected workspace targets passed on the composed tree: gRPC durable execution
70, developer-experience E2E 32 (5 pre-existing opt-in cases ignored), durable E2E 16,
inline-script E2E 28, saga E2E 21 (`evidence/d1-workspace-corrected-groups.txt`).
The parent also reran both full fault targets with `env -u RUST_LOG`: inline scripts 12
and workflow primitives 92 passed (`evidence/d1-parent-sandbox-workflow-faults.txt`, exit 0).
The latter includes the secret-redaction collector test, unchanged; this confirms the
host logging conflict rather than a secret-handling regression.

The first broad run additionally exposed an obsolete durable-output budget assertion;
its correction and complete integration retest remain in progress. No broad exit-zero
result or integrated T037-031 completion is claimed at this checkpoint.


The parent's independent runtime-role rerun of every modified primitives module passed
90 tests (`evidence/d1-parent-primitives-changes.txt`: byte_ceilings, chains,
scope_compensation_http, scoped_worker). The complete 772-test target remains in progress.
The parent also passed the complete fault_failover (6) and fault_security_multitenancy (10)
targets (`evidence/d1-parent-security-failover.json`). This includes retained lost-worker
obligations, stale runner fencing and a separate authorized script-denial control.

Additional CI browser targets passed: the database/OpenBao journey (4 Playwright cases) and
the real compose/CLI/SDK/Studio onboarding journey (`evidence/d1-extra-browser.json`).
The full opt-in SDK CI targets are now being checked beyond the four selected compensation
scenarios; they exposed additional obsolete expectation(s), so that broader gate stays open.


The complete durable-execution integration target passed 179 tests after the budget-oracle
correction (`evidence/d1-durable-output-budget-green.txt`). The parent independently reran
all hardening and runtime-quota tests: 27 passed (`evidence/d1-parent-durable-output.json`).
The original failed assertion and new assertions retain the same output-ceiling crossing;
the final result is review with no stored refused output or automatic retry, and exact byte
accounting plus duplicate-report replay remain verified.


### Delivery-1 demonstrations and operating boundary (T037-030)

| Demonstration | Observed evidence |
| --- | --- |
| Cancel/deadline cannot erase review; human outcome has provenance | Saga integration 81 passed, including `cancelling_after_review_preserves_the_unknown_effect_until_a_human_resolves_it`, `deadline_after_review_preserves_the_unknown_effect`, and late compensation-failure controls; saga E2E 21 passed with authority revocation and telemetry. |
| Explicit permanent compensation failure waits for independent work | `terminal_failure_waits_for_independent_compensation` and `unknown_compensation_overrides_fail` passed in the saga integration target; domain invariant/property suites also passed. |
| Unknown/negative lookup cannot repeat or create defensive compensation | `negative_lookup_never_leases_second_attempt_and_fences_the_stale_report`, no-attempt-left and absent/false high-risk controls passed; four compensation scenarios passed across six SDKs. Broader SDK CI remains separately open. |
| Crash, duplicate, lost hint and decision/cancel race preserve one outcome | Saga fault 14 and workflow fault 92 passed; held-lock/audit rollback cases passed in saga integration and the parent 90-case primitives retest. |
| Successful output refused by storage retains recovery | Durable integration 179; parent hardening/quota 27; parent primitives 90; corrected scoped-blob module 8 passed. Flat collection atomic known-success behavior stays distinct from unknown output loss. |
| Fresh installation and rollback boundary | Repository compose smoke installed and exercised CLI/SDK/Chromium, then removed its isolated project. The runbook documents removal only before external effects and preservation/fix-forward afterwards. No backup restoration, live-data downgrade, real-provider reconciliation or production rollback rehearsal was executed or claimed. |

Operational review confirmed the documented G28 journal retention/partitioning gap and G182 audit
retention/export gap. Operators own authorized evidence export, backup/PITR policy and follow-up;
delivered-outbox cleanup is not evidence retention. Explicit terminal failure has no promised
post-terminal recovery queue. These are recorded constraints, not placeholders for nonexistent
platform guarantees. The feature-009 ordinary secret-output limit remains explicit.

The original broad workspace exited 101 (`evidence/d1-backend-final-checks.json`), with diagnosed
and retained REDs. Additional independent retests passed: scoped blob completion 8, scalability
integration 44, and workspace doc compilation (`evidence/d1-parent-scoped-blob.json`,
`evidence/d1-late-gates.json`). The scheduling-weight test now uses the existing owner-only
operator path and asserts sagat_app remains SELECT-only; no production permissions changed.
The doc compilation's earlier missing-crate metadata error occurred amid shared-target
recompilation; the serial workspace doc gate passed without a source fix.


The parent independently reran the complete security/multitenancy integration target: 126 passed
(`evidence/d1-parent-security-integration.json`). The corrected tests preserve token/binding
revocation and delayed incident classification, while distinguishing expired unknown work from
a known retryable failure and a separately authorized new run. No production permissions changed.

Final review identified two existing opt-in CI commands beyond workspace defaults: executable
SDK examples (already queued after the full SDK run) and five native Go/Java sandbox checks
(now running). Neither is counted as passed merely because its prerequisite images exist.


Native Go/Java CI gate completed: the first five checks failed `Unavailable` because all four
repository-pinned builder/runtime images were absent (confirmed by image inspection). Building
exactly the CI Dockerfiles/tags succeeded (`evidence/d1-native-images.json`), then the unchanged
five tests passed (`evidence/d1-native-sandbox-ready.json` / `.txt`). Production isolation, native
loading, quota stop, host calls and cleanup were exercised on local Linux arm64 containers. The
amd64 CI matrix was not executed locally. No production or test-source change was needed.


The full opt-in SDK pass completed (exit 101, retained in `evidence/d1-sdk-ci.txt`): all 60
runtime scenarios ran across all six languages, with six additional builder-only scenarios.
All runtime outputs agree across languages. Exactly three scenarios differ from the obsolete
goldens: failures-and-retries, lease-lost and worker-shutdown; the other 57 runtime scenarios
match. E2E wrappers had 4 passed/2 old assertions failed; fault wrappers had 7 passed/2 failed.
Reviewed candidate updates are now being applied to those scenarios and to the saga human-audit
assertions. Final six-language conformance and wrapper retests remain required; the first pass
is not reported as green. Executable examples continue in the second queued CI command.


## Final delivery-1 result — 2026-11-01

All 32 delivery-1 tasks are closed with observed evidence. US04 tasks T037-032/033 remain open.
The final gate is a composed validation of unchanged broad-run cases and corrected-target retests;
neither the initial workspace command (exit 101) nor the first full SDK command (exit 101) is
misrepresented as green. No unresolved failure remains among the applicable delivery-1 checks.

| Gate | Final evidence/result |
| --- | --- |
| minimatch and frontend | Clean host and Node22 Docker installs; 11 actual-tool regression tests; audit zero; Studio 1,194 tests; lint/types/build/API checks passed. `d1-minimatch-workspace-host.txt`, `d1-minimatch-docker-independent.txt`, `d1-frontend-final-checks.json`. |
| Browser and fresh install | 38 Studio journeys passed (2 optional performance cases skipped), four managed-database browser cases, onboarding and compose smoke passed. `d1-browser-all.txt`, `d1-extra-browser.json`, `d1-smoke-checks.json`. |
| SDK per-language quality | TypeScript, Go race/vet/format, Python 264 tests/mypy/ruff, .NET test/format, Maven verify, Rust SDK fmt/clippy/test passed. Python's missing locked development tools were installed before its successful retest. `d1-sdk-checks.json`, `d1-python-final.json`. |
| SDK shared conformance | Initial full 60 runtime × six languages plus six builder-only scenarios found exactly three obsolete goldens, no language divergence. Four updated scenarios (including explicit saga audit evidence) passed all 24 combinations without bless. `d1-sdk-first-full-comparison.json`, `d1-sdk-fixture-review.txt`, `d1-sdk-*-green.txt`. Earlier compensation-safety/collection/loop gates also passed in all six languages (`d1-six-checks.json`). |
| SDK independent full wrappers | Parent E2E 6 passed and fault 9 passed, exit 0, all six languages; no bless or language filter. Prepared artifacts reused only after unchanged SDK-source hashes were verified. `d1-parent-sdk-wrappers.json`. |
| Executable examples/native sandbox | Four CI example tests passed across the required languages; five unchanged native Go/Java isolation/loading/quota/host-call/cleanup checks passed after building the pinned repository images. `d1-sdk-ci.json` (second command), `d1-native-sandbox-ready.json`. |
| Domain/application/telemetry and unchanged workspace targets | Passed in the broad workspace and earlier focused runs. Original failures below are superseded only by their explicit retests, not silently discarded. `d1-final-workspace.txt`, `resume-domain-application.txt`. |
| Corrected gRPC/E2E targets | gRPC durable 70; developer-experience E2E 32 plus its five opt-in journeys/examples; durable E2E 16; inline-script E2E 28; saga E2E 21 passed. `d1-workspace-corrected-groups.txt`, `d1-extra-browser.json`, `d1-examples-ci.txt`. |
| Corrected fault targets | Failover 6, security 10, inline scripts 12, workflow primitives 92 passed independently. `d1-parent-security-failover.json`, `d1-parent-sandbox-workflow-faults.txt`. Saga fault 14 passed in the broad run. |
| Runtime-role integrations | Saga 81, durable 179, managed database 7 wrappers, scalability 44 and security 126 passed. Parent independently reran durable output/quota 27 and the entire security target. `d1-final-workspace.txt`, `d1-durable-output-budget-green.txt`, `d1-parent-durable-output.json`, `d1-late-gates.json`, `d1-parent-security-integration.json`. |
| Runtime-role primitives | Clean full run: 749 passed/one fixture failure/22 explicit ignores. The corrected blob module passed 8 tests independently; parent also passed all 90 tests in the other changed modules. Thus all 750 applicable cases have passing evidence, without claiming an exit-zero original full target. `d1-workflow-primitives-full.txt`, `d1-parent-scoped-blob.json`, `d1-parent-primitives-changes.json`. |
| Static/contracts/advisories | Final fmt, workspace all-target Clippy with `-D warnings`, architecture, agent configuration, diff check and doc compilation passed; canonical contracts and existing-policy Rust audits passed. Pre-existing vendored rskafka warnings remain in dependency build output. `d1-composed-static.json`, `d1-late-gates.json`, `d1-contract-checks.json`, `d1-static-checks.json`. Diagnostic scripts: 76 passed, one opt-in Docker case skipped (`d1-static-final-tests.txt`). |
| Operations | Review observation at 1/100/1,000 recorded bytes/index plans/list latency/idle SQL with machine budgets; docs/demo mapping and 100 local file links passed. `d1-review-query-binding.txt`, `d1-delivery-doc-links-final.txt`. No invented SLO. |

Exact environment and artifact provenance: `d1-tool-versions.json`, `d1-review-host.txt`,
`d1-source-tree.json`, and `d1-sdk-reuse-source-sha256.json`. The main backend and Rust SDK targets
were reused private directories and were preserved; only execution-owned temporary experiments
were removed. The original develop checkout remains untouched.

Limits remain intentional: ordinary SECRET_IN_OUTPUT follows the owner-retained 009 final-failure
contract; the unsupported nested-output runtime fixture is not certified; the 22 primitives
ignores comprise one 10,000-item opt-in case, 19 immutable-old/release-upgrade cases and two private
probes exercised by their passing parent tests. This fresh-install delivery does not certify live
data upgrades, amd64 native CI, backup restoration, real-provider finality, regulatory retention,
US04, a production launch or formal Constitution/ADR adoption. No tests or guarantees are claimed
for those excluded activities.
