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
