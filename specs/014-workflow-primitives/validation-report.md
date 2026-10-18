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

At this checkpoint the five-SDK chain-command fixture stayed pending because it needed a
signal-controlled `waitEvent` graph. A later checkpoint below enables the activity-entry form.
State transfer, child-chain creation, chain-level
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
return `409 CHAIN_HEAD_CHANGED`. `waitForCompletion` returns the terminal predecessor segment;
an early `waitForChainCompletion` returns `WAIT_TIMEOUT` without changing the chain. A public
`cancelChain` then latches `CANCELLED`, which a second chain wait returns with the successor head.
The TypeScript runtime and workflow goldens were inspected before
running the same scenario without blessing in all five SDKs against real PostgreSQL.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=chain-query-handoff cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` with Colima `DOCKER_HOST` and `TESTCONTAINERS_DOCKER_SOCKET_OVERRIDE` | Passed: five SDKs, 35.60 seconds; no golden blessing |
| Runtime golden SHA-256 | `6874f91ffaf39e1a4972a7caea7e7129bcf7be4660f76dc1407867fab732461b` |
| Workflow golden SHA-256 | `da48fc543b51697127b6d6c301df18d4615570ab82a22be9a238a48ea42c0b60` |

This establishes the public successor happy path, replay, stale Query/Update fences and the
segment-versus-chain completion distinction. It does not
close the remaining crash, concurrent command and fault-order matrix or the broader workflow
shapes still listed in `tasks.md`.

Go `go test ./...`, Python `pytest` (255 tests), Ruff and Mypy, C# `dotnet test` (444 tests),
Java `mvn test`, Rust formatting/Clippy/architecture and the eight documentation contract tests
passed. TypeScript `npm test` twice timed out in the pre-existing worker cancellation test while
other suites ran concurrently; the isolated test passed, and `node --test --test-concurrency=1`
passed 124/124. No chain client test or conformance assertion failed.

## Final chain outcome and transfer expression failure (2026-10-15)

The `chain-final-success` scenario publishes the supported input-only conditional chain, completes
two activity segments through worker protocol 1.5 and the fenced scheduler, and receives
`SUCCEEDED` from `waitForChainCompletion` with a two-link lineage. The old run's
`waitForCompletion` still returns its own terminal segment. The reviewed TypeScript golden matched
Go, Python, C# and Java without blessing.

Investigation of a continued segment left at `PENDING` found a deterministic transfer-expression
failure: the sample successor had no `input.next` for its next unconditional handoff. Before the
fix, a real PostgreSQL regression test recorded `failed: 1` on the scheduler tick while the chain
remained running and the boundary stayed eligible for rediscovery. The scheduler now reacquires
the chain and run locks, verifies the head, revision, run version and shard fence, then commits
`EXPRESSION_ERROR`, `FAILED`/`FINAL`,
audit and journal in one transaction. The test now passes and a second tick rediscovers nothing.
The counter `scheduler_continuation_expression_failures_total` has no tenant, input or secret
label; the audit reason contains no expression value.

| Check | Result |
|---|---|
| `SAGAT_SDK_SCENARIO=chain-final-success cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` with Colima Docker | Passed: five SDKs, 26.00 seconds before and 25.13 seconds after the scheduler fix; no golden blessing |
| Runtime golden SHA-256 | `1078065be1da0a3d5e5962cbc553a6a23b9daa55e66eff169c16defef82290cb` |
| Workflow golden SHA-256 | `0a78ece6f78b1824d987d9237dcfff7bdc468f0b14a6f834c91ab324a7ecd63e` |
| Focused real-PostgreSQL `invalid_continuation_input_fails_the_chain_once` | Failed before the fix; passed after, including one typed failure event, one audit and no rediscovery |
| Full real-PostgreSQL `integration_workflow_primitives` target | Passed: 137/137, 232.18 seconds after the scheduler fix |
| Focused real-PostgreSQL `continuation_expression_failure_rolls_back_and_retry_finalizes_once` | Passed: injected `chain.finished` outbox failure rolled back run, journal and audit; retry finalized once |
| Full real-PostgreSQL `fault_workflow_primitives` target | Passed: 24/24, 53.19 seconds after the scheduler fix |
| Workflow-primitive and documentation contract targets | Passed: 26/26 and 8/8 after the fix |
| Rust formatting, workspace Clippy with denied warnings, architecture and diff checks | Passed after the fix |

At that checkpoint, the active shape still excluded declared state combined with a conditional
graph, child chains,
named bodies and other combinations listed in `tasks.md`. This validation does not close their
fault/race or performance gates.

## Conditional declared-state continuation (2026-10-15)

The public parser now accepts the closed four-node graph `activity -> condition ->
continueAsNew | end` when the continuation transfers the exact declared `state` snapshot. It
still rejects a missing or alternate state expression, extra nodes, bodies, triggers and other
unproven combinations. A real PostgreSQL test published the workflow, handed off from the first
segment, proved the successor retained state `19` although its input declared `0`, then completed
the successor with `SUCCEEDED`/`FINAL` and no further handoff. The reviewed shared SDK scenario
also reached a final head and queried `19.99` from it despite `0.00` in the successor input.

| Check | Result |
|---|---|
| `cargo test -p sagat-infra-ir --test continuation --locked` | Passed: 9/9, including conditional state transfer accepted and missing/alternate transfer rejected |
| Focused real-PostgreSQL `conditional_stateful_continuation_transfers_state_and_finishes` | Passed: one handoff, exact state transfer, one final outcome and no rediscovery |
| Full real-PostgreSQL `integration_workflow_primitives` target | Passed: 138/138, 230.07 seconds after conditional state transfer |
| Full real-PostgreSQL `fault_workflow_primitives` target | Passed: 24/24, 51.62 seconds after conditional state transfer |
| `SAGAT_SDK_SCENARIO=chain-state-final-success cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` with Colima Docker | Passed: TypeScript, Go, Python, C# and Java without golden blessing, 25.42 seconds |
| Runtime golden SHA-256 | `06f0247f38d0ec44df760b0c740b94f11c820ade9c4a26eb2046ccc92780ae40` |
| Workflow golden SHA-256 | `79e8b0983c27c6c63787d6c2b1b5364b082d73bffe90738a2a6976aa979c73f3` |
| Workflow-primitive and documentation contract targets | Passed: 26/26 and 8/8 |
| Rust formatting, workspace Clippy with denied warnings, architecture and diff checks | Passed |

The earlier direct-only release gates above record the state at their dates; this section adds
the conditional stateful shape. Child chains, bodies, triggers, broader graphs and their fault
matrix remain open.

## Signal-controlled continuation checkpoint (2026-10-16)

The published v1 continuation subset now also accepts exactly
`activity -> waitEvent -> continueAsNew`, with input transfer only. The wait has no timeout route or positive lookback.
The entry activity remains necessary because the successor handoff transaction plans an activity
task. Publication still rejects a direct `waitEvent` entrypoint and wider combinations.

The shared `chain-commands` scenario uses the public workflow, worker, scheduler and chain HTTP
routes. After a committed `wait.started`, the first signal and its exact-key replay both report
delivery to `release`. The predecessor finishes `SUCCEEDED`, the new head differs, and fresh
signal/cancel commands using the old expected run return `409 CHAIN_HEAD_CHANGED`. Cancelling the
successor and replaying that key both succeed; the chain wait returns final `CANCELLED`. A foreign
tenant sees `404`. No worker executes the successor before cancellation.

| Check | Result |
|---|---|
| `cargo test -p sagat-infra-ir --test continuation --locked` | Passed: 11/11, including the accepted wait graph and rejected timeout route, lookback, state transfer and extra node |
| `SAGAT_SDK_SCENARIO=chain-commands cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` with real PostgreSQL and five SDKs | Passed: 1 aggregate scenario, five languages; the worker stops before signalling so the successor cannot execute |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked chain_handoff -- --nocapture` with real PostgreSQL | Passed: 7/7 handoff regressions |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | Passed: 26/26 |
| `cargo test -p sagat-tests --test contract_developer_experience --locked` | Passed: 8/8 |
| `cargo fmt --all -- --check`, focused Clippy with `-D warnings`, `git diff --check` | Passed |

Reviewed normalized runtime golden SHA-256:
`e30141d6d49b2460ccd0b91324fd0614346da4735354f105d5247c75ad18de9c`.
Reviewed workflow golden SHA-256:
`489c5f7f15a36d450d5af4e6472ec63d99b0f7620770c47c11ca884ab8dc0176`.
These checks do not close the broader chain fault, parent/child or direct root-wait gates.

The runtime golden was extended and rerun in all five SDKs: an exact signal-key replay still
returns its original delivery after head rollover; a changed body under that key returns
`IDEMPOTENCY_KEY_REUSED`; a new signal to the activity-ready successor returns `STATE_CONFLICT`
without buffering. The worker is stopped before the first signal so the successor cannot race
into another wait. The reviewed SHA-256 above covers these added observations.

A real PostgreSQL fault test now injects failure at the `run.continued` outbox insert after the
signal has committed. The whole handoff rolls back, leaving one original head, one segment and no
`continuation.selected` event; removing the fault lets a retry commit exactly one successor.
The full integration target passed 138/138 with `--test-threads=2`, and the fault target passed
25/25 with the same setting. The first full integration runs stopped at 116/138 and 130/138
because the 30 GiB Colima Docker volume ran out of space while creating disposable PostgreSQL
databases; those interrupted runs are not counted as passing. Unused Docker images were removed
before the successful rerun. This environmental limit does not replace missing capacity tests.

## Five-SDK compatibility checkpoint (2026-10-16)

The complete active SDK conformance matrix passed: 36 shared scenarios, each executed by
TypeScript, Go, Python, C# and Java against the real API, scheduler, worker service and
PostgreSQL. The final command was
`cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`
with the Colima Docker socket; it passed its one aggregate test in 510.47 seconds.

The first matrix run exposed 17 historical root-work scenarios with implicit worker versions:
their goldens required protocol 1.4, while the current SDK default negotiated 1.5 and reported
root instance identity. Each historical fixture now explicitly offers 1.4. The separate
`protocol-negotiation` scenario still exercises 1, 1.4 and 1.5, and the primitive scenarios
exercise 1.5. No historical golden was reblessed for this version change.

The second matrix run found an independent race in `chain-query-handoff`: a still-running worker
could lease the successor before cancellation in some languages. The scenario now observes the
predecessor's committed `step.completed`, stops that worker, then waits for the new head. The
reviewed runtime golden changed only by these two observation steps; its SHA-256 is
`adc566af7ba7df74b4f73c407ba6e184f21e8c011551684ef9c8c7724158db87`.
The adjusted scenario passed twice on its own across all five SDKs and again in the final
complete matrix. This is active-scenario parity, not the still-open full WP-02–14 release gate.

## Telemetry projection compatibility (2026-10-16)

A newer journal event kind exposed a projection watermark defect: the decoder skipped the event,
but the watermark stopped at the preceding known event. The scheduler would clear the backlog in
an extra round by treating the remaining note as inconsistent, emitting a false warning and error
metric. A real-PostgreSQL regression failed with watermark 4 instead of 5 before the fix. The
projector now advances to the last persisted sequence in each bounded batch while deriving spans
only from recognized events. The regression confirms that an unknown event leaves no decoded entry
and the backlog clears in one projector round.

The focused regression passed after the fix, and the full
`integration_workflow_primitives` target passed 139/139 with `--test-threads=2`. Studio's
unknown-node card tests passed 76/76 across `RunNodeCard` and `RunDetailPage`. The broader
projection task remains open: this evidence covers unknown journal events, but does not prove
that every future definition node kind has an explicit unsupported representation in telemetry.
Rust formatting, focused Clippy with denied warnings and `git diff --check` passed.

## Declared-operation telemetry, published hints and terminal deliveries (2026-10-16)

