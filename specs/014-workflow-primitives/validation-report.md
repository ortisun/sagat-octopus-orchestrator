# Validation report — feature 014

> **Evidence paths.** References to `tasks/project-completion/` in this repository point at the working and
> evidence tree of the Codex session that built this work. That tree (about 1.7 million lines of raw command
> output, source snapshots and drafts) is deliberately not in git; it is preserved outside the repository
> (local ref `handoff/016-project-completion-20261024`). A tick or a claim that cites only such a path is
> historical until the named command is rerun in this tree.


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


## Dormant native capacity observations — 2026-10-18

Refs:T014-014/017/040. Local B2f adds a closed workflow.capacity.finish span under existing caller
trace, six safe reference identities plus finite returned outcome, attempts/duration labeled only
by outcome, and an unlabeled transition counter after acknowledged first commit only. Confirmed
abort, Busy/refusal, replay and unknown acknowledgement do not increment transitions. A committed
unknown followed by replay can remain uncounted in process metrics; durable receipt is authority.
No raw command/key/owner/error/business value is added to these observations. Untraced direct
calls still emit metrics but no OTLP span; cancellation has no returned-attempt outcome/counter.

Real deferred-constraint abort, G Busy, fence refusal, first commit/replay and gated native reply
loss/reconciliation produce exact outcome counts. Actual local OTLP collector validates exported
span parent/identities/outcomes/status and safe completion logs. A filtered-child regression uses
a real closed runtime pool and ensures Finish cannot overwrite its caller's outcome/error status.
RED missing counter0vs1 (3.12s) and filtered caller mutation (2.20s) are retained. Fresh three-cycle
review corrected ACK-loss test ordering and exact span targeting; final cycle found no issues.
Final focused observations default2/2 (5.08s), full-runtime2/2 (5.38s); pre-final exact-handle fix
native capacity set24/24 (64.70s), not represented as a final25-test run. Observability contracts
9/9 (0.44s), strict locked workspace/all-target Clippy81.88s, fmt4.08s, architecture24crates pass.
The transaction body is unchanged; only observations wrap it. Generic outbox routing still uses
sagat.v1.<event_type>.<tenant> with fixed event classification; no broker delivery proof is added.

Local dormant B2a-f closes, not full B1/B2 or canonical runtime tasks. Scoped settlement, production
caller intent/trace, retention/mixed writers, process restart, SDK/resource/Compose/advisory/release
acceptance remain pending. No production invocation or nested activation. Exact evidence:
tasks/project-completion/evidence/capacity-finish-observations-20261018/manifest.json.


## Integrated dependency advisory checkpoint — 2026-10-18

Refs:T014-043. cargo-audit0.22.2 (same pinned CI tool) executes cargo audit --deny warnings:
exit0 in2.29s,627locked dependencies,1338advisories from RustSec database commit
999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b. Existing .cargo/audit.toml policy is unchanged: RSA
RUSTSEC-2023-0071 and four rustls RUSTSEC-2026-0346..0349 exceptions remain explicitly documented
there and in foundation research. This result is passing under that policy, not absence of all
known advisories. No dependency/lock/policy change. Other T014-043 gates and the task remain open.
Exact lock/config/CI hashes, tool output and commands:
tasks/project-completion/evidence/dependency-advisories-20261018/manifest.json.


## Integrated canonical contract checkpoint — 2026-10-18

Refs:T014-043. Redocly1.34.21 validates canonical OpenAPI without warnings after adding existing
MIT-license metadata and six tag descriptions; initial seven warnings retained. paths/components
are byte-identical. protoc36.2 compiles worker.proto; AsyncAPI CLI3.6.0 reports0errors/0warnings/
1latest-versioninfo; AJV validates the payment-saga example with the documented custom-keyword
strict=false option. Rust primitive contracts47/47 (1.00s), before metadata-only addition; includes
canonical protobuf compilation, identity/schema/public IR contracts, not SDK client execution.
HostNode26.11.1 differs from CI22/Linux; no exact environment parity. Npx transitive dependencies
remain unpinned as in CI. T014-043 and broader release/runtime/SDK/resource gates stay open.
Evidence: tasks/project-completion/evidence/canonical-contracts-20261018/manifest.json.


## Pure successful collection shape — 2026-10-18

Refs:T014-014/017/040. B1b.2a.1b introduces only a version1 pure definition classifier for a root
collection owner and ordered selected ordinary activity body1..8→return. Frozen surrounding root/
body checks fail closed; encoded scripts, declared companions/effects, body controls/error edges,
nested/wrong owners and postcollection activities reject. Local IDs remain qualified by root
owner/body, including another body's independent effects. No version pin, source facts, database,
run state, capability or runtime activation is established.

MissingAPI compile failure is retained, not behavioral RED. Initial7/8 had an invalid short-slug
fixture; review also identified that managed DB+collection is already refused by the domain
constructor. The unreachable predicate fixture was removed, not claimed as predicate DB coverage;
the explicit defensive check remains. Final8/8 (4.30s command), strict domain/all-targetClippy10.18s,
fmt4.11s and24-crate architecture pass. Fresh Astra/xhigh cycle1 found that fixture issue, cycle2
found none. API/schema/source-writer/producer barrier/restart and broader B1/B2/runtime gates remain
open. Evidence: tasks/project-completion/evidence/collection-success-shape-20261018/manifest.json.

## Leaf certification application boundary — 2026-10-18

Refs:T014-014/017/040. B1b.2a.1c.2 adds only the fixed V1 application port and shared typed
identity/command/result/record/errors. Tests cover exact depth1 qualified identity, revision
bounds, distinct checkpoint/aggregate digests, ordered source references, overflow and wrong
return-path refusal. Constructing or structurally validating a record proves no database source,
definition pin, current authority, execution closure or capability.

Missing-module compilation failure is retained separately from two actual privacy failures:
initial Display exposed StoreError detail; review then found derived Debug exposed it too.
Both were reproduced with a synthetic canary and fixed; Display/Debug are safe while explicit
source-chain diagnostics remain internal and require caller redaction. Final7/7,5.01s command;
strict application/all-target Clippy6.76s, fmt4.10s and24-crate architecture pass. No database,
Bao, S3, scheduler, SDK or restart execution by this unit. Source schema/retention/audit/writer
and broader B1/B2/runtime gates remain open. Evidence:
tasks/project-completion/evidence/leaf-certification-api-20261018/manifest.json.

## Dormant leaf certification storage — 2026-10-18

Refs:T014-014/017/040. Migration20261211000000 adds one immutable scalar metadata table, an exact
namespace/original-run FK, source DELETE/TRUNCATE retention under D298 and original-run key
immutability, including privileged replication-role bypass. All six variable-length columns have
concrete bounded protected-system classification. Runtime has SELECT only; no source INSERT
adapter/caller. Copied fields and retained projections do not establish source UPDATE closure.

Initial4/4 fail on real missing schema42P01, retained separately from behavioral regression RED.
First4/4 pass48.20s; review identified non-discriminating identity/source coverage. Final5/5 default
42.23s and5/5 runtime10.26s verify fresh-audit duplicate identity, same-tenant namespace binding,
constraints/privileges/ALWAYS immutability, populated direct DELETE rejection for all8sources,
actual cascade isolation and own-run-before-down race. The cascade probe disables only the parent
DELETE guard in a rollback-only owner transaction; production guards remain enabled. Exact
TRUNCATE trigger kind/function/ALWAYS binding is also checked because CASCADE refusal can originate
at a descendant. Owner-created snapshots/source rows are relational fixtures, not source proofs.

Implementation review cycle2 found existing Boundary::obstruct DELETE/reseed incompatible with
retention. Direct execution of the already built test binary reproduced23001 (1.95s, binary/source
hashes retained). The fixture now reuses its row; both chain tests pass35.42s/2.21s. Final cycle3
found none. Native Finish25/25 (64.03s), classification1/1 (25.51s), startup append-only privilege
test1/1 with12grant probes (42.32s), runtime SQL/trigger privilege audit1/1 (4.48s), D298 maintenance
retention1/1 (2.65s), postfixture strict workspace/all-targetClippy8.35s, fmt4.08s and architecture
24crates0.31s pass. Native/D298/classification results precede the isolated chain-fixture-only fix;
their production sources are unchanged. No full-workspace test run, SDK/Bao/S3/100k/restart or
source-certifier acceptance follows. Evidence:
tasks/project-completion/evidence/leaf-certification-schema-20261018/manifest.json.

## Leaf certification audit vocabulary — 2026-10-18

Refs:T014-014/017/040. Dedicated LeafCollectionSuccessCertified/LeafCollectionSuccess tags map to
collection.leaf_success_certified/leaf_collection_success. The action has no generic repeat collapse,
window or distinct cap; a future canonical first-record transaction supplies exactly its own audit,
without generic dedupe suppression/advisory lock. No actual source/audit writer, hint or execution
journal event is added. Missing-variant compile failure retained, not behavioral RED. Domain audit
8/8 (4.27s), strict domain/all-targetClippy10.73s, fmt4.11s and24-crate architecture0.32s pass.
Fresh bounded Astra/xhigh review found none. Atomic source/audit/replay, producer preservation,
recovery and production activation remain pending. Evidence:
tasks/project-completion/evidence/leaf-certification-audit-20261018/manifest.json.

## Dormant leaf native source writer and bounded barriers — 2026-10-18

Refs:T014-014/017/040. Local B1b.2a.1d adds the dormant exact PostgreSQL certifier. Current chain/
originalRun and fresh post-lock scheduler authority precede canonical replay; replay precedes
creation CAS, immutable pin/profile and bounded selected-source proof. Canonical fact+noncollapsing
audit commit together, without changing source, Run, journal, usage or outbox. Storage errors expose
only sanitized SQLSTATE, malformed canonical time is refused and valid far-future shard expiry is
compared server-side. Runtime INSERT remains absent in released schema; disposable tests grant it.

Native first-increment evidence:4/4default and runtime tests,25/25shared guard regression, real
runtime SQL/trigger privilege audit and strict workspace gates; source-driver review3cycles and
source-proof review1cycle reached no remaining bounded finding. Evidence:
`tasks/project-completion/evidence/leaf-certification-native-20261018/manifest.json`.

Subsequent bounded increments use actual HTTP/gRPC execution as source and preserve release gates:

| Increment | Actual bounded evidence | Evidence directory under tasks/project-completion/evidence |
| --- | --- | --- |
| Source refusals |6tests, tenant/namespace/CAS/current authority, missing/contradictory evidence; corruption fixtures prove refusal only |leaf-certification-refusals-20261018 |
| Already stale reports |complete/fail with new keys, both observed flat originalRun queue directions |leaf-certification-late-reports-20261018 |
| Successful closure |scheduler checkpoint then actual finish_collection, both queue directions, fresh CAS and producer replay |leaf-certification-closure-20261018 |
| Unknown COMMIT |actual reply loss for commit/23514abort, retained guards and fresh direct reconstruction |leaf-certification-commit-loss-20261018 |
| Process recovery |kill/reap actual blocked native reconciler, natural backend cleanup, new OS client/current acquired authority, same identity |leaf-certification-process-20261018 |
| Later work/cancel |later completion/failure and READY cancellation both queue directions, real terminal planning with stable source/saga absence |leaf-certification-later-work-20261018 |
| Ambient binding |actual later lease expiry/session closure before proof and during blocked native COMMIT, without Run serialization |leaf-certification-binding-20261018 |

Owner corruption is not producer reachability. Native reconciliation-client restart is not a crash
of the original writer, control plane or engine. READY cancellation is not active-lease cancellation.
Chain-member source, effect-bearing later saga planning, remaining ambient/C0/source preservation,
path Finish, nested execution and canonical feature014 acceptance remain open.

## Dormant leaf returned-attempt observations — 2026-10-18

The operational span `workflow.collection.leaf.certify` inherits caller trace and carries only
qualified tenant/namespace/originalRun/owner/index plus closed outcome. Attempt counts/duration
have outcome only; `workflow_collection_leaf_certified_total` counts acknowledged first commits,
not Unknown/server-inferred success or replay. Cancellation has no returned outcome. These are
process observations, not durable counts or journal-projected workflow attempts.

Actual OTLP collector receives7native outcomes across invalid/current-authority refusals, deferred
COMMIT abort, acknowledged creation/replay and actual lost-reply commit/recovery. Metrics/histogram
counts, exact unredacted JSON completion events, empty exported status descriptions/events/links,
payload/driver/authority marker absence and filtered-child caller preservation are asserted.
First RED2/2failed missing counters; fixture SQL capture was corrected before final verification.
Three bounded fresh review cycles ended with no remaining concrete finding. Final integrated
regression passed18tests with1private subprocess helper ignored in direct selection but explicitly
executed by its parent in4OS processes;151.14s including compilation,86.61s harness.
Evidence:`tasks/project-completion/evidence/leaf-certification-observations-20261018/manifest.json`.
Strict workspace/all-target Clippy passed89.00s; formatting and architecture results are retained
in the same manifest. No production caller or released INSERT follows from these observations.

## Later effectful saga planning preserves the selected leaf — 2026-10-18

Refs:T014-014/017/040, local B1b.2a.1d.3b.5. A real public two-collection definition keeps `lines`
ordinary and declares compensateWith/reconcileWith/highRisk on completed work in `later`. Permanent
failure or cancellation with a READY successor reaches the actual drained preplanning boundary.
Deferred COMMIT gates observe the real saga planning transaction and first certification in both
orders. For certification first, the preliminary restore is explicitly queued before certification;
only after it releases can certification reach gated COMMIT, where the planner's second Run
acquisition must wait. This avoids mistaking the preliminary read for the writing transaction.
Planner first changes the creation revision and requires fresh CAS. Both cases retain exact source.

One matrix/4fresh real flows passes with actual later COMPENSATING header, COMPENSATE entry and
planning event, real line.refund compensation delivery/completion, and terminal COMPENSATED or
CANCELLED plus DONE/COMPENSATED later saga. Selected source and its exact saga absence stay fixed;
original canonical record replays and exact1fact/audit remain. First test passed73.79s before the
review's stronger stage oracle; final passed49.49s (compile34.51s; harness12.86s). Two fresh bounded
reviews ended with no concrete remaining stage/order/oracle issue. No production behavior changed.
Evidence:`tasks/project-completion/evidence/leaf-certification-saga-20261018/manifest.json`.
This closes only the known-effect later compensation unit; unknown-effect reconciliation,
active-lease cancellation, other producers/chain/C0 and released caller/INSERT remain open.

## All-five current SDK58 aggregate and native compatibility — 2026-10-18

Refs:T014-041, FR-014-016, NFR-014-004, SC-014-002/005. Registry preflight 1/1 passed 23.14 s and the
complete current-client 58-scenario aggregate passed 1/1 in 1055.10 s (harness 1054.80 s), with TypeScript,
Go, Python, C# and Java, five preparation scripts and no reuse selection/bless/scenario/language/
archived-source filters. Actual runtime database role is sagat_app. All 1238 tracked tested inputs
are hash-identical before/after. No new implementation or golden change was needed for this run.

