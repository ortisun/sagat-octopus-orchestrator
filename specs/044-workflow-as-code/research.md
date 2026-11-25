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