**Declared Query/Update telemetry (T014-009).** The endpoints recorded an ad hoc
`workflow_declared_operations_total` counter and no primitive span, although `contracts/README.md`
§6 names `workflow.query`/`workflow.update` spans, `workflow_operation_duration_seconds{kind,outcome}`
and `workflow_primitive_rejections_total{kind,code}`. The handlers in
`crates/api-http/src/routes/declared_operations.rs` now open the contracted span under the request
span and record the contracted series; outcomes come from `WorkflowPrimitiveOutcome` (`succeeded`,
`applied`, `rejected`, `failed`) and codes from `WorkflowPrimitiveRejectionCode::from_reason`, so a
`details.reason` outside the closed set cannot become a label. The ad hoc counter is gone; no
dashboard, alert or document referenced it. `tests/integration/workflow-primitives/observability.rs`
adds three real HTTP/PostgreSQL tests: spans received by the test OTLP collector carry only the
four closed attributes and hang from their request span; every primitive series on the metrics page
has only contracted label keys and closed values, and neither the page nor any span contains the
operation name, a secret-shaped update key, the run or the tenant; the query access audit carries
the caller's trace id and an error body has a distinct request id; a failed audit insert fails both
Query and Update closed with `INTERNAL`, no evaluated value, and no state, receipt, journal or outbox
change.

**Two runs finished without telling their subscribers.** A run whose loop is followed by `end`
was finished through the loop hint helper, which wrote `run.finished` with an extra `node` field
and never called `outbound::fan_out`; webhook and broker subscribers of `run.finished` received
nothing. `finalize_chain` had the same omission (no production caller yet) and wrote
`chain.finished` without `traceId`. New tests failed on the previous code with 0 deliveries
(`loop_progress::a_run_finished_by_its_loop_is_delivered_to_run_finished_subscribers`,
`chain_progress::final_head_is_delivered_to_run_finished_subscribers_once`) and pass now: both
paths use the documented RunFinished hint plus the fan-out in the same transaction, keeping the
command's outbox id so replay stays idempotent.

**Published hints against the contract (T014-006, AsyncAPI part).** The outbox published 15
event types that `events.asyncapi.yaml` did not document. The document (0.2.0 → 0.3.0, additive)
now has a channel and message for each, `TaskReady.instancePath` is documented as optional, and
`loop.timer.scheduled` gained the `tenantId`/`namespaceId` every other hint carries. The baseline
was regenerated and compared with the previous one: no property, type or enum value was removed
(8 → 19 messages). `@asyncapi/cli@3.6.0 validate` reports the document valid (one pre-existing
informational note). `tests/integration/workflow-primitives/hint_contract.rs` validates every
`event_outbox` row of 11 scenarios (loops, collections, approvals, chains, declared updates)
against its documented message and rejects undocumented properties; it first caught a test-seeded
conflict row, which is now excluded explicitly. The worker 1.5 proto and OpenAPI parts of T014-006
are unchanged, so the task stays open.

| Command (`DOCKER_HOST` = Colima, real PostgreSQL) | Result |
|---|---|
| `cargo test -p sagat-telemetry -p sagat-api-http -p sagat-infra-postgres -p sagat-scheduler --locked` | all passed |
| `cargo test -p sagat-tests --test contract_workflow_primitives --locked` | 27/27 |
| `cargo test -p sagat-tests --test contract_asyncapi_events_webhooks_messaging --locked` | 4/4 |
| `cargo test -p sagat-tests --test integration_workflow_primitives --locked -- --test-threads=2` | 144/144 |
| `cargo test -p sagat-tests --test fault_workflow_primitives --locked -- --test-threads=2` | 25/25 |
| `cargo clippy -p sagat-infra-postgres -p sagat-api-http -p sagat-telemetry -p sagat-tests --all-targets --locked -- -D warnings`, `cargo fmt --all -- --check` | clean |

Not run here: the five-SDK conformance matrix (the SDKs did not change), Compose smoke and the
dependency advisory audit. The `execution_chain.started` journal event was written without the
run's trace id; `initialize_chain_in_transaction` now copies it from the run row, and
`chain_handoff::public_activity_completion_is_handed_to_a_successor_by_the_scheduler` asserts it
(it failed with `None` first). Integration 144/144 and fault 25/25 after the change.


## Foundation closers and contract hygiene (2026-10-17)

Production negotiates worker protocol 1.5 (`with_scoped_worker_protocol` in the engine); earlier
sections that say 1.4 describe their own checkpoints. `worker.baseline.json` now holds the 1.5
fields (additions only). The reason table of `contracts/README.md` §5 gained the four reasons the
code already used (D292) and a contract test ties it to the OpenAPI and to the remediation rows.
New tests: a cut-off primitive transaction commits whole or not at all and leaves no transaction
open (`approval_progress::a_decision_cut_off_at_any_await_commits_whole_or_not_at_all`, 400 rounds
on a one-connection pool, some cut off), duplicate loop-iteration admission, loop bounds at
publication, and approval principal and key bounds over HTTP and at publication. Decisions D291–D301
are in `research.md`.

## Loop error edge, limits, byte ceilings, projection and the runtime role (2026-10-17)

- **Loop `onError` (D294)**: for a loop that declares `onError` the failure drains on the loop scope
  alone (no `run.stop_requested`), the scope ends `FAILED`, and the run continues at the edge in the
  same transactions; a loop without the edge, and every run-level cancel, behave as before.
  Discovery was missing for failed bodies, expired deadlines and draining loops and the planner
  cancelled a draining owner: both fixed. Tests: domain `primitive_loops.rs` (6 new), integration
  `loop_progress.rs` (12 error-edge cases incl. the previously untested iteration-limit route),
  `loops_http.rs` (3), `fault/loops.rs` (5; 4 failed against the old store).
- **Top-level `limits` (D297) and byte ceilings (T014-008)**: `research.md` D310–D316; evidence in
  `tasks.md`.
- **Projection (T014-045)**: approval edges and decision, unsupported journal entries, owner edges.
- **A defect no test could see (G294)**: approval decisions and declared Query/Update locked a
  `memberships` row, which the least-privilege runtime role cannot do, so both answered 500 in
  production. The suites connect as the database owner. Fixed (plain read) with two tests that
  connect as `sagat_app` and failed first; an audit of every store statement is in progress.
- **Manifest**: 94 columns of the 014 tables were missing from `storage-classification.json` and the
  legacy-blob migration test returned the schema past the scoped-identity migration; fixed.

| Target (real PostgreSQL) | Result at `ae450b9` |
|---|---|
| `integration_workflow_primitives` | 200/200 |
| `fault_workflow_primitives` | 30/30 |
| `contract_workflow_primitives` | 30/30 |
| `integration_security_multitenancy` | 95/95 |
| `integration_durable_execution` | 178/178 |

## Five-SDK scenarios for approvals, collections and loops (2026-10-17)

New shared scenarios, each equal to its reviewed golden in TypeScript, Go, Python, C# and Java
without bless: `approval-settlement` and `approval-security` (WP-06, WP-14), `collection-ordering`
(WP-03: 17 items at concurrency 3, completion order 2,3…16,1,0, input-order aggregate, duplicates,
an empty list) and `loop-semantics` (WP-05: a false `while` runs no body, `repeatUntil` runs one,
three carry checkpoints, `maxIterations: 1` ends `FAILED`/`ITERATION_LIMIT`, a four-second durable
delay elapses with no worker). The `approval-decisions` golden was corrected: it had recorded the
stuck state of a routed `end` as PENDING. Harness additions: evidence blocks (`scopeEvidence`,
`auditEvidence`, `mustNotAppear`), `revokeMembership`, role tokens, and a `holdUntilLogged` runner
behaviour (completion order from counts, not sleeps).

Defects the scenarios exposed, each fixed test-first: approval routes never ran in a served engine
(new `approval_routes` scheduler pass); a decision or replay after the run ended returned 503; a
collection `result` reading `item` or `index` stalled (the snapshot now restores them, also in the
aggregate-size check added by the byte-ceiling work: found when the two branches met); a loop at its
iteration limit left the run CANCELLING (new `stopped_runs` pass); the Java and TypeScript worker
hosts dropped concurrent iterations of one scope that share the legacy `task_id`.

After the merge: clippy clean workspace-wide; `integration_workflow_primitives` 207, `fault_workflow_primitives`
37, `contract_workflow_primitives` 30, `integration_platform_foundation` 88 (includes the runtime-role
privilege audit), `integration_security_multitenancy` 96, `integration_durable_execution` 178,
`contract_grpc_durable_execution` 63. The compatibility test expected protocol 1.4; the server has
negotiated 1.5 since the scoped protocol was enabled, and the test now says so.


### Populated release upgrade regression — combined completion branch, 2026-10-17

The actual v0.1.0 process gate reached migration 20261205000000 and failed with PostgreSQL 55006:
CREATE INDEX on steps had pending deferred steps_quota_flush trigger events. Fresh empty-database
migration tests had not exposed this. A focused real PostgreSQL regression restores the pre-005
schema over an existing activity step, then executes the complete up migration in one transaction.
It first failed with the same error (`/tmp/sagat-completion-discovery-migration-red.log`).

The unreleased bounded_execution_discovery migration now creates indexes before its node-kind
backfill. Deferred quota triggers retain their normal commit boundary and the migration remains
atomic. The focused regression passed 1/1 in 7.21 s
(`/tmp/sagat-completion-discovery-migration-green.log`), preserving step identities/status and
backfilling the activity kind with a usable index. It ran the actual integration target binary
built by the preceding focused Cargo command. All 60 migration files in the published v0.1.0 tag
were compared bytewise and remain unchanged. The full old-process upgrade must still be repeated
with a newly built current binary containing the corrected migration checksum. This does not close
T014-041 or T014-043 and does not bridge an already-applied unreleased checksum.


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


## Additional source-branch evidence (8ade8f18; integrated gates pending)

## Release-gate checkpoint (2026-10-17)

Run on `feature/workflow-primitives` after merging the chain, scoped-compensation, loop, approval,
collection, chain-wait, compatibility/performance and scale branches and `develop` (owner decisions,
G9/G21/G48/G141). Machine: one shared laptop, other agents building and testing at the same time.
PostgreSQL is a disposable container per test (`testcontainers`), started from this repository's
fixtures. No golden was blessed in this step; each one blessed earlier was read by hand first.

### Commands and results

