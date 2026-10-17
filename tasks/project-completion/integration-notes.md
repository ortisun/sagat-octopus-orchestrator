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


### Concurrent publication and active-version changes

`cargo test --locked -p sagat-tests --test fault_payload_classification
races::concurrent_publication -- --test-threads=1` passed 1/1 in 5.27 s
(`/tmp/sagat-completion-publication-race-2.log`). The classifier pauses only after PostgreSQL has
selected the pinned run/version. A new version is published and the active version is changed
while that read remains paused. The original request still returns version-1 PII under its grant;
a subsequent run uses version 2 and withholds its secret root. Exactly one audit accompanies the
original successful reveal. The first harness attempt blocked a synchronous channel on a Tokio
I/O worker and timed out; moving that barrier to spawn_blocking removed executor starvation.
This is a test-harness correction, not a production transaction change.

Together with the 4 existing transaction races/failures and the actual process death/lost-response
case, this completes T016-050's named fault scenarios. The final integrated workspace/release gates
remain separate tasks; an earlier combined command stopped at the first harness failure and did
not execute its following migration target. That migration regression was then run explicitly and
passed, as recorded in the compatibility report and feature 014 validation report.


### Event port completion and actual release process gate

`cargo test --locked -p sagat-application --test event_payloads` passed 6/6
(`/tmp/sagat-completion-event-ports.log`). The fake port now records the mandatory audit before
returning a revealed event/letter and can fail that write. Both failures return no view, successful
reads record one audit each with the caller identity, and Debug omits body/header/data canaries.
Unsupported list projection becomes opaque and clears retained raw fields. All three machine roles
are refused before any scope lookup, including the approved DatabaseRunner dependency.

The complete artifact-building `scripts/test-payload-classification-compat.sh` passed its actual
prior-release process case 1/1 in 49.88 s (`/tmp/sagat-completion-compat-release-5.log`). Manifest:
`/Users/marcelo/Documents/Projects/sagat-octopus-compat016-artifacts/run.PJb73x/artifacts/SHA256SUMS`.
It uses the annotated v0.1.0 commit 6ede1c7849c4ac5a209c08da8ad25294fbdd7795 and current binary
2108d08cd117f26847b605c92c8abeb95a3dc5ea. The populated migration, immutable old bytes/digest/run pin,
legacy opacity, new rules-1 publication and documented three event-read breaks now pass. The old
execution engine reaches readiness before/after the upgrade; this is not proof of all old-server
writes on new scoped workflows. Old Studio was served over HTTP in that passing case. Actual
Chromium behavior and released five-client conformance are being added and keep T016-052 open.


### Released five-client and Chromium compatibility completion

The five SDK runners built from annotated release v0.1.0 (6ede1c7849c4ac5a209c08da8ad25294fbdd7795)
passed the reviewed current root `run-lifecycle` scenario against the real current platform: one
aggregate / all five languages, 145.91 s (`/tmp/sagat-completion-old-sdks.log`). No golden was blessed.
The harness now accepts only an absolute optional `SAGAT_SDK_SOURCE_ROOT`; scenarios, normalization
and expected results still come from current source. An archived 361-file SDK/contract input
manifest records the release bytes. `scripts/test-released-sdk-compat.sh` reproduces that archive
and invocation; syntax and ShellCheck passed, while the actual measured run used the equivalent
archive directly.

The expanded actual-process compatibility case passed 1/1 in 17.48 s with real Chromium executing
the old Studio bundle (`/tmp/sagat-completion-compat-browser-2.log`). It authenticates, reads the run
and graph and renders its work row in the node table; browser errors and payload canaries are absent.
The driver receives its token over stdin and reports only closed stages on failure. An initial UUID
serialization compile error was fixed before that run. T016-052's named cases now have process,
client and browser evidence. This is not the full feature-014 server writer/rollback or complete
current SDK scenario matrix. The compatibility report records the exact component limits.


### Released engine actual run read

The strengthened compatibility case passed 1/1 in 24.96 s
(`/tmp/sagat-completion-compat-engine-read-2.log`). After migration it calls the old engine over
real gRPC with an intentionally unleased heartbeat. That immutable release implementation opens
and restores the persisted run before validating its lease; the expected Aborted/STALE_ATTEMPT
answer proves successful restoration without an old-server mutation. The first probe put a node
name in the optional UUID step_id field and received InvalidArgument; using the supported legacy
empty step_id form fixed the harness. This strengthens the earlier readiness-only evidence while
retaining the explicit limit on arbitrary old-server writes into new scoped workflows.


