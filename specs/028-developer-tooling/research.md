# Feature 028 — Baseline, sources, decisions, gaps and owner questions

Baseline investigation: 2026-10-26, worktree `sagat-octopus-parity-specs` at `70f09435` (release 0.2.0). Every
path below was opened or listed during the investigation. Nothing in this feature is implemented. Numbers allocated
to this feature: D1400–D1439, G1400–G1439, O1400–O1439, ADR-0063.

## Repository evidence

### Time and the clock

- `crates/application/src/clock.rs` defines the `Clock` port and `SystemClock` (UTC, truncated to the microsecond by
  `truncate_to_micros`). It is injected into a few services (identity, quotas, egress, bindings, reveal grants:
  `apps/control-plane/src/lib.rs`, `apps/execution-engine/src/lib.rs`) and replaced by `FixedClock` in
  `crates/application/tests/*`.
- The decisions that matter for a workflow do **not** go through that port. The run transaction samples the database
  clock after taking the run lock (`crates/infra-postgres/src/execution.rs`, "Sample mutation time only after the row
  lock is held": `SELECT clock_timestamp()`), and the domain receives it as `tx.now()`
  (`crates/application/src/execution.rs`). Due timers are selected with `due_at <= now()`
  (`crates/infra-postgres/src/maintenance.rs`, `due_timers`) and cron ticks with `next_fire_at <= clock_timestamp()`
  (`crates/infra-postgres/src/dispatch.rs`). `crates/infra-postgres/src` has 172 lines that call `clock_timestamp()`,
  `statement_timestamp()` or `transaction_timestamp()`, and 46 lines containing `now()` (about 40 of them SQL, the rest
  Rust method calls) — line counts by `grep`, not an audit; the migrations have 159 lines declaring `timestamptz`
  across about 50 distinct column names.
- ADR-0020 §4: candidate selection uses `now()` (stable) and not `clock_timestamp()` (volatile) so that the planner
  can use it as an index-range bound; a test refuses `clock_timestamp()` in the pass files. Any replacement must keep
  a stable function for selection.
- The same database clock also decides infrastructure liveness: worker session expiry
  (`execution.rs`, "expires_at <= now" after the session lock), shard leases (`scheduler_shards`, ADR-0020),
  lease expiry and cleanup leases (`secret_cleanup.rs`).
- The conformance suite moves time by back-dating rows: `tests/contract/sdk-conformance/control.rs` offers
  `elapseTimer`, `elapseRetryDelay`, `expireLease`, implemented in `tests/support/worker.rs` with
  `UPDATE steps SET due_at = clock_timestamp() - interval '1 second'`. The journal keeps the original `due_at`; the
  facility is not available to SDK users.

### Engine composition, replay and test-only code

- `apps/control-plane/src/lib.rs` (`serve`, `serve_with_admin`, `admin_router_with`) and
  `apps/execution-engine/src/lib.rs` (`serve`) are libraries; `tests/support` composes them in-process. A test
  server can compose them the same way.
- `crates/scheduler/src/passes.rs` exposes `Scheduler::tick` (one pass) and `housekeeping`.
- `crates/domain/src/execution/state.rs` has `RunState::replay` and `replay_addressed`;
  `crates/infra-postgres/src/execution.rs::verify_run` decodes `run_events` with `event_codec::decode` and compares the
  replay with the stored projection. This is the replay check an exported history needs.
- `crates/infra-connectors/Cargo.toml` has a test-only feature `unguarded-for-tests`, enabled by `tests/Cargo.toml`.
  `infra/docker/Dockerfile` builds every production binary in **one** `cargo build` invocation, so a cargo feature
  enabled by another package of that invocation would be unified into them.
- `SAGAT_ENV` is `dev | staging | prod`, default `prod` (`apps/control-plane/src/cli.rs`,
  `apps/execution-engine/src/cli.rs`); development relaxations refuse other values.
- `scripts/check-architecture.py` assigns layers by directory (`crates/infra-*` adapters, `apps/*` apps) and has no
  rule about which app may depend on which.

### SDKs

- `sdks/SPEC.md`: an SDK is a client, a worker host and builders, "not a workflow runtime"; §4 trace hook
  (`traceContext()`); §5 retries by code with keys kept across retries; §6.3–6.6 how a task runs and how outcomes map
  (`ActivityFailure`, `ACTIVITY_ERROR`, `INPUT_INVALID`, `PAYLOAD_TOO_LARGE`, reconciliation verdicts); §10 hooks must
  not kill the host. No interceptor or middleware exists.
- `sdks/typescript/src/config.ts` has a `fetch` transport hook ("the hook of the conformance suite and of tests") and
  `traceContext`; `sdks/typescript/src/opentelemetry.ts` bridges `onEvent` and trace context to OpenTelemetry.
- `sdks/error-catalog.json` (version 1) lists the canonical codes; SDK-origin codes are `TRANSPORT`, `CANCELLED`,
  `PROTOCOL`, `PAYLOAD_TOO_LARGE`. Each SDK embeds a copy proven equal by a test.
- Feature 012 G219: "The SDKs ship no fake context for unit testing an activity."
- `tests/contract/sdk-conformance/README.md` and ADR-0015: one platform per scenario and language, reviewed goldens,
  faults injected at the transport hook (`failBefore`, `dropResponse`), runners under a non-UTC time zone.

### CLI, templates and GitOps

- `apps/cli/src/args.rs`: `workflow validate|publish|run|runs|inspect|cancel|rerun`, `worker dev|bind`, `new`,
  `export`, `diff`, `publish`, `promote`, `rollback`; only `doctor` has `--json`. Feature 011 G201 records the absence
  of machine-readable output.
- `apps/cli/src/new.rs` embeds `templates/{hello-world,event-trigger,cron,approval,compensation,inline-script}` with
  `include_str!`; `apps/cli/tests/templates.rs` proves each validates.
- `apps/cli/src/mocks.rs` and feature 012 D208 define the `worker dev` answer file (`output`, `echo`, `fail`, `lose`,
  `sequence`, `delayMs`).
- `specs/011-gitops-versioning/contracts/manifest.schema.json`: `apiVersion`, `kind: Manifest`, `namespace`,
  `workflows[{name, file, releaseNotes?}]`, at most 500 entries, relative paths below the manifest. Feature 011
  G199 (workflows only), G210 (the platform does not know the commit author). `apps/cli/src/sync.rs` and `export.rs`
  implement the pipeline direction.
- ADR-0018: publish, promote, rollback and activate move the pointer in one transaction with
  `workflow_activations` (append-only); a draft has a revision (`UpdateDraftRequest.draftRevision`, 409 when stale);
  an approval is of a document digest.
- `crates/application/src/script_signing.rs` signs script artifacts with Ed25519 (`ring`).
- `docs/runbooks/release.md`: release files carry Sigstore build-provenance attestations through GitHub artifact
  attestations; `docs/owner-decisions.md` row 12: GitHub Releases only, no package registries for now.

### Studio and frontend

- `frontend/package.json` workspaces are `apps/*` and `e2e`; there is no shared package. The definition graph is
  computed in `frontend/apps/studio/src/features/workflows/canvasModel.ts` and laid out by
  `frontend/apps/studio/src/model/layout.ts`; `DiffView.tsx` and `ConflictDialog.tsx` show server diffs and draft CAS
  conflicts. ADR-0016: the server validates and compares; same origin; token in memory.

### Identity, secrets and egress

- `crates/domain/src/identity.rs`: permissions are a closed enum; roles `admin`, `developer`, `operator`, `viewer`,
  `worker`, `runner`, `database-runner`; reserved actors `system:provisioning|operator|engine|gateway`.
- `crates/infra-secrets/src/openbao.rs` (OpenBao KV v2, ADR-0035) and FR-017-005: new secret values live in OpenBao.
- `crates/infra-egress/src/lib.rs`: the guarded client used by webhooks, connectors and scripts.

## External semantics consulted (2026-10-26)

- Temporal TypeScript testing suite, <https://docs.temporal.io/develop/typescript/testing-suite>: the test server
  skips time while a result is awaited, returns to normal time while an activity runs, `sleep()` advances
  explicitly, and "time is a global property of an instance" of the environment; replay re-runs a history against
  current workflow code to detect non-determinism. **Taken:** automatic skipping while awaiting a result, explicit
  advance, one clock per environment, no skipping while work is leased. **Not applicable:** non-determinism of user
  code — the platform's workflow is immutable IR, so the replay risk lies in the engine and its codec (D1414).
- Temporal interceptors, <https://docs.temporal.io/develop/typescript/interceptors>: client, workflow
  inbound/outbound and activity inbound interceptors; each wraps the inner call and may change input and result.
  **Taken:** onion composition, client and activity interceptors. **Not applicable:** workflow interceptors (no
  workflow code runs in an SDK).
- Windmill Git sync, <https://www.windmill.dev/docs/advanced/git_sync>: a commit per deployment, pull/merge requests
  when Windmill holds a GitHub App or GitLab project token, `[WM]`-prefixed commits skipped to avoid loops, and an
  enterprise "automatic sync" that **deploys** commits of the tracked branch. **Diverged:** imports create drafts and
  never publish (D1423, O1401); loops are prevented by comparing digests, not commit messages (D1426).
- Windmill VS Code extension, <https://www.windmill.dev/docs/cli_local_dev/vscode-extension>: preview beside the
  file, live graph from the YAML, configuration shared with the CLI. **Taken:** preview and CLI-shared configuration.
- Windmill Hub, <https://www.windmill.dev/docs/misc/share_on_hub>: anyone may submit, the Windmill team approves,
  approved items appear in every workspace. **Diverged:** no submission and no automatic presence; operator-curated,
  signed indexes and explicit import as a draft (D1433, constitution Delivery Constraints).
- GitHub: installation tokens expire after one hour and can be restricted to repositories and permissions
  (<https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app>);
  the App JWT is RS256 with `exp` at most 10 minutes ahead
  (<https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app>);
  deliveries carry `X-Hub-Signature-256` (HMAC-SHA256, constant-time comparison)
  (<https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries>) and `X-GitHub-Delivery`, with
  payloads up to 25 MB (<https://docs.github.com/en/webhooks/webhook-events-and-payloads>).
- GitLab webhooks, <https://docs.gitlab.com/user/project/integrations/webhooks/>: the signing token (GitLab 19.0,
  generally available in 19.1) follows Standard Webhooks — `webhook-id`, `webhook-timestamp`,
  `webhook-signature: v1,<base64>` over `{id}.{timestamp}.{body}`; the receiver chooses the timestamp window; the
  older `X-Gitlab-Token` is a plain-text shared value, "not recommended for new webhooks".
- VS Code webviews, <https://code.visualstudio.com/api/extension-guides/webview>: minimum capabilities, scripts off
  by default, `localResourceRoots`, a content security policy starting from `default-src 'none'`, messages as the
  only bridge. Workspace Trust, <https://code.visualstudio.com/api/extension-guides/workspace-trust>:
  `untrustedWorkspaces: limited`, `restrictedConfigurations`, `workspace.isTrusted`, and "a command can still be
  called even if it is not presented in the UI". Secret storage: the `SecretStorage` API of
  <https://code.visualstudio.com/api/references/vscode-api>; its platform backing is not relied on beyond "not a
  setting and not a file of the workspace".
- The Update Framework, <https://theupdateframework.github.io/specification/latest/>: rollback is prevented by
  refusing a lower metadata version, freeze by expiry. RFC 8785 (<https://www.rfc-editor.org/rfc/rfc8785>) is
  already the canonical JSON of the SDK builders (`sdks/SPEC.md` §7).

## Decisions

### Test environment

- **D1400 — The test server is a composition root, not an engine.** `apps/test-server` composes the control plane,
  engine loops, REST API and worker service from the production crates (as `tests/support` does) and adds the
  testing routes, the virtual clock and an embedded issuer. Rejected: an in-memory engine (ADR-0019's first
  rejected option).
- **D1401 — Only workflow time is virtual.** Leases, sessions, shard leases, rate buckets, deduplication windows,
  tokens, cleanup, telemetry, synchronizer jobs and audit stay on real time. Moving them too would expire every
  shard and session at each advance (a fail-over per stop) and would make audit times fictitious. Rejected:
  shifting the whole database clock (libfaketime or a global offset).
- **D1402 — One SQL function derives workflow time from an infrastructure sample.** `sagat_workflow_from(ts
  timestamptz)` (`STABLE`, production body `SELECT $1`) turns one infrastructure instant into the workflow instant.
  Selection bounds index ranges with `sagat_workflow_from(now())`; the post-lock sample is one materialized
  `clock_timestamp()` from which both instants are taken; lag metrics use `sagat_workflow_from(clock_timestamp())`.
  Two volatile calls are never used for one decision, because PostgreSQL does not merge them and they would differ
  even in production (architecture review, finding 1). PostgreSQL inlines the production body, so production plans
  and values are unchanged (to be proven by plan-equality tests and an equality test at offset zero). The test
  deployment replaces the body to add the offset, keeping the declared volatility. Production composition roots
  refuse a database whose body is not the canonical one; the function is owned by the migration role, which the
  runtime role cannot replace, and the engine monitor re-checks the body every round.
- **D1403 — The split is typed in Rust.** The run transaction exposes an infrastructure instant and the workflow
  instant derived from it in the same statement (identical in production); the domain receives distinct newtypes so that comparing a
  lease expiry with a timer due time does not compile. The application `Clock` port keeps infrastructure time; a
  `WorkflowClock` port is added for the few application decisions in workflow time.
- **D1404 — A manifest classifies every time column.** `crates/infra-postgres/time-classification.json` gives each
  time column a class (`workflow_due`, `workflow_record`, `infrastructure`) and its readers and writers; a catalog
  test fails on an unclassified column, mirroring `storage-classification.json` (ADR-0027). The test also covers
  column defaults (37 at the baseline default to `now()` or `clock_timestamp()`) and the bodies of SQL functions in
  `pg_proc` (43 function definitions in the migrations): a workflow-class column may not default to a built-in clock,
  and a function body that compares a workflow-class column with a built-in clock fails unless listed with a reason.
  The rule for "now" is the class of the instant measured: due checks, lag metrics and retention ages over a
  workflow-class column use workflow now (the scheduler-lag query of `operations.rs`, the telemetry purge of
  `telemetry.rs` on `runs.finished_at`); infrastructure columns keep infrastructure now (lease lag, script test-run
  retention in `maintenance.rs`). The same manifest is published as the cross-feature catalog
  ([contracts/time-classes.md](contracts/time-classes.md), coordinator decision C13). The skip query for the next due
  instant is generated from the `workflow_due` entries and a test proves it covers them all.
- **D1405 — One clock per test deployment, one tenant per test deployment.** A deployment-wide offset keeps every
  existing query unchanged in the test build. Because the clock affects every run of the deployment, the deployment
  holds one tenant so that no tenant can move another's time. Parallel tests that need independent time use separate
  test servers (Temporal makes the same choice). Rejected: per-namespace offsets (a namespace argument on every
  sample, cross-namespace deadlines on two clocks; G1400).
- **D1406 — `step` and `jump` are two different statements about time.** `step` says "the platform was up the whole
  time" (each due instant is reached and processed in order: 30 daily cron ticks fire); `jump` says "the platform
  was down" (one late processing under the existing catch-up rules). Only `jump` may waive quiescence.
- **D1407 — The test server runs the production passes on demand.** After each clock change it calls the same
  `Scheduler::tick` and dispatcher passes the engine loops call, then polls quiescence; its loop intervals are short.
  No pass is reimplemented.
- **D1408 — Test-only code is in crates no production app depends on.** The clock store, the test migration and the
  replay adapter live in a new adapter crate `crates/infra-testing`; the routes and composition in `apps/test-server`.
  No cargo feature of a shared crate is used (the Dockerfile's single invocation would unify it). The architecture
  check gains a rule: only `apps/test-server` and `tests` may depend on them; `scripts/check-production-image.py`
  fails when the Dockerfile's `cargo build -p` list or a `COPY --from=builder` line names the test server, or when
  `cargo tree -p sagat-control-plane -p sagat-execution-engine` contains `sagat-infra-testing`. The test server is a
  new process justified under principle V (isolation of test-only code from production composition), with feature
  017's `apps/database-runner` as precedent (coordinator decision C21).
- **D1409 — The testing API is its own contract.** `contracts/testing.openapi.yaml` becomes
  `specs/001-platform-foundation/contracts/testing.openapi.yaml`, so the production REST conformance (which attacks
  every operation of `openapi.yaml`) is not asked to serve it. `PlatformVersion` gains an additive `capabilities`
  list; `virtual-clock` and `history-replay` appear only on the test server.
- **D1410 — SDKs connect; the CLI starts.** The five test environments connect to a running test server (URL and token
  files). `sagat test-server start|stop|status` launches the released binary against a scratch PostgreSQL database
  it creates and drops; a compose profile does the same in CI. Managing processes in five languages is deferred
  (G1401).
- **D1411 — The SDK clock is injectable and the virtual clock is local arithmetic.** The test environment's clock is
  the system clock plus the offset returned by the last clock read, advance or skip, refreshed by each of them; it is
  used only to turn duration options into instants (FR-028-010).
- **D1412 — The activity test context reuses the host's mapping.** Each SDK builds the test context from the same
  functions its host uses to map results and failures, and a shared fixture
  (`tests/contract/sdk-conformance/fixtures/activity-context.json`) lists handler behaviours and expected outcomes,
  copied into each SDK and proven equal like the error catalog. It closes G219 of feature 012 for unit tests only.
- **D1413 — Test workers are bound by name.** The test environment binds the activity names of a worker through the
  public binding API with the admin token of the test deployment, never a `*` pattern (feature 012 D225).

### History replay

- **D1414 — Replay checks the engine and its codec, not user code.** A history is decoded with the server's event
  codec and folded by the domain reducer, and the derived projection is compared with the recorded one, as
  `verify_run` does for stored runs. It answers "does this build read and fold these runs as the build that produced
  them did", which is what an upgrade risks. It runs in the test server only: it needs no production data access and
  accepts caller-supplied documents.
- **D1415 — Derived answers reuse `worker dev`.** `sagat history answers` writes a D208 answer file from the disclosed
  outputs and failures of a history (one `sequence` entry per recorded attempt). An output that the export did not
  disclose is omitted and listed, so the activity answers `NO_MOCK`. Classification follows ADR-0033 and feature 016:
  `secret` is never exported, `pii` only when the export revealed it.

### Interceptors

- **D1416 — One generic call shape per side; interceptors observe and decorate.** A client interceptor receives
  `{operation, metadata, value}` with `operation` from a closed, additively growing set and calls `next`; a worker
  interceptor has `execute`, `heartbeat` and `report`. A generic shape keeps the five SDKs aligned as operations are
  added; per-method interfaces would drift. The first registered is outermost. Interceptors may observe values
  (read-only), add headers outside the protected set and refuse a call; they never transform payload bytes. The only
  payload-transforming stage is feature 031's data converter (FR-031-016), placed between the chain and the transport,
  so interceptors see application values, as with Temporal's data converter (coordinator decision C5; this replaces
  the payload transform of the first draft).
- **D1417 — The chain runs once per logical call.** Decorations are fixed before the first attempt and every retry
  sends the same bytes and headers, because the server fingerprints idempotent requests and a decoration that changes
  per attempt would make retries differ. Feature 031 applies the same once-per-call rule to its converter
  (FR-031-013); 031 depends on 028 only for this ordering.
- **D1418 — Failure mapping adds one catalog code.** `INTERCEPTOR_FAILED` (origin `sdk`, non-retryable) on the client,
  with reason `AFTER_RESPONSE` when the server had answered. On the worker an interceptor failure is the handler's
  failure (`sdks/SPEC.md` §6.5): a raised `ActivityFailure` keeps its code and retryability, anything else is
  `ACTIVITY_ERROR`, retryable, named by its type. This is the platform's existing rule for "the effect may have
  happened and the handler then failed"; no new server semantics.
- **D1419 — No server-side interceptors or plugins.** Constitution V excludes in-process plugins; the server gains no
  hook. SDK interceptors run in the user's process with the user's trust.

### VS Code extension

- **D1420 — The extension is a client of the CLI only.** Every network call and every validation goes through
  `sagat` with `--json`, so there is one token-handling path (feature 012 D210, feature 011 D197) and one codec. The
  extension makes no REST call of its own.
- **D1421 — One graph model.** The IR-to-graph model and layout move from the Studio into
  `frontend/packages/workflow-graph`, used by the Studio and the extension's webview; the Studio's existing tests are
  the regression guard. Rejected: a second mapping in the extension or a graph computed by the CLI (layout would still
  be client-side and two mappings would drift).
- **D1422 — Workspace trust is `limited`.** Validation and preview run in an untrusted workspace; every command that
  runs the CLI against a server or starts a worker is disabled there. `sagat.cliPath`, `sagat.server`, `sagat.grpc`
  and `sagat.allowNonDev` are read from user or machine scope only, in trusted workspaces too: a repository must not
  choose the binary that runs, the origin that receives the token, or widen the guard (security review S11). A token
  is stored per server origin and passed only when the configured server has that origin; `sagat.namespace` may come
  from the workspace (it selects, it does not authorize).
- **D1423 — The extension sends no telemetry of its own.** It passes the user's `OTEL_*` variables to the CLI, whose
  opt-in command spans exist (feature 012 G238); the extension adds none.

### Git sync

- **D1424 — PostgreSQL stays the authority.** Git is a mirror of active versions and a source of drafts. An import
  never publishes; publication keeps the rules of ADR-0018, tightened for imported drafts by D1432 (O1401 asks
  whether a merge may publish).
- **D1425 — The export obligation is written with the activation.** The row is inserted in the transaction of
  ADR-0018 that moves the pointer (the outbox pattern of ADR-0002/0012). Rejected: scanning `workflow_activations`
  with a cursor — rows become visible in commit order, not timestamp order, so a cursor can skip one.
- **D1426 — Provider REST APIs, not `git`.** The synchronizer reads and writes through the GitHub and GitLab REST APIs
  (contents, trees, commits, refs, pull/merge requests). No repository is cloned, so no hook, submodule, LFS pointer or
  repository-sized object is fetched; reads are bounded per file. Loops are prevented by comparing digests: a
  repository document equal to the active one changes nothing. Placement: Git sync is a delivery of a fixed,
  platform-defined shape (the 011 manifest and canonical documents) to a tenant-configured destination, so the engine
  sends it through the guarded egress client holding only that binding's credential — category (b) of coordinator
  decision C3, recorded by feature 021 in ADR-0055, as ADR-0025 already does for outbound deliveries. No request body
  is authored by a tenant workflow, so no managed executor is needed.
- **D1427 — One branch per workflow, latest active wins.** An export renders the workflow's active version at execution
  time; earlier pending exports of that workflow become `SUPERSEDED`. The branch `sagat-sync/<workflow>` holds one
  open pull request at a time, updated with a new commit when a newer version is published before the merge.
- **D1428 — Webhook per binding, signatures only.** `POST /v1/git-webhooks/{bindingId}` verifies GitHub's
  `X-Hub-Signature-256` or GitLab's signing token over the raw body (GitLab's `webhook-timestamp` within 5 minutes;
  GitHub signs no timestamp, so its replay guard is the delivery-id receipt kept for the receipt window), then keeps
  only the extracted envelope (event, ref, before, after, repository id, delivery id). The legacy `X-Gitlab-Token` is refused: a plain
  shared value proves no integrity. Every refusal is the same `401` (ADR-0012 §3). Nothing in a payload is trusted
  for content: a replayed or forged-but-signed delivery can at most enqueue an import of the current head, which is
  read through the authenticated provider API. Before any secret is resolved the route applies a per-source-address
  and a per-binding token bucket; verification secrets are cached in memory for at most 60 seconds per binding
  revision (zeroized on eviction), so a flood does not reach OpenBao; an unknown binding performs the same HMAC work
  with a random per-process key, so the answer's timing does not reveal whether a binding exists (security review,
  minor items).