| Command | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `python3 scripts/check-architecture.py` | 22 crates, layers respected, no cycles |
| `cargo test -p sagat-tests --test integration_workflow_primitives` | 319 passed, 0 failed, 1 ignored (the 10,000-item run, `collection_10k`) |
| `cargo test -p sagat-tests --test fault_workflow_primitives` | 87 passed, 0 failed |
| `cargo test -p sagat-tests --test contract_workflow_primitives` | 47 passed |
| `cargo test -p sagat-tests --test integration_platform_foundation` (includes `runtime_privileges`) | 89 passed |
| `cargo test -p sagat-tests --test integration_durable_execution` | 178 passed |
| `cargo test -p sagat-tests --test integration_security_multitenancy` | 123 passed |
| `cargo test -p sagat-tests --test integration_sagas_compensations` | 74 passed |
| `cargo test -p sagat-tests --test integration_scalability_ha` | 44 passed |
| `cargo test -p sagat-tests --test contract_developer_experience` | 8 passed |
| `SAGAT_TEST_AS_RUNTIME_ROLE=1` over `integration_workflow_primitives`, `fault_workflow_primitives`, `integration_security_multitenancy` | 319 / 87 / 123 passed: every service connected as the least-privilege role `sagat_app` |
| `cargo test --workspace --lib --bins` | no failure |
| Five-SDK conformance, 51 scenarios x 5 languages, `cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, no bless | passed on the branch heads of the chain-wait (51 scenarios) and compat-perf (44 scenarios, 220/220) increments; rerun for the whole tree by CI on the pull request |
| Compose smoke, `cargo audit`, contract compilation, observability, Studio, native Go/Java | run by CI on the pull request (not run locally: the disk is near full) |

`SAGAT_TEST_AS_RUNTIME_ROLE=1` is not applied to `integration_scalability_ha`: those tests probe
owner-only operations on purpose and fail under the sweep (12 failures that are the point of the tests).

### Fault matrix (T014-040)

`fault_workflow_primitives` has 87 tests; each row names the tests and the evidence is the suite result
above. Races use held locks and `pg_stat_activity`, barriers or in-transaction hooks, never sleeps.

| Class | Tests (files under `tests/fault/workflow-primitives/`) |
|---|---|
| Crash at a critical point | `operations/crash.rs` (3: an Update cut off at every await, a connection killed while queued for the run lock, after the decision); `scope_compensation.rs` (a connection dying inside each transaction of the compensation, the scheduler cut at every await); `loops.rs::delay_checkpoint_restart_and_duplicate_timer_fire_are_atomic`; `chains.rs::a_dropped_connection_before_commit_…`, `a_restart_between_the_hand_off_and_the_event_…` |
| Duplicate delivery | `operations/races.rs::two_deliveries_of_one_update_key_…`; `approval_combinations.rs` (same decision twice, two deciders); `chains.rs::duplicate_hand_offs_of_one_boundary_…`; `scope_compensation.rs::a_compensation_reported_twice_…`; `loops.rs::two_replicas_delivering_the_same_due_timer_…`, `concurrent_replicas_finishing_one_drain_…` |
| Race: timeout vs signal / update / cancel / completion | `loops.rs` (timeout vs completing body in both orders; cancel vs timeout in both orders; a signal during the delay is refused); `operations/races.rs` (Update vs worker completion, vs firing timer, vs wait timeout); `approval_combinations.rs` (decision vs Update, vs run cancel, vs chain latch, vs hand-off, vs chain deadline, vs expiry); `approvals.rs::decision_and_expiry_race_…`; `chains.rs` (cancel vs hand-off, signal vs hand-off, Update vs hand-off, parent cancel vs child hand-off — each in both orders) |
| Lost broker delivery | `chains.rs::restart_recovers_the_successor_when_the_handoff_hint_is_lost`; `switch.rs::restart_recovers_the_recorded_choice_when_the_broker_hint_is_lost`; `collections.rs::bounded_public_collection_finishes_in_input_order_after_restart_and_lost_hints`; `scope_compensation.rs::a_compensation_whose_hint_never_arrived_is_leased_from_the_database`; `chains.rs::a_lost_wake_of_the_parent_…` |
| Failed outbox / audit write rolls everything back | `approvals.rs` (outbox and audit failures on opening, decision and expiry); `loops.rs::outbox_and_audit_failures_roll_back_the_whole_checkpoint`; `chains.rs::chain_limit_outbox_failure_…`, `signalled_wait_handoff_rolls_back_…`; `switch.rs::successor_outbox_failure_…`; `scope_compensation.rs::a_planning_transaction_that_cannot_write_its_hint_…`, `an_operator_decision_that_cannot_be_audited_…` |
| Stale fence / replica takeover | `approvals.rs::stale_scheduler_fence_cannot_open_or_expire_an_approval`; `loops.rs::scheduler_takeover_mid_drain_…`, `a_replica_that_lost_its_shards_cannot_fire_the_timer`; `scope_compensation.rs::a_scheduler_replica_that_takes_over_mid_compensation_…`; `collection_sources.rs::a_scheduler_that_loses_its_shard_after_reading_…` |
| Failed / unknown compensation | `scope_compensation.rs` (reported twice, stale report refused, operator decision unaudited, cancellation racing a stop for failure); `integration_sagas_compensations` (74) and `fault_sagas_compensations` (13) for the root saga |
| Restart | `loops.rs::a_scheduler_restarted_during_the_delay_wakes_the_loop_exactly_once`; `approvals.rs::restart_and_identical_replay_…`; `chains.rs::restart_…` |
| Tenant boundary | `approvals.rs::current_membership_and_tenant_boundary_are_checked_before_receipt_replay`, `a_revoked_approver_cannot_replay_a_receipt_it_committed`; `chains.rs` tenant-claim tests; `integration_security_multitenancy` (123) |
| Secrets | `operations/redaction.rs::a_resolved_secret_and_secret_shaped_text_never_leave_the_operation_boundary`; `operations.rs::undeclared_input_canary_never_crosses_…`; `integration_security_multitenancy::secrets` and the G48 pool tests |

Independently inspected persistence: every fault test ends with `verify_run` / `assert_consistent`
(the stored projection equals a replay of the journal) and the tests read the rows the claim is about
(`run_instance_usage`, `tenant_iteration_usage`, `loop_timers`, `chain_links`, `event_outbox`,
`audit_records`) rather than only the API response.

Failures seen on the way, and what they were: `scope_compensation::a_scheduler_replica_that_takes_over_…`
failed about half of its runs because the dead scheduler's membership was still unexpired (G407, fixed:
8 of 8 afterwards); three SDK scenarios failed once under host load 30–47 and passed alone (G467, not
investigated); four tests of the scale branch failed on the first run of the merged tree (a charge count
that differed from `RunState`'s, two fixtures, one plan test) and were fixed in `fix(scale): …`.
Skipped: the 10,000-item end-to-end run is `#[ignore]` (7,926 s before the scale fix; 1,405 s after, see the
performance report).

### Not proven

Production ceilings for `maxItems` and `maxIterations` of 10,000: after the scale fix a 10,000-item
collection takes 23 minutes in a debug build and the 10,000-iteration loop did not finish in 30 minutes,
because the worker `complete` and `poll` still restore the whole run (G520). Approvals inside bodies,
a run deadline over an open collection or loop, nested scopes, branches and timers in bodies stay closed.
OTLP export of approval spans is not exercised end to end. See `research.md` for G/O numbers.

### CI on the pull request (2026-10-17)

Run 39057216527 on head `7d7868ff`: all 12 checks passed (`rust`, `sdks`, `studio`, `contracts`, architecture,
`cargo audit`, Compose smoke, native Go and Java on both architectures, observability files, package smoke,
scale and recovery). The first run on the same pull request failed three jobs and a second round exposed
more; each failure was a real gap in the branch and was fixed in a commit of its own (see the T014-043
evidence). A `cargo test --workspace` I ran on the laptop afterwards reported 33 failures in suites that
need containers and toolchains this shell did not point at (`fault_developer_experience`,
`e2e_developer_experience`, `e2e_inline_code_sandbox`, ...) while the machine was under heavy load; the
same command passed in CI, so the laptop run is not used as evidence here and I did not investigate those
failures further.

### Combined integration checkpoint: bounded maintenance and current authority

The approved integration worktree records reproducible commands, failed baselines and exact
scope in `tasks/project-completion/integration-notes.md`. Current focused evidence includes:

- Exact-path automatic expiry/retry with whole-run iteration-history SELECT revoked: red first,
  then successful recovery and projection/journal replay. Exceptional content-quota drain keeps
  its full-run revision/fence guard; worker claims/reports are still a separate conversion.
- Shard expiry after an actual unchanged-row lock wait, for both full and bounded transactions,
  and binding authority through bounded commit: three real failing baselines, then **3/3** passed.
- Combined lock/byte/chain/scoped/loop regression **81/81**; after making the scoped fixture's pool
  explicitly follow the runtime-role switch, scoped/barrier repeat **19/19**, with the waiting
  execution connection asserted to be `sagat_app`. Current SQL privilege audit **1/1**, no exception.
- Full primitive fault matrix **87/87** on the explicit 1 GiB PostgreSQL shared-memory profile.
  The previous **78/9** run suffered shared-memory exhaustion and server recovery; its failures
  remain recorded. The same exhaustion warning precedes both preserved disposable-server crashes.
- Real PostgreSQL/OpenBao aggregate **3/3**, including blocked-provider cancellation/rotation/
  outage receipts; explicit cancellation acknowledgement works without granting new capabilities.
- Additive ceiling compatibility migration: both resource families and guarded configuration-
  preserving rollback/reapply; complete migration module **5/5**. No earlier migration was edited.
- Shared SDK expected limits/minors and a deterministic short secrets journal repaired with four
  individual all-five-language repeats; no auto-bless. The original 55-scenario run failed those
  four scenarios and is not relabeled successful.

The original workspace diagnostic completed 234 harnesses with **3310 passed/35 failed/61 ignored**
and two additional rustdoc compilation failures from sources newer than compiled libraries.
Named repaired cases/modules and successful current doctest compilation are retained; a final
current-source whole-workspace release gate is still required. Strict all-target Clippy passed
at the documented checkpoints. The earlier Compose/browser smoke passed on its recorded source
boundary; the new shared-memory Compose change has a configuration check, not a rebuilt smoke.
The moderate performance target passed **4/5 ignored**, which does not execute the separate
10k/100k capacity workloads. The open tasks stay unchecked: broad graphs/nesting, descendants under
low capacity, the wider compatibility matrix and the named large resource workloads are not
established by these focused repairs.


### Combined integration: scoped heartbeat

The exact-path heartbeat contract and deterministic barriers are recorded in
`contracts/conformance.md` and `tasks/project-completion/integration-notes.md`. Four current tests
passed after genuine failing baselines: no full-history read, current role after contention,
terminal late acknowledgement and twelve renewals on two runtime connections. The wider selected
workflow gate passed **85/85**, OpenBao aggregate **3/3**, all-target strict Clippy passed, and
`scoped-identity`/`lease-lost` each passed across all five SDKs without changed expected files.
Two whole gRPC repeats each passed **69/70** with different near-deadline stream fixture failures;
the byte-credit case passed separately with runtime privileges. Those failures remain under
investigation, so this checkpoint does not approve the complete gRPC or release gate. Claims,
completion/failure and the large resource workloads remain open.


The heartbeat gRPC follow-up passed the complete suite **70/70**, 31.87 s, with runtime-role
connections. Real row-lock barriers replace near-deadline REST admissions; the test transport now
matches the public SDKs' 16 MiB decoder. Its first barrier repeat exposed that default-decoder
mismatch (**69/70**), retained alongside the earlier failures in the integration notes. Production
stream deadlines, input credit and task bounds are unchanged. This checkpoint predates the next
bounded-claim slice and does not establish the final project release gate.


### Combined integration: bounded scoped claims

Discovery now retains exact paths from the same quota/pool/protocol/breaker-aware query as the
run-only API. Scoped leasing no longer restores whole-run iteration history. Real runtime-role
barriers verify binding-before-session ordering, current role after contention and refusal after
binding/session withdrawal. An injected later-path write failure proves an earlier committed token
still reaches its worker. Task/forward-effect identity construction remains shared with Root.

The selected worker/loop/scoped-compensation modules passed **79/79**, 69.68 s with four test
threads, after fixing six completed-loop NULL-fence failures; the full privilege module passed
**33/33**, 8.36 s, and strict workspace/all-target Clippy passed, 1 min 43 s. These results, failed
baselines and exact commands/logs are recorded in the integration notes. The expanded workflow
diagnostic remains **328/15/1 ignored**: it also exposed a distinct child-start shard fence defect
and connection exhaustion from parallel default-thread harnesses. An isolated child-start failure
confirms that defect independently of the exhausted profile. Final protocol/provider/SDK repeats,
child-admission correction, report/capability conversion and large resource gates remain open.


### Combined integration: child creation under parent shard authority

An explicit creation-only parent fence restores parent-owned child admission while keeping normal
run-shard checks strict. The adapter rejects reopened/ordinary/already-mutated transactions and
samples expiry after the shard lock. The old isolated child-start baseline failed **0/1**; the
complete pre-report-regression workflow suite now passed **344/344, 1 ignored**, 272.41 s, including
all eight child-chain failures and the six completed-loop compensation failures. Three real two-
replica ownership tests passed **3/3**, 17.38 s. Application/PostgreSQL/scheduler strict all-target
Clippy passed; no new privilege, schema, wire or external-effect guarantee was added.

The claim server also passed complete gRPC **70/70**, OpenBao **3/3**, and the three all-five SDK
identity/collection-compensation/loop-compensation repeats recorded in the integration notes.
These are focused current checkpoints. Five newly added report/history/current-receipt regressions
subsequently failed **0/5** and are outside the 344-pass checkpoint. Inline report/capability
conversion, wider graphs/nesting and full release/resource acceptance remain open.


### Combined integration: bounded inline completion and failure

Five real failing history/current-receipt baselines passed **5/5**, then the complete pre-provider-
barrier workflow suite passed **349 passed /0 failed /1 ignored**, 234.19 s. Two additional real
OpenBao scan barriers passed **2/2**, 5.34 s, including independent acquisition of the run lock and
committed role/binding withdrawal while the provider read was paused. Resumption commits no step
completion and returns the expected typed refusal. Current receipt authority also covers Root.

Current complete gRPC **70/70**, OpenBao aggregate **3/3**, primitive faults **87/87**, SQL privilege
module **33/33**, strict whole-workspace/all-target Clippy, format/diff checks and four individually
selected all-five SDK scenarios passed. Exact logs, wall/harness times, failed compilations and
source boundaries are recorded in the integration notes. Pure policy, logical output accounting,
mandatory secret-denial audit, transactional receipt/outbox and provider I/O boundaries remain;
no schema, wire or expected SDK output changed. Blob completion, secret/blob capabilities, broader
body/control/nesting, the complete compatibility matrix and 10k/100k resource gates remain open.


