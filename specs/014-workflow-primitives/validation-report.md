# Validation report — feature 014

Original report: 2026-10-14. It covers specification of future primitives and executable examples
of current v1 behavior. Later sections record the additive identity foundation and the bounded
switch implementation separately.

## Executed by the integrating agent

| Check | Result |
|---|---|
| `sagat workflow validate` on routing, fulfillment and reminder JSON documents | All three valid using the existing CLI/IR validator |
| `cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::` | Passed: 1 aggregate test, 25 workflow runs and 10 invalid-configuration checks across all five languages; 48.73 seconds |
| Same E2E with `SAGAT_EXAMPLE_LANGS=typescript,python` during integration | Passed: 10 workflow runs and 4 configuration checks; 22.54 seconds |
| `cargo clippy -p sagat-tests --test e2e_developer_experience --locked -- -D warnings` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo test -p sagat-tests --test contract_developer_experience --locked` | Passed: 8 tests, including links, executable scripts, commands, reference consistency and secret scanning |
| Spec Kit `check-prerequisites.sh --json --require-spec --require-tasks --include-tasks` with explicit feature directory and no-persist | Passed; spec, plan, tasks and supporting artifacts discovered |

The aggregate E2E builds every example, provisions a disposable real PostgreSQL-backed platform,
starts the engine, publishes all three workflows and binds the four activity capabilities. Its
assertions cover persisted terminal status, selected branch, absence of notification on rejection,
parallel join/timer ordering, traces, typed permanent failure, output digest parity and token
redaction. A language's successful build alone is not counted as runtime evidence.

Local test environment: macOS, the repository's pinned Rust toolchain, Docker through a Colima
context, and installed Node/Python/Go/.NET/Java toolchains. Build settings: a `CARGO_TARGET_DIR`
outside the repository, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_BUILD_JOBS=2`,
`RUST_TEST_THREADS=2`, with `DOCKER_HOST` pointing at the Colima socket and `JAVA_HOME` at the
installed JDK. These are local choices, not repository requirements; on another machine use its
Docker endpoint and JDK.

## Intermediate failures resolved during integration

- Documentation checks initially found links/scripts while language artifacts were still being
  created. They were not accepted as successful validation.
- The initial invalid-scenario assertion expected the rejected value to be echoed. It now verifies
  the variable name, consistent with diagnostics that avoid reflecting arbitrary input.
- An early full-matrix attempt stopped on the Go script's missing executable bit while its files
  were still being prepared. All ten scripts are executable in the completed delivery.

## Supporting agent evidence

A supporting agent wrote the language examples and the common E2E harness. Its reported focused checks
included TypeScript compilation, Python Ruff/strict mypy/compilation, shell checks, Go build/test/vet,
.NET Release build and Java build. The integrating agent's all-five E2E independently rebuilt and
executed the final examples; the language-specific lint claims above remain supporting agent reports.
Another agent wrote the specification and its first consistency review. The
[Spec Kit analysis](analysis.md) records that review and the later validation against the codebase
and accepted ADRs (11 findings, corrected in the documents). Five tasks are complete; all 40 future
implementation/release tasks remain open.

## Post-validation changes to US01 (2026-10-14)

After the validation, the E2E harness gained assertions and the documentation was corrected:

- `routing-approved` now also asserts that the rejected end is never reached and that the condition
  is evaluated before `notify` is leased.
- `reminder-notification-failure` now also asserts that no `step.completed` exists for `notify` and
  that `run.finished` carries the `FAILED` outcome.
- The harness header traces FR-014-001/002 and T014-010; a language subset prints that it is not
  five-language evidence.
- The TypeScript and Python READMEs export the settings before publishing and use the tested
  `reminder` failure case; the Go README says the identity is bound to activities; the boundary
  amount 0 is documented as rejected (not exercised by the suite).

The results in the table above predate these changes. Re-executed after them, on the same kind of
local environment:

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::` (all five languages, `SAGAT_EXAMPLE_LANGS` unset) | Passed: 1 aggregate test with the strengthened assertions; 47.80 seconds |
| `cargo clippy -p sagat-tests --test e2e_developer_experience --locked -- -D warnings` | Passed |
| `rustfmt --edition 2024 --check tests/e2e/developer-experience/order_processing.rs` | Passed |
| `cargo test -p sagat-tests --test contract_developer_experience --locked` (covers the edited READMEs' commands, scripts, links and secrets) | Passed: 8 tests |

## Limits of this evidence

No crash, tenant-isolation or failover guarantee for a **new primitive** has been tested: those
primitives are proposals and their runtime tasks remain unchecked. The new examples simulate
provider effects; they prove orchestration and SDK parity for these fixtures, not provider-side
idempotency or exactly-once external execution. The feature's current GitHub CI run has not been
executed as part of this local report. Full platform tests are not repeated for this documentation
and example-only change; existing crash/fault suites remain required for future runtime delivery.

## Additive v1 foundation increment (2026-10-14)

ADR-0032 supersedes the proposed separate workflow IR revision. The canonical workflow schema
now describes new v1 forms while the codec rejects every new declaration and node kind until its
execution support is complete. Existing accepted documents and the example digest remain valid.
Worker identity fields are additive; the server still advertises protocol 1.4 and refuses scoped
reports. No scoped task is scheduled.

The domain now models bounded execution paths and validates closed collection bodies without
enabling them. PostgreSQL gains empty-path root projection, scoped-key capacity and nullable
journal identity. New root events write an explicit identity; old journal events are not rewritten
and still replay as roots. Query/Update and approval permissions are represented, but no public
operation endpoint is enabled.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 10 schema, compatibility, worker and error-contract tests |
| `cargo test -p sagat-infra-ir --test codec --locked -q` | Passed: 40 tests, including rejection of new declarations and collection nodes before runtime support |
| `cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-api-grpc -p sagat-api-http --locked -q` | Passed across all selected packages and targets |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` with real PostgreSQL via Colima | Passed: 4 tests for migration, path constraints, root dual-write and old journal replay |
| `cargo test -p sagat-tests --test contract_grpc_sdk_multilanguage --locked -q` | Passed: 7 worker negotiation and compatibility tests |
| `cargo check --workspace --all-targets --locked -q` | Passed |
| `cargo clippy -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres -p sagat-api-grpc -p sagat-api-http -p sagat-tests --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all --check` and `git diff --check` | Passed |

These checks cover the compatibility foundation only. At that checkpoint collection scheduling,
loops, approvals, switch, state operations and continuation were disabled; their transaction, fault, tenant,
observability and five-SDK conformance tests are still required. The registered fault and end-to-end
targets contain no runtime scenario yet and are not counted as passing feature evidence.

## Bounded switch increment (2026-10-15)

The same workflow IR v1 now accepts a root `switch` with one to 32 ordered cases, unique local case
IDs and a required default. The first true predicate wins; a predicate error fails with
`EXPRESSION_ERROR`. The selected case ID or default is committed in the journal and step projection
with successor scheduling and outbox. Reload restores the decision without reevaluation. The
runtime graph exposes the selected route, and a committed event produces a stable trace span and a
metric with only closed kind/outcome labels. The schema disallows `onError` on switch, so a predicate
error cannot be disguised as a fallback route.

| Check | Result |
|---|---|
| `cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres -p sagat-api-http -p sagat-scheduler -p sagat-telemetry --locked -q` | Passed across all selected packages and targets, including new switch reducer, codec, graph and telemetry tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 10 schema, version and contract tests |
| `cargo test -p sagat-tests --test contract_grpc_sdk_multilanguage --locked -q` | Passed: 7 worker-protocol compatibility tests after synchronizing the five SDK proto copies |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` with real PostgreSQL | Passed: 8 tests, including case/default, first match, predicate failure, duplicate start, replay/projection, outbox rollback/retry, legacy NULL upgrade and guarded migration rollback |
| Full TypeScript, Go, Python, .NET and Java SDK unit suites | Passed: TypeScript 112, Python 243, .NET 429, Java 216; Go `go test ./...` passed (supporting agent execution) |
| `SAGAT_SDK_SCENARIO=switch-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: 1 shared WP-07 scenario in TypeScript, Go, Python, C# and Java. All five produced the same reviewed builder document and runtime observations for first match, default and `EXPRESSION_ERROR`; canonical workflow text SHA-256 `e3ba58ef16d08ee7c9c8d250877c3aa233d6b5becf7ad3db7c8e0b5d296f75d8`. |
| `cargo test -p sagat-tests --test fault_workflow_primitives --locked switch:: -- --nocapture` with real PostgreSQL | Passed: 3 tests for lost start response/duplicate replay, restart with an undelivered broker hint, and atomic rollback/retry when successor outbox insertion fails. Each test independently checks one journaled choice, one selected successor and one task hint. |
| `cargo check --workspace --all-targets --locked -q` | Passed |
| `cargo clippy -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres -p sagat-api-http -p sagat-scheduler -p sagat-telemetry -p sagat-tests --all-targets --locked -- -D warnings` | Passed after fixing test-only lint findings |
| `cargo test -p sagat-tests --test contract_developer_experience --locked -q` | Passed: 8 tests after regenerating the schema-derived workflow reference with a capability warning |