The registry includes 6 authoring-only cases, 1 publication-only case and 51 live workflow/protocol
cases: 290 language/scenario plays, of which 255 are live. All 260 runtime/publication normalized
artifacts from this exact harness PID were copied and independently compared in full to reviewed
goldens; authoring canonical equality is enforced by the aggregate harness. No truncated/subset
comparison or new golden blessing is used. The previous failed 56-case attempt remains historical;
this new aggregate is a separate complete success and includes the 58th instance-limit case.

Fresh native `compatibility::` regression passed 8/8 in 15.02 s (harness 12.53 s), including old-schema
active-run migration/pin/journal preservation, old-worker minors with scoped withholding/current
positive control, unsupported-reader no-write behavior and compatible-reader fairness. It is not
a newly run old-engine binary; prior archived v0.1.0 client evidence remains separate. Current
phase/version/capability results are appended to docs/sdk-compatibility.md; broader nested/C0/
resource/release acceptance remains open.
Evidence:`tasks/project-completion/evidence/sdk58-current-20261018/manifest.json`.

Bounded acceptance review keeps T014-041 open: shared WP-13 expression/list boundaries and
unsupported object literals, plus WP-12 switch cases at 32/excess 33, still need five-SDK coverage.
The successful current aggregate is retained without claiming those absent scenarios.

## Current flat ordinary-leaf producer preservation inventory — 2026-10-18

Refs:T014-014/017/040; local B1b.2a.1d.3b only. The current source inventory maps each distinct
reachable mutation family to exact lifecycle/path predicates and previously executed native
barriers for stale selected reports, actual closure, accepted later work/cancel, ambient binding
and later saga planning/compensation. Heartbeat/expiry/retry/rehint/review/reconciliation command
names are distinguished as static exclusions or shared exact-addressed writers; no new independent
native race or test run is claimed for them. Separate focused inventory and fresh bounded
Astra/xhigh adversarial review found no remaining current flat family requiring another barrier.

The local producer-preservation subitem closes with the source inventory and pinned previous
actual evidence. C0/old binaries, chain-source, arbitrary privileged UPDATE, nested relationships,
release INSERT/production caller and parent source-writer/runtime gates remain open. See
`tasks/project-completion/leaf-current-producer-preservation.md` and
`tasks/project-completion/evidence/leaf-current-producer-preservation-20261018/manifest.json`.

## Shared switch case-count boundary — 2026-10-18

Refs:T014-041, WP-07/12, FR-014-009/015. Exact32-case canonical publication/execution and33-case
public-builder refusal passed in all five SDKs, focused aggregate1/1 in28.14s (harness27.32s).
All five complete normalized outputs equal the unchanged reviewed runtime golden; only authored
scenario/canonical/error expectations change. Fresh focused review found no issue. No blessing,
reuse/language/source filter, server semantic change or optional workspace gate rerun. This closes
the concrete shared case-count gap, not T014-041 or broader limit/nested/resource acceptance.
Evidence:`tasks/project-completion/evidence/sdk-switch-case-boundary-20261018/manifest.json`.

## Shared expression limits and server publication refusals — 2026-10-18

Refs:T014-041, WP-13, FR-014-016. New reviewed shared `expression-limits` canonical/runtime goldens
were authored before execution. Actual RED failed at the missing server publication-refusal phase
in 27.66 s: the server correctly returned 422/EXPRESSION for 1,025 bytes while the old harness
required publication success. First implementation compile failed because Uuid lacks Serialize;
only the optional evidence ID projection was changed to String, with no dependency flag added.

Final registry preflight passed 1/1 in 28.08 s. Expression case passed all five SDKs, 1 aggregate
in 28.46 s (harness 28.13 s); unchanged ordinary state-operations case passed all five, 1 aggregate
in 19.05 s (harness 18.75 s). Exact 1,024 UTF-8-byte evaluation includes non-ASCII content; literal
64 and constructed 4,096 succeed. Constructed 4,097 fails with the existing typed evaluation
reason, and a subsequent query sees unchanged Run/state revisions. Shared HTTP staging rejects
1,025 bytes, literal 65 and unsupported object literal documents authored by the five builders,
with independently observed zero versions/no active version. Compile-time refusals never execute.

All ten complete normalized artifacts independently equal the pre-authored reviewed goldens;
all 1,241 tracked inputs stayed unchanged throughout testing. One fresh bounded contract/fixture/
harness review found no issue. No SDK runner/API/server semantics or blessing changes. These
focused results do not claim a fresh full 59-case run or close broader nested/C0/resource gates.
Evidence:`tasks/project-completion/evidence/sdk-expression-limits-20261018/manifest.json`.

Expression harness gates: targeted Clippy with warnings denied passed in 10.03 s; workspace
format check passed in 4.31 s; 24-crate architecture check passed in 0.46 s.

## Conditional scoped Finish schema contract — 2026-10-18

Refs:T014-014..017/040; local B1b.2b.2 only. The bounded storage contract proposes complete
ChildRoot/ScopedPath tuples, original Run FK and exact scoped member receipt binding, unchanged
group-wide receipt uniqueness and permanent tombstones. Original-column-only receipt INSERT
preserves the ChildRoot writer while new scoped discriminator/identity columns remain ungranted.
Conditional down migration must refuse any scoped row and preserve legacy-only data/guards.
One fresh bounded Astra review found no issue. No executable schema, test, source consumption,
RLS, global lock safety, C0, caller or nested authority follows. Actual migration/constraint/
privilege evidence and observed downgrade locks remain separate implementation units. See
`tasks/project-completion/scoped-leaf-finish-schema.md` and
`tasks/project-completion/evidence/scoped-finish-schema-decision-20261018/manifest.json`.

## Complete current 59-case SDK aggregate — 2026-10-18

The complete current matrix passed in all five SDKs in 1,142.10 s: 59 scenarios, 295 language/case
plays, including 30 authoring-only, 5 publication-only and 260 live workflow/protocol plays.
All 265 complete normalized outputs equal the exact reviewed golden, with every 53 server-phase
case/language pair present once and all 1,241 frozen input hashes unchanged. No blessing, reuse,
archived source or scenario/language filtering. The source base is bea961c2; a later documentary
schema contract changed none of those inputs. The earlier eight native compatibility tests retain
identical executable inputs and a separately recorded conformance documentation delta.

The final public-contract mapping still leaves T014-041 open for active sibling drain after failure,
Query result-schema rejection with unchanged revisions, and successful upper bounds of 32 approval
principals/queries/updates. These are separate from the open nested/C0/resource gates. Drafts for
those public observations are outside the tested snapshot and cannot be counted as passing yet.

Refs:T014-041. Evidence:`tasks/project-completion/evidence/sdk59-current-20261018/manifest.json`.

## Shared active drain, result-schema error and upper declarations — 2026-10-18

Refs:T014-041, WP04/08/12. Registry60 preflight1/1 passed1.38s; all-five approval-builders passed
16.11s (authoring-only), expression-limits passed20.38s, collection-fail-fast-drain passed22.33s.
All ten complete live outputs equal pre-authored goldens; all1,244frozen inputs stayed unchanged.
Actual concurrent failure drains the other active worker, stops at two of three admissions and
produces no successful join. Query schema-result failure is INVALID_OPERATION_STATE with raw
full/prefix canary absent and unchanged Run/state revisions. Exactly32query/update declarations
publish;32approval principals author successfully and prior33 negatives remain.
One fresh review per fixture unit found no actionable issue. No new Rust/runner/server logic,
blessing or unchanged lint suite rerun. Full60 integration and other canonical runtime gates stay
open. Evidence:`tasks/project-completion/evidence/sdk-public-gaps-20261018/manifest.json`.

## Dormant scoped Finish schema execution — 2026-10-18

Refs:T014-014..017/040; local B1b.2b.3. The initial five schema probes were RED with42703 before
migration; the first implementation snapshot passed5/5. Final complete capacity Finish regression
passed30/30 in75.28s (harness39.41s), including the strengthened flat-Run, tenant/group/namespace,
complete nullable variants, grammar/index endpoints, group-wide keys, ACTIVE identity, FINISHED
timestamp retention and exact20-column effective runtime INSERT tests. Native ChildRoot
apply/replay, contention, uncertain-COMMIT, independent members, handoff and observations remain
successful through sagat_app. Metadata completeness1/1, actual runtime SQL privilege audit1/1
and append-only/least-privilege1/1 passed against unchanged SQL/catalog.

A previous full regression passed13 then failed PostgreSQL initialization with SSLRequest0x48,
followed by16 poisoned shared-server-mutex failures. That complete failure log is preserved;
the successful repeat used a dedicated held PostgreSQL16.15 with unique per-World databases.
Bounded SQL review had no issue; test review cycles1..3 produced concrete oracle/fixture fixes.
No fourth protocol review or external CLI. Conditional down/up and lock-before-inspection tests
remain a separate unit; no scoped writer, source consumption, INSERT grant, C0 or caller follows.
Evidence:`tasks/project-completion/evidence/scoped-finish-schema-20261018/manifest.json`.

Final schema gates: strict targeted Clippy passed8.78s, format4.10s and24-crate architecture0.35s.
Initial Clippy flagged two complex test tuple annotations; local type aliases preserve the same
SQL/result values. After that type-only change all five scoped tests passed again in38.32s
(harness4.03s), with unchanged frozen final sources through the gates. No full native rerun is
claimed for this type-only annotation delta; its complete30/30 result is retained separately.

## Fixed scoped Finish application contract — 2026-10-18

Refs:T014-014..017/040; local B1b.2b.5. The reviewed additive interface reuses the exact existing
LeafCollectionPathIdentity, validates ordinary owner/depth/index and positive SQL-range counters,
and specifies a distinct length-prefixed SHA-256 domain. Results explicitly name original_run_id;
ChildRoot head semantics remain separate. Future owning order is current authority, validated
canonical retained certificate, group-wide operation receipt, then creation CAS/active checks.
One fresh bounded Astra review found no issue. This is documentation only: no native consumer,
source proof from types, runtime permission, caller or nesting acceptance. No tests run for this
contract decision. See tasks/project-completion/scoped-leaf-finish-application-contract.md and
evidence/scoped-finish-port-decision-20261018.

## Conditional scoped Finish downgrade execution — 2026-10-18

Refs:T014-014..017/040; local B1b.2b.4. Three actual PostgreSQL tests pass37.62s (harness3.45s):
explicit sagat_app native ChildRoot apply/down-replay/up-replay with preserved legacy rows/grants;
separate ACTIVE and FINISHED+receipt retained variants refusing55000; and uncommitted original-Run
thenG writer observed blocking runs AccessExclusiveLock before inspection, whose later commit is
retained and causes55000. Migration version, scoped identity, ALWAYS guards and effective grant
survive refusal. No sleep-only lock oracle, row rewrite, source-consumption or global lock claim.
Native compatibility passes8/8 in6.32s on the current migration set. Strict targeted Clippy8.78s,
format4.12s and24-crate architecture0.32s pass. All frozen SQL/test inputs unchanged. One fresh
read-only bounded Astra review found no issue; native store uses an explicit runtime pool.
Evidence:tasks/project-completion/evidence/scoped-finish-downgrade-20261018/manifest.json.

## Canonical ordinary-leaf metadata reader contract — 2026-10-18

Local B1b.2b.7 only, refs:T014-014..017/040. The reviewed crate-private extraction accepts the
genuine shared path identity and actual locked Run pin/revision, preserving the exact existing
metadata-only query, finite timestamp guard, typed decoder and authority-before-replay ordering.
It proves stored metadata/key/pin relationships only and supplies no locks, new source proof,
protection from coherent owner-written certificates, chain-source acceptance or INSERT grant.
One independent bounded Astra review found no issue. No tests or executable extraction claimed;
actual current baselines and concrete missing metadata controls precede that next implementation.
See tasks/project-completion/canonical-leaf-reader-contract.md and
evidence/canonical-leaf-reader-decision-20261018.

## Incomplete first current60 SDK aggregate — 2026-10-18

The first unfiltered current60 attempt failed after593.74s (harness570.55s), with31 scenarios
completed and135 complete normalized outputs independently equal to reviewed goldens. At initial
managed-secret setup for database-connections, the API returned503/UNAVAILABLE. All1672 frozen
inputs remained identical. The exact same source/configuration then passed that focused case in
all five SDKs in25.53s (harness24.66s), with prepared-runner reuse only. No underlying cause was
established and no speculative product fix, write retry or golden change was added. Failed full
and passing focused logs are retained separately at
tasks/project-completion/evidence/sdk60-first-failure-20261018/manifest.json. T014-041 remains
open pending a complete current60 pass; component successes are not combined into one.

## Scoped Finish audit/hint vocabulary contract — 2026-10-18

Local B1b.2b.9 only, refs:T014-014..017/040. The reviewed vocabulary reuses the genuine capacity
membership audit action/resource with a scoped reason and proposes a distinct closed
capacity.scoped_member.finished reference message. It contains originalRun/owner/index metadata,
never invented chain/head values, and preserves the ChildRoot message/baseline. Existing closed
returned-attempt outcomes/metrics and exact span-handle redaction rules apply; no identifier metric
labels or broker/execution authority follows. One fresh bounded Astra review found no issue.
No schema, tests, native producer, privilege or caller is claimed by this documentary unit.
See tasks/project-completion/scoped-leaf-finish-audit-hint-contract.md and
evidence/scoped-finish-vocabulary-decision-20261018.


## Complete current 60-case SDK acceptance — 2026-10-18

T014-041 closes on the complete, unfiltered current five-SDK aggregate: 60 scenarios, 300
language/scenario plays, 30 authoring-only, 5 publication-only and 265 live workflow/protocol.
All 270 complete normalized outputs independently equal the reviewed golden; all 1,672 frozen
inputs are unchanged. The aggregate passed in 1,067.94 s (harness 1,067.57 s), without blessing,
prepared-runner reuse or archived source. Eight actual native compatibility tests pass on
identical executable inputs, covering active-old-run and old-worker-minor compatibility.

The first full attempt failed during managed-secret PUT with an unexplained 503, before the
five database scenario outputs. Its same-source focused retry and this subsequent full pass
are retained separately; no inferred provider cause or speculative fix is claimed. The third
and final supported-public mapping review found no further missing observation after the
last focused fixtures; stale matrix wording was corrected. Actual tagged v0.1.0 lifecycle
coverage remains separately scoped, with other historical features/versions untested.

Evidence: [complete SDK60 aggregate](../../tasks/project-completion/evidence/sdk60-current-20261018/reproduce.md).
This evidence does not accept general nested runtime, C0, production ceilings or final release gates.


## Scoped Finish application port and flat-context prerequisite — 2026-10-18

Local B1b.2b.6/.11 only, refs:T014-014..017/040. The additive public port shares the genuine
LeafCollectionPathIdentity, names original_run_id in its separate receipt result, validates
ordinary depth1 owner/index and signed counters, and uses the independent fixed152-byte SHA256
vector. Six public integration tests passed in5.42s after natural E0432 RED before module export.
Field perturbations include same-length owner changes; invalid commands refuse before hashing.
Renewable scheduler/operation key are excluded; raw StoreError stays absent from ordinary errors.
Strict all-target application Clippy8.15s, fmt4.16s and architecture0.40s pass. Two bounded
implementation reviews corrected two oracle gaps and ended without findings.