### Grant REST and tenant security completion

Two new live grant contract tests passed (13.05 s) in
`/tmp/sagat-completion-grant-contract-3.log`. Six non-manager roles cannot manage grants;
Admin can self-grant, repeated PUT preserves the response, pagination and repeated DELETE
follow the contract. Unknown/ineligible subjects return closed 422 reasons. At 256 entries the
service correctly returns the documented **429 QUOTA_EXCEEDED / GRANT_LIMIT**. The initial test
expected 422 and was corrected to the existing contract, not a production behavior change. Actual
PostgreSQL catalog checks prove runtime SELECT/INSERT/DELETE without UPDATE and tenant-composite
namespace/membership foreign keys with membership cascade. The application and real-PG grant
suites already cover transaction rollback, concurrent cap enforcement and current authority.

The live tenant-boundary test passed 1/1 in 4.84 s in
`/tmp/sagat-completion-security-http.log`. A positive namespace-A reveal witnesses the grant;
namespace-B run/event/letter reveals remain forbidden. Foreign and absent run/journal, namespaces,
events, connectors, letters and grants remain opaque. A foreign-only subject is indistinguishable
from an unknown one; invalid selectors/reveal values are not reflected. The foreign namespace's
actual quota bucket tuples stay byte-for-byte equivalent, including timestamps. A real restricted
cross-tenant audit exists in storage but is omitted from the tenant's audit API.

The original registered event gates passed: contract 2/2 (18.82 s), e2e 11/11 (93.65 s), integration
111/111 (357.46 s), no ignored tests, in `/tmp/sagat-completion-original-event-suites.log`:
`cargo test --locked -p sagat-tests --test contract_rest_events_webhooks_messaging
--test integration_events_webhooks_messaging --test e2e_events_webhooks_messaging -- --test-threads=1`.
The source reader inventory used `rg -n 'getInboxEvent|listConnectorDeadLetters|getConnectorDeadLetter|
dead-letters|InboxEvent|DeadLetter' frontend/apps/studio/src apps/cli/src sdks`, excluding generated
`schema.d.ts`, target/bin/obj/node_modules/dist/.venv/.mypy_cache/build artifacts. It found no
handwritten product reader of these event values. Canonical generated types retain the new shapes.


### Live event REST fault and full dead-letter page

`events::actual_event_and_letter_reads_validate_reveal_refusals_and_revocation` passed 1/1
in 8.51 s (`/tmp/sagat-completion-event-contract-live-2.log`). It validates every actual
response against canonical OpenAPI: default structure, explicit reveal without/with grant, invalid
reveal, an injected real PostgreSQL audit-insert failure, and the next call after grant revocation.
Two successful reveals create exactly two durable reveal audits; neither 503 nor refusal adds one.
The original RED (`/tmp/sagat-completion-event-contract-live.log`) showed that the server's closed
422 rejection was absent from both detail operations in OpenAPI. Those two response declarations
and generated Studio types are fixed. The reference generator passed all 8 checks in
`/tmp/sagat-completion-event-reference-gate.log`; no server behavior was relaxed.

The same test seeds exactly 200 retained letters. A non-JSON body and a truncated but valid JSON
body independently return opaque structure. The entire 200-row page validates as DeadLetterPage,
contains no body/header fields or canary, and creates no reveal audit even with an active Admin
grant and reveal=pii in the list URL. Earlier PG event/dead-letter/canary tests plus the new live
security test cover current roles, all audience combinations, cross-namespace and foreign opacity.

The complete classification contract target then passed 18/18, 25.27 s
(`/tmp/sagat-completion-contract-complete.log`). Studio `check:api` confirms exact generated types
(`/tmp/sagat-completion-event-api-check.log`). The fixture corpus now distinguishes explicit-parent
loosening from an unmarked-root child and lists all 22 unique runtime canary classes.
`cargo test --locked -p sagat-infra-ir --test classification` passed 14/14 after this update
(`/tmp/sagat-completion-fixture-corpus.log`). These results close the named corpus and event cases;
full integrated release/workspace gates remain open.


### Bounded path mutation foundation integration

Merged the committed bounded-execution work through 4f70bd08 into the integration tree. The
shared step lifecycle reducer is used by both full replay and the exact-path reducer. The new
PostgreSQL transaction rechecks the run revision, exact instance/lease/session identity, current
worker authority, quotas and script retention and persists the journal, output accounting, next
task hints, receipts and audit atomically. It remains a foundation: production WorkerService still
uses the complete RunTx path; bounded failure/control/blob/reconciliation and compensation
authority must be completed and verified before switching that service.