The enabled switch fixture and both reviewed goldens cover the five public SDKs: amount `1` proves
the first of two true cases, amount `-1` proves the default, and a missing amount proves typed failure
without fallthrough. The runtime golden also verifies that non-selected nodes remain `NOT_STARTED`.
The real PostgreSQL fault target separately proves durable replay, broker-independent recovery and
transaction rollback. The other proposed primitives remain rejected at publication. The server still
advertises worker protocol 1.4 and rejects scoped work; additive 1.5 fields in the canonical and SDK
copies do not enable collection execution. No workflow IR v2 is introduced.

## Scoped read-side checkpoint (2026-10-15)

The locked run reader now reconstructs every step using its full typed path instead of collapsing
equal definition-node names. A real PostgreSQL regression inserts a root step and two scoped rows
for the same node, then checks that `RunStore::begin` retains all three. The domain resolver follows
declared collection bodies for nested paths and rejects invalid owners, local nodes and indices.
These changes do not schedule scoped work; planner commands, storage writers and worker 1.5 remain
release gates.

| Check | Result |
|---|---|
| `cargo test -p sagat-domain --locked -q` | Passed: all domain test targets, including nested resolution and scoped finalization |
| `cargo test -p sagat-tests --test contract_workflow_primitives --test integration_workflow_primitives --test fault_workflow_primitives --locked -q` | Passed: 14 contract, 10 real-PostgreSQL integration and 3 fault tests |
| `cargo clippy -p sagat-domain -p sagat-infra-postgres -p sagat-tests --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all` | Passed |

The application graph snapshot was then extended to retain each step's full instance identity.
The HTTP graph continues to render root nodes only, so a scoped projection cannot overwrite a
root node with the same local name. The complete scoped graph topology remains pending. The
real-PostgreSQL integration target passed all 10 tests after this read-side change.

## Structured validation and scoped reducer checkpoint (2026-10-15)

The explicit structured IR parser now checks additive v1 collection bodies and limits, compiles
expressions in all graphs and preserves canonical bytes on reparse. The production publication
parser still refuses these forms. A separate reducer address carries the typed scope path for
release, lease and start on already restored instances, with independent fences. Completion,
admission, storage writes, journal replay from an empty run and worker 1.5 remain disabled.

| Check | Result |
|---|---|
| `cargo test -p sagat-infra-ir --locked` | Passed: 74 tests across unit, cache, codec, diff and projection targets |
| `cargo test -p sagat-domain --locked -q` | Passed: full domain suite, including 6 scoped reducer tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | Passed: 14 contract tests |
| `cargo clippy -p sagat-infra-ir -p sagat-domain --all-targets --locked -- -D warnings` | Passed in focused package runs |

## Five-SDK collection authoring checkpoint (2026-10-15)

The five public builders now emit `forEach`, its `map` alias, named bodies and `return` in the
same additive v1 document. IDs are local to each body. The shared `collection-builders` fixture
uses `buildOnly: true`: it verifies builder documents, canonical text and validation errors, and
does not publish or execute these workflows. Its golden was reviewed manually.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored` | Passed: 1 shared fixture across TypeScript, Go, Python, C# and Java |
| Go full SDK suite | Passed |
| Python full SDK suite | Passed: 244 tests |
| Java full SDK suite | Passed |
| .NET full SDK suite | 429/430 passed; an existing stream timing test passed when rerun alone |
| TypeScript full SDK suite | 112/113 passed; an existing worker cancellation timing test passed when rerun alone; the new workflow tests passed |
| `cargo test -p sagat-tests --test contract_workflow_primitives five_sdk_collection_golden_is_a_valid_structured_v1_definition --locked -q` | Passed: both five-SDK golden documents parse as structured v1, match server canonical bytes and remain rejected by production publication |

The full Rust workspace test invocation was interrupted during compilation to free Cargo's build
lock for the five-SDK conformance run. The focused Rust domain, IR, contract, PostgreSQL and fault
targets above passed. A clean full-workspace run is still a release gate.

## Collection reducer and worker identity checkpoint (2026-10-15)

The domain now records collection entry, ordered admission, return, result checkpoint, drain and
completion as typed events. Digests identify the captured input, individual items and retained
results. Replay rejects a return without its admission. The reducer refuses to drain an iteration
with an in-flight, waiting, retrying or otherwise unreconciled attempt. The PostgreSQL event codec
round-trips these event fields and rejects malformed instance paths. PostgreSQL storage still
refuses to project a collection wait, because durable scope/checkpoint storage is not integrated.

The worker application and gRPC request types now carry `ExecutionInstanceId`. Root task IDs and
step IDs remain compatible; the server validates path shape but refuses scoped RPC operations before
opening storage. It continues to negotiate protocol 1.4, including for clients requesting 1.5.
This does not enable scoped claims, completions, secret access or blob access.

| Check | Result |
|---|---|
| `cargo test -p sagat-domain -p sagat-application -p sagat-api-grpc -p sagat-infra-postgres --locked -q` | Passed across all package test targets, including 10 collection reducer tests and 3 PostgreSQL event codec tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --test contract_grpc_sdk_multilanguage --test integration_workflow_primitives --locked -q` | Passed: 15 contract, 7 gRPC compatibility and 10 real-PostgreSQL integration tests |
| `cargo check --workspace --all-targets --locked -q` | Passed |
| `cargo clippy -p sagat-domain -p sagat-application -p sagat-api-grpc -p sagat-infra-postgres -p sagat-tests --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

An earlier `cargo test -p sagat-tests --no-run --locked` reached test linking and failed with
`No space left on device`. Regenerable test executables were removed, and the focused checks above
passed afterward. A complete workspace test run remains a release gate. Planner admission, full
scope/journal projection, tenant-wide quota, worker 1.5, scoped fault tests and publication support
are still pending; no collection runtime guarantee is claimed by this checkpoint.

The candidate query was subsequently restricted to root instance paths. A real-PostgreSQL test
placed a scoped READY step beside a non-ready root step and confirmed that a protocol-1.4 worker
receives no candidate run (`old_worker_candidate_query_does_not_offer_scoped_ready_steps`, passed).

## Five-SDK loop authoring checkpoint (2026-10-15)

All five SDKs now build additive v1 `while` and `repeatUntil` documents with named bodies,
pre/post-test conditions, initial and carry expressions, required iteration and timeout limits,
optional delay and error route, and a required successor. The shared `loop-builders` scenario is
build-only; neither publication nor execution of these nodes is enabled.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: one shared scenario across TypeScript, Go, Python, C# and Java; the canonical document and validation errors match the reviewed golden |
| TypeScript SDK tests | 114 passed (supporting agent) |
| Go SDK `go test ./...` and `go vet ./...` | Passed (supporting agent) |
| Python SDK tests, strict mypy and Ruff | 245 tests passed; type and lint checks passed (supporting agent) |
| .NET SDK tests and format verification | 431 tests passed; format passed (supporting agent) |
| Java SDK tests | Passed (supporting agent) |

The production publication parser still rejects loops. The explicit structured-v1 validation
path accepts their definition mapping; these builder results are authoring evidence only.

## Loop model and structured validation checkpoint (2026-10-15)

The explicit structured v1 parser now maps `while` and `repeatUntil` to typed definition nodes,
checks expressions and closed body references, and preserves the five-SDK loop golden byte for
byte. The production publication parser still rejects these nodes. A pure domain reducer records
pre/post-test entry, sequential iteration admission, result and carry digests, the predicate
decision, delay, limit exhaustion, drain and final outcome. It replays without reevaluating a
predicate, rejects an active lease during drain, and checks iteration/deadline bounds. The
PostgreSQL event codec round-trips its eight event kinds, but storage has no loop projection or
timer/outbox transaction and explicitly refuses to persist a loop wait.

| Check | Result |
|---|---|
| `cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres --locked -q` | Passed across all package test targets, including 10 new loop reducer tests and 4 event codec tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --test integration_workflow_primitives --locked -q` | Passed: 16 contracts and 14 real-PostgreSQL integration tests at this checkpoint |
| `cargo check --workspace --all-targets --locked -q` | Passed |

The collection planner helper is private and prevalidates a bounded event batch against a cloned
`RunState`; the production planner still returns `UnsupportedStructuredNode`. It does not yet
dispatch bodies or bridge to the storage adapter. Scoped Return routing, projection restoration,
quotas, compensation and failure tests remain release gates for loops and collections.

## Scoped storage and transaction checkpoint (2026-10-15)

PostgreSQL now retains a collection entry and bounded activity admission with the addressed
journal, steps, outbox and counters in one transaction. The canonical run reader restores an open
collection and its admitted instances; `verify_run` replays addressed journal events and detects a
divergent scope projection. Journal attempt spans use the complete instance path. Legacy worker
claims and maintenance candidates remain root-only. The domain reducer accepts scoped activity
completion, fenced failure, retry and same-path routing, while the legacy transaction adapter
refuses scoped events and outputs before it can flatten their identities.