- **D1429 — Webhooks are hints.** A head reconciliation per binding (default every 5 minutes) compares the tracked
  head with the last imported commit and enqueues an import when they differ.
- **D1430 — No automatic merge.** When both sides changed, a conflict is recorded with the three documents; a person
  takes the repository's or keeps the draft. A workflow removed from the manifest is reported, never deleted.
- **D1431 — Tenant-owned credentials only.** Each binding uses its tenant's GitHub App or GitLab token from namespace
  secrets. A platform-operated GitHub App shared by tenants would put one key in front of every tenant and is out of
  scope.
- **D1432 — One binding per namespace; imports act as `system:git-sync`, and that weakens separation of duties.** A
  reserved actor writes imported drafts; the commit author and committer are stored as unverified provenance. Because
  ADR-0018 decision 4 excludes only the last person to save the draft, the commit author could approve their own
  change (architecture review finding 3, security review S10). Compensation: (1) a publication of a draft last saved
  by `system:git-sync` needs one approval more than the namespace's policy, at least one, counted at publication; (2)
  `DRAFT_AND_REQUEST` opens requests only while the binding's latest probe shows that the tracked branch requires at
  least one approving review, otherwise the import creates drafts only and reports `GIT_REVIEW_NOT_VERIFIED`; (3) the
  request shows the commit, author and committer as unverified. This tightens ADR-0018 for one new actor and changes
  nothing for drafts saved by people; it is stated as such in the spec's assumptions.