Actual local gates on this integration:
- `cargo test --locked -p sagat-domain`: 501 passed across 37 targets including doc tests; 0 failed or
  ignored (`/tmp/sagat-completion-bounded-domain.log`). This includes full-state lifecycle/property
  tests and the five bounded/full-replay differential/planning cases.
- `cargo test --locked -p sagat-tests --test integration_workflow_primitives byte_ceilings::
  -- --test-threads=1`: 13 passed, 117.27 s, no ignored cases
  (`/tmp/sagat-completion-bounded-byte-gate.log`). Actual PostgreSQL verifies bounded lease and
  completion persistence, retention admission and all logical/value/aggregate/task-input ceilings.
- `python3 scripts/check-architecture.py`: passed, 24 crates, no forbidden layers or cycles
  (`/tmp/sagat-completion-architecture-latest.log`).

A follow-up regression is being added for snapshot reads queuing behind the run writer and for
indexed compensation authority. Neither the foundation nor these focused gates close the remaining
feature-014 runtime, large-workload or release tasks.


### Exact-path snapshot and compensation authority correction

The writer-lock regression failed on the previous implementation in
`/tmp/sagat-completion-bounded-snapshot-red.log` (the held writer caused the one-second timeout).
Path reads now use one REPEATABLE READ READ ONLY transaction without a run-row lock. Mutation
still compares the snapshot revision under its existing exclusive run lock. A typed bounded saga
projection reads only ancestor/local headers and the exact iteration's indexed entries; it never
imports sibling compensation authority. Derived leases require that persisted compensation or
reconciliation authority. The other worktree's uncommitted source was read and applied as a patch
to this integration checkout; its contents and worktree remain untouched.

Actual focused domain gate: 5 bounded planning/lifecycle tests plus 26 scoped-compensation tests
passed in `/tmp/sagat-completion-bounded-authority-domain.log`. Actual PostgreSQL gate:
`cargo test --locked -p sagat-tests --test integration_workflow_primitives
byte_ceilings::a_bounded -- --test-threads=1` passed 4/4, 12.55 s, no ignored cases
(`/tmp/sagat-completion-bounded-snapshot-green-2.log`). These cover the nonblocking snapshot,
per-iteration saga selection, existing bounded completion and script retention. The first saga
fixture had admitted only iteration 0, so correctly refused iteration 1; an additional scheduler
admission and an explicit two-member database assertion fix that harness prerequisite.

This does not enable a partially implemented bounded worker. Failure/reconciliation reducers,
control progression, blobs, root finalization and the accepted large-workload evidence remain
required before that production switch.


Focused Clippy passed with warnings denied for `integration_workflow_primitives`,
`contract_payload_classification` and `integration_payload_classification`
(`/tmp/sagat-completion-focused-clippy-3.log`, 2.79 s final incremental check). Two unnecessary
explicit dereferences at the real audit insert boundary were removed. Two test helpers now mark
their intentional test panics/expect calls consistently with the other fixtures; production lints
remain unchanged. This focused check is not the full workspace/all-targets T016-054 gate.


### Expanded Chromium disclosure lifetime gate

The expanded real-browser journey passed 1/1 (2.6 s browser / 6.24 s actual API/PG harness) in
`/tmp/sagat-completion-disclosure-browser-expanded-2.log`. It repeats explicit confirmed reveals
and verifies clearing on panel close, navigation, run change, session replacement and tenant
replacement. It reads actual IndexedDB records as well as local/session storage and checks every
captured completed JSON API response for the secret canary. The original role matrix (viewer,
developer, operator, granted developer) remains asserted. The new foreign-tenant transition first
encountered an expected workflow-list 404 because that fixture had no namespace; the fixture now
creates its own namespace via the foreign administrator API, preserving the no-browser-errors gate.
The source compiles with `npm run typecheck --workspace @sagat/e2e`.

Command: `SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_REBUILD=0
SAGAT_STUDIO_GREP='viewer, developer, operator and revealer' cargo test --locked
-p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture`. As before, the
unrelated sandbox journey is explicitly excluded; this is not full Studio acceptance.


### Actual pre-classification byte compatibility