The store validates the scheduler shard lease under a row fence and reads tenant iteration
capacity from a persisted policy. Missing policy prevents entry before the owner is parked.
Admissions across runs serialize on tenant usage. A root-step trigger counts its first activation
against the lifetime run budget; historic cancelled steps remain an explicitly documented lower
bound because their prior activation cannot be reconstructed. The policy has no public management
route yet. Collection checkpoint/finish, scoped worker claims/completions, compensation and public
publication remain disabled.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 23 real-PostgreSQL tests, including transaction rollback, tenant contention and projection divergence |
| `cargo test -p sagat-domain --locked -q` | Passed: complete domain suite including scoped lifecycle, loop and declared-operation model tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 21 contracts |
| `cargo check --workspace --all-targets --locked -q` | Passed |

These results validate isolated foundations, not an executable collection or loop. The complete
workspace test, fault matrix, five-language runtime conformance and performance gates remain open.

## Approval authoring and domain checkpoint (2026-10-15)

The five SDKs now build the same additive v1 approval document in build-only mode. Their common
golden covers the default initiator exclusion, explicit opt-out, role and subject policy, bounded
timeout and the three mandatory outcome routes. The structured parser maps those documents to a
typed `ApprovalPolicy` and checks the graph; the production publication parser still rejects them.
The pure domain approval model validates current authorization and eligibility before accepting a
decision or replaying a receipt. It records a first settlement and treats losing expiry/cancel
attempts deterministically. There is no request, timer, audit, journal, outbox or API transaction
for approval yet, so this is not runtime evidence.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five SDKs matched the reviewed canonical golden; normalized SHA-256 `7da0a408466d50f813a820a55de9a2f1c23673373c53030014b581101f1e0fbc` |
| SDK suites | TypeScript 114; Python 245 plus Ruff/Mypy; .NET 431; Go and Java suites passed |
| `cargo test -p sagat-domain --locked` | Passed, including 11 approval domain tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 23 contracts, including canonical approval IR with publication gate |
| `cargo clippy -p sagat-domain -p sagat-infra-ir --all-targets --locked -- -D warnings` | Passed |

## Worker protocol capability gate (2026-10-15)

The worker adapter can negotiate minor 1.5 only when its storage implementation explicitly
declares transactional support for scoped claims and reports. PostgreSQL still declares no such
support, so it negotiates 1.4, claims only root instances and rejects scoped reports. The six
attempt-scoped RPCs validate the same structural identity; the Protobuf contract retains field
numbers and accepts legacy messages without an instance field. This gate does not prove scoped
worker execution because the PostgreSQL claim, lease and transaction paths remain root-only.

| Check | Result |
|---|---|
| `cargo test -p sagat-application --locked` | Passed: 168 tests |
| `cargo test -p sagat-api-grpc --locked` | Passed: 24 tests |
| `cargo clippy -p sagat-application -p sagat-api-grpc --all-targets --locked -- -D warnings` | Passed |

## Transactional collection and addressed-storage checkpoint (2026-10-15)

The private collection adapter now checkpoints an iteration and finishes its collection in
PostgreSQL with journal, projection, counters and outbox in the same transaction. The finish checks
that the retained aggregate array equals checkpoint values in iteration order. The canonical run
reader restores open and finished collection state, including the aggregate digest, and
`verify_run` detects a corrupted digest. The generic run transaction now has an addressed scoped
write path for a bounded set of step events and outputs; it retains full instance identity in the
journal and step projection. Scoped worker claims are still disabled, and the collection planner
is not connected to the adapter, so no end-to-end collection runtime guarantee follows.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 32 real-PostgreSQL tests, including checkpoint/finish restart, addressed storage, empty collection, corrupted snapshot/index rejection and declared-operation transactions at this checkpoint |
| `cargo check --workspace --all-targets --locked -q` | Passed |
| Clippy for application and infra-postgres all targets, plus the primitive integration target, with `-D warnings` | Passed in the collection adapter checkpoint; a combined rerun remains pending after concurrent edits |

Full fault matrix, quota and audit semantics, scoped worker
claim/report, body routing, compensation and all-five runtime conformance remain open gates.

## Continuation contract and storage boundary (2026-10-15)

The five SDKs produce a common additive-v1 continuation document in build-only mode. The
`continuation-builders` conformance scenario passed in all five languages, and their complete
language suites passed after synchronizing worker-protocol copies. A parser check found that the
state-transfer fixture needed a root state declaration; the fixture was corrected and now passes
the internal typed parser round trip. The production publication parser remains closed.

The chain migration records bounded head metadata, immutable segment identities and explicit
child-chain links. Tenant and namespace foreign keys reject cross-boundary members; deferred
membership keys prevent a head or successor from naming a run outside its chain. A real
PostgreSQL test exercised a valid initial chain, rejected a foreign head/successor, and rejected
an identity rewrite. The migration is a storage prerequisite only. No initial-run creation,
handoff, chain-target command, parent settlement, quota or journal transaction is wired yet.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives chain_head_must_be_a_segment_of_its_own_chain --locked -q` | Passed against real PostgreSQL |
| Complete SDK suites | TypeScript 114, Python 245, .NET 431, and complete Go/Java suites passed |
| `SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed in all five languages after the root-state fixture correction; typed parser round trip passed |

## Loop and approval transaction checkpoint (2026-10-15)

The private loop adapter commits return checkpoint, timer or next iteration, quotas, journal,
projection, outbox and audit together. A delayed loop has no worker lease. Its four real
PostgreSQL tests cover restart, duplicate timer fire, stale owner fence, policy reductions,
capacity limits and rollback. Terminal `Complete`/`IterationLimit` routing and canonical loop
restore are still closed; no public loop runtime is enabled.

The private approval adapter uses canonical `ApprovalOpened`, `ApprovalDecided` and
`ApprovalExpired` journal events. Opening and expiry require a current scheduler fence; a decision
checks current tenant membership and policy. First-commit decision/expiry races, replay, stale
fence, cross-tenant denial, projection recovery and late audit/outbox rollback were exercised in
seven PostgreSQL tests. Approval has no public endpoint or runtime planner wiring yet.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 44/44 against real PostgreSQL, including seven approval and four loop transaction tests |
| `cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres --locked -q` | Passed across all package test targets at this checkpoint |
| `cargo fmt --all -- --check`; `git diff --check` | Passed |

Fault tests for worker delivery, cancellation, compensation and signals against these primitives,
plus public API, telemetry, schema publication and five-language runtime conformance remain open.

## Studio unknown-node display (2026-10-15)

The Studio run card, table, details panel and accessible graph label now identify a runtime node
type absent from its registry as unsupported. Long unknown type values stay bounded on cards and
remain available through an accessible label. This is reader degradation only; it does not add
Studio authoring or prove scoped/chain graph projection.

| Check | Result |
|---|---|
| `npm test -- --run src/features/runs/RunNodeCard.test.tsx` in `frontend/apps/studio` | Passed: 8/8 |
| `npm run typecheck`; `npm run lint` in `frontend/apps/studio` | Passed |

## Scoped graph, chain links and telemetry checkpoint (2026-10-15)

The runtime graph now gives repeated instances distinct path-qualified IDs and retains their
definition node and structured path. A missing or newer node kind is rendered as unsupported.
Graph changes return a full reset when a scoped transition cannot be represented unambiguously as
a root-only delta. Chain segment links are read with the graph snapshot and shown in the Studio run
view. These read-side changes do not enable collection execution or continuation handoff.

