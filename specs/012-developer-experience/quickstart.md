# Validation quickstart: CLI, quickstarts, local testing and documentation

Everything below was run against this repository. The pieces that are **not** built, and what could not be run where it
was written, are listed at the end. The path a developer follows is `docs/quickstarts/README.md`; this page is how the
feature is checked.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and a PostgreSQL the suites can create databases in: Docker
  (Testcontainers starts PostgreSQL 16) or, with `SAGAT_TEST_DATABASE_URL=postgres://…@host:port/postgres`, a server
  you already have.
- For the ignored suites: Node 20+, Python 3.10+, Go 1.25+ (a 1.24 `go` fetches the toolchain `go.mod` asks for), .NET 10 and a JDK 17+; Docker for the
  one test that runs a script in the real sandbox.
- To use it by hand: `docs/quickstarts/README.md`.

## Validation flow

1. **The stories, the faults, the documents, against real PostgreSQL, a fake OIDC provider and real sockets** (the engine
   as a process, the API and the command line in the test process):

   ```sh
   cargo test -p sagat-tests --test e2e_developer_experience         # US01..US04, doctor, diagnostics, the development provider and the seed, the script commands (30, 3 ignored)
   cargo test -p sagat-tests --test fault_developer_experience       # restarts, signals, caps, tenants, the token, the environment guard (14)
   cargo test -p sagat-tests --test contract_developer_experience    # the generated reference and the examples in the documents (7)
   cargo test -p sagat-cli -p sagat-dev-idp -p sagat-control-plane  # the parser, the mocks, the templates, the errors, the provider, the seed (50, 15, 47)
   cargo test -p sagat-tests --test contract_rest_platform_foundation # GET /v1/version against the OpenAPI document
   ```

2. **The examples in five languages, the saga and the real sandbox** (`#[ignore]` because they need the toolchains and Docker):

   ```sh
   cargo test -p sagat-tests --test e2e_developer_experience -- --ignored
   SAGAT_EXAMPLE_LANGS=go,python cargo test -p sagat-tests --test e2e_developer_experience us03 -- --ignored   # some languages
   ```

3. **The Studio's generated types** follow the OpenAPI document: `npm run check:api` in `frontend/` (the contract gained
   `getPlatformVersion`).