A separate fresh bounded Astra review accepts only the future consumer restriction: actual
originalRun has no parent and no exact qualified chain membership, after authority and before
source/receipt. UnsupportedRunContext is additive; no caller assertion or certificate-profile
change supplies this proof. Actual native context tests remain pending. Current child creation
always records ParentLink, but initializes a chain only with continuation policy.
No native adapter, source query, runtime grant, caller, C0 or broader nested acceptance follows.
Evidence: tasks/project-completion/evidence/scoped-finish-application-20261018/manifest.json;
contracts: scoped-leaf-finish-application-contract.md and scoped-leaf-finish-flat-context.md.


## Shared canonical leaf metadata reader — 2026-10-18

Local B1b.2b.8, refs:T014-014..017/040. The crate-private reader now accepts genuine shared
identity and actual locked Run pin/revision; exact query/binds/strict finite timestamp and
metadata/error/replay behavior are unchanged. Two owner-only corruption/exact-key controls
passed before refactoring (72.48s including compilation;2.98s harness). After extraction,
full current leaf_certification_native passed21 tests in106.65s (64.44s harness), with one
private subprocess probe ignored in direct enumeration; the parent process-recovery test
executes that probe in its own actual children. Native uncertain-COMMIT, both-order source
barriers, recovery, typed refusals and OTLP/redaction regressions pass. Targeted strict
Clippy23.71s, fmt4.34s and architecture0.34s pass; both frozen input hashes unchanged.
Two fresh bounded reviews found no issue. No metadata reader grants authority or detects
coherent arbitrary database-owner forgery; no new source producer, permission or caller.
Evidence: tasks/project-completion/evidence/canonical-leaf-reader-20261018/manifest.json.


## Scoped Finish canonical hint vocabulary — 2026-10-18

Local B1b.2b.10, refs:T014-014..017/040. Canonical AsyncAPI adds only the distinct scoped
channel/operation/closed message and one hand-authored baseline entry; all20 old entries are
unchanged. The audit enum/string is unchanged; its comment now covers scoped membership. Two
new tests were naturally RED on absent channel, then all3 hint tests pass in49.97s including
compilation. Exact signed bounds and malformed/omitted/extra payload controls pass with UUID
format validation enabled. AsyncAPI evolution/conformance4/4 passes in71.15s including build
lock wait; pinned CLI3.6.0 validates in4.25s with0errors/0warnings and one existing-version info.
Actual runtime-role existing capacity Finish regression33/33 passes in45.14s (44.61s harness),
including actual old outbox schema checks, observations, COMMIT uncertainty and scoped schema/down/up
permission guards. Strict targeted Clippy23.90s, fmt4.28s and architecture0.44s pass. Four frozen
inputs unchanged. One fresh bounded implementation review found no issue. No scoped producer,
caller, new runtime grant or production capacity eligibility is introduced.
Evidence: tasks/project-completion/evidence/scoped-finish-hint-20261018/manifest.json.


## Dormant flat scoped native Finish contract — 2026-10-18

Local B1b.2b.12, refs:T014-014..017/040. One fresh bounded Astra review found no issue in
the next owning transaction contract. Namespace-qualified binding discovery precedes the
existing applicable chain/Run/G NOWAIT/current scheduler guards; flat context and validated
canonical metadata precede operation receipt, creation CAS and A1 mutation. Closed source/
receipt decoding, guarded trace, local SQLSTATE-only diagnostics and conservative COMMIT
outcomes are specified. Three executable prerequisites are integrated; no native scoped
result, production caller or new grant is claimed yet. The contract splits first transaction,
decoding, actual barriers, unknown-COMMIT/process and observations into separate evidence units.
This closes no broader B1/B2/C0 or canonical runtime/release task.
See tasks/project-completion/scoped-leaf-finish-native-contract.md and
evidence/scoped-finish-native-decision-20261018/manifest.json.


## First dormant native scoped Finish — 2026-10-18

Local B1b.2b.13, refs:T014-014..017/040. Four runtime-role PostgreSQL tests pass in49.22s
(6.47s harness) after natural E0432 pre-export RED. The store consumes genuine native leaf
metadata under own chain/Run/G/current-fence/flat-context ordering and commits only selected
capacity contribution plus one receipt/audit/scoped hint. Actual two-index certification and
two native Finishes prove original-result replay after later G advance and legitimate Run
failure. A1 holder/nonholder/vacant/last transitions, exact member/audit/receipt time, actual
canonical hint, source/execution/nonselected snapshots, typed ordered refusals and signed
overflow are checked. Source INSERT is revoked before Finish; four extra receipt columns are
granted only in disposable fixtures, with explicit sagat_app/current_user assertions.

Two fresh implementation-slice reviews found no issue. Test review identified three actionable
oracle gaps (scalar two-iteration snapshot, CAS/inactive ordering, exact member time); all were
fixed before running, then the second fresh test review found none. Strict adapter all-target
Clippy13.62s, integration Clippy29.72s, fmt4.31s and architecture0.39s pass. All1091 frozen
Rust/SQL/Protobuf/Cargo/canonical-contract input hashes are unchanged. No runtime grant/caller
or source activation is added. Source/receipt corruption and qualification, metadata-only
reads, full context matrix, real barriers/failures, unknown-COMMIT/process and observations
remain subsequent units; no general nested/B1/B2/C0 or canonical task closes here.
Evidence: tasks/project-completion/evidence/scoped-finish-native-first-20261018/manifest.json.


## Scoped Finish source-before-receipt and metadata-only controls — 2026-10-18

Local B1b.2b.14a, refs:T014-014..017/040. Three actual runtime-role PostgreSQL tests pass in
39.89s (4.88s harness). Genuine applied receipts do not bypass a corrupted/missing canonical
certificate; stale authority still precedes source absence. Labelled owner faults restore
ALWAYS guards, compare snapshots before repair and finally recover the exact genuine row.
Native apply/replay succeeds with actual42501 read denials on11 tables, including payloads
and captured collection items, certificate SELECT/noINSERT and audit INSERT/noSELECT.
No runtime is driven after denial; owner snapshots verify no source/execution mutation.

Actual first compilationE0521 was fixed by declaring the literal-only SQL probe &'static str.
Initial nine-table3/3 pass is retained separately; reviewC1 found missing payloads/items, added
before the final run, and freshC2 found no issue. Strict integration Clippy14.80s, fmt4.28s and
architecture0.40s pass. All1092 frozen inputs unchanged. No product implementation change,
released privilege/caller, owner-corruption resistance or general decoder/fault guarantee.
Evidence: tasks/project-completion/evidence/scoped-finish-source-controls-20261018/manifest.json.


## Scoped Finish binding and receipt controls — 2026-10-18

Local B1b.2b.14b/.14c, refs:T014-014..017/040. The complete current scoped family passes12/12
in54.22s (19.48s harness):4first-native,3source,3qualification and2receipt controls. Native
qualification proves opaque tenant/namespace/group/member absence versus observable binding
mismatch, cross-namespace holder preservation without a holder certificate, and group-wide
differing-fingerprint ChildRoot conflict followed by genuine fresh-key apply/replay. Hidden
bindings deliberately mismatch to catch omission of discovery qualifiers before later lookups.

Receipt-owner fixtures retain every CHECK/FK and exact user-trigger catalog, restore ALWAYS
before calls, and exercise year10000 rejection, fingerprint precedence,9999/BC exact replay,
coherent counter/binding discrepancies and exact native restoration. Every call preserves
source/execution/nonselected members and its full before-call capacity state where required.
No owner-corruption resistance or statement-level foreign-Run read proof follows. Qualification
reviewC1/C2 found masking oracles, fixed; C3None. Receipt reviewNone. ActualE0282 HTTP RunId
helper inference failure is retained separately and fixed with shared RunId annotation. Strict
Clippy9.48s, fmt4.24s, architecture0.35s pass;1094 frozen inputs unchanged. Residual exact-FP
ChildRoot decoder control is inventoried in14d; actual barriers/recovery/observations stay open.
Evidence: tasks/project-completion/evidence/scoped-finish-binding-receipts-20261018/manifest.json.


## Current-schema decoder completion — 2026-10-18

Local B1b.2b.14/14d close after a fresh bounded inventory and matching-fingerprint cross-kind
control. A schema-valid owner-only ChildRoot receipt with exactly the scoped fingerprint
reaches decoding and returns ContradictoryProjection, preserving exact source/capacity
snapshots. A fresh key with the same scoped fingerprint applies once and its genuine scoped
receipt replays exactly. All CHECK/FK/append-only guards remain intact; this fixture does not
claim a hash collision or native ChildRoot finality.

Root's complete current scoped family passes13/13 in86.09s wall (21.12s harness), with all1095
frozen executable/contract inputs unchanged. Strict integration Clippy16.17s, fmt4.22s and
architecture0.43s/24crates pass. Fresh Astra review found no substantive issue. Inventory
separates current-schema guarantees from defensive changed-schema branches. Actual wait/fault,
commit recovery, diagnostics/OTLP units15..17 and production/C0/nesting gates remain pending.

Evidence: tasks/project-completion/evidence/scoped-finish-decoder-residual-20261018/manifest.json.


## Actual native entry barriers — 2026-10-18

Local B1b.2b.15a adds3 tests witnessing actual Run-first entry and selected-G Busy rollback
(released Run while G still held), exact duplicate commands serialized into one apply/one
original-result replay, and applicable-chain-before-Run followed by context refusal before
actually denied source/receipt reads. Chain membership is an owner-labelled context fixture,
not evidence of non-flat source closure. Source/capacity snapshots and linked fact counts
are preserved; ordering uses actual pg_stat_activity/blocking graphs and NOWAIT probes.

Root's full scoped family16/16 passes in61.22s wall (25.97s harness), strict integration
Clippy9.43s, fmt4.19s and architecture0.29s/24crates. All1096 frozen executable/contract
inputs match. Fresh bounded Astra review found none. Production adapter unchanged;15b..e,
16/17, production/C0/nesting and canonical runtime/release tasks remain open.
Evidence: tasks/project-completion/evidence/scoped-finish-entry-barriers-20261018/manifest.json.


## Post-shard-wait current authority — 2026-10-18

Local B1b.2b.15b. Two tests use real renewal to shorten the current lease, hold its committed tuple unchanged, observe the native shard-row wait while still live, and prove original Run/G guards retained with NOWAIT probes. Database-clock expiry occurs before releasing that unchanged row. Both first application and exact receipt replay return SchedulerFenceLost with exact source/capacity snapshots and no extra effects. Actual fresh owner/generation changes only renewable command authority; the unchanged logical fingerprint applies once or returns the exact original receipt.

Actual focused unchanged-row shard expiry controls 2/2 in44.33s wall (8.99s harness); strict integration Clippy9.31s, fmt4.2s, architecture0.28s/24crates. All1097 frozen executable/contract inputs match.
Fresh bounded Astra review found no remaining substantive issue.
No production implementation changed. Previous complete scoped16/16 result remains separately scoped to2426083b; this run is focused2/2, not an aggregate18-test pass. Remaining15c..e,16/17 and production/C0/nesting/canonical runtime/release gates stay open. No sleep ordering, forced expiry UPDATE during wait, or non-flat source claim.
Evidence: tasks/project-completion/evidence/scoped-finish-shard-expiry-20261018/manifest.json.


## Public context, writer ordering and confirmed atomic failures — 2026-10-18

Local B1b.2b.15c, B1b.2b.15d, B1b.2b.15e, B1b.2b.15. Three public-context tests cover ordinary child live/detached, settled FINAL child chain, and an independent parentless historical CONTINUED segment after real handoff; current authority precedes UnsupportedRunContext and actually denied source/receipt SELECT. Three writer tests cover first native certifier versus Finish in both actual Run queue orders, later worker completion/failure versus Finish in both orders, and both distinct genuinely certified member orders with actual fresh discovery/new logical keys after confirmed stale refusals. One atomic test uses four isolated worlds: audit/outbox/receipt BEFORE INSERT and deferred COMMIT 23514, each with a nontransactional owner-only marker proving the exact failure point, exact rollback snapshots and unchanged-command apply/replay. Actual source/certificate and nonselected state is preserved; legitimately changed Run/steps are not asserted unchanged.

Actual complete current scoped native family 25/25 in94.56s wall (58.70s harness); strict integration Clippy9.81s, fmt4.22s, architecture0.28s/24crates. All1100 frozen executable/contract inputs match.
Fresh bounded Astra review found no remaining substantive issue.
Production driver unchanged. Initial actual full run22pass/3fail in92.87s wall/56.83s harness is retained with identical1100 frozen failure inputs and owner-only fixture diagnostics. Two failures were children disabled in the fixture scheduler; fourth atomic workflow name deferred_commit was rejected422. After fixing those fixtures and the reviewed oracles, the complete25-test aggregate passes on the newly frozen tree. No combined partial-pass claim. Only dormant local15 closes;16/17, production/C0/callers/general nesting and canonical runtime/resource/release tasks remain open. Child and chain capacity rows are registry fixtures without canonical child-source certification. Returned canary checks cover sanitized detail/Display/Debug; actual diagnostic export remains17.
Evidence: tasks/project-completion/evidence/scoped-finish-context-writer-failures-20261018/manifest.json.


## Unknown COMMIT and cold exact-command reconciliation — 2026-10-18

Local B1b.2b.16a. One param-loop test executes actual committed and deferred23514-aborted server outcomes after losing the dispatched COMMIT reply. Actual COMMIT/pg_blocking_pids and NOWAIT probes prove original Run/G/shard guards remain held while unacknowledged. A distinct real public Run reaches G Busy and releases its Run; the same complete original command waits at the original Run. Dropping that claimant does not resolve the unknown original outcome. The relay observes actual C/E plus idle ReadyForQuery after gate release. A fresh runtime pool/store reconciles the unchanged complete command/key/expectations: committed receipt replays its exact original IDs/time/counters without changing its pre-recovery snapshot; aborted original applies once. Subsequent replay preserves the exact resolved snapshot and one receipt/audit/hint set. Genuine source state remains unchanged.

Actual focused COMMIT loss test with both actual server resolutions 1/1 in39.35s wall (4.02s harness); strict integration Clippy9.83s, fmt4.22s, architecture0.28s/24crates. All1101 frozen executable/contract inputs match.
Fresh bounded Astra review found no remaining substantive issue.
No production driver changed. The earlier focused1/1 pass at46a978de (39.37s/4.00s harness) is retained separately as pre-original-result-control, with1101 identical inputs; final stronger-oracle run is its own actual result. No invented failure or combined aggregate. The previous scoped25/25 aggregate remains separately scoped to80fc3917; this is focused1/1 with two real outcomes, not aggregate26. Process restart16b, actual observations17, production/C0/callers/nesting and canonical runtime/resource/release tasks remain open.
Evidence: tasks/project-completion/evidence/scoped-finish-commit-loss-20261018/manifest.json.


