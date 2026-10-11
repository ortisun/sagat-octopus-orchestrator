# Research & Decisions: CLI, quickstarts, local testing and documentation

## Main decision

A command line that is a client of the API (`sagat`, in `apps/cli`, its own parser); templates tested in CI; the real engine
served by `worker dev` from a file of canned answers instead of an emulator; a development identity provider that refuses to
run outside development; documentation whose reference pages are generated from the contracts and whose examples are
checked by the tool they show.

## Rationale

Enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

## Alternatives considered

1. **Generic ad hoc queue-based solution:** rejected as the sole durability mechanism, since message delivery is not equivalent to a transaction over workflow state.
2. **Independent services per feature from day 1:** postponed to avoid operational and consistency complexity without a benchmark.
3. **SDK-specific logic:** rejected for core invariants, to avoid behavior differences between languages.

## Internal references

- `docs/adr/` (architectural decisions)
- `docs/architecture/overview.md`
- `docs/test-strategy.md`
- `specs/001-platform-foundation/contracts/`

## Risks and mitigation

- Complexity/scope risk: deliver P1 user stories separately, running contract tests.
- Semantic ambiguity risk: encode reducers and golden fixtures, do not decide via UI alone.
- Crash/redelivery risk: test harness with real failures at checkpoints.
- Security risk: specific pentest and tenant isolation from the first milestone.

## Questions for spike validation

- Latency and overhead benchmarks on the critical path defined for this feature.
- Detail compliance and retention requirements according to the deployment environment.

## What was decided while building it (D206–D222) and what is left (G215–G232)

Numbering continues after feature 011 (D183–D205, G193–G214). The decision is
[ADR-0019](../../docs/adr/ADR-0019-the-first-day-runs-on-the-real-engine-with-canned-answers-and-a-development-identity-provider.md);
how to operate it is `docs/runbooks/developer-experience.md`; the path itself is `docs/quickstarts/README.md`; the data is in
`data-model.md`; the evidence and the reviews are in `quickstart.md`.

### Decisions

- **D206 — The tool keeps the name `sagat` and lives in `apps/cli`.** The specification's `df` and `tools/cli/` were placeholders; the
  tool is the one feature 011 introduced and the commands of 012 (`new`, `workflow`, `script`, `worker`, `dev`, `doctor`) are
  added to it. Tested: `apps/cli/tests/args.rs`, the architecture check (21 crates).
- **D207 — There is no second engine.** The "local emulator consistent with the real semantics" of FR-012-003 is the real engine
  plus `sagat worker dev`, a real worker that answers from a file. An emulator would have its own retry, compensation,
  reconciliation, timer and lease. Tested: every story of `e2e_developer_experience` runs a workflow on the engine as a process.
- **D208 — What a mock file can say.** `output`, `echo`, `fail {code, retryable, message}`, `lose`, `sequence`, `delayMs`; a compensation
  not listed succeeds, a reconciler not listed says `unknown`, an activity not listed fails for good as `NO_MOCK` (and the tool
  warns before it serves). `lose` takes the task and never answers, so the lease has to expire and a `highRisk` step with a
  `reconcileWith` is reconciled; a `lose` inside a sequence counts as a call (at first it did not: found while writing the stories, fixed with a test).
  Calls are counted per name and kind, not per run. Tested: `apps/cli/tests/mocks.rs`; `e2e stories.rs::us02_*` (a failure injected into the
  provider is retried, compensated and, when the answer is lost, reconciled).
- **D209 — The tool checks with the server's own codec and explains with a table checked in both directions.** A finding has a code,
  a place, a help line and a link to `docs/quickstarts/errors.md`; one test fails if the codec can return a code the page lacks
  (or the reverse), another if the page's sentence is not the `help:` the tool prints. Tested: `apps/cli/tests/diagnostics.rs`,
  `e2e diagnostics.rs`.
- **D210 — The tool's handling of the token and of what it prints.** The token comes from `--token-file` or `SAGAT_TOKEN`, never
  `argv`; it goes only to an `https` address or an `http` one on loopback (the URL is parsed, not matched; credentials in the URL
  are refused); no proxy for `http`; no redirect; control and bidirectional characters in anything somebody else wrote are shown
  as `?`; a parse error does not quote the file; YAML is expanded within a budget. Tested: `apps/cli/tests/client.rs`, `manifest.rs`;
  `fault_developer_experience::no_command_prints_the_token_…`.
- **D211 — `GET /v1/version` is authenticated and needs no permission.** It says the service, the build version, the API version and
  the range of worker protocol minors (`1.0`–`1.{PROTOCOL_MINOR}`), so `doctor` can say "the tool is older than the platform"
  in words. It is in the OpenAPI document, the Studio's generated types and the REST conformance suite. Tested:
  `contract_rest_platform_foundation` (`/version` scenarios), `e2e doctor.rs`.