## 2026-10-17 — Current scoped secret and blob capabilities

Refs: T014-016–017, T014-040–041, FR-014-014–017, NFR-014-001–002.

Scoped secret delivery now opens current exact-path transactions in both authority phases. Root
and scoped delivery share the same handoff algorithm through an internal transaction-lifecycle
enum, preserving Root legacy-attempt handling and audit text. Both phases check current worker
membership/bindings, live token/attempt/generation, cancellation, worker pools and publisher
ACL/version inputs. Real provider I/O occurs between committed transactions. Changed authority
or inputs discards the resolved value; a successful value-free audit commits before any handoff.
Scoped secret success/denial and pool-refusal audits include the complete logical step UUID, so
another iteration of the same node does not collapse into the first iteration's audit.

Scoped blob access and report-scope lookup also use current exact-path state. Blob access retains
live lease and cancellation fences. A step declaring secrets retains its typed upload scanning
refusal, including a current cancelled attempt; that exception only returns a denial. Report-scope
lookup intentionally permits an expired lease for receipt replay while checking current authority
and echoed step identity. Actual catalog/object authorization remains in the existing blob service;
these tests do not claim an object-store byte transfer, or enable scoped blob completion.

Follow-up public-RPC evidence: the managed database bound probe previously restored the whole run
before the bounded upload check. `database_blob_limit` now returns no managed bound for non-root
instances; this is not authorization, and the existing upload capability remains mandatory.
The new real gRPC test first failed with `Internal` under denied historical reads (0/1,4.63s).
After the fix, the pipeline group passed5/5,8.01s, including actual uploaded bytes/hash and a
self-consistent forged generation refused with `STALE_ATTEMPT` before a second object reservation.
An intermediate test incorrectly expected a capability error for an internally inconsistent task
ID/generation; the server correctly returned `InvalidArgument`. The fixture now changes both and
asserts `Aborted/STALE_ATTEMPT`. A prior field-name compiler error is also retained, not counted as
behavioral RED. Logs: `/tmp/sagat-completion-scoped-grpc-blob-{red,red-runtime,green,green-fixed}.log`.
Affected regressions passed on disposable repository fixtures: complete gRPC70/70,107.87s and
real PostgreSQL/OpenBao aggregate3/3,45.24s. Strict workspace/all-target Clippy passed2m57s;
their logs use the same prefix. The full all-five scenario matrix is separate and still pending.

### Chain-client acceptance closure (T014-038)

The current acceptance audit found missing Python synchronous `update_chain`,
`wait_for_chain_completion`, `rerun_as_new`; the three aliases now share the existing async
implementation/private loop. RED3/3 missing-method failures preceded focused43/43 and complete
Python264/264 passing results, strict mypy and Ruff. Eight selected current scenarios completed
in all five SDKs without bless: one continuation-authoring scenario and seven chain runtime
scenarios. All35 normalized runtime artifacts were independently parsed against exact reviewed
goldens, including final-chain versus exact-segment waits and parent cancellation. The retained
manifest is `tasks/project-completion/evidence/chain-sdk-20261017.json`. T014-038 is closed;
T014-041 and the other shared release gates remain open. The concurrent full56-scenario command
has an original blob-heartbeat failure; its corrected all-five case passed separately, so this
task-specific evidence is not an assertion of a wholly green aggregate command.

Evidence:

- The two new history-denial cases first failed **0/2** with `Internal`, because the old full-run
  restoration read revoked `iteration_checkpoints` (`/tmp/sagat-completion-bounded-capabilities-red.log`).
  A subsequent test build failed on an incorrect test-only `TenantId` constructor; it was corrected
  and is retained at `...-green.log`, not counted as a test result.
- Corrected focused tests passed **7/7**, 12.86 s (`...-green-fixed.log`). The same compiled cases
  then passed **7/7**, 8.54 s using the repository's disposable PostgreSQL fixture with no external
  database URL (`...-canonical-fixture.log`). They exercise the runtime role with two connections,
  revoked unrelated-history access, correct secret scope, per-instance audit deduplication,
  incorrect step/generation, foreign tenant, current membership withdrawal and full replay oracle.
- Four of those cases use real OpenBao 2.7.1 with explicit secret migration and a wrapper that only
  pauses the actual provider read: role/binding withdrawal during output scanning or secret
  resolution commits independently, including a run-row lock probe. Resumption refuses the report
  or delivery and records neither a completion nor a successful secret handoff.
- Complete primitive integration: **356 passed /0 failed /1 ignored**, 443.78 s, four harness threads
  (`...-workflow-regression.log`). This diagnostic used the dedicated local verification database;
  it is not the final canonical whole-workspace release gate.
- Fresh repository-owned disposable database fixtures, without an external database URL:
  security **123/123**, 130.62 s; gRPC **70/70**, 83.91 s; primitive faults **87/87**, 90.13 s;
  PostgreSQL/OpenBao aggregate **3/3**, 44.18 s. Logs are
  `/tmp/sagat-completion-bounded-capabilities-{security,grpc,faults}-regression.log` and
  `...-provider-all-regression.log`. Command wall times/exit codes are separately retained in
  `...-gate-results.json`. An initial `provider::` filter selected **zero tests**; that result is
  explicitly marked unaccepted in the manifest and was replaced by the unfiltered three-test run.
- Strict whole-workspace/all-target Clippy passed, 3 min 09 s (`...-clippy.log`). Formatting and
  diff checks passed. No migration, PostgreSQL adapter query, privilege grant or wire field changed.
- All five SDKs passed the unchanged `secrets` scenario (38.79 s command wall time). The first
  `scoped-identity` run failed after 74.23 s: .NET/Java inspected `WAITING` at version 2 while
  TypeScript/Go/Python inspected `RUNNING` at version 4. The 500 ms sleep read different legitimate
  scheduling milestones. Commit `6c019922` replaces it with the existing public journal admission
  barrier. The manually reviewed golden changes only `sleep` to `awaitJournal`; the expected
  `RUNNING` status and all other observations remain unchanged. Corrected all-five conformance
  passed, 54.65 s, on a disposable fixture with no external database URL. Scenario/expected-document
  validation passed **1/1**, 0.29 s. Logs and retained initial failure are in
  `...-sdk-results.json`, `...-sdk-scoped-identity-barrier.log` and
  `/tmp/sagat-completion-scoped-identity-{sleep-race.json,scenario-validation.log}`.

This increment does not close the remaining broad body/nesting, low-capacity descendant admission,
blob completion, full old-version compatibility, 10k/100k resource or release gates. The task
checkboxes remain open. The ongoing flat 10k benchmark executes frozen commit `025ce8d4`, not this
increment; its source and environment remain explicit in its own manifest.


## 2026-10-17 — Public root activity sources for collections (G420)

Refs: T014-004, T014-016–017, FR-014-003–005,016–017.

Publication now accepts a plain root activity prefix followed by the existing root collection
chain. The root producer may return an inline array or commit its output blob; the existing
collection source port captures it under the original ownership, digest, value and context limits.
No runtime transaction, wire field, migration or adapter query changed. Error edges, compensation,
reconciliation, effect policies and managed database operations in the prefix remain unsupported,
as do activity successors and nested/broader bodies. This is another implementation increment,
not a reduction of the accepted feature scope or completion of its remaining tasks.

Four native public pipeline tests first failed **0/4**, 11.08 s, on the closed entrypoint gate
(`/tmp/sagat-completion-collection-pipeline-red.log`). After enabling the bounded prefix they
passed **4/4**, 6.46 s (`...-green.log`). The inline case was strengthened to use a legacy minor
1.4 producer advertising both activities: it produces root work but cannot claim the admitted
scoped work; a minor 1.5 body worker completes the run. These four cases passed **4/4**, 4.92 s
(`...-legacy-root.log`). They cover REST publication/start, real gRPC blob upload and exact source
completion replay, ordered/equal members, uncommitted descriptor refusal (`BLOB_UNAVAILABLE`),
failed producer with no collection, and independent full replay. They use the repository's
self-contained PostgreSQL/object fixtures, not an external database URL.

The final fixture deliberately gives the root producer and body activity the same `fetch` node
id. The identical `steps.fetch.output` expression resolves the root array at capture and the
scoped result at body return. Updated native cases passed **4/4**, 5.66 s (`...-same-node.log`).
Both data and binding identity remain scoped; this also checks that body contexts do not expose
the producer's array as local step data.

The new five-SDK `collection-step-source` scenario first failed only because its two reviewed
expected phase documents did not yet exist (20.59 s, `...-sdk-review.log`); all five builders and
normalized runtime documents agreed. Both complete documents were manually reviewed against the
scenario before writing the new expected files, with no bless mode. The initial reviewed case
passed **1/1 across all five**, 19.81 s (`...-sdk-green.log`). After strengthening the fixture to
the equal root/body node ids, the manually updated definitions/observations passed all five again,
22.71 s (`...-sdk-same-node.log`). Review manifests are `...-golden-review.json` and
`...-same-node-review.json`; existing semantic goldens and SDK/runner sources remain unchanged.
The suite now contains 56 scenarios. Scenario/phase validation passed **1/1**, 0.05 s.

Two focused IR guard tests passed (0.03 s), exercising five unsupported source-policy mutations
and the still-closed activity successor. Complete IR package tests passed **169/0/0 ignored**;
doctests compiled with zero executable examples (`...-full-ir-regression.log`). Complete primitive
integration passed **360/0/1 ignored**, 270.21 s, with four threads on the repository's disposable
fixture (`...-workflow-regression.log`), before the final same-node test-data enhancement; the
production code stayed unchanged and the four affected cases then passed as recorded above.
Strict workspace/all-target Clippy passed (1m10s, `...-clippy.log`); formatting/diff checks passed.

The G420 public source gap is closed for this plain prefix. Broader source policies/graphs,
nesting and descendant admission under low quotas, remaining resource/compatibility and release
criteria stay open. No task checkbox is advanced on the strength of this focused subset alone.


## 2026-10-17 — Flat diagnostic completion and execution-role measurement guard

The frozen 025ce8d4 diagnostic completed all 10k/width 1,16,256 cases with zero scheduler errors.
Raw fragments, source/environment and limitations are retained in
[the capacity report](../../tests/performance/reports/workflow-primitives.md). This owner-role,
serial-driver diagnostic is not final capacity acceptance; sampled peaks 1/15/255 do not establish
observed 16/256. Checkpoint-only SQL probes, queue delay, true peak RSS and canonical infrastructure
remain explicit measurement limitations.

Two new real measurement tests failed 0/2 (2.85s): an even interval selected no admission probes,
and a journal trigger observed actual writes with current_user=postgres. The driver now creates
an 8-connection runtime pool, verifies current_user=sagat_app and labels observer/execution roles.
The trigger guards all measured scheduler, claim, completion and finalization journal writes.
Two consecutive probe points cover both transition families. Corrected tests passed 2/2 (2.66s),
with RED/GREEN logs at `/tmp/sagat-completion-perf-profile-smoke/`. An initial duplicate-out-dir
compiler error is preserved separately and is not counted as a behavioral RED. The old benchmark
completed before its executable path was rebuilt; its digest was checked unchanged at completion.
These safeguards improve measurement validity and do not close T014-042 or production ceilings.


## 2026-10-17 — Scoped forward BlobOutput completion

Ordinary forward scoped activities now upload and commit native BlobOutput results using current
exact-path authority, without restoring unrelated iteration history. Preparation returns an exact
receipt or a live scoped target in one transaction. A miss cannot reach catalog metadata with a
forged/expired lease; an exact receipt remains replayable under current membership/binding after
lease completion. Cancellation can report an already produced result for scope effect accounting.
Derived and declared-secret blob outputs remain refused.

The storage port binds descriptor ID/size/digest/content type to its BlobCommit, and the existing
catalog UPDATE repeats exact catalog metadata/owner/state checks. Output, catalog state, journal,
quota, outbox and receipt commit or roll back together. Blob quota-failure recovery repeats the
live cancellation-tolerant fence after rollback. No migration, broad grant or wire field is added.
Existing worker.scoped.report spans, upload/read audits and quota refusal counters remain; no blob
bytes, lease tokens or provider values enter logs.