## Actual process recovery and closed returned observations — 2026-10-18

Local B1b.2b.16b, B1b.2b.16, B1b.2b.17. The actual process matrix kills and reaps a genuinely blocked reconciliation OS child while the original uncertain writer remains alive, observes actual committed/23514-aborted server resolution, changes scheduler authority through real release/acquire, and starts another OS process from the byte-identical eleven-field complete original command. The replacement discovers only scheduler owner/generation, preserves original key/CAS and returns the pre-captured original IDs/time/counters or applies once after confirmed abort; subsequent replay preserves exact capacity/source snapshots. Three current-thread observation tests exercise all six returned outcomes, actual OTLP parent/status/exact identity attributes, fixed log sequence, closed metric series/labels, raw diagnostic/source canary redaction, filtered parent safety, and abandoned-call zero returned metrics/logs/effects with exact five identity values. Only acknowledged first apply increments the transition counter.

Actual final strengthened observation controls 3/3 in6.35s wall (6.07s harness); strict integration Clippy10.95s, fmt4.19s, architecture0.35s/24crates. All1103 frozen executable/contract inputs match.
Fresh bounded Astra review found no remaining substantive issue.
No production driver changed (832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780). Native source+scoped baseline51passed/2private probes ignored in direct enumeration,170.51s wall/134.30s harness, is retained at its exact1103 frozen inputs. Both private probes actually run through their parent matrices:8 OS children total,4 killed while blocked and4 successful replacements. Final sources differ from that baseline only by abandoned-span identity assertions; focused final3/3 verifies those assertions. This is not a new aggregate54-test pass. Final-tree regressions:capacity/legacy/schema/privileges/downgrade33/33 in79.36s wall/42.91s harness, hint3/3 in0.30s, AsyncAPI4/4 in22.42s. Earlier72.99s full invocation was administratively SIGINT-superseded for review-oracle corrections; it is retained as incomplete, not a test assertion failure or completed aggregate. This proves reconciliation-client process death, not original writer/control-plane/engine restart. General B1b.2b/B1/B2/C0, source/scoped release grants, caller intent, nesting, resource ceilings and canonical runtime/release tasks remain open.
Evidence: tasks/project-completion/evidence/scoped-finish-process-observations-20261018/manifest.json.


## Genuine ordinary-child baseline — 2026-10-18

A genuine publicly started parent creates an ordinary unchained child through the production scheduler. Before source closure the native certifier refuses SourceNotClosed; actual worker completion/checkpoint/collection closure creates the source while later child work remains LEASED and unfinished. First native certification retains exact qualified source identity, pinned definition, scope/body/Return/digests/five actual event references and one truthful audit. Cold runtime pool/cache/store replay returns the complete original record. Stale own authority precedes replay; foreign tenant/namespace, missing index and parent identity cannot reuse the child evidence. Actual later work finishes child and parent while selected source/certificate stay unchanged; terminal replay preserves the entire observed snapshot. A second genuine certificate fixture still receives UnsupportedRunContext from Scoped Finish before denied source/receipt reads (actual42501 probes), with no source/execution/capacity effects. Both use explicit sagat_app and a disposable test-only source INSERT grant that is revoked before replay. No production implementation changed.

Actual focused child baseline: 2/2 default (40.1s wall) and 2/2 runtime-role (6.58s wall), the same two tests in separate invocations. Strict integration Clippy 39.21s, formatting 4.49s and architecture 0.41s/24 crates passed. All 1104 frozen executable/contract inputs match.

This closes local B1b.2c.1a only. It characterizes existing source acceptance, not new child support or complete parent-writer preservation. Cold reconstruction is not an OS restart. Parent CANCEL races, child cancellation/drain/later compensation and terminal fanout remain separately pending. All parent/chain Scoped Finish refusals remain. Broader B1/B2/C0, production source/scoped INSERT grants/callers, nested runtime, resource ceilings and canonical T014-014/015/016/017/040/042/043/044 remain open. Earlier compilation is compilation only. Initial runtime0/2 failed at public workflow creation422; shortened test fixture prefixes fixed the public Slug63 mismatch. The retained preformat run passed2/2 in each mode and Clippy/architecture, but formatting rejected one edition2024 trailing comma. Final results above are from the corrected final source, not a combined aggregate.
Evidence: tasks/project-completion/evidence/ordinary-child-source-baseline-20261018/manifest.json.


## Actual incoming-parent flag preservation — 2026-10-18

Real public parent CANCEL competes with first native certification of a genuine ordinary unchained child under PROPAGATE, DETACH and REQUEST_CANCEL in both observed child-row queue directions. Exact three-PID role/database/query/wait/blocker witnesses, parent Run conflict probes, original post-lease child CAS, policy-specific parent journal/status/step/flags, atomic audit/dedupe/outbox and immutable selected source/full child core/journal/steps/exact later lease are asserted. All six first certifications succeed without changing the captured revision. A seventh fresh fixture pauses the actual certifier COMMIT behind a deferred advisory gate; the real parent UPDATE waits behind that COMMIT while child Run, parent Run and child scheduler SHARE guards remain held. Releasing only the gate commits both. After revoking the disposable source INSERT grant, a cold runtime store/cache returns the exact original complete record/audit and preserves the full snapshot.

Actual focused child suite:4/4 default in53.65s wall/16.71s harness and4/4 runtime-role in17.43s wall/16.78s harness. These are the same four named tests in separate invocations:two existing baseline controls and two new tests executing six policy/order fixtures plus one actual COMMIT fixture. Seven asserted three-PID observations are retained per mode. Strict integration Clippy11.27s, fmt4.33s and architecture0.36s/24 crates passed. All1105 frozen executable/contract inputs match.

Local B1b.2c.1b closes only this incoming legacy flag writer family. No production code, grant, caller, profile, schema or capability changed. Later remains LEASED and unfinished; no own-child cancellation/drain, detached later progress or terminal fanout is claimed. The parent flag writers do not advance child revision/journal/updated_at. B1b.2c.1c/.1d/.1e and the master .1 remain open, as do broader B1/B2/C0, production activation, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044. Initial default invocation passed the two baseline tests and failed both new tests at the shared uppercase dedupe expectation; the six-cell loop then stopped in its first cell. Initial Clippy failed two test-only lints. All original records/source pins are retained separately, and no partial run is combined with the final suite.
Evidence: tasks/project-completion/evidence/ordinary-child-parent-flags-20261018/manifest.json.


## Actual detached-child later progress — 2026-10-18

After genuine ordinary-child source certification, real public DETACH parent CANCEL commits a terminal parent with parent_settled=true and parent_cancel=false. The original leased later activity then completes through real gRPC, retaining its exact expected output, clearing lease metadata and advancing child revision/steps/journal without stopping or finishing the child. Selected source/certificate stay unchanged. Exact parent Run/steps/journal/audit/outbox snapshot stays unchanged, with no child.finished notification. After the disposable source INSERT grant is revoked, a cold explicit sagat_app store/cache replays the original creation CAS and returns the complete original certificate/audit under current own-child authority, preserving the entire post-progress snapshot.

Actual focused DETACH later-progress test:1/1 default in38.62s wall/1.94s harness and1/1 runtime-role in2.53s wall/1.91s harness. These are separate invocations of the same named test, not two distinct cases. Strict integration Clippy11.21s, fmt4.27s and architecture0.35s/24 crates passed. All1106 frozen executable/contract inputs match.
Fresh bounded Astra review:None.

Local B1b.2c.1e only. No scheduler tick, child termination, terminal result propagation, fanout, own-child cancellation/drain/compensation, race matrix, production code/schema/profile/grant/caller or capacity activation is claimed. B1b.2c.1c/.1d/.1d.0 and master .1 remain open. The separately identified collection End/owner-result retention defect requires genuine RED and a reviewed correction; this test stops before that path. Broader B1/B2/C0, nesting/resource acceptance and canonical T014-014/015/016/017/040/042/043/044 stay open. Prior four-test child-suite results at2cdd5aaf remain separate at1105 inputs; this one-test proof at1106 inputs is not a new five-test aggregate.
Evidence: tasks/project-completion/evidence/ordinary-child-detached-progress-20261018/manifest.json.

## .1c acceptance checkpoint (2026-10-18)

Two real PostgreSQL tests pass in default mode (42.77 s wall / 4.74 s harness) and explicit runtime-role mode (5.30 / 4.66); same two names in both, no aggregate four-test claim. Actual three-PID witnesses establish both original-child writer COMMIT orders. After real public PROPAGATE, cancellation, drain and known-effect refund, selected source/certificate/audit remain unchanged and the complete original record replays cold. The child remains unfinished; End/result/fanout remain separate. Strict Clippy passed 11.69 s, fmt 4.36 s, architecture 0.34 s / 24 crates. Final 1108 source inputs unchanged. See evidence/ordinary-child-own-cancel-20261018 manifest/review/reproduce and retained initial failures. B1b.2c.1c closes only; parent umbrella, .1d/.1d.0 and canonical 014 tasks stay open.

Implementation/evidence commit: `de64eafc`. Prior child-suite, detached-progress and this two-test selection are separate source revisions and invocations, not a new combined suite. The independent collection End result and instance-limit RED regressions remain unresolved at `6ef22a85`; these successful cancellation tests stop before that path.

## Shared End preparation checkpoint — 2026-10-18

`da9201b2` closes local .1d.0b only: shared bounded End evaluator and optional closed answer on CollectionFinish, default None. Existing real result gRPC tests passed5/5 in each separate mode, final1108 inputs; strict application Clippy/fmt/architecture passed. See tasks/project-completion/evidence/collection-end-preparation-20261018/manifest.json. No native collection consumer activated; collection End/instance-limit RED and .0Q remain unresolved. No canonical task closure.

## Data-limit vocabulary and domain checkpoint — 2026-10-18

Q1 closed RunDataLimit vocabulary/domain/codec preparation and Q1a four new pure domain cases have bounded evidence in tasks/project-completion/evidence/collection-data-limit-vocabulary-20261018 and collection-data-limit-domain-20261018. Codec9/9; updated domain targets17/17,6/6,30/30 in one invocation. Strict relevant Clippy/fmt/architecture pass. Native reason production explicitly remains RuntimeNotIntegrated before transaction; full/path readers, additive schema, effective legacy accounting, real report/compensation/upgrade and native End correction are pending. No public capability/shape/grant or canonical task closure. Public empty-collection and End result/instance-limit regressions remain genuine RED.

## Collection data-limit schema checkpoint — 2026-10-18

`55ea9423` adds migration20261213000000 without changing applied migrations, grants or writer activation. Five behavioral PostgreSQL schema tests pass separately in default and runtime-role modes (49.04s/4.85s and5.14s/4.82s wall/harness), including total CHECK, old-form preservation, guarded down/up and observed down-before-writer rollback. Strict integration Clippy/fmt/architecture pass; final1112 inputs unchanged. See tasks/project-completion/evidence/collection-data-limit-schema-20261018/manifest.json, including the separately retained initial compilation failure. New-reason rows are explicit static fixtures, not native producer, replay or actual old/current upgrade evidence. Schema down eligibility never authorizes old execution binaries. Native producer, effective accounting, report preservation, scheduler lifecycle and End correction remain pending; no canonical task closes.

## Collection reader and known-effect prerequisites — 2026-10-18

`c620c2bd` and `7212a4b1` add strict new-reason full, owned-path and descendant-ancestor readers. Four focused SQL tests pass in each access mode, including ten static malformed projection cases and native finish no-write rejection; executed behavior pins and subsequent lint-only revision are separately recorded in collection-data-limit-readers-sql-20261018. The dormant path-owned nested shape is not exercised by these public depth1 fixtures, and native new-reason production/replay remains pending. Existing runtime byte_ceilings19/19 passes.

`a7dc19b3` adds the metadata-only effective-byte helper, verified read-only legacy overlay and full positive-charge guard. Bounded reviews found no remaining issue, but actual frozen old-producer overlay/guard/upgrade acceptance is still pending. It is preparation, not a deployable bridge.

`c04b786d` adds the root-only preparation command for refused known-success scoped outputs. Pure domain scoped_compensation34/34 passes, including four new ordinary/Applied identity, prior-stop, version and compensation-plan tests. PostgreSQL report atomicity/authorization and the matching scheduler consumer remain pending. `ffec26d5` adds typed audit/evidence with audit9/9 and constructor3/3, plus strict relevant Clippy/fmt/architecture. Final source pins are1114 for the domain command and1115 for audit/evidence. See their separate manifests under tasks/project-completion/evidence. No public shape, grant, production caller or canonical task closes.

## Effective-byte/native preparation checkpoint — 2026-10-18

`b0a7ea5e` shares verified E=L+V with exact-path scalar/positive guards and ScopeRun in its existing repeatable-read snapshot. Strict integration Clippy and existing runtime byte_ceilings19/19 pass, final1115 inputs unchanged. Actual old E>L snapshot/CAS/report acceptance remains pending. `012d746f` uses the effective scalar in four existing full preflights; existing real gRPC result5/5 passes separately per access mode and strict all-target application Clippy/fmt/architecture pass. These existing regressions do not establish new legacy signal/chain acceptance. See the effective-output-path-frontier-20261018 and effective-output-preflights-20261018 manifests.

`9fd9405f` adds the guarded internal native drain writer. C1 owner/checkpoint gaps were corrected; fresh bounded C2 on e0928f4f found no required issue. Strict PostgreSQL library Clippy/fmt/architecture pass with1117 unchanged inputs; executed native behavior tests are zero. The old unconditional RuntimeNotIntegrated refusal has been replaced by checked audit/evidence plus locked source/budget/stop predicates; normal scheduler/report producers remain absent. Native positive/replay/corruption/fault/runtime acceptance and End correction remain pending. This intermediate source is not a deployable bridge.

`46d3824f` corrects Q7b's unconditional report-outbox assumption to existing durable collection discovery. Report refusal creates no outgoing obligation or task release; its expected new outbox count is zero, with journal/projection/audit/receipt/telemetry atomic. The actual later drain retains its own outbox. Fresh bounded amendment review found no issue under Constitution I/VII and ADR-0002/0017. Cold normal-tick discovery without hints remains a required unexecuted gate. No public message, grant or shape, ADR divergence or canonical task closure follows.


## Actual legacy-output compatibility acceptance — 2026-10-18

Q5 is accepted with source `ce0cb786` and immutable old producer `6c44a0e4`: the complete
old/current coordinator passed in133.91s. Each of the two independent role passes contains one
real public old-producer test and one current matrix test with five ordered cases, not five tests.
The old producer always uses the runtime role; only the current native store pool varies between
owner and `sagat_app`. Read-only overlay/per-owner accounting, exact migration/read-only witnesses,
isolated corruption refusals, unclamped E>L, fitting L-only charge, zero-delta access above the
limit and positive-delta rollback passed. Stored `legacyControl` discovery metadata is allowed
only with the unchanged pinned rootForEach/full source qualification; no old source/result is
rewritten. Strict Clippy/fmt/architecture passed with1117 unchanged native inputs.