4. **The benchmark** (the machine's part of the first day):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/developer-experience.md \
     cargo test -p sagat-tests --test perf_developer_experience --release -- --ignored --nocapture
   ```

5. **The compose stack**: `SAGAT_BIN=target/debug/sagat scripts/smoke-compose.sh` (Docker; the `local-environment` job of CI).

## Story evidence

| Story | Acceptance scenario | Evidence |
|---|---|---|
| US01 Start the first workflow quickly | A developer follows the quickstart (compose + CLI + SDK) and a two-step workflow runs and can be seen | `e2e stories::us01_following_the_quickstart_a_two_step_workflow_runs_and_can_be_seen` (`doctor`, `new`, `validate`, `publish` twice (the second changes nothing), `worker dev`, `run --wait`; then the run, its graph and its journal through REST, which is what the Studio draws); `us01_a_run_that_waits_for_a_worker_says_so_…` (no hang, and it can be cancelled); the whole sequence on real binaries wired like compose (G217); the SDK half: `e2e us03` |
| US02 Test a saga locally | A failure injected into the mock provider shows reconciliation and compensation | `e2e stories::us02_a_payment_saga_is_rehearsed_against_a_mock_provider_that_fails_on_demand` (decline → compensated; timeout once → retried), `us02_a_lost_answer_is_reconciled_before_anything_is_repeated` (a worker that dies with the task: the lease runs out, the reconciler is asked); with an SDK worker and a provider with memory: `e2e saga::us02_the_payment_saga_of_the_example_…` (a good day, a decline compensated, a lost answer reconciled with one charge, not two) |
| US03 Generate examples per SDK | A new developer uses Go or C#, copies the example and runs it: the contract and the trace match the other SDKs | `e2e us03::us03_the_hello_world_of_each_sdk_makes_the_same_run_as_the_others` — all five languages against one platform: the same three lines and the `delivering` line, a fresh token each, exit 2 without a token, the journal identical (types, steps, attempts, the digest of every output) and equal to the digest of the exact expected outputs, the same shape as the journal `worker dev` makes, a trace on every event; plus the templates: `stories::us03_*` (event trigger, approval, cron, inline script) |
| US04 Diagnose publishing errors | An invalid node: the CLI says where, what to do and where it is explained | `e2e diagnostics.rs` (each code from the codec with its place, help and docs line; what only the platform can find: `UNKNOWN_SCRIPT`, a secret you may not use), `apps/cli/tests/diagnostics.rs` (the table and `docs/quickstarts/errors.md` hold each other to account in both directions) |

## The functional requirements

| Requirement | Where it is proven |
|---|---|
| FR-012-001 the CLI: `workflow validate\|publish\|run\|inspect\|cancel`, `script test\|publish`, `worker dev` | `e2e stories.rs`, `e2e scripts.rs` (against the production sandbox runner with a stand-in for the container engine; the real engine in one test CI runs), `fault` suite, `apps/cli/tests` |
| FR-012-002 quickstarts for five SDKs and a saga business example | `docs/quickstarts/*.md`, `examples/`, `e2e us03`, `e2e saga` |
| FR-012-003 compose with services and fixtures, and a local emulator consistent with the real semantics | the compose file, the seed (`e2e dev_environment`), the provider (`apps/dev-idp/tests`, `e2e dev_environment`); **the emulator is the real engine** (D207), served by `worker dev` |
| FR-012-004 templates: workflow, event trigger, cron, approval, compensation, inline script | `templates/`, `apps/cli/tests/templates.rs` (each is a document the codec accepts, with answers and a README), `e2e stories.rs` (each runs) |
| FR-012-005 API docs and contracts generated from the schemas; examples executable in CI | `docs/reference/`, `contract_developer_experience` (7); both kinds of check were shown to fail when an example or a command was changed |
| FR-012-006 onboarding instrumentation and diagnostics for environment, credentials and incompatible versions | `sagat doctor` (`e2e doctor.rs`), `GET /v1/version`; **the onboarding is instrumented by the benchmark and the checks, not by telemetry sent from the tool** (it sends none) |

## Edge cases of the specification

| Edge case | Where it is handled |
|---|---|
| A client without Docker | The tool and the SDK examples need no Docker; the local stack does. `sagat doctor` says what is missing; the dev provider and the platform can be run as processes (G217 describes how that was done here) |
| Mocks that diverge from the real behaviour | There is no second implementation: the engine is the real one and only the answer is canned (D207). The file says what it does not answer (a warning per section before it serves; `NO_MOCK` for a step it has no answer for) |
| A version of the tool older than the platform | `GET /v1/version` and `doctor` (`versions`): the build, the API and the range of worker protocol minors, in words |
| A worker that dies, a duplicate, a timeout, another tenant | `fault_developer_experience` (14): stop and start, a signal at every point of the loop, the cap of tasks, a run waited for in vain, the same request twice, a run that waits for a person, another tenant, an unreachable platform |

## Reviews of the implementation

Two independent, read-only reviews of the diff against `develop`, by agents other than the one that wrote it: one for
security, one for correctness and test quality. The authors of both were told to confirm what they claimed, by reading or by
running, and to say which areas were sound.

- **Security review:** no critical or high finding. **Confirmed and fixed (with a test that fails without the fix):** the token went in the
  clear to a non-loopback worker endpoint, so the ADR's claim was false for the worker path (D224); a workflow naming `*` bound the
  administrator's identity to every activity (D225); the worker commands could give canned answers to the real tasks of a
  staging namespace (D223); `dev token --out` followed a link (written through it); the terminal filter missed the invisible
  "tag" characters and several server strings kept their newlines (D210); two reads were not capped; the key file of the
  provider was checked by name and then read by name, followed links and did not look at its directory; the provider trusted
  the first `Host` header only and not an absolute-form request line; `.dockerignore` let a host's `node_modules` into the
  Studio stage; the Python example installed without hashes; the smoke test put the token in the arguments of `curl`.
  Left, and written down: G229 (corrected: the insecure-JWKS setting is refused outside `dev`; the hazard is the compose
  default of `SAGAT_ENV`), G234, G235.
- **Correctness review:** 21 items. **Confirmed and fixed:** `worker dev` lost almost every Ctrl-C (confirmed by a scratch program: a fresh
  listener at each await; D227), asked for eight tasks whatever `--max-tasks` said (the test served 3 of 1 before), and its
  `doctor` judged a token by age where the platform judges it by span; the tests and commands read the shell instead of the
  injected environment (two tests failed on a developer's machine after the quickstart's exports; D226); an example token
  could expire during a slow clean build on CI; `workflow run --wait` waited out its timeout for a run only a person can move
  (D228); an answer in the wrong section of the mock file was ignored in silence; the doc checks did not look at the runbook, the
  ADR or commands inside `$(…)` and accepted an empty excerpt; the generated pages said every call takes a token and linked a
  heading they did not have; the Go example's `run.sh` ended 1 where the others end 2; the examples showed a `WAIT_TIMEOUT` as a network
  error; the seed created the workspace before it validated the environment and called a lost race a failure; the provider ignored
  SIGTERM, wrote its key non-atomically (a test of eight starts at once found that a shared temporary name collided, which
  is real when every container is process 1) and normalised its issuer's name wrongly; the `event-trigger` README said the data
  is the run's input (it is under `data`; asserted now); and a handful of documents that still said there is no development
  provider (G10) or exit code 6 only for `workflow run`. **Left, and written down:** G233 (paths no test reaches), G235 (small edges),
  G236, and the items the review itself marked accepted.
- **My own checks, beyond the reviews:** the Dockerfile's Studio stage replayed in a scratch directory (it found that the build also
  needs `tsconfig.base.json` and `.npmrc`); `docker compose config`; the real binaries (`dev-idp`, `control-plane`,
  `execution-engine`, `sagat`) run as processes with the compose environment against PostgreSQL: migrate, the seed twice (the
  namespace once), a token, `doctor` (all nine checks), publish, the Go example, `worker dev`, and SIGINT/SIGTERM to a worker
  that was idle (exit in 0.05 s); a mutation of the batch size and of two documented examples, which the tests caught.

## Gate results

Run on the machine where this was written (macOS arm64, PostgreSQL 18.6 for the suites; CI runs 16):

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `python3 scripts/check-architecture.py` and its 8 rule tests | passed (21 crates, layers respected, no cycles) |
| `cargo test --workspace --locked --no-fail-fast` | see below |
| The Studio: `check:api`, `lint`, `typecheck`, `npm test`, `build` | passed (863 tests; three autosave tests of feature 007 failed once while the workspace was being tested at the same time and passed alone: G237) |
| The five-language examples and the saga | passed locally (Node 26, Python 3.14, Go 1.24.2→1.25.14, .NET 10, JDK 23) |

**What could not be run here:** the container engine on this machine could not create containers (its metadata store is
read-only), so 63 tests of earlier features that need a container engine (NATS, Kafka, ElasticMQ, the script sandbox) fail
here for that reason and pass in CI; the new Dockerfile stages, the `dev-idp` service and the smoke test's new steps are first
proven by CI (G217); the script test with the real sandbox is `#[ignore]` and was not run here (G218).

## What is not built

Everything in `research.md` under "Gaps" (G215–G237). The ones that matter most for a decision: **no person was timed** (G215), **no prebuilt
`sagat`, no published SDK, no hosted documentation** (G216), **the compose stack has not been run on a machine of the author's**
(G217), and **the development provider has no browser flow** (G220).
