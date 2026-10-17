# Integration checkpoint — 2026-10-17

This is an implementation checkpoint, not a completed release gate.

## Reconciled boundaries

- Preserve all 100 migration files from the combined 014 baseline byte for byte.
- Allocate 017 migrations after that history (20261122–20261128); preserve its standalone branch.
- Preserve scoped protobuf field numbers; reserve managed capabilities for minor1.6 independently of scoped minor1.5. Default adapters keep ordinary workers at minor1.4 until scoped execution is enabled.
- Bind managed credential pins and all managed step/journal selections to root identities; reject unproven managed/structured or continuation composition at publication and typed construction.
- Keep provider reads outside transactions; repeat current role, lease, publisher ACL, binding and pool checks before disclosure and commit its audit first. Use binding → session (managed) → catalog → membership ordering, retain final grant locks through commit.
- Bind optional worker pools into durable OpenBao intents and apply the restrictions with activation. Omitted pools preserve existing policy; changed replay metadata conflicts.

## Executed checks

- `cargo test -p sagat-application --lib worker::tests:: --locked`:8passed,0failed; includes ordinary/managed protocol feature-gate matrix.
- `cargo test -p sagat-infra-ir --test database --locked`:8passed,0failed; includes publication composition boundary.
- `cargo check --workspace --all-targets --locked`: passed after integration fixes (dev profile,1m06s including shared-cache contention). This compiles test targets but does not execute them.
- `python3 scripts/check-architecture.py`: passed,24workspace crates, no dependency cycles.
- Canonical `worker.proto` compiled with `protoc` into a descriptor.
- Automated byte comparison against HEAD: all100preexisting migration files unchanged.
- Storage classification metadata:387entries,0duplicate(table,column)keys.
- Python and Go protobuf code regenerated; Studio OpenAPI types regenerated from the combined017/014contract. References and future016types still need integrated regeneration.

## Pending verification

Workspace all-target compilation exposed cross-feature interface mismatches; the final check passed after correcting them. Earlier failed attempts are not passing test evidence. The database composition RED fixture exposed the existing body-kind schema refusal; its assertion was corrected to accept the documented schema refusal and separately demand the mixed-root publication gate. Real PostgreSQL/OpenBao pool/authority races, scoped regressions, all-five-SDK runtime conformance and the final full gates remain pending.

Independent Astra xhigh static review found and drove fixes for pool bypass, stale ordinary worker membership/ACL authority, same-named scoped lease deadlines, binding/session deadlock order and protocol feature-gate negotiation. It ran no executable tests. A subsequent final review remains required after integration.

## Classification integration checkpoint

- New normal and private ad hoc versions pin interpreter rules 1; legacy migration defaults to 0.
- `integration_payload_classification` ran 8 PostgreSQL/HTTP tests successfully on the combined tree
  (`/tmp/sagat-completion-grants-http-green.log`): immutable pinning, byte preservation across the
  exact up/down DDL, grant rules/tenant/cascade, stale actor refusal, audit rollback, a concurrent
  256-grant ceiling, revocation blocked until an effective grant read transaction ends,
  repeated DELETE returning 404, no-store metadata, and oversized request refusal.
- This is focused integration evidence. It does not establish old-binary compatibility, full
  classified disclosure behavior or the final release gates. Classifier review findings remain
  under correction before value disclosure is mounted.

### Publication and SDK checkpoints

The production IR codec now runs the closed classification findings before the general workflow
schema, and validation includes its bounded report. Two codec tests first failed (SCHEMA instead
of CLASSIFICATION_NOT_APPLICABLE; a lowered declaration accepted) and passed after wiring
(`/tmp/sagat-completion-publication-{red,green}.log`). Full REST report validation remains pending.
Named-body report rows carry an optional `bodyName` to disambiguate repeated node identifiers.

SDK verification at `a74ec176` used the existing pinned runners and found no SDK defect:
TypeScript 130 tests, Python workflow 14, .NET builders 80, Java builder 24, and Go workflow
package passed. All five classification outputs matched the golden: 1,474 canonical carrier
bytes, SHA-256 `0700a0eb7dfff6baed1a3173e14f77e2a178a33db30d7b9cb4222dfd42dcb046`.
This focused proof does not replace the final complete protocol/runtime conformance matrix.