- **D1433 — Exports pin the binding revision.** A change of repository, branch or directory supersedes pending jobs of
  the older revision and enqueues a full re-export of active versions under the new one; a retry never jumps to
  another repository mid-way (the rule of FR-017-002 for connections).

### Template hub

- **D1434 — Curation is a property of the source.** The release-bundled index (signed in the release job) and
  operator-configured indexes with operator-pinned keys are the only sources; there is no tenant source and no
  submission. This keeps the hub outside the constitution's excluded "connector marketplace".
- **D1435 — Ed25519 over RFC 8785, with version and expiry.** The index is signed like script artifacts (Ed25519), over
  the canonical form the SDKs already produce; a version lower than the highest seen is a rollback and an expired index
  is a freeze (TUF's two cheapest protections). Rejected for V1: full TUF roles and delegations (no threshold signing
  infrastructure exists) and Sigstore verification at import (needs online transparency-log access from air-gapped
  deployments); the release-bundled index is also covered by the release attestations.
- **D1436 — Import creates a draft and a list.** The import validates with the codec, creates a workflow or script
  draft with provenance and returns what must be prepared; it never publishes, binds, creates secrets or executes.
- **D1437 — Two new catalog permissions, and one test-server-only check.** `gitsync:manage` (admin) and
  `gitsync:read` (admin, developer, operator) join the catalog. `testing:clock` is not added to the production catalog
  or to any production role: the test server defines it and maps it to the admin and developer roles of its own
  tenant (architecture and security reviews, minor items). Conflict resolution needs `definition:write`;
  template import needs `definition:write` or `script:write`; catalog reading needs `definition:read`.