The telemetry projector keys attempts by the full execution instance, so two iterations of one
definition node cannot share an attempt or span identity. Its metrics use bounded labels and do not
include payload values.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives one_definition_node_can_have_distinct_tenant_scoped_instance_paths --locked -q` | Passed: one real-PostgreSQL test; graph retains root and two scoped identities |
| `cargo test -p sagat-tests --test integration_workflow_primitives chain_head_must_be_a_segment_of_its_own_chain --locked -q` | Passed: one real-PostgreSQL test; graph exposes the committed chain link |
| `cargo test -p sagat-api-http --lib runtime_graph_projects_scoped_steps_and_resets_ambiguous_deltas --locked -q` | Passed: scoped IDs, paths, edges and safe reset |
| Studio `npm test -- --run src/model/runGraph.test.ts src/features/runs/RunDetailPage.test.tsx`; `npm run typecheck`; `npm run lint` | Passed: 78/78 focused tests, typecheck and lint |
| Application telemetry tests and scoped worker PostgreSQL checks reported by the implementing agent | Passed: 18/18 pure telemetry tests and 2/2 focused PostgreSQL scoped-worker tests at that checkpoint |

The chain graph link is visibility evidence only. A complete continuation must additionally prove
the handoff, command forwarding, parent settlement, quota and retention transactions.

## Scoped worker recovery checkpoint (2026-10-15)

An explicit test-only PostgreSQL store configuration exercises worker protocol 1.5 for scoped
inline activities. Its real-database tests cover distinct paths, report fencing, restart, due
retry, lost-lease recovery and journal-failure rollback. Production still negotiates 1.4 and
withholds scoped tasks. The implementing agent reported 4/4 focused PostgreSQL tests and Clippy
with `-D warnings` for domain, application, PostgreSQL, scheduler, gRPC and the integration target.

Independent review found release blockers: body-local binding and secret resolution must use the
full instance path; task input and effect-idempotency context must materialize the captured
collection item or loop carry; and scoped stale hints need reconciliation. The test-only path is
not an end-to-end collection/loop execution guarantee. The worker capability remains closed until
these checks and the broader fault/security matrix pass.

## Collection item-retention correction (2026-10-15)

Collection entry now writes the captured items' canonical bytes under their pinned digests in the
same PostgreSQL transaction as scope creation. Previously an admission could depend on a separate
caller preloading each item. An existing digest row with different bytes rejects the entry; the
append-only payload table prevents later mutation. The changed PostgreSQL test failed with a
missing row before the fix, then both focused tests passed after it. This does not yet prove full
collection scheduling, retained-run byte ceilings or garbage-collection safety.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives admitted_collection_reopens_with_exact_scope_and_verifies_from_addressed_journal --locked -q` | Passed: item retained without preloading, then admission and restore |
| `cargo test -p sagat-tests --test integration_workflow_primitives collection_payloads_require_same_tenant_and_matching_content_digests --locked -q` | Passed: digest/content mismatch rejects entry; foreign provenance remains rejected |

## Private chain, approval and declared-operation integration checkpoint (2026-10-15)

The chain store now keeps initial, continued and final segment metadata, parent settlement and a
chain-first lock for its private inspect/signal/cancel port. The private command path invokes the
canonical engine inside that transaction; cancellation and handoff cannot both win. The public
run-command path does not yet take that chain lock for every direct mutation, and scoped waits do
not yet have complete signal semantics, so chain-target HTTP/SDK commands remain closed.

An HTTP approval-decision endpoint accepts a previously opened durable request with current
eligibility, a body decision key, typed receipt, denial audit and no-store response. A public
workflow still cannot create and surface such a request through the scheduler. Query/Update have
private published-state/start hooks and an HTTP path tested with seeded declarations; public IR
publication stays closed while five-SDK runtime conformance and fault coverage remain incomplete.

| Check | Result |
|---|---|
| `cargo check --workspace --all-targets --locked -q` | Passed at this checkpoint |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 23/23 |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 79/79 against real PostgreSQL |
| `cargo test -p sagat-tests --test fault_workflow_primitives --locked -q` | Passed: 15/15 against real PostgreSQL |
| Focused chain PostgreSQL suite reported by the implementing agent | Passed: 17/17, including cancel versus handoff |
| Approval-decision HTTP PostgreSQL suite reported by the implementing agent | Passed: 4/4; the prior denial-audit race passed ten consecutive deterministic reruns |

These counts are checkpoint evidence for the private slices, not a full release gate. The
remaining suites include public workflow round trips, all-five WP-02–14 runtime scenarios,
old-client/worker upgrades, full crash/security matrix and performance ceilings.

## Bounded declared-operation publication checkpoint (2026-10-15)

The public v1 parser now accepts closed declared state and Query/Update definitions only when the
workflow uses the basic root nodes already supported by the engine. It parses and compiles the
initial-state, query, validator, state-reducer and result expressions before publication. The same
path still rejects `bodies`, top-level `limits`, `chainPolicy`, continuation and structured node
kinds, so this change does not enable chain rollover, approval, collection or loop combinations.
The internal typed parser remains available for validating those additive documents without
publishing them.

Publishing a supported document stores its immutable operation definitions with the workflow
version, and starting a run creates schema-checked initial state in the run transaction. Public
run-addressed Query/Update then use current namespace authorization, bounded declarative
evaluation, no-store responses, tenant-opaque lookup, access audit and atomic state/receipt/
journal/audit/outbox persistence. The enabled five-language scenario keeps a normal activity
unserved so the run remains mutable; it does not depend on a chain or structured node.

| Check | Result |
|---|---|
| `cargo test -p sagat-infra-ir --test declared_state --test codec --test continuation --locked` | Passed: 56 tests; basic declared state publishes, invalid expressions fail through the public parser, and chain/limits/structured combinations remain closed |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 23/23 contracts |
| `cargo test -p sagat-tests --test integration_workflow_primitives operations:: --locked -- --nocapture` | Passed: 11/11 against real PostgreSQL, including publication/start, snapshot consistency, concurrent revisions, activity race, replay and atomic rollback |
| `cargo test -p sagat-tests --test integration_workflow_primitives operations_http:: --locked -- --nocapture` | Passed: 5/5 against real HTTP and PostgreSQL, including RBAC, tenant opacity, durable rejection and injected outbox failure rollback/retry |
| `cargo test -p sagat-tests --test fault_workflow_primitives operations:: --locked -- --nocapture` | Passed: 3/3 against public HTTP and real PostgreSQL; discarded update response replays one receipt, five independent update/signal races retain both transitions, and an undeclared input canary is absent from operation responses, declared state/result payloads and non-payload evidence |
| `SAGAT_SDK_SCENARIO=state-operations SAGAT_SDK_REUSE=1 cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: one shared runtime scenario in TypeScript, Go, Python, C# and Java; runtime golden SHA-256 `ca177e4c82fc5e195705c36711c13e5289a011173002ace04fd28ace5121eecf`, workflow golden `04d6cfc485f4367c54cc13541e98d09ca4ff33caa52c5a333019b3c468fb4969` |
| Clippy for `sagat-infra-ir` all targets and the contract/integration/fault targets with `-D warnings` | Passed |

The five-language runtime checks initial query, applied update, identical receipt replay, query
after update, stale revision, validator rejection and its replay, unauthenticated access and opaque
foreign-tenant lookup. SDK retry preservation of the same update key is covered in each language's
client tests. The fault target separately discards a successful response before reading it, then
retries the exact request and observes the original receipt with one mutation. Its signal race
starts both public requests together five times and finds one state transition and one wait
transition each time. The canary case proves that an undeclared input field does not cross the
declared state/result, audit, journal, outbox or receipt-metadata boundary; it intentionally does
not claim that persisted run input is absent from the content-addressed payload store. A timer race,
secret-manager capability canaries, chain-addressed operations and the performance ceiling remain
open evidence and are not implied by this publication gate.

## Integrated command and recovery checkpoint (2026-10-15)

Direct run commands now acquire a chain lock before the run lock when the run belongs to a chain.
Cancellation latches the chain in that transaction; a terminal engine transition projects the
segment, chain and child settlement atomically. Public chain inspect, signal and cancel require an
expected head, use current authorization and return no-store responses. The HTTP/PostgreSQL tests
cover tenant opacity, signal and cancel replay, and stale-head rejection. Creation and handoff of
a chain through a published workflow are not enabled.

The test-only scoped worker path now resolves bindings and secrets by the complete instance path,
materializes captured collection items or loop carry in task input and idempotency context, and
renews hints per scoped instance. A scheduler transition can durably enter, admit, checkpoint and
finish a collection under a real shard fence; its PostgreSQL scenario covers worker completion,
restart and injected checkpoint rollback. This does not enable structured workflow publication,
automatic collection ticks or worker protocol 1.5 in production.

| Check | Result |
|---|---|
| `cargo check --workspace --all-targets --locked -q` | Passed |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 84/84 against real PostgreSQL |
| `cargo test -p sagat-tests --test contract_workflow_primitives --test fault_workflow_primitives --test contract_developer_experience --locked -q` | Passed: 23/23 primitive contracts, 18/18 fault tests and 8/8 documentation contracts |
| `cargo clippy -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-postgres -p sagat-api-http -p sagat-scheduler -p sagat-telemetry -p sagat-tests --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all -- --check`; `git diff --check` | Passed |

The outstanding release gates include automatic collection admission, scoped compensation and
reconciliation, scoped scripts and blobs, public approval creation, public continuation handoff,
and the corresponding five-language runtime scenarios. These passing checkpoint suites do not
establish those capabilities.

## Typed continuation definition checkpoint (2026-10-15)

The internal continuation parser now retains the bounded chain policy and the actual
`continueAsNew` terminal in the domain workflow definition. It no longer substitutes an ordinary
workflow end while validating the graph. Domain validation requires the terminal at the top level,
forbids a successor, error route or attempt timeout, and rejects it inside named bodies, parallel
branches and saga regions. Both transfer expressions are compiled by the bounded evaluator.

The production publication parser remains fail-closed for `chainPolicy` and `continueAsNew`, and
the execution planner still rejects the terminal as an unsupported structured node. This evidence
therefore establishes lossless typed retention only. It does not establish run initialization,
expression transfer, atomic handoff, successor scheduling or public continuation.

| Check | Result |
|---|---|
| `cargo test -p sagat-domain --test continuation_workflow --locked` | Passed: 3/3 policy, terminal-shape and region tests |
| `cargo test -p sagat-infra-ir --test continuation --locked` | Passed: 4/4 canonical retention, SDK golden and invalid-expression/graph tests |
| `cargo check --workspace --all-targets --locked` | Passed |
| `cargo clippy -p sagat-domain -p sagat-application -p sagat-infra-ir --all-targets --locked -- -D warnings` | Passed |
| Focused `rustfmt --check` for the changed domain/application/IR files; `git diff --check` | Passed |

## Approval discovery checkpoint (2026-10-15)

An authorized caller can list the approval requests owned by a run through a bounded, cursor-based
read. Admission resolves the run inside the authenticated tenant and requires both `run:read` and
`approval:read`. The PostgreSQL adapter filters by tenant, namespace and run, then verifies every
returned projection against the canonical approval journal before exposing its safe view. The
response includes the approval identifier, full scoped node identity, status and timestamps. It
does not include the eligibility policy, initiating identity, decision actor or receipt.

The endpoint returns `Cache-Control: no-store`, uses the common typed error envelope, records a
bounded read metric without identifiers and makes foreign-tenant and missing runs indistinguishable.
At this checkpoint, the public workflow parser still rejected approval nodes: discovery and
decisions were usable only for requests opened through the internal durable approval boundary.
The later approval-runtime checkpoint records the scheduler and public-runtime evidence that
removed that restriction for the reviewed root approval shape.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives approvals_http:: --locked -- --nocapture` | Passed: 5/5 against real HTTP and PostgreSQL; includes two-page traversal, invalid cursor, viewer access, worker denial, tenant opacity, no-store and disclosure canaries |
| `cargo test -p sagat-tests --test contract_developer_experience --locked` after regenerating `docs/reference/api.md` | Passed: 8/8 documentation contracts |
| `cargo clippy -p sagat-application -p sagat-infra-postgres -p sagat-api-http -p sagat-tests --test integration_workflow_primitives --locked -- -D warnings` | Passed |
| Focused `rustfmt --check`; `git diff --check` | Passed |