Known-success report baseline tests in `03a0043a` independently reproduce actual FAILED acknowledgements
for a real successful forward effect and a genuine Applied reconciliation; their correction and
later compensation assertions remain pending. Q8 second-owner discovery compatibility and actual
producer composition, native End/scheduler activation, full old-binary rollout, source/fanout,
nested capacity and final release gates remain open. No canonical runtime task is closed by Q5.
Evidence: `tasks/project-completion/evidence/legacy-output-view-20261018/reproduce.md` and
`scoped-output-refusal-red-20261018/reproduce.md`.
## Collection result correction checkpoint — 2026-10-18

The focused [composition evidence](../../tasks/project-completion/evidence/collection-result-composition-20261018/reproduce.md)
adds owner/End/Run result retention, actual scalar child-to-parent fanout, pinned classified HTTP
result disclosure, post-persist Finish rollback/retry and fresh-store source-bound bodyless replay.
Default/runtime selections pass, as do strict Clippy, formatting and architecture on 1,121 pinned
native inputs. Public Flow producers use sagat_app in both invocations. No production source or
scoped Finish INSERT grant/caller is introduced.

The [separate old/current gate](../../tasks/project-completion/evidence/collection-discovery-integration-20261018/reproduce.md)
passes both current store roles with unchanged old production: an effective-over-limit OPEN
collection is discovered with a live RUNNING worker, records its audited drain/stop, respects
the real worker lease and finishes without rewriting closed source. This is not old in-flight
report acceptance, mixed-writer rollout, OS restart or uncertain COMMIT. The local correction
and all eight canonical open feature014 tasks remain open for their remaining required gates.
## Report atomicity and terminal End replay — 2026-10-18

Source `6ed26a12` has [focused real evidence](../../tasks/project-completion/evidence/collection-report-replay-20261018/reproduce.md)
for report receipt fault/rollback/same-key retry, failed-expression/pre-evaluation exact native
replays, contradictory retained End bytes, prior-owner child context and the legal End result
crossing the remaining Run-byte budget. Scope success and owner output remain durable while the
Run fails; the final oracle checks physical payload absence, terminal hints, audit and head.
Current-version default/runtime results are separate from old in-flight upgrade acceptance.
All canonical open tasks remain unchecked; the later merged Clippy and release gates still apply.


### Current worker compensation requiring a refused output — 2026-10-18

Actual strict handler over real compensation lease input succeeds for 16 retained results and
fails honestly for the refused result. Public FailStep retains operator review; exact authorized
failed resolution is audited and ends COMPENSATION_FAILED. Final-source focused test passes
in default/runtime invocations (both Flow sagat_app), strict integration Clippy/fmt/architecture
pass. Full byte 23/23 passed before the named SQL result-model-only lint correction.
See `tasks/project-completion/evidence/refused-output-worker-20261018/README.md`.
This is current worker/engine behavior with fixture in-memory state, not external provider undo,
old-process upgrade or canonical task closure.


### Successful refusal report and drain fault boundaries — 2026-10-18

Actual PostgreSQL barriers prove rollback after the root-only persist and before the first
scoped journal insertion, then separately at the later drain outbox insertion. The former
rolls back the whole report and permits the same key/lease to retry; the latter preserves
the committed report/stop/ACK and rolls back only the drain. Both tests pass in default
and runtime invocations (actual Flow sagat_app), with strict integration Clippy/fmt/architecture
and 1122 matching source inputs. Normal retries drain and complete 17 actual engine compensation
tasks. Evidence: `tasks/project-completion/evidence/refusal-transaction-faults-20261018/README.md`.
No old binary, OS process, unknown-COMMIT or external-provider claim; canonical tasks remain open.


### Post-correction five-SDK collection regression — 2026-10-18

At e51b3390, existing collection-runtime passes across all five SDKs without golden changes
(one selected scenario, cached runners, runtime-role stack). Aggregate 114.62 s/harness 33.74 s;
1122 native and 582 SDK/scenario hashes match, five fresh normalized outputs match the reviewed
golden with type-preserving independent comparison. See
`tasks/project-completion/evidence/collection-sdk-regression-20261018/README.md`.
This is execution/identity regression, not result-value disclosure or a new complete 60 pass.


### Current binding recheck after the checked report path — 2026-10-18

The genuine public seventeenth-output fixture now proves NOT_BOUND from fresh full authority
after native HTTP unbind while the exact full checkpoint restoration query remains blocked.
It verifies only the intended lease change, exact binding-byte refund and traced unbind audit;
after rejection it permits only the exact report-traced mandatory policy denial audit. All
execution/payload/usage/receipt/head/outbox state otherwise remains identical. Fresh store/cache
journal reconstruction passes. Final focused 1/1 default/runtime (actual Flow sagat_app both),
strict integration Clippy/fmt/architecture and 1122 matching source inputs.
Evidence: `tasks/project-completion/evidence/refusal-fresh-binding-20261018/README.md`.
Two failed fixture contracts are retained and explained. This does not close other authority,
secret, fence, cold-process, legacy or nested capacity cases, nor canonical task checkboxes.


### Immutable-old ordinary success before current drain — 2026-10-18

The real frozen 6c44a0e4 producer/current consumer coordinator now passes all four phases
(owner and actual sagat_app). An original still-leased compensable member reports success
with its original authority/request before any current scheduler tick, at legacy E=4,200,020.
Current success is retained without output, exact receipt/replay is stable, durable discovery
starts the genuine drain and real compensation reaches COMPENSATED with matching target/key.
Closed old source witnesses and fresh native journal reconstruction pass. Final strict
Clippy/fmt/architecture and 1124 matching input hashes; two failed fixture expectations are
retained separately. Evidence: `tasks/project-completion/evidence/legacy-inflight-ordinary-v1-20261018/README.md`.
This closes only ordinary report-before-drain at E>limit. Drain-first, Applied, the other byte
condition, cold post-ACK process/unknown-COMMIT, external effects and the full Q10c/canonical
014 release gates remain open. No production nesting activation is inferred.


### Fresh full exact OpenBao version recheck — 2026-10-18

Actual OpenBao 2.7.1 public secret rotation now passes while the genuine seventeenth
completion is held at observed full checkpoint restoration after its checked path/provider scan.
Exact immutable v1 rows, one native v2 operation/version, two operation-specific audits and
physical retained-byte charge are verified. Releasing the gate returns UNAVAILABLE without
execution/refusal effects for the stale scanned vector. Same original key/lease retry accepts
SUCCEEDED/RUN_DATA_LIMIT without output/new report outbox; cold journal reconstruction and
stable replay pass. Both secret values remain absent from captured logs. Final focused 1/1
default/runtime, both actual sagat_app; strict Clippy/fmt/architecture and1125 identical inputs.
Evidence: `tasks/project-completion/evidence/refusal-secret-versions-20261018/README.md`.
Two fixture-oracle failures are retained (shared key across instances and SQL alias/column
collision). This is not a literal pre-begin fence/terminal race or an owner/runtime matrix.
Other authority, old-order/Applied, cold-process/uncertain-COMMIT, nesting and release gates
remain open; canonical014 checkboxes are unchanged.


Merged regression after8d07b0d3: full byte_ceilings26/26 (zero ignored/failures) passes
under runtime invocation in41.49s,1125 identical source hashes. This combines the recent
report/drain faults, compensation and binding controls with the byte/End suite. Evidence:
`tasks/project-completion/evidence/collection-current-merged-regression-20261018/README.md`.
OpenBao/old-current tests retain separate evidence; full014/nesting/release tasks remain open.


Current dependency advisory observation: exact CI `cargo audit --deny warnings` passes
on25a197df with existing policy, cargo-audit0.22.2,627 dependencies/1338 advisories.
No new exception or dependency change. See
`tasks/project-completion/evidence/current-dependency-advisories-20261018/README.md`.
This is not complete T014-043 acceptance.


### Immutable-old ordinary drain-before-report — 2026-10-18

Primary D1 passes all four real old/current phases in both API pool modes,149.81s.
A live original member retains its slot/reservations through actual normal drain and an extra
waiting tick; the original success stays SUCCEEDED, without output/new report outbox or journal
payload reference. Exact first stop/drain and receipt replay remain stable. Actual original-target
compensation must finish before COMPENSATED. Default report-before-drain regression also passes
all four phases147.54s. Strict Clippy/fmt/architecture and1125 identical source inputs.
Evidence: `tasks/project-completion/evidence/legacy-inflight-drain-first-20261018/README.md`.
Only ordinary E>limit ordering is accepted; cancellation-first, other byte condition, Applied,
cold-process/uncertain-COMMIT and full Q10c/canonical014 release gates remain open.


### Current scanned original-lease expiry — 2026-10-18

Actual public binding narrowing preserves the report activity but natively closes its incompatible
session and expires the original lease while full checkpoint restoration remains observed blocked.
After release, the scanned inline report returns Aborted/STALE_ATTEMPT with raw post-mutation
execution and complete tenant authority unchanged. Exact quota/audit/expiry-only deltas, original
natural-expiry control, checkpoint/catalog preservation, no refusal/receipt/outbox/head effect,
cold journal reconstruction and log redaction pass. Final focused1/1 default/runtime, both actual
sagat_app; strict Clippy/fmt/architecture and1125 identical source inputs.
Evidence: `tasks/project-completion/evidence/refusal-current-lease-20261018/README.md`.
One fixture registration-order failure is preserved. This proves current lease expiry after path
drop inside full restore, not a literal pre-begin/generation/terminal race or full Q7b closure.


### Immutable-old public cancellation before success — 2026-10-18

D2 passes four real old/current phases in198.77s, actual postgres/postgres and
sagat_app/sagat_app pools. Public CANCEL owns the exact first CANCELLED/null stop and operation
receipt before normal cancelled drain. Original live lease/reservations survive waiting; original
success records SUCCEEDED without output/report outbox and remains genuinely compensation-eligible.
Exact original report receipt/ACK and operation receipt replay preserve their separate semantics.
Only one truthful successful-output-refusal audit is allowed; all other/NULL data-limit reasons
are excluded after waiting/success/replays/finality. Real original-target/key compensation must
complete before final CANCELLED, with DONE/COMPENSATED saga and unchanged first stop/drain.
Shared default and drain-first regressions each pass four phases (173.73s/154.46s).
Final strict Clippy33.54s, fmt7.03s, architecture0.97s; each execution pins1125 unchanged inputs.
Source/evidence:5ad34370, `tasks/project-completion/evidence/legacy-cancellation-first-20261018/README.md`.
This supersedes the earlier cancellation-first pending status only. Ordinary E>limit is accepted
for these three orderings; under-limit increment, Applied, OS-process/uncertain-COMMIT recovery,
output-dependent old compensation, full Q10c and all eight canonical014 tasks remain open.


### Current canonical contracts — 2026-10-18

Evidence commit e80c061a records gates on source23127c38. All four repository CI contract commands pass: OpenAPI3.75s, Protobuf0.14s,
AsyncAPI4.54s (zero errors/warnings, one informational version notice), IR/example1.99s.
One real Rust invocation passes classification18/18, platform30/30 and primitives47/47,
zero ignored/failures,78.73s including compilation.1125 native input hashes match before/after.
Local Node26 differs from CI22 and npx resolution remains as configured; no tooling update is
claimed. Evidence: `tasks/project-completion/evidence/current-canonical-contracts-20261018/README.md`.
This is partial T014-043 evidence only; workspace/Compose/final integration and activation gates
remain separate and canonical checkboxes are unchanged.


### Supplied-pool public bootstrap prerequisite — 2026-10-18

Source/evidence c7f62b4a adds only a test-support supplied PgPool constructor and genuine public
Worker reopener. Original capability/current JWT survives exact owned-DB reconstruction; one
sagat_app backend is observed at the exact public receipt gate. Actual250000-byte persisted output,
session credential hash, independently canonical receipt digest and full replay identity pass.
Final process-guarded api_reopen:: default2/2 (3.04s) and runtime2/2 (2.66s), both API pools sagat_app;
strict Clippy18.07s/fmt4.91s/architecture1.02s and1125 unchanged inputs. Owner inspection confirms
all four terminal fixtures and removed gates. The90s inner timeout is cooperative; required final
GNU timeout supervises the whole process at180s/TERM+5s/KILL, covering synchronous lifecycle.
Evidence: `tasks/project-completion/evidence/public-pool-bootstrap-20261018/README.md`.
This accepts P0 bootstrap only. Supplied-pool scoped rollback/full reopen and actual lost-COMMIT
recovery remain separate; no Q7b/canonical task or production activation closes.


### Immutable-old fitting total / refused increment — 2026-10-18

8b8046e5 accepts only ordinary report-first E<=limit<E+candidate over genuine frozen6c44a0e4
history: L2000000,V2000020,E4000020, actual250000-byte candidate makes4250020>4194304.
Full production overlay fits without backfill; normal pre-report Waiting/candidate exclusion and
valid native ExistingOutputs refusal preserve complete execution/telemetry/saga footprint.
Original successful report stays SUCCEEDED/nooutput with exact canonical receipt/replay/zerooutbox;
fresh owned scheduler starts StoppedForRunDataLimit with its own truthful audit/outbox. Real original
target/key compensation and terminal original-ACK replay pass with retained sources/bytes unchanged.
Newcase four phases156.91s; original default/drain/cancel regressions four phases each132.53/135.47/
145.53s. All16separate phases pass with actual postgres/postgres or sagat_app/sagat_app pools,
1125identicalinputs across all before/after pairs and8created DB/private-receipt cleanups verified.
Strict Clippy42.33s/fmt5.74s/architecture0.76s/shellsyntax pass. Evidence:
`tasks/project-completion/evidence/legacy-underlimit-increment-20261018/README.md`.
Only this report-first byte condition closes locally. Under-limit prior cancellation/drain, Applied,
post-ACK OS/uncertainCOMMIT, frontier/CAS/rollout/general nesting and full Q10c/canonical014 stay open.


### Public scoped refusal after complete API reopen — 2026-10-23

Source/evidence98a1247a accepts P0b only. Real public17-member setup retains4M of sixteen outputs;
the original seventeenth250000-byte command is submitted after exact owned-DB/capability reconstruction.
An actual receipt advisory barrier ties its write to the supplied max1 sagat_app backend. First
FAILED/RUN_DATA_LIMIT plus SUCCEEDED/nooutput, original canonical receipt/ACK and byte-identical replay
preserve complete session/expiry/authority, tenant iteration ledger and retained source/payload rows.
Default1/1 (47.69s including compilation), runtime1/1 (6.63s), api_reopen2/2 (4.58s), ordinary-refusal1/1
(9.59s); no ignored tests. Root SQL independently verifies both positive databases. Final Clippy17.99s,
fmt4.89s/architecture0.39s pass;1126native hashes match all before/after pairs. The first actual0/1
fixture failure is retained: shard release expires rows instead of deleting owner history; the corrected
oracle requires0liveleases/0members and empty Placement. No production state was patched.
The2026-10-23 executions use only repository-declared Compose PostgreSQL on exclusive loopback65118
and a fresh private target outside all checkouts. Evidence/reproduction:
`tasks/project-completion/evidence/public-scoped-reopen-baseline-20261018/README.md`.
Local .5b closes only the unarmed reopener prerequisite. Actual committed/aborted COMMIT reply loss,
post-ACK process recovery, Q7b.5/master quota correction and all canonical task gates remain open.