- **D212 — Binding replaces.** `worker bind` and `worker dev --bind` put the identity of the token in the namespace's worker bindings
  for the activities of a workflow (its steps, compensations and reconcilers), **replacing** what it was bound to there, and the
  output says so. In the development stack an administrator's token can serve as a worker; in production a worker has an
  identity of its own. Tested: `e2e stories.rs::worker_bind_lets_…`, `fault … another_tenant_cannot_…`.
- **D213 — The development identity provider (closes G10 of feature 001).** A binary (`apps/dev-idp`), an image stage and a compose
  service, all of them separate from the platform. It signs only when `SAGAT_ENV=dev`, listens on loopback unless
  `SAGAT_DEV_IDP_EXPOSED=true`, answers only to the host names it is configured with, takes `application/json` of at most 4 KiB,
  refuses the platform's reserved `system:` actors, caps a token at the hour the platform accepts, sends no CORS headers, and
  refuses a key file others can read. Tested: `apps/dev-idp/tests/provider.rs` (11); `e2e dev_environment.rs` (the platform
  accepts its tokens, refuses a non-member's and another provider's).
- **D214 — `sagat dev token` prints the token and nothing else.** So `export SAGAT_TOKEN=$(sagat dev token)` works; `--out FILE`
  writes it with mode `600`. The provider's address is `--idp`, `SAGAT_DEV_IDP` or `http://127.0.0.1:18090`, and the same
  rule of addresses applies (https, or http on loopback). Tested: `e2e dev_environment.rs`.
- **D215 — `seed-dev` can leave a workspace and a namespace.** `SAGAT_SEED_NAMESPACE` (and `SAGAT_SEED_WORKSPACE`,
  `SAGAT_SEED_ENVIRONMENT`) make them as the administrator would, through the identity service, so they are audited; it is a second
  function, not a change to `SeedConfig`, so nothing that built one changes. Tested: `e2e dev_environment.rs` (idempotent,
  refused outside `dev` and before the tenant exists), and the real binary seeded twice by hand.
- **D216 — The control-plane image carries the Studio's files and serves them only when told.** `/srv/studio` is in the image;
  `SAGAT_STUDIO_DIR` is set by compose and by nothing else. The stage was replayed in a scratch directory (and found
  that the build also needs `tsconfig.base.json` and `.npmrc`).
- **D217 — An example is one program that is the worker and the client, configured as the tool is.** The names are
  `SAGAT_SERVER`, `SAGAT_GRPC`, `SAGAT_NAMESPACE`, `SAGAT_TOKEN`; the output is three fixed lines and one that proves the data
  flowed (`delivering Hello, Ada!`). The SDK is used from the checkout until it is published, and each README says which line
  changes then. The handler shape is the SPEC's: the SDK hands the run's `input` and the outputs of earlier steps in the
  context — the first Go example read the step document instead and made `Hello, !` (found by comparing with the Python
  example's report; `e2e us03` now asserts the output digests and the `delivering` line).
- **D218 — The five make the same story, checked.** One test runs them against a platform and requires: the same journal
  (types, steps, attempts and the digest of each output) across languages, equal to the digest of the exact expected
  outputs, the same shape as the journal `worker dev` makes, and the trace of the run on every event. Tested: `e2e us03` (ignored
  without five toolchains; the `sdks` job of CI runs it).
- **D219 — The saga example is one language, three days.** `examples/payment-saga/typescript` serves the six activities of
  `templates/compensation` against an in-memory provider keyed by the idempotency key: a good day, a card declined for good
  (compensated), a charge whose answer is lost (reconciled, one charge). Tested: `e2e saga.rs`.
- **D220 — Documentation is generated and checked by a test, not by a separate tool.** `docs/reference/{api,workflow-document,cli}.md`
  are what a regeneration gives (`SAGAT_DOCS_BLESS=1` rewrites); the same target parses the `sagat` commands of the pages with the
  tool's parser, requires `excerpt=` blocks to be in their files as shown, the scripts a README runs to exist and be
  executable, and links to lead somewhere. Both kinds were shown to fail when an example is changed. Tested:
  `contract_developer_experience` (7).
- **D221 — The script commands are tested against the production runner with a stand-in for the container engine.**
  The platform, the sandbox runner (the production code) and the tool are real; what runs the code is a `Sandbox` that
  echoes or overruns, so the tool's paths (create, replace, test, publish, the digest that pins a workflow, a build the
  platform rejects, a missing runner) run on any machine. One test uses the real engine and is `#[ignore]`.