### Conformance

- **D1438 — New scenarios use the test server.** `time-skipping`, `interceptors`, `history-replay` and
  `activity-context` scenarios run against the test-server composition. The existing `elapseTimer` and
  `elapseRetryDelay` control actions stay as they are (their goldens are reviewed); replacing them is a separate,
  reviewed change (G1416).
- **D1439 — The ADR records the time split.** The separation of workflow and infrastructure time and the rule that only
  a test deployment may move workflow time are hard to reverse and touch every feature that stores an instant:
  [ADR-0063](../../docs/adr/ADR-0063-only-a-test-deployment-moves-workflow-time.md) (Proposed). The Git and template
  decisions are reversible feature decisions and stay here.

## Gaps

- **G1400 — One clock per test server.** Tests that need independent time need separate servers; per-namespace clocks
  are not designed.
- **G1401 — SDKs do not start a test server.** A test suite starts it through the CLI or compose; no SDK downloads or
  launches it.
- **G1402 — Interceptors see streamed blobs as metadata only.** For `uploadRunInput`, `writeBlob` and `readBlob` the
  chain receives the operation, size and digest, never the stream; blob envelopes are feature 031's.
- **G1403 — The activity test context is not a server.** Leases, retries, heartbeats timing out and secret access
  authorization are proven only on a test server.
