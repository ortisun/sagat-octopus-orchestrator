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
The public workflow parser still rejects approval nodes: discovery and decisions are usable only for
requests opened through the internal durable approval boundary until scheduler opening, expiry and
routing are proven end to end.

| Check | Result |
|---|---|
| `cargo test -p sagat-tests --test integration_workflow_primitives approvals_http:: --locked -- --nocapture` | Passed: 5/5 against real HTTP and PostgreSQL; includes two-page traversal, invalid cursor, viewer access, worker denial, tenant opacity, no-store and disclosure canaries |
| `cargo test -p sagat-tests --test contract_developer_experience --locked` after regenerating `docs/reference/api.md` | Passed: 8/8 documentation contracts |
| `cargo clippy -p sagat-application -p sagat-infra-postgres -p sagat-api-http -p sagat-tests --test integration_workflow_primitives --locked -- -D warnings` | Passed |
| Focused `rustfmt --check`; `git diff --check` | Passed |