- **D222 — The benchmark measures the machine's part of the first day, and says it is not the evaluation.** The commands of the
  quickstart, a token, a run served by `worker dev`, and each example's `prepare.sh` (from a clean build, package caches warm)
  and `run.sh`. Tested: `perf_developer_experience`; report in `tests/performance/reports/developer-experience.md`.

### Gaps

- **G215 — The first day has not been timed with people.** `SC-012-001` ("a new team member runs the first runs in TS and C#
  following only the documentation") and the fifteen-minute target of `NFR-012-PERF` are not evaluated: no person was asked.
  What is measured is the machine's part (D222). The documentation is checked mechanically (D220), not read by a newcomer.
- **G216 — No prebuilt `sagat`, no published SDK, no hosted documentation.** The tool is installed with
  `cargo install --path apps/cli --locked` (a Rust toolchain), the SDKs are used from the checkout, and the pages are files in the
  repository (`SAGAT_DOCS_URL` turns the `docs:` line of an error into a link when somebody hosts them).
- **G217 — The compose stack was not run on the machine where it was written.** The container engine there could not create
  containers (its metadata store is read-only). What was done instead: `docker compose config` on the file; the Studio stage
  of the Dockerfile replayed in a scratch directory; and the real binaries (`dev-idp`, `control-plane`, `execution-engine`,
  `sagat`) started as processes with the same environment as compose, against a PostgreSQL: migrate, the seed twice, a token,
  `doctor` (all nine checks ok, the Studio included), publish, the Go example, `worker dev`. **The Dockerfile's new stages, the
  `dev-idp` service and the volume ownership of `/data` are first proven by the `local-environment` job of CI.**
- **G218 — Some tests run only in CI.** The five-language examples and the saga (five toolchains), and the script test with the real
  container engine, are `#[ignore]`; the `sdks` job runs them. Locally the five languages and the saga were run (Node 26,
  Python 3.14, .NET 10, JDK 23; Go 1.24.2 with the 1.25.14 toolchain it fetches); the real-engine script test was not.
- **G219 — The SDKs ship no fake context** for unit testing an activity. The quickstarts say to keep the effect behind an interface
  and to use `worker dev` or the example as the harness.
- **G220 — The development provider has no browser flow.** The Studio takes a pasted token. A token lasts at most an hour, and a
  worker's session ends with its token, so a worker left running overnight stops being accepted.
- **G221 — A mock file cannot answer by what the input was, or keep state beyond a sequence.** A provider with memory is a real
  worker; the saga example is one.
- **G222 — `worker dev` serves activities, compensations and reconciliations only.** It does not send events or signals, decide
  an approval or serve a node the engine does not run: `humanApproval` and `foreach` are refused by the codec's findings
  (`UNSUPPORTED_NODE`, the Studio's G147), so the approval template waits for an event.
- **G223 — The documentation checks parse, they do not run.** A `sagat` line must be a command the parser accepts, not one that
  succeeds against a platform; an anchor in a link is not followed; a reference-style link or an image is not checked; shell
  that is not `sagat` (`docker compose …`) is not checked at all (the smoke script runs it).
- **G224 — The API reference is a map, not a copy.** It lists operations, their feature and whether they take an idempotency key;
  the parameters, bodies and schemas are the OpenAPI file's, which is not rendered anywhere.
- **G225 — The Java and C# examples download their dependencies on first use** and the Java one installs the SDK into
  `~/.m2`; on a machine without network the first run fails there. The C# SDK also restores the net8.0 packages.
- **G226 — The saga example exists in TypeScript only.** The other four languages have the hello-world.
- **G227 — The benchmark does not include building the images** (no container engine on the machine), the network between
  tool and platform, or any person.
- **G228 — `worker dev --bind` replaces bindings of an administrator's identity** in the namespace (D212); a developer who
  also runs a real worker with that token loses its bindings. The output says so before it happens, not before it is chosen.
- **G229 — The development stack trusts plain `http` between its containers** and `SAGAT_ALLOW_INSECURE_JWKS_HTTP=true` in the
  example environment file; copied into a shared environment without reading it, the API would fetch its keys over `http`
  from a provider that signs for anyone. The provider's own refusals (D213) and the runbook stand between that and a
  server that signs for anyone; nothing stops an operator who sets all of it on purpose.
- **G230 — The seed makes the namespace in the environment `dev`.** Another environment is `SAGAT_SEED_ENVIRONMENT`; a policy of
  approvals for `prod` (feature 011) applies to a seeded production namespace like any other.
- **G231 — The script tests with the stand-in sandbox prove the tool and the platform's wiring, not the sandbox.** The sandbox's
  own behaviour is feature 005's.
- **G232 — Reviews.** See `quickstart.md`.
