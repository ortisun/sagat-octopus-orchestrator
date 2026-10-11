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

## What was decided while building it (D206–D228) and what is left (G215–G238)

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
  are refused); no proxy for `http`; no redirect; what an answer may weigh is capped (64 MiB for the API, 1 MiB for a probe or the
  development provider); control characters, the bidirectional and the invisible format characters (the "tag" block that can carry
  text nobody sees, the variation selectors, the soft hyphen) in anything somebody else wrote are shown as `?`, and a value from
  the server is put on one line; a parse error does not quote the file; YAML is expanded within a budget. Tested:
  `apps/cli/tests/client.rs`, `manifest.rs`, `terminal.rs`; `fault_developer_experience::no_command_prints_the_token_…`;
  `e2e dev_environment::a_provider_that_never_stops_answering_…`.
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

- **D223 — The worker commands give canned answers only to a `dev` namespace.** `worker dev` and `worker bind` look up the
  environment of the namespace and refuse `staging`, `prod` or an environment they cannot read, unless `--allow-non-dev` says that
  is meant (and then say it); the banner names the environment. Found by the security review: a shell still holding the
  address and the administrator's token of a staging session would have answered its real tasks with canned answers.
  Tested: `fault canned_answers_are_given_to_a_dev_namespace_and_to_no_other_unless_asked`.
- **D224 — The worker endpoint is held to the rule of the REST address.** `https`, or `http` on a loopback address: the token is
  also in the headers of every gRPC call. Found by both reviews (the ADR said it already; the code did not do it). Tested:
  `fault the_token_is_not_sent_to_a_worker_endpoint_in_the_clear`.
- **D225 — A binding is made of names, never of patterns.** The platform accepts `*` and `payments.*` as worker bindings; a workflow
  from somebody else's repository can name an activity `*`, and the administrator's identity would have been bound to
  everything. The names taken from a document are held to the rule of an activity name. Tested:
  `fault a_workflow_from_somewhere_else_cannot_make_the_tool_bind_a_pattern`.
- **D226 — The library reads the process only through `Env`.** The worker address, the development provider and the documentation
  URL are fields of `Env` filled by `from_process`, so a test (or a program that embeds the tool) is not changed by what the
  shell around it exported, and `run_until` lets the caller supply what stops a long command. Found when the quickstart's
  exports made two tests fail on a developer's machine.
- **D227 — `worker dev` listens for its stop signals from its first moment and takes no more than it was told.** One task waits for
  Ctrl-C or SIGTERM and tells the loop, which honours it while polling, waiting and in the middle of an answer; before, a signal
  that arrived while no one was waiting for it (almost always) was lost. Polling asks for at most what `--max-tasks` leaves.
  Verified by hand against the real binary (SIGINT mid-sleep: exit in 0.05 s) and tested in the loop: `fault
  a_worker_told_to_stop_stops_whatever_it_is_doing`, `the_cap_of_tasks_is_the_cap_…` (it served 3 for `--max-tasks 1` with the old
  batch size).
- **D228 — `workflow run --wait` stops waiting for a run that only a person can move.** `MANUAL_REVIEW` and `PAUSED` are reported
  at once (exit 6, "it was not cancelled"), not after the timeout with a message about workers. Tested: `fault
  a_run_that_waits_for_a_person_is_reported_at_once_…`.

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
- **G229 — The development stack trusts plain `http` between its containers.** `SAGAT_ALLOW_INSECURE_JWKS_HTTP=true` is in the
  example environment file, but the control plane and the engine refuse it unless `SAGAT_ENV=dev`, and the provider exits
  unless `SAGAT_ENV=dev`. The hazard is the compose default `SAGAT_ENV: ${SAGAT_ENV:-dev}`: a deployment copied from this file
  that never sets the variable is a development one, with a provider that signs for anyone. The runbook and the file say so;
  nothing stops an operator who does it on purpose.
- **G230 — The seed makes the namespace in the environment `dev`.** Another environment is `SAGAT_SEED_ENVIRONMENT`; a policy of
  approvals for `prod` (feature 011) applies to a seeded production namespace like any other.
- **G231 — The script tests with the stand-in sandbox prove the tool and the platform's wiring, not the sandbox.** The sandbox's
  own behaviour is feature 005's.
- **G232 — Reviews.** See `quickstart.md`.
- **G233 — Paths of `worker dev` no test reaches:** the re-registration after `Unauthenticated`, the "attempt was no longer current" line for a
  stale report, and the clip of a payload over 400 characters shown by `--show-payloads`. The examples end differently on an interrupt (130 or 1).
- **G234 — Two installs are not hash-locked.** The Python example installs its run-time dependencies with the SDK's hashes but its
  build backend (`setuptools`) comes from PyPI unpinned; the C# restore pins versions centrally but has no lock file; Maven uses the
  wrapper's checksums for what it downloads.
- **G235 — Small edges that were left.** `script test|publish` finds its script by name among the first 200 of the namespace; a `--key`
  with a space is reported as a transport error; `diff` and `publish` read the namespace from the manifest before `SAGAT_NAMESPACE`
  (feature 011); the provider's health check probes the IPv4 loopback only; a mock whose answer lies in the wrong section is warned
  about, not refused.
- **G236 — `dev-idp` is not behind a compose profile.** With `SAGAT_ENV` other than `dev` it exits and restarts for ever (it fails
  closed) while `docker compose up -d` succeeds.
- **G237 — The Studio's autosave unit tests are timing-sensitive.** Three of them failed once while the whole workspace was being
  tested at the same time and passed on a quiet machine (feature 007's, unchanged here).
- **G238 — Closed for opt-in command instrumentation, not a newcomer study.** The CLI now propagates a W3C trace context through REST and gRPC even when exporting is disabled. `OTEL_EXPORTER_OTLP_ENDPOINT` opts into OTLP/HTTP JSON command spans, completion logs and duration gauge points; a trace-specific endpoint exports spans only. `SAGAT_ONBOARDING_TELEMETRY_FILE` optionally records local completion logs and duration points. Command, stage and outcome are closed labels; token, payload, file path, endpoint, namespace ID and tenant ID are excluded. Tenant context is only a bounded classification, not verified identity. `apps/cli/tests/telemetry.rs` captures the wire signals and checks redaction, opt-in, correlation and failure flushing. This does not identify people or measure their reading time; G215 stays open.