### Actual committed public refusal response loss — 2026-10-23

Source/evidence ef12c32c accepts only current ordinary inline committed response loss. Exact receipt
Gate A selects the writer before arming; SAME supplied sagat_app PID is observed at actual COMMIT
behind deferred Gate B. After client EOF produces existing safe UNAVAILABLE/retry5000, the same
COMMIT remains unresolved, Run locked and complete pre-state invisible. Actual server COMMIT/idle
resolution precedes independent canonical receipt/Run+telemetry+journal-head/original-trace audit
inspection. The receipt-derived ACK is frozen BEFORE a second full API/direct runtime-pool reopen;
original-capability/protobuf/key replay is already-applied and leaves the full durable snapshot intact.
Exact fault default1/1 (10.03s), runtime1/1 (7.74s); preserved P0b final1/1 (48.53s including compilation),
all0ignored. Strict Clippy15.47s/fmt4.77s/architecture0.47s and1127equal native input pins pass. Root SQL
independently verifies both positive databases and removed gates/functions/proxy connections.
Evidence/reproduction: `tasks/project-completion/evidence/ordinary-public-committed-loss-20261023/README.md`.
No production change or manufactured RED; two C1 witness gaps were fixed and fresh C2 review is None.
Only local .5c closes; actual abort, OS/post-ACK recovery, wider report shapes, full Q7b.5/quota/nesting
and canonical feature acceptance remain open. A lost commit ACK may omit the acknowledgement-dependent
counter increment; no exactly-once metric or external undo guarantee is inferred.


### Actual aborted public refusal response loss — 2026-10-23

Source/evidence426629d1 accepts the current ordinary inline aborted counterpart. Exact Gate A/B/PID
selection and client UNAVAILABLE occur while actual COMMIT remains unresolved; only afterward does
the fixed deferred23514 probe abort, validated by server ErrorResponse/constraint/idle wire evidence.
Owner canonical snapshots and exact original live lease/session/head prove complete rollback BEFORE
retry and again after full API/direct-runtime-pool reconstruction. Safe actual before/after hashes and
zero receipt/stop/success/audit counts are retained before recovery changes the state. Original
protobuf/key/capability first applies once; fresh independent journal/head/trace/canonical receipt
inspection validates that ACK, then the second exact replay preserves all durable bytes. Capacity
and private quota deltas stay absent without new runtime grants. No scheduler/drain occurs.
Six separate exact cases pass with0ignored: aborted10.90/9.26s, committed8.11/8.02s, P0b7.00/7.14s
(default/runtime labels, both actual sagat_app). Clippy18.17s/separate locked build40.18s/fmt5.07s/
architecture0.46s pass;1127input hashes and executable before/after match. Root SQL independently
verifies all six final DBs and removed probes/connections. Evidence/reproduction:
`tasks/project-completion/evidence/ordinary-public-aborted-loss-20261023/README.md`.
Three fresh bounded reviews None; first actual new fault passes, with no manufactured production RED
or production/proxy/schema/grant change. Only local .5d closes. OS/post-ACK recovery, broader report
shapes, genuine-old/frontier/rollout/nesting and canonical feature acceptance remain separate.


### Genuine old pending nonfinal Return — 2026-10-23