## Public approval runtime checkpoint (2026-10-15)

The public v1 parser now accepts the reviewed root approval shape while retaining the typed policy,
timeout and three outcome routes. It continues to reject approval combined with declared state and
does not enable approvals inside bodies, chain handoff, collection, loop or continuation shapes.
Starting a run leaves the approval idle for the scheduler, which opens the request and timer in one
fenced PostgreSQL transaction. Restart preserves the pending request. An approval, rejection or
expiry commits one canonical outcome and routes the owner step exactly once; a raw signal cannot
bypass eligibility or the approval decision boundary.

Cancelling a run either prevents a request from being opened or cancels the exact pending request
and timer in the same transaction as the canonical step cancellation. The cancellation projection
is verified against that journal event. Decision-versus-cancellation and expiry-versus-cancellation
races admit one committed outcome, while injected outbox or audit failures roll back the entire
transition. Audit and outbox records contain bounded identifiers and do not disclose the approval
policy or decision payload.

The five SDKs execute the same public scenario through the HTTP API: publish, start, list the
request, approve or reject it, replay the same decision, exercise authentication and verify tenant
opacity. The reviewed runtime golden has SHA-256
`4cd74239c4305ce39d214571f94cdded0746fd3da32ae12a25f8838bfbdfeb0e`; the workflow golden has
SHA-256 `c184b3dd7e4e9c7556ef0448e2e83f8beef99eb4579986ddfbc99d65ac235c86`.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives approval --locked -- --nocapture` | Passed: 20/20 against real HTTP and PostgreSQL, including public start, scheduler opening, restart, approval and rejection routing, expiry, raw-signal denial, cancellation races and rollback |
| `cargo test -p sagat-tests --test fault_workflow_primitives approval --locked -- --nocapture` | Passed: 7/7 fault tests |
| `cargo test -p sagat-infra-ir --locked`; `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: parser/codec suites and 23/23 primitive contracts |
| `SAGAT_SDK_SCENARIO=approval-decisions SAGAT_SDK_REUSE=1 cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: one common runtime scenario across TypeScript, Go, Python, C# and Java, both when reviewing the generated golden and on a no-bless replay |
| Focused Clippy with `-D warnings`; `cargo fmt --all -- --check`; `git diff --check` | Passed |

This checkpoint enables only the reviewed root approval shape. Approval combined with declared
state, approvals in named bodies and chain-addressed approval execution remain closed.

## Automatic collection tick checkpoint (2026-10-15)

The scheduler now discovers open collection scopes for runs in `RUNNING` or `WAITING`, selects
them through its existing fair shard pass and advances one durable transition per candidate.
Admission rechecks tenant quota and the owner fence under lock. Captured items and results retain
their digests; restart and lost broker hints resume from PostgreSQL. Finishing a collection with
an `Activity` successor releases that successor with its journal and outbox in the same
transaction. Other successor kinds are still withheld from publication until their normal engine
path is integrated. Structured workflow publication and production worker 1.5 remain closed.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 86/86 against real PostgreSQL, including automatic admission, quota contention, restart, rollback and lost-hint recovery |
| Scoped worker PostgreSQL filter | Passed: 7/7 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, as reported by the implementing agent |
| `git diff --check` | Passed |

Collection failure/cancellation drain, nested scopes, compensation/reconciliation reservations,
scoped scripts and blobs, all successor kinds, and five-language runtime execution are still
release gates. The passing internal tick does not authorize publishing a collection workflow.

## Derived reservation and tenant contention checkpoint (2026-10-15)

Collection admission now reads the pinned workflow definition and verifies the owner, body,
entrypoint and activity before calculating the exact reservation for derived instances. A caller
cannot under-reserve capacity to bypass tenant limits. The admission and its quota charge remain
one PostgreSQL transaction; an insufficient reservation leaves no iteration or charge. Two runs
from the same tenant contend for the same slot budget, including after a policy reduction.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives security:: --locked -q` | Passed: 3/3 real-PostgreSQL tests for tenant isolation, cross-run slot contention and derived reservation rejection |
| Focused Clippy for PostgreSQL and integration targets with `-D warnings`; `git diff --check` | Passed, as reported by the implementing agent |

This verifies admission for the tested collection shape. Nested bodies, compensation and
reconciliation settlement, retained-byte ceilings and full production publication remain open.

## Combined primitive regression checkpoint (2026-10-15)

After integrating the public approval scheduler and the private chain handoff path, the complete
primitive targets were rerun on the same real PostgreSQL harness. A green aggregate verifies the
current tested slices together; it does not open collection, loop or continuation publication.

| Check | Result |
|---|---|
| `cargo check --workspace --all-targets --locked -q` | Passed |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -q` | Passed: 97/97 against real PostgreSQL |
| `cargo test -p sagat-tests --test fault_workflow_primitives --locked -q` | Passed: 18/18 |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked -q` | Passed: 23/23 |

This checkpoint predates any public chain creation or scoped production-worker capability. Those
gates, the remaining primitive fault matrix and measured capacity evidence remain open.

## Public bounded continuation checkpoint (2026-10-15)

The public v1 parser now accepts two closed continuation graphs. Both start with one root activity.
The first routes directly to an input-only `continueAsNew`; the second routes through one condition
that selects either the input-only continuation or a terminal `end` without output. Publication
still rejects declared-state transfer, bodies, top-level limits, triggers, `onError`, non-activity
entrypoints and wider graphs.

Starting either shape creates the initial run, chain row, first segment, journal and outbox in one
transaction. The original chain deadline derives from `chainPolicy.timeoutMs` and cannot be renewed
by a successor. A fair scheduler pass discovers a pending boundary at the current head, evaluates
the selected input outside the write transaction, then rechecks the chain, run, scheduler fence and
payload under the canonical handoff locks. The predecessor completion, successor run and task,
lineage, head revision, audit and outbox commit together.

When the current segment already consumes `maxSegments`, the scheduler uses the normal engine and
run transaction to persist `RunStopRequested(FAILED, CHAIN_LIMIT)`, drain the continuation boundary,
finish the run and project the segment and chain as `FINAL`. One audit record explains the limit.
A later tick cannot rediscover the finalized chain. The conditional shape also proves a successful
second segment reaches `SUCCEEDED` and latches the chain outcome without another handoff.

| Check | Result |
|---|---|
| `cargo test -p sagat-infra-ir --locked --test continuation -- --nocapture` | Passed: 7/7, including both public shapes and fail-closed combinations |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked chain_handoff -- --nocapture` with real PostgreSQL | Passed: 4/4 for prepared atomic handoff/replay/rollback, public handoff, conditional `SUCCEEDED`/`FINAL`, and terminal `CHAIN_LIMIT` with zero rediscovery |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked chains_http -- --nocapture` with real PostgreSQL | Passed: 3/3 for atomic public start, tenant-scoped inspect/signal replay and cancellation CAS/replay |
| `cargo clippy -p sagat-infra-ir -p sagat-scheduler -p sagat-infra-postgres --all-targets --locked -- -D warnings` | Passed |