Three bounded independent read-only reviews found five actionable issues, all corrected with
behavioral regressions. The final split-preparation race was reproduced before replacing the two
lookups with atomic receipt-or-scope preparation; no fourth full review was run after the skill's
three-cycle bound was surfaced. The user selected the current review and no external CLI was used.

| Executed gate, final source | Result |
| --- | --- |
| Native scoped_blob_completion filter, runtime role | 8 passed,0 failed,19.56s |
| Complete contract_grpc_durable_execution | 70 passed,0 failed,51.04s |
| Complete fault_workflow_primitives | 87 passed,0 failed,95.07s |
| Complete integration_database_connections, real OpenBao | 3 passed,0 failed,30.40s |
| Complete integration_workflow_primitives | 369 passed,0 failed,1 ignored,191.06s |
| collection-blob-output, all five SDKs | 1 passed,17.68s; five exact normalized documents |
| blobs, all five SDKs, existing reviewed golden | 1 passed,18.69s; five exact normalized documents |
| Scenario phase/golden registration,57 cases | 1 passed,0.06s |

The ignored test is the separately long10k reference-source end-to-end case; it is not counted as
passing in this round, and the frozen flat performance workloads are not substituted for it.
Public native tests cover300KB uploads, history SELECT refusal, journal rollback/retry, foreign
iteration refusal, current role withdrawal, changed-key conflict, pre-catalog fencing, cancellation,
descriptor/canonical catalog type mismatch and expiry at a real quota lock barrier. HTTP/gRPC and
PostgreSQL are real; the blob fixture uses the production Memory backend, not an S3 acceptance test.
The initial all-five execution failed only because this new case lacked reviewed golden files;
all five builder/runtime documents were independently compared and inspected before creating the
new goldens. No existing golden, normalizer or SDK runner was changed for this increment.

A superseded validation attempt produced58 passing/12 failing gRPC tests with57P01/57P03 database
shutdown errors while older validation processes were being interrupted. That result is retained,
not accepted. The uninterrupted final run above passed all70; no product-causality claim is made
from the interrupted fixture. Test-only compile/SQL mistakes are retained separately from actual
behavioral RED. [Exact evidence and source hashes](../../tasks/project-completion/evidence/scoped-blob-20261017/manifest.json)
retain failures, native/runtime results and all ten current normalized SDK documents.

Broader body planning/nesting, descendant capacity,100k nested measurements and complete release
gates remain open. This focused increment does not close T014-014..017 or T014-040..044.


Final static gates for this increment: `cargo fmt --all -- --check` and `git diff --check` passed;
`cargo clippy --locked --workspace --all-targets -- -D warnings` passed in1m30s;
`python3 scripts/check-architecture.py` passed for24 crates with respected layers/no cycles.
These are recorded with the same focused source hashes, not claimed as the complete T014-043
release gate (Compose, full locked workspace execution and advisories still require their named gates).


## 2026-10-18 — Isolated capacity-group lifecycle A1

After the three-cycle whole-protocol escalation, work was decomposed into pure lifecycle A,
transaction/eligibility B and rollout/composition C. Three independent component reviews (abstract
invariants, bounded representation, implementation) found no substantive violations under the
exact current member-fact and caller proof preconditions. This was a deliberate smaller unit,
not a fourth whole-protocol review; the whole nested protocol remains unaccepted.

`cargo test --locked -p sagat-domain --test capacity_groups` first failed to compile with E0432
because the new typed API did not exist. That is API-absence RED, not a reproduced runtime bug.
After implementation it passed6/6 in0.23s, including1024 deterministic histories(seed3714) checked
against an independent full registry. Named cases cover intermediate holder FINAL with surviving
A/C, non-holder finality preserving the child epoch, yielding before child completion, finished
member facts refusing join and stale/foreign operations leaving the header unchanged.
`cargo test --locked -p sagat-domain --lib execution::capacity::tests::` passed2/2 for overflow
without partial mutation. Locked domain/all-target Clippy with warnings denied passed10.63s;
formatting and the24-crate architecture check passed. [Exact evidence](../../tasks/project-completion/evidence/capacity-a1-20261018/manifest.json)
contains the missing-API failure, results and source hashes.

The model keeps a constant-size header plus one selected member fact, not a historical registry
scan. No PostgreSQL store, scheduler/worker gate, new quota projection or public nesting is
implemented by A1. Authority, quiescence, exact fact freshness, atomic header/member persistence,
compatible restore and actual low-quota/100k behavior remain unproved B/C dependencies; canonical
T014-014..017 and the broader release tasks remain unchecked.


## 2026-10-18 — Child admission/continuation lock regressions

B0 source inventory exposed two existing admission-order risks, independently of nested groups.
Two controlled real-PostgreSQL competitions were added to the fault suite: depth-two child
handoff against root-child creation under the tree advisory gate, and evaluated continuation
against a new public root start sharing a novel payload under the active-run quota gate.
Before correction both failed: the first returned an explicit PostgreSQL deadlock; the second
returned public `Unavailable`. The source review identifies its quota/payload uniqueness
inversion; the public error alone does not expose the database cause. Earlier invalid parallel
and overclaiming-session fixtures, and a later test Arc ownership compile error, are retained
separately and are not behavioral RED evidence.

The reservation's parent lock now uses `NO KEY UPDATE`, permitting stable-key FK checks while
still excluding parent writes. Both input and initial-state retention blocks now follow tree/
active-run admission, matching evaluated handoff's resource order. No boundary, payload hash,
role, audit, outbox, migration or wire contract changed. Two bounded independent review cycles
addressed payload inversion findings and then found no issues in the actual patch/barriers.
The owner selected the existing review; no external CLI was invoked.

Focused regression: **2/2 passed in8.59s**. The first also verifies rollback of admitted child
creation, retries through the real scheduler and verifies journals; the second commits both
new root and successor with independent journal checks. Runtime-role regression passed709 tests: gRPC70/38.98s, workflow faults89/79.82s, real
PostgreSQL/OpenBao3/30.03s, durable execution178/95.89s, and workflow integration369/178.62s.
The separately long10k reference-source case remains ignored, not passed. Locked workspace/all-target
Clippy passed1m57s; format/diff checks and24-crate architecture checks passed.
[Exact commands, failures and source hashes](../../tasks/project-completion/evidence/child-root-fk-20261018/manifest.json)
are retained. This does not close the full workspace/Compose/advisory release gate. The working lock inventory remains incomplete, including
its recorded parent-guard/legacy child-link questions. A1 does not authorize runtime nesting;
B0 and all previously open canonical tasks remain unchecked.


## 2026-10-18 — Five-SDK instance-limit admission

The new58th common case `workflow-instance-limits` proves maxInstances3 succeeds for a->b->end;
maxInstances2 accepts both activity completions, fails with persisted INSTANCE_LIMIT and leaves
end CANCELLED/attempt0. All five builders preserve depth1/8 and reject instance0/100001 and depth0/9
with VALIDATION_FAILED. No nested depth8 execution is inferred. Source/status/graph evidence and
four worker calls agree in all five complete documents. Two fresh-context reviews found then
resolved a start-response race: each root now starts while workers are stopped, then its dedicated
worker starts; it is stopped/drained before the next root. No sleeps or normalizer changes hide it.
The missing-golden probes are retained separately from the actual final pass: all five completed
in11.83s. Complete builder/runtime documents were independently compared and inspected before
writing the two new goldens, without blessing. Native primitive contracts passed47/47 in0.45s;
registry preflight passed1/1 with58 scenarios. This is focused evidence, not an all58 matrix result.
Exact commands/hashes/results are in tasks/project-completion/evidence/sdk-instance-limits-20261018/manifest.json.
T014-004 still needs its public HTTP malformed-scope/object-literal evidence; canonical broader
runtime, nesting and release tasks remain open. G425's old SDK-authoring limitation is resolved.


## 2026-10-18 — Public publication diagnostics and T014-004 fixtures

Public HTTP rejection coverage passes 10/10 under the real PostgreSQL application role. Each
case validates the saved draft, attempts publication, checks 422/VALIDATION_FAILED/retryable=false,
asserts response/header trace identity and independently verifies no active/published version.
Cases cover schema-forbidden terminal body nodes, escaping and recursive bodies, body-edge
cycles, overlapping parallel branches, unsupported object literals and ordinary root dangling
edges. Named-body escapes retain provenance and use INVALID_SCOPE; root dangling edges keep
GRAPH and dry-run UNKNOWN_TARGET remains stable with an additive body path.

Effective expression byte-limit failures are typed separately from scope rules. The suite
reproduced and fixed both 1202-byte body/switch expressions and a 2102-byte schema-valid body
expression rejected by the legacy 2048-byte wrapper before domain validation. All use EXPRESSION
with no expression content in the byte diagnostic. Schema validation remains first.

The three bounded independent reviews found these provenance/byte-boundary issues. Their exact
findings and reconciliation are retained; no fourth or external review was run. The final adapter
precheck was verified by tests, not another independent verdict. The initial mixed oracle log
contains a mistaken expected reason for a schema-forbidden body end, retained separately from
the corrected behavioral failures.

The four native packages passed 971 tests before the final adapter precheck; after that precheck,
all 169 infra-ir tests passed again. The47 primitive contracts passed. These counts overlap and
are not summed as unique tests. T014-004 closes its fixture scope together with the existing
all-five instance-limit evidence and the fresh10,000-reference materialization test.
Nested runtime/capacity and T014-040–044 remain open. This is not the full all 58 SDK matrix or
the workspace/Compose/advisory release gate. Exact commands, source hashes, failures and final
gate results: `tasks/project-completion/evidence/publication-scope-20261018/manifest.json`.

The final runtime-role workflow regression passed379 tests in246.25s, including exact ordered
materialization of10,000 blob references and refusal of10,001 before retaining a scope. The
separate long10,000-completion case remained ignored and is not counted as a fresh pass.

Strict locked workspace/all-target Clippy passed in1m43s. Formatting, diff and24-crate
architecture checks passed. These are local gates; shared release tasks remain unchecked.


## 2026-10-18 — Separate chained-child settlement from legacy flags

Refs: T014-014..017/040, FR-014-012–015, ADR-0030§18–19. The B0 inventory found that both
parent cancellation variants and final notification updated the first child segment's legacy
flags before the authoritative chain link. Three real PostgreSQL trigger guards reject those
flag updates after chain initialization. All 3 failed before correction (two cancellation errors;
one failed scheduler notification), then all 3 passed in 2.54s. This proves the mixed writes on
actual service/scheduler paths; no database deadlock reproduction is claimed for these guards.

The parent now updates its exact tenant/own-parent/child link and returns on a match. A child
without a link retains legacy flags with an exact own-parent predicate. Propagate/RequestCancel
and final latch/settlement behavior stay distinct; legacy metadata remains unchanged for chains.
Cancellation has exactly one request audit, and automatic final notification preserves existing
audit behavior with one child.finished journal event. Journal/projection consistency is checked
for parent and child. Existing transaction/outbox boundaries and external-effect semantics remain.

Two bounded reviews found and resolved a missing audit_records oracle, then found no concrete
source/test issues. The handoff comment/contract now state its actual chain→parent KEY SHARE→
head/link order. This leaves the existing strict-ADR metadata-guard decision and complete B0
graph pending; no nested group persistence/activation or canonical task closure is inferred.

The full fault target passed 92/92 in 91.80s, including both ordered parent/handoff races and
failed/duplicate parent notification. Durable execution passed 178/178 in 146.65s, including
legacy Propagate/RequestCancel/Detach, child races and tree budgets. Exact final commands,
source hashes, failed and broader results: evidence/child-link-only-20261018/manifest.json
under tasks/project-completion/. Counts from focused and full runs overlap.

The chain-filtered workflow integration target passed72/72 in42.27s. The existing common
chain-child and chain-cancel-parent goldens passed unchanged in all five SDKs (21.88s/12.93s
test execution), including three child segments before parent continuation and propagated versus
detached mid-chain cancellation. All ten complete normalized documents independently equal
their existing golden and are retained; no blessing, SDK/runner/normalizer change or complete
all58-matrix claim. Strict locked workspace/all-target Clippy passed in1m45s; formatting and
24-crate architecture checks passed. Final exact manifest: tasks/project-completion/evidence/
child-link-only-20261018/manifest.json. The remaining B0/ADR and release tasks stay unchecked.