### Managed secret capability checkpoint

`integration_database_connections` passed all 3 top-level tests on the combined publication tree
(`/tmp/sagat-completion-database-integration-4.log`, 65.03 s), including its aggregated real
OpenBao lifecycle, database executor, pool revocation/provider-I/O races, output and blob cases.
Worker registration now requires persisted current membership with WorkerExecute; test worlds
persist their default worker identities. The aggregate test allocates an explicit 16 MiB test
thread stack and acquires a fresh lease immediately before the final revocation race. This
removes the earlier default-harness stack overflow and an expired-lease barrier wait; production
runtime stack/lease settings are unchanged. Final combined workspace gates remain pending.

### Studio precision checkpoint

The disclosure view now requests safe numeric parsing. A textual response containing
`9007199254740993` reproduced the rounded display in a failing component test; the corrected view
refuses numbers the browser cannot preserve and points the user to exact API/CLI representations.
Seven disclosure component tests, Studio typecheck and focused ESLint passed
(`/tmp/sagat-completion-ui-precision-{red,green,typecheck,lint}.log`). The server keeps its exact
numeric encoder. Final classifier review still has pending numeric-applicator and hidden-child
budget regressions; the production disclosure service remains unwired until these are corrected.

### Numeric validation and hidden-child budget checkpoint

The second security review closed indirect integer restrictions and hidden-child projection
budgets in `e5145923` (18 focused tests / 32 schema package tests). Two further exact repros
were added: `not`/`if` with `uniqueItems`, and an integer `multipleOf` divisor above the
dependency's exact compilation range. Both reproduced disclosure of mathematically invalid
values before the guards. All 20 focused classifier tests then passed
(`/tmp/sagat-completion-{unique-items,multiple-of}-{red,green}.log`). Numeric comparisons fail
closed when exact validation cannot be established; large integer values remain supported with
proven exact small divisors. This conservative boundary and the final service composition still
require the final integrated security/contract gates.

### Disclosure composition and deferred audit failures

The run disclosure service and US08 event read service are now mounted in both production and
real-API test factories. Root's combined PostgreSQL run passed 18 of 19 cases; the remaining result
case exposed a logical-output ledger defect: an `end` output created in the same transaction had
no preexisting projection row. A narrow fix and exact two-reference/replayed-report assertion are
under verification; this checkpoint does not claim the result case passed.

Two real PostgreSQL fault tests use `DEFERRABLE INITIALLY DEFERRED` audit triggers. The denial
commit first returned an internal storage error instead of the required typed refusal. Both run
and event denial paths now treat the entire denial audit transaction as best effort, including
commit/rollback failures, while a failed mandatory success commit still returns no values. Both
fault tests passed (`/tmp/sagat-completion-denial-commit-green.log`, 1.99 s). The initial success
control's expected error class was corrected from unavailable to the adapter's actual internal
classification; its refusal of values was already closed.

The actual authoring REST contract target passed 13/13
(`/tmp/sagat-completion-authoring-contract-green.log`). Its unrelated-operation digest baseline
changed only for approved 014 `getChain` pagination guidance (112f1e1d); the operation remains in
the checksum. API request unit tests passed 4/4 after endpoint-only unknown-field handling. Actual
REST audience and request boundary gates are still being finalized by the owning test stream.

### Authoring review corrections

Independent static review found reference-reachable annotations beneath unsupported applicators,
and two diff fallback errors involving explicit ancestors and secret inheritance. Commit
`b1d96a4b` reproduced the publication and diff regressions RED, then passed publication classification
14/14, semantic classification diff 14/14, publication 3/3 and carrier compilation 2/2 on its isolated
warmed target. Focused adapter Clippy passed. These fixes are merged; complete integrated gates remain.

### Python SDK integration gates

The combined Python SDK passed 261 tests in 22.23 s
(`/tmp/sagat-completion-python-tests.log`). Mypy passed all 38 checked files and Ruff passed after
renaming the chain page local variable (which had reused the run-page inferred type), sorting the
public export list, and applying the pinned formatter. This is SDK-local proof, not the all-five
runtime server conformance gate.