- **G1404 — US03 waits for feature 020's history format.** Until it exists there is no exported document carrying the
  codec-level event details; the public journal (`JournalEvent`) does not carry them.
- **G1405 — Mixed-class comparisons inside SQL are reviewed, not proven.** Rust types, the manifest, the column-default
  and `pg_proc` checks catch most; a predicate written in raw SQL that compares a `workflow_due` with an
  `infrastructure` column is found by a lint over the manifest's column names and by review, not by the compiler.
- **G1406 — Git sync carries workflows only.** Scripts (feature 011 G199), connections, secrets, resources and app
  documents (027) are not synchronized.
- **G1407 — Commit authors are unverified.** The platform records them; it cannot prove them.
- **G1408 — Conflicts are resolved one workflow at a time.** No bulk resolution in V1.
- **G1409 — GitLab before 19.1 has no signed webhooks.** Bindings to it work by head reconciliation only.
- **G1410 — Catalog trust is single-key, and the first refresh trusts on first use.** No threshold signatures or
  delegations; a compromised key is handled by the operator removing it from the pinned set; no revocation list is
  distributed. A fresh deployment has no stored highest version, so its first refresh accepts any unexpired index,
  including an older one; the operator may pin a `minimumVersion` per index to close this window.
- **G1411 — Extension distribution is undecided beyond GitHub Releases** (O1400).
- **G1412 — Resource-type templates need feature 021.**
- **G1413 — Costs are unmeasured.** The workflow-time function in production, skip throughput, synchronizer latency and
  catalog refresh have benchmark criteria only.