The five-SDK chain-command fixture stays pending because it needs a signal-controlled `waitEvent`
graph, which is outside this publication subset. State transfer, child-chain creation, chain-level
Query/Update and all other continuation graph combinations remain closed until their own runtime
and conformance evidence exists.

## Restricted public collection checkpoint (2026-10-15)

The public v1 parser accepts a root chain of `forEach` nodes ending at `end` when every referenced
body contains exactly one activity followed by one `return`. Broader bodies, nested collections,
triggers and combinations with other structured nodes remain closed. Starting the workflow parks
the owner for the fenced scheduler. Collection admission stays fail-closed until an administrator
sets the tenant-wide `iteration_concurrency` quota through the public governance route. The setting
is audited and shares a conservative default operator ceiling of 1,024 active iterations across
collections and loops; an operator can explicitly grant a higher ceiling.

The public PostgreSQL round trip publishes and starts `lines -> taxes -> end`, provisions capacity
and the worker binding through authorized HTTP routes, and executes two items in each body through
worker protocol 1.5. Each scope checkpoints results in input order. The final transition commits
the second `collection.finished`, terminal `end`, `run.finished`, event outbox, telemetry head and
run projection together. An injected failure on the terminal outbox rolls the whole transition
back; a later scheduler tick retries it and canonical replay matches the stored projection.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked collections_http:: -- --test-threads=1 --nocapture` | Passed: 3/3 against real HTTP, gRPC and PostgreSQL, including missing policy, RBAC/audit, two chained collections, terminal rollback/retry and replay |
| `cargo test -p sagat-infra-ir --test codec --locked` | Passed: 49/49 parser and canonicalization tests |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | Passed: 25/25, with the public loop contract still ignored |
| `cargo test -p sagat-domain --test collection_reducer --locked`; `cargo test -p sagat-domain quota --locked` | Passed: 14/14 collection reducer tests and 10/10 quota tests |
| `cargo clippy -p sagat-domain -p sagat-infra-ir -p sagat-infra-postgres --all-targets --locked -- -D warnings`; formatting and diff checks | Passed |

The shared five-language scoped-worker runtime scenario remains a separate compatibility gate at
this checkpoint. Collection bodies with more than one activity, nested scopes, collection-local
compensation/reconciliation, scripts, blobs and combinations with other structured nodes remain
unpublished.

## Worker and chain checks (2026-10-15)

The public collection transaction and two-collection chain are covered in the preceding checkpoint.
Separate worker checks verify scoped cancellation, heartbeat, acknowledgement and restart, along
with tenant, secret and blob boundaries. The collection scenarios do not yet cover 17 items at
concurrency three, nested bodies, compensation or all-language collection runtime.

| Check | Result |
|---|---|
| Focused scoped-worker integration filters on the same Colima PostgreSQL | Passed: 2/2 for cancellation/heartbeat/ack/journal/restart and tenant/secrets/blob isolation |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked chains:: -- --nocapture` | Passed: 4/4 for handoff/cancel serialization, restart after a lost hint, limit-finish rollback/retry and workflow-version pinning |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock SAGAT_SDK_SCENARIO=protocol-negotiation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five SDKs against the reviewed protocol-1.5 golden; protocol 1 and 1.4 retain root behavior |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock SAGAT_SDK_SCENARIO=chain-inspect-cancel cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five SDKs against the reviewed public inspect/cancel golden |

The first protocol run failed because every SDK emitted an absent legacy instance as JSON `null`
while the golden required omission. The runners were corrected to omit it for workers through 1.4;
the reviewed 1.5 golden requires the root instance object. The first chain run failed only because
its runtime golden did not yet exist; equal five-language observations were inspected before that
golden was added and the no-bless rerun passed. Neither run is counted as a pass before its rerun.

Loop entry and restoration are separate private slices: `loop_progress::` passed 11/11 real
PostgreSQL tests, including restart, terminal routing and canonical projection verification.
Publication, timeout/cancel/drain, chain-target signals and the full fault and capacity matrix
remain open. The aggregate integration run after these changes passed 113/116 and exposed three
regressions in old-schema quota lookup and a pre-terminal test expectation. Each of those three
cases passed on its focused rerun after correction. The complete aggregate rerun then passed
116/116 against real PostgreSQL (184.20 seconds, one test thread).

## Collection capacity, recovery and drain checks (2026-10-15)

The public collection fault fixture processed 17 items with `maxConcurrency: 3`. It marked the
first three task hints delivered without broker delivery, expired the scheduler lease, then used
durable polling after restart. It completed each admitted batch in reverse order and verified the
final aggregate in input order, 17 distinct instance identities, at most three active iterations,
terminal `SUCCEEDED` and canonical run replay. Recovery did not depend on a duplicate hint.

The drain transaction now records cancellation requests for leased/running members with the
collection transition. Focused tests injected rollback at the cancellation outbox and member
journal, exhausted a retry into fail-fast, and verified that failed derived compensation or
reconciliation obligations retain slots and reservations. These checks do not prove successful
scoped compensation or reconciliation; the restricted public body still excludes them.

| Check | Result |
|---|---|
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked collections:: -- --nocapture` | Passed: 1/1 real PostgreSQL 17-item/restart/lost-hint scenario |
| Focused `scoped_worker::` drain tests against real PostgreSQL | Passed: cancellation rollback/restart, fail-fast, and retention under failed derived obligations |
| Focused legacy-upgrade and collection terminal replay regressions | Passed: 3/3 against real PostgreSQL after the old-schema quota query and terminal assertion corrections |

## Scoped worker item and identity conformance (2026-10-15)

The worker protocol 1.5 lease contains a typed execution path and a pinned collection item and
index. Each of the five SDK hosts now exposes that `item/index` pair to the activity handler while
preserving the existing root input argument. A worker negotiated at 1.4 has neither the scoped
identity nor the pair. The shared scenario executes `same`, `same`, `different` through three
logical paths. One retry per path proves stable path, item and idempotency key within an instance,
distinct keys between instances, and a terminal scoped graph; the old worker executes a separate
root task and cannot lease the scoped tasks. The two goldens were inspected before the no-bless run.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=scoped-identity cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` with real PostgreSQL | Passed: all five SDKs against reviewed runtime and workflow goldens, no bless; runtime SHA-256 `b89c63513a56839d412b1b62ca1b2abf990b7af59f565afb8553116d896e26d9`, workflow SHA-256 `b0a307dc9ecb871a122937055d25fcfb772a4910b78346a42500b1a61beb5318` |
| SDK unit suites | TypeScript 122/122, Python 253/253, .NET 442/442, Go `go test ./...` and Java Maven passed; Python focused Ruff and mypy also passed |
| Rust conformance harness check, Clippy, format, JSON and diff checks | Passed |

## Aggregate regression and retention quota checkpoint (2026-10-15)

The real-PostgreSQL aggregate integration suite passed **120/120** with one test thread after
the collection retention-quota change. The two new cases cover quota denial before collection
entry and while a scope is open. The former ends the owner without creating a scope; the latter
commits stop, drain, member cancellation, terminal failure and capacity release. Restart and
canonical projection verification match the journal, and duplicate receipts do not add events.
The aggregate fault suite passed **23/23** with one test thread, including the 17-item collection
recovery case and loop timer races. These results do not close the remaining collection or loop
release gates in `tasks.md`.

| Check | Result |
|---|---|
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test integration_workflow_primitives --locked -- --test-threads=1` | Passed: 120/120, 196.41 seconds |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked -- --test-threads=1` | Passed: 23/23, 52.43 seconds |
| `python3 scripts/check-architecture.py`; `python3 -m unittest discover -s scripts/tests -q` | Passed: 22 workspace crates and 8 architecture-script tests |

## Transactional collection payload and chain Query checkpoint (2026-10-15)

Collection snapshot, member item, checkpoint result and aggregate bytes now enter the same
PostgreSQL transaction as their addressed transition. A stale scheduler fence or run revision,
outbox or audit failure, and quota denial leave no new payload or retained-byte charge. A
matching replay returns the previous transition without writing bytes or audit again. Retention
quota stops are audited as `run.storage_quota_exceeded`; the drain's telemetry head advances with
the stop and cancellation events. The focused PostgreSQL transaction test passed 1/1, and the
quota/audit tests passed 2/2. The earlier 120/120 and 23/23 aggregate runs preceded this
transactional-retention change and must be rerun before a release claim.

An explicit chain Query endpoint resolves the current head under a chain lock before taking its
run snapshot. It returns the actual run ID and an optional `expectedRunId` rejects a stale head
with `CHAIN_HEAD_CHANGED`. The PostgreSQL HTTP/security and deterministic Query-vs-handoff race
tests passed 2/2. Chain Update remains unavailable. The run/chain Query contract and operator
guide document this boundary. The tests seed chains with declared state directly; publication of
one workflow combining declared state and continuation remains closed, so this is not yet a public
end-to-end Query workflow.