### Combined HTTP, result and browser checkpoint

At `a08bb21b` plus the narrow logical-output ledger fix, the combined target command passed
15 contract, 2 fault and 21 real PostgreSQL/HTTP integration tests
(`/tmp/sagat-completion-classification-combined-gates-3.log`). This includes the complete 96-cell
REST audience matrix, fixed selector/reveal/unknown/malformed errors without reflection, typed
1KiB refusal, private/no-store success and error responses, opaque tenant resolution with no foreign
debit, and exact namespace admission. The result regression now passes and confirms two logical
output references despite shared content, with no extra byte charge on an idempotent report replay.

The actual Chromium disclosure journey passed 1/1 (`/tmp/sagat-completion-disclosure-browser.log`):
viewer, developer, operator and revealer projections, forbidden canary absence in wire and DOM,
keyboard confirmation, PII absent from browser stores and cleared on close. Browser time 2.5 s;
real API/PG harness 7.66 s. The focused command used `SAGAT_STUDIO_NO_SANDBOX=1`, so the unrelated
script sandbox journey was explicitly excluded; this is not the full Studio gate or human visual
acceptance. The production Studio build passed 16.60 s before that run.

The cache benchmark report distinguishes D281's 16 MiB serialized schema weight from measured
compiled retention (~414.24MiB requested live heap, not RSS). A reviewed concurrent-miss accounting
fix and bounded compile admission are in progress. No actual 16 MiB heap guarantee is inferred.


### Registered package, UI and projection gates

- `cargo test -p sagat-domain -p sagat-application -p sagat-infra-schema -p sagat-infra-ir -p sagat-cli`
  passed: 992 tests, 86 targets (including zero-test doctests), no failures or ignored tests.
  Log: `/tmp/sagat-completion-classification-package-gates.log`. This includes 5 domain classification,
  30 identity, 6 audit, 2 annotation-inertness, 20 projector, 4 cache admission/accounting,
  14 publication classification, 14 semantic diff, 3 production publication and 2 carrier compilation
  tests. It is a package gate, not the full workspace gate.
- The latest combined PostgreSQL run passed 25 integration and 4 fault tests
  (`/tmp/sagat-completion-pinning-races-gates-2.log`). It includes both approved/promotion pinning
  paths and API/telemetry canaries. The races reconstruct services; they do not prove process crashes.
- Studio: 85 files / 1,149 tests passed after updating the stale digest-only journal copy assertion;
  full ESLint and production build passed. Logs: `/tmp/sagat-completion-studio-tests-full-2.log`,
  `/tmp/sagat-completion-studio-lint-full.log`, `/tmp/sagat-completion-studio-build.log`.
- Generated API/CLI references passed all 8 `contract_developer_experience` tests with
  `SAGAT_DOCS_BLESS=1` (`/tmp/sagat-completion-reference-gates.log`). The generator now retains
  approved operation descriptions for extensions, including the three event-read breaks.
- `SAGAT_PERF_REPORT=tests/performance/reports/payload-classification.md cargo test -p sagat-tests
  --test perf_payload_classification -- --ignored --nocapture` passed 1/1, 720 measured projections,
  zero failures (`/tmp/sagat-completion-classification-performance-registered.log`). The checked-in
  report records actual percentiles, encoded sizes and heap snapshots. The 16 MiB cache policy is
  serialized schema weight; measured retained compiled heap was 414.24 MiB, not RSS.

### Actual child result carrier correction

`cargo test -p sagat-tests --test integration_payload_classification step_outputs::child
-- --test-threads=1` failed on the actual G21 `child.finished` result: the retained payload was
ineligible despite a parent child-node schema (`/tmp/sagat-completion-child-carrier-red-2.log`).
The resolver now accepts that event only for the exact pinned parent `child` node and persisted
scope. No journal event is duplicated and no database carrier becomes eligible.