## 2026-10-18 — Stable parent identity guard (ADR-0037)

The existing handoff keeps chain→parent KEY SHARE→own head→incoming link. ADR-0037 permits
this narrow identity protection under immutable initial/head/link ancestry; it grants no parent
business-state or mutation authority. Parent commands still require explicit own-run FOR UPDATE.
No executable production change, migration, SDK/API change or nested activation is introduced.

Removing only the early guard as a reversible negative control made the existing ordered race
fail 0/1: handoff committed but parent cancellation returned Unavailable. That is an observed
public error, not proof of PostgreSQL SQLSTATE/deadlock cause. Original source bytes were restored
exactly and the same two-order race passed 1/1 (3.64s). Added positive ancestry assertions passed
for three direct child segments/link (3.05s) and a depth-two handoff with distinct parent/root
(2.85s). Two independent scoped reviews resolved an unstated ancestry precondition, then found
no remaining concrete defects. This is a current-writer invariant, not a stronger FK guarantee.

Exact controls, hashes and review: tasks/project-completion/evidence/parent-fk-guard-20261018/.
The complete B0 graph, B1/group persistence, compatibility fencing and production resource/release
gates remain open. No canonical checkbox closes from this clarification.


## Exact-path chain prefix — 2026-10-18

The existing begin_current_path and begin_path helpers previously took only their own run lock.
Both now invoke the same tenant-scoped optional chain guard as full RunStore::begin, in the same
transaction before lock_path_revision. A contended chain returns None for SkipLocked; Wait keeps
the chain guard until path commit. Snapshot CAS, database-time sampling and exact-path mutation
bounds are unchanged. The additional membership lookup uses chain_segments_run_unique; it does
not load historical members. Retaining the SQL guard grants no new chain finality authority.

Four real storage-protocol tests use a trusted scoped fixture plus real singleton chain
initialization before loading the snapshot. General public chain+collection composition remains
closed. Both entry points fail the pre-fix SkipLocked and Wait oracles (0/4, 3.40s): contended chains
were ignored. With the fix, all eight bounded fence tests pass (6.51s). Actual pg_blocking_pids
barriers prove Wait blocks on chain before holding run; independent NOWAIT probes prove both
locks last through commit and are then released. Skip tests prove None/nonblocking and no
retained run lock, with ordering established by the corresponding Wait test.

Two isolated reviews identified fixture deadline/revision and lock-lifetime oracle requirements,
then found no concrete final defects. An earlier prototype RED and a fixture import compile
failure are retained separately from the corrected-fixture behavioral RED. Exact records:
tasks/project-completion/evidence/path-chain-prefix-20261018/. B0 remains incomplete; this does
not accept the other primitive/direct-Update prefixes, group transactions or general nesting.

Final regression for this increment: workflow383 passed,1 ignored in260.71s; the separate long
10k-completion test remains ignored, not a fresh pass. Strict locked workspace/all-target Clippy
passed in1m17s; formatting, diff and24-crate architecture gates passed. This is not a full
workspace execution, five-SDK matrix, Compose/advisory or nested-capacity resource result.
Exact source/results: tasks/project-completion/evidence/path-chain-prefix-20261018/manifest.json.


## Direct run Update chain prefix — 2026-10-18

Refs:T014-040/044, FR-014-014, ADR-0030§18. PgDeclaredOperationStore::apply_update now
locks a run's tenant/namespace-scoped membership chain before its exact run. A Run address
never resolves to a newer head. Chain-addressed behavior, request hashes, current state CAS,
receipt replay, audit and outbox boundaries remain unchanged. No migration or wire change.

The public HTTP regression addresses a successor distinct from the chain ID. A real chain lock
barrier must precede own-run acquisition; a test-only receipt trigger exposes guard lifetime
until commit. Before correction, the test failed because the chain wait was never observed
(0/1,7.03s); this is not a reproduced PostgreSQL deadlock. After correction, the10 HTTP operation
cases passed (5.56s), including earlier-segment RUN_CONTINUED, terminal refusal, rollback/retry,
permissions and tenant opacity. The new case checks successor mutation, unchanged predecessor,
one durable receipt/state event/outbox, two trace-bearing audits for apply+replay, and foreign
tenant NOT_FOUND. A replay is a separately audited access, not a second mutation.

Two bounded reviews required a non-singleton successor and correct replay audit counts, then
found no production defect. The second suggested a stronger SHARE probe for the chain mutation
guard; its final focused verification is recorded in the evidence manifest. Remaining primitive
adapter prefixes, post-wait authority checks and complete B0/group ordering stay open. Exact
source, failures and commands: tasks/project-completion/evidence/direct-update-chain-prefix-20261018/.

Final direct-Update regression passed26 operation cases in10.33s (includes the10 HTTP cases);
the strengthened exclusive-guard probe passed1/1 in2.23s. Strict locked workspace/all-target
Clippy passed in1m34s before that one-line test-probe refinement; production did not change.
Exact test-source variants and limits: evidence/direct-update-chain-prefix-20261018/manifest.json
under tasks/project-completion/. Broader workspace/release/nesting gates remain open.


## Chain scheduler expiry after row-lock wait — 2026-10-18

Refs:T014-040, FR-014-014/015. The chain port's expiry WHERE predicate could accept an authority
that expired while waiting for an unchanged scheduler_shards tuple. Three real PostgreSQL
barriers now cover legacy handoff, evaluated continuation and finalization. Each observes the
actual runtime-role wait while the lease is live, keeps the tuple unchanged until DB-clock
expiry, then releases it. On unchanged production all three committed under expired authority
(behavioral RED:1 pass for the overflow compatibility case,3 failures,6.47s). Earlier fixture API and
slug setup failures are retained separately and do not count as three behavioral counterexamples.

The shared execution::locked_scheduler_time helper remains transaction-typed and becomes
crate-visible. Chain checks use its lock-then-clock validation, retain own-shard rejection and
their existing oversized-generation Storage(Internal) conversion. Evaluated continuation keeps
its separate transaction_now after the incoming link lock for deadline/wait timestamps. No
lock order, wire capability, deadline policy, migration or external-effect guarantee changes.

GREEN passed4/4 (6.70s). Failed authority leaves independent chain/segments/run/journal/outbox/
audit/retained-usage snapshots unchanged. Fresh own authority succeeds, exact replay is detected
and journal consistency holds. The private chain fixture now honors the runtime-role option via
world.execution_pool. Its older owner-pool tests remain real results but did not prove runtime
privileges merely because that environment option was set; the existing explicit least-privilege
chain test was separate coverage. New wait barriers require sagat_app when the option is active.

Two bounded reviews identified generation-error compatibility, transaction typing and fixture
pool preconditions; the actual five-file review found no concrete defects. Reviewers ran no
tests. Evidence: tasks/project-completion/evidence/chain-post-wait-fence-20261018/. Collection,
loop and approval post-wait checks, their applicable chain prefixes and complete B0/group
ordering remain open. No canonical checkbox closes from this chain correction.

Final chain-expiry regression passed81 chain-filtered workflow cases (55.26s), all92 fault
cases (93.86s), and all8 bounded fence cases. These counts overlap. Strict locked workspace/
all-target Clippy, formatting, diff and24-crate architecture gates passed. No fresh full-workspace
or all-five SDK matrix is claimed. Exact source/commands, mixed/setup failures and scope limits:
tasks/project-completion/evidence/chain-post-wait-fence-20261018/manifest.json.

## Collection and loop scheduler expiry after row-lock wait — 2026-10-18

Refs:T014-040, FR-014-014/015. Both private scheduler checks now use the owning transaction's
locked_scheduler_time helper after preserving wrong-shard rejection and oversized-generation
Storage(Internal). All seven collection and six loop callers own their transaction. No new lock
prefix, deadline clock, capability, migration or nested execution eligibility is introduced.

Two runtime-role PostgreSQL entry barriers reproduced expired-authority commits after an
unchanged scheduler row wait. Clean RED was2 compatibility passes/2 failures (6.54s); GREEN
was4/4 (6.16s). Rejection leaves an atomic snapshot of17 tenant tables unchanged. The loop carry
is newly retained data, explicitly absent before entry. Fresh authority succeeds and exact
replay leaves persisted state unchanged. Collection success requires exactly one traced audit.
The initial collection attempt instead exposed a duplicate audit INSERT: enter_collection and
finish_version wrote the same record. Removing the entry INSERT retains the single atomic
finish_version audit. That audit-only correction preceded the clean RED; the earlier masked
audit failure and a SQLx static-SQL compile failure are recorded separately.

Two bounded reviews required non-vacuous new payload/audit oracles, then found no concrete
defects. New stores explicitly use runtime pools; historical owner fixtures do not acquire
least-privilege evidence from an environment flag. Existing scheduler mapping reports fenced
collection/loop outcomes with scheduler_actions_total and scheduler_fenced_total; these private
port barriers do not assert telemetry counters. Approval fencing, primitive chain prefixes,
complete B0 and all nested-capacity runtime/release gates remain open. Exact source variants,
commands and limits: tasks/project-completion/evidence/scope-post-wait-fence-20261018/.

Final focused regression passed15 collection transaction cases (10.64s),38 loop cases (22.18s)
and all92 fault cases (82.63s). Counts overlap the focused barriers. Strict locked workspace/
all-target Clippy (91.92s), formatting, diff and24-crate architecture gates passed. Exact tested
source, commands and staged evidence hashes are recorded in the scope-post-wait manifest.
No canonical task closes from this adapter correction.

## Approval scheduler expiry after row-lock wait — 2026-10-18

Refs:T014-040, FR-014-008/014. Approval open/expire now use the existing transaction-typed
lock-then-clock scheduler check. Own-shard rejection and stored_i64 generation conversion
retain their existing error precedence; user decide authorization and all business clocks stay
unchanged. The shared scope snapshot now includes telemetry_pending and all three approval
tables (21 tenant tables in one atomic SELECT).

Three new private tests use explicit runtime stores and the actual sagat_app wait oracle.
Opening and genuinely due expiry both committed after authority expiry before correction:
RED1 compatibility pass/2 failures (65.16s). The expiry fixture waits for its real60-second
business deadline using bounded75-second asynchronous PostgreSQL-clock polling, then waits
for scheduler expiry under an unchanged tuple. Its actual due timestamp supplies audit and
occurred_at together. It does not pretend a future occurred_at is a real due-time barrier.
Fresh authority opens/expires; opening success has one traced audit and expiry has one traced
audit/event/outbox plus a FIRED timer. Opening replay retains RuntimeUnavailable with no
persisted changes, matching its existing revision contract rather than inventing replay success.

The plan review required telemetry_pending coverage, correct opening replay and a deadline
wait longer than the normal five-second eventually helper. These are implemented. This bounded
correction adds no chain prefix, nested eligibility, migration, wire or public capability.
Existing scheduler_actions_total/scheduler_fenced_total map these errors to kind=approval_open
or approval_expire; the private port barriers do not test counters. Complete B0 and all nested
capacity and release gates remain open. Evidence: tasks/project-completion/evidence/approval-post-wait-fence-20261018/.

Final approval-expiry GREEN passed3/3 (67.95s, including the genuine60-second deadline).
Regression passed46 approval-filtered workflow cases (35.54s) and21 approval fault cases
(29.07s), skipping the3 already-run focused barriers. Strict locked workspace/all-target
Clippy (61.64s), formatting, diff and24-crate architecture gates passed. Two bounded reviews
ended with no concrete final defects; reviewers ran no tests. Exact source/commands/evidence:
tasks/project-completion/evidence/approval-post-wait-fence-20261018/manifest.json. No canonical
checkbox closes; complete B0, nested runtime and integrated release gates remain pending.

## Approval chain-before-run writer prefix — 2026-10-18

Refs:T014-040/044, FR-014-008/014, ADR-0030§18. Approval's common writer helper now takes its
applicable tenant/namespace/run member chain UPDATE guard before exact own-run UPDATE. It covers
open/expire/decide, including ended-outcome receipt reads, without forwarding the request or
granting head/finality authority. The unchanged direct Run Update membership SQL moves to a
transaction-typed shared execution helper; its old wrapper is removed and StoreError conversion
stays at the calling boundary. No wire, migration, group or public-shape change.

