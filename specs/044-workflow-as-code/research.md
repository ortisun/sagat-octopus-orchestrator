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
