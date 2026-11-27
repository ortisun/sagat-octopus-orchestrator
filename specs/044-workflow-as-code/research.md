# Research: workflow as code (design 044)

Registers of decisions (`D044-…`), gaps (`G044-…`) and owner questions (`O044-…`) for
[docs/designs/044-workflow-as-code.md](../../docs/designs/044-workflow-as-code.md). Append-only: a number is never
reused or renumbered, a gap is closed by evidence and never deleted, and a later finding is a new dated entry. The
identifiers keep the old form in this register until the identifier sweep (`docs/governance/identifier-scheme.md`);
prose elsewhere cites them as "Decision 044-001", "Gap 044-001", "Owner question 044-001".

The owner decision this feature implements is OD-2026-11-24-3 ("Sim, entra na 0.3.0", 2026-11-24): workflow as code in
release 0.3.0 in all six SDKs, under a design that starts with a determinism study per language. Its questions are
delegated to the coordinating session under OD-2026-11-20-6, and the acceptance of its architecture decisions and the
settlement of review findings under OD-2026-11-24-4 (2026-11-24, "Quero que os agents continuem por mim. Tomem a melhor
decisão"). The three rows are recorded on the branches `docs/0.3.0-full-parity-scope` and the pull request that carries
OD-2026-11-24-4; they are not on `develop` at the time of writing.

## 1. What exists today (verified 2026-11-24 on `develop` at `546d5aa5`)

- There is no workflow-as-code mechanism. `docs/architecture/workflow-ir-and-sdk-modes.md` lists "Procedural
  code-first (later phase)"; `docs/architecture/execution-semantics.md` calls it "Code-orchestrated (P2)"; ADR-0003
  defers it; `sdks/SPEC.md` says an SDK "is **not** a workflow runtime".
- The journal is `run_events (tenant_id, run_id, seq, event_type, node, attempt, detail jsonb, payload_digest, ...)`,
  append-only by trigger (`crates/infra-postgres/migrations/20261009000100_execution.up.sql`), with 66 event kinds
  (`RunEvent::name`, `crates/domain/src/execution/event.rs`). `runs.run_version` is the sequence of the last event and
  is compared and swapped by `PgRunTx::persist` (`crates/infra-postgres/src/execution.rs`).
- A step completion runs `WorkerService::complete` → `Engine::run` → `PgRunTx::persist` → `PgRunTx::commit`: payloads,
  journal rows, outbox hints for `step.released` and retries, step upserts (timers and waits included), sagas, the run
  row's compare-and-swap, audit and idempotency in one transaction.
- The worker protocol (`specs/001-platform-foundation/contracts/worker.proto`) has `RegisterWorker`, `PollTasks`,
  `StreamTasks`, `Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob`;
  the server's newest minor in code is 1.6 (`crates/application/src/worker.rs`); the register in `sdks/SPEC.md` gives
  1.7 to feature 041, and the branch `docs/delivery-acknowledgement` reserves 1.8 for feature 006.
- Signals to graph runs are never buffered: a signal nobody waits for is refused (`crates/application/src/runs.rs`,
  feature 014 research R5). Updates and queries are declared CEL over declared state, answered by the engine
  (`crates/application/src/declared_operations.rs`). Continue-as-new keeps the same version (feature 014, bounded
  decision 7). Resets copy a journal prefix into a new run (`run.reset`).
- Feature 019 (worker deployments, approved, not built) versions activity builds only and says Temporal-style
  workflow versioning "does not apply here" because the engine does not replay code; this design changes that premise
  for code versions only (Decision 044-035).

## 2. Decisions

- **D044-001** (2026-11-24) A code workflow is a decider. Each workflow task delivers the events since the last
  decision and returns commands; every effect-bearing command is an IR node fragment executed by the existing engine.
  No second execution engine. ADR-0103. Reason: the one transaction of principle I, compensation, uncertain effects,
  leases and gates stay single-sourced.
- **D044-002** (2026-11-24) The workflow task is a lease of the reserved step `~workflow` (reserved activity name
  `sagat.workflow`, task kind `WORKFLOW`): no heartbeat, time-to-live of `workflowTaskTimeoutMs` (default 10,000 ms,
  1,000 to 60,000 ms), expiry re-leased with generation + 1 without review, no secrets. ADR-0103. Reason: a workflow
  task has no external effect; its commands act only when committed.
- **D044-003** (2026-11-24) The activation boundary: events committed while a workflow task is started belong to the
  next activation; the boundary is the pair of journal events `workflow_task.started` and `workflow_task.completed`.
  ADR-0104. Alternative rejected: buffering such events outside the journal until the completion (another store to
  keep consistent, and a signal that is not a fact until later).
- **D044-004** (2026-11-24) Command matching `standard` compares kind and target; `strict`, chosen per version,
  also compares the SHA-256 of the canonical input. Reason: `standard` is Temporal's and does not refuse harmless
  input changes; `strict` is for authors who want input drift to count.
- **D044-005** (2026-11-24) The server keeps a 32-byte hash chain of the accepted command descriptors per code run and
  refuses a completion whose replayed chain differs (`CHAIN_MISMATCH`). ADR-0104.
- **D044-006** (2026-11-24) Hold instead of retry-forever: a nondeterminism, a chain mismatch and a command outside the
  envelope hold at once; other failures and timeouts are retried after 1, 2, 4 and 8 s and the fifth consecutive one
  holds. delegate (OD-2026-11-20-6): a hold is visible, alerted and keeps the run's state; Temporal's endless retry
  burns workers and hides the problem; five tries over at least 15 s absorb a worker restart.
- **D044-007** (2026-11-24) A code run pins one version and its digest; a move re-pins one run only after a replay of
  its history on a worker of the target digest matched, its journal has not grown since and its outstanding steps fit
  the target's envelope. ADR-0105.
- **D044-008** (2026-11-24) `patched(id)` and `deprecatePatch(id)` with Temporal's semantics, recorded as `patch`
  markers.
- **D044-009** (2026-11-24) The code digest is the SHA-256 of the canonical JSON of language, SDK name and version,
  workflow type and the content hash of Technical Plan 3.2 (bundle, module sources, executable, assembly, jars).
  delegate (OD-2026-11-20-6), Owner question 044-002.
- **D044-010** (2026-11-24) Time is the database time of `workflow_task.started`; the generator is xoshiro256** seeded
  by a 32-byte per-run seed; UUIDs are version 4 from the generator; the same algorithm in six SDKs. ADR-0106.
- **D044-011** (2026-11-24) The runtime per language of Technical Plan 1. ADR-0106.
- **D044-012** (2026-11-24) The TypeScript SDK bundles workflow code with esbuild, pinned and hash-locked, as a
  build-time dependency of the `sagat-workflows bundle` command, never loaded by a worker at run time. delegate
  (OD-2026-11-20-6): the bundle makes the digest the code that runs, and the bundler is where imports are refused;
  writing a module loader for ECMAScript modules in a `vm` context needs `--experimental-vm-modules`. Its supply-chain
  review is Gap 044-012.
- **D044-013** (2026-11-24) The Python SDK refuses to register workflow types unless `sys.flags.hash_randomization` is
  0. delegate (OD-2026-11-20-6), Owner question 044-004.
- **D044-014** (2026-11-24) The Java code-workflow runtime is a new artifact `sagat-workflows` compiled for release 21
  on virtual threads; the core SDK stays on 17. delegate (OD-2026-11-20-6), Owner question 044-005.
- **D044-015** (2026-11-24) The activation deadline is 2 s by default (100 ms to the workflow task timeout).
- **D044-016** (2026-11-24) Signals to code runs are journaled at any non-terminal state for declared names; graph runs
  keep refusing a signal nobody waits for. delegate (OD-2026-11-20-6), Owner question 044-006.
- **D044-017** (2026-11-24) Updates of code runs are asynchronous: admitted, validated and accepted or rejected in an
  activation, handled by code that may issue commands, completed later; the REST call waits up to `waitMs` (at most
  60,000 ms) and answers `200` or `202`. Graph runs keep the synchronous CEL updates.
- **D044-018** (2026-11-24) Queries of code runs: declared CEL queries over the declared state, answered by the engine
  (default), and worker queries over ephemeral `code_query_requests` rows, never journaled. ADR-0108.
- **D044-019** (2026-11-24) A saga opened from code is an engine compensation domain; an abort compensates the scope's
  completed steps in reverse completion order, one at a time. ADR-0107.
- **D044-020** (2026-11-24) No local activities; eager activity dispatch in the completion transaction and an eager
  workflow task on a sticky session's activity completion. ADR-0107, Owner question 044-007.
- **D044-021** (2026-11-24) Sticky: `sticky_until` is the time the step becomes `READY` plus 5,000 ms; default cache
  256 runs per worker; `ReleaseSticky` on eviction.
- **D044-022** (2026-11-24) History limits: from 10,240 visible events the function is told continue-as-new is
  suggested; at 51,200 visible events a non-ending completion is refused `HISTORY_LIMIT` and the run held. Pages of at
  most 1,024 events or 4 MiB. Reason: 51,200 events replay in 5 s at the budget of 97.65625 µs per event, half the
  default workflow task timeout of 10 s.
- **D044-023** (2026-11-24) A completion carries at most 256 commands and 4 MiB of inline command payload and runs at
  most 12 statements.
- **D044-024** (2026-11-24) The protocol change takes the next free minor of the register when its contract merges
  (expected 1.9). ADR-0109.
- **D044-025** (2026-11-24) The code version document: the top-level `code` object replaces `entrypoint` and `nodes`
  through a `oneOf`; the capability envelope (activities, secrets, children, scripts, approvals) is checked per command.
- **D044-026** (2026-11-24) Publication of code versions is gated on the instance capability `code-workflows` (ADR-0082).
- **D044-027** (2026-11-24) Continue-as-new from code: `version: same` (default) or `active`. delegate
  (OD-2026-11-20-6), Owner question 044-008: a new run pinned to the active version is not a migration of a running run,
  and it is the usual way a long-lived code workflow takes new code.
- **D044-028** (2026-11-24) A binding covers a workflow type through an entry `workflow:<type>` in the binding's list;
  registration refuses an uncovered type with `NOT_BOUND`; the binding is re-checked on every completion.
- **D044-029** (2026-11-24) A workflow task holds no secret; `ResolveSecrets`, `ResolveDatabase` and `AppendStepLogs`
  refuse a `WORKFLOW` lease.
- **D044-030** (2026-11-24) The history a worker receives is a filtered, typed projection (`HistoryEvent`), not the
  journal's JSON, so the journal's detail can change without a protocol change.
- **D044-031** (2026-11-24) `GET /runs/{runId}/history` under `run:read` and the new `history:export`, audited, with
  payloads as stored.
- **D044-032** (2026-11-24) Mixing graph and code is by child runs in both directions; inline graph fragments in a code
  run are not in 0.3.0 (Gap 044-011).
- **D044-033** (2026-11-24) An uncaught exception that is not the SDK's workflow failure type fails the workflow task
  (and holds after five), never the run. delegate (OD-2026-11-20-6), Owner question 044-009.
- **D044-034** (2026-11-24) Cancellation is cooperative, with a grace (default 60 s, 1 s to 3,600 s) after which the
  engine ends the run `CANCELLED` itself.
- **D044-035** (2026-11-24) Feature 019 routes activities only; workflow tasks are routed by the code digest of the run's
  version, which is itself the pin feature 019 adds for activities.
- **D044-036** (2026-11-24) Feature 022: run-level gates at admission as for any run; step-level keys on activity
  commands at readiness; workflow tasks and query tasks are never gated.
- **D044-037** (2026-11-24) Feature 031: the server reads no code-run payload except declared state, update arguments the
  engine reads, search attributes and `waitEvent` correlation inputs, which stay plaintext.
- **D044-038** (2026-11-24) The SDK records the behaviour flags it used for the first time in a run in the workflow task
  completion, and uses a flag on replay only if the history recorded it.
- **D044-039** (2026-11-24) Promotion may require a replay check of up to 64 of the most recent completed runs of the
  active version.
- **D044-040** (2026-11-24) The Studio draws a code run's observed graph from `code_nodes` with the IR node pictures,
  marked `Code-orchestrated`.
- **D044-041** (2026-11-24) The determinism spikes, run once each on the development machine (macOS on Apple silicon;
  Node 26.13.1, Python 3.15.2, Go 1.24.2, .NET 10.0.401, OpenJDK 23.0.2, rustc 1.100.0), not committed:
  - TypeScript: a `vm` context with `microtaskMode: 'afterEvaluate'`, replaced `Date` and `Math.random`, and one empty
    script evaluation after each resolved event, gave the same five commands and the same result in two runs;
    `Promise.race` followed journal order; a context cost 0.19 ms to 0.45 ms to create; `codeGeneration.strings: false`
    refused `eval` with `EvalError`; `this.constructor.constructor("return typeof process")()` returned `object` from an
    empty context and through a host function passed in.
  - Python: a loop subclassing `asyncio.AbstractEventLoop` turned `asyncio.sleep(30)` and `asyncio.wait_for(timeout=5)`
    into timers of 30 s and 5 s, deterministically over two runs; a set of six strings printed four orders under
    `PYTHONHASHSEED` 1 to 4 and one order under 0 in four processes; an audit hook refused `open` and a meta-path finder
    refused `import socket` and `import secrets` during an activation only; `time.time()` is not audited.
  - Go: a coroutine round trip through `iter.Pull` cost 45 ns, a goroutine and two unbuffered channels 241 ns; 50 ranges
    over a map of 8 keys gave 8 orders; `select` over two ready channels chose the first 59 times in 100.
  - C#: a single-threaded `TaskScheduler` kept the SDK's awaits and `Task.WhenAll` on itself; `ConfigureAwait(false)` and
    `Task.Run` left it.
  - Java: a handoff between two virtual threads cost 4,460 ns, between two platform threads 8,869 ns; a parked virtual
    thread about 1,845 bytes of heap; the virtual-thread builder with a custom executor refused reflective access with
    `InaccessibleObjectException`.
  - Rust: a single-threaded executor of 70 lines gave the same commands and result in two runs; 50 `HashMap`s of the
    same 8 keys built in one process gave 49 iteration orders.

### Decisions after the first review round (2026-11-24)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-24/round-1/`; `resolution.md` there says how each
finding was answered. These entries supersede the earlier ones they name; the earlier ones stay as written.

- **D044-042** (2026-11-24, supersedes the pin of D044-007) A move changes `runs.code_version_id`, a new column; the start
  version `runs.workflow_version_id` never changes, because step identifiers, journal instance identifiers and
  compensation identifiers are derived from it. Only the `SECURITY DEFINER` function `sagat_move_code_run` may change the
  new column; `sagat_app` has no update privilege on it. A move waits while the `~workflow` step is leased, compares the
  visible count (not `run_version`, which counts invisible events), and ends `RUN_BUSY` after three re-checks. Every
  completion is refused as stale when the session's digest is not the run's current digest.
- **D044-043** (2026-11-24, supersedes D044-004 and D044-005) The command chain covers the command number, kind, target
  and `HMAC-SHA-256(seed, canonical fragment)` of every command, and the history sent to workers carries no fragment, so a
  worker can produce the chain only by producing the same commands, inputs included. `strict` matching is removed: the
  server's chain is strict by construction, and the SDK keeps the kind-and-target comparison for its report.
- **D044-044** (2026-11-24, supersedes D044-006) A run that needs a person goes under the existing step review of feature
  003 (`step.needs_review` on `~workflow`, the review retry), not a new hold state, route and events. The causes and their
  behaviour are one table (design, Technical Plan 6.3); `NONDETERMINISM_RISK` and `VALIDATOR_COMMAND` go under review at
  once, because retrying the same code cannot help.
- **D044-045** (2026-11-24) Commands carry fragments of a *code profile* of each node kind: expression and template
  fields become literals (`correlationKey`, child `input`) or are refused (`idempotencyTemplate`); publisher-only fields
  are refused (`outputSchema`, `destinations`, an inline approval policy, `onError`) and come from the version (the
  envelope's per-activity output schema, named approval policies). Delegated (OD-2026-11-20-6): run-time code must not
  choose classifications, egress or the four-eyes rule of an approval.
- **D044-046** (2026-11-24, supersedes the envelope of D044-025) The envelope's activities are entries with their own
  secrets and output schema; the `script.` and `sagat.` prefixes are refused; scripts are listed by digest and fill
  `workflow_version_scripts`; the accepted fragment is stored in `payloads` and its digest in the step, so
  `ResolveSecrets` reads the declared secrets of a `c-<n>` step from an immutable record.
- **D044-047** (2026-11-24, supersedes the claim of D044-002 and the table of the first draft) The `~workflow` step's
  activity field holds `sagat.workflow:<type>:<digest>` (at most 128 characters, so a workflow type has at most 48), and
  the existing claim, index, hints and reconciler route it unchanged; bindings cover a workflow type only by the exact
  entry `workflow:<type>`; an activity registration or binding that names `sagat.workflow` is refused. `code_nodes` and
  `worker_session_workflow_types` are dropped.
- **D044-048** (2026-11-24, supersedes the route of D044-007) Moves of many runs are a batch action `MOVE_CODE_VERSION` of
  feature 018; a single run has `POST /runs/{runId}/code-version`. Permissions: `run:operate` and `version:activate`, and
  `run:batch` for many; the target must be approved for activation where the namespace requires it. Replay-check tasks
  read each run's history by pages under a per-run read capability.
- **D044-049** (2026-11-24, supersedes D044-039) No promotion replay check: under pinning a promotion changes no open
  run; a dry-run move (Requirement 044-026) and the replay of exported histories answer the question.
- **D044-050** (2026-11-24, supersedes D044-031) The history export applies feature 016's redaction and reveal rules to
  every payload, never exports a `secret` field, and is granted to the admin and developer roles; a replay that reads a
  placeholder stops `INCONCLUSIVE_REDACTED`.
- **D044-051** (2026-11-24, supersedes D044-013) Python workflow workers run with a fixed `PYTHONHASHSEED` chosen by the
  operator (any value but `random`), and the digest includes the `hash()` of a fixed string under that seed. Delegated
  (OD-2026-11-20-6), Owner question 044-013.
- **D044-052** (2026-11-24) The code digest includes the runtime's name and major and minor version and the dependency
  closure (Technical Plan 3.2); each runtime gives workflow code UTC and the root locale; Python workflow code needs 3.11
  and TypeScript workflow code Node 22.8 (`vm.constants.DONT_CONTEXTIFY`).
- **D044-053** (2026-11-24) The TypeScript bridge places no host object in the context: the host calls in with string
  arguments and reads the commands back as a JSON string. Spike of 2026-11-24 (appended to D044-041): with
  `DONT_CONTEXTIFY`, `this.constructor.constructor("return typeof process")()` returned `undefined`; an ordinary
  contextified object returned `object` even with code generation from strings refused.
- **D044-054** (2026-11-24) A reset of a code run keeps its source's seed; to the same code version it starts at once, to
  another it is replay-checked and answered `202`.
- **D044-055** (2026-11-24, supersedes D044-023) An ordinary completion runs at most 10 statements (7 always, at most 3
  for saga rows, declared state and update receipts); a terminal or continue-as-new completion adds exactly the existing
  terminal or handoff path's statements. The batched persistence path is new code with its own crash tests.
- **D044-056** (2026-11-24) A terminal command is refused with `OUTSTANDING_OBLIGATIONS` while a saga is open or aborting
  or a step is under review or has an unknown outcome; other outstanding work is cancelled by its policies in the same
  transaction. After 3 consecutive `UNHANDLED_EVENTS` refusals the run refuses new signals and updates until its next
  completion commits.
- **D044-057** (2026-11-24, supersedes D044-022 in part) The continue-as-new suggestion fires at 10,240 visible events or
  3 MiB of retained bytes; at 51,200 the run goes under review `HISTORY_LIMIT`, whose retry opens a closing window of
  1,024 visible events in which only commands that end, continue or wind down the run are accepted; external events are
  refused from 50,176 (51,200 − 1,024) so that the window always has room.
- **D044-058** (2026-11-24) Bounds: 1,024 signals per activation window per run, 64 updates awaiting acceptance per run,
  2,048 outstanding steps, timers, waits and children per run; per namespace 64 worker queries, 256 update waits and 16
  replay-check tasks in flight. Update waits re-read the receipt after 50, 100, 200, 400 and 800 ms and then every
  1,000 ms, at most 64 reads in 60 s.
- **D044-059** (2026-11-24, supersedes D044-021) The sticky preference lasts 1,000 ms by default (at most 5,000 ms) from
  the moment the step becomes `READY`; `ReleaseSticky` applies only to the caller's own sticky steps.
- **D044-060** (2026-11-24, supersedes D044-020 in part) Eager dispatch is kept in 0.3.0 but built after the non-eager
  path, as a deferrable task; an idempotent replay returns no lease and the idempotency record stores no token.
- **D044-061** (2026-11-24) A code saga's plan is computed at the abort, in reverse completion order, and stored in
  run-scoped plan rows; its compensation identifiers are derived from the start version and the forward node. Task
  044-22 chooses between the existing run saga rows and a new table and records the reason here.
- **D044-062** (2026-11-24) The first publication of a code version activates the instance registry's requirement
  `code-workflows` (ADR-0082), so that a process that registers later without the capability closes its gate.
- **D044-063** (2026-11-24) An abandoned activation run is counted; at 4 a worker withdraws its workflow types and reports
  itself unhealthy. Rust runs activation runs on a pool of threads without a Tokio runtime (one per core, at most 8) and
  replaces a stuck one.
- **D044-064** (2026-11-24) A namespace can pause the workflow tasks of all its code runs, audited.
- **D044-065** (2026-11-24, supersedes D044-038) Behaviour flags: a reserved protocol field only in 0.3.0 (Gap 044-015).
- **D044-066** (2026-11-24) The search-attribute command and event are not in 0.3.0 until feature 020 lands; Task 044-27
  adds them.
- **D044-067** (2026-11-24) Worker queries and code updates declare `disclosure` and a closed `resultSchema`, validated by
  the platform's validator of design 043 before an answer is returned (ADR-0031). A receipt is readable by its admitting
  principal or a principal with `run:operate`. Worker queries stay in 0.3.0 as a deferrable task, Owner question 044-014.
- **D044-068** (2026-11-24) The failure detail of a workflow task is a closed cause, a command number, the actual kind and
  target checked against the grammar of the kind, and an exception type name matching
  `^[A-Za-z_][A-Za-z0-9_.$]{0,127}$`; the expected command comes from the server's record; no message is stored.

### Decisions after the second review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-2/`; `resolution.md` there says how each
finding was answered. These entries supersede the earlier ones they name.

- **D044-069** (2026-11-25) Supersedes D044-003 in its wording: the activation window is `(S(k − 1), S(k)]` over journal
  sequences, `S(k)` the start of the task that produced completion k; D044-003's pair of events is how `S` is found.
- **D044-070** (2026-11-25) A per-run node overlay (`code_step_nodes`: the node spec of each accepted work command, at most
  4 KiB, and its derived companions) is how the engine finds the node of a `c-<n>` step; a code version's parsed
  definition is a code definition with no graph. Every `node_for_instance` call site is audited in Task 044-4.
- **D044-071** (2026-11-25) The workflow-task step's node identifier is a reserved form recognised by
  `NodeId::is_workflow_task_step`, tested before `is_derived` and `forward`, and accepted by `parse_stored`.
- **D044-072** (2026-11-25, supersedes the flag of D044-042 and of round 1) Pending is derived: a workflow task is due
  when `code_last_waking_seq` is greater than `code_last_completed_start_seq`; the journal append of a code run moves the
  first for every waking event (`RunEvent::is_waking`, exhaustive), a completion moves the second; a failed task moves
  neither; the reconciler repairs a pending run whose step waits.
- **D044-073** (2026-11-25, supersedes D044-016 and the signal bounds of D044-058) Signals of a code version use the
  inbox of Architecture decision 0111 (every declared signal buffered); the activation consumes them and its completion
  marks them consumed. Delegated (OD-2026-11-20-6): one buffered-signal mechanism for graph and code runs, rather than
  two; Gap 044-017 tracks the refinement Architecture decision 0111 needs for code versions.
- **D044-074** (2026-11-25, supersedes D044-026 and D044-062) `code-workflows` is recorded by an operator (Architecture
  decision 0082), never by a tenant's publication; publication is refused until it is recorded.
- **D044-075** (2026-11-25) A move is always a person's explicit request; a batch move freezes its selection at creation
  so that each item is a per-run request; no pipeline or policy moves runs (constitution 5.0.0, principle IV). The current
  code version lives in `code_run_pins`, written only by `sagat_move_code_run`; the start version is the run's identity
  origin. Supersedes the `runs.code_version_id` column of D044-042.
- **D044-076** (2026-11-25) The replay check replays activations 1 to k (the last completed) and the move compares
  `code_last_completed_start_seq` with the replayed `S(k)`, not a visible count; a move clears the sticky fields and makes
  the step `READY` with a hint when it was under review or a task is pending. Supersedes the visible-count comparison of
  D044-042.
- **D044-077** (2026-11-25, supersedes D044-055) A completion runs at most today's activity-completion statements plus 4,
  reads included; the batched path is held to `persist` by a differential test.
- **D044-078** (2026-11-25, supersedes the cancellation of outstanding work in D044-056) A terminal command goes through
  the engine's stop drain: `run.finished` is written only when no leased work is left or what is left went to review.
  Continue-as-new waits for quiescence through feature 014's handoff and the function gets no further activation.
- **D044-079** (2026-11-25) A completion carries `applied_through_seq`; a terminal command waits for the whole window in
  the SDK; a completion below `S(k)` is refused `UNHANDLED_EVENTS`, journaled `workflow_task.failed`, and the SDK replays.
- **D044-080** (2026-11-25, supersedes the closing window of D044-057) The closing window is 1,024 visible events plus two
  per outstanding step, timer, wait and child at the retry; exhausted, the run fails `HISTORY_LIMIT` through the stop
  drain. The largest history is 56,320 visible events, replayed in 5.5 s at the replay budget.
- **D044-081** (2026-11-25, supersedes D044-045 and D044-046 in part) Risk metadata (`irreversible`, `highRisk`,
  `compensateWith`, `reconcileWith`) belongs to the envelope entry; a fragment may repeat or omit it, never change it;
  derived steps receive the secrets of their own entry; activity inputs are validated and classified by the entry's
  `inputSchema`.
- **D044-082** (2026-11-25, supersedes the keyed digests of D044-043) The chain uses the SHA-256 of the fragment bytes as
  received (after a canonical check); the claim that keying protected readers is withdrawn. The hashing is behind a domain
  port implemented in `sagat-application`.
- **D044-083** (2026-11-25, supersedes D044-061) A code saga's plan lives in `code_saga_plan_entries`; step B depends on
  step A when B's command was issued in an activation after the one that delivered A's outcome; independent
  compensations stay eligible together. Architecture decision 0107 supersedes Architecture decision 0010 decision 3 for
  code runs.
- **D044-084** (2026-11-25) Code updates follow `ADMITTED`, `ACCEPTED` or `REJECTED`, `COMPLETED` or `FAILED`;
  Architecture decision 0108 supersedes Architecture decision 0031 decisions 8 and 9 for code versions; a result outside
  its schema fails the update only (`RESULT_SCHEMA_VIOLATION`).
- **D044-085** (2026-11-25, supersedes D044-054) A reset of a code run is to its current code version only, in one
  transaction, within Architecture decision 0050's prefix ceiling; an eligible point is the end of a completion block with
  no attempt in flight, no unknown outcome and no compensating saga; `READY` steps, timers and waits are rematerialized;
  each new event kind has its reset rule. Architecture decision 0105 refines Architecture decision 0050 decisions 5 and 8
  for code runs.
- **D044-086** (2026-11-25, supersedes D044-050 and D044-067 in part) `history:export` is held by the admin role only by
  default. Delegated (OD-2026-11-24-4), Owner question 044-015.
- **D044-087** (2026-11-25) Worker queries and replay checks are durable rows claimed through `PollTasks`, re-announced
  by the reconciler; replay checks live in `code_replay_checks`.
- **D044-088** (2026-11-25) Bindings gain `workflowTypes`; activity bindings never cover a workflow type.
- **D044-089** (2026-11-25, supersedes D044-059) While sticky, the workflow-task step's activity is
  `sagat.workflow-sticky:<session id>`, so the claim's SQL is unchanged; the scheduler's timer pass rewrites it to the
  digest name and writes the hint when `sticky_until` passes or the session ends.
- **D044-090** (2026-11-25, supersedes D044-060 in part) Eager activity leases are provisional leases of feature 006,
  released safely when unacknowledged; the eager workflow task checks the session's digest and binding.
- **D044-091** (2026-11-25) The protocol minor a step needs is decided per step kind, so old-minor activity workers still
  lease a code run's activities.
- **D044-092** (2026-11-25) A code run starts at most one workflow task per 100 ms (the step's earliest next start).
- **D044-093** (2026-11-25) Reading history extends the workflow task lease, up to 120,000 ms in all.
- **D044-094** (2026-11-25) The time of `workflow_task.started` is the later of the previous start's time and
  `clock_timestamp()`; the idempotency key is looked up before any fence and any bound.
- **D044-095** (2026-11-25) The runtime string of the digest carries the full version and the locale-data version; Python
  hashes extension binaries; SDKs declare whether they offer workflow as code, and the suite reports "not offered".
- **D044-096** (2026-11-25) A command on one of the run's own steps that is already terminal is an audited no-op.
- **D044-097** (2026-11-25) The activity code profile admits design 045's timeout and retry fields when design 045 lands.
- Supersession notes for the first register: D044-040 (`code_nodes`) is superseded by D044-047 and D044-070; D044-026 by
  D044-074; D044-062 by D044-074.

### Decisions after the third review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-3/`; `resolution.md` there says how each
finding was answered. These entries supersede the earlier ones they name.

- **D044-098** (2026-11-25, supersedes D044-070) A code version parses to `DefinitionGraph::Code`; a run transaction
  composes it with the run's overlay, so `node`, `node_for_instance`, `entrypoint` and `scope_body` all resolve through
  it; a source-text check lists every call site; arms that let an unknown node through fail closed. The overlay is
  cached in process by run and command count.
- **D044-099** (2026-11-25, supersedes D044-071) The workflow-task step has `steps.kind = WORKFLOW_TASK` and
  `NodeKind::WorkflowTask`, built from the step row and the pin; `is_derived`, `forward` and `parse_stored` change
  themselves; the claim has a workflow-task arm that skips activity quotas, breakers, pools, capacity windows and the
  run-document input, and counts the task in the namespace bucket `sagat.workflow` (default 1,024); waiting kinds
  `DECIDER`, `CLOSED` and `APPROVAL`.
- **D044-100** (2026-11-25, supersedes D044-072 in part) Waking depends on the event and its origin; every journal write
  goes through `PgRunTx::append_events(events, origin)`, and the architecture check refuses raw inserts; only a step
  `WAITING` with kind `DECIDER` is woken; the reconciler reads the journal, not the counters.
- **D044-101** (2026-11-25, supersedes D044-096) A command on a terminal step is a no-op with its own command record
  (`command.noop`), counted and chained; its audit is one record per completion.
- **D044-102** (2026-11-25, supersedes D044-083) Step B depends on step A when B's command record has a greater journal
  sequence than A's outcome; code saga plans live in `run_sagas` and `run_saga_entries`; engine-driven stops apply each
  open scope's `cancelPolicy`, which the open-saga command carries.
- **D044-103** (2026-11-25, supersedes D044-082) The chain covers the SHA-256 of every command's canonical body, typed
  commands included; the canonical form is a new public RFC 8785 function of `infra-ir` beside the unchanged
  `canonical_json`; `CommandDigester` is an application port implemented in `infra-ir`.
- **D044-104** (2026-11-25, supersedes D044-073 in part) Signals take Architecture decision 0111's shape; publication
  also needs `signal-inbox-v1`; a signal is consumed only when the function's handler or wait takes it, reported by the
  completion; delivered but unhandled signals stay pending and move or are discarded as Architecture decision 0111
  decides. Task 044-21 builds on that design's inbox task, or builds the inbox to its decisions if it has not landed.
- **D044-105** (2026-11-25, supersedes D044-084 in part) Code update lifecycles are rows of the append-only
  `code_update_transitions`; graph receipts stay immutable; `acceptPending` opts in to `202`, otherwise `409
  UPDATE_PENDING` after 60 s; the receipts route is `/runs/{runId}/update-receipts/{updateKey}`; another principal's key
  gets `409 UPDATE_KEY_IN_USE` with no content; updates need `argsSchema`.
- **D044-106** (2026-11-25, supersedes D044-074 in part) The capability is `code-workflows-v1`, required of the control
  plane and the engine; removing the requirement is refused while a code version exists.
- **D044-107** (2026-11-25, supersedes D044-088) Bindings list workflow types in `worker_binding_workflow_types`; the
  four binding checks of today treat reserved names against it; replacing a binding closes sessions of removed types.
- **D044-108** (2026-11-25) The envelope: an exact entry wins over a prefix and a longer prefix over a shorter; only an
  exact entry may list secrets; script entries are objects; `children` carry versions; `events` list event types with a
  largest look-back; `requiresApproval` makes the engine hold the step behind an approval of that step.
- **D044-109** (2026-11-25, supersedes D044-089 and D044-059) The sticky preference is the deferrable Task 044-14; the
  core lets any session of the digest take a workflow task, the lease carries its window start, and a worker reuses its
  cache only when it holds the run at that start.
- **D044-110** (2026-11-25, supersedes D044-078 and D044-079 in part) A terminal command refused for obligations or unseen
  events is not counted and is offered again; an accepted one closes the workflow-task step (`CLOSED`) and goes through
  the stop drain, with `StopReason::Completed` carrying the output reference for a success; completions during the drain
  are stale.
- **D044-111** (2026-11-25, supersedes the waiting continue-as-new of D044-078) Continue-as-new is refused while an
  activity, child, approval or saga is open, cancels open timers and waits in its transaction, and pins the current code
  version by default (`same`); Architecture decision 0105 supersedes Architecture decision 0030 decision 16 for code runs.
- **D044-112** (2026-11-25, supersedes D044-085 in part) A reset of a moved run starts on the source's current code
  version, with the prefix's overlay rows copied; `run.code_version_moved` is `EXCLUDE`; the registry of reset rules gains
  a column for code runs.
- **D044-113** (2026-11-25) The headroom refuses external signals and updates from a visible count of 50,176 and during
  the closing window; only the first cancellation request is journaled; no workflow task is offered once the window is
  used up, so the largest replayed history is 56,320 events.
- **D044-114** (2026-11-25, supersedes D044-092) The rule of one workflow task per 100 ms is withdrawn (steps have no
  not-before column and the claim must not change); the namespace bucket `sagat.workflow` bounds workflow tasks in
  flight.
- **D044-115** (2026-11-25, supersedes D044-077) The completion budget is at most 57 statements: the 50 of
  `COMPLETION_WITH_A_NEXT_STEP` plus at most 7 listed in Non-functional requirement 044-003.
- **D044-116** (2026-11-25, supersedes D044-086 in part) Business literals of fragments (`input`, `correlationKey`,
  `subject`, `businessId`) are classified payloads; activity inputs are classified by the entry's `inputSchema`; a lease
  input of a `c-<n>` step is its literal input; a derived step's input is the forward input and output.
- **D044-117** (2026-11-25) All moves are batch jobs (a single-run move is a one-item job); the creator is re-authorized
  per item; verdicts and retries live on `batch_items`; `code_replay_checks` holds only the lease; an unclaimed item ends
  `NO_TARGET_WORKER` after 3,600 s; `REPIN` is refused for code runs; the move function checks the replay check and the
  appended event.
- **D044-118** (2026-11-25) Worker queries answer from the function's state at its last committed completion, with that
  completion's journal sequence; their arguments and answers are classified payloads; Architecture decision 0108
  supersedes Architecture decision 0031 decision 7 for them.
- **D044-119** (2026-11-25) The history export has a format version and a baseline; a replayer refuses an unknown format.
- **D044-120** (2026-11-25) All retained run content of a code run (inputs, literals, side effects, update arguments and
  results, declared state) is charged to the 4 MiB and the tenant's content quota.
- **D044-121** (2026-11-25) Python workflow modules run in a private module namespace whose `__import__` is the SDK's, so
  preloaded modules are refused too; at most 10,000 keys per decoded object; TypeScript, Python and C# require a UTC
  process time zone.
- **D044-122** (2026-11-25) The SDK surface and packaging of Technical Plan 1.12; `publishCode` saves the draft and uses
  today's publication route, which activates.
- **D044-123** (2026-11-25) A session on the new minor implements the host procedures of 1.7, 1.8 and 1.9; design 045's
  `steps.min_protocol_minor` decides the minor per step (added here if design 045 has not landed).
- Supersession notes: D044-019 is superseded by D044-083 and then D044-102; D044-028 and D044-047 by D044-088 and then
  D044-107; D044-002's "expiry without review" stands, and its expiry counts as a failure (Technical Plan 6.3).

### Decisions after the fourth review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-4/`; `resolution.md` there says how each
finding was answered.

- **D044-124** (2026-11-25) The journal alone rebuilds a code run: `run_events.origin`; command records carry the node
  spec (at most 1 KiB) and the body digest; `run.started` carries the seed; `signal.consumed` records consumption;
  deferrals are recorded in `workflow_task.completed`. The overlay is a projection of the records; these fields are
  server-side and never returned on reads.
- **D044-125** (2026-11-25, supersedes D044-110 in part) A terminal or continue-as-new command that must wait is deferred:
  the other commands commit, `S` moves, the command takes no number, and the SDK re-emits it last at each later
  activation; a completion below the window is an SDK fault (`NONDETERMINISM_RISK`).
- **D044-126** (2026-11-25) The code definition answers every `WorkflowDefinition` method, saga and scope accessors
  included; scopes `s-<n>` are `NodeKind::Saga` nodes; the planner's automatic abort and completion of graph sagas do not
  apply to code scopes; `policies` fails closed; the call-site check reads the compiler's resolution.
- **D044-127** (2026-11-25, supersedes D044-102's dependency rule) B depends on A when A's outcome is at or before the
  start of the workflow task whose completion recorded B.
- **D044-128** (2026-11-25) Python's private namespace has three classes: shared vetted modules, restricted views,
  refused modules; the audit hook acts only while the activation's context variable is set; `PYTHONHASHSEED=0` is
  refused.
- **D044-129** (2026-11-25) The reconciler checks each wait once (`code_wait_checked_at`), reads the journal backwards by
  its primary key and the origin, at most 256 runs per pass, with a parity test of its waking kinds against the domain.
- **D044-130** (2026-11-25, supersedes the cancellation of D044-034) Cancelling a code run journals
  `run.cancel_requested` and sets the deadline without stopping the run; a scheduler pass stops it at the deadline;
  steps, cleanup activities included, keep being leased in the grace.
- **D044-131** (2026-11-25) Graph publication refuses `sagat.` activity names (except feature 017's managed names); a
  reserved name on a step that is not `WORKFLOW_TASK` is refused at claim; a scan precedes recording the requirement.
- **D044-132** (2026-11-25) A lease-kind matrix per RPC; `workflowTypes` only on ordinary bindings of the worker role.
- **D044-133** (2026-11-25) Eager leases go through the claim's admission function; the eager workflow task counts in
  the `sagat.workflow` bucket.
- **D044-134** (2026-11-25) The initiators of an engine-held approval are the run's starter and every principal whose
  update or signal was delivered up to the issuing activation.
- **D044-135** (2026-11-25, supersedes D044-105 in part) Update keys are scoped by run and admitting principal; an
  unknown key answers `404`; `UPDATE_KEY_IN_USE` is withdrawn.
- **D044-136** (2026-11-25, supersedes D044-087 in part) Worker-query arguments (at most 64 KiB) and answers (at most
  256 KiB) are inline in their row, charged to the content quota while it lives, deleted with it.
- **D044-137** (2026-11-25) Feature 016 gains, under a new version of its classification rules, the carriers of a code
  version's envelope; every other code-run payload is `pii` as a whole (Architecture decision 0108).
- **D044-138** (2026-11-25, supersedes D044-085 and D044-112 in part) A code-run reset point also needs no open,
  aborting or compensating saga scope and no open approval or child; the reset run's overlay is rebuilt from the
  prefix's records.
- **D044-139** (2026-11-25) The publication holds each fleet requirement `FOR SHARE` through the registry's definer
  function (the `hold_requirement` of Architecture decision 0110 decision 8, shared); `signal-inbox-v1` is needed only by
  a code version that declares signals.
- **D044-140** (2026-11-25) The envelope declares `timeoutLevel` for design 045's fields and owns feature 022's flow
  control per activity entry; fragments carry only literal keys.
- **D044-141** (2026-11-25, supersedes D044-115) The completion budget is at most 62 statements (50 + 12), tested with a
  completion that mixes every command kind; the commits are counted as the budget suite counts them (10 and 7 against a
  graph run's 4).
- **D044-142** (2026-11-25) A completion that would cross 51,200 visible events is refused and reviews at the count it
  had; the closing window counts its own workflow-task events and admits markers, set-state and update completions.
- **D044-143** (2026-11-25) The crash tests use a `test-support` feature, `SAGAT_TEST_CRASH_POINT`, an advisory-lock
  barrier, a script and `SAGAT_TEST_EXECUTION_ENGINE`; the server lane uses a scripted decider in `tests/support/`.
- **D044-144** (2026-11-25) The typed append is a free function `append_events(conn, run, rows, origin)` with
  `JournalRow`; graph statement counts are unchanged; the raw-insert check covers `crates/` and `apps/` only.
- **D044-145** (2026-11-25) The TypeScript workflow entry point checks Node 22.8 at load (the package's `engines` stays
  Node 20); the bundler is a separate package with the `sagat-workflows` binary and the pinned esbuild.
- **D044-146** (2026-11-25) A move request carries a `reason` (8 to 2,000 characters); the server computes a replay
  check's verdict; `code_replay_checks.first_difference`; only the move item's apply path calls the move command.
- **D044-147** (2026-11-25) A workflow type is unique in its namespace; a code run's signals never satisfy its waits, and
  an event type may not be a signal name; `version: active` must be a code version of the same type.

### Decisions after the fifth review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-5/`; `resolution.md` there says how each
finding was answered.

- **D044-148** (2026-11-25) Every path that cancels a run (the route, a chain-addressed cancel, a parent's `propagate`
  policy, the source cancel of a pending reset) goes through `execution::request_cancellation`, which takes the code arm
  for a code run; a source-text check holds that `stop_run` with `Cancelled` has no other caller but the deadline pass.
  The deadline is cleared when a terminal command is accepted or a stop begins; a deferred terminal command is not an
  answer at the deadline; a reset re-arms the deadline from the grace that `run.cancel_requested` carries.
- **D044-149** (2026-11-25) The stop drain and the handoff finish every update that has not completed (`REJECTED` or
  `FAILED`, reason `RUN_ENDED` or `RUN_CONTINUED`); an update to a stopping or ended run is `409 RUN_NOT_MUTABLE`; a
  holder of `run:operate` reads every principal's update under a key as a list.
- **D044-150** (2026-11-25, supersedes D044-125 in part) A deferral record holds the deferred command's kind, target and
  body digest; a re-emitted command that differs is `CHAIN_MISMATCH`; a replaying SDK expects it last; the replay check
  answers its body digest. A terminal or continue-as-new command while a scope is open and not aborting is
  `COMMAND_INVALID`; an aborting scope defers it. A completion below `S(k)` reviews at once as `NONDETERMINISM_RISK`.
- **D044-151** (2026-11-25, supersedes D044-126's mechanism) The call-site check counts the calls of every method name
  of `WorkflowDefinition` in production code against a checked-in allowlist of `(file, method, count)`, with no new
  dependency; rustdoc JSON lists no call sites and needs a nightly toolchain.
- **D044-152** (2026-11-25, supersedes D044-142 in part) The closing window also admits signal consumption and update
  acceptance, rejection and completion.
- **D044-153** (2026-11-25) The initiators of every engine-held approval, an activity's or an approval command's, are the
  starter, the actors of external events delivered up to the issuing activation, and the actors of the run's moves and
  review retries before it.
- **D044-154** (2026-11-25, withdraws the opaque reference of D044-137's text) A `secret` field reaches a workflow lease
  as the typed placeholder; a code run cannot route a secret through the function. Architecture decision 0108 refines
  Architecture decision 0033 for schema-less code payloads (as Decision 302 did for events) and extends Architecture
  decision 0036 decision 6's carriers.
- **D044-155** (2026-11-25) Every code version of a workflow keeps the workflow type of its first one, so a move and
  `version: active` never change the type; a command names only a blob referenced by its run's history or written under
  its workflow lease; query and replay-check tokens may read those blobs; every name with the prefix `sagat.workflow` is
  refused at registration; a sticky name serves only its session.
- **D044-156** (2026-11-25) `append_events(conn, run, rows)` carries the origin on each `JournalRow`; the reconciler binds
  `RunEvent::waking_kinds()` with a parity test, reads `steps.code_wait_started_at` through a partial index; `verify_run`
  compares the overlay and the cancellation deadline.
- **D044-157** (2026-11-25) `sagat_move_code_run` checks the pin's integrity, a matching replay check at the current
  `S(k)` and the appended move event; `platform_requirement_hold(p_capability, p_kinds)` is specified here in full.
- **D044-158** (2026-11-25) A reset rebuilds the inbox from the prefix's `signal.buffered` and `signal.consumed`. Python's
  module set is closed at load. Eager leases are outside the completion's 62 statements and get their own budget entry
  in Task 044-14. `code_query_requests.args` and `answer` are class `content`.
- **D044-100** (2026-11-25, later state) Its `PgRunTx::append_events(events, origin)` is superseded by D044-144 and
  D044-156: a free function with the origin on each row.
- **D044-104** (2026-11-25, later state) Its sentence that Task 044-21 builds the inbox itself is superseded by the later
  state of G044-017: Task 044-21 waits for Architecture decision 0111's inbox.
- **D044-123** (2026-11-25, later state) Its "added here if design 045 has not landed" is superseded: design 045 owns
  `steps.min_protocol_minor`, and Task 044-8 depends on it (G044-020).

### Decisions after the sixth review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-6/`; `resolution.md` there says how each
finding was answered.

- **D044-159** (2026-11-25) `S(0) = 0`; `run.started` is visible and waking (origin `engine`) and carries the input, the
  predecessor and the carried state; the start transaction makes the first workflow task pending by the rule.
- **D044-160** (2026-11-25) `workflow_task.started` journals `continue_as_new_suggested` and `history_limit_reached`; the
  SDK answers both from the started event that opens the activation, never from the current lease.
- **D044-161** (2026-11-25) There is no terminate (Architecture decision 0050 decision 5): the remedy is cancellation,
  which stops a run under review at once; a continue-as-new accepted after `run.cancel_requested`, or a handoff that
  meets a chain or link latch, ends the run `CANCELLED`; a request to a run with a stop reason journals nothing; the
  cancellation deadline is judged by the database clock; cooperative cancellation is Architecture decision 0103
  decision 9.
- **D044-162** (2026-11-25) A move is refused for a run with a stop reason or a `CLOSED` workflow-task step
  (`RUN_NOT_MUTABLE`); `sagat_move_code_run` is called right after the move event, and `code_replay_checks` holds the run,
  `S(k)`, the computed chain and the verdict.
- **D044-163** (2026-11-25, supersedes D044-138 in part) A reset point needs no open code update; `run.code_version_moved`
  is `COPY`; a code run's `saga.aborting` is `COPY` once the scope settled; restored rows count for a code run's
  projections; update transitions are rebuilt; `reapply: UPDATES` is refused for code runs.
- **D044-164** (2026-11-25, supersedes D044-153 in part) Initiators are kept per run in `run_initiators`, copied by the
  handoff, a child start and a reset, and include resolvers, resetters and the publisher of each pinned code version; a
  code approval checks them against the bound of its issuing activation.
- **D044-165** (2026-11-25) Each code scope has a step row `s-<n>` waiting on the saga, so feature 003's saga machinery
  drives it and a stop never winds down its compensations.
- **D044-166** (2026-11-25) `RETAINED_BYTES_LIMIT` reviews a completion over the run's 4 MiB, with a closing window on
  retry; `STORAGE_QUOTA_EXCEEDED` from the tenant's quota is counted with backoff; a code completion never calls
  `stop_for_storage_quota`.
- **D044-167** (2026-11-25, supersedes D044-141 in part) The count of the steps that can still act is read by one
  statement over a partial index, never kept by hand; the completion budget is 63 statements (50 + 13); the append's
  wake statement is not issued inside a completion.
- **D044-168** (2026-11-25) A blob whose carrier schema has a `secret` field is never delivered to a workflow, query or
  replay-check token; the export emits blobs as references only; orphaned lease blobs are released; `ReadHistory`
  streams have caps and a byte budget; the completion and answer RPCs are of class `Lease`; a tenant holds at most 64 of
  a process's update waits.
- **D044-169** (2026-11-25) A deferral answers OK with `ACCEPTED_DEFERRED` and its cause; `UNHANDLED_EVENTS` and
  `OUTSTANDING_OBLIGATIONS` are not gRPC errors.
- **D044-170** (2026-11-25) A binding with workflow types only is stored with its profile in
  `worker_binding_workflow_types`; the binding store reads both tables; both lists empty is the unbind.
- **D044-171** (2026-11-25, supersedes D044-151 in part) The call-site check counts only the distinctive method names,
  holds the definition loaders for the generic ones, fails on a count that shrinks, and is hosted as a test of
  `apps/execution-engine` so that scoped CI runs it; the same host carries the other tree-wide source-text checks.
- **D044-172** (2026-11-25) `timeoutLevel` admits only `"none"` until design 045 lands; Task 044-27 widens it.
- **D044-173** (2026-11-25, supersedes D044-137 and D044-154 in part) An update's `argsSchema` and `resultSchema` and
  `state.schema` are carriers; only side-effect values and schema-less continue-as-new inputs are `pii` as a whole; raw
  reads follow the classification and declared query and update answers follow Architecture decision 0031.
- **D044-174** (2026-11-25) `CommandDigester` is a trait of the domain; the history projection is one application
  function parameterized by its reader; overlay rows record their accepting version and granted secrets; the pause does
  not count its own refusals or timeouts and worker queries answer `QUERY_UNAVAILABLE` on a paused run or one under
  review.

### Decisions after the seventh review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-7/`; `resolution.md` there says how each
finding was answered.

- **D044-175** (2026-11-25, supersedes D044-167 in part) The can-act count reads the overlay's `scope` against the run's
  sagas in their forward or aborting phase, not `run_saga_entries`; the completion budget is 62 statements (50 + 12),
  the saga plan rows leaving it because an abort only journals `saga.aborting` and feature 003 plans once every member
  has settled.
- **D044-176** (2026-11-25) The overlay row and the command record carry `issued_seq` and `accepted_version_id`;
  `plan_entries` takes a dependency source with a code arm; code scopes may nest, recorded by the open-saga command.
- **D044-177** (2026-11-25, supersedes D044-166 in part) `RETAINED_BYTES_LIMIT` has no retry and no window; the
  `HISTORY_LIMIT` window is one per run, judges completions against its own bound, ends the run in the crossing
  completion or through a scheduler pass when another writer uses it up; the claim raises the review at 51,200.
- **D044-178** (2026-11-25, supersedes D044-164 in part) Copied initiator rows bind at sequence 0; a child of any
  parent inherits the parent's starter, chain initiator and publisher; every approval of a run with rows checks them.
- **D044-179** (2026-11-25) Feature 022's literal keys from code are business literals classified `pii`; the spec, the
  journal and the gate rows carry only the keyed digest.
- **D044-180** (2026-11-25) Every request fits the worker service's unchanged 320 KiB decode limit (completions at most
  256 KiB of fragments and values, the SDK moving the largest to blobs; replay-check answers one item and 8,192 digests
  per message); responses stay below 4 MiB (pages of 3 MiB, first pages of 256 KiB, poll answers of 4 MiB).
- **D044-181** (2026-11-25) The overlay cache is bounded to 256 MiB per process, a tenant at most a quarter.
- **D044-182** (2026-11-25) The golden vectors come from a checked-in Node 22 generator with no new dependency, so they
  do not come from the Rust code they check.
- **D044-183** (2026-11-25, supersedes D044-171 in part) `DefinitionCache::get` becomes `get_version`; every caller of the
  cache is counted and states whether it composes the overlay; the expiry parity and the journal-insert rule are tests
  of `apps/execution-engine`, which scoped CI selects.
- **D044-184** (2026-11-25, supersedes D044-157 in part) `platform_requirement_hold` is design 045's function
  (`capability text`, `RETURNS boolean`, `lock_timeout`, `REQUIREMENT_BUSY`); a migration that finds it compares its
  definition and fails on a difference.
- **D044-185** (2026-11-25) The workflow-task step is not a `RuntimeNode` and a code run's reset point is
  `STEP_COMPLETED` at `~workflow`, so no closed REST enumeration gains a value; the run view gains `codeWorkflow`.
- **D044-186** (2026-11-25, supersedes D044-163 in part) For code runs `compensation.planned` and a scope's outcome
  events are `COPY` once it settled, and the reset rebuilds the scope's saga rows.
- **D044-187** (2026-11-25, supersedes the handoff of D044-161 and of Requirement 044-019's first text) An accepted
  continue-as-new only closes the step; feature 014's continuation pass performs the handoff in its lock order; chain and
  tree refusals end the run `FAILED`; input, target and missing `chainPolicy` refusals are `COMMAND_INVALID`; a
  detached child is no obligation.
- **D044-188** (2026-11-25) The pause stops workflow tasks and worker queries, not replay checks or moves; its refusals
  and timeouts are journaled as not counted; the flag has a column grant and is read `FOR SHARE`; worker queries to an
  ended run answer `QUERY_UNAVAILABLE`; marker identifiers have a pattern; `ReleaseSticky` takes at most 256 runs.

### Decisions after the eighth review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-8/`; `resolution.md` there says how each
finding was answered.

- **D044-189** (2026-11-25, supersedes D044-175 and D044-176 in part) A closed scope nested in an open parent stays
  compensable by the parent; the can-act set counts members by `root_scope` in the forward, aborting or compensating
  phase; `steps.outcome_seq` is stored; code scopes store linear `(issued_seq, outcome_seq)` pairs, never a quadratic
  `after` list; the SDK awaits members before closing and aborts every open scope before a terminal command.
- **D044-190** (2026-11-25, supersedes D044-178 in part) Initiator rows are written only in trees that contain a code run;
  a code approval also refuses the binder of the issuing session's binding; a workflow-type binding grants authorship and
  needs `definition:publish`.
- **D044-191** (2026-11-25, supersedes D044-180 in part) A completion's message holds at most 64 KiB of metadata and 192
  KiB of inline values; the declared state is inline and at most 128 KiB, once per completion; every other value may be
  a blob; poll answers at most 3.5 MiB.
- **D044-192** (2026-11-25) The node spec is the canonical code-profile fragment without literal values, merged with the
  entry's publisher fields under the IR's `apiVersion`, decoded only by `parse_code_node_spec` behind `IrCodec`.
- **D044-193** (2026-11-25, supersedes D044-184 in part) The shared hold function has no capability list in its body;
  the table's closed check bounds the value; a migration compares the rest; Architecture decision 0105 records the
  refinement of Architecture decision 0082 decision 1; `CAPABILITY_NOT_ACTIVE` is `422`.
- **D044-194** (2026-11-25) A workflow-task lease journals only `workflow_task.started`; feature 008's attempt projector
  gains arms for the four `workflow_task.*` kinds.
- **D044-195** (2026-11-25) Move items record `MATCHED`, `REPLAY_MISMATCH` or `ENVELOPE_MISMATCH` and a
  `firstDifference` on `BatchItem`; a move under a `HISTORY_LIMIT` review is refused; tokens are bound to their session
  and invalidated at the verdict; ranges arrive in order.
- **D044-196** (2026-11-25) A rerun of a code run starts on its current code version.
- **D044-197** (2026-11-25, supersedes D044-187 in part) The completion journals `run.continue_requested` with the
  resolved successor version, and the acknowledgement journals `run.stop_requested`; the continuation pass gains a code
  arm (boundary, quiescence, per-segment version, recorded input check, workflow-task entry, latch).
- **D044-198** (2026-11-25) A workflow failure keeps a closed code (`^[A-Z][A-Z0-9_]{0,63}$`) and stores its message
  only as a `pii` payload; the same for update failures.
- **D044-199** (2026-11-25, supersedes D044-181 in part) The overlay cache is keyed by the run alone and reads only new
  rows by command number; the completion feeds it.
- **D044-200** (2026-11-25) `DefinitionCache::get` becomes `get_parsed`; the Go and C# analyzers take
  `golang.org/x/tools` v0.28.0 and `Microsoft.CodeAnalysis.CSharp` 4.11.0, pinned and reviewed as esbuild is; the goldens
  test is a guard; `NodeMissing` maps to `INTERNAL` with `NODE_MISSING`; signals declare `maxBytes` (default 64 KiB).

### Decisions after the ninth review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-9/`; `resolution.md` there says how each
finding was answered.

- **D044-201** (2026-11-25) A blob-carried value enters a command body as its descriptor (SHA-256 and size of its RFC
  8785 bytes), never a blob identifier; the server checks the stored blob against it and validates blob-carried inputs
  and results like inline ones; a deferral holds its blob; exported histories stop at blob values.
- **D044-202** (2026-11-25) An activation makes at most 256 commands and 64 KiB of command metadata; the SDK throws
  `CommandBatchLimitExceeded` at the crossing call, live and in replay.
- **D044-203** (2026-11-25, supersedes D044-177 in part) The closing window is 1,024 + 3 × the steps that can still act,
  its bound capped at 58,368; the replay ceiling is 58,368 events (5.7 s); the claim leases only with room for a
  completion.
- **D044-204** (2026-11-25) The SDKs' test environments keep their local-time guards for activity workers and run
  code-workflow workers under UTC and a fixed hash seed; refusal tests inject their readers; red tests that wait on a
  later task carry `#[ignore = "green with Task 044-NN"]`, removed by the prerequisite; the release gate checks none
  remains.
- **D044-205** (2026-11-25) `CommandDigester` exposes `sha256` and `command_digest_input`; the domain folds the chain;
  golden equality is Task 044-5's.
- **D044-206** (2026-11-25, supersedes D044-193 in part) A migration that finds `platform_requirement_hold` compares its
  security, configuration, owner and whitespace-normalized body with the exact body this design gives, which has no
  capability list; Gap 044-026 is a precondition of Task 044-6.
- **D044-207** (2026-11-25) Move items end in feature 018's item model: `SUCCEEDED` (a dry-run match with `MATCHED`),
  `SKIPPED` with `REPLAY_MISMATCH`, `ENVELOPE_MISMATCH` or `RUN_NOT_MUTABLE`, deferred `RUN_BUSY` then `FAILED`; the fit
  also covers pending signal names, open updates and the committed state; a dry-run item cannot move a pin.
- **D044-208** (2026-11-25) Workflow types are unique by storage (`code_workflow_types`); `workflow_versions` carries the
  type and the digest.
- **D044-209** (2026-11-25, supersedes D044-199 in part) The overlay cache is filled only after a commit and dropped on a
  count mismatch; a cold read loads only the addressed step; whole-definition methods answer from counts or fail closed;
  a run transaction reads at most 768 KiB of overlay rows except the stop drain and a scope's planning.
- **D044-210** (2026-11-25) A continue-as-new needs the run to head an open chain and an inline object input; the
  chain's limits govern the successor.
- **D044-211** (2026-11-25, supersedes D044-190 in part) The claim writes the session's binder into `run_initiators`;
  `runs.tracks_initiators` marks the trees that keep rows; an operator may cancel without grace under `run:operate`.
- **D044-212** (2026-11-25, supersedes D044-173 in part) `state.schema`, `argsSchema` and `resultSchema` stay outside
  feature 016's carriers, as feature 016 requires, and their raw payloads are `pii` as a whole; the server-side fields
  live in `run_events.server_detail`; the pause is an advisory lock; the completion budget is 63 statements (50 + 13),
  the saga scope rows included.

### Decisions after the tenth review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-10/`; `resolution.md` there says how each
finding was answered.

- **D044-213** (2026-11-25, supersedes D044-201 in part) Every value field that may travel as a blob is hashed as its
  descriptor whatever its carrier; the server computes an inline value's descriptor; `d(k)`'s kind strings and targets
  are tabled in Technical Plan 6.2, digests as 64 lowercase hexadecimal characters.
- **D044-214** (2026-11-25) A continue-as-new input is never a blob: the declared state and that input stay inline within
  192 KiB, and the SDK throws `InlineValuesExceeded` at the call that would cross it.
- **D044-215** (2026-11-25) Command-value blobs have the purpose `COMMAND_VALUE`, owned by the run's `~workflow` step at
  the lease's attempt and generation; the accepting completion commits them; a deferral holds its blob through
  `blobs.held_by_seq`; validation reads each distinct blob once, at most 4 MiB per completion, under a fair semaphore.
- **D044-216** (2026-11-25, supersedes D044-202 in part) A completion carries at most 339 commands: 256 from the
  function's calls (48 KiB of metadata) and 83 the SDK issues (64 update decisions, one consumption, one set-state, at
  most 16 scope aborts, one re-emitted deferral; 16 KiB); a run has at most 16 open scopes.
- **D044-217** (2026-11-25, supersedes D044-211 in part) A cancellation without grace stops a code run at once whatever
  was journaled before, under `run:operate`; the grace is in the operation's idempotency fingerprint; any other
  `graceMs` is refused; first-request-wins holds for cooperative requests only.
- **D044-218** (2026-11-25, supersedes D044-212 in part) `run_events.server_detail` is the one place of the seed, the node
  specs and the digests; its readers are named; the reset copies it with every re-emitted row.
- **D044-219** (2026-11-25, supersedes D044-207 in part) A move item awaiting its verdict is deferred `REPLAY_PENDING`,
  which is not a refusal; checks are created ahead for up to 64 items; `created_at` and `busy_checks` are kept.
- **D044-220** (2026-11-25) The migration is split, validation under `SHARE UPDATE EXCLUSIVE` in a second migration,
  indexes as the scheduler-shards migration builds them, the registry's checks swapped as design 043 did.
- **D044-221** (2026-11-25) Conformance runs a code-workflow worker as a separate process through `run.sh workflow-worker`
  under UTC and a fixed seed, controlled over standard input and logging over standard output.
- **D044-222** (2026-11-25) The overlay cache is keyed by the count it was filled at and reads incrementally only within
  256 commands; the reconciler re-checks a waiting step every 3,600 s; the SDK drops its cache on a `REVIEW` answer; the
  activator of a pinned version is an initiator; a refusal that depends on mutable state offers a retry.
- **G044-012** (2026-11-25, later state) The esbuild review is Task 044-10's procedure (exact version, lock integrity,
  `npm audit signatures`, the reviewer's checklist); `scripts/check-vendor.py` is not used.
- **D044-200** (2026-11-25, later state) `golang.org/x/tools` is chosen when Task 044-16 starts (the newest release at
  least two weeks old whose `go` directive is at most 1.25.0), not pinned to v0.28.0 here.

### Decisions after the eleventh review round (2026-11-25)

The reports are in `docs/designs/044-workflow-as-code.reviews/2026-11-25/round-11/`; `resolution.md` there says how each
finding was answered.

- **D044-223** (2026-11-25, supersedes D044-216 in part) A completion carries at most 403 commands: 256 from the
  function's calls and 147 the SDK issues (128 update commands for at most 64 open updates, one consumption covering
  every signal name, one set-state, 16 scope closes and aborts, one re-emitted deferral).
- **D044-224** (2026-11-25, supersedes D044-215 in part) A command names only a blob written under the run's workflow
  lease or held by its latest deferral, which a re-emission names by descriptor; at most 16 command-value blobs per run
  and per completion, apart from the tenant's count of uploaded blobs; a gated input and an approval's subject are
  always inline; the validation semaphore holds 64 MiB, refusing with `UNAVAILABLE` that is not counted.
- **D044-225** (2026-11-25) A terminal output and a continue-as-new input and carried state leave the run and are judged
  against the tenant's content quota only; an update result over the run's bound fails that update only.
- **D044-226** (2026-11-25) The continue-as-new check reads the chain inside the member-chain lock every run transaction
  takes first (`lock_member_chain_sql`), so the lock order is the continuation pass's.
- **D044-227** (2026-11-25) Cancellation takes a closed `CancelOrigin`; the source-text check counts every `stop_run(` call
  site; `steps.code_step` marks code steps for the `outcome_seq` check and the can-act index.
- **D044-228** (2026-11-25) Code update keys and chain signals keep the scope of the address the client used; each stored
  payload is classified by the carrier of the version that accepted it.
- **D044-229** (2026-11-25) A replay-check task is leased for 300 s; an item whose lease expired three times ends
  `REPLAY_CHECK_ABANDONED`; a recorded verdict wakes its item; a verdict of a revoked session is not applied; the unserved
  alert is the in-app kind `WORKFLOW_TASK_UNSERVED`.
- **D044-209** (2026-11-25, later state) The per-transaction overlay bound is 771 KiB (Decision 044-213's round counted
  the addressed step's rows).

- **D044-224** (2026-11-26, later state) The 16 uncommitted command-value blobs are counted per lease (attempt and
  generation), not per run; a later attempt may name an earlier lease's unreleased blob by descriptor.
- **D044-226** (2026-11-26, later state) The chain lock run transactions take is `lock_chain_for_run` (`PgRunStore`),
  which reads the chain's columns; `lock_member_chain_sql` reads none.

### The approval (2026-11-26)

- **D044-230** (2026-11-26) Design 044 is Approved on the path of OD-2026-11-25-2, delegate (OD-2026-11-24-4): round
  11's fixes were re-read once by every reviewer whose findings they answer, one at a time, and each Major fixed; the
  security reviewer re-read until a pass found no Major; the confirmation reports at 14513c85
  (`docs/designs/044-workflow-as-code.reviews/2026-11-26/approval/`) hold 19 Majors (correctness 7, architecture 4,
  egm-review 8, security 0), each settled by a row OD-2026-11-26-441 to OD-2026-11-26-459 and answered by the
  follow-up Tasks 044-34 to 044-38. The Nits of those reports are left for the build, in the tasks that touch their
  text. Architecture decisions 0103 to 0109 stay Proposed until their build-evidence conditions are met.
- **D044-231** (2026-11-26) The later rules the approval re-reads added, recorded here because the earlier decisions
  are partly superseded: a run holds at most 32 uncommitted command-value blobs (lease 16) and the tenant 1,024, a
  namespace a quarter; refusals report `semaphore_full`, `tenant_share`, `command_value_ceiling` or
  `run_command_value_ceiling`, the reason stored in `steps.unavailable_reason`; replay-check items expire 300 s after the
  claim, extended by reading up to 3,600 s; a move requires equal gated entries and `state.maxBytes`; a child's output is
  classified by the union of the parent's entry and the child's current version; the verifier's session is matched by
  id, token hash and namespace; the continue-as-new suggestion fires at 2.5 MiB.

### Decisions while building the server lane (2026-11-26)

- **D044-232** (2026-11-26) The body each command kind contributes to the command chain is the table of
  `specs/044-workflow-as-code/contracts/goldens/README.md`: a work command's code-profile fragment with its value field
  (`input`, or an approval's `subject`) as the descriptor `{"sha256", "size"}`; each typed kind a small closed object, a
  value that may travel as a blob again as its descriptor. delegate (OD-2026-11-24-4): Technical Plan 6.2 fixes the chain
  and which values enter as descriptors but leaves the typed bodies to the corpus; six SDKs and the server must produce
  the same bytes, so they are written once, beside the goldens that test them, rather than left to each implementation.
- **D044-233** (2026-11-26) The worker-protocol contract of Task 044-2 takes minor 1.10, the next free one after design
  045's 1.9 (Gap 044-013, later state). Draft 019 names 1.10 to 1.12 on the branch of PR #279 but has no contract merged,
  so it takes the next free minors when its contract merges, as the register's rule says. Its field numbers are the next
  ones no design has named: `WorkerCapabilities` 11 and 12 (6, 7 and 10 are drafts 019 and 031 and feature 006's),
  `TaskLease` 28 to 30 as a `oneof` (19 to 27 are named by drafts 019 and 031, feature 006 and design 045),
  `CompleteStepRequest` 14 (13 is design 041's), `StepCommandResponse` 5. A work command's value travels in
  `Command.value` beside its fragment, so a blob-carried value never needs a marker inside user JSON; the workflow lease
  carries `workflow_type` and `code_digest` beside the reserved activity name, so no SDK parses that name; one enumeration
  `WorkflowTaskFailureCause` holds every cause of Technical Plan 6.3, and the server refuses a server-only cause in
  `FailWorkflowTask`. delegate (OD-2026-11-24-4): taking 1.13 to keep clear of draft 019 would make a 1.13 session owe
  019's host procedures, which minors being cumulative forbids; the other choices keep the contract smaller for six SDKs.
- **D044-234** (2026-11-26) The REST codes of Technical Plan 13.3 that carry an HTTP status are `details.reason` values
  of the closed `Error.code` set (`409 CONFLICT`, `422 VALIDATION_FAILED`, `429 RATE_LIMITED` and so on), as the
  repository writes every specific REST code (`workflow_primitives_keep_the_rest_error_code_set_closed`). The error
  catalog lists each of them, as Task 044-2 asks, with `http: []`, so the bodiless status mapping of the six SDKs does
  not change (a bodiless `504` must not become `QUERY_TIMEOUT`). delegate (OD-2026-11-24-4): the design's table names
  codes; the repository's contract rule decides where a code travels.
  The catalog then holds three codes that are not failures (`MATCHED`, the reason of a dry-run match, and the deferral
  causes `OUTSTANDING_OBLIGATIONS` and `UNHANDLED_EVENTS`, gRPC `OK`), and an SDK's closed code type accepts the reason
  codes as envelope codes; both are kept, settled against the SDK lane's finding: the catalog is the SDKs' one
  dictionary of codes with their remediation, which a batch item's reason and a completion's cause need as much as an
  error does, and a new origin value would change the closed origin type of six SDKs for no behaviour.
- **D044-235** (2026-11-26) `history:export` is a reserved entry of feature 029's draft permission catalog (not
  assignable to a custom role in 0.3.0, Owner question 044-015), held by the admin role only, and joins the test list of
  permissions only the admin role holds.
- **D044-236** (2026-11-26) The reset rules of design 044 are a table of their own in feature 018's registry with a
  column "Code run" for the rules that differ for code runs (`timer.started` and `wait.started` `REMATERIALIZE`, the
  settled saga outcomes `COPY`), instead of a fourth column on every existing table: the existing rows keep their
  meaning for graph runs and the code-run differences are in one place, which `every_new_event_kind_has_a_reset_rule`
  reads.
- **D044-237** (2026-11-26) Task 044-6 is built against Architecture decision 0112 (design 054, approved on PR #279):
  its shared registry table replaces the shared body of `platform_requirement_hold` that Technical Plan 14 and Decision
  044-206 give, and design 045's rollback floor. Until PR #279 merges, Task 044-6 stays closed; whether the hold of
  Architecture decision 0112 can starve an operator's commands is settled by Task 045-21's barrier test, on which this
  lane does not depend yet.
### Decisions while building the server lane: documents (2026-11-26)

- **D044-238** (2026-11-26) `publishCode` takes the code document's declarations as well:
  `client.definitions.publishCode(manifest, envelope, declarations?)`, where `declarations` holds the other parts of the
  code document of Technical Plan 3.1 (`signals`, `state` with its queries and code updates, `inputSchema`,
  `outputSchema`, `triggers`, `limits`, `chainPolicy`, and the `code` object's `workflowTaskTimeoutMs`,
  `cancellationGraceMs` and `workerQueries`); the SDK assembles the code document from the three and publishes it as
  Technical Plan 3.3 says. The command line's `sagat workflow publish-code --manifest <file> --envelope <file>` gains
  `--declarations <file>`. delegate (OD-2026-11-24-4): Technical Plan 1.12 and 3.3 give publishCode only the manifest
  and the envelope, so signals, state and updates had no way in (architecture review of Task 044-1, finding Major 1); a
  third optional argument keeps the manifest (produced by the build) and the envelope (reviewed by the publisher) apart
  from the declarations, which change with the code's contract with its callers.
- **D044-239** (2026-11-26) The Go workflow-as-code package is `.../sdks/go/workflows` (plural, as the TypeScript entry
  point and the Python module are named), because `sdks/go/workflow` is already the side-effect-free IR builder
  (`sdks/go/workflow/workflow.go`) and exports names such as `Retry` and `Timeout` that would mean something else in the
  runtime. delegate (OD-2026-11-24-4): Technical Plan 1.12 names `.../sdk/workflow`; the collision was found by the
  architecture review of Task 044-1 (finding Major 3). Task 044-16 builds it there.
- **D044-240** (2026-11-26) Query and replay-check tasks carry the run's seed (`QueryTask.seed`, `ReplayCheckItem.seed`
  in the contract of Task 044-2), since the history never does and a replay of a run that drew random numbers or
  identifiers needs it. delegate (OD-2026-11-24-4): Technical Plan 13.1 gave the seed only to the workflow lease
  (architecture review of Task 044-1, finding Major 2); the holders of the seed are unchanged (workers bound to the
  type).

- **D044-241** (2026-11-26) A fragment may repeat a publisher field of its covering envelope entry (`secrets`,
  `irreversible`, `highRisk`, `compensateWith`, `reconcileWith`, `requiresApproval`) only with the effective entry's
  value (`secrets` compared as sets); a field the entry does not set is refused, so a command that adds or changes one is
  `COMMAND_OUTSIDE_ENVELOPE`. Task 044-5 holds it with `a_fragment_that_adds_a_secret_is_outside_the_envelope`.
  Raised by the security review of Task 044-2 (finding Minor 5), fixed in the schema's description of the profile.
- **D044-242** (2026-11-26) The security review of the contract of Task 044-2 (commit 6e641a94; no Blocker, two Major,
  eleven Minor) is answered by fixes in the contract, none settled otherwise: `ReadBlobRequest.read_token` (field 9)
  and the narrowed `ReadBlob` and `WriteBlob` rules with the order NOT_FOUND before PERMISSION_DENIED (Major 1); the
  `secret` fields of an inline value replaced by the typed placeholder for workflow leases and read tokens (Major 2);
  a withheld blob carrying its size only; code-profile durations, event types and child workflow identifiers under the
  graph's bounds and patterns; Decision 044-241; an error block on each new RPC; the lifetime and re-checks of read
  tokens; the outer fields of `TaskLease` per task kind; the bounds of answers, consumption lists, failure messages,
  markers, workflow types, digests and query failure codes; the export paged at 1,024 events or 3 MiB; the conditions of
  the eager workflow task; replay-check ranges bound to their lease. Its finding Minor 13 (the seed now reaches query and
  replay-check workers, Decision 044-240): Technical Plan 4.5, 13.1 and 14 read with Decision 044-240, and the
  source-text test of Task 044-24 on `server_detail` allows the seed on the claim paths of the three task kinds.

- **D044-243** (2026-11-26) On the wire a terminal or continue-as-new command travels the first time last in
  `commands`, numbered as the next command (the SDK cannot know it will be deferred); a deferred one takes no number,
  and the next accepted command takes that number; each re-emission after a deferral travels in `deferred_command` with
  number 0, and is numbered when accepted. delegate (OD-2026-11-24-4): Technical Plan 4.4 and 13.1 named the re-emission
  only; found while building the fake server of Task 044-30, whose tests hold it.

### Decisions on the TypeScript lane's relay (2026-11-27)

- **D044-244** (2026-11-27) An activity's or a script's saga scope is compared at every level that compares commands.
  It already enters the hashed body (`"scope": "s-<n>"`, the body table of the corpus README, Decision 044-232), so the
  server's chain answers `CHAIN_MISMATCH` when a changed function schedules the same activity in another scope or
  outside any; in addition `HistoryCommandRecord` gains `saga_scope` (field 5: `s-<n>` for an activity or script
  scheduled in a scope, empty otherwise), so that the SDK compares kind, target and scope at replay (Technical Plan 6.1)
  and reports `NONDETERMINISM` at the command that moved, and a replay of an exported history (Technical Plan 7.4),
  which has no chain to compare, catches it as well. delegate (OD-2026-11-24-4): the scope decides which compensations
  the engine plans (Technical Plan 10.1), so a replay that moves a member between scopes is another saga; left to the
  chain alone it would be found only by the server and a dry-run move, under the less precise cause, and never by the
  replayer in continuous integration. The scope is a structural identifier of the run, as `node` is, not a body, an
  actor or risk metadata, so the rule that a record never carries a body holds. A report of a mismatch names the scope
  the function used (`FailWorkflowTaskRequest.actual_saga_scope`, field 14), so a change of scope alone is reported
  with what differs; `Command.saga_scope` on any kind but an activity or a script is `COMMAND_INVALID`. An export of
  format 1 gains the field additively: a replayer ignores the fields of a line it does not know (`sdks/SPEC.md` 12.9),
  and no export of format 1 exists yet (Task 044-24 builds it), so no reader meets a format 1 line without the scope
  of a scoped activity. Answers the TypeScript lane's Gap 044-303.
- **D044-245** (2026-11-27) A cancelled step that the function can cancel with `step_cancel` (an activity, a script, a
  wait for an event, an approval command's step, or an activity held for its entry's approval) reaches the function as
  `HISTORY_STEP_OUTCOME` with `STEP_OUTCOME_CANCELLED` and the error code `CANCELLED`, the history form of the journal's
  `step.cancelled` for that step. The origin is `effect` when the completion's own `step_cancel` cancelled a step whose
  current attempt has not begun (`READY`, including a step waiting for its retry after earlier attempts that each
  ended in a known failure, or `WAITING` on an event, an approval, or a gate of feature 022; an open approval request
  is withdrawn in that transaction), checked by a compare-and-swap on the step's row under the run lock (Technical
  Plan 5.2, step 6). A step whose attempt began (leased, or awaiting its reconciliation or a
  review after an unknown outcome, feature 037) is never cancelled by the command: `step_cancel` journals
  `step.cancel_requested`, the review or reconciliation stays in place, and the step's outcome is whatever its
  attempt, its reconciliation or its review settles, with the origin `engine` (`CANCELLED` only when the worker or the
  review establishes it), because stopping is not evidence that an effect never happened. The origin is `engine` too
  when the engine cancels a step: a saga abort's `cancelPolicy` on a member still in flight, and after the run closed
  the stop drain and continue-as-new's handoff (a timer or a wait included, which the function never sees). A child
  whose run has not started yet (its `child_run_id` reserved, the children pass not run) ends with
  `HISTORY_CHILD_FINISHED`, status `CANCELLED`, an empty `child_run_id` and the origin `effect`, and the children pass
  skips it. `HistoryWaitOutcome` (satisfied or timed out) and `HistoryApprovalDecided` (approved,
  rejected or expired) keep their outcomes; a child's cancellation is its `HISTORY_CHILD_FINISHED` with the status
  `CANCELLED` once the child run ends; a timer the function cancels ends with the record of its `timer_cancel`, which is
  that `step.cancelled`. delegate (OD-2026-11-24-4): one outcome for every cancellable step, the one the journal
  already writes, instead of a cancelled variant in each of two messages; the TypeScript runtime already takes it. The
  corpus gains the run `code-cancellation/own-steps`.
- **D044-246** (2026-11-27) In every transaction that creates a run (a start, Signal-with-Start, Update-with-Start, a
  schedule's or trigger's start, a child's start, a successor's handoff), `run.started` is the run's first journal row,
  at sequence 1; the signal a Signal-with-Start buffers and the update an Update-with-Start admits are appended after it
  (and after the `run.start_delayed` that feature 018 journals right after `run.started` for a delayed start) in the same
  transaction, so activation 1 applies `run.started`, where the function begins and registers its handlers,
  before the `signal.buffered` or the `update.admitted`. A reset's new run begins with the prefix it copies, whose first
  row is the source's `run.started`. delegate (OD-2026-11-24-4): an SDK starts the function on `run.started`, and the
  TypeScript runtime rejects an update applied before any handler exists (`HANDLER_NOT_REGISTERED`); the corpus
  (`code-update-with-start`, sequences 1 and 2) and `the_goldens_parse_and_their_chains_recompute` ("run.started
  first") hold it for the goldens, and Task 044-21 holds it on the server with
  `update_with_start_journals_run_started_first` and `signal_with_start_journals_run_started_first`.
- **D044-247** (2026-11-27) The code profile follows the graph's nodes for both: a script command takes `timeoutMs`, as
  a graph script node does (the base node's `timeoutMs`, which `crates/infra-ir/src/convert.rs` reads for every node),
  so Technical Plan 1.12's `script(..., {retry, timeoutMs})` stands and `codeProfileScript` gains the field; the
  attempt is bounded by it and by its artifact's policy (`wall`, 1 s to 15 min), whichever ends first; and a wait for
  an event requires `timeoutMs`, as the graph node does, so Technical Plan 1.12's signature becomes
  `waitForEvent<T>(eventType, correlationKey, options: { timeoutMs: number; lookbackMs?: number })`, and `sdks/SPEC.md`
  with it. Both durations are the code profile's (1 ms to 366 days), stricter than the graph's bare minimum on purpose
  (Decision 044-242). delegate (OD-2026-11-24-4): a wait without a bound would hold its subscription for the run's
  lifetime, which the graph refuses for the same reason; a function that waits longer loops over bounded waits, each
  with a `lookbackMs` that covers the gap between one wait's timeout and the next wait's start (at most 7 days), or an
  event that arrives in the gap is not seen. Task 044-5 holds the script's field with
  `a_script_fragment_takes_timeout_ms_as_a_graph_script_node`.
- **D044-248** (2026-11-27) Technical Plan 1.12 names the cancellation of a function's own steps and what it raises:
  `requestCancellation(command)` on the promise of an activity, a script, a wait, an approval, a child or a sleep makes
  `step_cancel` (`timer_cancel` for a sleep); the promise of a cancelled activity, script, wait or approval rejects with
  `CommandCancelled`, a cancelled sleep with `TimerCancelled`, a cancelled child with `ChildWorkflowError` of status
  `cancelled`. The promise settles from the history's outcome, never at the call: it resolves if the attempt completes,
  and rejects only on a cancelled outcome (a sleep, whose `timer_cancel` is its end, rejects once the command is
  issued). delegate (OD-2026-11-24-4): the names the TypeScript lane built (its Decision 044-311), so six SDKs make
  the same commands and raise the same three errors for one golden.
- **D044-249** (2026-11-27) The independent correctness review of the relay changes (commits 9808a47e and b1f03c8b; no
  Blocker, six Major, five Minor, two Nit) is answered by fixes, none contested: the reported scope
  (`actual_saga_scope`, Major 1); Decision 044-247 restated on the graph's real rules (Major 2: a graph script node does
  take `timeoutMs`); the cancellation of a step whose attempt began, and of a child not started (Majors 3 and 4,
  Decision 044-245); the cancel call and its errors (Major 5, Decision 044-248); the named server tests (Major 6, in
  Tasks 044-5, 044-8, 044-9 and 044-21); the corpus checks of one end per step and of the origins (Minor 7); the
  additive export (Minor 8, Decision 044-244); the other cancellation sources (Minor 9) and the scope on another kind
  (Minor 10), Decision 044-245 and 044-244; the gap between bounded waits (Minor 11, Decision 044-247); the Nits in
  Technical Plan 4.2 and Decision 044-246.
- **D044-250** (2026-11-27) The independent security review of the relay changes (commits up to 3c40a11b; no Blocker,
  one Major, three Minor, one Nit group and one suspicion) is answered by fixes, none settled otherwise (a security
  finding is always fixed): `actual_saga_scope` is checked against `^s-[1-9][0-9]{0,9}$` and dropped otherwise, the
  expected scope comes from the server's record, and it joins Non-functional requirement 044-007's list of identifiers
  checked before they appear (Major 1; `a_reported_saga_scope_outside_its_pattern_is_dropped`, Task 044-8); the
  effect rule counts a step waiting for its retry after known failures, and Technical Plan 10.2 states that a script's
  timeout is a known failure as for a graph script, Gap 044-032 (Minor 2); the cancellation is a compare-and-swap on
  the step's row under the run lock, with two barrier tests in Task 044-9 (Minor 3); no eager lease for a step its own
  completion cancels, Task 044-14 (Minor 4); `actual_saga_scope` in Technical Plan 13.1 and `sdks/SPEC.md`, a script's
  `timeoutMs` in Technical Plan 5.1, the description of Decision 044-241 on `codeProfileScript`, and the main
  cancellation golden's wording (Nit 5); and the promise of a cancelled step settles only from the history's outcome
  (the suspicion, Decision 044-248 and `sdks/SPEC.md`).
- **D044-251** (2026-11-27) The closed list of update rejection reasons is `VALIDATION_FAILED` (the function's
  validator refused the update), `HANDLER_NOT_REGISTERED` (the function had registered no handler for the update's name
  when its `update.admitted` was applied) and the engine's `RUN_ENDED` and `RUN_CONTINUED`; a completion that rejects
  with any other reason is `COMMAND_INVALID`. The catalog entry of `HANDLER_NOT_REGISTERED` and its six SDK copies land
  with the update transitions of Task 044-21. delegate (OD-2026-11-24-4): the TypeScript lane's Decision 044-309 asked
  for `HANDLER_NOT_REGISTERED` and `VALIDATION_REJECTED`; the first names a case the list lacked (a code defect that
  the caller and the operator must tell apart from a refused argument, as Temporal's unknown-update rejection does),
  while the second would be a second name for `VALIDATION_FAILED`, which the corpus and the catalog already use.
- **D044-254** (2026-11-27) Two alignments the security review of pull request 296 asked of the contract (its Nits 2
  and 3; the domain's answers are Decision 044-253 on that branch): `codeActivityPattern` admits an exact name of 128
  characters, as `codeActivityName` and the domain do, and a prefix of 128 with its `.*`; and Technical Plan 5.1 says
  that a fragment may repeat only the publisher's fields (`secrets`, `irreversible`, `highRisk`, `compensateWith`,
  `reconcileWith`, `requiresApproval`), while `outputSchema`, `concurrency` and `rateLimit` come from the entry and are
  refused in a fragment, as the code profiles already refuse them: a repeated `outputSchema` could drop a `secret`
  classification. delegate (OD-2026-11-24-4).

## 3. Gaps

- **G044-001** (2026-11-24) The memory of an idle cached run is not measured in any SDK; the TypeScript spike's heap
  measurement was unreliable (negative). Non-functional requirement 044-005 is a target until Task 044-25 measures it.
- **G044-002** (2026-11-24) The Java SDK has no static determinism checker in 0.3.0; detection relies on the per-call
  thread check, the command comparison and the server's chain.
- **G044-003** (2026-11-24) The `database`, `icebergWrite`, `forEach`, `while`, `repeatUntil` and `parallel` nodes are not
  commands in 0.3.0; a graph child covers them.
- **G044-004** (2026-11-24) Signalling or cancelling another run from code is not a command in 0.3.0; an activity that
  calls the REST client does it, at-least-once.
- **G044-005** (2026-11-24) A worker of the tenant can forge a `match` verdict for a move; the chain check limits what a
  forged verdict can claim, and the harm stays inside the tenant.
- **G044-006** (2026-11-24) No automatic move policy (for example "move every run whose replay matches at each deploy")
  in 0.3.0; it would need the constitution to admit it.
- **G044-007** (2026-11-24) A Go workflow that starts a native goroutine or uses a native channel escapes the dispatcher;
  only `sagatvet` and the activation deadline catch it.
- **G044-008** (2026-11-24) A `vm` context is not a security boundary; deliberate escape by the tenant's own workflow
  code is out of scope because the code runs in the tenant's worker.
- **G044-009** (2026-11-24) Rolling back the platform below 0.3.0 with code runs open is refused by the migration's down;
  there is no conversion of a code run.
- **G044-010** (2026-11-24) The constitution 5.0.0 text (owner decision OD-2026-11-24-2, drafted in another pull
  request) should say that an explicit, verified, audited per-run move is not the automatic migration principle IV
  forbids; until it does, the Constitution Check records it as "Pass with conditions".
- **G044-011** (2026-11-24) Inline graph fragments (an engine-run `forEach` inside a code run without a child) are not in
  0.3.0.
- **G044-012** (2026-11-24) The esbuild dependency of the TypeScript SDK's bundle command needs the supply-chain review of
  `scripts/check-vendor.py` and a pinned, hash-locked version before Task 044-10 merges.
- **G044-013** (2026-11-24) The protocol minor is provisional (expected 1.9) until the contract of Task 044-2 merges and
  takes the next free row of the register.

- **G044-014** (2026-11-24) Publication approval reviews the envelope, the declarations and the digest, not the code,
  which the platform never receives; signed build provenance for the digest, as script artifacts have, is not in 0.3.0.
- **G044-015** (2026-11-24) The behaviour flags of the SDKs are a reserved protocol field; the mechanism is built by the
  first SDK release that changes how it turns calls into commands.
- **G044-016** (2026-11-24) `PYTHONHASHSEED` is a 32-bit value; anyone who can read a Python code version's digest and
  knows its modules' sources could search the seed from the fingerprint in the digest. Readers of versions are the
  tenant's own; an outside sender sees no digest.

- **G044-017** (2026-11-25) Architecture decision 0111 (branch `docs/temporal-parity-readiness`, Proposed) requires a
  buffered name to be the event type of a `waitEvent` node; for a code version the consumer is the activation. Its owner
  must admit code versions (all declared signals buffered, consumption by the completion) before Task 044-21 builds on it.
- **G044-018** (2026-11-25, replaces the second entry the first round wrongly numbered G044-005) With a chain over
  fragments the history does not carry, a worker can produce a `match` only by producing the same commands; a worker of the
  tenant that runs the tenant's code to forge a verdict harms only the tenant's own runs. The first draft's sentence that
  "the chain check limits what a forged verdict can claim" was false for its chain.
- **G044-019** (2026-11-25) A reset of a code run to another code version is not in 0.3.0.
- **G044-020** (2026-11-25) Design 045's deadline on a live attempt (Architecture decision 0110 decision 4, branch
  `docs/temporal-parity-readiness`) must exclude the workflow-task step; its owner is told, and Task 044-27 tests it.
- **G044-010** (2026-11-25, later state) The constitution 5.0.0 draft (branch `docs/constitution-5.0.0`, principle IV)
  states the verified move as this design does; the gap closes when that text merges.

- **G044-013** (2026-11-25, later state) Design 045 names 1.9, so the minor of this design is the next free one after
  it at merge, not 1.9; Decision 044-024's "expected 1.9" is withdrawn.
- **G044-021** (2026-11-25) The constitution 5.0.0 draft keeps "every run references exactly one version"; a code run
  references its start version as its identity origin and is pinned to one current version. The draft's owner should
  say so before it merges, and its sentence "within one version, explicit version markers" should say that patches take
  effect on a running run only through a verified move.
- **G044-022** (2026-11-25) The version diff (`DefinitionDiff`) has no form for two code versions in 0.3.0; an envelope
  and declaration diff is a follow-up.
- **G044-023** (2026-11-25) The update waits are bounded per API process (256), not across processes; a wait holds no
  database resource and re-reads at most 64 times in 60 s.
- **G044-016** (2026-11-25, later state) Outside data (third-party outputs, webhook events) does reach Python workflow code,
  so the seed's secrecy matters; the SDK also caps decoded objects at 10,000 keys, which bounds the damage of colliding
  keys whatever the seed.

- **G044-021** (2026-11-25, later state) The 5.0.0 draft now reads "a run's pin names exactly one version at any moment"
  and covers version markers; the gap closes when that text merges.
- **G044-020** (2026-11-25, later state) Design 045 must also accept the envelope's `timeoutLevel` as the level of a code
  version, and own `steps.min_protocol_minor`, on which this design depends.
- **G044-017** (2026-11-25, later state) Architecture decision 0111 must admit code versions as consumers, its inbox
  design must be approved, and Task 044-21 waits for both; no fallback builds the inbox here.

- **G044-024** (2026-11-25) The constitution 5.0.0 draft (branch `docs/constitution-5.0.0`, principle IV, "A completion
  is one transaction") says a completion "commits all or nothing" and "a completion that ends the run is refused while
  an accepted signal, update or cancellation is still unseen". This design defers the terminal command instead and
  commits the rest of the completion, because a refused completion repeats for ever when the cause is an obligation the
  function cannot settle (round 4). The draft's owner should word the clause as "the command that ends the run is not
  applied while ... and is deferred; the rest of the completion commits all or nothing". Until then the Constitution
  Check records principles I and IV as "Pass with conditions". Raised by the architecture review of round 5.

- **G044-025** (2026-11-25) The 5.0.0 draft (principle IV) applies a move only while "the journal has not advanced past
  the activation that was verified". This design applies it while no completion has followed the verified activation;
  waking events committed after it are not yet applied by the function and are delivered in the next activation on the
  target code. The draft's owner should word the clause "the run has not completed an activation past the one
  verified". Raised by the architecture review of round 6.

- **G044-026** (2026-11-25) Design 045 (branch `docs/temporal-parity-readiness`) writes the closed list of capabilities
  into the body of `platform_requirement_hold`, so a second design that adds a capability would change the shared body.
  This design keeps the list out of the body and relies on the table's closed check; design 045's owner should do the
  same before either migration lands. Raised by the architecture review of round 8.

- **G044-027** (2026-11-25) Draft 049 (journal buckets) and draft 052 (the next step in the completion answer), in the
  scale-resilience worktree, touch `append_events`, the new tables and Task 044-14's eager workflow task; whichever lands
  second adapts, and their owners are told. Raised by the architecture review of round 9.

- **G044-027** (2026-11-25, later state) Draft 048 (high availability and rolling upgrades: expand and contract phases,
  validation in a later migration, `platform_requirement_activate_v2`) binds Task 044-6 and `code-workflows-v1` as well;
  its owner is told.

- **G044-021, G044-024, G044-025** (2026-11-26, later state) Answered by constitution 5.1.0, which the owner approved
  (OD-2026-11-25-3) to word the pin, the deferred terminal command and the move clause as this design needs; they close
  when 5.1.0 merges; Tasks 044-9 and 044-13 merge only after it (OD-2026-11-25-3 is recorded with the 5.1.0 pull
  request).
- **G044-017** (2026-11-26, later state) The buffered-signals design 054 takes Architecture decision 0111's inbox and its
  refinement for code versions, and its inbox row carries the version that buffered each signal (Decision 044-228);
  Task 044-21 waits for it.
- **G044-020, G044-026** (2026-11-26, later state) Handed to design 045's owner by the coordinating session.

- **G044-028** (2026-11-26) Draft 019's `FailStepRequest.details_json = 13` (PR #279, `specs/019-worker-deployments-dispatch/contracts/worker-protocol.md`)
  takes the number design 041 gives to `StepLogCarry step_logs = 13`; not this design's to settle, reported to the
  coordinating session for draft 019's owner.
- **G044-026** (2026-11-26, later state) Design 045's text on PR #279 drops the capability list from the hold's body,
  and design 054's Architecture decision 0112 replaces the shared hold with a registry table (Decision 044-237); the gap
  closes when PR #279 merges.
- **G044-029** (2026-11-26) Design 044, Technical Plan 8.2, says a chain-addressed update of a code run behaves as for
  a run, but table 13.2 does not list the chain's update route (`updateExecutionChain`), whose contract therefore has no
  `waitMs`, `acceptPending` or `202`; Task 044-21 adds them with the update transitions.

- **G044-030** (2026-11-26) Feature 020's draft plan grants a permission `run:history:export` to the admin, developer and
  operator roles for its own export route; if it lands without excluding code runs, it would bypass Owner question
  044-015 (`history:export`, admin only). Reported to the coordinating session for feature 020's owner. Raised by the
  security review of Task 044-2.
- **G044-031** (2026-11-26) A code run's command records must not carry a payload digest in `run_events.payload_digest`
  that `JournalEvent.payloadRef` would return (an input, a state or a side-effect digest would be a server-side field
  under Technical Plan 4.5); Task 044-9 writes them and Task 044-24's `no_read_returns_a_body_digest_risk_metadata_or_the_seed`
  checks the column. Raised by the security review of Task 044-2.

- **G044-032** (2026-11-27) A script's attempt that ends by its `timeoutMs` or its artifact's `wall` fails as a known
  failure (`SCRIPT_TIMEOUT`, retryable), for a graph script node and for a code run's script command alike, although
  the script's egress through its supervisor may have had an effect; feature 037's unknown-outcome rule covers
  activities only. A code run keeps the graph's rule (Technical Plan 10.2); whether a script's timeout after its
  attempt began should be an unknown outcome is for the owners of features 005 and 037. Raised by the security review
  of the relay changes (Decision 044-250).

## 4. Owner questions (all delegated)

- **O044-001** (2026-11-24) Should code under a running run ever change? delegate (OD-2026-11-20-6): yes, only by an
  explicit, per-run move verified by a replay of that run's history on the target code. Never-move would hold old workers
  for the lifetime of the longest run and leave a held run only reset or termination; Temporal's replay-on-new-code is
  the source of production nondeterminism. ADR-0105.
- **O044-002** (2026-11-24) How is the code digest computed for compiled languages? delegate (OD-2026-11-20-6): over the
  executable, assembly or jars, strict and honest; a user-chosen build id was rejected because a reused id with
  different code is the case that corrupts runs, and moves of unchanged workflow code always match.
- **O044-003** (2026-11-24) Should the server check determinism as well as the SDK? delegate (OD-2026-11-20-6): yes,
  the command chain (32 bytes per run, one SHA-256 per command) turns an SDK or cache bug into a hold instead of a
  corrupted run.
- **O044-004** (2026-11-24) Python's hash randomization: require it off, or detect set iteration? delegate
  (OD-2026-11-20-6): require `PYTHONHASHSEED=0` for workers that register workflow types. Set iteration cannot be
  detected at run time; a fixed seed is the only reliable fix. The protection it removes is against crafted colliding
  keys: with a known seed, a payload of up to 256 KiB can hold keys that collide and make a dictionary slow. The cost
  falls on the tenant's own worker, and the activation deadline (2 s) turns such a slow activation into a workflow task
  failure rather than a stalled worker.
- **O044-005** (2026-11-24) Java 17 with platform threads, or Java 21 with virtual threads? delegate (OD-2026-11-20-6):
  a new artifact on 21; about 1,845 bytes per parked coroutine against a platform thread's reserved stack makes the
  sticky cache affordable, and a dual implementation would double the test matrix.
- **O044-006** (2026-11-24) Keep refusing signals nobody waits for, as graph runs do? delegate (OD-2026-11-20-6): no
  for code runs, journal declared signals at any open state; a function cannot always be waiting, and the journal is
  where replay finds them; graph runs are unchanged.
- **O044-007** (2026-11-24) Local activities? delegate (OD-2026-11-20-6): no; principle III; eager dispatch recovers the
  latency.
- **O044-008** (2026-11-24) Continue-as-new to the active version? delegate (OD-2026-11-20-6): allowed for code versions
  with `version: active`, default `same`.
- **O044-009** (2026-11-24) An unexpected exception: fail the run or hold it? delegate (OD-2026-11-20-6): hold; a code bug
  must not lose a run whose effects already happened; the workflow failure type fails it on purpose.
- **O044-010** (2026-11-24) Sagas in code: engine-planned or hand-written? delegate (OD-2026-11-20-6): engine-planned,
  with hand-written compensation still possible as ordinary activities; feature 037's rules then apply to code for free.
- **O044-011** (2026-11-24) Queries: engine or worker? delegate (OD-2026-11-20-6): both, declared queries by default
  because they work without workers and after the end; worker queries for what only the code knows.
- **O044-012** (2026-11-24) Which commands in 0.3.0? delegate (OD-2026-11-20-6): the list of Requirement 044-012; the
  rest are graph children until a later release (Gap 044-003, Gap 044-004).
- **O044-013** (2026-11-24, supersedes the answer of O044-004) A fixed hash seed of `0`, or a seed the operator
  chooses? delegate (OD-2026-11-20-6): the operator's own fixed value. A seed known to everyone lets an outside sender
  build colliding keys; a fixed secret value keeps determinism (every worker of a digest has the same seed, which the
  digest proves) without that exposure. Raised by the security review of round 1.
- **O044-014** (2026-11-24) Worker queries in 0.3.0, or declared queries only? delegate (OD-2026-11-20-6): keep worker
  queries, as a deferrable task. Some answers exist only in the function's memory (journey 7 of the design), and the
  alternative, publishing state at every iteration, grows the journal by one event per iteration. Contested against the
  zoom-out review of round 1 (finding 7), which asked to defer them.
- **O044-015** (2026-11-25) Who holds `history:export`? delegate (OD-2026-11-24-4): the admin role only, by default.
  Exported histories leave the platform for developers' machines; even redacted, they hold business data, and a wider
  default is a security choice a tenant should make itself.
- **O044-016** (2026-11-25) May a deploy pipeline request moves? delegate (OD-2026-11-20-6): no. Constitution 5.0.0 says a
  pin changes only by a person's explicit request and no policy moves runs; a pipeline step that moves every open run at
  each deploy is such a policy. A person requests a dry run and the move of a frozen selection.
- **O044-017** (2026-11-25) Approvals in code runs: advisory signals, or engine-held steps? delegate (OD-2026-11-24-4):
  engine-held. An entry's `requiresApproval` makes the engine hold the step and open the approval of that very step, so
  the decision is bound to the effect it guards; publication approval can see which activities are gated. Raised by the
  security review of round 3.
- **O044-018** (2026-11-25) Sticky exclusivity in the core slice, or deferrable? delegate (OD-2026-11-20-6): deferrable
  with eager dispatch (Task 044-14). The core keeps the worker's cache, reused when the window follows it; exclusivity is
  an optimisation to be measured, and it brings a routing string with several writers. Raised by the zoom-out review of
  round 3.