Source/evidence b8f44488 accepts only frontier case (b). Immutable6c44a0e4 publicly
produces twenty105000-byte successes and ten210003-byte closed aggregates; c10[0]
work succeeds with JSON1 while its Return is PENDING and item1 remains unadmitted.
Actual L/V/E=2100001/2100030/4200031. Migration preserves complete projection and
closed/target source. Current normal frontier advance records first FAILED/RDL and
native DRAINING before Return checkpoint or admission, with exact journal envelopes,
original trace, existing_outputs audit and public FAILED hint. Ordinary cancellation
of Return, one real original-target/key compensation with output1 in its input, held
compensation Waiting and native COMPENSATED/FAILED-RDL finish all pass. Slots and
reservations settle to0; completed_count becomes1 on DRAINED (correcting the original
design snapshot's terminal0); no c10 checkpoint/result/owner output/End/new payload
or old-source repair. Fresh verify and terminal replay preserve the full projection.
Final new coordinator132.05s: four separate1/1 processes, actual API/store postgres or
sagat_app. Preserved default matrix186.24s: four separate1/1; its immutable old Flow
always uses sagat_app, and owner/runtime continues to describe the current reader.
Isolated old RED coordinator127.55s observes two separate0passed/1failed tests,
exit101 and exact Checkpointed marker; neither DB is consumed by current. All six
final coordinator DBs/private receipts/source directories are cleaned;1127 native
and four fixture hashes stay identical. Strict Clippy15.56s/fmt4.84s/architecture0.49s,
shell/source checks and deterministic helper-wait/credential-order probes pass.
Evidence/reproduction: `tasks/project-completion/evidence/legacy-nonfinal-return-acceptance-20261023/README.md`.
Only local Q7c.1 closes. Free-admission/live-only/boundary/competing-stop/CAS/rollout,
Q7c/Q8/master quota correction, general nesting and canonical014 tasks remain open.
No production/schema/grants/SDK or source/scoped-Finish activation changed.


### Genuine old free-admission frontier — 2026-10-23

Source/evidence528e10fd accepts only §11.4(a), local Q7c.2. After20old public105000B
successes, exact old c9Checkpointed1→Finished→c10Entered creates OPEN/WAITING with
captured[[0,1],[1,2]], progress/members/reservations/tasks0 and actual L/V/E=
2100000/2100030/4200030. Separate old controls actually admit0 and fail the expected
DrainStarted assertion. Untouched handoffs reach current: exact DrainStarted then
DrainFinished FAILED/RDL without admission, no compensation/effect flags, then fresh
store/cache verification and AlreadyFinished with unchanged full projection/source.
Complete stable Run/entry/version/snapshot/items/old history hashes, exact stop/drain/
finish envelopes/native UUID outboxes/telemetry/audit and zero obligations pass.
Final five-selector matrix: free RED121.13s, free positive146.92s, preserved nonfinal
positive132.89s, nonfinal RED119.82s, original output-view184.04s including sequential
builds. Twelve separate1/1positive processes and four separate0passed/1failed old
controls exit101 with exact markers; no old RED DB is used by current. Ten final
coordinator DBs/private receipts/source directories removed;1127native and four
fixture hashes identical. Clippy27.67s/fmt4.72s/architecture0.41s, overlay rustfmt,
shell/source checks pass. Root independently inspects all four exploratory DBs,
including actual admitted READY work in old RED and FAILED/no-member current.
Evidence/reproduction: `tasks/project-completion/evidence/legacy-free-admission-acceptance-20261023/README.md`.
Only Q7c.2 closes. Live-only exact entry/oldWaiting RED, equality/precedence/CAS,
rollout, general nesting, parent Q7c/Q8/quota and canonical014 remain open. Existing
live-worker discovery/report/compensation evidence is preserved, not invalidated.
No production/schema/grant/SDK or source/scoped-Finish activation change.

### Genuine old only-live-work frontier — 2026-10-23

Source/evidence59618184 accepts only §11.4(c), local Q7c.3. Immutable6c44a0e4
publicly retains L2100000/V2100020/E4200020 with exactly one original uncancelled
leased c10[0]/reserve and no admission capacity/stop/failure/Return/checkpoint.
Separate old controls prove public Waiting with unchanged projection/source and
fail the expected DrainStarted assertion. Untouched original19-field handoffs
reach current, whose first execution call directly returns DrainStarted; exact
scope/revision/leased entry and unchanged admission/member/identity inventories
join the existing first-stop/drain/audit/trace/hint/authority/source/payload checks.
Original success, canonical duplicate replay, real original-target/key compensation,
held-compensation nonfinality and COMPENSATED/fresh verification remain accepted.
Original discovery/report/cancel/under-limit wrappers retain their behavior.

Final frozen six-selector matrix: live RED115.14s, live positive119.38s, original
report-first115.76s, drain-first111.84s, cancel-first104.68s, under-limit104.39s,
including sequential builds. Twenty separate1passed/0failed/0ignored processes and
two independent old0passed/1failed/0ignored exit101 controls pass exact recognition;
no RED DB is consumed by current. Actual API/shared store postgres or sagat_app is
queried/asserted, with producer/current role binding. All twelve final DBs/private
receipts/source directories are removed; original26 owned harness DBs unchanged.
All1127 native/four fixture hashes identical. Strict Clippy27.59s/fmt4.68s/
architecture0.39s, shell/source checks, credential-order/TERM-resistant/held-wait-lock
probes pass. Root SQL separately checks both exploratory old Waiting and current
COMPENSATED states. Safe source snapshots and actual-private-value scans pass.
Evidence/reproduction: `tasks/project-completion/evidence/legacy-live-frontier-acceptance-20261023/README.md`.
Only Q7c.3 closes. Parent Q7c/Q8/master quota, equality/precedence/CAS, rollout,
general nesting and canonical014 tasks remain open. No production/schema/grant/SDK
or source/scoped-Finish activation change; no provider-undo/performance claim.

### Genuine old exact4MiB admission — 2026-10-23

Source/evidence52c827c7 accepts only §11.4 equality, localQ7c.4. Same public old
20-completion boundary uses19values105000B/final102137B, nine210003B aggregate
references/final207140B: actual L2097137/V2097167/E4194304exact. No tick after
completion20; exact c9checkpoint1→finish→c10entry leaves WAITING/OPEN and no members.
Dedicated18-field equality handoff preserves real source/projection across migration.
Current actual frontier reports exactE; first public advance admits0. Complete
native delta proves one stable READY instance/member/slot/reservation, admitted/
charged/active counters+1, two exact journal envelopes and one task.ready outbox.
Original history/source/output/payload/audit/receipt/ledger inventories remain exact;
no stop/drain/Return/checkpoint/item1/backfill. Fresh verify has no durable effects.
Ends at legitimate admission, not workflow completion; no poll or observed wire key.

Final frozen coordinator wall times (including sequential builds): free-admission-equal-limit 112.06s, free-admission-red 108.91s, free-admission 107.40s, nonfinal-return 108.78s, nonfinal-return-red 105.62s, output-view 170.54s.
Six selectors:16separate1passed/0failed/0ignored positives and four independent
expected old0passed/1failed/0ignored exit101 controls; exact roles/counts/markers,
archive/migration/toolchain/lock/executable/fixture/receipt hashes retained. Twelve
new final DBs/private receipts/source directories removed; original22 owned harness
DBs unchanged. All1127native/four fixture hashes identical. Clippy26.10s/fmt4.65s/
architecture0.37s, overlay rustfmt and shell/source checks pass. Initial owner tests
reached real admission but failed an incomplete expected member (two omitted null
drain columns); safe diagnostic and schema-grounded correction retained separately.
Corrected exploratory113.41s has four passing phases; fresh producer C1/current C2
reviews None. No production change or artificial equality RED; strict > already
allows equality. Evidence: `tasks/project-completion/evidence/legacy-equal-limit-acceptance-20261023/README.md`.
OnlyQ7c.4 closes. ParentQ7c/Q8/master quota, precedence/CAS/rollout/general nesting/
canonical014 remain open; source/scoped-Finish activation closed.

### Genuine old failed-member priority over excess — 2026-10-23

Source/evidence21294435 accepts only localQ7c.5. Actual20old public105000B successes
and ten210003B aggregate references retain L2100000/V2100030/E4200030. After exact
c9checkpoint1→finish→c10entry, normaloldadmit0 and originalworkerpoll produce the
real c10[0]/legacyWork lease. Nonretryable Q5_MEMBER_FAILURE FailStep plus canonical
replay yields FAILED/WAITING, no root stop, OPEN/ADMITTED/heldslot/reserved1, no
Return. Exact entry/admission/release/lease/failure/route suffix givesH=cutoff+6.
Dedicated18-field handoff binds actual role, original failed row/reported hash/
canonical lease-scoped receipt/member identities/source/events and owned DB.

Current first public advance selects that actual member failure beforeE. Exact
DrainStarted→Drained0→DrainFinished yields H+1first custom-code FAILED stop/H+2drain,
H+3member settlement, H+4collection finish/H+5RunFAILED;1/1/2 native outboxes. Full
per-phase deltas retain original trace/stop/drain/failed entry/receipt/closed source/
payload/L/V/E while releasing slot/reservation and retiring the uncreated Return.
No RDL audit, new work/checkpoint/output/item1/saga/compensation/backfill. Fresh
verify plusAlreadyFinished have no durable effects. No fabricated preexisting
FAILED Run over emptyOPEN scope; existing genuine priorCANCELLED coverage reused.

Final frozen coordinator wall times, including sequential builds: failed-member 113.40s, free-admission-equal-limit 111.96s, free-admission-red 110.62s, free-admission 113.25s, nonfinal-return 110.73s, nonfinal-return-red 104.66s, output-view 169.60s, q10-cancellation-before-report 105.16s.
SevenQ5 selectors:20positive native1passed/0failed/0ignored and four expected old
0passed/1failed/0ignored exit101 controls. Q10cancel adds fourpositive phases;
total24passing/4expectedfailures. Actual queried API/store profiles, exact count/
marker/test/source/archive/restoredmigration/toolchain/lock/binary/receipt hashes
retained. Sixteen final DBs/private receipts/source directories removed; original22
owned harness DBs unchanged. All1127native/eight fixture pins identical. Strict
Clippy26.43s/fmt4.67s/architecture0.44s, overlay rustfmt and shell/source checks pass.
First exploratory122.66s has fourpassing phases, fresh producer andcurrent/source
reviewsNone. No production change or manufactured RED. Evidence/reproduction:
`tasks/project-completion/evidence/legacy-failed-member-acceptance-20261023/README.md`.
OnlyQ7c.5 closes. ParentQ7c/Q8/Q10/quota, CAS/OS-restart/rollout/nesting/canonical014
stayopen; source/scoped-Finish activation remainsclosed.

### Genuine original-success/frontier CAS — 2026-10-23

Source/evidencef821de85 accepts only localQ7c.6. Original immutable-old ordinary
19-field handoff/session/lease/request and E4200020 are unchanged. Initial actual
frontier proves OPEN/no-stop/original live uncancelled member/no failure/Return/
capacity, exact source/revision105 and existing telemetry. Dedicated owner holderH
locks telemetry; pinned autocommit API-role observerO proves real committing
refusalR waiting there while owning RunTx, then dedicated role-correct max1
schedulerS waiting behindR at EXACT native drain run-lock SQL. Both blocking edges
and unchanged committed footprint are observed before holderROLLBACK.

Actual report commits success/nooutput plus first FAILED/RDL stop and ACKs revision
107. Public staleadvance returns exact native Primitive RevisionConflict current107;
no stale drain/audit/outbox/capacity/rejection effect. Exact member/run/tenant usage
and canonical receipt remain intact before fresh scheduling. Capture complete
original stop/event/trace before advancement; compare afterreplay/freshdrain.
Fresh store/cache publicadvance selects stopped_for_run_data_limit, preserves
stop, orders stop<completion<drain and retains trace in event/audit/hint. Refusal
counter+1; primitive rejection+0 stale/+1 fresh. Reused shared genuine compensation
checks originaltarget/key, held nonfinality, realcompletion, COMPENSATED/FAILED-RDL/
DONE-COMPENSATED, terminaloriginalACK replay and freshverify. No capability reset,
full-scheduler fallback, source repair, output/blob/charge or forward work.

Final frozen coordinator wall times, including sequential builds: report-frontier-cas 106.06s, live-work-frontier-red 101.22s, live-work-frontier 105.94s, report-before-drain 103.81s, drain-before-report 105.95s, cancellation-before-report 105.00s, under-limit-increment-before-drain 105.66s.
Seven selectors:24positive separate1passed/0failed/0ignored processes plus two
expectedoldWaiting0passed/1failed/0ignored exit101 controls. Actual queried role
binding, exactmarkers/testcounts/commands/exits/harness/source/archive/restored-
migration/compiler/lock/fixture/binary/receipt hashes retained. Fourteen final DBs/
private receipts/source directories removed; original22 harness DBs unchanged.
All1127native/eight fixture hashes identical. StrictClippy13.49s/fmt4.45s/
architecture0.31s andshell/source checks pass. C1 missing selectormap caused real
validationfailure beforeconsumer/cleanuprefusal; exactownedDB creation interval/
ordinaryfixture content independently proved and removed23→22. C1 proof gaps
(frontier/capacity/originalstop/trace/servercleanup) fixed; freshC2 reviewsNone.
First realCAS105.01s fourphases pass; unused-assignment warnings andcollapsible-if
failure/fix retained. No production fix or artificialRED. Emergency cleanup source
reviewed, injected runtime emergency acceptance not claimed. Evidence/reproduction:
`tasks/project-completion/evidence/legacy-frontier-cas-acceptance-20261023/README.md`.
OnlyQ7c.6 closes. ParentQ7c/Q8/Q10/quota/otherfaults/Applied/rollout/nesting/canonical
remainopen; source/scoped-Finish activation staysclosed.

### Genuine under-limit increment after cancellation — 2026-10-24

Source/evidence e2c0237c accepts only local Q10c.5. Immutable old6c44a0e4 produces
L2000000/V2000020/E4000020 and retains the original250000-byte success: L+candidate
fits while E+candidate4250020 exceeds4194304. Read-only full overlay agrees before
public CANCEL. Native cancelled drain precedes exact original report; accepted
SUCCEEDED/CANCELLING retains no output/new charge/successor. First CANCELLED/null
stop, original authority/capability/source, exact receipts and one refusal audit
remain; zero data-limit-drain audits. Real original-target/key compensation stays
nonfinal while held, then finishes Run/scope/owner CANCELLED, saga DONE/COMPENSATED.
Terminal original-report replay returns SAVED SUCCEEDED/CANCELLING ACK with same
command ID/already_applied, unchanged complete footprint/retained output/source/
stop/drain/authority. Fresh store/cache verification remains read-only.

Final new and ordinary cancellation selectors take107.49s/104.40s including builds:
eight separate old/current owner/runtime processes, each1passed/0failed/0ignored.
Actual API roles postgres/sagat_app; four created DBs/private handoffs removed,
original22 owned harness DBs unchanged. All1127 native/eight fixture hashes match.
Strict Clippy24.20s/fmt4.42s/architecture0.34s and Bash/source checks pass. Source
review C1 None; exploratory four-phase105.84s and compile31.54s are separate from
final counts. No production change/manufactured RED. Root executes; consequential
review GPT-6 Astra/xhigh, Sol unavailable without substitution, optional external
review previously declined. Evidence and reproduction:
`tasks/project-completion/evidence/legacy-underlimit-cancel-acceptance-20261024/README.md`.
Only local Q10c.5 closes. Applied, output-dependent compensation, post-ACK/OS/
uncertain-COMMIT compositions, Q10c/quota/rollout/nesting/canonical tasks stay open.

### Bounded Q7c and ordinary-inline Q7b.5 parent reconciliation — 2026-10-24

Fresh read-only GPT-6 Astra/xhigh audit maps Q7c directly to accepted b0a7ea5e and
.1–.6: exact effective path/positive-write guard, one read-only REPEATABLE READ
revision/stop/code/E snapshot, all three genuine-old frontiers, exact equality,
selected real cancellation/custom failure, observed original-report/native CAS
and fresh recovery. These bounded Q7c obligations close. Broader quota/report/
rollout/nesting/canonical gates remain separate.

Q7b.5 closes expressly CURRENT ORDINARY INLINE public refusal recovery: accepted
.5a–d/final426629d1 six exact processes prove committed and deferred-aborted COMMIT
reply loss plus unarmed full-reopen baseline in both invocation profiles, each
1passed/0failed/0ignored. Both profiles actually sagat_app, not owner/runtime
privilege evidence. Root rechecks saved exact results and verifies production/
public recovery test sources unchanged since426629d1. This audit runs no new
native tests; current affected cancellation regression separately passes eight
processes in e2c0237c. Separate overlapping matrices are not summed.

Bootstrap's proposed broader Applied P1 was explicitly context-only/unaccepted;
subsequent accepted .5c/.5d isolate ordinary-inline. Applied response loss,
post-ACK cold scheduler/OS recovery, remaining authority/fence/terminal controls,
genuine-old Applied/output-dependent/recovery compositions stay under Q7b/Q10c.
No public unknown-outcome enum, production source/scoped-Finish activation or
canonical/master closure is inferred. Evidence with exact mapping/limitations:
`tasks/project-completion/evidence/quota-parent-closure-audit-20261024/README.md`.
Historical “parent open” notes describe their then-current state; this scoped audit
supersedes them only for Q7c and current ordinary-inline Q7b.5.

### Genuine-old Applied-ready handoff prerequisite — 2026-10-24

Source/evidence 299f8b2e accepts only local Q10c.6a. Actual old public ten210000B
completions retain L2100000/V2100020/E4200020 with absent owner outputs. Separate
Applied workflow/binding/session publicly fails original reserve OUTCOME_UNKNOWN;
normal old scheduler/poll produces real reserve~r lookup. Its target/key match
ORIGINALforward, notlookup. Distinct private21-field handoff retains originalforward
andlookup protobufs, exact Appliedrequest and full authority hash; noSQLstatefab.

Strict canonicalwire/request and BOTH run/node/generation taskIDs, original fence/
reportedhash/requestedevent, lookup LEASED/live/uncancelled and forward WAITING/
RECONCILIATION hold. Before/after migration/startup/clientconstruction/freshrole
frontier+verify_run: completeprojection/source/fullmembership/bindingtimestamps/
poolgrant/session EXACTexpiry and bounded audit/receipt/telemetry/saga/usage/
policy/quota/activecounter footprints are unchanged. SQL E and nativefrontier4200020
agree; no checkpoint/success/conclusion/drain/reportreceipt/compensation/newoutput.
Current reconciliation/refusal/primitive counters stay unchanged. Client signs
original bytes but sends NO current RPC: authentication/Appliedacceptance are NOT
claimed by attachment.

Final affected coordinator times including builds: applied-handoff 103.53s, report-before-drain 103.67s, under-limit-increment-cancellation-before-report 105.67s, live-work-frontier-red 101.73s.
Four selectors produce12separate1passed/0failed/0ignored positives plus two isolated
oldWaiting0passed/1failed/0ignored exit101 controls. Actualqueried APIroles postgres/
sagat_app, oldRED API/store rolechecks. Eight created DBs/privatehandoffs/source
folders removed; same22previousownedDBs. All1128native/elevenfixture-docpins match.
StrictClippy24.32s/fmt4.42s/architecture0.34s, finalseparateparserunit1/1 andBash/
overlay/sourcechecks pass. Actualfixture RED ordinary1passed/currentkind1failed
native101/outer1 in95.39s; priorcompile/setupfailures excluded. SourceC1 fixes absent
oldcolumns, uncheckedtaskIDs, omittedcapacity/pool/audit witnesses. Coherentmutated
syntheticwireunit observedRED33.73s thenGREEN33.82s; actualAppliedfirstfourphase
102.84s exploratory predates finalstrengthening. FreshsourceC2 None/focusedfourcase
scopeaccepted. Root executes; reviewGPT-6Astra/xhigh, Sol unavailable/no substitution,
optionalexternalreviewdeclined. Exact evidence/reproduction:
`tasks/project-completion/evidence/legacy-applied-handoff-acceptance-20261024/README.md`.
Only prerequisite closes; actualAppliedreport/refusal/drain/compensation/oldquotaRED,
otherorders/bytes/outputdependent/OS/COMMIT recovery, Q10c/quota/rollout/nesting/
canonicaltasks stayopen. Source/scoped-Finish production activation remainsclosed.

### Genuine immutable-old Applied quota RED — 2026-10-24

Source/evidence 10b61669 accepts only localQ10c.6b/B1, upstreamA299f8b2e. Applied-only
endmode reaches genuineoldlookup, submits exact40-byte retained Appliedrequest before
handoffpublication. ActualoldAPI identitylookups increase; ACKSUCCEEDED/RUNNING,
bothstepsSUCCEEDED, originalforward retains exact10B{"hold":7}, lookupoutputNULL.
L2100010/V2100020/E4200030 withnoRDLstop/audit; returnedPENDING, no postreporttick/
checkpoint/saga/compensation. Originalauthority EXACTexpiry/poolgrants/closedsource
preserved; actualconcludedApplied andlookupcompletion/fences, nofakeforwardcomplete.
OriginalLOOKUPreceipt scope/hashofwhole40wire/resource/created_at is capturedwith
safeSHA; ACK/head/version/telemetry aligned. Onlythen emitexactrole/DB/proofmarkers
andfail REQUIREDcurrentrefusal (RDLstop/nooutput): native101,onefailure/zeroignored.
No handoff/currentconsumer, even ifguardunexpectedlypasses. SeparateB2 usesfreshDB.

Finalfivecoordinator times includingbuilds: applied-report-before-drain-red 145.9s, applied-handoff 133.4s, report-before-drain 122.89s, under-limit-increment-cancellation-before-report 136.32s, live-work-frontier-red 121.55s.
Twelve separate positive1passed/0failed/0ignored processes andfour expectedold
native101/0passed/1failed/0ignored (twoAppliedquota,twoWaiting). Outercoordinator0
validatesexpectedfailure, notcurrentGREEN. Actualpostgres/sagat_appAPI/sharedstore
roles andpositivehandoffbindings queried. TencreatedDB/privatehandoffs/sourcefolders
removed; original22ownedDBinventory unchanged. All1128native/twelvefixture-docpins
match. StrictClippy30.45s/fmt5.27s/architecture0.68s/standaloneproducerfmt/Bash pass.
SourceC1/C2None; root actualsource-markercontrol REDsuffixacceptance thenApplied-only
Fxqfix gives12positive/negativechecks withoutSQL/DROP. Firstexploratory101.21s/two
nativeexpectedfailures keptseparate. Sourceimplementationroot whileSolunavailable;
consequentialreviewsGPT-6Astra/xhigh, optionalexternalreviewdeclined. Directowned
containerinspection running/healthy/127.0.0.1:65118 afterread-onlyComposequerymissing
interpolationvars; noinfrachange. Exactevidence/reproduction:
`tasks/project-completion/evidence/legacy-applied-quota-red-acceptance-20261024/README.md`.
OnlyB1closes; currentAppliedreport/refusal/drain/comp/replay, otherorders/bytes/
fault/OS/COMMIT recovery/outputdependentcomp/quota/Q10c/rollout/nesting/canonical
remainopen; productionactivation staysclosed/noexternalprovidereffectorundoguarantee.


## Capacity eligibility after a policy reduction (B1c) — 2026-10-26

Decision only; no production code. The owner answered O701: a reduction does not pause admitted work
(`docs/owner-decisions.md`). A pause-based pure model built first (`d917a54f`) was therefore withdrawn
and removed; it is history, not a delivered capability. Decision record and open obligations:
[capacity-eligibility.md](./capacity-eligibility.md). Canonical tasks T014-014..017, 040, 042-044 stay
unchecked. Evidence of the withdrawn model, for the record: red first (13 failed against a stub, then 3
more after review), 17 green, eight named mutations killed, domain suite 561 passed; two read-only
reviews found no code defect, and the architecture review exposed the conflict with the released
reduction contract that led to O701. Gates re-run on the tree after removal are listed in the commit
that removes it.

## 0.3 scoped timers and explicit zero-policy progress — 2026-10-27

The ongoing implementation ledger is [implementation-0.3.md](./implementation-0.3.md).
Timers: core1017passed, focused native35passed plus separate2-test full/frontier journal/
existing-source oracle; common five-SDK scenario64 verified without regeneration; strict
workspace Clippy passed. These are separate invocations, not one combined count. Timer
fault92passed; the following integration phase was deliberately stopped because a concurrent
build replaced the executable before Cargo launched it. An immutable-checkout replacement
is running; no full native pass is claimed for that mixed-source invocation.

O701: domain and public API zero refusals reproduced before change. Additive migration
20261222000000 widens the existing policy CHECK; retained zero blocks downgrade under the
policy-table lock. Five focused native tests pass, including absent-row INSERT/source/clear,
accepted timer progress at zero, fresh-admission refusal/resumption and both actual writer/
downgrade lock orders. The table validation/rollback rollout windows are documented in
[zero-iteration-policy.md](./zero-iteration-policy.md). Bounded correction review, complete
contracts/lint, fixed-source full regression, released-profile rollout and resource workloads
remain pending. All eight canonical tasks remain unchecked; no release is accepted.

### Frozen timer regression completed

At19c60a86 the isolated integration target passed580/0failed/16ignored in2584.38s
harness (2693.95s command, wall gap0.01s). The independent fault target passed92/0/0.
The mixed-source interrupted attempt remains invalid for integration acceptance. Later
migration22/zero policy and pure event replay are separately tested increments and are not
covered by this frozen binary. The complete release gate still requires the final tree,
64-scenario conformance, released-profile rollout and remaining resource/nested behavior.


### Root event dispatch preparation

The app-role page/scan regressions reproduced both existing scope-isolation defects before
fixing the three queries. The reviewed tests pass2/0/0,3.90s harness; existing root dispatch
regression passes24/0/0,66.34s harness. Sentinels are append-only storage fixtures retained
until owned DB teardown, not a scoped runtime proof. Event type/key/since and namespace/tenant
CAS controls preserve legitimate root marking. Qualified delivery/start/timeout, activation
fencing and public scoped waits remain pending. Full64 on38d7b3f0 initially failed for a
missing database-runner prerequisite; the prerequisite build and focused five-language database
scenario now pass, with full64 retry still running. See the implementation ledger for exact
source, executable hashes, command timings and retained failures. No canonical task closes.


### Qualified event candidates, receiver still closed

The candidate port now carries canonical path/node plus stored UUID, and its explicit SQL
cursor tuple is preserved by scheduler pagination. Root receipt construction, wake and lookback
refuse nonroot candidates. A current root wake checks UUID against its pinned version/run;
scan CAS also pins physical/logical identity. Four application and four app-role native tests
pass after three genuine RED assertions (legacy receipt construction, cursor and UUID/path CAS).
Existing root consumer/trigger metadata is unchanged. No scoped consumer or receipt is built,
and no native start/timeout/receiver or public capability is enabled. Exact source gate results
and limitations are in the implementation ledger; final integrated release gates remain open.


### Pure explicit-clock scoped event start

The start-only command passes15 focused tests, including varied keys/clocks and ten malformed
Pending projection fields in both models. Full domain passes574/0/0 across42targets in38.79s;
strict domain Clippy passes12.08s. The initial pending-command RED and one journal-expectation
failure are retained in the implementation ledger. Read-only review found no required issue.
This is not native subscription, timeout, event receipt or public capability evidence; callers
still need database time, current scheduling authority and evaluation-snapshot CAS. No canonical
task is closed by this preparation increment.


### Complete64-scenario SDK checkpoint

Immutable38d7b3f0 passed64scenarios across all five SDKs,1aggregate/0failed/0ignored,
1396.68s harness/1399.68s command. Root independently confirmed290complete normalized outputs
from58scenarios equal their exact goldens; six authoring-only scenarios remain separately
identified. No bless or selection filter; prepared SDK reuse after the initial missing-runner
failure is explicit. [Command, log, hashes and limitations](./evidence/sdk64-20261027/README.md)
are retained. This strengthens timer/control/recovery and zero-policy snapshot evidence; it is
not acceptance for later event preparations, native scoped events, nesting or release0.3.


### Pure explicit-clock scoped event timeout

Full domain passes582/0/0 across42targets in37.29s after23focused event-wait tests.
Strict domain Clippy passes11.11s; read-only review found no required issue. The timeout
uses the recorded deadline/pinned route, preserves paused deferred routing, and refuses
stale/foreign/malformed/stopped sources. Pure ordering and boundary properties do not replace
native PostgreSQL races or database-time/current-authority proofs. No native subscription,
receiver, public shape or canonical task is enabled by this increment.


### Exact-context correlation evaluator

The additive internal port preserves root delegation and defaults legacy adapters to scoped
refusal. The real interpreter uses its existing exact-path/lazy context and key conversion.
Corrected full application+IR passes437/0/0 across36targets in5.13s, including19evaluator tests;
strict all-target lint for those crates passes2.09s. The unsupported-scoped RED, two required
test corrections, and already-green computed-key masking diagnostic are recorded separately
in the implementation ledger. Final review found no required issue. No native start caller,
public body or admission/receipt authority is enabled by this evaluator preparation.

### Native scoped event-wait start storage

The real admitted-activity → Pending-wait structured-codec fixture now proves full/path start
storage and correlation failure with exact binding/lookback and own-shard database time.
The corrected focused runtime-role gate passes11/0/0 in40.06s harness/97.72s command; full
domain583, contracts49, timer13, control23, common fences10 and root dispatch24 pass in separate
invocations. Strict workspace/all-target Clippy passes130.39s. Two real review/TDD defects were
fixed: generic timeout-code failure bypassed clock authority, and all three readers accepted
wrong namespace/UUID. Physical source changes without a Run revision and journal-trigger
failure are refused/rolled back without repairing their source. Read-only corrections are
reviewed; exact commands, failures, counts, source limits and executable SHA are in the ledger.
All-services sagat_app is explicit for final native/regression profiles. This uses the named
internal structured codec, not public publication or released-profile proof. Native timeout,
receiver/drain, nesting/capacity lifecycle and final release gates remain open; no task closes.

### Native scoped event-wait timeout storage

Actual due-deadline full/path storage failed2before the whitelist correction, then passed13,
19 and20successive focused cases under sagat_app. The20-case gate passed72.95s harness/
192.06s command; the corrected isolated authority case passed separately1/0/0 in4.77s/
56.62s. The negative-test coverage correction and source provenance are explicit in the ledger.
The tests cover handled/unhandled/local error routes, subscription+scan cleanup, actual pause/
audited resume with explicit cold routing once, late projection abort with journal rollback,
and key/type/coherent-window source CAS without Run revision. Strict adapter+tests lint passes;
read-only mechanism review found no required issue. This is private structured-codec/native-port
evidence. Scheduler production/discovery, external receipt/value handling, cancellation/drain,
old-dispatcher cutover and final SDK/rollout/resource/release acceptance remain open.

### Scoped event-wait collection drain

The new native cancellation case first failed with a run stuck CANCELLING. The reducer, both
scheduler eligibility paths and independent PgCollectionStore writer now agree on a workerless
event suspension, validate its current complete locked binding and canonical identity, clear the
subscription/scan fields and retire reservations/slots atomically. A reviewed failure-entry
replay mismatch was reproduced and fixed before acceptance; Pending-entry behavior is retained.

Full domain587/0/0 across42target summaries passes36.59s command. The combined native lifecycle
filter passes26/0/0 in97.65s harness/97.66s command. Native drain6 then final7cases additionally
cover full/frontier journal parity, actual sibling-expression failure, both held-Run timeout/
cancel orders, source corruption, current authority/revision, duplicate receipt and late
post-cleanup journal failure with rollback of tenant capacity as well as all previous observations.
The final7pass21.89s harness/97.83s command; this is separate from26, not33distinct cases.
Strict workspace Clippy90.15s and final test-target lint26.93s pass. Runtime-role regressions
pass timers13, controls23 and native transactions15 on the same executable; exact timings,
hashes, fixture/compile failures and review correction are in implementation-0.3.md.
Formatting, architecture and agent checks pass. No canonical task, loop event drain, external
event receiver/producer/public body, nested-capacity or release acceptance is closed.

### Pure scoped event satisfaction and canonical lower bound

Typed full/path commands now match the exact current instance/type/key and inclusive received
window, preserve paused deferred routing and first-commit semantics, and leave both models
unchanged on failure. The checked lower-bound calculation is shared with native metadata
validation; root legacy behavior remains separate. The focused gate passes39/0/0 after two
review findings were reproduced and corrected (partial mutation on route failure; wrong typed
root/unadmitted error). Final core domain+application+IR passes1035/0/0 across78targets in65.44s
command, strict workspace Clippy106.70s and contracts49/0/0 pass. Before the final pure-only
corrections, native lifecycle27passes96.51s harness with the same canonical window helper;
that is separate source evidence. The ledger retains every RED/compile/coverage correction.
No native receipt/payload authority, WaitSatisfied persistence, discovery/producer/public body
or canonical runtime/release task is enabled by this pure preparation.

### Shared canonical event document preparation

The shared application encoder preserves the old JsonEvaluator bytes and extension interface.
New factory cases first failed2while the existing exact-byte evaluator baseline passed1; all3
then pass. Full application+IR440/0/0 across38targets passes71.33s command, root dispatch24/0/0
passes57.61s harness/105.00s command under sagat_app, and strict workspace Clippy passes82.80s.
Read-only compatibility review found no issue. This is representation reuse, not new receipt,
payload authority, ingress validation or public scoped-event acceptance.


### Native scoped receipt and canonical retained value

The internal full/path transaction ports now pin the current own Run, subscription and inbox
admission before staging an exact receipt and its canonical document. WaitSatisfied and its
one output require the private same-transaction witness. Commit refuses orphan receipts,
post-consumption ignored errors and prior failures hidden by successful pre-receipt retries.
Real REDs, bounded-review corrections, fixture errors and the discarded zero-test invocation
are retained in implementation-0.3.md; none is relabeled as passing acceptance. No grants,
migration, public body or production dispatcher caller changes.

Final native lifecycle43/0/0(186.67s harness/248.91s command), root dispatch24/0/0
(65.38s/135.51s), contracts49/0/0(0.60s/2.81s), strict workspace Clippy132.03s pass. Native tests
use actual admitted scoped activity → wait and EventService ingress/claim under sagat_app.
They include full/path identity/value isolation, malformed conflict metadata/current claims,
whole-transaction rollback on a current retained quota, paused cold explicit routing/duplicate
with one logical charge and actual classified journal disclosure under pinned rules1 schemas
(public/PII/secret). This is an internal structured-codec checkpoint, not public or released
upgrade proof. Native event/outcome/cleanup races and producer/receiver activation, rollout,
nested capacity, final SDK/resource/release gates and all eight canonical tasks remain open.


### Native scoped event outcome and cleanup ordering

On production80ae824a, held Run locks prove both event/timeout and event/cancellation orders;
actual public cancellation queues behind the event-first commit, and native cancellation-first
refuses the queued event before its receipt. Real scheduler drain/replay verifies finality.
Actual purge_events waits for the receiver's inbox lock, then removes inbox/receipt while the
retained run value and replay survive. Cleanup-first refuses stale claims in both writers.
A prepared bounded entry queued behind timeout must return typed RevisionConflict. Exact
backend identity/blocking edges and bounded joins replace timing sleeps. Seven cases pass
16.58s/66.49s; the tightened typed-error assertion passes1/0/0 in2.45s/52.89s separately.
Current-row fingerprint/JSON-object corruption and existing PostgreSQL size-bound coverage
pass1/0/0 in2.40s/3.06s after a documented fixture-setup error. Test-target strict lint passes
16.29s before the final assertion tightening. No production code, public activation or
canonical release task changes; broader faults, production callers and rollout remain open.


### Collection event subscription and deferred local routing

Both scheduler read models now start exact scoped subscriptions with captured-instance key
evaluation, current revision/own authority and acknowledged database clock, and release deferred
valid local outcomes. Genuine start/deferred-failure REDs and bounded review corrections are
recorded in the0.3 ledger. Unrouted Failed waits with onError defer collection failure while
no Run stop is committed, stay waiting under pause and take the declared local route on resume.
No domain failure predicate or public body guard is weakened. Existing collection entry lease
fencing already refuses placement-free calls; its new baseline test required no production fix.

Eight caller cases pass36.04s/89.58s, including actual COMMIT quota refusal and separately
injected persistence-time typed quota, failed-transaction rollback and current scope drain.
Final lifecycle59/0/0(229.10s/229.84s), controls23/0/0(86.53s/86.54s), contracts49/0/0
(0.68s/5.27s), strict workspace lint63.81s and architecture/agent checks pass. Timer regression
is retained in the ledger. These are immutable structured-codec runtime fixtures under sagat_app;
production deadline/receiver activation, released-profile/public/SDK/nested-capacity/resource
acceptance and all eight canonical tasks remain open.


### Qualified automatic event deadlines

Due discovery and fair selection preserve complete instance identity. Scoped consumers use
bounded revision/current own-shard/database-clock authority; root singleton behavior remains
covered. Both collection readers preserve paused unrouted failures until local routing,
including the unhandled no-onError case. Seven native deadline cases include actual ticks,
no-placement/stale-generation controls, three paused/resume policies, cold duplicates and an
observed held-event/queued-deadline entry race. Native lifecycle66/0/0 (259.49s/260.23s), root
dispatch24/0/0 (60.83s/109.78s), contracts49/0/0 (0.56s/2.57s), workspace lint72.31s and static
checks pass. REDs, the root-activity fixture correction, scoped review and source hash are in
the implementation ledger. External delivery, public/old-reader rollout and canonical release
acceptance remain pending; none of the eight open tasks closes here.


### Scoped production event receiver and fair lookback recovery

The dispatcher now consumes qualified candidates through current bounded path entry and the
native canonical receipt/value witness. Root receipts keep their legacy identity. Current
lookback/scan CAS, full tuple paging and a process-local bounded keyset cursor preserve pending
failed work without monopolizing the head page. Selected-admission loss leaves a scan pending;
post-admission settlement errors retry instead of finalizing the event. Nine focused tests
pass21.35s/96.38s; the0.3 ledger retains genuine REDs, fixture corrections and scoped reviews.

The completed executable passes79/0/0 (346.74s/346.76s), root dispatch24/0/0 (70.83s/211.27s),
contracts49/0/0 (0.63s/2.49s), migration checks5/0/0 (9.37s/10.28s). Index-only migration
20261223000000 keeps the old index and roundtrips admitted waits. The initial32pass/1fail
privilege audit exposed unsupported shared SQL macros; separate519dfa82 fixes the fitness
function without grants/exceptions, with42/0/0 (14.30s/43.50s) and final workspace lint29.82s.
Exact provenance/source hash and static checks are in the implementation ledger.

These remain immutable internal structured-codec fixtures, not public wait entry/body or
released-v0.2 cutover evidence. General loops/nesting/capacity, SDK/fault/resource/release
acceptance and all eight canonical tasks remain open.


### Scheduler-owned event entry admission

Pinned WaitEvent entries now use the existing Scheduler admission, PENDING without worker or
subscription authority, and the same exact dispatch/journal validation in full/path/frontier
reads. Two genuine native admission REDs become full/frontier worker-free end-to-end success.
Seven focused cases pass26.64s/81.05s; expanded pause/zero assertions were corrected to existing
typed refusals, preserving actual tick failed=0. Cancellation retires the unstarted return,
admission journal faults roll back all state and current wrong dispatch metadata fails closed.

The fixed event gate passes86/0/0 (338.78s/338.79s). A later, separate cold single-connection
case passes1/0/0 (2.46s/60.18s), proving durable discovery and actual subscription ticks without
worker hints. These are separate invocations, not an87-case aggregate. Controls23/0/0
(97.74s/97.75s), contracts49/0/0 (0.58s/67.26s), workspace lint76.45s and final targeted lint
30.65s pass. The0.3 ledger retains exact executable identities and bounded-review evidence.
No public wait body/entry activation, released-v0.2 cutover or canonical task closure is claimed.


### Actual released-v0.2 active-data upgrade

The verified immutable v0.2.0 source archive now produces a real public activity collection
and root event wait. Normal current migrations preserve old data; current runtime-role worker/
scheduler/event services finish both, and the exact released reader verifies REST and native
replay afterwards. Initial subscription metadata, immutable prefixes and legacy consumer/UUID
are checked. Explicit stopped-owner expiry proves actual current Keeper takeover, not a live
old-owner race. No missing migration or checksum exception is used.

The final runner passes325.75s; clean current build313.03s; runtime command9.66s. Coordinator
1/0/0 passes8.42s, with nested old producer1/0/0 in3.86s and old reader1/0/0 in0.30s. Eight Python
source-boundary tests pass0.082s. The0.3 ledger retains setup compile errors, the invalidated
shared-target cache collision, isolated rebuild, separate functional preflight and three bounded
reviews. Copied executable/source hashes and exact outputs are retained in
[evidence/released-v02-20261027](./evidence/released-v02-20261027/README.md).

This proves released-source runtime compatibility for old READY/unleased work and root waits.
Old in-flight reports, scoped waits in old code, old-dispatcher credential fencing/cutover,
public activation, nesting/capacity and final SDK/fault/resource/release acceptance remain open.
All eight canonical tasks remain unchecked.
