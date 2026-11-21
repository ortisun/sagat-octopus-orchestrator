# Implementation plan — 028 developer tooling

Status: Proposed — not started.

**Spec:** [spec.md](spec.md) · **Research:** [research.md](research.md) · **Data model:** [data-model.md](data-model.md) ·
**Contracts:** [contracts/](contracts/) · **Decision:** [ADR-0063](../../docs/adr/ADR-0063-only-a-test-deployment-moves-workflow-time.md) (Proposed)

## Summary

Five deliveries share one rule — no second engine, no second codec, no second semantics. A test server composes the
production crates with a virtual *workflow* clock; the SDKs gain interceptors (observation and decoration only;
payload bytes are transformed only by feature 031's data converter, coordinator decision C5), a test environment and
an activity test context; the CLI gains machine output that a VS Code extension drives; the engine gains a Git
synchronizer that mirrors activations and turns commits into drafts; and the control plane serves a signed, curated
catalog imported as drafts.

## Technical context

- **Language/runtime:** Rust (pinned toolchain), Tokio, Axum, tonic, SQLx; TypeScript (Studio, VS Code extension,
  shared graph package); the five SDK languages.
- **Storage:** PostgreSQL (authority); no new broker subject; no object-store use beyond existing blobs.
- **New crates/apps:** `apps/test-server`, `crates/infra-testing` (adapter: clock store, test migration, replay
  codec adapter), `crates/infra-git` (adapter: GitHub/GitLab REST through `crates/infra-egress`).
- **Testing:** unit, integration on real PostgreSQL (also as the runtime role), contract (OpenAPI, CLI output schema,
  index schema), five-SDK conformance on the test server, fault (crash points, duplicates, lost hints), security
  (tenant battery, canaries), VS Code extension tests in the editor's test host, browser journeys for the Studio.
- **Constraints:** production behaviour and plans unchanged; contracts additive; nothing executes in the API or
  scheduler that is not already allowed; provider effects at-least-once.

## Architecture and crate placement

| Concern | Placement |
|---|---|
| Time classes (`WorkflowInstant`, `InfrastructureInstant`), skip stop selection, quiescence verdict, clock ceilings | `crates/domain/src/time.rs`, `crates/domain/src/testing.rs` (pure) |
| Replay comparison of a derived projection with a recorded one | `crates/domain/src/execution/replay_check.rs` (pure) |
| Import decision `decide(base, draft, repository, active) → Skip | Replace | Create | Conflict | Removed` and status derivation | `crates/domain/src/gitsync.rs` (pure) |
| Index verification rules (version, expiry, validity, digests), entry kinds, requirements | `crates/domain/src/templates.rs` (pure; signature check through a port) |
| Test clock, skip orchestration, replay use case | `crates/application/src/testing.rs`, `history_replay.rs` (ports `TestClockStore`, `PassDriver`, `EventCodec`) |
| Git sync bindings, conflicts, export/import use cases | `crates/application/src/gitsync.rs` (ports `GitSyncStore`, `GitProvider`, `SecretResolver`) |
| Catalog and import use cases | `crates/application/src/templates.rs` (ports `TemplateStore`, `IndexSource`, `SignatureVerifier`) |
| Workflow-time SQL function, time-classification manifest, Git and template tables, export row in the activation transaction | `crates/infra-postgres` (`migrations/`, `time-classification.json`, `src/gitsync.rs`, `src/templates.rs`, `src/versioning.rs`) |
| Test migration (marker, clock tables, replaced function bodies, advance function), clock store, replay codec adapter | `crates/infra-testing` |
| GitHub/GitLab REST clients, App assertion signing, webhook signature verification | `crates/infra-git` |
| Export, import and head-reconciliation loops (leased) | `crates/scheduler/src/gitsync.rs` |
| Manifest rules (closed schema, entry limits, path rules) and manifest parsing, shared by the CLI and the import loop | rules in `crates/domain/src/manifest.rs`, parsing in `crates/infra-ir/src/manifest.rs` behind an application port (moved from `apps/cli/src/manifest.rs`; `crates/scheduler` cannot depend on an app or an adapter) |
| REST routes for bindings, sync status, conflicts, webhooks, catalog, imports | `crates/api-http/src/routes/{gitsync,templates}.rs` |
| Testing routes and composition (control plane + engine + issuer), on-demand passes | `apps/test-server` (routes are not in `crates/api-http`) |
| CLI `--json`, `inspect --follow`, `history`, `test-server`, `template` | `apps/cli/src/{output,history,test_server,template}.rs` and existing command modules |
| SDK interceptors, test environment, activity test context | `sdks/{typescript,go,python,dotnet,java}` |
| Shared graph model | `frontend/packages/workflow-graph` (extracted from `frontend/apps/studio/src/features/workflows/canvasModel.ts` and `src/model/layout.ts`) |
| VS Code extension | `frontend/apps/vscode-extension` |
| Studio screens (sync settings/status/conflicts, catalog/import, provenance) | `frontend/apps/studio/src/features/{gitsync,templates}` |
| Release-bundled catalog | `templates/catalog/` (index source; signed by the release job) |

`scripts/check-architecture.py` gains two rules: no package except `apps/test-server` and `tests` may depend on
`sagat-infra-testing`, and no package except `tests` may depend on `sagat-test-server`. No shared crate gains a cargo
feature for testing (D1408). `scripts/check-production-image.py` (CI) fails when `infra/docker/Dockerfile`'s
`cargo build -p` list or a `COPY --from=builder` line names the test server, or when
`cargo tree -p sagat-control-plane -p sagat-execution-engine` contains `sagat-infra-testing` (FR-028-002).

## Workflow time and the test server

1. **Classify.** `crates/infra-postgres/time-classification.json` lists every time column with its class and owners.
   A migration test compares it with `pg_catalog` (every `timestamp`/`timestamptz` column of the migrated schema), like
   the storage manifest, and also checks column defaults (37 default to a built-in clock at the baseline) and the
   bodies of SQL functions in `pg_proc` (43 definitions in the migrations). A lint over `crates/infra-postgres/src`
   fails when `now()`, `clock_timestamp()`, `statement_timestamp()` or `transaction_timestamp()` is compared with or
   assigned to a `workflow_due` or `workflow_record` column, and on a predicate that compares columns of different
   classes (G1405: lint plus review). The rule for "now": a due check, a lag metric or a retention age over a
   workflow-class column uses workflow now — the scheduler-lag query of `operations.rs` (timers, waits, retries, run
   deadlines) and the telemetry purge of `telemetry.rs` over `runs.finished_at`; lease lag, `script_test_runs`
   retention (`maintenance.rs`), the telemetry projector's own lag (`behind_since`) and every job, window and
   credential keep infrastructure now. The cross-feature catalog is [contracts/time-classes.md](contracts/time-classes.md)
   (coordinator decision C13).
2. **Sample once, derive, type twice.** The run transaction takes one infrastructure sample after the run lock
   (`WITH tick AS MATERIALIZED (SELECT clock_timestamp() AS infra) SELECT infra, sagat_workflow_from(infra) ...`,
   replacing today's single sample at `execution.rs`) and derives the workflow instant from it; two volatile calls are
   never used for one decision, so in production the two values are equal by construction (an equality test at offset
   zero proves it). The domain receives `WorkflowInstant` for timers, deadlines, retry release, loop delays, approvals,
   chain deadlines and journal instants, and `InfrastructureInstant` for leases. Candidate selection bounds
   workflow-due predicates with `sagat_workflow_from(now())`; the ADR-0020 test that refuses `clock_timestamp()` in pass
   files is extended to refuse a bare `now()` against a workflow-due column. Plan-equality tests
   (`EXPLAIN (FORMAT JSON)` of every pass query before/after) prove inlining.
3. **Test deployment.** `apps/test-server` refuses unless `SAGAT_ENV=dev`; creates (or accepts) a database with no
   tenant rows or with its own marker; applies the production migrations, then the test migration from
   `crates/infra-testing` (marker row, `sagat_test_clock`, `sagat_test_clock_advances`, the replaced body of
   `sagat_workflow_from`,
   `sagat_test_clock_advance(...)` `SECURITY DEFINER`); seeds one tenant, workspace, namespace, admin and worker
   identity; starts the embedded issuer (feature 012's development identity provider as a library, loopback, key per
   start) and writes token files `0600`; serves REST, the worker service and `/v1/testing/*`.
4. **Production refusal.** Control plane and engine start-up (`apps/control-plane`, `apps/execution-engine`) check that
   `sagat_deployment_profile` does not exist and that the body of `sagat_workflow_from` is the canonical one, and exit
   with code 2 otherwise (the existing exit for a role-privilege refusal). The primary control is ownership: the
   function belongs to the migration role, which the runtime role cannot act as, so the runtime role cannot replace
   it; the engine's monitor round repeats the body check and stops the engine with the same refusal on a mismatch. The check takes a
   `DeploymentProfile` chosen by the composition root at compile time: the production binaries pass `Production`
   (refuse the marker), `apps/test-server` passes `Test` (require it). No environment variable or setting selects the
   profile. The test server is not a Dockerfile target of the production image; the release job builds it in a
   separate invocation.
5. **Skip algorithm** (`crates/application/src/testing.rs`), per call: read the clock; until the target or a limit:
   (a) find the next workflow-due instant `d` (generated query over the manifest's `workflow_due` columns of
   non-terminal rows); (b) if none, or `d` is beyond the target, set the clock to the target (when the target is an
   instant or a duration, resolved once at the start of the call) and stop; (c) otherwise advance to `d` (one transaction, epoch + 1); (d) run one `Scheduler::tick` and one
   dispatcher pass; (e) wait for quiescence up to the bound, polling; on timeout return `BLOCKED` with counts (or `409
   TIME_ADVANCE_BLOCKED` when no stop was made); (f) if the target is a run or chain and it is terminal, stop. With
   nothing due, quiescent and a run target not terminal: `WAITING_EXTERNAL` and the run's waiting kinds. `jump` mode
   performs (c) once to the target, then (d), and returns without waiting unless asked.
6. **Shifted-clock regression.** The engine, saga and workflow-primitive integration suites gain a mode that runs on the
   test-server composition with a 400-day initial offset; results must equal the zero-offset run.

## SDK interceptors and test environment

- Client: `interceptors: ClientInterceptor[]`, each `interceptCall(call, next)`; `call` has `operation` (closed set:
  `START`, `START_WITH_BLOB`, `GET`, `LIST`, `OPERATE`, `SIGNAL`, `PUBLISH_EVENT`, `QUERY`, `UPDATE`,
  `DECIDE_APPROVAL`, `LIST_APPROVALS`, `INSPECT_CHAIN`, `LIST_CHAIN_RUNS`, `QUERY_CHAIN`, `UPDATE_CHAIN`,
  `SIGNAL_CHAIN`, `CANCEL_CHAIN`, `EVENTS`, `WAIT_FOR_COMPLETION`, `UPLOAD_RUN_INPUT`, plus test-environment
  operations), read-only `metadata`, a read-only `value` and a bounded set of headers it may add outside the protected
  set. After the chain, feature 031's data converter (when configured) encodes, the SDK freezes the bytes and headers
  and enters its retry loop; the answer passes the converter and then the chain in reverse.
- Worker: `interceptExecute(task, next)`, `interceptHeartbeat(beat, next)` (metadata only), `interceptReport(report,
  next)`; 031's converter decodes the step document before `execute` and encodes the output after the report chain has
  observed it. Exceptions follow FR-028-023. No payload-transforming hook exists in this feature (decision C5). Contract: [contracts/sdk-interceptors.md](contracts/sdk-interceptors.md).
- Test environment and activity test context: [contracts/test-environment-sdk.md](contracts/test-environment-sdk.md).
  No protobuf change; the worker protocol stays at its current minor.

## History replay

`POST /v1/testing/history-replays` (test server) validates the document against
[contracts/history-replay.md](contracts/history-replay.md), decodes each event with `event_codec::decode` (through the
`crates/infra-testing` adapter), folds with `RunState::replay_addressed`, decodes the pinned version with the server's
IR codec and capability set, and compares projections with the pure check. It writes only an audit record (in its own
transaction, before returning). The CLI's `history answers` and `history compare` are offline over the document and a
run's journal read through REST.

## VS Code extension

The extension (`frontend/apps/vscode-extension`) activates on JSON/YAML documents whose `apiVersion` is the workflow
IR's and on manifests; runs `sagat ... --json` as a child process with `SAGAT_TOKEN` in its environment; maps JSON
pointers to text ranges with a position-preserving parser; renders the graph webview from
`frontend/packages/workflow-graph`; follows runs with `sagat workflow inspect --follow --json`. Commands, settings,
messages and CSP: [contracts/vscode-extension.md](contracts/vscode-extension.md); CLI output:
[contracts/cli-output.schema.json](contracts/cli-output.schema.json). The CLI version range is read from
`sagat version --json`.

## Git sync

- **Export.** `crates/infra-postgres/src/versioning.rs` inserts the `git_sync_exports` row and marks older pending rows
  of the same workflow `SUPERSEDED` inside the ADR-0018 transaction when the namespace has an `ACTIVE` binding. The
  engine's export loop claims a row (lease, attempt fence), resolves credentials (secret service, OpenBao), mints the
  token (GitHub: RS256 assertion → installation token restricted to the repository; GitLab: the stored token), reads the
  tracked head, and renders the active document (canonical JSON from `sagat-infra-ir`) plus the manifest updated in
  the 011 format. It scans the rendered bytes with the existing secret-output checks. For `PULL_REQUEST`: look up branch
  `sagat-sync/<workflow>` and its open request; if the branch tree already equals the rendered tree, record and stop;
  else commit on it (parent: the branch head, or the tracked head when created) and open the request if none is open.
  For `DIRECT_PUSH`: commit on the tracked head and update the ref without force; a non-fast-forward answer re-reads
  the head and retries within the attempt; a document changed in Git since the base digest becomes a conflict instead
  (no overwrite). A `RESYNC` export renders every active workflow in one commit.
- **Import.** The webhook route verifies the signature, then in one transaction records the receipt and enqueues or
  coalesces the binding's import. The import loop reads the tracked head, the manifest (the rules moved from
  `apps/cli/src/manifest.rs` into `crates/domain/src/manifest.rs` and `crates/infra-ir/src/manifest.rs`: closed schema,
  paths below the directory, no links — file modes are read from the tree) and each listed
  document within the ceilings, computes digests, and applies `gitsync::decide` per workflow in its own transaction.
- **Head reconciliation** (leased singleton per shard set, like ADR-0017's monitor) enqueues an import when the head
  differs from the last imported commit, every `reconcileIntervalSeconds`.
- **Review verification.** The probe (and each import with `DRAFT_AND_REQUEST`) reads whether the tracked branch
  requires at least one approving review: GitHub's rules for a branch (`GET /repos/{owner}/{repo}/rules/branches/{branch}`,
  `pull_request.required_approving_review_count`) or classic branch protection; GitLab's merge-request approval rules
  (Premium and Ultimate tiers). Without a positive answer the import opens no request and reports
  `GIT_REVIEW_NOT_VERIFIED`. Publication of a draft last saved by `system:git-sync` counts the namespace's required
  approvals plus one (at least one), in the ADR-0018 publication transaction; the request shows the unverified commit
  author and committer (research D1432).
- **Placement.** The synchronizer runs in the engine and sends only platform-defined content (the 011 manifest and
  canonical documents) through `crates/infra-egress` with that binding's credential: category (b) of coordinator
  decision C3, recorded by feature 021 in ADR-0055. A refused destination is `GIT_DESTINATION_DENIED`.
- REST: [contracts/git-sync.openapi.yaml](contracts/git-sync.openapi.yaml).

## Template hub

The control plane loads the release-bundled index at start and, for each operator-configured index
(`SAGAT_TEMPLATE_INDEXES`: id, HTTPS URL, pinned keys, optional `minimumVersion`), refreshes through the egress guard
on a schedule. Without `minimumVersion` the first refresh of a fresh deployment trusts the first unexpired index it
sees (G1410). Refresh and
import both verify (`crates/domain/src/templates.rs` rules, `ring` Ed25519). Import validates with the IR codec (or
the script source rules of feature 005), creates the draft with the existing create paths and provenance in one
transaction. REST: [contracts/templates.openapi.yaml](contracts/templates.openapi.yaml); index:
[contracts/template-index.schema.json](contracts/template-index.schema.json).

## Transaction boundaries

| Operation | Commits together | Outside the transaction |
|---|---|---|
| Clock advance (one stop) | clock row (offset, epoch), advance history row, audit `testing.clock.advanced` | quiescence polling; the passes (their own existing transactions) |
| History replay | audit `testing.history.replayed` | decoding, folding, comparison (pure) |
| Activation (publish, promote, rollback, activate) in a bound namespace | ADR-0018 records + `git_sync_exports` row + older pending rows `SUPERSEDED` + document status `EXPORT_PENDING` | nothing about the provider |
| Export claim | state `LEASED`, attempt + 1, lease token hash, infrastructure expiry | credential resolution, provider calls |
| Export record | lease/attempt fence check, export state and provider ids, document base digest/commit/status, audit, binding health counters | — |
| Webhook admission | receipt (dedupe), import enqueue/coalesce | rate limits, then secret resolution (cached ≤ 60 s) and signature verification (before) |
| Import of one workflow | draft compare-and-swap or create, draft provenance, document state, import entry outcome, optional publication request, conflict row, audit | provider reads (before) |
| Import finish | import state, binding's last imported commit, audit | — |
| Conflict resolution | conflict state, draft compare-and-swap (TAKE_REPOSITORY), provenance, document status, audit | — |
| Binding create/update/pause/delete | revision, head, secret-access check, audit, supersede pending jobs and enqueue `RESYNC` when the target changed | probe (a job of its own) |
| Catalog refresh | index state (version must not decrease), entries, content-addressed files, index event | fetch and signature verification (before; repeated in the transaction against the stored version) |
| Template import | draft (workflow or script) via existing create path, provenance, import record, audit, quota accounting | — |

The webhook endpoint applies a per-source-address and a per-binding token bucket before resolving any secret, keeps
verification secrets in memory for at most 60 seconds per binding revision (zeroized on eviction), and verifies an
unknown or paused binding with a random per-process key so that the `401` does not reveal by its timing whether the
binding exists; a database error while verifying is a `5xx`, never a verdict.

## Authorization

| Action | Permission | admin | developer | operator | viewer | worker / runner |
|---|---|:-:|:-:|:-:|:-:|:-:|
| Read the test clock and quiescence | `testing:clock` | yes | yes | no | no | no |
| Advance or skip the test clock | `testing:clock` | yes | yes | no | no | no |
| Replay a history (test server) | `definition:read` | yes | yes | yes | yes | no |
| Create, change, pause, delete, probe a binding; redrive jobs; manual import | `gitsync:manage` + current `secret:use` on every referenced secret | yes | no | no | no | no |
| Read binding, status, exports, imports, conflicts | `gitsync:read` | yes | yes | yes | no | no |
| Resolve a conflict | `gitsync:read` + `definition:write` | yes | yes | no | no | no |
| Webhook delivery | the binding's signature (no bearer token) | no | no | no | no | no |
| Browse the catalog | `definition:read` | yes | yes | yes | yes | no |
| Import a workflow template / a script template | `definition:write` / `script:write` | yes | yes | no | no | no |
| Configure catalog indexes | deployment configuration (operator), no API | no | no | no | no | no |

`testing:clock` is not in the production permission catalog and no production role holds it: `apps/test-server`
defines the check and maps it to the admin and developer roles of its own tenant; production answers
`404 ROUTE_NOT_FOUND` for the paths. Every check runs against current membership; foreign resources are opaque
`NOT_FOUND`; denials use the existing restricted denial audit. Imported drafts are written by the reserved actor
`system:git-sync`; their publication needs the namespace's approvals plus one and, for requests opened by import,
verified branch review (the stated weakening of ADR-0018 decision 4 and its compensation, research D1432).

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| One advance (duration) | 3,660 days | 3,660 days |
| Total virtual offset | 36,600 days | 36,600 days |
| Stops per skip call | 1,000 | 10,000 |
| Quiescence wait per stop (real time) | 10 s | 60 s |
| Wall time per skip call | 120 s | 600 s |
| Clock advance history rows per test deployment | 100,000 | 100,000 (then advances are refused) |
| History document size | 8 MiB | 32 MiB |
| Events per history | 10,000 | 100,000 |
| Concurrent replays per test server | 4 | 16 |
| Interceptors per client or worker host (SDK-local) | 16 | 64 |
| Bindings per namespace / per tenant | 1 / 50 | 1 / 500 |
| Pending exports per binding | 100 | 1,000 |
| Manifest entries read per import | 500 | 500 (feature 011) |
| Manifest bytes / document bytes | 256 KiB / 1 MiB | 1 MiB / 1 MiB |
| Webhook body bytes (larger → 413, head reconciliation catches up) | 1 MiB | 4 MiB |
| Webhook receipt window / rate per binding / rate per source address | 7 days / 10 per s / 20 per s | 30 days / 50 per s / 100 per s |
| Webhook verification-secret cache | 60 s | 60 s |
| Provider call deadline / job wall time | 10 s / 2 min | 30 s / 10 min |
| Attempts before `DEAD` / consecutive dead before pause | 12 / 20 | 12 / 20 |
| Head reconciliation interval | 300 s | min 60 s, max 3,600 s |
| Open conflicts per binding | 500 | 500 |
| Catalog indexes per deployment | 4 | 16 |
| Entries per index / index bytes | 500 / 2 MiB | 2,000 / 8 MiB |
| Files per entry / bytes per file / bytes per entry | 32 / 1 MiB / 4 MiB | 64 / 1 MiB / 4 MiB |
| Index validity (expiry − signing time) | 30 days | 365 days |
| Template imports per namespace per hour | 60 | 600 |
| CLI JSON read by the extension / one follow line | 16 MiB / 1 MiB | 64 MiB / 1 MiB |

Effective values are the minimum of request, namespace and deployment; zero is not unlimited; admission fails before
work. Binding settings, conflict documents and import provenance are retained tenant content under ADR-0027; imports
count against the existing workflow and script count quotas.

## Failure and recovery

| Failure | Expected outcome | Proof |
|---|---|---|
| Test server crashes after the clock commit, before the pass | next start (or next call) runs the pass; due items fire once | fault test |
| Two advances race | one epoch wins, the other `409 CLOCK_EPOCH_CHANGED` | race test |
| Advance with a leased attempt (`jump`, quiescence waived) | attempt deadline in workflow time may time it out; late report `STALE_ATTEMPT` | integration |
| Timer and signal due at one stop | first commit wins; loser audited no-op | 20-round race |
| Production binary on a test database | exit 2 with typed refusal | integration |
| Interceptor raises before / after the server answered | `INTERCEPTOR_FAILED` / with `AFTER_RESPONSE`; retry with same key replays | conformance |
| Lost response with a decorating interceptor | the chain ran once; retries resend identical bytes and headers; one run | conformance |
| Probe cannot show required reviews (`DRAFT_AND_REQUEST`) | drafts only, `GIT_REVIEW_NOT_VERIFIED`; no request | integration (provider stub) |
| Webhook for an unknown binding under load | rate limit before any secret read; uniform `401` with the same verification work | integration |
| Workflow-time function body replaced on a running engine | monitor round detects it; the engine stops with the start-up refusal | integration |
| Export: crash after provider commit, before record | lease expires; next attempt finds the branch tree equal and records | fault (provider stub) |
| Export: crash after pull request, before record | next attempt finds the open request by branch and records | fault |
| Export: duplicate claim / stale lease | attempt fence rejects the stale record | fault |
| Provider down during publish | publication commits; export retries; `DEAD` after 12; redrive | integration |
| Provider rate limit | backoff honours `Retry-After` / reset headers | unit + stub |
| Webhook duplicated / replayed outside window | receipt dedupe / `401` | integration |
| Webhook lost | head reconciliation enqueues the import | fault |
| Import races a Studio edit | draft compare-and-swap fails → conflict, no overwrite | race test |
| Crash between two workflows of an import | committed ones stay; import resumes the rest | fault |
| Credential revoked or rotated | job fails `GIT_CREDENTIALS_INVALID`, retried; binding paused after 20 dead | integration |
| Catalog index rolled back / expired / tampered | refused, previous verified state kept | integration |
| Catalog refresh races an import | import re-verifies against the stored index version | race test |
| Restart of engine/control plane | loops resume from rows; nothing in memory is required | restart test |

Lost broker delivery does not apply to the synchronizer (it uses no NATS hint); the loops poll their tables, and the
outbox of feature 004 is untouched. Failed compensation does not apply (no saga is introduced); the engine suites under
a shifted clock include the saga compensation-failure cases.

## Observability

- **Spans:** `testing.clock.advance`, `testing.clock.skip` (child per stop), `testing.history.replay`,
  `gitsync.export`, `gitsync.import`, `gitsync.webhook`, `gitsync.reconcile`, `template.index.refresh`,
  `template.import`; attributes from the existing `sagat.*` vocabulary plus `sagat.gitsync.provider` and
  `sagat.outcome`. No payload, document, token or URL with credentials.
- **Metrics (fixed labels):** `testing_clock_advances_total{mode,outcome}`, `testing_clock_stops_total`,
  `testing_quiescence_wait_seconds`, `history_replays_total{outcome}`,
  `gitsync_exports_total{provider,outcome}`, `gitsync_export_duration_seconds{provider,outcome}`,
  `gitsync_imports_total{provider,outcome}`, `gitsync_webhooks_total{provider,outcome}`, `gitsync_backlog{state}`,
  `gitsync_oldest_pending_age_seconds`, `gitsync_conflicts_open`, `template_index_refresh_total{outcome}`,
  `template_imports_total{kind,outcome}`. `outcome`, `state`, `mode`, `kind` and `provider` are closed enums.
- **Audit actions (tenant):** `testing.clock.advanced`, `testing.history.replayed`, `gitsync.binding.created`,
  `.updated`, `.paused`, `.resumed`, `.deleted`, `.probed`, `gitsync.export.recorded`, `.dead`, `.redriven`,
  `gitsync.import.applied`, `gitsync.import.conflicted`, `gitsync.conflict.resolved`, `gitsync.webhook.refused`
  (rate-limited, like webhook ingress refusals), `template.imported`, `template.import.refused`. Catalog refreshes are
  deployment-level: `template_index_events` (append-only) and logs.
- **Namespace alerts (ADR-0017 monitor):** `GIT_SYNC_STUCK` (oldest pending export older than 30 minutes),
  `GIT_SYNC_PAUSED`, `GIT_SYNC_CONFLICTS` (open conflicts). Runbooks: `docs/runbooks/test-server.md`,
  `docs/runbooks/git-sync.md`, `docs/runbooks/templates.md`.

## Migration and compatibility

- Production migrations (timestamps allocated at implementation after the latest on `develop`; the baseline's latest is
  `20261220000000`): the workflow-time function `sagat_workflow_from`; Git sync tables; draft provenance; template tables. All additive with
  grants to `sagat_app`, `down` scripts that refuse to drop non-empty tables, storage and time classification entries.
- The SQL refactor to the workflow-time function is code, not schema: old and new engine replicas compute the same
  instants in production during a rolling upgrade.
- Contracts: `openapi.yaml` gains the Git sync and catalog operations and `PlatformVersion.capabilities`; the testing
  API is a separate document; `error-catalog.json` gains `INTERCEPTOR_FAILED` (catalog version 2, additive); the
  AsyncAPI `OperationalAlertWebhook.kind` enumeration gains the three Git sync alert kinds (additive, baseline
  regenerated on purpose); `worker.proto` and `workflow.schema.json` are unchanged.
- Rollback: disabling the synchronizer (pause all bindings) leaves exports pending; dropping the feature's tables is
  refused while rows exist. Older SDKs ignore `capabilities`.

## Constitution Check

| Principle | Result | How the design meets it |
|---|---|---|
| I. State and retry semantics durable | Pass | Activation and export obligation in one transaction; clock change one transaction; imports by compare-and-swap; leases and attempt fences on jobs; workflow time keeps "first commit wins" |
| II. Modular domain, ports and adapters | Justified | Pure rules in `crates/domain`; ports in `crates/application`; provider clients only in `crates/infra-git`; manifest rules moved out of the CLI so the engine can use them; test-only code in `crates/infra-testing`. The new process `apps/test-server` is justified under principle V in Complexity Tracking (coordinator decision C21) |
| III. At-least-once effects | Pass | Provider writes looked up before repeat; no exactly-once claim; no new compensation |
| IV. Compatibility versioned and pinned | Pass | Immutable versions untouched; imports create drafts; contracts additive; five-SDK conformance for interceptors, test environment and replay calls |
| V. Isolation of code, tenant, secrets | Pass | No code executes at import; credentials are references resolved per job; one tenant per test deployment; webhook per binding with uniform `401`; extension workspace trust |
| VI. Observability and audit | Pass | Spans, fixed-label metrics, audit in the decision's transaction, alerts for stuck sync |
| VII. Redeliverable events | Pass | Webhook receipts deduplicate; webhooks are hints with head reconciliation; dead jobs visible and redrivable |
| VIII. Failure tested honestly | Pass | Crash points, duplicates, races, lost webhook, restart, tenant and ceiling tests planned; replay and costs not claimed before measured |
| Delivery constraints (no marketplace, security-by-default) | Justified | The catalog is curated by source (bundled + operator) with signatures; the test server is a development-only binary refused by production |

### Complexity tracking

| Choice | Cost | Simpler alternative rejected |
|---|---|---|
| Two time classes across the store | Refactor of every time sample, a manifest and a lint | Shifting the whole database clock: every shard lease, session and rate bucket would expire at each stop (research D1401) |
| Export row inside the ADR-0018 transaction | Touches publication code of feature 011 | Polling `workflow_activations` with a cursor can skip rows committed out of order |
| New adapter crates `infra-git`, `infra-testing` | Two crates | Putting provider clients in `infra-connectors` would mix inbound brokers with repository writes; test code in shared crates would ship in production binaries |
| Shared graph package | Moves Studio code | A second IR-to-graph mapping in the extension would drift |
| Generic interceptor call shape | One more indirection | Per-method interceptor interfaces in five languages drift as operations are added |
| New process `apps/test-server` (with `crates/infra-testing`) | A binary and a crate built and released separately | A test mode inside the production control plane and engine: the virtual clock and its routes would ship in production binaries, guarded only by configuration. Justified under principle V (isolation of test-only code from production composition), with feature 017's `apps/database-runner` as the precedent for a separate process; not a split for scale (decision C21) |

## Delivery sequence

1. Foundational: time classes, manifest, SQL functions, store refactor with plan-equality and zero-offset regressions;
   permissions; contract drafts merged.
2. US01: test server, testing API, skip algorithm, SDK clock and test environment, activity test context, conformance.
3. US02: interceptors in five SDKs, catalog code, conformance (fixes the ordering 031's converter relies on).
4. US03: replay (after feature 020's export format is accepted), CLI history commands.
5. US04: CLI JSON output, shared graph package, extension.
6. US05: Git sync tables, export, webhook, import, conflicts, Studio, faults.
7. US06: catalog, signing in the release job, import, Studio and CLI.
8. Gates: security battery, canaries, performance reports, runbooks, quickstart demos, traceability.

## Verification plan

Each task in [tasks.md](tasks.md) names its test. Required release evidence: production refusal (SC-028-002);
manifest completeness and plan equality (SC-028-003); shifted-clock suites; time-skipping conformance in five SDKs;
interceptor conformance with lost-response faults; replay over the conformance corpus; extension diagnostics parity and
canary tests; Git sync crash/duplicate/lost-webhook/race suites against a provider stub and one real-provider journey per
provider (ignored locally without credentials, run in CI with a test repository); catalog tamper/rollback/expiry tests;
tenant battery over every new operation; performance reports in `tests/performance/reports/developer-tooling.md` with
environment, numbers and failures. Unrun tests are reported as unrun.