`cargo test -p sagat-tests --test integration_payload_classification step_outputs::
-- --test-threads=1` then passed all 6 tests in 27.15 s
(`/tmp/sagat-completion-step-outputs-green.log`), including actual activity, wait, reconciliation,
built Docker script, own-pinned child input/result, parent completion, rerun/retry attempts,
no-schema/forged blob JSON and catalog-backed external output. Carrier unit tests passed 3/3
(`/tmp/sagat-completion-child-carrier-unit.log`).

Commands above used the coordinated warmed target, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=3`
and `RUST_MIN_STACK=16777216`. Reused worktrees can leave incompatible local-crate metadata in
that target; refreshing the current workspace source mtimes forces its actual interfaces to
recompile. Earlier stale-metadata compilation failures are not counted as passing tests.


### Real process death and lost-response audit evidence

The new `tests/fault/payload-classification/process.rs` launches the actual control-plane binary.
A PostgreSQL trigger waits on an advisory lock before inserting the mandatory reveal audit. The
test observes that database wait, kills the OS process and verifies no response body and no audit
commit. A restarted process successfully reveals with one committed audit. A TCP proxy then holds
all response bytes after the next upstream response/committed audit; killing the server and cutting
the client connection preserves that second audit despite response loss. A further process restart
and retry returns the authorized value and creates a third audit. Process logs contain no PII canary.

The actual case passed 1/1 in 4.24 s as the first target in
`/tmp/sagat-completion-process-compat-3.log`. The command selected both ignored process/compatibility
cases; the later compatibility case failed independently, so the whole command exited nonzero.
Retained process logs: `/Users/marcelo/Documents/Projects/sagat-octopus-process-fault-artifacts/run.retest`.
`scripts/test-payload-classification-process-faults.sh` is the reproducible standalone command; it
builds the binary, records a manifest and explicitly selects the ignored process case. The first
harness attempt used an incorrect readiness path and is a failed prerequisite, not production RED.
The corrected case uses `/v1/ready` and initializes the HTTP client's pinned crypto provider.

T016-050 remains open for concurrent publication/make-active evidence and its final combined gate.


### Registered core gate and completed walker coverage

`cargo test --locked -p sagat-tests --test integration_payload_classification
--test fault_payload_classification --test contract_payload_classification
--test e2e_payload_classification -- --test-threads=1` passed all 56 executed tests:
15 contract, 4 e2e (including actual CLI subprocess), 4 transaction fault and 33 PostgreSQL/HTTP
integration tests. Log: `/tmp/sagat-completion-classification-registered-core.log`. Two separately
gated process/compatibility cases were intentionally ignored by this default command; process death
already passed explicitly, and prior-release compatibility remains pending. Integration runtime was
303.94 s on the loaded shared machine; this is correctness evidence, not a latency measurement.

The canary now seeds 22 distinct key/value classes, including dynamic maps under all four declared
levels. Undeclared keys under public/internal maps still require PII reveal. Both run and event
journeys capture actual logs, OTLP spans, Prometheus labels and audit records. The helper checks raw
bytes and base64-encoded JSON fields (at most two nested encodings); its own two detector tests prove
an encoded letter body cannot be mistaken for absence. It is not a general decoder for arbitrary
encodings. Full unchanged-surface golden replay and the broader browser matrix remain T016-049 gaps.

`cargo test --locked -p sagat-infra-schema --test classification_properties` passed 3/3 in 0.08 s
(`/tmp/sagat-completion-walker-properties.log`): all sound corpus shapes with runtime effective levels,
the exact 1000-admitted/1001-refused expansion boundary, and generated nested ancestor policies.
The existing locked proptest dependency is reused only as a schema-package dev dependency; no
production dependency or schema acceptance policy changed. The previous 20 classifier tests remain
covered by the package gate.


### Application refusal and Debug completion

`cargo test --locked -p sagat-application --test payload_disclosure` passed 10/10
(`/tmp/sagat-completion-disclosure-ports.log`). Added cases prove forbidden roles perform no store
lookup, missing and foreign runs are the same typed NotFound, every closed projector failure
returns no values and no success audit, and retained definition/payload Debug output exposes only
sizes rather than schema fragments, data keys or values. Namespace admission is tested through its
actual HTTP owner in the registered integration target, rather than inventing an application port.