Initial membership commits with uncommitted new-run creation (execution.rs create/commit);
subsequent member identity is immutable under SQL constraints/triggers. This is not a protocol
for live reparenting or delayed external initialization. The trusted native fixture first opens
an approval, then invokes the real chain initializer with a deadline: journal revisions are
start1/open2/chain3/decision4/route5. General entrypoint/body shapes remain gated.

The real runtime decision waits on chain before holding run, observed through pg_blocking_pids
and independent NOWAIT. A test-only final-audit trigger, immediately before commit, exposes
exclusive chain and own-run guard lifetime through two55P03 probes. Success has one receipt,
one approval.decided and one step.routed, one decision outbox and one traced decision audit.
Receipt replay preserves the21-table checkpoint and chain head. Initial test tuple compilation
failure is retained separately; behavioral RED0/1 (8.37s) times out on the missing chain wait,
not a reproduced deadlock. Collection/loop prefixes and complete B0 remain pending.
Evidence: tasks/project-completion/evidence/approval-chain-prefix-20261018/.

Final approval chain-prefix GREEN passed1/1 (2.58s). Regression passed49 approval workflow
cases (55.35s, includes opening/overflow post-wait checks; skips the unchanged real60-second
due-expiry case last run atd413e5a8),28 operation cases (13.88s, including public direct
successor Update), and all92 fault cases (94.72s). Counts overlap. Strict locked workspace/
all-target Clippy (62.76s), formatting/diff and24-crate architecture passed. Two bounded reviews
ended with no concrete final defects. Exact source variants, commands and limits:
tasks/project-completion/evidence/approval-chain-prefix-20261018/manifest.json. No canonical
checkbox closes; remaining collection/loop prefixes and B0/group ordering stay pending.

## Collection/loop member-chain writer prefixes — 2026-10-18

Refs:T014-040/044, FR-014-004/006/014, ADR-0030§18. Collection lock_run/lock_drain_run and
loop lock_run now require their owning Transaction and acquire the existing scoped member-chain
guard before own-run UPDATE. All seven collection and six loop callers already own begin_safely
transactions. Their run validation, errors, revision, scheduler fence, business clocks, quotas
and drain rules are unchanged. No new group, head authority, forwarding, migration or capability.

Three trusted native runtime tests cover collection entry, start-drain following real cancellation,
and loop entry. Setup supplies the required deadline and invokes the real chain initializer;
commands use its updated revision. Before correction all three actually committed while an
independent chain UPDATE guard was held (RED0/3,3.12s), not fixture errors or SQL deadlocks.
Each barrier polls the actual mutation concurrently, observes its sagat_app chain wait and proves
its own run remains free with NOWAIT before releasing the chain. Fresh mutation and exact replay
have persisted event/outbox/state controls. This is a prefix oracle, not an independently repeated
near-commit lifetime barrier for each primitive; the shared helper's lifetime is separately
covered by approval's final-audit barrier. Transaction typing keeps the guard in the owning TX.

These are explicitly trusted storage compositions; general public chain+collection/loop shapes
remain gated. Initial membership commits with new-run creation and later identity is immutable;
no live reparent/delayed-initialization protocol is inferred. Prior performance counts are not a
fresh measurement of the extra membership lookup. Complete B0 graph, group persistence/eligibility,
nested execution and release/resource gates remain pending. Existing typed errors, outcome metrics
and sensitive-data-free logs remain mapped as before; port barriers do not assert telemetry.
Evidence: tasks/project-completion/evidence/scope-chain-prefix-20261018/.

Final scope-prefix GREEN passed3/3 (3.04s). Full workflow-primitives regression passed399
tests with1 long10k completion test ignored (300.14s), including fresh10k reference-source
materialization and the genuine60-second approval expiry barrier. All92 fault cases passed
(93.23s). Strict locked workspace/all-target Clippy (62.56s), formatting/diff and24-crate
architecture gates passed. Two bounded reviews ended with no concrete final defects; reviewers
ran no tests. Exact tree/commands/limits: tasks/project-completion/evidence/scope-chain-prefix-20261018/manifest.json.
This is not full-workspace execution or fresh production/nested100k resource acceptance; no
canonical checkbox closes. Complete B0 includes the separately recorded incident/binding and
worker-session source/test gaps, plus the remaining implicit/resource graph.

## Production incident/report lock order — 2026-10-18

Refs:T014-040/044, FR-009-004/005/008, FR-014-014. ADR-0038 explicitly refines ADR-0022's
production binding-before-run clause. Attribution now uses applicable member chain UPDATE→
exact own run UPDATE/namespace validation→existing binding EXCLUSIVE→own step UPDATE→audit
deduplication. It never forwards, changes execution state, acquires a second run or emits outbox.
Existing category/redaction/current fence/session/binding/expiry and commit-before-ACK rules
remain intact; the test-run branch is untouched. Binding revocation/registration take no runs
or chains, preserving the corrected source edge with ordinary report/heartbeat entry.

Two runtime-role tests fail unchanged production: controlled native RunTx.bound_patterns plus
public record_incident form a real cycle (PostgreSQL deadlocks0→1; incident Unavailable), and
a public published/built/leased script+continuation workflow's incident bypasses its held chain.
RED0/2,3.09s; GREEN2/2,2.12s. The first tests the exact report storage prefix, not controlled
concurrency with the whole public completion call; public completion/journal verification are
separate positive controls. The second observes actual app-role chain wait and ownrun NOWAIT
free before release, followed by audit1/completion/journal verification. No fabricated source
deadlock attribution from a generic typed error is used.

Existing script_incidents_total{category} and the destination-free committed log retain their
after-commit mapping; these native tests do not assert counters. Worker-session postwait
freshness and remaining implicit/resource edges keep B0 unchecked. No nested/group persistence,
public capability, migration or release/resource gate is inferred. Exact source/commands:
tasks/project-completion/evidence/incident-lock-order-20261018/.

Final incident-prefix GREEN passed2/2 (2.12s); the entire security integration suite passed
125/125 (61.19s), including stale/revoked/expired/duplicate/tenant/rollback and prior incident
concurrency cases. Strict locked workspace/all-target Clippy (73.93s), formatting/diff and
24-crate architecture passed. Three bounded reviews reconciled the required real chain case
and newADR refinement, then found no concrete code/document defects; reviewers ran no tests.
Exact tree/commands/limits: tasks/project-completion/evidence/incident-lock-order-20261018/manifest.json.
No new all-workspace/SDK58/Compose/advisory/resource gate is claimed. B0/group and remaining
worker-session post-wait source/test gaps remain open; no canonical checkbox closes.


## Worker session renewal freshness — 2026-10-18

Refs:T014-040/044, FR-009-001/003, FR-014-014. WorkerStore.use_session locks only the matching
tenant/id/token row, then samples database clock and renews in the same transaction if live.
Missing/expired sessions retain the existing unauthenticated mapping. No session expired during
an unchanged-row wait may extend its TTL or claim work. Existing TTL, caller-subject check and
wire remain unchanged; this sample is not continuous authority through commit. No run/group
lock or new API is added. Separate full/path claim-room and managed-database checks stay open.

Actual public-poll RED renewed an expired session and leased1 task (0/1,5.10s). GREEN1/1,5.82s
observes an actual sagat_app session wait while live, releases only after database expiry and
compares one atomic checkpoint of expiry/run version/events/outbox/active leases. A fresh
session claims successfully; old authority remains refused. No token/hash/task debug is logged.
The entire security integration passes126/126 (81.09s); strict locked workspace/all-target
Clippy (80.91s), format/diff and24-crate architecture pass. Two bounded plan/code reviews ended
with no concrete defects; reviewers ran no tests. Owner's current-review selection applies,
with no external CLI. Existing worker authentication/transport error and trace mapping is used;
these integration tests do not assert telemetry counters. No all-workspace/SDK58/Compose/advisory/
resource/nesting acceptance or canonical checkbox closure is claimed. Exact source/commands/
limits: tasks/project-completion/evidence/session-renewal-postwait-20261018/manifest.json.


## Full/exact-path claim-room session freshness — 2026-10-18

Refs:T014-040/044, FR-009-001/003, FR-014-014. Both claim-time ports now share a private
transaction-typed matching tenant/namespace/id/token/subject session guard. Sample DB clock
after row acquisition; expired/missing yields0, otherwise preserve active LEASED/RUNNING count,
conversion and saturation. Keep row guard on owning execution TX through count/lease commit;
no renewal or new lock/API/TTL policy. Preserve root run/drain→session and scoped run/drain/
binding→session separately; there is no new root binding guard. This sample does not promise
continuous validity until commit. Separate managed-database SHARE guard/B0/group remain open.

Both native-port RED cases returned8 after expiry (0/2,5.58s); GREEN2/2,7.02s. Actual known
blocker and sagat_app wait are observed before expiry, unchanged guard rolls back after DB
expiry, and zero room leaves21-table execution checkpoint and session expiry unchanged. Fresh
public registration supplies8, old remains0, verify_run passes. These are claim-time port
barriers, not publicpoll expired-lease acceptance: earlier publicpoll renewal is separate.
Regression: scoped_worker18/18 (15.62s), bounded_fences10/10 (14.04s), security126/126 (85.39s).
The10 include the focused2. Strict locked workspace/all-target Clippy113.29s, fmt/diff and
24-crate architecture pass. Two bounded reviews ended with no concrete issues after correcting
the prompt's root/scoped prefix wording; reviewers ran no tests. Existing worker.scoped.claim
trace and outcome/error mapping stay intact; port tests do not assert counters. No new full
workspace/SDK58/Compose/advisory/resource/nested acceptance or canonical checkbox closure.
Exact source/commands/limits: tasks/project-completion/evidence/session-room-postwait-20261018/manifest.json.


## Managed owner session freshness after SHARE wait — 2026-10-18

Refs:T014-040/044, FR-017-012/019, FR-014-014. Existing database_owner now retains matching
session SHARE then samples DB clock; missing/expired supplies no owner authority. Keep exact
tenant/namespace/ownrun/rootnode/subject/minor>=6 filters and binding→session→namespaceSecrets
quota→current dedicated membership order. No TTL/wire/provider/role change or new run lock.
Validity is at that postlock sample, not continuous through later catalog waits or commit.

Real PostgreSQL/OpenBao aggregate creates credential/catalog/published database query/dedicated
runner and obtains an actual public lease. Unchanged-row barrier observes known blocker and
actual sagat_app JOINsession waiter whilelive, rolls back only after DBexpiry. RED aggregate
0/1,17.20s: public heartbeat accepted and extended lease after session expiry. This proves
heartbeat authorization failure, not credential disclosure. GREEN whole connections target3/3,
28.78s: heartbeatNotBound and atomic runversion/events/outbox/audits/pins/sessionexpiry/leaseexpiry
unchanged, verify_run; public same-name reconnect preserves ID, replaces token and permits
heartbeat before continuing all prior aggregate cases. No owner SQL revives positive authority.

Fresh isolated database-runner process integration1/1,28.49s (latency measurement filtered),
with rebuilt binary; strict locked workspace/all-target Clippy67.59s, fmt/diff and24-crate
architecture pass. Two fresh bounded plan/code reviews found no actionable issues; no tests
run by reviewers or external CLI. Existing database.resolve/worker heartbeat metrics, closed
errors and after-commit audit mappings remain; native barriers do not assert counters.
Complete B0/group, fullworkspace/SDK58/Compose/advisory/resource acceptance remain pending,
with no canonical checkbox closure. Exact sources/commands/limits:
tasks/project-completion/evidence/database-session-postwait-20261018/manifest.json.


## Managed report receipt binding order — 2026-10-18

Refs:T014-040/044, FR-017-019, FR-014-014. Existing managed report replay now retains binding S
before namespace Secrets quota X and dedicated membership S. Permission, durable attempt owner,
fingerprint and current binding/role remain required, including after session removal. Live/
missing-receipt and ordinary paths retain their semantics. No new provider I/O, execution/pin/
resolution-audit/outbox write or public field. This is a focused existing017 dependency of B0.