| Check | Result |
|---|---|
| Focused collection payload transition test against real PostgreSQL | Passed: 1/1 for four payload paths, fence/revision conflict, outbox/audit/quota rollback and replay |
| Focused retention quota and audit tests against real PostgreSQL | Passed: 2/2 for pre-scope stop, open-scope drain, terminal replay, one audit and telemetry head |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock cargo test -p sagat-tests --test integration_workflow_primitives chain_query_ --locked -- --nocapture --test-threads=1` | Passed: 2/2, 123 filtered; tenant/expected-head/no-store/audit and a handoff lock race |

## Consolidated integration rerun (2026-10-15)

The first consolidated run after these changes passed 124/126 and found two regressions. A
corrupted stored digest returned `DuplicateMismatch` instead of `InvalidTransition`; the payload
verifier now separates storage corruption from a conflicting supplied body. An older capacity
fixture acquired scheduler authority twice; if two generated runs shared a shard, the second
acquisition invalidated the first. The fixture now reuses the same generation for the same owner.
Both cases passed isolated PostgreSQL reruns, then the complete integration target passed
**126/126** with one test thread in 200.32 seconds. The timeout case was then extended to assert
restart and terminal `RUN_TIMED_OUT` and passed its focused PostgreSQL rerun. The consolidated
fault target was rerun after the loop drain, transactional payload and chain Query additions and
passed **23/23** in 49.16 seconds.

An initial loop-drain test attempt did not reach PostgreSQL because container creation returned
`meta.db: read-only file system`; the shared fixture poisoned the remaining cases in that attempt.
The Colima daemon and VM filesystem were inspected, a cached PostgreSQL container launched in
isolation, and each affected drain case then passed against real PostgreSQL. The failed attempt is
not counted as test evidence. Loop publication remains closed because carry/result payload
retention still precedes the progress transaction and a revision/cancellation race can retain
unreferenced bytes.

| Check | Result |
|---|---|
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test integration_workflow_primitives --locked -- --test-threads=1` | Passed: 126/126, 200.32 seconds, after both focused regressions passed |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | Passed: 26/26; the public loop contract remains separately gated |
| Focused loop drain PostgreSQL cases | Passed: 4/4 for expression failure/restart, timeout/stale fire/terminal restart, cancellation/restart and outbox rollback |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked -- --test-threads=1` | Passed: 23/23, 49.16 seconds |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock SAGAT_SDK_SCENARIO=scoped-identity cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five SDKs, no golden bless, after transactional collection payload change (52.41 seconds) |

The locked workspace check across all targets and Clippy across all targets with `-D warnings`
passed. `cargo test --workspace --lib --locked` passed, with the repository's intentionally ignored
native-container cases still ignored. An unrestricted `cargo test --workspace --locked` was stopped
during parallel test-binary compilation when the host had only 347 MiB free; it did not produce a
test result. Regenerable `target/debug/deps` performance/e2e build artifacts were removed, restoring
space, and the bounded workspace library run was used instead. The full all-target workspace test
gate therefore remains open.

| Check | Result |
|---|---|
| `cargo check --workspace --all-targets --locked` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock CARGO_BUILD_JOBS=2 cargo test --workspace --lib --locked` | Passed; intentionally ignored native-container tests were not counted |
| `cargo fmt --all -- --check`; `git diff --check`; `python3 scripts/check-architecture.py`; `python3 -m unittest discover -s scripts/tests -q` | Passed |

### Review follow-up: chain Query and collection invariants

The final review found and corrected three gaps in the restricted paths. A chain-addressed Query
with a stale expected head now returns `409` with `code=CONFLICT`; its audit targets the resolved
run under the chain lock, so predecessor and successor reads have distinct records. A same-tenant
Viewer has `run:read` but no `run:query` and receives `403` with `no-store` and no Query audit.
The focused chain Query PostgreSQL target passed 2/2 after these changes.

Collection entry, admission, checkpoint and member drain now advance `telemetry_pending.head_seq`
in the same transaction as the journal. A storage-quota drain requires an engine audit for the
same tenant, run and action before the first transition; invalid evidence is rejected without
changing the run. The real PostgreSQL collection transaction target passed 11/11, including
entry/admission/checkpoint telemetry checks. The aggregate storage-quota rollback/replay target
passed 1/1 with missing and wrong-action/resource/actor audit rejections. The cancellation/drain
telemetry PostgreSQL target also passed 1/1. Two focused PostgreSQL loop tests passed for both
`LEASED` cancellation and `RUNNING` timeout: the transactional drain writes
`step.cancel_requested`, the scoped heartbeat observes it, the worker acknowledges it, and
restart/replay releases iteration capacity. The `RUNNING` case also exposed a scoped renewal that
incorrectly entered the root fast path in `PgRunTx::persist_addressed`; that selection was fixed
and covered by the timeout regression. Loop carry/result bytes are now retained in the progress
transaction after fence, replay, revision and quota checks, with no scheduler-side pre-retention.
Three focused PostgreSQL tests passed for entry commit/replay, stale fence/iteration/budget/content
quota/outbox rollback without orphan carry, and checkpoint outbox/audit rollback without orphan
result or carry. The application loop unit target passed 2/2 and the affected crates passed
Clippy `-D warnings`. Broader public loop and release gates remain open.

Chain-addressed declared Update now accepts optional `expectedRunId` and hashes the stable chain
address, request content, expected state revision and explicit head precondition, never the
resolved head. Under the chain-before-run lock it finds a prior receipt before comparing the
current head, so an exact retry after handoff returns the original applied/rejected receipt;
a new key with a stale explicit head is rejected. The focused HTTP/PostgreSQL test passed 1/1 for
RBAC, tenant opacity, `no-store`, lock serialization, applied and rejected receipts, handoff,
replay, key mismatch, stale-head CAS, successor application and run-targeted audit.
The complete `operations_http::` PostgreSQL regression then passed 7/7, including the existing
run-addressed Query/Update paths and the chain paths.