`scripts/test-payload-classification-surfaces.sh` passed its complete build/migrate/HTTP gate
(1 test, 5.96 s runtime; `/tmp/sagat-completion-surface-compat-full-3.log`). The immutable baseline
is 3637ee8c4b98d2e5809f1527a9b2bb1fcaf2406a: the approved integrated 014/017 platform immediately
before feature 016, rather than a release without Query/Update. The actual baseline publishes and
starts terminal and stateful runs and applies a declared Update. Its responses are retained,
then the actual current server migrates the populated database and repeats the exact requests.
No response is normalized and no golden is blessed. Actual bodies are byte-identical for all ten
surfaces: run, journal, graph, graph changes, completed SSE stream, telemetry, attempts, structural
inspection, declared Query, and a replayed declared Update. The latter two positively return the
canary under their explicit declared-result policy; classification does not filter those results.
Telemetry uses a legitimate caught-up persisted projection (lag zero), not a substituted clock.

Manifest/binaries/response pairs: `/Users/marcelo/Documents/Projects/sagat-octopus-surface016-artifacts/run.U6mTUY/artifacts/SHA256SUMS`.
The first complete script run reached the real old server but refused an unbounded-string test
fixture, as the existing contract requires. Adding maxLength=256 to that fixture corrected the
harness. A direct retry passed 1/1 in 5.68 s before the full script was rerun successfully.
The script keeps its detached source read-only and removes only that clean source on exit;
artifacts and logs remain. This extra gate complements the separate actual v0.1.0 release upgrade,
released-five-SDK and old Studio/browser evidence; it is not an arbitrary old-server writer claim.


### Expanded six-role real-API canary matrix

`canary::six_roles_with_and_without_grant_keep_every_public_surface_value_free` passed 1/1
in 18.71 s (`/tmp/sagat-completion-e2e-canary-expanded-3.log`). It now seeds four actual runs
(classified, schema-invalid, rules-0 legacy, unmarked), probes all six roles with/without a stored
grant and an explicit reveal, and checks all four runs over the metadata reads, structural
inspection and SSE. These are 768 REST matrix requests and 96 stream requests, plus setup/audit.
The unmarked canary positively appears only on an authorized explicit reveal; invalid and legacy
values/keys never appear. Actual exported spans, structured logs and rendered metrics are captured
and checked for all 22 classes, with a positive disclosure-span/metric witness.

The fixture retains real rate admission: the platform quota service grants a 10,000 ceiling and
the tenant sets that supported policy over HTTP. An initial direct tenant increase was correctly
refused above its default platform ceiling. The synthetic legacy fixture temporarily changes only
the classification pin in its isolated database, inside one transaction; deferred checks are
flushed before restoring the immutable trigger and committing. The first fixture attempt found
PostgreSQL's pending-trigger DDL guard; this fixture correction changes no production migration.
The separate old-release binary upgrade remains the evidence for natural rules-0 creation.

The CLI/browser/event matrices and the frozen prior-server byte comparison remain separate named
evidence. T016-049 is kept open while completing its last boundary combinations.


### CLI audience matrix and T016-049 completion

The complete E2E classification target passed 5/5 in 190.26 s, no ignored tests
(`/tmp/sagat-completion-e2e-classification-expanded.log`). It includes the four-run six-role HTTP
matrix, the two canary-detector checks, the existing actual CLI grant/reveal/revoke journey and
96 actual CLI executions over all role/grant/reveal/payload combinations. The latter inspects
both stdout and stderr, checks exit statuses and positively witnesses authorized values. After
adding explicit assertions for every dynamic/undeclared key and value, that 96-process case
passed again (1/1, 120.86 s, `/tmp/sagat-completion-cli-keys-matrix.log`). Shared fixture helpers
retain the same real publication and isolated legacy preparation.

Together with the existing actual event/dead-letter audience matrix, captured server telemetry,
expanded Chromium lifetime/storage/network proof, and all ten byte-identical frozen pre-feature
HTTP responses, this closes the named T016-049 boundary criteria. It does not close the workspace
release gate or the manual/CLI quickstart task.


### Executed classification quickstart and guide reconciliation

