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


### Current-instance charge projection

The incoming frontier still used a full historical `count(*)` over steps for every scope read.
Migration `20261207000000` now materializes the exact `RunState::charged_instances` predicate in
the existing tenant/run usage row. The always-enabled trigger updates it with the step transaction;
unchanged contributions do no write. This is separate from lifetime admission/reservation counts:
a root cancellation changes the current predicate but releases no admission/reservation budget.
The initial backfill explicitly pauses writers, with table locks ordered run/step/usage. It does
not rewrite steps, workflow definitions or journal history; down removes derived metadata only.

Actual tests:

- The new cancellation/rollback case initially failed because the new column did not exist:
  `/tmp/sagat-completion-charge-projection-red.log` (0 passed / 1 failed).
- `integration_workflow_primitives -- scope_frontier:: byte_ceilings:: --test-threads=1`:
  **25/25**, 78.33 s (`/tmp/sagat-completion-charge-projection-green.log`). Includes the
  new runtime-role transactional cancellation test and the existing full-replay differentials
  across collection/loop success, failure, cancellation and delays, plus byte ceilings.
- Added nonempty backfill, rollback then retry of schema creation, and byte-identical retained
  step assertions. The five focused migration/identity/counter cases passed with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`: **5/5**, 6.71 s
  (`/tmp/sagat-completion-charge-upgrade-runtime.log`).

The broader pre-counter merge regression also completed: database connections **3/3**, 33.89 s.
Classification initially returned 20 passed / 15 failed / 2 ignored: a temporary PostgreSQL failed
its 60-second readiness check, followed by 14 poisoned harness-lock failures. That failed log is
retained (`/tmp/sagat-completion-merged-classification-database.log`). Repeating the whole target
against an exclusively owned, query-verified PostgreSQL with fresh databases, now also including
the new counter migration, passed **35/35**, 59.25 s, with the two explicitly external binary
compatibility gates ignored (`/tmp/sagat-completion-merged-classification-retry.log`). Those binary
gates retain their separately executed earlier evidence; this retry does not claim to execute them.
AsyncAPI validation also passed (one informational schema-version suggestion, no errors/warnings).
The full bounded worker, nested runtime and 10,000/100,000 workloads remain open.


Strict whole-workspace/all-target Clippy passed again with the counter migration and both new
cases (93 s, `/tmp/sagat-completion-charge-clippy.log`). The existing 160/320/640 transition-cost
checks are executing separately; their result will be recorded when complete, not presumed here.
The clean detached release source `run.1mfG99/source-retry` was removed after verifying its
immutable release HEAD and empty status. Its read-only directories required finishing Git's
otherwise successful unregister/removal; the actual binaries/manifests/responses remain intact.


The instance-count transition checks completed after the counter commit: **2/2**, no ignored,
434.37 s (`/tmp/sagat-completion-charge-flat-cost.log`). Collections at 160/320/640 items:
median 44/44/44 rows and 247/241/266 buffers. Loops: 67/67/67 rows and 370/389/391 buffers.
The environment, exact command, phase exclusions and unproven larger workloads are recorded in
`tests/performance/reports/workflow-primitives.md`. These checks measure scheduler checkpoints,
not the still-unbounded worker/root finalizer, and do not close T014-042.


### Shared bounded effects, commands and global status

New differential coverage first failed on `UnsupportedScopedEvent("reconciliation.requested")`
(`/tmp/sagat-completion-bounded-effects-red.log`). Extracted the existing full-run effect/review
reducer into a shared implementation over addressed instance access. The first differential retry
then exposed a real status mismatch: the bounded path retained RUNNING after the full oracle
entered MANUAL_REVIEW. The new checked RunWorkSummary folds local facts with a bounded summary
of other paths, using the same status precedence as full replay. Snapshot reads are repeatable-read;
the existing mutation CAS still rejects outside progress. The additive 20261208000000 indexes
cover live/review/root compensation probes without changing prior migrations.

Scoped failure, lookup, retry and operator decisions now share one command policy across both
representations. An initial API fixture could not compile before fail_path_instance existed
(`/tmp/sagat-completion-bounded-failure-command-red.log`); this was an API red, not a runtime failure.
The compensation decision baseline passed before any extra planner change, so no speculative
planner fix was added (`/tmp/sagat-completion-bounded-compensation-decisions-baseline.log`).

Actual verification:

- Complete domain target after reducer/status extraction: **504 passed**, 37 test targets,
  zero failed/ignored (`/tmp/sagat-completion-domain-effects-regression.log`). Later new command
  cases are covered separately by the next focused result; no final whole-workspace claim here.
- Final focused differentials: **11 bounded-path + 27 scoped-compensation = 38 passed**,
  zero ignored (`/tmp/sagat-completion-bounded-command-differentials.log`). Cases include
  known failure, retry, cancellation acknowledgment, unknown effect, lookup verdicts, review
  decisions, stale fences, other-path review/live status and failed-compensation decisions.
- Real PostgreSQL/runtime role: **7/7**, 10.30 s
  (`/tmp/sagat-completion-bounded-effects-pg-commands.log`). The new case runs both UNKNOWN and
  APPLIED lookup verdicts through the new commands and exact-path writer, with real ordinary
  forward/lookup claims, reservation/hints, reported token, retained lookup/forward outputs,
  reloaded global status and full journal/projection verification.
- Focused strict Clippy passed, 40.15 s (`/tmp/sagat-completion-bounded-effects-clippy.log`);
  architecture passed, 24 crates. A final expanded static/workspace run follows this checkpoint.

The worker service is deliberately still on the complete RunTx consumer. These port/command
checks do not prove that the not-yet-wired provider, blob, control, full drain and root finalizer
paths are bounded. They do not close the canonical remaining feature-014 acceptance tasks.


Final strict Clippy of the complete workspace/all targets passed in 1m57s
(`/tmp/sagat-completion-bounded-effects-workspace-clippy.log`); formatting and the 24-crate
architecture check passed. These are the static gates for this checkpoint. The next full
workspace execution will use a fresh task-owned target directory with debug symbols disabled,
keeping assertions and the test profile enabled, to limit additional disk usage.


### Conformance fixture phase correction

The complete five-SDK suite reached classification, then failed after 433.28 s because its
synthetic script digest was correctly refused by publication (422 UNKNOWN_SCRIPT)
(`/tmp/sagat-completion-current-sdk-conformance.log`). Classification's task/golden are authoring
only: T016-042 requires identical canonical documents, and only classification.workflows.json
exists. Added the missing buildOnly marker without changing its reviewed canonical output or
relaxing publication. A fast test now checks all 55 scenario registrations for the expected
canonical document and, for runtime scenarios, their separate reviewed runtime golden.
The focused classification run across all five SDKs and both harness tests passed **3/3**, 1.64 s
(`/tmp/sagat-completion-sdk-classification-phase.log`). This is builder parity; the whole SDK
runtime gate is rerun separately. The failed complete run is retained, not claimed successful.

The complete Studio gate passed on this checkpoint: lint/types, **1149/1149** unit tests,
production build and generated API comparison (`/tmp/sagat-completion-final-studio-*.log`).


### Immutable definition restoration and integration fixtures

A new frozen-contract regression genuinely failed before the fix: an `inputSchema` extension
`x-sagat-classification: "pre-016-note"`, accepted by the v0.1.0 schema, was reinterpreted by the
new authoring parser during a cold execution-cache restore (ClassificationInvalidLevel).
The failed test is retained in `/tmp/sagat-completion-legacy-annotations-red.log`.
The frozen schema is copied byte-for-byte from 6ede1c7849c4ac5a209c08da8ad25294fbdd7795,
SHA256 5035e781a2a531da0317594cd8b082fb9291d8e3f0a7b0322625c2260d89bff4.
This proves old contract acceptance, not by itself an old binary execution.

IrCodec now separates publication from immutable restoration, sharing executable conversion and
syntax/graph/expression/capability checks. Execution cache and declared-operation snapshots restore
stored definitions without retroactively applying classification/carrier-schema authoring checks.
New publications remain strict; stored bytes and disclosure pins remain unchanged. Focused IR
checks passed **81/81** across six targets (`/tmp/sagat-completion-legacy-annotations-green-2.log`),
then cache/continuation/declared-state/operation evaluator passed **31/31**
(`/tmp/sagat-completion-legacy-state-green-2.log`). The real prior-release process fixture now also
publishes the older extension; its expanded binary gate is executed separately.

The fresh workspace checkpoint exposed duplicated primitive CLI help entries and duplicated docs;
removing the first duplicate block restores exact code/help/doc correspondence. Diagnostics passed
**7/7** (`/tmp/sagat-completion-diagnostics-deduplicated.log`). Two integration fixtures also needed
the combined profile: an offered protocol 1.999 negotiates the current 1.6 ceiling, and the new
cross-tenant disclosure POST needs its valid JSON body before testing opaque resource resolution.
Their actual PostgreSQL rerun is recorded when complete. The broad checkpoint is retained as
failed evidence and is not described as a final current-tree pass.


### Regression repair after the fresh workspace checkpoint

The complete gRPC/security fixture rerun passed **70 + 17 = 87 tests**, no ignored, with two test
threads and fresh databases on the owned PostgreSQL (92.90 s and 21.35 s;
`/tmp/sagat-completion-integration-fixtures-green-2.log`). Both the negotiated response and persisted
minor now assert 1.6. The earlier first rerun missed the persisted minor assertion and failed;
it also observed an HTTP/2 BrokenPipe in the cumulative byte-credit case. That case passed isolated
(7.54 s, `/tmp/sagat-completion-stream-credit-isolated.log`) and in the complete 70-case retry.
These failures remain retained; no retry is relabeled as an initial success.

The missing pinned .NET images were built from `runners/dotnet/Dockerfile.{builder,runtime}`
(`/tmp/sagat-completion-dotnet-{builder,runtime}-image.log`). Replaying the actual failed C# E2E
binary case passed **1/1**, 9.97 s (`/tmp/sagat-completion-dotnet-images-e2e.log`). This establishes
the image prerequisite and that scenario, not a final repeat of the entire workspace.

The five-SDK retry reached 29 scenarios then aborted at database-connections because the harness
looked only in this checkout's `target/debug`, ignoring the coordinated target where the runner
had been built (`/tmp/sagat-completion-current-sdk-conformance-2.log`, 663.39 s). Before that abort,
all five languages independently reported the same missing `step.routed` compensation events.
A new domain test genuinely failed on that omission
(`/tmp/sagat-completion-compensation-routing-red.log`). Global pause/stop fencing now suppresses
forward routing/release while retaining empty routing of completed derived work. The bounded and
full planners share this behavior. **39/39** focused planning/compensation tests passed
(`/tmp/sagat-completion-compensation-routing-green.log`); the reviewed SDK golden is unchanged.
The runner harnesses now locate a sibling binary of the actual test executable, retaining the
explicit override. SDK runtime repeats are recorded separately when complete.


The strengthened immutable-extension binary gate passed **1/1**, no ignored, 6.23 s
(`/tmp/sagat-completion-legacy-restore-process-green.log`), using copied current control plane
b05e3d59 and verified retained v0.1.0 binaries/Studio. Real current-process pause/resume mutations
and persisted projections now exercise cold restore, alongside old Studio rendering, unchanged
bytes/digest/version pins and rules-0 opacity. The same fixture against retained pre-fix current
2108d08c genuinely failed at old Studio graph rendering (500 after invalid annotation at restore),
21.30 s (`/tmp/sagat-completion-legacy-restore-process-red.log`). Full provenance, manifest and
scope are in `tests/integration/payload-classification/compatibility-report.md`. The frozen schema
checksum is now asserted too. This is actual process evidence; it does not claim the later routing
fix or the whole combined release was exercised by that copied control plane.


### Current static gates and repaired five-language scenarios

Final restore-cache/declared-state/operation checks passed **16/16**, no ignored
(`/tmp/sagat-completion-legacy-restore-final-unit-2.log`). The first extra checksum assertion used
LowerHex on sha2 0.11's array type and did not compile; it now uses the existing domain Digest's
hex encoding. Both failed compiler logs are retained. Current whole-workspace/all-target strict
Clippy then passed in **23.62 s**
(`/tmp/sagat-completion-restoration-routing-workspace-clippy-2.log`), formatting/diff and the
24-crate architecture check passed, cargo audit passed its checked-in policy (627 dependencies /
1338 advisories; `/tmp/sagat-completion-restoration-audit.log`), and all four canonical contract
checks passed: OpenAPI (seven documentation warnings), protobuf compile, AsyncAPI (one info,
zero errors/warnings), IR example validation (`/tmp/sagat-completion-restoration-*.log`).
This closes the specifically static T016-054 gate, not the independent full runtime release gate.

The repaired collection-compensation scenario passed in **all five languages**, 43.69 s
(`/tmp/sagat-completion-sdk-repair-collection-compensation.log`), with its reviewed golden unchanged.
The runner path correction let the database scenario execute real PostgreSQL/OpenBao work in every
language. Its first comparison then found only the ordinary consumer's default 1.5 protocol and
two corresponding root identity fields versus the existing 1.4 golden; typed database evidence,
errors, effects and all other observations matched. Like the documented established root fixtures,
it now explicitly offers 1.4; the isolated managed executor still offers 1.6. This preserves the
old-worker compatibility contract and reviewed golden rather than replacing its observations.
The failed comparison is retained (`/tmp/sagat-completion-sdk-repair-database-connections.log`),
and its actual repeat is recorded when complete.


The explicitly pinned database-connections repeat passed **all five languages**, 28.75 s,
no ignored (`/tmp/sagat-completion-sdk-repair-database-connections-2.log`). It used the runner
resolved from the actual external Cargo profile with no SAGAT_DATABASE_RUNNER_BIN override,
real PostgreSQL/OpenBao, a 1.6 isolated executor and the 1.4 ordinary consumer. Canonical documents
and reviewed runtime goldens remain unchanged. The full 55-scenario matrix is repeated separately.


### Complete standard Studio journeys on the combined branch

The real PostgreSQL/OpenBao/isolated-runner browser harness passed **4/4 Chromium journeys**,
14.5 s browser time / 23.51 s aggregate, no ignored
(`/tmp/sagat-completion-current-database-browser.log`). It covers catalog and secrets with a lost
save response, keyboard/accessibility/responsive views, managed probes/ad hoc navigation identity,
visual workflow execution, role-scoped results, connection pause and safe SQL refusal. The harness
now uses the same actual-profile binary lookup as the SDK fixture; no binary override was supplied.

The standard complete Studio harness then passed **34 browser tests, 2 skipped** in 1.5 min,
98.89 s aggregate (`/tmp/sagat-completion-current-full-studio-browser.log`). It used real script
sandbox runners (SAGAT_STUDIO_NO_SANDBOX and grep filters unset), actual engine/API/PostgreSQL,
and the current built Studio. The two graphical benchmarks require STUDIO_PERF=1 and were
explicitly skipped by their existing configuration; their timing/resource criteria are not claimed.
No other journey was excluded. This includes authoring, script test/publish/execute, OIDC, graph,
security/accessibility and the expanded classification/disclosure journey. It is standard browser
acceptance, not the separate Docker Compose or large engine workload evidence.

### Bounded expiry command parity

Added expire_path_instance through the same scoped expiry/failure policy used by the full state.
An initial missing-symbol fixture failed to compile (API red, not a runtime failure;
`/tmp/sagat-completion-bounded-expiry-api-red.log`). The first focused gate passed 40/40; stronger
assertions initially used the wrong bounded accessor and did not compile. Corrected to the local
node accessor, then **41/41** focused tests passed, no ignored
(`/tmp/sagat-completion-bounded-expiry-final-green-2.log`). The new forward case checks 32
lease/attempt-timeout × high-risk/lookup × cancellation × stale-fence combinations, exact events,
independent expected statuses/reconciliation or review waits, and no stale mutation. A separate
case compares both expiry causes for compensation and lookup and verifies journal-only replay.
Strict Clippy of domain/all targets plus the integration-test package passed, 66 s
(`/tmp/sagat-completion-bounded-expiry-clippy.log`). The production scanner remains on full RunTx;
these command/representation checks do not claim bounded scanner or worker execution.

### Bounded scheduler fence and mutation clock checkpoint

The exact-path transaction now exposes both live lease clocks and the current scheduler shard
owner/generation. Outside-path reads return no lease; another shard, owner, generation, an
unrepresentable generation and an expired lease cannot authorize a write. The PostgreSQL shared
shard lock lasts through commit. A real competing update is refused with `55P03` until that commit,
then takeover succeeds and the former generation is refused. This is a port prerequisite; the
automatic scanner and worker still use full restoration at this checkpoint.

Two real PostgreSQL lock barriers exposed a separate deadline bug: a volatile SELECT target can
be evaluated before its `FOR UPDATE` finishes waiting. Both full and bounded mutations now sample
`clock_timestamp()` after acquiring the run lock. The failing baselines are retained in
`/tmp/sagat-completion-{bounded,full}-lock-clock-baseline.log`; the new assertions compare the
transaction time with database time immediately before the competing lock is released.

The first combined gate passed 15 cases and failed four during a PostgreSQL 16.15 backend SIGSEGV,
not a product assertion (`/tmp/sagat-completion-mutation-clock-byte-regression.log`). The exclusively
owned disposable server's image/identity/log are preserved under
`/tmp/sagat-completion-postgres-crash-evidence/`; Docker reported no OOM and the cause is not
established. After retiring that server, a fresh instance of the same image, without statistics
preload, passed **19/19**, no ignored, 58.32 s, as `sagat_app`
(`/tmp/sagat-completion-fresh-postgres-clock-byte-regression.log`). This functional profile does
not supply performance evidence or establish that instrumentation caused the crash. The exact
new container/image/port profile is recorded in `replacement.json` alongside the crash evidence.

### Reviewed SDK fixture alignment

The complete 55-scenario/five-language checkpoint completed in 2291.45 s and failed four scenarios
(`/tmp/sagat-completion-current-sdk-conformance-3.log`). All other selected scenarios passed. The
three stale expected-document fixes are narrowly reviewed: loop-runtime and scoped-identity now
preserve their explicitly authored limits in both workflow JSON and canonical text; the registry
preflight checks that relationship. Protocol negotiation now expects 1.4 and 1.5 for the two
workers that explicitly offer those versions; the newer 1.9 probe still negotiates the current
server ceiling 1.6. No SDK source, runtime policy or unrelated golden was changed or auto-blessed.

The fourth discrepancy was an optional `step.started` event when a 200 ms heartbeat beat a short
secret-read activity. The secrets fixture now explicitly schedules that heartbeat at 60 seconds,
after its 30-second completion timeout. Its golden and raw no-leak assertions remain unchanged;
normalization removes no journal events. Dedicated heartbeat scenarios remain in the matrix.

All four reviewed repeats passed in all five languages: loop-runtime 35.88 s,
protocol-negotiation 56.19 s, scoped-identity 46.04 s, secrets 24.19 s
(`/tmp/sagat-completion-sdk-reviewed-{loop-runtime,protocol-negotiation,scoped-identity}.log`,
`/tmp/sagat-completion-sdk-secrets-heartbeat-fixture.log`). Registry preflight passed 1/1, 0.05 s
(`/tmp/sagat-completion-sdk-reviewed-registry.log`). These targeted repeats repair the recorded
failed checkpoint; they are not a claim that a later whole matrix has already completed.

### Canonical local Compose execution

The Docker Rust builder now defaults to three Cargo jobs, configurable through the
`CARGO_BUILD_JOBS` build argument. The documented local memory budget completed both real
five-process release builds. The first Compose smoke passed migration/seed/health/runtime audit
immutability and CLI work, then exposed an uncompiled TypeScript example prerequisite; its failed
log remains `/tmp/sagat-completion-current-compose-smoke.log`. After locked installation and
compilation of that example, the complete smoke passed, including CLI, TypeScript worker and real
Chromium **1/1**, no ignored (`/tmp/sagat-completion-current-compose-smoke-2.log`). The script
removed only its own Compose project and volumes. Images were built before the later mutation-clock
fix; this checkpoint does not claim rebuilt post-clock release images or final release acceptance.

### Runtime event-read privilege repair

The broad workspace checkpoint exposed two privilege-audit findings. The dead-letter read really
failed as `sagat_app`: `FOR SHARE` requires UPDATE permission on at least one column, while this
table intentionally grants no UPDATE. The isolated runtime-role baseline failed 1/2
(`/tmp/sagat-completion-dead-letter-runtime-privilege-red.log`). The read now selects the committed
immutable record without that row lock; namespace/membership/reveal authority stays locked through
the projection/audit commit. No new privilege is granted. Its runtime-role repeat passed **2/2**,
18.04 s (`/tmp/sagat-completion-dead-letter-runtime-privilege-green.log`).

The other finding was the SQL auditor treating the LATERAL keyword as a relation. A standalone
regression failed first (the initial test also incorrectly included `public.` in the auditor's
normalized function name; both red logs are retained), then **17/17** scanner tests passed. The
new case still requires function EXECUTE and nested-relation SELECT/row-lock checks. The complete
real-database runtime-privilege module then passed **32/32**, 55.49 s
(`/tmp/sagat-completion-current-runtime-privilege-audit.log`), with no added exception or grant.
The model document now describes the implemented prepared-request and authority-lock boundaries.

### Current-authority fixture alignment

Legacy worker fixtures must persist their current tenant membership before expecting a successful
registration/session. Added explicit worker memberships in the foreign durable-worker/quota cases,
foreign compensation breaker, operation redaction, secret-pool helpers, binding helpers and direct
session-store fixture. Deliberately unauthorized cross-tenant calls remain unauthorized; production
membership/binding checks are unchanged by this fixture commit. The session-quota fixture also
creates its second identity through the membership service.

Targeted real repeats passed: operation redaction **1/1**, 4.43 s; foreign durable worker **1/1**,
21.91 s; current tenant isolation **9/9**, 12.23 s; secret pools **17/17**, 91.47 s; foreign breaker
**1/1**, 19.86 s. Logs are `/tmp/sagat-completion-operation-redaction-green.log`,
`/tmp/sagat-completion-foreign-worker-membership-green.log`,
`/tmp/sagat-completion-current-isolation-regression.log`,
`/tmp/sagat-completion-secret-pool-membership-fixtures-green.log` and
`/tmp/sagat-completion-breaker-membership-fixture-green.log`. The preceding complete durable repeat
was **177 passed/1 failed**: it had compiled before the final foreign-session membership repair;
that remaining case is included in the successful nine-case isolation repeat.

The security sweep exposed two additional binding fixture failures, reproduced separately:
the direct store fixture had no membership, and the lease-index availability test was selecting
the newly added `steps_path_live_idx` on its tiny tables. Like its existing temporary removal of
the open-run index, the latter now removes that competing index only inside a rolled-back test
transaction; the assertion still requires the session lease index. Both failed baselines remain
`/tmp/sagat-completion-binding-{session-membership,lease-index}-red.log`.
The complete binding repeat passed **16/16**, 81.59 s, as the runtime role
(`/tmp/sagat-completion-current-binding-regression.log`). The independently running broad
workspace/security checkpoints retain their original failures; they are not rewritten as passed.

### Cancellation acknowledgement with declared secrets

The current scoped-worker regression reproduced a real refusal of the worker's explicit
nonretryable `CANCELLED` acknowledgement: internal report scanning incorrectly required a live
external capability after cancellation. The isolated baseline failed with `StaleAttempt`
(`/tmp/sagat-completion-bounded-maintenance-cancel-isolated.log`); the first scoped scanner
repeat was **15 passed/1 failed**, with that same pre-existing report problem.

Only that closed acknowledgement now permits cancellation through the internal scan's fence.
Current membership/binding, exact attempt/generation/token, unexpired lease, secret-version
snapshot and durable receipt replay remain checked. Complete and generic failure reports keep
the live capability fence; no new secret/blob/database access is issued. The strengthened scoped
case verifies each refusal and the acknowledged cancellation's journal/projection/restart outcome.
Its initial extra blob assertion incorrectly expected `StaleAttempt`: ordinary secret-bearing
blob uploads already return the stricter existing `NotAllowed` refusal. The failed 51/1 checkpoint
is retained (`/tmp/sagat-completion-scoped-cancellation-and-iteration-ceiling-regression-2.log`).

The corrected combined scoped-worker/loop repeat passed **52/52**, 178.30 s, as `sagat_app`
(`/tmp/sagat-completion-scoped-cancellation-and-iteration-ceiling-regression-3.log`). The complete
real PostgreSQL/OpenBao lifecycle/contract target passed **3/3**, 155.07 s
(`/tmp/sagat-completion-cancellation-ack-provider-regression.log`), including ordinary
resolve/complete/fail cancellation barriers during provider I/O, rotation, outage receipt replay,
derived recovery and managed-output cancellation. Whole-workspace/all-target strict Clippy passed
2 min 44 s (`/tmp/sagat-completion-cancellation-ceiling-workspace-clippy.log`). These combined-tree
checks also include the bounded scanner and ceiling repair recorded in the following checkpoints.

### Iteration ceiling and database-policy schema coexistence

The loop entry rollback fixture reproduced a combined-schema regression: the feature-017 database
policy allowlist omitted feature 014's `iteration_concurrency`. Inserting the accepted scheduler
ceiling failed with PostgreSQL `23514` before the intended capacity test
(`/tmp/sagat-completion-loop-entry-quota-isolated.log`). Added migration
`20261209000000_iteration_ceiling_compatibility`; every pre-existing migration remains unchanged.
Both resource families remain constrained to their explicit allowlist and existing value ranges.

The real-database coexistence test retains both an iteration and database ceiling, proves rollback
refuses to discard the former, explicitly withdraws it, rolls back and re-applies, then verifies
the database ceiling stayed unchanged. It and the previously failing loop-entry case pass in the
52-case combined repeat recorded above. The full migration module passed **5/5**, 14.31 s
(`/tmp/sagat-completion-iteration-ceiling-migration-regression.log`), including concurrent runners,
idempotence, complete rollback/reapply and writing data after reapplication. This verifies the
current combined schema; it does not assert an untested upgrade from every unreleased branch snapshot.

### Automatic bounded scoped expiry and retry

Wired the automatic scoped lease-expiry/retry passes to `PathExecutionStore` and their shared pure
bounded commands. Exact-path state is loaded outside the run write lock; acquisition compares the
run revision, then holds the scheduler shard fence, rechecks both live lease clocks and persists
the addressed events/effects in the same short transaction. A revision conflict is a skipped hint
for rediscovery. The exceptional content-quota drain preserves the prior full-run
revision/fence/condition recovery guard. Root maintenance and scheduler metric/span names remain
unchanged. Worker claim/report paths are not converted by this checkpoint.

The real isolation regression revokes `iteration_checkpoints` SELECT from `sagat_app` in its own
database, first proves full restoration is unavailable, then requires lease recovery and retry
release without that history. After restoring the normal grant it verifies journal replay against
the projections. The first invocation accidentally selected zero cases and is not evidence;
the correctly selected baseline failed with one scheduler action failure
(`/tmp/sagat-completion-bounded-maintenance-red-2.log`). After wiring, it passed **1/1**, 10.28 s
(`/tmp/sagat-completion-bounded-maintenance-green.log`). The wider scoped-worker/loop gate passed
**52/52** as recorded above, including rollback, restart, stale attempts, cancellation acknowledgement,
capacity, timeout, routing, outbox/audit failures and the schema coexistence case. Strict whole-workspace
Clippy passed; the separate complete fault matrix is recorded when it finishes. These are bounded
maintenance correctness checks, not the pending 10k/100k resource acceptance.

### Complete workspace diagnostic checkpoint and immutable chain admission

The original `cargo test --locked --workspace --no-fail-fast` diagnostic finished: **234 completed
test harnesses, 3310 passed, 35 failed, 61 ignored**, plus two doctest compilation failures
(`/tmp/sagat-completion-workspace-tests.log`). Thirteen targets failed overall. Its build began
from the earlier integration checkpoint while this work continued; it is not final-current-tree
acceptance. The CLI/protocol/tenant/runner/worker membership/privilege/report/quota failures and
their named focused repeats are recorded in the preceding checkpoints. The two rustdoc failures
used current trait implementations against libraries compiled before those trait additions;
current IR/PostgreSQL doctest compilation subsequently passed, with **zero executable examples**
(`/tmp/sagat-completion-current-ir-postgres-doctest-compilation.log`), not two executed tests.
The earlier primitive performance target passed **4 cases/5 ignored**, 344.08 s; its modest
flat-transition assertions do not run the independently ignored 10k/100k capacity workloads.

One remaining chain fixture attempted to change an admitted run's immutable version pin. The
isolated baseline reproduced the correct `23001` admission-identity refusal
(`/tmp/sagat-completion-chain-handoff-retained-quota-isolated.log`). The fixture now publishes its
real continuation definition and admits the chain with the correct immutable version and initial
chain metadata. Its direct handoff/rollback/replay assertions remain, without disabling a guard or
rewriting an admitted identity. All **7/7** chain-handoff cases passed, 10.30 s
(`/tmp/sagat-completion-chain-handoff-immutable-admission-regression-shm-2.log`). The first repeat
hit the disposable database's recovery; a fresh-cluster repeat then exposed a separate concurrent
test-role bootstrap race, both retained as failures. The successful repeat uses the healthy,
explicitly sized PostgreSQL shared-memory profile recorded in the next checkpoint.

### Explicit PostgreSQL shared memory and concurrent test bootstrap

The replacement functional server, with no statistics preload, also crashed. Its log records
`could not resize shared memory segment ... to 33554432 bytes: No space left on device` before the
SIGSEGV; inspecting Docker showed its separate `/dev/shm` limit was **64 MiB**, not the free space
of the data volume. The original server's preserved log contains the same resize error. Removing
statistics preload therefore did not resolve the exhaustion. Logs and filtered identity/resource
metadata of the second failure are retained in
`/tmp/sagat-completion-postgres-crash-evidence/functional-second-crash/`. No vendor SIGSEGV cause
is asserted beyond this observed sequence; Docker reported no OOM.

Only the exclusively owned disposable server was retired. Its fresh replacement keeps the same
PostgreSQL 16.15 image and default query/instrumentation settings, with **1 GiB** shared memory
(`replacement-shm.json` alongside that evidence). The complete fault matrix, unchanged executable,
then passed **87/87**, 161.21 s
(`/tmp/sagat-completion-bounded-maintenance-fault-matrix-shm.log`); the preceding **78/9** run is
retained. The Rust container harness and Compose PostgreSQL now budget that shared memory
explicitly, following the official image's documented Docker shared-memory guidance. No query
parallelism or product limit was reduced to obtain the repeat. Compose interpolation with public
verification-only settings confirms `shm_size=1073741824`
(`/tmp/sagat-completion-shm-compose-config-2.json`); the first lint attempt lacked required OIDC
variables and is retained. This is a configuration check, not another rebuilt Compose smoke.

Starting two Rust test processes against the fresh shared server exposed `23505` on
`pg_authid_rolname_index`: CREATE ROLE's concurrent catalog race is not the `duplicate_object`
exception the helper already handled. The test helper now takes a transaction advisory lock before
creating the role. A separate exclusively owned fresh-cluster check ran two actual compiled Rust
processes concurrently: each passed **1/1**, 1.28 s, both exited zero
(`/tmp/sagat-completion-runtime-role-concurrency/`). The reproduction script is retained at
`/tmp/sagat-completion-runtime-role-concurrency.py`; it saved identity/logs and removed only its own
container. No production role/grant changed.

### Authority and expiry after lock contention

Three real lock barriers failed on the healthy 1 GiB server, independently of the earlier database
crashes: the bounded binding read did not retain its revocation fence; both full and bounded
scheduler predicates could authorize a lease that expired while waiting for an unchanged shard row.
The first baseline failed **0/3**, 10.11 s; the explicit shared-memory repeat also failed **0/3**,
11.59 s (`/tmp/sagat-completion-bounded-authority-lock-barriers-red{,-shm}.log`). These are genuine
product assertion failures, not infrastructure failures.

Full and bounded shard fences now share one adapter operation: validate the run's shard and exact
representable generation, lock the matching owner/generation row, then sample database time and
compare expiry. A successful check refreshes mutation time before the scheduler's deadline checks.
Bounded and full binding reads share the transaction advisory authority lock used by revocation.
No domain representation, schema, wire shape, external effect or grant changed.

The three corrected barriers passed **3/3**, 9.39 s
(`/tmp/sagat-completion-bounded-authority-lock-barriers-green.log`). The combined locks, byte
ceilings, chain handoffs, scoped maintenance and loops passed **81/81**, 85.70 s
(`/tmp/sagat-completion-current-lock-maintenance-regression.log`). Strict whole-workspace/all-target
Clippy passed 1 min 19 s (`/tmp/sagat-completion-current-authority-workspace-clippy.log`).

Runtime-role profile clarification: enabling `SAGAT_TEST_AS_RUNTIME_ROLE=1` makes World services use
that role; low-level fixtures that explicitly construct a trusted owner store are not thereby
converted. Earlier combined-profile notation is not a claim that every helper used a non-owner
connection. The scoped-worker fixture now explicitly selects an eight-connection runtime pool when
the switch is enabled. Its scoped operations plus the three barriers passed **19/19**, 22.41 s
(`/tmp/sagat-completion-bounded-fences-runtime-role-regression.log`); the shard-wait barrier requires
the waiting connection's actual `usename='sagat_app'`. The database-wide privilege audit passed
**1/1**, 4.71 s (`/tmp/sagat-completion-authority-lock-runtime-privilege-audit.log`), without an added
exception or grant. Worker claim/report conversion and large workload acceptance remain open.

### Current exact-path heartbeat and ordinary worker authority

Scoped heartbeat now locks the run and restores its current bounded path on the same database
connection. The repeatable-read prepared reader and revision-checked scheduler transaction remain
separate; both readers share the complete bounded restoration implementation. This avoids holding
one pool connection while requesting another and avoids heartbeat revision retry storms. Missing
unadmitted paths remain opaque stale attempts. Live renewal and the first started event commit
atomically; a late acknowledgement after the same lease reports, even after run closure, renews
nothing and still requires current binding/membership. Root heartbeat now rechecks persisted
ordinary-worker membership after the run lock; the managed executor keeps its separate authority.
No wire, SDK source, schema, privilege or expected conformance output changed.

Four real tests failed before conversion: revoked-history scoped heartbeat, terminal late heartbeat,
current Root membership after contention, and twelve concurrent heartbeats on two runtime-role
connections (`/tmp/sagat-completion-bounded-worker-heartbeat-pool-red.log`, **0/4**). The correctly
configured terminal baseline also failed independently. Earlier fixture-placement and error-mapper
mistakes are retained in their named red/green logs; they are not product evidence. The corrected
focused suite passed **4/4**, 16.72 s
(`/tmp/sagat-completion-bounded-worker-heartbeat-green-3.log`). Shared-reader intermediate regression
passed **38/38**; current bounded worker/fence/byte/chain/scoped/loop gate passed **85/85**, 155.21 s
(`/tmp/sagat-completion-bounded-heartbeat-workflow-regression.log`). Current PostgreSQL/OpenBao
aggregate passed **3/3**, 95.75 s (`...-provider-regression.log`); strict whole-workspace/all-target
Clippy passed, 3 min 40 s (`...-workspace-clippy.log`). The new scoped trace contains tenant/run
identity, no input/token, and preserves the existing late-heartbeat counter.

The current server's `scoped-identity` and `lease-lost` scenarios each passed in all five SDKs
(**1/1** aggregate, 43.43 s and 16.97 s; `/tmp/sagat-completion-bounded-heartbeat-sdk-*.log`).
These focused repeats are not the full 55-scenario gate. A first secrets invocation omitted
`--ignored` and selected zero executed cases; it is not passing evidence.

The current gRPC suite twice finished **69/70**, with different stream timing failures. The first
byte-credit test stopped consuming while making thirty large REST admissions inside a five-second
stream window, then saw a transport BrokenPipe. That case passed alone both as owner (**1/1**,
11.05 s) and runtime role (**1/1**, 6.66 s). The second suite approved that case but failed the
watchdog survivor test's expected task after sleeping 4.1 s and then admitting a large run near the
same five-second deadline. These observations do not establish a production regression or an
approved complete gRPC gate; deterministic fixture synchronization and another complete repeat
remain required. Claim/report conversion, wider body graphs and large capacity gates stay open.

The correctly selected current `secrets` all-five repeat subsequently passed **1/1**, 33.48 s
(`/tmp/sagat-completion-bounded-heartbeat-sdk-secrets-2.log`). Current SQL privilege audit passed
**1/1**, 4.67 s: 1,034 statements in 52 files, 653 distinct needs, 620 real runtime-role probes,
no additional exception (`/tmp/sagat-completion-bounded-heartbeat-runtime-privilege-audit.log`).

### Deterministic stream work admission and real SDK receive limits

The byte-credit contract now admits both waves before opening the finite stream, holds the second
wave behind actual run row locks, checks the first frame belongs to the first wave, then releases
the barrier. The watchdog survivor similarly prepares its delayed work before opening its stream
and releases its run lock after the old deadline, retaining the 4.1 s delayed-work assertion.
No production stream deadline, capacity or receive credit changed.

The first barrier repeat exposed a previously hidden fixture limit: the second frame was
6,172,012 bytes, above Tonic's default 4 MiB decoder. The actual public SDK transports already
allow 16 MiB for the protocol's finite 8 MiB input budget plus metadata; the Rust test worker now
uses that same decoder bound. The failed repeat is retained
(`/tmp/sagat-completion-bounded-heartbeat-grpc-barrier-regression.log`, **69/70**). The corrected
complete runtime-role gRPC suite passed **70/70**, 31.87 s
(`/tmp/sagat-completion-bounded-heartbeat-grpc-barrier-regression-2.log`). The earlier transport
and near-deadline failures are retained above; the final passing checkpoint belongs to the
heartbeat server and these fixture changes, before the subsequent bounded-claim implementation.

### Bounded worker claim implementation and expanded regression findings

The baseline `bounded_worker::scoped_claims_need_no_unrelated_iteration_history` failed **0/1**,
1.30 s with `iteration_checkpoints` SELECT revoked from its explicit two-connection runtime pool
(`/tmp/sagat-completion-bounded-worker-claim-red.log`). After exact-path leasing the first focused
repeat passed **5/5**, 1.94 s (`...-green.log`, before three additional barrier/fault cases).
The expanded workflow run subsequently passed all **8** bounded-worker cases: claim without
history, a later-path failure preserving the earlier committed token, and current binding/role
under actual authority-lock contention, alongside the heartbeat cases.

Discovery now preserves `(run, path)` from the existing quota/pool/protocol/breaker-aware finite
candidate query. Run-only and exact-path projections share its SQL; an earlier independent path
hint was replaced during review because it could hide eligible activities behind capped/restricted
work. Claims are atomic per path. Drain/binding/session/current membership/admission fences remain
held through lease journal/projection/outbox commit. Input bytes and returned token accounting only
include committed work. Root task identity/template construction is shared without cloning the
whole immutable definition. Managed tasks remain root-only; report/capability conversion stays open.

The nested fixed security predicate in that shared SQL template initially required extending the
privilege extractor, including a focused extractor test; its first compilation exposed a missing
map value annotation and is retained (`...-runtime-privilege-audit.log`). The corrected complete
privilege module passed **33/33**, 8.36 s, including **1,035 statements in 52 files**, 653 distinct
needs and 620 actual runtime probes, with no new grant/exception
(`/tmp/sagat-completion-bounded-worker-claim-runtime-privilege-audit-2.log`).

The first full workflow diagnostic finished **328 passed /15 failed /1 ignored**, 209.40 s
(`/tmp/sagat-completion-bounded-worker-claim-workflow-regression.log`). Its concurrent gRPC run
finished **68/70** with two workspace-admission HTTP 503 responses
(`...-grpc-provider-regression.log`). Running both default-thread harnesses against the default
100-connection disposable server exhausted connections (`53300`, setup PoolTimedOut); that
invocation is not a resource acceptance profile. Subsequent heavy suites use four harness threads
and run in sequence rather than raising the server's production limits. No server crash is
inferred from connection exhaustion.

Not all fifteen failures were infrastructure: six loop compensation cases exposed the bounded
reader decoding SQL NULL as an active-iteration boolean after loop completion. Completed loops
correctly have no active iteration; the reader now records false and retains separate durable
saga authority for derived work. The repeated scoped compensation module covers those real failed
cases. Eight child-chain cases exposed the strict run-shard fence being invoked on a new child's
transaction with the parent's lease. The isolated child-start test still failed **0/1**, 1.15 s
without competing harnesses (`...-child-chain-isolated-2.log`); its earlier isolated setup hit
`53300` and is not product evidence. A separate explicit child-admission fence fix is required;
ordinary full/bounded run shard validation must remain strict. Final current-source workflow,
protocol/provider, SDK and large capacity gates remain open until their repeats are recorded.

The current shared-discovery/NULL-fence implementation passed the selected bounded worker, scoped
worker, loop and complete scoped-compensation modules **79/79**, 69.68 s with four harness threads
(`/tmp/sagat-completion-bounded-worker-claim-focused-regression-2.log`), including all six genuine
loop failures recorded above. An initial invocation passed multiple filters to Cargo rather than
the test harness and executed no tests (`...-focused-regression.log`); it is not passing evidence.
Strict whole-workspace/all-target Clippy subsequently passed, 1 min 43 s
(`/tmp/sagat-completion-bounded-worker-claim-clippy-2.log`). Earlier interface/signature compilation
failures are retained in `...-check{,-2}.log`. Protocol/provider and all-five current claim repeats
are still pending at this commit; this focused checkpoint does not relabel the 328/15 diagnostic
or close the child-admission defect or the open feature tasks.