The shared `state-operations` SDK conformance target passed 1/1 against real PostgreSQL in 18.01
seconds after a reviewed normalized golden was installed, then rerun without blessing. TypeScript,
Go, Python, C# and Java produced the same normalized output: one dropped Update response, decimal
string `19.99`, exact UTC value `2026-10-15T15:34:56.789Z`, and `replayed:true` on retry. The
five SDK workflow builders also matched the reviewed canonical workflow golden. Their local unit
suites passed (TypeScript 124, Python 255, C# 444, Java 230, and Go `./...`). Chain Query
conformance remains separately gated because public state-plus-continuation publication is not
yet enabled.

The journal-derived projection target passed 3/3 in PostgreSQL: graph deltas preserve exact
scoped identities, selected switch routes follow persisted decisions, and continuation chain
links agree between graph and telemetry. The scoped attempt span now carries both
`sagat.instance.path` and bounded `sagat.scope.depth`. The telemetry derivation unit target
passed 20/20, graph DTO tests 3/3, and affected application/API/PostgreSQL crates and the
integration target passed Clippy with `-D warnings`. The Studio already has an explicit
unsupported-node fallback and its existing test; no new authoring behavior is claimed.

## Public loop discovery and aggregate verification (2026-10-15)

The restricted public `while`/`repeatUntil` body now advances after a scoped activity finishes.
An admitted iteration has `return_node=NULL` until checkpoint; candidate discovery instead reads
the single exact `return` node from the pinned immutable body definition. A PostgreSQL regression
asserts that pre-checkpoint projection and pending return, then completes both loop forms across a
scheduler restart. It also injects terminal outbox rollback and verifies retry. Publication still
rejects unproven body shapes. These tests cover the restricted public worker path, not arbitrary
nested bodies or all-five SDK runtime parity.

The first aggregate fault run passed 22/23. Its collection restart fixture created a different
scheduler owner after expiring the old leases; the new owner could lack the run's shard, so the
first checkpoint tick did no work. Recreating the same owner on restart restored the intended
lease takeover without changing production behavior. That case passed in isolation and the full
fault suite then passed 23/23. The complete PostgreSQL integration suite passed 135/135 with one
test thread after the public loop fix. Contract tests passed 26/26. The locked workspace check,
workspace Clippy with `-D warnings`, workspace library tests, format, architecture checks and
`git diff --check` passed. At this checkpoint the feature-specific E2E target was still empty,
and the remaining task matrix was open; these aggregate runs are not a full release claim. The
following E2E checkpoint fills that target with a real public scenario.

| Check | Result |
|---|---|
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test integration_workflow_primitives --locked -- --test-threads=1` | Passed: 135/135, 215.78 seconds |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked -- --test-threads=1` | Passed: 23/23, 49.74 seconds after the fixture correction |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | Passed: 26/26 |
| `cargo check --workspace --all-targets --locked`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --lib --locked` | Passed |
| `cargo fmt --all -- --check`; `python3 scripts/check-architecture.py`; `python3 -m unittest discover -s scripts/tests -q`; `git diff --check` | Passed |

## Feature E2E harness (2026-10-15)

The previously empty `e2e_workflow_primitives` target now runs a real PostgreSQL-backed HTTP
scenario. It creates and publishes a switch workflow through the public API, starts positive,
default and invalid-predicate runs, and reads each committed graph without a sleep or a mocked
platform. Both valid routes finish and the invalid predicate fails without selecting either
terminal route. The target passed 1/1 with no ignored or filtered cases. This proves the harness
and this route scenario, not the remaining structured-primitive E2E matrix.

| Check | Result |
|---|---|
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test e2e_workflow_primitives --locked -- --nocapture` | Passed: 1/1 real HTTP/PostgreSQL scenario |

## Five-SDK restricted loop runtime (2026-10-15)

The shared `loop-runtime` scenario builds the same additive-v1 document in TypeScript, Go, Python,
C# and Java. Each language publishes a restricted `while` followed by `repeatUntil`, starts a
worker negotiated at 1.5, completes one activity/return iteration in each loop and observes a
terminal `SUCCEEDED` graph. The golden distinguishes `pre[0]/work` from `post[0]/work` and records
two different worker activities. The TypeScript document and normalized PostgreSQL observation
were inspected before the two goldens were retained; the five-language run then passed without
blessing. The existing `loop-builders` scenario still covers broader authoring only and also
passed in all five languages. This runtime case does not prove delay, error routing, nested body
graphs or all loop fault orders.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five builders |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock SAGAT_SDK_SCENARIO=loop-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five SDKs against reviewed runtime and workflow goldens, no bless; runtime SHA-256 `40fb51face6671645bce5aae420c96bfd167ca1e51c45e9e76cc08e7c77298ed`, workflow SHA-256 `0d5c7b2e7c9c225e77ce565242e4bd9db7a95c36ec9be5caf4a4ba45596f077e` |

## Mapped collection result and five-SDK runtime (2026-10-15)

The first shared `map` runtime attempt timed out. PostgreSQL showed that both return nodes had
checkpointed but the owner remained `WAITING`: the finish adapter required the aggregate to equal
the raw checkpoint array, so a legitimate `bodyResult.amount` mapping was rejected. The finish
command now binds the scheduler-evaluated aggregate to the ordered checkpoint source digests. The
same transaction verifies each source index/digest, retains the aggregate and writes the terminal
journal, projection, audit and outbox. The identity mapping still requires exact raw-array equality.
The public two-scope PostgreSQL test first reproduced the failure, then passed with a mapped
second scope and asserted `[1,2]` from durable storage; a wrong source digest is rejected. The
first five-SDK run with automatic worker transport differed in attempt count: TypeScript and
Python received their second task after a stream lifetime and retry, while Go, C# and Java claimed
attempt 1. The semantic scenario now selects supported polling transport in all five SDKs. It
passed without blessing against reviewed goldens, with two distinct scoped members and one
terminal graph. Stream timing remains a separate transport behavior, not mapped-result parity.

| Check | Result |
|---|---|
| Focused public PostgreSQL mapped collection and source-digest tests | Passed: 2/2 after the failing mapped-finish reproduction |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock SAGAT_SDK_SCENARIO=collection-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: all five SDKs, no bless; runtime SHA-256 `3ee04e2567bd92d95d720eec7ad2d15fd6ded53eca7d6c6718b17ab7df53f8db`, workflow SHA-256 `b12d8efe915d2cea7fed18cefc81f321806e9a537c9308d296842bac45664ce5` |

The complete PostgreSQL integration target passed 135/135 in 222.94 seconds after the mapped
aggregate change. The post-change fault target passed 23/23 in 49.40 seconds. Contract tests
passed 26/26 and the locked workspace library suite passed. Workspace Clippy with `-D warnings`,
formatting, architecture and diff checks passed.

| Check | Result |
|---|---|
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test integration_workflow_primitives --locked -- --test-threads=1` | Passed: 135/135, 222.94 seconds after mapped-result change |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked -- --test-threads=1` | Passed: 23/23, 49.40 seconds after mapped-result change |

## Direct declared-state continuation (2026-10-15)

The public parser now accepts only the direct `activity -> continueAsNew` form when a workflow
declares state, and requires the continuation's state expression to be exactly `state`. It still
rejects missing or alternate state transfer and conditional stateful graphs. In the handoff
transaction PostgreSQL verifies the selected state digest, exact bytes, size and expected revision
against the locked predecessor snapshot, then copies its digest, schema, size and revision to the
successor before advancing the chain head. The public PostgreSQL test publishes the document,
starts it, completes a worker task, advances it with the fenced scheduler, checks the successor's
declared state, resolves a chain Query snapshot to that successor and evaluates the declared query
to `19`. Its successor input contains a different account value, so the observed state is
specifically the transferred snapshot.

The first shared `chain-query` attempt published successfully but returned
`OPERATION_EVALUATION_FAILED`: operation evaluation reparsed the pinned definition through a
state-only gate that rejected `chainPolicy`. The operation reader now uses the same restricted
published continuation parser, preserving the definition digest and Query disclosure rules. The
TypeScript normalized observation was reviewed: initial run `RUNNING`, pending activity, Query
result `"19.99"`, run version 3 and state revision 0. The reviewed scenario also starts another
chain and receives `409 CHAIN_HEAD_CHANGED` when its run ID is used as the first chain's
`expectedRunId`. All five SDKs then matched the reviewed workflow and runtime goldens without
blessing. This proves public initial-head chain Query, not
Query after rollover or chain Update parity.

Focused parser tests passed 8/8; the focused public PostgreSQL handoff test passed 1/1. The
complete PostgreSQL integration target passed 136/136 in 220.88 seconds before the operation
reader adjustment; the all-five Query scenario and subsequent focused parser tests exercised
that adjustment. Updated state revisions and every handoff fault order remain open. The remaining
validation for this increment is recorded below.

| Check | Result |
|---|---|
| `cargo test -p sagat-infra-ir --locked` | Passed after updating stale publication assertions; parser acceptance/rejection and operation reparse included |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock SAGAT_SDK_SCENARIO=chain-query cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` | Passed: five SDKs without bless; runtime SHA-256 `f4df508341e60e92f7b1f3bd946a50895acda94857eaa11e1d9718520b919bbe`, workflow SHA-256 `428b7640ff6fc47bf9eb21eae08cd2b7ff05047ead35a89e0d254c4f5bf22c5f` |
| `DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE=/var/run/docker.sock cargo test -p sagat-tests --test fault_workflow_primitives --locked -- --test-threads=1` | Passed: 23/23, 51.69 seconds |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | Passed: 26/26 after updating a stale rejection assertion |
| `cargo clippy --workspace --all-targets --locked -- -D warnings`; `python3 scripts/check-architecture.py`; `git diff --check` | Passed |
| `cargo fmt --all -- --check`; `cargo test -p sagat-tests --test contract_developer_experience --locked` | Passed after regenerating the schema-derived workflow reference; 8/8 documentation contract tests |
| Focused public stateful handoff after the operation-reader change | Passed: 1/1 with real PostgreSQL, fenced scheduler and Query evaluation against the successor snapshot |

## Public chain Query and Update after handoff (2026-10-15)

The shared `chain-query-handoff` scenario publishes a direct stateful continuation through each
SDK. A protocol 1.5 worker completes the predecessor, and the fenced scheduler commits the
successor. The successor input contains `0.00`, but its declared state Query returns the
transferred `19.99`. Chain Update changes that state to `24.99` at revision 1; an exact retry of
the same key returns the original receipt with `replayed: true` and no second revision. Query
then returns `24.99` at revision 1, while an Update and a Query fenced to the predecessor each
return `409 CHAIN_HEAD_CHANGED`. The TypeScript runtime and workflow goldens were inspected before
running the same scenario without blessing in all five SDKs against real PostgreSQL.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=chain-query-handoff cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` with Colima `DOCKER_HOST` and `TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE` | Passed: five SDKs, 35.78 seconds; no golden blessing |
| Runtime golden SHA-256 | `ea2e0d6f095a1a47a9975854acdb47bb1f001581d2176c0fe6be0eedd59c503d` |
| Workflow golden SHA-256 | `da48fc543b51697127b6d6c301df18d4615570ab82a22be9a238a48ea42c0b60` |

This establishes the public successor happy path, replay and stale Query/Update fences. It does not
close the remaining crash, concurrent command and fault-order matrix or the broader workflow
shapes still listed in `tasks.md`.

Go `go test ./...`, Python `pytest` (255 tests), Ruff and Mypy, C# `dotnet test` (444 tests),
Java `mvn test`, Rust formatting/Clippy/architecture and the eight documentation contract tests
passed. TypeScript `npm test` twice timed out in the pre-existing worker cancellation test while
other suites ran concurrently; the isolated test passed, and `node --test --test-concurrency=1`
passed 124/124. No chain client test or conformance assertion failed.