Added complete `specs/016-payload-classification/examples/` workflows (original/relaxed/invalid),
synthetic input and a canned mock worker response. Actual standalone CLI validation passed both
sound documents and refused the invalid one at the expected CLASSIFICATION_LOOSENED pointer.
The new `quickstart::the_documented_classified_orders_complete_and_keep_their_original_pin`
passed 1/1 in 21.00 s (`/tmp/sagat-completion-classification-quickstart-3.log`). It runs the actual
`sagat` and `curl` processes against HTTP/gRPC/PostgreSQL and the production engine entry point:
publish, worker-dev binding/execution, terminal result, actual step sequence, public/internal/PII
reads, grant/revoke and refusal, republish/activate and unchanged old run pin. It also publishes an
actual event, executes the four documented event/letter reads and checks the audit API for data
absence. The retained paused-connector letter is explicitly an isolated fixture; no Kafka delivery
is claimed by this walkthrough. The first compile needed named arrays for async CLI arguments;
this was a harness lifetime correction before execution.

The guide now points at real files, uses valid Cargo filter invocations, includes the worker/run
commands and version activation and explains receiving-carrier secret declarations. Its spec
prerequisite command passed (`/tmp/sagat-completion-classification-spec-prerequisites.log`); the
PayloadDisclosure/WorkflowSettings tests passed 8/8 in 1.96 s
(`/tmp/sagat-completion-classification-guide-ui.log`). Developer-experience references/links/CLI
syntax/secret scan passed 8/8 (`/tmp/sagat-completion-classification-guide-references.log`); strict
Clippy of the three classification targets passed with warnings denied in 27.81 s
(`/tmp/sagat-completion-classification-final-focused-clippy.log`).

The benchmark, transaction/process faults, actual old release/old Studio/five-client compatibility
and Chromium commands in the guide have their separate executed evidence recorded above; those
are reused where the tested production path is unchanged, not invented reruns. The requirements
checklist now distinguishes executed local evidence from the still-open full workspace/release
gates. T016-056 closes for the delivered guide and traceability; T016-054 remains open.


### Feature-014 source scale/CI integration

The local source branch advanced to 8ade8f18 and records CI on its own 7d7868ff profile. Those
records are retained as supporting source-branch evidence, not reported as execution on the
combined tree. Integration preserves worker protocol 1.6, classification's generated error
context, managed run origin, all database error variants and the correct Python chain-page
iterator. The new open-step indexes are allocated at 20261206000000 to avoid OpenBao's existing
20261122000000 identity; the source reference worktree and every existing migration remain intact.
The feature-014 completion ticks imported from the other profile are reopened until the full
accepted criteria and combined gates pass. No partially supported shape is substituted for those
criteria. Bounded scope frontiers complement the exact-path worker foundation; neither alone
closes the remaining general runtime and resource proof.


Combined focused verification on the resolved merge:

- `integration_workflow_primitives -- scope_frontier:: compatibility::
  byte_ceilings::a_bounded identity_storage:: journal_identity:: --test-threads=1`:
  **27/27**, no ignored tests, 62.42 s
  (`/tmp/sagat-completion-merged-frontier-compat-3.log`). An earlier round was
  20 passed / 1 failed: the legacy fixture called the current publisher after undoing
  classification's column. Preparing the ordinary definition before undo and inserting only
  the legacy run/journal afterward restores the intended upgrade boundary; all three users
  of that fixture now use this preparation. This is synthetic schema compatibility, distinct
  from the separately executed frozen release binary gate.
- Strict whole-workspace/all-target Clippy passed in 22.50 s
  (`/tmp/sagat-completion-merged-workspace-clippy-3.log`). Earlier rounds found intentional
  fixture assertions without local test-only lint annotations; no production lint was disabled.
  Formatting passed again after the fixture change.
- Architecture: 24 crates; script unit tests: 8/8; generated Studio API matches canonical
  OpenAPI. Protobuf compilation and IR example validation passed. OpenAPI lint passed with
  seven existing documentation warnings, not a warning-free claim.
- Changed Studio autosave target: 33/33 (5.21 s); Python mypy: 38 source files; C# SDK:
  450/450, zero skipped (22 s), plus format verification. The first mypy command could not
  locate a local venv; the retry used the existing interpreter with this tree's `src`.
- Dependency audits: Cargo checked 627 dependencies against 1,338 advisories and exited zero;
  npm reported zero vulnerabilities.
- Byte comparison against pre-merge HEAD: all 124 existing migration files are unchanged.

The added schema-diagnostic canary regression passed on the incoming implementation:
`IrError::Schema` already renders only the number of issues. No schema log leak or failing
red test is claimed (`/tmp/sagat-completion-reader-diagnostics-red.log` is a misleading
historical filename). This finding does not prove every other parser diagnostic is safe.
The default whole-run worker and the remaining large/nested runtime criteria stay open.