Actual real PostgreSQL/OpenBao public receipt replay RED aggregate0/1 (17.62s) retains Secrets X
while blocked on Binding S. This ordered-prefix failure is not a three-party deadlock proof.
GREEN full connections3/3 (27.25s): known blocker/app-role wait, free Secrets probe, session
cleared, exact already-applied receipt/status, unchanged atomic runversion/events/outbox/audit/
pincounts and verify_run; public reconnect and next managed claim succeed.
Fresh isolated runner integration1/1 (28.85s, benchmark filtered), rebuilt binary; strict locked
workspace/all-target Clippy85.74s, formatting/diff and24-crate architecture pass. Sol/high
provided a read-only test draft; root inspected, extended/applied and ran all tests. Fresh
Astra/xhigh bounded code review found no concrete issues, no tests run by reviewer. Owner's
current-review selection applies, no external CLI. Existing typed errors, database/worker
metrics and committed audit mapping remain; these barriers do not assert counters.
No fullworkspace/SDK58/Compose/advisory/resource/nested/group acceptance or task closure.
Exact source/commands/limits: tasks/project-completion/evidence/managed-replay-prefix-20261018/manifest.json.


## Rerun source identity admission — 2026-10-18

Refs:T014-040/044, FR-014-012–015, T017-012/024. ADR-0039 advances exact tenant/source KEY SHARE
after existing receipt replay and before retained definition, managed catalog, active-run and
payload admission. Identity only; missing reference stays MissingReference/NotFound. Cleanup
FOR UPDATE SKIP LOCKED is unchanged. Current reruns create unrelated roots and child writers
have no rerun source: documented writer invariant, not a schema/type guarantee. No source-state
or group authority, wire field, provider I/O or external-effect guarantee is added.

The runtime-role real PostgreSQL/OpenBao barrier actually reproduced the old source/catalog
cycle: deadlocks0→1; rerun succeeded and native pin cleanup returned Unavailable. Corrected
full connections initially3/3 (27.30s). Missing source leaves atomic counts unchanged; cancelling
the source-blocked request rolls back before same-key retry; cleanup skips guarded source and
later full cleanup releases it. A real new rerun claims, resolves, completes and releases its pin;
strengthened metadata control requires the current rotated credential and rejects source version.
The first strengthened-control run hit an INT4/i64 fixture decode error, retained separately;
only test SELECT casts changed. Final exact results are in the manifest below.

Segment rerun/independent-chain regression1/1 (1.95s). Three fresh Astra/xhigh bounded plan/code
reviews; actionable SKIP/error/runtime-role/writer-invariant and stale-credential-test findings
incorporated, final no concrete issues. Sol/high read-only draft, root sole writer/test executor;
owner's current review applies, no external CLI. Existing run.rerun trace/typed error and committed
RunRerun audit/outbox/metrics mappings remain; native barriers do not assert metric counters.
No fullworkspace tests/SDK58/100k/Compose/advisory/S3/nested/group acceptance or canonical closure.
Evidence: tasks/project-completion/evidence/rerun-source-identity-20261018/manifest.json.

Final stronger-control full connections3/3 (27.36s), strict locked all-target workspace Clippy
passes2.87s, formatting4.11s, architecture24crates and diff check pass.


## Pure capacity header restoration — 2026-10-18

Refs:T014-014/017/040. Checked CapacityGroup::restore preserves identities/revision/epoch/count/
possession, validates nonzero/bounded counters and exact Active holder consistency, and rejects
malformed snapshots with InconsistentSnapshot. Maximum counters restore without reset; mutation
overflow is inert. No registry hydration or new infrastructure/domain dependency.

Test-first missing API compilation is retained, not reported as behavioral RED. Focused model9/9
(0.22s),1024histories seed3714 roundtrip after every accepted independent-registry transition,
15invalid header/fact tuples, explicit maximum preservation; internal overflow2/2. Strict domain
all-target Clippy1.00s, fmt4.05s, architecture24crates. Fresh adversarial Astra/xhigh code review
found no substantive issues; only root ran tests. No external reviewCLI, owner's current selection.
Supplied consistency is not actual database count/tenant/unique history/own-member authority. SQL
port/atomic journal/audit/outbox/OTel, mixed writers, eligibility/resource/SDK/nested acceptance
remain pending adapter/integration work; no canonical checkbox closure. Evidence:
tasks/project-completion/evidence/capacity-header-restore-20261018/manifest.json.


## Dormant capacity Finish vocabulary — 2026-10-18

Refs:T014-014/017/040. Truthful engine audit capacity.member.finished / capacity_group, without
generic repeat collapse; native receipt dedup proof remains pending. Canonical closed AsyncAPI
hint contains reference identities/result revision/epoch/retired/optional nullable trace only.
Only new baseline message added manually; all prior parsed shapes and SDK goldens unchanged.
Audit7/7, synthetic hint1/1, AsyncAPI evolution/actual existing outbound conformance4/4 (2.02s).
Initial API/fixture compile errors retained separately; corrected hint RED0/1 (0.04s) on missing
channel. Actual new producer/outbox/receipt behavior remains B2d/e; no broker delivery claim.
Pinned CLI validates0errors/0warnings plus1version info; strict locked workspace/all-target
Clippy83.89s, fmt4.04s,24-crate architecture pass. Fresh Astra/xhigh review's only finding was an
overclaiming test name, corrected and audit rerun. No external reviewCLI. Native schema/port,
SDK58/fullworkspace/resource/old-writer/nested acceptance and canonical tasks remain pending.
Evidence: tasks/project-completion/evidence/capacity-finish-vocabulary-20261018/manifest.json.


## Dormant capacity Finish schema — 2026-10-18

Refs:T014-014/017/040. Three dormant metadata tables, protected bounded classification, deferred
active-holder FK, immutable member/group tombstones, append-only Finish journal and narrow
sagat_app column privileges. Actual PostgreSQL runtime-role schema tests9/9 (18.13s), exhaustive
variable-column classification1/1 (1.79s), strict locked workspace/all-target Clippy54.85s,
fmt4.23s and architecture24crates pass. Typed55000 guarded downgrade tested empty/nonempty and
with an actual migration lock wait behind a concurrent committing writer. Fresh Astra/xhigh SQL
review found no concrete issue. Initial schema absence, compile issue and two corrected test-probe
failures remain in evidence; they are not behavioral regression proofs. No native Finish/current
child FINAL/fence/replay/producer/unknown-COMMIT evidence follows from these fixtures. Permanent
chain FKs pin cleanup until C0 retention/replay compatibility is resolved. B2c only closes locally;
canonical014, fullB1/B2, SDK58/fullworkspace/resource/old-writer/nested/release gates remain open.
Evidence: tasks/project-completion/evidence/capacity-finish-schema-20261018/manifest.json.


## Dormant native ChildRoot Finish — 2026-10-18

Refs:T014-014/017/040. Exact internal application/PG port, no production caller or group creator.
Direct exact own chain → commanded current head → one G NOWAIT → current postwait scheduler
fence; physical child lineage comes from own parent identity plus immutable link, never parent
business state. Own committed FINAL/no successor and matching terminal outcome authorize A1
Finish. Same-key canonical fingerprint replay precedes changed group/member checks and requires
current own fence; result is original receipt, not current authority. New header/member/audit/
closed outbox/receipt writes share actual commit. Server-confirmed23xxx/40001/40P01 aborts are
storage failures; other commit errors are conservatively unknown, and cancellation after dispatch
requires retained-command reconciliation. No native fault/recovery proof is inferred yet.

Actual runtime-role schema/native16/16 (36.29s), classification1/1 (1.79s), strict locked workspace/
all-target Clippy76.92s, final fmt and24-crate architecture pass. Application fingerprint vector1/1.
Independent API/adapter Astra/xhigh reviews found no concrete issue. Expanded peer test initially
failed because its helper leased/discarded a second task; corrected to poll1 without service change.
Real runtime child creation/completion/continuation drives FINAL tests. Holder/nonholder survival,
last retirement and original historical replay pass for three independent child chains (not nested
A→B→C activation). Execution journals/versions/steps/chain projection/telemetry/usage stay unchanged;
actual outbox validates canonical hints. B2e concurrency/fault/unknown-COMMIT evidence and B2f
observations remain pending, as do C0 retention compatibility, fullB1/B2 and canonical runtime/
SDK/resource/release gates. Evidence: capacity-native-finish-20261018/manifest.json under
 tasks/project-completion/evidence.


## Native capacity transaction barriers — 2026-10-18

Refs:T014-014/017/040. B2e.1 test-only proof: G Busy consumes TX and releases own chain/run before
the external G holder releases; two same-command chain waiters hold no earlier Run lock and yield
one apply plus identical replay. First application and replay wait on an unchanged shard while
DB time is live, then reject after actual DB expiry; renewed current authority succeeds. Audit/
outbox/receipt statement and deferred receipt COMMIT23514 faults atomically preserve old state,
with zero new side effects before one fresh same-key success. Injected-marker assertions exclude
unrelated storage errors. No retained-quota delta/barrier or unknown-COMMIT proof is inferred.

Fresh Astra/xhigh cycle1 found a test-role assumption, fixed by explicit sagat_app native pool;
cycle2 found no remaining concrete issue. Corrected default4/4 (13.87s), full-runtime4/4 (14.24s);
tightened marker assertion rerun1/1 in both modes (1.98s/2.02s). Strict locked workspace/all-target
Clippy8.01s, final fmt and24-crate architecture pass. No production adapter changes. B2e.2 actual
COMMIT acknowledgement loss/reconciliation and B2e.3 handoff/multiple-namespace counterexamples
remain open, as do B2f observations/fullB1/B2/canonical runtime/SDK/resource/rollout gates. Evidence:
tasks/project-completion/evidence/capacity-finish-barriers-20261018/manifest.json.


## Native capacity unresolved COMMIT recovery — 2026-10-18

Refs:T014-014/017/040. B2e.2 test-only local wire relay loses the reply while the real PG server
still executes native COMMIT behind a labelled deferred receipt advisory gate. Actual query/lock
observations prove dispatch and retained own chain/run/G/shard. Native caller returns Unknown
with the original key while the server remains unresolved. A distinct current FINAL member gets
G Busy and releases its own locks while preserving membership; same-command reconciliation blocks on the
original own-chain prefix, times out/cancels without establishing original rollback.

After gate release, the relay withholds actual committed C/COMMIT or labelled23514 abort plus Z/I.
A fresh direct runtime transaction using the unchanged original command replays commitment or
applies after confirmed abort; receipt/audit/hint end at1each, execution/usage unchanged, actual
hint canonical validation passes. This is actual native response loss, not discarded success.
One automated test executes both actual outcomes: final default1/1 (4.49s), full-runtime1/1 (4.71s).
Strict locked workspace/all-target Clippy8.06s, final fmt and24-crate architecture pass. Initial
compile-only nested import error retained. Three bounded harness reviews fixed close/startup
cancellation task ownership; final review and independent sequence review found no concrete issue.
Cleanup cancellation is source reviewed, not fault injected. No production adapter change, caller
intent implementation, process restart or quota proof. B2e.3 authority race/multi-namespace and
broader B2/canonical/SDK/resource/rollout gates remain open. Evidence:
tasks/project-completion/evidence/capacity-finish-commit-loss-20261018/manifest.json.


## Native capacity authority race and namespace boundaries — 2026-10-18

Refs:T014-014/017/040. B2e.3 test-only proof uses an actual evaluated chain handoff queued ahead of
native Finish behind an unchanged chain lock. PostgreSQL blocker observations prove ordering and
own Run remains free before the chain guard. After handoff commits, Finish rejects the old head
without capacity effects; only current successor FINAL from real worker completion may settle.
A separate test runs two same-tenant physical child chains in different namespaces in one dormant
owner-seeded G, with each holder placement. Wrong namespace rejects; first Finish preserves its
peer; last Finish retires; original replay returns its historical result after peer settlement.
Execution/usage snapshots are unchanged and receipt-linked audit/outbox carry exact namespace and
member identities, with actual canonical hint validation.

Both tests pass in default2/2 (6.50s) and full runtime-role2/2 (6.65s). Native adapter pools explicitly
authenticate as sagat_app in both modes. Separate fresh Astra/xhigh reviews of the handoff and
namespace artifacts found no concrete issue. Strict locked workspace/all-target Clippy8.12s,
fmt4.06s and24-crate architecture pass. Production source unchanged. Local B2e closes, while B2f,
fullB1/B2, durable production caller intent/process restart, C0 retention and mixed writers,
canonical runtime/SDK/resource/release gates remain pending. No nested activation. Evidence:
tasks/project-completion/evidence/capacity-finish-races-20261018/manifest.json.