- **G1414 — Replay compares structure.** Payload digests are compared, values are not; a semantic change that keeps
  every structural field is not detected.
- **G1415 — No web or remote-only VS Code.** The extension needs a local `sagat`.
- **G1416 — Existing conformance scenarios still back-date rows.**
- **G1417 — Secret scanning of rendered Git content is heuristic.** It is defence in depth (ADR-0031 point 5 wording),
  not proof that a document holds no confidential value.

- **G1418 — Test-deployment spans carry workflow-time instants.** Journal-derived spans of a test deployment are
  stamped in workflow time; with a large offset a tracing backend may refuse or misplace them. The test server does not
  export telemetry by default.
- **G1419 — Branch-review verification is a snapshot.** The probe reads branch protection when it runs (and before each
  import); a protection removed between two reads is seen at the next one. GitHub exposes the rules of a branch at
  `GET /repos/{owner}/{repo}/rules/branches/{branch}`, whose `pull_request` rule carries
  `required_approving_review_count` (<https://docs.github.com/en/rest/repos/rules>), besides classic branch protection;
  the App permission it needs was not confirmed. GitLab documents merge-request approval rules for the Premium and
  Ultimate tiers (<https://docs.gitlab.com/user/project/merge_requests/approvals/rules/>), so on other tiers
  `DRAFT_AND_REQUEST` stays unavailable.
- **G1420 — Retention ages run on workflow time in a test deployment.** Advancing the clock far can purge run content
  that a test still wants to inspect; this is the consequence of measuring retention over workflow-class instants.

## Owner questions

- **O1400 — Where is the VS Code extension published?** Options: GitHub Releases only (VSIX attached with checksums and
  attestations), or also Open VSX and the Visual Studio Marketplace under a project publisher. **Recommended default:**
  GitHub Releases only, consistent with owner decision 12; revisit with the first release that publishes SDK packages
  to registries.
- **O1401 — May a merge into the tracked branch publish automatically?** Windmill's automatic sync deploys commits.
  **Recommended default:** no. An import creates drafts and, with `DRAFT_AND_REQUEST`, a publication request bound to
  the document's digest; publication remains an explicit platform action under the namespace's approval policy.
- **O1402 — May operators add catalog indexes?** **Recommended default:** yes, deployment-level only, each with its own
  pinned Ed25519 keys and the same verification; no tenant-level sources and no public submission. This keeps
  curation with the operator and the hub outside "marketplace".
