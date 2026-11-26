# Feature 022 — Research: baseline, external semantics, decisions, gaps and owner questions

Baseline investigated on 2026-10-26 in the worktree cut from `origin/develop` at `70f09435` (release
0.2.0). Nothing below is implemented by this feature.

## Repository baseline (verified paths)

| Area | What exists | Where |
|---|---|---|
| IR and capability gates | v1 schema (`bodies`, `limits`, `state`, `chainPolicy`; `base` has `next`, `onError`, `timeoutMs`); codec refuses disabled kinds | `specs/001-platform-foundation/contracts/workflow.schema.json`, `crates/infra-ir/src/convert.rs` |
| Canonical JSON | sorted keys, ordered arrays, SHA-256 digest | `crates/infra-ir/src/canonical.rs` |
| Evaluator caps | 1024-byte source, 256 tokens, depth 16, 64 KiB strings | `crates/infra-expr/src/limits.rs` |
| Node model, journal | `NodeKind`, `Node` (`on_error`, `effect`, `origin`); events `step.completed`, `end.reached`, `run.finished`, `saga.*` | `crates/domain/src/workflow.rs`, `crates/domain/src/execution/event.rs` |
| Run result | `end.outputExpr` → `runs.result_digest`; `verify_run` check | migration `20261110000000_run_result`, `specs/002-durable-execution/research.md` (G21), `crates/infra-postgres/src/execution.rs` |
| Result disclosure | `payload-disclosures` `{"source":"result"}` under the pinned carrier | `crates/application/src/payload_disclosure.rs`, FR-016-006 |
| Classification carriers | workflow `inputSchema`/`outputSchema`; `outputSchema` on activity, script, waitEvent, child | FR-016-002, ADR-0033, ADR-0036 decision 6 |
| Ad hoc admission | private definitions per actor and digest; origins `AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE` | `crates/application/src/runs/ad_hoc.rs`, migration `20261125000000_database_ad_hoc` |
| Script test runs | draft script, no namespace secrets, own quota | `createScriptTestRun` (canonical OpenAPI), FR-005-005 |
| Per-activity concurrency | `activity_quota_limits`; claim counts leases | migration `20261019000000_activity_quotas`, `crates/infra-postgres/src/workers.rs` |
| Quotas | `QuotaResource`; policy fences; deferred byte accounting | `crates/domain/src/quota.rs`, `crates/application/src/quotas.rs`, `crates/infra-postgres/storage-classification.json`, ADR-0027 |
| Namespace environment | `namespaces.environment IN ('dev','staging','prod')` | migration `20261008000000_identity` |
| Permissions | 48-entry catalog; roles admin, developer, operator, viewer, worker, runner, database-runner; `script:execute` is the runner's | `crates/domain/src/identity.rs` |
| Keyed digest precedent | `ring::hmac` binding key of 017 provisioning | `crates/application/src/managed_secrets.rs` |
| Scheduler | 64 shards, lease + generation fence; passes, monitor | `crates/scheduler/src/{passes.rs,shards.rs,monitor.rs}`, ADR-0020 |
| Obligations | rows with the fact; 12 attempts; `DEAD`; redrive | ADR-0012, ADR-0025, migration `20261012000000_events` |
| Worker protocol | minor 1.6; `TaskLease.input_json` is the run data document | `specs/001-platform-foundation/contracts/worker.proto` |
| Errors | closed `Error.code` + `details.reason`; `DefinitionProblem.code` | `sdks/error-catalog.json`, canonical OpenAPI |
| Conformance | five SDKs, golden observations | `tests/contract/sdk-conformance/README.md`, `sdks/SPEC.md` |

Observations that shape the design:

1. An activity has no input mapping; its lease carries the whole run data document. A digest of that
   document would rarely hit, so the cache needs an explicit `key` (D1161).
2. The run result has one storage location and one disclosure path; early return reuses both (D1169).
3. 017 solved "durable work without publication" with private retained definitions; standalone steps and
   draft test runs reuse it (D1183, D1185).
4. Step permits fit the existing claim-time counting; run permits need a queue because an admitted run
   has no `READY` state to wait in (ADR-0056).

## External semantics consulted (2026-10-26)

- Windmill cache — <https://www.windmill.dev/docs/flows/cache>: same inputs within the duration "return
  the cached result"; key construction, scope and invalidation are not described (defined here).
- Windmill early return — <https://www.windmill.dev/docs/flows/early_return>: "the rest of the flow will
  continue asynchronously"; loop steps cannot be selected; an untaken branch means the caller waits for the
  final result. Adopted with stricter placement (D1170).
- Windmill flow error handler — <https://www.windmill.dev/docs/flows/flow_error_handler>: runs after
  retries, receives the error; the flow stays failed unless `recover: true`. Recovery not adopted (O1161).
- Windmill error handling — <https://www.windmill.dev/docs/core_concepts/error_handling>: workspace handler
  for failures without a specific one; error-handler jobs never trigger handlers; cancelled jobs may be
  skipped. Adopted as the namespace default (D1175, D1176).
- Windmill concurrency limits — <https://www.windmill.dev/docs/core_concepts/concurrency_limits>: maximum
  executions per time window, optional custom key, excess "automatically queued". Here only concurrent
  holders are limited and keys are namespace-scoped (D1181).
- Windmill mocking and pinning — <https://www.windmill.dev/docs/flows/step_mocking>: a mocked or pinned step
  "will immediately return the specified value"; production use is not described; here never (D1186).
- Temporal Standalone Activities — <https://docs.temporal.io/standalone-activity>: client-started activity
  with its own ID space, conflict/reuse policies, describe/list, cancel, terminate, retention-bound result;
  GA, operator commands in preview. Adopted as an ordinary run of a one-node definition; ID policies are 018
  (G1166), heartbeat details 019.
- Temporal Early Return — <https://docs.temporal.io/design-patterns/early-return>: Update-with-Start returns
  after initialization; replaced here by a declarative node (D1193).

## Decisions

- **D1160 — Four IR capabilities, two API surfaces.** `stepCache`, `earlyReturn`, `failureHandler`,
  `concurrencyKeys` are additive gated v1 capabilities enabled one at a time (ADR-0032 decisions 3, 8);
  standalone and test runs are API surfaces; no protocol minor. *Rejected:* one umbrella gate.
- **D1161 — Explicit cache key.** `cache.key` is required, canonicalized and keyed-digested; the fingerprint
  includes the canonical key expression. *Rejected:* digest of `input_json` (rarely hits, up to 8 MiB) or of
  the run input (wrong when earlier outputs are read).
- **D1162 — Scopes.** `version` (default), `workflow` (identical fingerprint), `namespace` (same `name` and
  fingerprint); never across namespaces. A private definition is its own version.
- **D1163 — Cacheable nodes.** Root activity/script without compensation, reconciliation or high-risk or
  irreversible effects, so no run compensates or skips another run's effect. Database results keep their
  own disclosure (G1164); bodies wait for evidence (G1160).
- **D1164 — A hit is a journaled completion with a retained copy.** Replay never reads the cache; the copy
  is charged like an output and can hit `RUN_DATA_LIMIT` or the storage-quota path as a completion can.
  The lookup runs inside the readiness transaction under the run lock and takes no shard fence: readiness
  happens in worker completion transactions, which hold no shard lease (`locked_scheduler_time_sql` in
  `crates/infra-postgres/src/execution.rs` is only for scheduler paths); deferring the lookup to a
  scheduler pass would let a worker claim the `READY` step first (architecture review 022 S1).
- **D1165 — Asynchronous store.** An in-transaction store would let the deferred quota check (ADR-0027) turn
  a successful step into `STORAGE_QUOTA_EXCEEDED`; completion writes an obligation, a writer stores later.
  Blob outputs are not stored (G1161).
- **D1166 — Hits are authorized like attempts.** A secret-declaring step hits only if the publisher may
  still use its secrets (the `ResolveSecrets` check, no resolution).
- **D1167 — Invalidation.** Scope invalidation bumps a dead-before marker (O(1)); key invalidation deletes
  one entry; purge runs in bounded batches; past runs never change.
- **D1168 — Keyed digests.** Business keys are often personal data; a plain SHA-256 visible to run readers
  is an offline oracle. HMAC-SHA-256 under a 32-byte flow-control key (namespace-separated, control plane
  and engine only, `ring::hmac` as in `managed_secrets.rs`); listings show a 12-hex label. A new key version is not activated while any semaphore holder or waiter
  exists under the old one (G1163); old cache entries simply miss and expire.
- **D1169 — One run result.** `earlyReturn` writes the existing result; at most one of `earlyReturn` or
  `end.outputExpr` sets it; status unchanged. *Rejected:* a second "early result" with its own disclosure.
- **D1170 — Placement.** Root only; not in saga bodies (a success answer before the saga outcome), parallel
  branches (racing returns), bodies, handler regions or with `continueAsNew` (segment vs chain result).
- **D1171 — Parents wait for terminal children.** Completing a `child` node on an early result interacts
  with `cancelPolicy`; deferred (G1162).
- **D1172 — Visibility reuses surfaces.** `resultSource`, `resultAvailableAt`, existing disclosure, journal
  and SSE, payload-free `run.result_returned` event; run-and-wait is 026.
- **D1173 — Handler phase.** Between the end of the stop (compensation first) and the terminal
  transaction; the run stays live with a recorded pending status; outcome never changes (O1161); none for
  `CANCELLED`. The child form is a derived child node `onFailure~h` (ADR-0010 `~` precedent), so child
  lease, cancellation, tree budget and shard rules apply unchanged.
- **D1174 — Region and deadline.** Closed node set without waits; own `timeoutMs` (default 5 min, max 1 h)
  from handler start, independent of an expired run deadline.
- **D1175 — Namespace handler obligation.** The dispatcher admits the run and marks the obligation in one
  PostgreSQL transaction: exactly one handler run per failure; the handler's own effects stay at least once.
- **D1176 — Loop prevention.** Handler runs (dispatched, or `onFailure` children whose failure is covered by
  the parent's dispatch), test-tree runs and cancelled runs create no dispatch.
- **D1177 — Handler run identity.** Initiator `system:engine`; publisher authority as for triggered runs;
  immutable provenance `handlerFor` with the ADR-0039 identity guard before admission.
- **D1178 — Run permits.** Acquired at admission, granted by the waiter's shard owner (ADR-0056); queued
  runs are `WAITING` with wait kind `CONCURRENCY` and a timer; no new status.
- **D1179 — Step permits.** Acquired in the claim after activity-quota locks; ready steps are the queue.
- **D1180 — Key errors end the run.** Every start source behaves alike: the run exists and ends
  `FAILED`/`EXPRESSION_ERROR` in its admission transaction; no inbox event retries a deterministic error.
- **D1181 — Groups.** Default group derived from the workflow (run) or workflow and node (step); explicit
  groups shared by name per level; permit scope is part of the identity; the acquirer's limit applies.
- **D1182 — Queued runs are active runs, with their own ceiling.** They count against active-run quotas and
  a separate namespace `queued_runs` ceiling (default 1,000, hard 10,000), so many keys with long queues
  cannot exhaust the namespace (review S4); per-key queue ceiling; `activity_concurrency` still applies.
- **D1183 — Standalone generalizes ad hoc.** One endpoint for activity, script and database nodes; result
  and carrier from the node; origin `STANDALONE_STEP` added to the existing enumeration;
  `database-executions` kept on the same code.
- **D1184 — Standalone permission.** New `step:standalone` (admin, developer, like `database:adhoc`);
  script nodes also need `script:read`; database nodes keep 017's permissions.
- **D1185 — Draft test runs.** Private definition at an explicit revision, origin `DRAFT_TEST`, publisher the
  actor, refused in `prod` (O1160), no deliveries, triggers or namespace handler, children marked test tree;
  `definition:write` + `run:start` (FR-005-005 precedent). Test trees acquire permits in a separate
  `DRAFT_TEST` semaphore pool, and 018 refuses them as reset, rerun or batch sources
  (`RESET_SOURCE_TEST_RUN`, coordinator decision of 2026-10-27).
- **D1186 — Pins never enter a definition.** Request data retained with the run, journaled `step.mocked`;
  mockable kinds activity, script, child, waitEvent; excluded from compensation planning.
- **D1187 — Pin from a run.** Same tenant, namespace, workflow; inline success; disclosure check under the
  source carrier at pin time; strictest level becomes a floor in the test run; no taint tracking beyond
  (016 D283).
- **D1188 — Test runs and cache.** Version scope on the private draft version; shared scopes bypassed so
  tests cannot poison production entries.
- **D1189 — Test secrets.** Actor's current `secret:use` and access lists, non-`prod` only (O1162).
- **D1190 — Catalog.** `cache:invalidate` (admin, developer), `failure-handler:manage` (admin),
  `step:standalone` (admin, developer); 51 entries; roles unchanged (custom roles are 029).
- **D1191 — Migration identity.** Allocated at implementation after `20261220000000` and coordinated with
  the other 018–032 drafts; no number reserved here.
- **D1192 — No new error codes.** Only `details.reason`, `DefinitionProblem` and failure codes
  (`contracts/errors.md`).
- **D1193 — Replay-only Temporal concepts do not apply.** Local activities, sticky queues, patching and
  side-effect markers exist for code replay; the interpreter journals decisions instead (ADR-0003).
- **D1194 — Encrypted fields (031).** This feature's positions and write paths are entries of 031's
  encoded-field registry (`specs/031-client-side-encryption/contracts/workflow-encoding.md`): cache keys
  (`ENCODING_READ_BY_CACHE_KEY`), concurrency keys (`ENCODING_READ_BY_CONCURRENCY_KEY`, registered by 031),
  `earlyReturn.result` as a carriage position (`ENCODING_CARRIAGE_MISMATCH`), and mocks, pins, standalone
  inputs and cache copies as write paths. The earlier own code `KEY_READS_ENCRYPTED_FIELD` is withdrawn
  (coordinator decision C7); encoded values pass as opaque bytes.
- **D1195 — Failure context is protected metadata.** Closed codes, node, attempt, kind, time; worker
  `safe_message` excluded; a child handler's input is classified by its `inputSchema`.
- **D1196 — Grants skip waiters their own limit cannot admit.** With per-acquirer limits, strict
  oldest-first lets an older `limit: 1` waiter block a younger `limit: 5` one (review S3). A grant goes to
  the oldest unpaused waiter whose own limit admits it; a skipped waiter can starve until its queue
  timeout. *Rejected:* one fixed limit per key (a busy key would never adopt a new version's limit).
- **D1197 — No workflow handler without a permit.** A run that ends before it ever acquired its run permit
  (queue timeout, key error at admission) finishes directly; its workflow handler would otherwise run
  outside the key limit (review S5). Only the namespace handler, an independent run admitted through the
  permit port, may follow.
- **D1198 — One permit port for every run creator.** ADR-0056's invariant requires that the start API,
  triggers, children, reruns, 017 ad hoc, 022 standalone/test/dispatch and the 018 schedule, batch, reset
  and Signal/Update-with-Start writers all call it (plan, Run creators). A reset acquires a fresh permit at
  reset admission and never queues (refused when none is free); copied `concurrency.*` events are marked
  `restored` and ignored by the invariant (review 018 B1, 022 S6). A start delay acquires at release.
- **D1199 — Erasure reaches every copy.** Cache entries are inline copies with no FK to their source and
  hits copy them into other runs (review 020 S8). Entries and obligations are deleted by `source_run_id`
  when 020 erases a run; hit provenance and pins record the source so 020 can reach consumer copies. The
  non-FK copies (entries, hit provenance) are registered in 020's content-store registry
  `crates/infra-postgres/run-content-references.json` with the hook "invalidate by `source_run_id`";
  obligations and pins reference payloads by foreign key, which 020 discovers from the catalog
  (coordinator decision C8).
- **D1600 — Reset rules for 022 events.** Each added event type declares its rule in 018's registry
  (coordinator decision C1; table in `contracts/journal-events.md`): permit events, `result.returned`,
  `step.cache_hit` and `run.started` provenance are `REMATERIALIZE` (fresh permit or refusal, result
  columns, provenance row, no inherited `handler_for`); `step.mocked`, `failure_handler.*`,
  `concurrency.wait_timed_out` and `concurrency.dequeued` are `REFUSE`, so no reset point lies inside a
  failure-handler phase, after a queue timeout or between `concurrency.queued` and its acquisition. Admission
  semaphores take slot 7 of ADR-0050's merged lock order (C4).
- **D1601 — Unmocked effect nodes refuse a draft test.** Nodes of 021 `resourceOperation`, 024 data, 025
  agent and 030 `serviceCall` reachable from `startAt` must be pinned or mocked, else the request is refused
  with `TEST_RUN_UNMOCKED_EFFECT` (coordinator decision C18); this feature owns the rule for other features'
  kinds. 017 database nodes keep executing against the namespace's (non-`prod`) connections.
- **D1602 — Priority order with 019.** When 019 is present, grants follow (019 priority 1 first, enqueue time
  ascending) among the waiters D1196 admits (coordinator decision C19); without 019 the order is enqueue
  time. A start delay (018) acquires at release (D1198).

### Decisions of the release 0.3.0 slice (2026-11-13)

The numeric block of this feature (D1160–D1199) and its overflow (D1600–D1602) are used up, and the free
overflow numbers are scattered among other features, so the decisions below take feature-scoped ids
`D022-NNN`, the form feature 035 uses. Gaps and owner questions continue the numeric block (G1174…,
O1163…), which still has room. Nothing above is renumbered.

- **D022-001 — The slice.** Release 0.3.0 takes US02 as specified and the new US07 (rate limits with
  buffering), by OD-2026-11-13-11. The foundational tasks are cut to what these two need (plan, Release
  0.3.0 slice); the other stories keep their tasks untouched. *Rejected:* building rate limits as a variant
  of the concurrency semaphore (a slot is freed by time, not by a holder; the release event that drives
  ADR-0056's grant does not exist).
- **D022-002 — Declaration shape.** `rateLimit: {limit, per, key?, group?, queueTimeoutMs?}` at top level
  (run starts) and on root `activity`/`script` nodes (attempt dispatches), beside `concurrency`. `per` is
  the closed set `second` | `minute` | `hour`. *Rejected:* a free `periodMs` (the retained window and the
  estimate would have no natural bound, and "per 37 seconds" has no user); nesting it in `concurrency`
  (two lifetimes in one object; either may be declared alone); a `burst` field (it belongs to a token
  bucket, which D022-003 does not choose; the owner confirmed no `burst` in O1163).
- **D022-003 — The window is a durable sliding window of grants.** A key admits a grant when fewer than
  `limit` of its grants still count within the last `per` on the database clock; the grants that count
  are rows. It is strict (never more than `limit` in any window), lets the first `limit` start at once,
  and does not depend on how often the scheduler runs. *Rejected:* a token bucket (the platform's
  `quota_rate_buckets` pattern): grants to buffered work happen in a scheduler pass (500 ms by default,
  `SAGAT_SCHEDULER_INTERVAL_MS`), so a strict bucket of capacity 1 delivers at most two grants a second
  per key whatever the declared rate, and a bucket large enough to keep up admits up to `limit + burst`
  in a window; a fixed window (twice the limit across a boundary, and every waiter granted at the
  boundary); an in-process limiter (lost on restart, one per replica). Proposed as ADR-0088; the owner
  chose this meaning in O1163 (OD-2026-11-13-16).
- **D022-004 — The scheduler grants; the claim never does.** A slot becomes free because time passed, so
  something must wake without an event; that is a scheduler pass under the waiter's shard fence (ADR-0020,
  as ADR-0056 decision 5). A grant at claim time would add a row lock and a count to the claim and make
  every poller probe steps it cannot lease, which design 038 measured as the main waste of the hot path.
  When the window is free and nobody waits, the admission or readiness transaction grants at once, so an
  uncontended key pays no pass latency.
- **D022-005 — A step's slot counts from its lease.** A granted step is claimable but may sit unleased
  while workers are busy or down; counting from the grant would let a returning worker lease a backlog at
  once. A grant counts while unleased and for `per` after the lease instant, which the pass copies from
  the `step.leased` journal fact. Then any window holds at most `limit` leases: at the grant of the last
  of them every other one was either unleased or leased within the window, so it counted. The pass may
  record a lease late; that only keeps a slot longer.
- **D022-006 — Permit and slot together.** With both declared at run level a run gets both in one
  transaction or waits for both, once: one deadline, and its two waiter rows leave together. Holding a permit while waiting for a
  slot would waste the scarcer resource; spending a slot while waiting for a permit would let granted
  starts pile up and begin together. Lock order: semaphore (ADR-0050 slot 7), then the rate-limit rows as
  rate rows of slot 8b, sorted, before usage rows; ADR-0056 decision 2 already allows rate rows after a
  semaphore.
- **D022-007 — The buffer.** First in, first out within a key by the enqueuing commit; a waiter its own
  limit or its other gate refuses is skipped, not waited for (as D1196); no order between keys. Bounded by
  the per-key waiter ceiling and the namespace `queued_runs` ceiling shared with concurrency waiters; a
  full buffer refuses a synchronous start and leaves event and cron starts pending (as D1182). *Rejected:*
  an unbounded buffer (a stuck key would hold the namespace's active-run quota); dropping the oldest
  waiter (it loses admitted work).
- **D022-008 — Every attempt takes a slot.** The downstream system is called again on a retry, so a retry
  asks for a new slot when its backoff ends; the wait is not an attempt (O1166, answered yes).
- **D022-009 — Each acquirer applies its own limit to the shared grants.** Definitions sharing a group may
  declare different `limit` and `per`; each counts the key's grants over its own `per` against its own
  `limit` (as D1181, and possible because the grants are rows with instants). Publication warns
  `RATE_LIMIT_MISMATCH`. A namespace policy ceiling clamps every acquirer at its next evaluation; a new
  version's lower limit binds new runs only. No grant is revoked.
- **D022-010 — A reset takes a slot or is refused.** As D1198 for permits: a reset reproduces a run that
  was already admitted once, so it never waits; without a free slot it is refused
  (`RATE_LIMIT_SLOT_UNAVAILABLE`, to be registered by 018). `rate_limit.queued` and `rate_limit.granted`
  are `REMATERIALIZE`, `rate_limit.wait_timed_out` and `rate_limit.dequeued` are `REFUSE` in 018's
  registry (`contracts/journal-events.md`). Confirmed by the owner (O1168).
- **D022-011 — Root graph only, and not with `continueAsNew`.** A body node declaring `rateLimit` is
  refused (`SCOPED_RATE_LIMIT`, G1175) for the reason of G1160: body instances become ready in 014's
  exact-path transactions, which need their own evidence. `continueAsNew` is refused as for `concurrency`
  (G1177). A body `child` node starting a rate-limited workflow is the supported fan-out shape.
- **D022-012 — One more gate, no new surface kinds.** Capability `rateLimits`, enabled separately from
  `concurrencyKeys`; wait kind `RATE_LIMIT` for runs and steps; failure code `RATE_LIMIT_WAIT_TIMEOUT`;
  reasons `RATE_LIMIT_QUEUE_FULL` and `RATE_LIMIT_SLOT_UNAVAILABLE`. No run or step status, no
  `Error.code`, no worker-protocol minor: a waiting step is simply not offered to `PollTasks`.
- **D022-013 — Rows of its own, not the tenant quota buckets.** `quota_rate_buckets` (feature 009) are
  token buckets over a closed resource set that refuse a request; they stay usage rows locked last. Rate
  limits get their own window, grant and waiter rows; they reuse the database-clock read of that code,
  not its table.
- **D022-014 — Position is exact, the grant time is an estimate.** The position is the waiter's rank in
  its key at read time. The estimated grant time is computed at read time from the instants at which the
  counted grants age out; it is absent when a slot ahead is held by an unleased step or the position is
  beyond one window. Neither is journaled.
- **D022-015 — A pass is bounded per key and per tenant.** One key gets at most a fixed number of grants
  per pass and tenants are visited in rotation, so a flood cannot starve another key's eligible waiter
  for more than one further pass. The price is a throughput ceiling per key (G1174).
- **D022-016 — Six SDKs.** This feature was written before the Rust SDK (ADR-0070); the slice adds the
  Rust builders and runs the conformance scenarios in six languages (T022-067, T022-083, T022-084).
- **D022-017 — Timers and passes.** A queue deadline is an ordinary timer on the run or step, handled by
  the existing timer pass (as T12). Grants and lease recording are one new pass in
  `crates/scheduler/src/flow_control.rs` on the scheduler tick; no new process and no new hint is needed
  for correctness (a hint only shortens the wait).
- **D022-018 — Composition with stream operators (design 042).** An operator chain on a trigger decides
  which events ask for a run and with what input; this feature decides when a requested run may start
  (042 FR-042-022). No operator paces a start and nothing here gathers events: 042's `throttle` row
  points to `rateLimit`, and `debounce` or `window` reduce how many starts are asked for before
  admission sees them. A sealed window's start request is an event-trigger start through the one
  admission port. When it is buffered a run exists (`WAITING`) under the window's key, so the window
  stays sealed and a repeated request finds that run. A full buffer is not a refusal for good: like any
  event start the request stays pending and is retried, and the window must not become `REFUSED` for it.
  A key error creates the run and ends it `FAILED`/`EXPRESSION_ERROR` (D1180); the window started its
  one run. A `forEach` with operators is still a collection: `rateLimit` on its body nodes stays refused
  (G1175). Nothing in the slice contradicts 042; T042-3 on that side cites this entry.

Numbers taken on 2026-11-13 after listing `docs/adr` on `origin/develop`, every `origin/*` branch and the
sibling worktrees (first as ADR-0084, renumbered by the coordinator because designs 041 and 042 took
ADR-0084..0087 the same day): ADR-0088 (ADR-0081..0083 are on `origin/feature/018-business-ids`); T022-066..090;
FR-022-039..050; NFR-022-007..008; SC-022-017..023; G1174..1183; O1163..1168.

### Decisions after the reviews of 2026-11-13

Three independent reviews (architecture, security, correctness) read the slice at `1a21cbdc`; the records
are in `reviews/2026-11-13/`. Each code fact cited below was checked on `origin/develop` at `a4f6e9db`.

- **D022-019 — A gated step waits outside `READY`; the claim never sees it.** The claim's candidate query
  (`crates/infra-postgres/src/workers.rs`, `CLAIMABLE` and `CLAIMABLE_PATHS`) takes the oldest `READY`
  steps of each activity, at most `limit.min(64)`. A step kept `READY` without a permit (FR-022-019 as first
  written, ADR-0056 decision 3) sits in that window for every poll: 64 such steps of key A hide every step
  of key B of the same activity and undo FR-038-002. So a step with node-level `concurrency` waits like one
  with a rate limit (`WAITING`, wait kind `CONCURRENCY`) and the grant routine makes it `READY` already
  holding its permit; the permit is held from grant to attempt end, so granted, unleased steps count as
  holders and cannot be hoarded (correctness M3). This changes an accepted ADR, so it is proposed as
  ADR-0089. *Rejected:* a filter in the claim query (a per-candidate semaphore probe in the hot path);
  dropping node-level concurrency from 0.3.0 (kept as the fallback of O1169).
- **D022-020 — The gate is in the reducer, and one adapter locks the keys.** Steps become `READY` in many
  places: the planner emits `step.released` inside every `settle` of `crates/domain/src/execution/command.rs`
  (worker reports, timers, signals, approvals, updates, child completions), the retry pass emits
  `step.retry_due` (`due_retries` in `crates/infra-postgres/src/maintenance.rs`), an operator emits
  `step.retry_requested` (`state_effects.rs`, which sets `Ready`). Guarding each caller would miss the next
  one, so the reducer maps those three events to `WAITING` for a gated node and only a grant makes it
  `READY` (fail-safe). `persist` (`crates/infra-postgres/src/execution.rs`), the one place that writes every
  transition and already writes the task-ready hint for exactly those three events, collects the gate keys
  after the reducer, locks them once sorted and journals grant or queued for each; it writes no hint for a
  step left waiting (correctness N5).
- **D022-021 — One grant routine, driven by key, with a reservation rule.** ADR-0056 decision 5 (grant by
  the waiter's shard owner, strict oldest first) and D022-007 (FIFO with skip-ahead) are merged: a pass
  selects keys through a partial index on (key shard, next eligible instant) and the scheduler's fair
  selection per tenant (`crates/scheduler/src/fairness.rs`), takes the key's home-shard fence, and grants
  the waiters it can in order, each in its own transaction that checks every gate of the waiter. A waiter
  at eligible position `p` is granted when counted + `p` is below its limit: the waiters ahead hold a
  reservation, so a head refused only by its other gate is not starved (correctness M2), and grants of one
  key need not wait for the head's shard, so throughput is not bounded by about two grants per second per
  replica (security M1, architecture M2, correctness M1). The next-eligible instant is a hint kept under the
  key's lock that may be early and never late; lease recording therefore needs no window lock
  (architecture M9).
- **D022-022 — One timeout routine.** A waiter has one deadline (the earlier of its gates'); at it, the
  routine journals the refusing gate's `wait_timed_out` and the other gate's `dequeued`, rate first when
  both refuse (correctness B2, M8). A waiter whose run is terminal is a breach: alert, skip, never a head.
- **D022-023 — Fleet activation through 018's registry.** ADR-0050 decision 9 requires a feature that every
  process must honour to be activated through the instance-capability registry (018 `data-model.md`,
  T018-080, ADR-0082 accepted). An engine without the gate code would make gated steps `READY` on its own
  paths (architecture B2). Both capabilities are instance capabilities; until activation, use is refused
  with the feature's own reason (the registry's rule 4: 018 uses `SCHEDULE_TRIGGER_HANDOVER_PENDING`, 029
  `ACCESS_CAPABILITY_DISABLED`), here `FLOW_CONTROL_ACTIVATION_PENDING`; `BINARY_CAPABILITY_MISSING` is what a
  process lacking a recorded requirement answers. The coordinator's message asked for publication and start
  to be refused `BINARY_CAPABILITY_MISSING`; the registry's rules say otherwise, and the text follows the
  registry. The registry is on `feature/018-business-ids` (pull request 98, open at this writing): it is a
  prerequisite of the slice. The digest key version becomes a database row (architecture M10).
- **D022-024 — Queue bounds.** Per-key run waiters default to 100, below the namespace `queued_runs`
  (default 1,000), so one key cannot fill the namespace (security M2). Step waiters have no per-key bound:
  at most one per run and gated node, so `active_runs` bounds them (correctness M10). A child node meeting a
  full buffer fails with a typed, retryable code through its retry policy (correctness M9): child creation
  has no pending inbox to keep it in, and a routable failure is visible.
- **D022-025 — Keys: cardinality, retirement, rotation, classification, failure.** A key that reads only
  the run's identity makes one key per run, so publication refuses it (security M3). The purge keeps a grant
  for the longest period the window row has seen plus one minute (security M3, correctness M5). Rotation is
  per key: both digests are computed during a rotation and the existing row wins (security M4). A key
  reading a `secret` location is refused, `pii` warned, and labels are shown only to callers who may
  disclose the key's source (security M6). A step key that fails fails the step through its route without
  an attempt (security M5): failing open would run it ungated.
- **D022-026 — Counters are sharded usage rows.** `queued_runs` and live keys are kept in 16 slot rows per
  namespace, like `run_active_counts` (migration `20261017000100`), maintained by `SECURITY DEFINER`
  triggers, locked last in slot 8b (architecture M6). A dual-gated run counts once (correctness N4).
- **D022-027 — Time.** The instant is `clock_timestamp()` read after the window lock and is the recorded
  `granted_at` (correctness M5): a later decider of the same key waits for the lock and reads a later
  instant. The skew between decision and commit is G1184. A set-back clock is tested with `libfaketime` in
  the PostgreSQL test container, not with a clock port (the port would test the port).
- **D022-028 — Returning slots and permits.** A granted step that leaves `READY` other than by a lease
  (pause, review, cancellation, run end) returns both in that transaction (`rate_limit.returned`,
  `concurrency.released`) and asks again on resume or retry (architecture M8, correctness M6); this bounds
  granted, unleased steps by `limit` at every instant.
- **D022-029 — A step grant names its attempt.** The grant is for attempt `attempt() + 1`; lease
  recording accepts only a `step.leased` of that attempt committed after the grant event (correctness M7).
- **D022-030 — Hot-path checks come first.** The budget gains a line for a plain admission before any
  admission change (Phase 2); the selector (`scripts/select-test-targets.py`, whose budget rule today
  covers `crates/infra-postgres/`, the worker and execution application files and `runs.rs`) also selects
  the budget suite for `crates/scheduler/`, `crates/domain/src/execution/` and the new application files
  (architecture M4).
- **D022-031 — The admission port is enforced by a check.** Today `INSERT INTO runs` appears in
  `PgRunStore::create` (`execution.rs`) and twice in `chain_progress.rs` (continue-as-new successors); the
  gate goes inside `create`, the successors assert their definition has no run-level gate, and
  `scripts/check-architecture.py` refuses a new insert (architecture M5). Schedule firing is on `develop`
  (`crates/application/src/schedule_firing.rs`, `RunService` "starts the run of a schedule's action"), so its
  case is in the slice now; reset, batch rerun, with-start and the registry are still on 018 branches
  (G1181 corrected).
- **D022-032 — Invariant 10 per grant.** For each grant `e` with instant `t_e`, the grants of its key with
  instant in `(t_e − period_e, t_e]` number at most `limit_e` (architecture M7, correctness M4): half-open,
  and per acquirer, which is what the decision checked.
- **D022-033 — Each invariant lands with its first writer** (architecture M11): invariants 1–2 with
  T022-022, 8–12 with T022-072 and T022-096.
- **D022-034 — No re-authorization at grant**, stated and tested (security nit).

Numbers taken after the reviews, after listing `docs/adr` on every `origin/*` branch and the sibling
worktrees (041 holds ADR-0084/0085, 042 ADR-0086/0087): ADR-0089; FR-022-051..060; SC-022-024..029;
T022-091..107; D022-019..034; G1184..1186; O1169.

### Decisions of the foundation build (2026-11-15)

Taken while building the first part of the 0.3.0 slice (T022-002..014 narrowed, T022-068..070, T022-091..094). Numbers
taken after `git grep D022-` on every `origin/*` branch and the sibling worktrees.

- **D022-035 — Until a reset admits through the gate, a flow-control event blocks every reset point after it.** The
  rules of `contracts/journal-events.md` (`REMATERIALIZE` for `concurrency.queued/acquired/released`,
  `rate_limit.queued/granted/returned`; `REFUSE` for the timeouts and dequeues) need the reset admission to take a fresh
  permit or slot through the one port (T022-064, T022-086). That code does not exist in this build, so 018's registry
  (`crates/domain/src/execution/reset.rs`) declares the ten event types `EXCLUDE`: no reset run can start holding a
  permit or a slot it was never granted. The contract's rules replace these in the change that builds the reset case.
  No journal holds these events yet (a gate cannot be activated in this build, D022-037), so the change is free.
- **D022-036 — Every flow-control event names its scope, and a step-scope one its attempt.** The detail of every one of
  the ten events carries `pool`, `scope`, `group` and `key_label`, and at step scope `attempt` (the node is the row's own
  column). `contracts/journal-events.md` lists `concurrency.dequeued` without `scope`; a step can be dequeued since
  ADR-0089, so the scope is needed to replay it. The detail is closed: the codec reads back only the keys it wrote.
- **D022-037 — A binary announces a flow-control capability only once it runs the gate.** *(Amended by D022-057: the concurrency gate is listed and admitted by a process with its switch and the digest keys; the rate gate stays unlisted until T022-072 and T022-074.)* `flow-control-concurrency-v1`
  and `flow-control-rate-v1` mean "this process enforces the gate on every path it serves" (ADR-0089 decision 6). This
  build has the declarations, the events, the tables and the activation, but no admission gate, readiness gate or grant
  routine (T022-022, T022-072, T022-096..098), so the control plane and the engine do not list either capability.
  Activation is therefore refused while any instance is live (`INSTANCES_LACK_CAPABILITY`); publication and start of a
  gated definition are refused `FLOW_CONTROL_ACTIVATION_PENDING` before activation; and `PgRunStore::create`, the one
  admission point (FR-022-059), refuses a gated definition with `BINARY_CAPABILITY_MISSING` should a requirement ever be
  recorded with no live instance. Fail closed at every layer; the change that builds the gate adds the capability.

- **D022-038 — What later writers of the flow-control rows must keep (review of 2026-11-15).** A transaction that
  changes a key's holders, waiters or grants locks the key row (semaphore or window) `FOR UPDATE` first, in
  `(pool, scope, group, key)` order, semaphores before windows, and only then inserts or deletes the child rows: the
  counter triggers update the key row after the child row, and a writer that skipped the key lock could deadlock against
  one that took it. An emptied key row is deleted at commit by a deferred constraint trigger, so a release followed by the
  grant of the same key in one transaction keeps it. The namespace counters of T022-105 (`live_keys`, `queued_runs`) must
  be written by deferred triggers too, so that the slot-8b usage rows stay last (ADR-0050 decision 7). Applies to
  T022-022, T022-072, T022-097, T022-098, T022-105.
- **D022-039 — `grants_total` counts one life of a window row.** A window row is deleted when its last grant and waiter
  go, and a later grant creates a new one; invariant 9 ("grows by exactly one per `rate_limit.granted`") is checked per
  life of the row (T022-079), not across a purge.

### Decisions of the second build (2026-11-16)

Taken while finishing the foundation's leftovers and starting the gate (T022-002, T022-013, T022-069, T022-095..). Numbers
taken after `git grep D022-04` on every `origin/*` branch.

- **D022-040 — The quota resources of the concurrency gate.** T022-013 adds six `QuotaResource` values, all counted or
  set per namespace (OD-2026-11-16-2 for the OpenAPI enum): counts `queued_runs` (1,000, hard 10,000; a run waiting at
  both gates counts once), `concurrency_live_keys` (10,000, hard 100,000; semaphores and windows) and
  `flow_control_key_waiters` (100, hard 10,000; run waiters of one key, D022-024; like `activity_concurrency`, its
  scalar usage is the busiest key's); positive policies `concurrency_limit` (10,000/10,000: the most holders of one
  key whatever a definition declares), `flow_control_queue_timeout_ms` (one day/one day) and
  `flow_control_node_declarations` (16, hard 64: node-level declarations of one kind per definition). The effective value
  of a declaration is the lower of the declaration and the resolved namespace value, which already carries the
  deployment's ceiling (`FlowControlCeilings`, FR-022-034). A count may be zero (it refuses every new waiter or key);
  a policy may not (nothing could be declared under it). Until the sharded counters of T022-105 exist, the quota view
  reads the counts from the waiter and key rows. The node-declarations policy is enforced at publication now, under the
  code of its kind, as the domain's own bound of 64 is; the others are consumed by the gate (T022-022, T022-072). The
  rate-limit rows of the ceilings table (per-window limits) stay with T022-077. The canonical OpenAPI baseline of
  `contract_iceberg_writes` digests each operation without following `$ref`, so the enum change moves no pinned digest:
  its bless diff was read (18 additions, all operations of 018 and 035 that the baseline deliberately leaves out; no
  digest changed or removed) and not applied.
- **D022-041 — Namespace warnings read a table of declared groups.** `CONCURRENCY_LIMIT_MISMATCH` and
  `RATE_LIMIT_MISMATCH` (G1187) need the declarations of the other definitions of the namespace. Parsing every other
  document at each publication is unbounded in bytes, so each published version writes its explicit groups to
  `flow_control_group_declarations` in its publication transaction (immutable, like the version). Publication and
  validation compare against the *active* version of every other non-internal workflow of the namespace, at the same
  level (run or step) and gate, one warning per own declaration naming the first such workflow; a derived group is never
  shared and never listed. The read is outside the publication's transaction and bounded (1,000 rows): a racing
  publication may miss a warning, which never refuses. The store method has no default, so a delegating store cannot
  silence it (the first run of the test found `PgRunStore` doing exactly that).

- **D022-042 — Per-key rotation of the digest key (T022-095, FR-022-056).** Activating a new version no longer waits for
  the queues to drain: the active version becomes `RETIRING` while a semaphore or window row uses it (`RETIRED` at once
  when none does), and the transaction that deletes its last row retires it (a trigger that locks the version row first,
  so two transactions deleting the last two rows cannot both miss it). A deciding transaction reads both versions
  (`flow_control_key_versions_in_force()`: the active one `FOR SHARE`, the retiring one without a lock, since it can only
  retire), digests the key under each (`KeyRing::candidates`) and files it with `file_keys`: the row under the retiring
  version wins, else the key is filed under the active one. A key is never split: no row is created under a retiring
  version (the trigger of `20270130000000`) and an old row, once gone, never returns. Keys of one table are resolved
  first and then locked in one order, by the digest each resolved to, so a rotation does not change the lock order of a
  key against release paths that lock it by its stored digest. One rotation at a time (`ROTATION_IN_PROGRESS` while the
  retiring version is used): a decider computes two digests, never three. A replaced version is never active again, like
  a retired one. The command records the database role that ran it (`activated_role`, G1190). The activation no longer
  takes the key tables in `SHARE` mode: no row can be created under a non-active version, so its count of the retiring
  version's rows can only be too high (a refusal the operator repeats), and the deadlock between it and a release path
  is gone. Each deciding transaction of T022-096..098 and T022-022/T022-072 must call these two functions; until they
  exist no transaction decides a key (D022-037).

- **D022-043 — What the reviews of 2026-11-16 changed** (`reviews/2026-11-16/`).
  - *Retirement is lazy* (correctness F2, architecture M1): D022-042's trigger on the last row's delete is dropped; it
    serialized every key deletion of a rotation on one row and counted the retiring rows on each. A retiring version is out
    of force the moment no row uses it (`flow_control_key_versions_in_force()` answers NULL for it) and is recorded
    `RETIRED` by the next activation. At most one version is `RETIRING` (a unique index), and the migration retires what the
    earlier command left `RETIRING` (security S2, correctness F3).
  - *A vanished retiring row is a retryable refusal* (correctness F1, architecture M3): filing it under the active digest
    at that point would lock it out of the one order; `file_keys` refuses with `KEY_ROW_VANISHED` and the deciding
    transaction runs again. Only `flow_control.rs` inserts a key row (`scripts/check-architecture.py`).
  - *The per-key rotation diverges from ADR-0056 decision 7* (architecture B1): ADR-0093 (proposed) records it; T022-095 is
    not ticked until the owner accepts it, and O1170 asks what ends a rotation that live rows keep open (security S1,
    architecture M2).
  - *Live keys are semaphore keys* (architecture m4): the plan has separate ceilings for semaphore keys and rate-limit
    keys; `flow_control_live_keys` is renamed `concurrency_live_keys` and counts the semaphores; the windows get theirs with
    T022-077.
  - *One key's run waiters stay below `queued_runs`* (correctness F5, architecture m5): `effective_key_waiters` clamps the
    key ceiling below the namespace's, whatever an administrator sets; a run already counted in `queued_runs` (it waits at
    its other gate) adds nothing to it (correctness F9).
  - *Declarations name their namespace's workflow* (security S3): the foreign keys are `(tenant, namespace, workflow)` and
    `(tenant, workflow, version)`, the read joins on the namespace too, and the index leads with the group.
  - *A warning never fails a publication* (correctness F6): a failed read of the declarations gives no warning (logged);
    the node-declarations refusal of a request asked again with its own key is answered by the replay (as D042-105).
  - *The node-declarations ceiling is a publication-time policy* (security S5, architecture M4): like the `workflows`
    quota, its limit is read by the application before the store's transaction; lowering it never revokes what is
    published, and re-activating an older version is not judged again (a lowered limit does not revoke, FR-022-021).
  - *Fitness functions* (architecture m2): the OpenAPI enums and the newest restatement of the ceilings tables' `CHECK`
    lists are pinned to `QuotaResource::ALL` (`contract_flow_control_step_policies::quotas`); the quota maxima of the
    declaration bounds are the domain's constants.
  - *Sort order* (correctness F10): `file_keys` sorts by byte order; an SQL path that locks the same rows sorts
    `group_name COLLATE "C"` (D022-038).
- **D022-044 — How the readiness gate is built (T022-096, 2026-11-19).** D022-020 places the gate in the reducer and one
  adapter call; the build keeps both and says where each half lives.
  - *The reducer knows the gated nodes of its run* (`RunState::with_gated_steps`, given by every transaction that folds
    or restores a run of a definition): `step.released`, `step.retry_due` and `step.retry_requested` of such a node leave
    it `WAITING` (`WaitingOn::Gate`), and only the grant that completes every gate the node declares makes it `READY`; a
    lease of it without them is refused. The journal alone still folds (a state rebuilt without the gated nodes sees the
    step `READY` for one commit and reaches the same state through the decisions that commit journals); `verify_run`
    checks both foldings agree and invariant 11 (`RunState::flow_control_breach`).
  - *The decision is an evaluation round, not a step of `persist`*: the planner, once it has nothing else to emit, asks
    `Evaluation::Gates` for every step that waits without a decision; the application evaluates the keys (a key that
    fails or exceeds 256 bytes fails its step `FLOW_CONTROL_KEY_INVALID` through its route, and that round locks
    nothing) and the adapter (`PgRunTx::decide_gates`) files and locks every key once, semaphores then windows, sorted,
    before `persist` locks any usage row (slot 8b); `persist` writes no task-ready hint for a step left waiting. Doing it
    inside `persist` would have locked the keys after the usage rows `persist` charges first, and could not route a key
    failure. `persist` gives back what the planner releases (`concurrency.released`, `rate_limit.returned`, a stopped
    run's `*.dequeued`) before it charges anything, for the same order.
  - *A new arrival is granted only behind the queue*: a gate admits at once when its holders (or counted grants) plus
    the waiters already queued are below the acquirer's own limit (the reservation rule of D022-021 for the last
    position); both or neither (FR-022-044), with the earlier deadline of the two.
  - *The claim is untouched*: the lease instant of a rate-limited step's grant is recorded by the pass (T23, T022-074);
    until then the grant counts as unleased, which never admits more than the limit. The claim of a rate-limited step
    costs what the plain claim costs (T022-076).
  - *The step projection*: `steps.flow_control_gates` (bounded `jsonb`: phase, attempt, pool, group, the key's label and a
    wait's deadline; never a value) and `waiting_kind` `RATE_LIMIT` / `CONCURRENCY` (migration `20270130000600`), so a
    transaction restores a gated step as replay folds it (closes the step half of G1188).
  - *Reviews of 2026-11-19* (`reviews/2026-11-18/`): what the transaction gives back is locked in the same sorted pass as
    what it decides and given back first (a release and a decision on one key are counted once, and no key row is locked
    out of order, correctness 1 and architecture M2); a paused run's retry is decided in its commit (correctness 3); the
    answer may fail only an asked step and only `FLOW_CONTROL_KEY_INVALID` (architecture m1); the collection- and
    loop-finish paths refuse a gated successor instead of writing it `READY` (correctness 2, G1197 corrected); the
    door's fitness rule also refuses a production call and a feature of the owning package that enables it
    (architecture m2, security minor). The divergence from ADR-0089 decision 2 is ADR-0094 (proposed, O1171).
  - *Tests admit step gates through a test-only door*: `PgRunStore::admitting_step_gates_for_tests` (feature
    `step-gates-for-tests`, `[dev-dependencies]` only, a rule of `scripts/check-architecture.py`) admits a definition
    gated only at its steps; production keeps refusing every gated definition until the run-level admission, the grant
    and the timeout routines exist (D022-037).

- **D022-045 — How the grant and the timeout routines are built (T022-097, T022-098, G1199, G022-001; 2026-11-19).**
  Both rest on the readiness gate as built (ADR-0094, proposed, O1171).
  - *Grant (T26)*: a scheduler pass (`Scheduler::gate_grants`, after the retry pass) selects keys, not waiters
    (`MaintenanceStore::due_gate_keys`: step keys with waiters whose hint has come, on the home shards the replica holds,
    through `pick`'s per-tenant fairness, `batch` keys a pass), lists each key's eligible waiters oldest first (live, not
    paused, before their deadline; at most `GRANTS_PER_KEY_PER_PASS` = 256, the plan's bound) and tries each in its own
    run transaction: the run first (`SKIP LOCKED`), then the key's home-shard lease (`RunTx::holds_key_shard`, which is
    not the run's shard), then the step's keys sorted (`grant_queued_step`). It grants every gate the step waits for, or
    none, by the reservation rule, and never a waiter past its deadline; `execution::grant_step_gates` journals the grants
    (`from_queue`) and the step becomes `READY`; `persist` announces it. The order is run, fence, keys, as every pass
    (ADR-0050 decision 7), where plan T26 names the fence first.
  - *The hint (early, never late)*: a key is due when a permit is released, a slot returned or a waiter dequeued (as
    before), and now also when a waiter is queued (`key_is_due`) and when a run with queued steps pauses or resumes
    (`rearm_queued_keys`), since a paused waiter is neither granted nor counted ahead. A pass that grants nothing of a
    key, or reads all of its waiters, rests it (`rest_gate_key`: only if the hint is still the one read and the replica
    still holds the key's shard); a window rests until its oldest counted leased grant ages out (`NULL` while no grant
    has a lease instant, G022-003).
  - *Reservation*: a waiter ahead counts when it is live, not paused, before its deadline and its own limit exceeds what
    is taken; the count stops at the waiter's own limit. Conservative: it may reserve more than the rule needs, never
    less (correctness m5).
  - *Timeout (T27)*: a pass (`Scheduler::gate_timeouts`, before the grants) takes the waiters whose deadline passed on
    the run shards it holds, under the run's fence, reads which gate refuses (`refusing_gate`, rate first; read without
    the keys' locks, so a gated failure route cannot lock a key out of order) and `execution::time_out_step_gates`
    journals that gate's `wait_timed_out`, the other's `dequeued` and `step.failed` (`CONCURRENCY_WAIT_TIMEOUT` or
    `RATE_LIMIT_WAIT_TIMEOUT`, no attempt spent) through the route; the rows go with the events under the one sorted lock.
    Grant and timeout serialize on the run's lock; the first commit decides.
  - *Ceilings (G1199)*: the readiness round reads `concurrency_limit`, `flow_control_queue_timeout_ms` and
    `concurrency_live_keys` once (one statement); limit and timeout are the lower of declaration and namespace; a step
    whose key would be a new semaphore beyond the live keys, or whose effective ceiling is zero, fails
    `FLOW_CONTROL_LIMIT` through its route with nothing filed (`StepGateAnswer::refused`). Step waiters are not bounded
    per key (plan, ceilings table: one per run and gated node). The gated budget lines cost one statement more.
  - *Telemetry (G022-001)*: each decision and return of a committed transaction is counted after its commit
    (`concurrency_acquisitions_total`, `rate_limit_decisions_total`, step scope, closed outcomes); spans
    `workflow.concurrency.acquire`, `workflow.rate_limit.admit`, `workflow.concurrency.grant`, `workflow.rate_limit.grant`
    name the group, never the key. ADR-0089's "Enforced by" line (an accepted text) is answered by the suites named in
    T022-096..098.
  - *Fitness function*: only `execution/readiness_gate.rs` and `execution/gate_queue.rs` write holder, waiter and grant
    rows (`scripts/check-architecture.py`, `gate_row_write_errors`).
  - Reviews of 2026-11-19 (second set) in `reviews/2026-11-19/`, with their resolutions.
- **D022-046 — The operator's force ends a rotation at once (T022-095, T022-108; ADR-0093 decision 5; OD-2026-11-19-2;
  2026-11-19, revised after the reviews of 2026-11-20).** The owner answered O1170 (a) with (b) for incidents. The force
  is a separate, operator-only command, `flow_control_key_force_retire(version, actor, reason)` (migration
  `20270202000000`), rather than a flag on the activation: it applies only to a `RETIRING` version, so an emergency
  rotation away from a leaked active key is the ordinary activation of a successor followed by the force (one
  transaction if the operator wants), and a leaked retiring key is forced out without a new version. It takes the
  version table `EXCLUSIVE`, as the activation does, so it waits for every decider in flight and every later decider
  waits for it. From its commit `flow_control_key_versions_in_force()` no longer answers that version: no process needs
  its key, a key decided afterwards is filed under the active version (a second row), and the old row keeps its holders,
  grants and waiters, gains no waiter (a decider files a waiter only under the row it resolved; only the gate's two files
  write waiter rows) and grants only its own waiters within its own limit. The over-admission is therefore, at any
  moment, at most what the old row holds at that moment (never more than its limit), until its holders and waiters drain;
  order holds within each row, not across them (the exception to ADR-0088 decision 2 and ADR-0056 decision 5 that
  ADR-0093 states; the owner confirmed this reading of "its old row's holders", O1174 (a), OD-2026-11-20-4). One forced version drains at
  a time (`FORCED_VERSION_DRAINING`), so a key has at most two rows (reviews of 2026-11-20). No release, grant or timeout
  path digests a key, so no Rust change is needed there; the readiness round's live-keys count reads only the rows of
  the versions in force, so a key filed again after a force is not refused for the row it replaces (correctness review
  M2). A rate window's grants never drain in this build (G022-003): a forced rate over-admission lasts until T022-074.
  Every force is a row of the append-only `flow_control_key_forced_retirements` (operator and reason each one line and
  bounded as typed, `session_user`, rows left); the down migration refuses while one is on the record.
  `flow_control_key_version_runs(version)` lists the runs whose holders, waiters or grants use the version's rows, for
  O1170 (a). Evidence: `integration_flow_control_step_policies::{rotation::a_forced_retirement_over_admits_a_key_by_at_most_its_old_rows_holders_until_they_drain, rotation::a_key_filed_again_after_a_force_is_not_refused_by_the_live_keys_ceiling, migrations::the_force_is_the_operators_audited_and_retires_only_a_retiring_version}`.
- **D022-047 — The live-keys overshoot is documented, not locked (G022-002; OD-2026-11-19-4; 2026-11-19).** The
  readiness round counts the namespace's committed semaphore rows before it files a key (`refused_by_ceilings`) and
  creates new keys while the count plus its new keys stays within the ceiling. Transactions that create keys at the same
  moment do not see each other's uncommitted rows, so together they can pass the ceiling by at most the new keys that
  the others create. When each creates one key (one gated step decided per round, the usual case) that is at most their
  number minus one, within the owner's bound; a round that decides several gated steps with distinct new keys (parallel
  branches released in one commit) can add up to its own room, so the exact bound is in keys, not transactions (O1173 (a), accepted OD-2026-11-20-3).
  The bound is written in `docs/runbooks/flow-control.md`; the text of `contracts/errors.md` (`FLOW_CONTROL_LIMIT`) gains
  it when the approved texts are next revised with G1195.
- **D022-048 — A pause gives back, a resume asks again (T022-100, T29; FR-022-040, FR-022-046; ADR-0096, accepted
  OD-2026-11-20-2; 2026-11-20).** The pause settles the run (`execution::pause_run` now returns a `Progress`): the planner gives back the
  permit and the slot of every gated root step granted for its next attempt and not leased, and the reducer makes it wait
  again (`WAITING`, `Gate`) from those return events, so the journal alone reaches the same state. While the run is
  paused that step is not asked (`RunState::waits_for_resume`, shared by the planner, invariant 11 and `persist`, whose
  `waiting_kind` is its first declared gate, rate first). A step made ready while paused is still decided in its commit
  (ADR-0094 decision 2); a grant is then given back at once. On resume, or on an operator's retry after a hold for
  review, the step asks again for the same attempt: the reducer accepts a decision over a slot that attempt gave back
  unleased (`ENDED`). A pause or resume locks the keys of its returns and of the run's waiters in one sorted pass before
  it writes any (correctness review M1 of 2026-11-20). Hold for review, cancellation and run end gave back already (the
  step leaves `READY`); T022-100 adds their tests. The exception to ADR-0094 decisions 2 and 3 is ADR-0096 (accepted 2026-11-20, OD-2026-11-20-2).
- **D022-049 — A run's start is admitted through one helper and projected on the run (T022-022, T10; FR-022-018,
  FR-022-021, FR-022-059; D022-031; 2026-11-20).** Every creator on the tree (the start API with reruns, schedule
  firings, with-start, batch and ad hoc admission through `RunService::finish_start`; event and cron triggers in
  `scheduler/events.rs`; child nodes in `scheduler/passes.rs`) starts a created run through `Engine::start`, and
  `execution::start_run` and `start_run_delayed` refuse a definition that gates its start, so a creator that bypasses
  the helper fails closed instead of starting ungated. The helper evaluates the start key over the input (a key that does
  not evaluate or exceeds 256 bytes ends the run `FAILED`/`EXPRESSION_ERROR` at once, D1180) and asks the adapter
  (`RunTx::decide_run_gates`) to file and lock the key after the run's row (slot 7), count it, and write a holder or a
  waiter row; the reservation rule of D022-021 applies (holders plus waiters below the limit). Refusals are synchronous:
  the key's waiter ceiling `CONCURRENCY_QUEUE_FULL`; `queued_runs`, the live keys or a zero effective limit or timeout
  `FLOW_CONTROL_LIMIT`; nothing of the run exists. The gates of the start are projected on `runs.flow_control_gates`
  (migrations `20270203000000`, `20270203000100`), written by `persist` only in a transaction with a run-scope gate event
  and read with the run's row (no statement more on the plain path), so a finished run keeps what its gates ended as and
  `verify_run` compares the journal, the projection and the rows (invariants 1 and 2 for the start). The grant (T26) and
  timeout (T27) routines serve run waiters through the same adapter code as step waiters (`node` is `None`; scope
  `RUN`); a queued start that times out fails the run directly (`time_out_run_gates`). Run-scope outcomes are counted at
  scope `run`. A start's `rateLimit` stays refused (`BINARY_CAPABILITY_MISSING`) until T022-072; a delayed start and a
  reset of a definition that gates its start are refused `BINARY_CAPABILITY_MISSING` rather than started ungated
  (G022-009), and so is a definition with a start gate and a `forEach`, `while` or `repeatUntil` node, whose finish paths
  end the run without the reducer and would never give the permit back (reviews of 2026-11-20, B1; lifted by G1197).
  **Supersedes the placement of D022-031** and the words "inside `PgRunStore::create`" of FR-022-059 (delegated by
  OD-2026-11-20-6; architecture review B1 of 2026-11-20): the key is an expression over the run's input that only the
  application evaluates, and the decision is journaled through the domain's start functions, which `create` cannot call;
  ADR-0056 decision 3 asks for "one application port that every run creator calls", which `Engine::start` is. What
  FR-022-059 protects (no creator starts a run past the gate) is kept by three layers: `PgRunStore::create` still refuses a
  gated definition a process does not run the gate for; the domain refuses a gated definition in `start_run`,
  `start_run_delayed` and `record_reset`; and `scripts/check-architecture.py` (`run_start_errors`) refuses a call of a
  domain start function outside `Engine::start`. The approved text of FR-022-059 is revised with G022-007.
- **D022-050 — Decisions delegated by OD-2026-11-20-6 while building T022-022 (2026-11-20).** Each weighed every option
  and took the one with the most architectural and business advantage. All eight were accepted by the owner on 2026-11-20
  (relayed by the coordinator), item 3 on the condition that T022-105 makes `queued_runs` exact, derived and tested with a
  held-lock race, before pull request 237 leaves draft (G022-011, merge blocker); every count, budget line or bound is
  derived and tested, never estimated:
  1. *Where the start gate runs:* `Engine::start` (D022-049), over `PgRunStore::create` (it cannot evaluate the key or
     journal) and over a gate in each creator (the "guard each caller" shape ADR-0089 rejected).
  2. *A start gate with a collection or a loop:* refuse at admission until G1197 routes their finish through the reducer,
     over failing the run at its finish (a run that worked for nothing) and over building the routing now (G1197 is the
     slice's next item and changes the same paths for a gated successor: one change, not two).
  3. *`queued_runs` without a counter:* count the waiter rows unlocked until T022-105's trigger-kept usage rows
     (FR-022-058, the approved design), with the bound stated (at most the concurrent queuing admissions), over a namespace
     row lock, which would serialize every queuing admission of a namespace, the throughput the feature exists for
     (G022-011).
  4. *Admission counts every waiter of the key, the grant routine only eligible ones:* kept. A new arrival never takes a
     permit ahead of any waiter (D022-021); a waiter that is paused, past its deadline or orphaned is served or removed
     by the next pass, so the stricter count costs at most one tick.
  5. *The start's decision code beside the step's:* one shared core is built with T022-072, the change that gives a start
     its second gate, so that it is designed against both gates of both levels at once (G022-012).
  6. *Signal- and Update-with-Start of a start that queues:* hold the signal or update with the queued run and deliver it
     at the grant (FR-022-018 treats a with-start as an ordinary admission), over refusing every with-start of a gated
     definition (load-independent, but it loses the feature). Until built, the existing typed refusal
     (`SIGNAL_NOT_ACCEPTED_AT_START`, nothing created) stands (G022-010).
  7. *The refusal code of a path not built:* `BINARY_CAPABILITY_MISSING` (503, retryable), as for every other gated path
     this build does not run, over a new non-retryable reason that would become dead with the path; no process lists the
     capability, so no production client meets it (G022-009).
  8. *Lock order of a commit that ends a gated run:* lock the run's active-run counter row first through the
     `SECURITY DEFINER` `run_active_counts_lock_slot` (ADR-0050 slot 6 before slot 7), over moving the returns after the
     run row (it would lock a key after the usage rows of slot 8b, D022-038). One statement, only in a commit that ends a
     run with a flow-control event.
- **D022-051 — `queued_runs` is exact under the namespace's admission lock (G022-011, T022-105; 2026-11-20).** D022-050
  item 3 said the count was read without a lock; reading the admission path for T022-105 showed it is not. Every run
  admission passes `PgRunStore::create`, which takes the namespace's active-run quota lock
  (`pg_advisory_xact_lock(quota:active_runs:<tenant>:<namespace>)` in `ensure_room_for_run`) before the runs row and
  holds it to the commit; only an admission adds a run-scope waiter row (the grant and timeout routines and the stop
  paths only remove them); and the start's gate counts the namespace's run waiters after that lock in a statement of its
  own, which under READ COMMITTED sees every admission committed before it. So two admissions at the boundary are decided
  one after the other, and the namespace never holds more queued runs than its ceiling. Derivation tested with the lock
  held by the test (no sleep): `run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other`;
  hand mutations caught: the lock removed (the two starts no longer wait on it), the ceiling ignored (both queue). The
  live-keys count (G022-002) is not exact in the same way: a step's key is filed by a readiness round, which takes no
  namespace lock.
- **D022-052 — A start delay decides the start's gates at its release, through the planner (T022-064, G022-009;
  FR-022-018; 2026-11-20).** Delegate (OD-2026-11-20-6). FR-022-018 says a run with an 018 start delay "acquires or
  enqueues when the delay elapses". Three choices were weighed for each part, and the one with the most architectural and
  business advantage taken:
  1. *Where the release decides:* a new planner question, `Evaluation::StartGates { queue }`, asked when a released
     start (or a reset, D022-053) has gates no decision reached, and answered by the engine like the readiness gate's
     (`Engine::decide_start_gates`, the start key evaluated over the run's input, `RunTx::decide_run_gates`). Taken over a
     scheduler-side call in the release pass (a second code path beside `Engine::start`, which G022-012 asks to
     remove) and over deciding at admission with a not-before instant (it contradicts FR-022-018 and would hold a queue
     place for the whole delay). Any path that releases the delay asks, so no creator can start the run around the gate;
     `start_run_delayed` therefore no longer refuses a gated definition, and `start_run` still does.
  2. *A key that refuses the released run* (its waiter queue, `queued_runs` or the live keys full): the release rolls
     back and the run stays delayed; the next pass tries again (`Outcome::Deferred`, `Report.gate_starts_deferred`, a
     warning, not a failure). Taken over failing the run with the refusal code (an accepted run lost to a moment's load,
     and a new run-level failure code) and over queueing past the ceiling (it breaks the bounded queue the ceilings
     exist for). It matches the asynchronous creators (an event, a cron tick, a schedule's time are retried, plan "Run
     creators"). Cost: one transaction per such run per pass, bounded by the pass's batch; nothing is written.
  3. *`queued_runs` stays exact:* a release queues a run outside its creation, so it takes the namespace's admission lock
     (`lock_run_admission`, the lock `ensure_room_for_run` takes first) before it counts; an admission holds it already
     (D022-051 extended: every transaction that adds a run waiter holds that lock). Order: the run's row (taken by the
     pass), then the lock, then the key; no transaction holds the lock and then waits for an existing run's row, so no
     cycle. Proved by `run_creators::a_release_and_a_start_at_the_queued_runs_ceiling_are_decided_one_after_the_other`.
  A key that does not evaluate at the release ends the run `FAILED`/`EXPRESSION_ERROR` there (D1180).
- **D022-053 — A reset takes a fresh permit right after `run.reset`, or is refused, and never queues (T022-064;
  FR-022-018, FR-022-038; 018's plan, rows 17 and 18; 2026-11-20).** Delegate (OD-2026-11-20-6) for items 2 to 4; item 1
  is the approved contract.
  1. *The reset rules of the five `concurrency.*` events* are installed as `contracts/journal-events.md` states:
     `queued`, `acquired`, `released` `REMATERIALIZE` (history), `wait_timed_out` and `dequeued` `REFUSE` to the end. A
     point while a start waits is no quiet state already (`NotRunning`), and one with a step open is `OpenWork`. The
     `rate_limit.*` events stay `EXCLUDE` until T022-086.
  2. *How the history is kept out of the new run:* the reducer clears the start's gates at `run.reset` and retires the
     restored step permits (`Held` to `Ended`: their attempts ended at the point), so the reset run gives nothing back for
     them and holds no row of them; `persist` skips the returns of a restored prefix; the closed detail of a gate event
     accepts the `restored` marker (and only `true`). Taken over special rows for restored holders (rows nobody holds)
     and over filtering restored events in every reader (each reader would have to know).
  3. *Where the fresh decision is taken:* the planner asks `StartGates { queue: false }` right after `run.reset`, before
     anything of the run goes on, and the reducer accepts a run-level decision only there (`reset_admission`). The
     adapter refuses `CONCURRENCY_PERMIT_UNAVAILABLE` when holders plus waiters reach the limit (the reservation rule of
     D022-021: a reset never takes a permit a queued run waits for). One path with the delay's (D022-052).
  4. *The refusal's answers:* `409 CONFLICT` with `details.reason` `CONCURRENCY_PERMIT_UNAVAILABLE` (018's contract);
     a pending reset (`CANCEL_SOURCE`) fails its entry with that reason (018's data model lists it; it joins
     `FAILURE_REASONS`, so its metric label is closed); a batch item `CONFLICT`. Proved by
     `run_creators::{a_reset_takes_a_fresh_permit_or_is_refused_and_never_queues,
     a_reset_after_a_gated_step_restores_its_permit_as_history,
     a_pending_reset_takes_a_fresh_permit_or_fails_concurrency_permit_unavailable}`.
- **D022-054 — What the reviews of T022-064 changed (2026-11-20; `reviews/2026-11-20/creators-*.md`).** Delegate
  (OD-2026-11-20-6) for every choice below; each finding was checked against the code, and those a test can show were shown
  red first (or by a hand mutation of the fix).
  1. *A refused release no longer holds the release pass* (architecture B1, security Major 1, correctness M1): the pass
     records the deferral in a transaction of its own (`start_release_deferrals`, migration `20270204000000`: one row per
     run, `deferrals`, `retry_at`, the closed `reason`; cascade with the run) and the due query skips a run until its
     `retry_at`; backoff 5 s doubling to 5 min (018's plan, row 4). Over the alternatives: moving `start_delayed_until`
     (it must keep matching the journaled `run.start_released`), a column on `runs` (the hot table, and an UPDATE an older
     build does not name), failing the run (loses an accepted run, D022-052). No bound beyond the run's deadline: a
     delayed run is an active run like any other, visible (`WAITING`, its deferral row) and counted. Only `start_release`
     defers; a ceiling's refusal in any other action is a failure (security nit 8). The first deferral of a run is logged
     once; its retries are `scheduler_actions_total{kind="start_release",outcome="deferred"}`. Proved by
     `run_creators::deferred_releases_back_off_and_do_not_hold_back_the_starts_behind_them` (a pass of batch 2: the two
     refused releases do not stop the third; 5 s then 10 s).
  2. *Counts* (correctness M2, architecture M3, security minor 5): a refused release is not counted `refused_full` (it was
     counted once per pass); a reset's refusal is counted `refused_reset`, the closed outcome the plan's table names.
  3. *Fail closed in the reducer* (architecture M1): a state that knows its definition releases nothing before every gate
     of its start holds (`RunState::start_gated`, compared as configuration, not state); the reducer itself refuses a wait
     after `run.reset` (architecture m2); the adapter refuses a start rate limit, a collection or a loop whatever the
     cause (security minor 4). With `start_run`'s refusal and `run_start_errors`, these are the layers of FR-022-059 now
     (D022-049's "the domain refuses a gated definition in `start_run_delayed` and `record_reset`" no longer holds: those
     two decide their gates through the planner).
  4. *One rule, one cause* (architecture M2): the cause of a start's decision is the domain's `StartCause` (`Admission`,
     `Release`, `Reset`), carried by `Evaluation::StartGates` and `RunGateKeys` unchanged, and one function
     (`check_start_decisions`) validates every start decision. G022-012 keeps the rest (one core for both levels).
  5. *Projection and decoding* (security minor 3, correctness m3; security nit 7): `run.reset` rewrites the start's gate
     projection of a gated definition; the closed detail accepts `restored: true` only on a `concurrency.*` event.
  6. *A pending reset* (correctness m6, architecture m4): `BINARY_CAPABILITY_MISSING` is retried, not a final
     `ADMISSION_REFUSED`. With `CANCEL_SOURCE` on a busy key the freed permit goes to the queued run first and the entry
     fails `CONCURRENCY_PERMIT_UNAVAILABLE` (the reviewers' note): kept, since a reset jumping the queue would break the
     order every queued run was promised (FR-022-042); the runbook says so (`docs/runbooks/run-reset.md`).
  7. *Update-with-Start of a queued start* (architecture M6): D022-050 item 6 held "the signal or update" so that the outcome
     does not depend on the key's load. An Update needs no hold to meet that: it changes the declared state, which a
     queued run has, and its receipt is the same whether the run holds a permit or waits
     (`run_creators::an_update_with_start_admits_through_the_key_and_applies_its_update`). Holding it would delay a
     business answer the caller waits for, for nothing. Only the signal needs the hold (T022-109).
  8. *Contracts* (architecture M4, security minor 6, correctness m7): `CONCURRENCY_QUEUE_FULL` and
     `CONCURRENCY_PERMIT_UNAVAILABLE` are raised now, with 018's remediation sentences, named in the canonical OpenAPI's
     `Error.details` (and the Studio client regenerated); the 409 has its API test.
  9. *Rolling upgrade* (security Major 2, correctness m5, architecture m3): no build that runs the gate has left this
     branch (pull request 237 is a draft, and no process lists `flow-control-*-v1`, D022-037), so no older replica can
     release a gated delayed start or read a restored gate row; the capability listing (G1198) must be a build with this
     change. A nit for this release.
  10. *The moment after `run.reset` is a within-commit fact* (correctness m2, architecture n3): the reviewers called it
      latent; `integration_schedules_bulk_operations::reset_corpus` showed it is not (a reset of an ungated definition
      whose commit ends right after `run.reset`: the replay said set, the projection not; 4 tests red). It is now compared
      as always equal (`WithinCommit`), since a gated definition's planner asks for the start's decision before that
      commit ends and nothing reads it across commits. *Not changed:* a deferral's own deadline beyond the run's
      (architecture B1 item 7).

### Decisions of the eighth build (2026-11-20)

- **D022-055 — What the reads of a key show, and from where (T022-026; FR-022-022; delegate (OD-2026-11-20-6)).**
  1. *Source of `Run.concurrency`.* Options: (a) the run's gate projection `runs.flow_control_gates` alone; (b) the
     run's holder and waiter rows; (c) the projection with the rows for `since`. Chosen: **(b)**, one lateral probe of
     the two primary keys in `get` and `list` (`run_permit_lateral_sql!`). The rows are what the gate decides with and
     what the keys listing reads, so a run's permit and the listing of its key cannot disagree; they carry `since`
     (`acquired_at`, `enqueued_at`) and the exact queue deadline, which the projection lacks; invariants 1 and 2 tie
     them to the journal (`verify_run`, `assert_consistent`). (a) would show no `since` and be a second source for the
     same fact; (c) reads two sources for one field. A run that neither holds nor waits has no `concurrency` field
     (absent rather than `null`, so no existing body or golden changes; the contract makes it optional and nullable).
  2. *Order and cursor of the listing.* Oldest wait first, keys nobody waits for last, then pool, scope, group and
     digest: a total order, since those five are the row's key within a namespace. Options for the cursor: an offset
     (moves under live rows and costs a scan), the label alone (two keys of one group may share a 48-bit label: a page
     boundary between them would skip or repeat one), server-side cursor state (a table and its expiry for a read), or
     the full keyed digest in an opaque, versioned cursor (*amended by D022-059: the cursor is sealed*). Chosen: **the digest** (`KeyPosition`, `k1.` text of hex
     fields). It is the output of the same keyed function as the label (an HMAC under the deployment's secret key), so
     it lets nobody test a guessed key value, and it goes only to the caller the labels already go to. A cursor this
     process did not write is a `400`; a page and its `queuedRuns` are one `REPEATABLE READ` snapshot, and a key whose
     oldest wait changes between two pages may move (live state, stated in the contract's "snapshot").
  3. *One count of queued runs.* The listing, the `queued_runs` ceiling at admission and its quota usage read one
     statement (`QUEUED_RUNS_SQL`), so the number an operator reads is the number the ceiling compares, by
     construction. T022-105's counter rows replace that statement in one place.
  4. *`keyLabel` is always the label for now* (*amended by D022-060: only to a caller who may reveal personal data*). Its `null` for a caller who may not disclose every location the key reads
     is T022-104's (G1185); the listing needs `ops:read`, which every human role holds and no machine role does.
- **D022-056 — Invariants 1 and 2 across runs (T022-027).** `verify_run` checks one run; the invariants are about a
  key, whose rows belong to many runs. `World::assert_consistent` (every test of the suites) now also runs
  `support::flow_control::key_consistency` over the tenant in one `REPEATABLE READ` snapshot: counters equal rows; every
  holder row is a `HELD` permit of its run's or step's gate projection with the same attempt, pool, group and label, and
  every waiter row a `QUEUED` one with the same deadline to the microsecond; every `HELD` or `QUEUED` projection has its
  row; and each run a row names is verified against its journal (restored events are excluded by the replay). Rejected:
  checking from the journals alone across runs (a replay of every run of the tenant per assertion, where the
  projections are already proved equal to the replay run by run).
- **D022-057 — The deployment's digest key ring reaches both binaries as a file, and a process serves the gates it
  runs with it (G1198; delegate (OD-2026-11-20-6)).** G1198: no configuration carried the `KeyRing` to the stores of the
  control plane and the engine, and the admission point refused every gated run (D022-037) except through a test door.
  *Where the key material comes from.* The codebase supplies server key material three ways: an environment value
  (`SAGAT_SECRETS_KEYS`, `SAGAT_SCRIPT_SIGNING_KEY`), a file a variable names (`SAGAT_HTTP_TLS_KEY`,
  `SAGAT_OPENBAO_INTENT_KEY_FILE`, `SAGAT_RUNNER_TOKEN_FILE`, read bounded), and feature 015's secret provider (OpenBao),
  which serves tenants' secrets through intents. Options: (a) an environment value `version:hex` (copied into every child
  process, `docker inspect`, `/proc/<pid>/environ` and crash reports); (b) a file named by
  `SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE`, mounted read-only from a Kubernetes or compose secret; (c) fetched from OpenBao
  at start (the gate of every run would depend on OpenBao at every restart, and the provider is the tenants', not the
  platform's own keys); (d) a database row (forbidden by FR-022-053). Chosen: **(b)**, the strongest of the existing
  patterns: the file is read once at start, bounded (16 KiB), wiped from memory after parsing (`zeroize`), parsed by
  `KeyRing::parse` (whose errors name no key) and never printed (`KeyRing`'s `Debug` shows versions only); a raw-key
  variable is not accepted at all. The Helm chart mounts a Secret (`flowControl.digestKeys.existingSecret`, mode 0440
  as the OpenBao CA is) and compose an overlay (`compose.flow-control.yaml`, the file ignored by git).
  *Startup rules.* Either switch on without the file is a configuration error (`Missing`) in both binaries, as is a file
  that is unreadable, too large, not text, not a ring, or empty; and a process refuses to start when the database has a
  version in force (active, or retiring) that its ring lacks (`check_flow_control_keys`), instead of starting and
  refusing every key of that version (G1193).
  *What a process lists and admits.* `flow_control::served(switches, keys)`: the capabilities this build runs the gate
  of (`BUILT` = the concurrency gate), switched on, with the ring. Both binaries list `flow-control-concurrency-v1`
  exactly then, and never `flow-control-rate-v1` until T022-072 and T022-074 exist. The admission point admits a gated
  definition when every capability it needs is switched on, activated for the fleet and served by this process
  (refusals in that order: `CAPABILITY_DISABLED`, `FLOW_CONTROL_ACTIVATION_PENDING`, `BINARY_CAPABILITY_MISSING`); a
  start's `rateLimit` stays refused (T022-072). The test door `admitting_step_gates_for_tests` is gone: tests admit
  concurrency gates through the real rule (switch, ring, activation), and the step-level rate gate through
  `serving_rate_gates_for_tests` (feature `rate-gates-for-tests`, `[dev-dependencies]` only, a production call refused
  by `scripts/check-architecture.py`) until T022-074. Rejected: listing the capability from the switch alone (a
  process without the ring would promise a gate it cannot decide); requiring only the concurrency switch to need the
  ring (a rate switch without keys would fail the same way once the rate gate exists).
  Proved by `integration_flow_control_step_policies::admission::a_process_with_the_switch_the_activation_and_the_keys_admits_a_concurrency_gated_run`,
  `apps/execution-engine/tests/registry.rs` (listing, refusals, no key in an error or a printed configuration) and the
  control plane's registry tests. The harness gives a real engine the switch and the ring with
  `support::flow_control::process_env` and activates the concurrency capability only (`activate_concurrency`);
  `admission::a_real_engine_with_the_ring_lists_the_gate_and_one_missing_the_active_version_does_not_start` starts a real
  engine with it (it registers listing the capability) and one whose ring lacks the active version (it does not start).
  A process with every switch off neither reads nor holds a key file it was given (review of the fixes, m5; re-review). *Amended by D022-062: it holds one it is given, to drain, and lists the gate.*
  *Listing before T022-104 (review of the fixes, architecture M3).* Listing the capability makes activation possible
  while publication does not yet refuse a key over a `secret` location (T022-104). Options: hold the listing back until
  T022-104 (blocks the SDK conformance of the release slice for a publication rule); build the `secret` refusal alone now
  (needs the classification of every location a key expression reads, which is T022-104 itself); list now and make
  T022-104 a gate of the release and of production activation. Chosen: **the last** (G022-015): the runbook says not to
  activate the capability in production before T022-104, the release cannot close with T022-104 open, and until then a
  key's label goes only to a caller who may reveal personal data (D022-060).
- **D022-058 — A page of the key listing costs its own keys (keys review of 2026-11-20, architecture M3; T022-026,
  T022-105; delegate (OD-2026-11-20-6)).** The listing computed each key's oldest wait by a correlated `min()` over its
  waiters and sorted every live key of the namespace before `LIMIT`: a page cost O(live keys), up to the 100,000 the
  live-keys ceiling allows, and paging the whole listing was quadratic, on the API's pool, for any human with
  `ops:read`. Options: (a) a statement timeout only (bounds the harm, keeps the cost); (b) a server-side materialized
  view refreshed by a job (stale counts, a refresh that costs the whole table anyway); (c) keep the oldest wait on the
  semaphore row, maintained by the trigger that already keeps its counters, and walk an index in the listing's order.
  Chosen: **(c) and the timeout of (a)**. Migration `20270205000000_flow_control_key_listing`: `oldest_wait_at` (CHECK
  `(waiters = 0) = (oldest_wait_at IS NULL)`, added `NOT VALID`, validated by `20270205000100`); the count trigger keeps
  it in the transaction that adds or removes a waiter (`LEAST` on an insert; on a delete the minimum of the key's
  waiters, read after the trigger locks the key's row by its own statement so that its snapshot sees every waiter
  committed under that lock); `concurrency_semaphores_listing_idx` and `_group_listing_idx` in the listing's order; the
  holders' key index extended by `acquired_at`, so the oldest holder is one probe. The listing's position is always an
  index condition (`'-infinity'` before the first key, never an `OR` that a generic plan would make a filter). A page is
  at most 201 index entries and 201 holder probes; `SET LOCAL statement_timeout = '2s'` refuses anything slower as
  `Unavailable` (`503`). Proved by `rest::a_page_of_keys_reads_only_its_own_keys` (the planner, on 5,000 keys, walks
  the index with no sort and reads exactly the page's 51 rows, for both statements, from the first key and after a
  cursor) and by `key_consistency`'s new check that every key's oldest wait is its waiters'. Cost on the write path: an
  update that changes `oldest_wait_at` is not HOT (an indexed column changes) once per change of a key's oldest
  waiter. Rejected (b): its staleness contradicts "counts read in one snapshot". `queuedRuns` is still a count of the
  namespace's run waiters, bounded by the `queued_runs` ceiling (T022-105's counter rows replace it).
- **D022-059 — The listing's cursor is sealed for its namespace (keys review of 2026-11-20, security M2, architecture
  M1; amends D022-055 item 2; delegate (OD-2026-11-20-6)).** D022-055 put the key's full digest in the cursor in clear;
  the label is the digest's first six bytes, so once a label is hidden (D022-060) the cursor would show it, and a forged
  cursor could bisect a hidden digest (about 256 requests). No cursor of the codebase is signed (they are run ids,
  instants and offsets of the caller's own rows), so the keyed-digest machinery seals this one. Options: (a) an HMAC
  over the clear position (tamper-evident, still readable); (b) server-side cursor state (a table, its expiry and its
  cleanup, for a read); (c) a surrogate key column ordered instead of the digest (a sequence shared by every tenant
  leaks their creation rate); (d) authenticated encryption of the position. Chosen: **(d)**: ChaCha20-Poly1305 under a
  key derived from the version of the deployment's flow-control digest key the database has active in the page's
  snapshot, which every process of a rotation holds (review of the fixes, m4: the newest version a process holds would
  break paging across replicas during a rolling addition) (`HMAC(key, "sagat.flow-control.cursor.v1\0")`, a message no
  key digest can be), a random nonce per cursor, the version in clear and the tenant and namespace as associated data
  (`KeyRing::seal_cursor`, `KeyPosition::seal`); a position is at most 364 bytes and the bound is 384, derived from the
  longest group (`MAX_CURSOR_PLAIN_BYTES`, review of the fixes, correctness M1). A caller learns nothing from it, cannot
  alter it, and cannot use it in another namespace; a cursor sealed under a version the process no longer holds, an
  altered one, a forged clear position (`k1.`) and one of another namespace are each a `400`, decided before admission
  (the same answer whether the namespace exists or not). A position before 1970 is refused (the `500` of security
  minor 2). A cursor of one group's listing must name a key of that group. Without a ring a process cannot hand out a
  cursor: a page that has a next one is refused `503` rather than cut short (with D022-057 a process that runs the gate
  always holds the ring). Proved by the unit tests of `keyed_digest` and `flow_control_keys` (every single altered
  character of a cursor refused) and by `rest::the_namespace_lists_its_keys_oldest_wait_first_and_pages_them_exactly`.
- **D022-060 — A key's label is shown only to a caller who may reveal personal data (keys review of 2026-11-20,
  security M1; FR-022-057; amends D022-055 item 4; delegate (OD-2026-11-20-6)).** FR-022-057 gives the label only to a
  caller who may disclose every location the key reads; T022-104 (classifying those locations at publication) is not
  built, and an unannotated location is `pii` (`Level::UNMARKED`). D022-055 item 4 showed every label to every reader.
  Options: (a) keep it until T022-104 (a viewer can confirm a guessed customer value, G1185); (b) refuse to start with
  the switch on until T022-104 (blocks the feature's release slice for a read-side rule); (c) fail closed: take every
  key to read `pii`, so the label goes only to a subject whose current role may use a reveal grant and who holds the
  namespace's `payload:reveal-pii` grant (feature 009's rule), `null` for every other caller. Chosen: **(c)**. It is
  built so that forgetting the check hides the label: the store returns no label (`ConcurrencyKeyView.label` is `None`,
  `RunPermit` hides its own), and only the services reveal it (`OperationsService::concurrency_keys`,
  `RunService::get` and `list`, one check per namespace). The check reads the membership and the grant in one statement
  without the disclosure path's locks: a label is a pseudonym, not an audited disclosure, so a revocation that commits
  during a read may let that one read show a label. T022-104 relaxes this per location (a key over `public` or
  `internal` fields to their readers) and refuses `secret` keys at publication. Proved by
  `rest::a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data` (a viewer and a developer without a grant
  read `null` in the run, the run listing and the key listing; an admin with the grant and the developer once granted
  read the label).
  *Revised after the review of the fixes (security Major 1).* A key's `oldestHolderSince` and `oldestWaitSince` equal
  the `since` of its runs to the microsecond, so a caller who may not see the label could still link the runs of one key
  by them. First made `null` for such a caller; the re-review (correctness Major) found that this breaks FR-022-022's
  "oldest wait" for most operators, so such a caller gets them cut to the minute: how long a queue has waited stays
  visible, and linking needs runs one minute apart. A weaker channel stays (the counts of a key and the oldest-wait order,
  G022-014). The reveal rule has one implementation, `reveal_grants::may_reveal_payload_unlocked`, and a fitness rule
  keeps `payload_reveal_grants` read only there (review of the fixes, architecture M1).

- **D022-061 — What a caller without the reveal right learns of a key's instants (re-review of 34b8be41, correctness
  Major 1; FR-022-022, FR-022-057; amends D022-060's revision; delegate (OD-2026-11-20-6)).** The cut to the minute does
  not stop linking: `Run.concurrency.since` reaches every `run:read` caller whole, so a key's instant to the minute still
  names its runs whenever the group's runs start in different minutes, the usual case for a low-rate key (one customer's
  serialised runs), and the listing's order within a minute is the whole instants' order. Options: (a) keep the cut and
  say what it does (narrows linking in busy groups, does not stop it); (b) for such a caller, list one aggregated row per
  (pool, scope, group) (holders, waiters, oldest holder and oldest wait of the group, whole) instead of per-key rows;
  (c) also cut `Run.concurrency.since` (minutes still match: no gain, and FR-022-022's run view loses precision);
  (d) per-key rows without instants (the counts still link, G022-014, and FR-022-022's "oldest wait" is lost). Chosen:
  **(b) as the end state, (a) until it lands.** A group is not a secret (it is the definition's name or a declared
  slug, already on each run), so a per-group aggregate reveals nothing a run reader lacks, closes this channel and
  G022-014's count channel together, and serves the operator's question (which queue is stuck, for how long);
  FR-022-022's per-key listing "by key label only" stays the view of callers who may see labels. Built now, a group's
  sums would cost a scan of its keys per page (up to the live-keys ceiling, the cost D022-058 removed); built right, they
  are per-group counter rows kept by the triggers that keep the key rows' counters, which is the counter work of
  T022-105. So it is follow-up **T022-111**, a release item beside T022-104 (which shows labels of keys over `public` or
  `internal` fields to their readers and so shrinks the population it serves). Until then the cut stays, its
  arithmetic fails closed (no path returns the whole instant), and the contract, the runbook and the release note say
  that it narrows and does not stop linking.
- **D022-062 — A process with the switches off that is given the key file keeps it, to drain (re-review of 34b8be41,
  correctness Major 2; FR-022-031; amends D022-057's "a process with every switch off neither reads nor holds a key
  file", review of the fixes m5; delegate (OD-2026-11-20-6)).** FR-022-031: "disabling MUST stop new publications and
  starts while existing runs drain". 34b8be41 dropped the ring of a switched-off process, so its grant pass and its
  step decisions refused (`BINARY_CAPABILITY_MISSING`) and every waiter could only time out. Options: (a) as 34b8be41
  (no drain; contradicts FR-022-031); (b) hold the ring whenever a file is named, whatever the switches, read and
  checked as strictly as with a switch on; the switches govern admission (`CAPABILITY_DISABLED`) and the listed
  capabilities only; (c) a third setting for "draining". Chosen: **(b)**: it is FR-022-031's behaviour with no new
  setting, and the concern of m5 (a switched-off process using a stale ring unchecked) is met by checking the ring
  against the versions in force whenever one is held (`check_flow_control_keys` no longer looks at the switches). A
  switch on without a file is still `Missing`; no file with the switches off holds nothing. Proved by
  `apps/execution-engine/tests/registry.rs::an_engine_with_its_switches_off_holds_a_given_key_file_to_drain_and_lists_the_gate`
  (red with 34b8be41's early return) and
  `run_admission::with_the_switch_off_a_process_holding_the_keys_drains_the_queue` (its store built from the engine's
  parsed configuration).
  *Amended after the re-review of 39157608 (Major 1).* With the activation recorded, a process that does not list the
  gate is refused by the registry and serves nothing (FR-022-053), so a switched-off process that did not list it could
  never drain, and a rolling restart with the switch off would stop the fleet. Options: (a) document an order (remove
  the fleet requirement first, then restart switched off); (b) list the capability from the ring alone, whatever the
  switches, since a listed capability promises that the process runs the gate for what was admitted, which a draining
  process does, while the switch says only whether new gated work is admitted (`CAPABILITY_DISABLED` first, at the one
  admission point and at publication). Chosen: **(b)** (`flow_control::listed`): no operator order to get wrong, no
  window where a requirement is removed while gated runs live, and the registry keeps meaning "this process will not
  mishandle a gated run". Rejected (a): one forgotten step stops every process of the fleet. Proved by
  `admission::a_draining_engine_with_the_keys_stays_in_a_fleet_that_requires_the_gate` (a real engine restarted with the
  file and no switch registers listing the gate; with a file lacking the version in force it does not start). A child
  run started by a draining run is a new start, refused `CAPABILITY_DISABLED`; its node's typed failure joins T022-102.
- **D022-063 — A cursor of a version out of force is refused; the image's group; the two guards of an unserved gate
  (keys reviews of 2026-11-20, minors; delegate (OD-2026-11-20-6)).** (1) A version forced out (ADR-0093 decision 5, a
  leaked key) may still be in a process's file, and its holder could seal positions the process would open. Options:
  refuse only versions with a forced-retirement row; refuse every version the database does not have in force
  (active, or retiring with rows left); require operators to remove it from every file first. Chosen: **in force
  only**, read through `flow_control_key_versions_in_force()` (the one reading of the versions) before the namespace is
  checked (the same `400` whether it exists or not): it covers a force and an ended rotation alike, and cursors are
  short-lived, so refusing an ended rotation's cursor costs a client one fresh listing. Proved by
  `rest::a_cursor_is_sealed_under_the_active_version_and_one_of_a_forced_version_is_refused` (a cursor sealed under the
  active version; a retiring version's cursor opens; a forced one is `400`; red with the check removed). (2) The key
  file recipe said `chgrp 10001`, but the runtime image created its user without a group id, so its primary group was a
  system group of unknown id and the recipe granted nothing under compose. The image now creates group `sagat` with gid
  10001 (as the chart's `fsGroup`), so one recipe holds for both. (3) The admission point refuses a gate its process does
  not serve, and the start gate's own guard refused the same way, so dropping the first survived. The start gate's guard
  now answers an admission or a reset (both created through the admission point) with an internal error, and remains
  the refusal of a release (which has no admission point): `admission::a_process_with_the_switch_the_activation_and_the_keys_admits_a_concurrency_gated_run`
  and `run_creators::a_process_without_the_digest_keys_refuses_a_reset_and_decides_no_release` are red with the
  admission point's check dropped.

### Decisions of the ninth build (2026-11-20)

- **D022-064 — The rate gate is served: its ceilings, a run's slot, a step's lease and the purge (T022-072, T022-074,
  T022-077; G1199 rate half; FR-022-040..047, FR-022-056; delegate (OD-2026-11-20-6)).** Each choice weighed every
  option and took the one with the most architectural and business advantage.
  1. *Namespace ceilings of a rate limit (T022-077):* four `QuotaResource` values: the count `rate_limit_live_keys`
     (window rows of both scopes, 10,000/100,000, judged before a key is created as the semaphores' are) and three
     policies, one per window length, `rate_limit_per_second` (20/20: FR-022-047's deployment ceiling until the
     benchmark of SC-022-029 meets its floor, `DEPLOYMENT_PER_SECOND_LIMIT`), `rate_limit_per_minute` (6,000/6,000) and
     `rate_limit_per_hour` (10,000/10,000). Taken over one `rate_limit_limit` for every length (a namespace lowering its
     per-second draw would also clamp its hourly declarations to that count, a cut of up to 3,600 times nobody asked
     for) and over a normalized grants-per-hour ceiling (an operator thinks in the window the downstream API states).
     The run waiters of a key, `queued_runs` and the longest wait stay the shared concurrency resources (plan, "Ceilings,
     revised"). Migrations `20270206000000` (the three resource lists restated `NOT VALID`; a down that refuses while a
     limit or ceiling of one exists) and `20270206000100` (validated); the canonical OpenAPI's two enums gain the four
     names (an enum the operation digest does not follow, as D022-040 read) and the Studio client is regenerated.
  2. *One decision for both levels and both gates (G022-012):* a run's start and a step are turned into the same
     `Pending` gates by one function (`gates_of`: the declaration, the effective limit and timeout, the key to file) and
     decided by one core (`decide_filed`: every gate admits the target or it waits for every one with the earliest
     deadline, D022-006). A start adds only its own rules (`start_may_wait`): a reset never waits, and a run waiter meets
     `queued_runs` and each key's waiter ceiling under the key's lock. Refusals rate first (FR-022-052,
     `contracts/errors.md` US07): a start that declares a rate limit is refused `RATE_LIMIT_QUEUE_FULL` for a full key or
     a full namespace, a dual-gated one included; `FLOW_CONTROL_LIMIT` names `rate_limit_live_keys` or the window's
     policy. Each declared family counts the refusal (`refused_full`).
  3. *A run's slot counts from its grant:* the grant row stores `leased_at = granted_at` (the table's `CHECK` already
     required it), whether the readiness gate or the grant routine grants it, and is never given back: it stops counting
     by time. `verify_run` checks invariants 8 to 10 for the start (rows against the journal, the key's counters, and
     invariant 10 over the key's journals for the run's grant).
  4. *A step's lease instant (transaction T23):* a scheduler pass, before the grants of each tick, copies onto the unleased step
     grants of the run shards it holds the instant of the granted attempt's `step.leased` journaled after the grant
     (D022-029), or the grant's instant when later (the lease's transaction may have read its clock first; a later
     instant keeps a slot longer, never frees one early). One statement; it locks the grant rows only; `workers.rs` is
     not edited. A stale replica writes the same instant (it is read from the journal), so no fence is taken.
  5. *The hint of a window, early and never late, with no window lock at the lease (D022-021; plan "The grant pass, by
     key"):* the coordinator's plan said to move the key's hint at each lease; that writes the window row from a
     transaction that holds a grant row, the reverse of every other transaction's order (window, then grants: a
     deadlock with a return under the window's lock). Instead, as revised after the reviews (D022-065): a key rests until
     the first instant a grant that counted when the pass began may stop counting for one of its waiters, taken for
     every window length its waiters declare (each acquirer counts over its own window, D022-009), a leased grant at its
     lease plus that length, an unleased one at the pass's start plus it; a rate grant made to a window with waiters
     moves the hint no later than its instant plus the shortest waiting window; and the lease pass, after its commit,
     lowers each recorded window's hint to the lease plus that window, one window a statement (no transaction then holds
     a grant row and waits for a window). The rule it replaces used the grant's own period, so a waiter with a shorter
     window than a grant's rested up to an hour late
     (`rate_gate::a_waiter_with_a_shorter_window_is_woken_when_the_grant_ages_out_for_it`, red before).
  6. *The purge (transaction T25):* the housekeeping deletes, a key a transaction (window row locked first), the leased grants older
     than their window row's longest period plus one minute (`max_period_ms`, FR-022-056: exactly at that instant kept,
     one microsecond later deleted); the empty window goes with its last row. Taken as the approved design over keeping
     every grant an hour (up to 3,600 times the rows of a per-second key); the cost is G022-017.
  7. *A reset of a start that declares `rateLimit`* is refused `BINARY_CAPABILITY_MISSING` until T022-086 builds
     `RATE_LIMIT_SLOT_UNAVAILABLE` with the reset rules of the `rate_limit.*` events (G022-016): one change, not two,
     over a refusal code with no reset rules behind it.
  8. *The test-only door is closed:* `BUILT` lists `RateLimits`, so `serving_rate_gates_for_tests`, the
     `rate-gates-for-tests` feature and their architecture rule are removed (D022-057's condition met).

- **D022-065 — What the reviews of the rate gate changed (2026-11-20; `reviews/2026-11-20/rate-gate-*.md`, responses in
  `rate-gate-responses.md`).** Delegate (OD-2026-11-20-6) for every choice below.
  1. *One instant per decision* (correctness Major 2): every count of a decision (the readiness gate's and the grant
     routine's, deadlines included) is taken at the instant the decision journals, read once after the keys' locks, so a
     grant that ages out between that read and a count can no longer let a second grant into the window it lies in.
  2. *Never late* (correctness Major 3, Major 4, minor 2; architecture on item 5): the rest uses the pass's start (the hint it was
     selected by) instead of its own clock; a rate grant to a window with waiters moves the hint (only then, so the
     uncontended path costs nothing more); the lease pass lowers the hint of each window it recorded. D022-064 item 5 is
     revised accordingly; the claim "exact" is withdrawn: the hint is early, and never later than the slot frees.
  3. *A full key is named by its own kind* (correctness minor 1, architecture Major 6): `CONCURRENCY_QUEUE_FULL` for a dual-gated
     start whose permit queue is full; `RATE_LIMIT_QUEUE_FULL` for its full slot queue or a full namespace.
  4. *Live windows* (security review, Major 1): `rate_limit_live_keys` counts a window while it has a waiter or a grant that can
     still count (unleased, or leased within its longest period), in the gate and in the quota usage, so one tenant's
     purge backlog never refuses another tenant's keys. *Reversed by D022-066 item 1 (the re-reviews): the rows are
     counted again and a decision reclaims its own namespace's purgeable windows.*
  5. *Bounded passes* (security Major 2 and minor 6, architecture Major 5 in part): the lease pass probes the journal only for
     a step whose projection says the attempt was leased; the purge skips a key another transaction holds and deletes at
     most 500 grants a key a transaction, and is counted (`kind="rate_purge"`). Driving the purge by key shards with an
     index on the lease instant is left to T022-062/089 (the benchmark), G022-018.
  6. *The test door is guarded* (architecture Major 2, security minor 4): `purge_rate_grants_as_of` stays (the boundary needs
     an exact instant on the database clock) and `scripts/check-architecture.py` refuses a production call of it outside
     the purge itself; the gate-row-writer rule names the lease and purge statements as the two non-decision writes.
  7. *Kept, with a release gate* (correctness Major 5, architecture Major 1): a reset of a rate-limited start stays refused
     `BINARY_CAPABILITY_MISSING` until T022-086 builds `RATE_LIMIT_SLOT_UNAVAILABLE` with the reset rules of the
     `rate_limit.*` events (G022-016); the runbook forbids production activation before T022-080, T022-081, T022-086 and
     T022-104. Building only the refusal now would ship a reset code with no reset rules behind it.
  8. *Left open, recorded* (architecture Major 3, Major 4): the cross-run consistency of the rate tables (`key_consistency`) and the
     lock-order barrier tests are T022-079 and T022-081.

- **D022-066 — What the re-reviews of the rate gate changed (2026-11-20; `reviews/2026-11-20/rate-gate-rereview-*.md`,
  responses in `rate-gate-rereview-responses.md`).** Delegate (OD-2026-11-20-6) for every choice below except item 6,
  which is the owner's.
  1. *`rate_limit_live_keys` counts window rows again, and a decision that would be refused reclaims its own namespace's
     purgeable windows first* (architecture Blocker 1, security Major 1 and correctness Major 2 of the re-reviews; this
     reverses decision D022-065 item 4). Counting only windows that still count (item 4) let a namespace revive aged
     windows past the ceiling without a check, and left the rows bounded by nothing but the purge's throughput.
     Weighed: (a) keep the live count and put the same predicate into the existence check (the bypass closes, the rows
     stay unbounded); (b) the live count and a second cap on all rows at twice the ceiling (bounded, but a namespace over
     the cap waits for the deployment-wide purge, so another tenant's backlog refuses it again, the first review's
     security Major 1); (c) count the rows, as decision D022-064 item 1 defined the resource and as the semaphores are
     counted, and let a deciding transaction that lacks room delete, before it decides, the grants of windows of its own
     namespace that only grants the purge may delete keep; (d) count the rows and wait for the purge (the first review's
     security Major 1). Chosen (c): one unit for both kinds of key, rows bounded by the ceiling (plus the owner's
     overshoot of item 6 and a forced retirement's rows, section 4 of the runbook), and no admission waits for another
     tenant's purge backlog. Bound: a row is created only while the rows counted are below the ceiling, and a reclaim
     removes only rows the purge could delete, so the rows of the versions in force never pass the ceiling by more than
     the overshoot of concurrent creations. The reclaim runs only when the count would refuse: it locks (`FOR UPDATE SKIP
     LOCKED`, so it never waits and cannot deadlock) at most as many windows as the decision lacks room for, oldest
     update first, each with no waiter, at most 64 grants, every grant one the purge may delete, and not a key the
     decision files; it deletes their grants in a second statement whose snapshot is taken under those locks, and the
     windows go at the commit with their last grant. The predicate of a grant the purge may delete is one text for the
     purge and the reclaim (`purgeable_grant!`), so the purge's boundary test covers both. A window of more than 64
     grants waits for the purge (gap G022-018). Business effect: a namespace keeps up to its ceiling of keys that
     FR-022-056 obliges the platform to keep (in use, or within their longest window and one minute after it); a
     namespace that creates a new per-second key for every start sustains its ceiling divided by 61 seconds of new keys
     (164 a second at the default 10,000, 1,639 at the hard 100,000). The quota usage counts the same rows.
  2. *A rest reads the grants after it holds the window* (correctness Major 1 of the re-review): a rate key's rest is a
     transaction of two statements, the window row locked if its hint is still the one the pass was selected by, then
     the hint computed by a new statement, whose snapshot holds every grant committed before the lock was granted (a
     grant is written only under its window's lock). A single `UPDATE` re-checked its compare-and-set on the new row
     version but computed from its old snapshot, so a grant whose `least` left the hint unchanged was lost and the window
     rested with no hint. Taken over a second compare-and-set field (`updated_at`), which every key selection would carry.
  3. *The rest's instant is the selection's* (architecture minor 1, correctness minor 1, security suspicion 1 of the
     re-reviews): a selected key carries `selected_at`, the instant of the statement that selected it (at or after its
     hint, at or before every count of its pass), and an unleased grant rests its key until that instant plus each
     waiter's window. The hint the key was selected by fell behind real time after a scheduler lag, so a key was visited
     once per aged grant.
  4. *A hint is never set to the end of time* (correctness nit of the re-review): a grant or a recorded lease moves the hint
     with `least(hint, instant)` (PostgreSQL's `least` ignores a null), so a waiters counter that has drifted keeps the
     hint instead of writing `infinity`.
  5. *The open gates bind where they are read* (architecture Major 2, correctness minor 7, security minor 4 of the
     re-reviews): runbook section 6 forbids production before tasks T022-079, T022-080, T022-081, T022-086 and T022-104
     and gap G022-018; gap G022-016 is a release gate; tasks T022-079, T022-081 and T022-086 say so; the interim
     refusal of a reset of a rate-limited start is pinned by
     `run_creators::a_reset_of_a_rate_limited_start_is_refused_until_its_rules_are_built`.
  6. *The rate count's overshoot is the owner's* (OD-2026-11-20-11, "Yes, extend it"): `rate_limit_live_keys` is read
     without a namespace lock, as `concurrency_live_keys` is (G022-002); a run's start counts under the namespace's
     admission lock, a readiness round under none. This closes suspicion 4 of the first security review and of its
     re-review.

- **D022-067 — What a reader sees of the rate gate (task T022-078; FR-022-048; plan "Visibility of the rate gate";
  2026-11-20).** Delegate (OD-2026-11-20-6) for every choice below.
  1. *`Run.rateLimit` is `GRANTED` while the run's slot counts* (`leased_at > t - per` at the read's instant), and absent
     after: a run-level slot is never given back and its row stays until the purge, so a state read from the row's
     existence would show a slot that no longer limits anything for up to the window and one minute more. Taken over
     "while the row exists" (misleading) and over a third state (`EXPIRED`, a value no client needs: the journal has it).
  2. *The position ranks the waiters the pass visits ahead of the run* (not past their deadline, their run live and not
     paused, in the queue's order), and the estimate is the `k`-th oldest counted lease plus the window with `k = n + a
     - L + 1`, derived in the plan, `null` beyond one window or while the run also waits for a permit. Taken over a rank
     of every waiter (it counts waiters past their deadline or paused, which the pass skips) and over the grant
     routine's full predicate (`admits` also skips a waiter whose own limit is below the count at that moment: read at
     a full key it ranks every waiter first, as the first build of this read showed), and over an estimate from an
     average rate (not derivable, D022-014).
  3. *The listing lists every window row* (the unit `rate_limit_live_keys` counts, D022-066 item 1), so an operator sees
     exactly the keys a ceiling counts; `nextSlotAt` is the first instant a counted grant stops counting for one of the
     waiters (each waiter's own window, the rest rule's instant at `t`), `null` when nobody waits (nothing waits for a
     slot) or a grant is unleased (it frees by a lease or a return, not by time). The contract's earlier text ("null when
     the window is not full") named a fullness no single limit defines, since each waiter has its own.
  4. *The listing's order is kept on the row* (`oldest_wait_at`, maintained by the window's count trigger) and served by
     two indexes, as decision D022-058 did for the semaphores, so a page costs its own keys; a key's
     `grantsInWindow` adds at most its counted grants (the hard ceiling of its longest window, 10,000).
  5. *Its cursor is of its own kind* (`r1`), sealed as the concurrency listing's (D022-059): a cursor of one listing
     is a `400` in the other, so neither can be replayed to probe the other's order.
  6. *Instants that derive from a run's own instants are truncated for a reader without the reveal grant*:
     `oldestWaitSince` (a waiter's `since`) and `nextSlotAt` (a grant's instant plus a window), to their minute toward
     the past, as D022-060 does for the concurrency listing.
  8. *The estimate is shown only to a caller with the reveal grant* (security review Major 1 of task T022-078): at run
     scope it is another run's grant instant plus the window, to the microsecond, and that run's `since` is shown whole
     by `listRuns`, so it would tell a caller who may not see the label that the two runs share a key (the linking
     D022-060 and D022-061 narrow). Taken over truncating it to the minute (it only narrows the link, the other run's
     `since` stays whole). The position stays for every reader of the run: it says how many runs of the key are ahead,
     the class of gap G022-014, recorded there.
  7. *A step's wait is named in the runtime graph* (`waitingOn` `rate_limit` or `concurrency`), the gate its row records
     (rate first, FR-022-052): two values added to a response enumeration, which older clients read as unknown.
- **D022-068 — The rate reads after their reviews (task T022-078; reviews `rate-reads-*` of 2026-11-20; plan
  "Visibility of the rate gate").** Delegate (OD-2026-11-20-6) for every choice below.
  1. *No estimate for a run the pass will not grant* (correctness minor 2): `estimatedGrantAt` is `null` when the
     reading run is paused, when its own deadline is at or before the read's instant `t`, and when the computed
     instant is at or after its deadline (the pass grants a waiter only while `deadline_at > now()`, so at the deadline
     exactly it times out first). The position stays: it is where the run stands. Taken over showing the instant with a
     flag (a second field no client needs) and over a `null` position (it hides an exact count the run's reader has).
  2. *The place is read in its own read-only transaction with a statement timeout of 2 s, and a failure of that read
     leaves `position` and `estimatedGrantAt` `null`* (architecture minor 5, security minor 3, correctness nit 3): an
     estimate is never a promise, so it must not turn `getRun` into an error; the failure is logged as a warning with the
     run's id and the error's kind, never a key (no new series: the read is not a gate decision). *Revised by the
     re-review:* only an unavailable database (a statement over its timeout, a lock wait it ended, a lost connection,
     the pool's limit) is left out this way; a corrupt row stays an error, as every other read reports it. A run granted between the run's row and its place is shown `WAITING` with no place, which the
     contract now says. Taken over failing the read (an operator's view lost to a busy key) and over a second
     snapshot-sharing read of the run's row (it doubles the run read for a value that is an estimate).
  3. *`nextSlotAt` reads one index probe a period, over the periods of the first 256 waiters in the queue's order*
     (architecture Majors 1 and 2, security minor 2): `min(leased_at) + p` over `leased_at > t - p` (the same half-open
     window, written so `rate_limit_grants_count_idx` serves it), and the periods of at most
     `GRANTS_PER_KEY_PER_PASS` (256) waiters through `rate_limit_waiters_order_idx`, the most one pass grants a key. A
     page then costs at most `limit + 1` keys, each at most three grant probes (three periods exist), 256 waiter rows
     and its counted grants, never the purge's backlog or the key's whole queue. The first 256 rows are not exactly the
     256 the pass visits (the pass skips paused, ended and expired waiters, which the listing does not join to bound its
     cost), so with skipped waiters among them `nextSlotAt` can name a later instant than the pass will act on; the
     contract says "the first 256 waiters in the queue's order", which is what it reads. Taken over the earlier scan of every grant and every waiter of the key (it reaches the 2 s
     timeout at a busy step key, the moment an operator needs the view), and over dropping `nextSlotAt` (the operator's
     "when does this queue move" is the field's purpose). Pinned by the `EXPLAIN` twin of the concurrency listing's
     page cost, with a step key of more waiters and aged grants than the bound.
  4. *The listing shows every window row, as the quota usage of `rate_limit_live_keys` counts them* (correctness
     minor 6): the gate leaves out the rows of a forcibly retired digest version, which drain on their own; the listing
     keeps them so an operator sees the drain. The contract said "the rows the ceiling counts"; the text is corrected,
     not the query.
  5. *`rate_limit_windows.oldest_wait_at` is checked against its waiters at the end of every flow-control test*
     (architecture Major 3, correctness Major 3), as D022-058 checks the semaphores' column, with a tamper test and a
     principles row.
  6. *Recorded, not changed in this task* (minors): one text of the pass's eligibility predicate for its five uses
     (architecture minor 4) and a gate-neutral permit state (architecture minor 2), task T022-112 (a refactor
     task of its own, after the re-review: the fault suite is no home for it); one handler for both listings (architecture minor 3) with task T022-111, which reshapes the
     listings for callers without the reveal grant. Each is a refactor of code that is correct today.
  7. *After the re-review of the fixes* (reviews `rate-reads-rereview-*`): the store returns what the statement read
     (`PlaceReading`) and the run service decides the estimate from it, with the reveal and dual-gate rules, so one layer
     decides the field (architecture minor 5); the reading's instant is the statement's (`statement_timestamp()`), not
     its transaction's (correctness minor 4); the waiters' bound of `nextSlotAt` is one text (`grants_per_key_per_pass!`)
     whose value the build checks against `GRANTS_PER_KEY_PER_PASS` (architecture minor 1); the place's count of counted
     grants reads only `leased_at` at run scope, where it is never null (security minor 2). The pass's `NOT r.paused` is
     kept in the place's waiters ahead although no black-box test can tell it from the status filter (a pause stores the
     run `PAUSED` and the flag together): the text stays the pass's, so the two cannot drift apart.
- **D022-069 — A reset of a rate-limited start takes a fresh slot right after `run.reset`, or is refused, and never
  waits (task T022-086; FR-022-041, FR-022-038; `contracts/journal-events.md`, reset rules; closes gap G022-016 when
  the task is ticked; 2026-11-21).** Item 1 is the approved contract; items 2 to 5 are delegate (OD-2026-11-20-6), each the rule
  D022-053 set for a permit, applied to a slot.
  1. *The reset rules of the five `rate_limit.*` events* are installed as `contracts/journal-events.md` states:
     `queued`, `granted` and `returned` `REMATERIALIZE` (history), `wait_timed_out` and `dequeued` `REFUSE` to the end
     of the journal. A point while a start waits for its slot is no quiet state already (`NotRunning`), and a point with
     a step waiting for a slot, or granted and not yet completed, is `OpenWork`, as for a permit.
  2. *The history stays out of the new run* by the mechanism of D022-053 item 2, which serves both gates: the reducer
     clears the start's gates at `run.reset` and retires the restored slots of steps, `persist` writes no row for a
     restored prefix, and a restored `rate_limit.granted` creates no grant row and counts in no window.
  3. *The fresh decision* is the one decision of both levels and both gates (`decide_filed`, D022-064 item 2) with the
     cause `Reset`, right after `run.reset`: the start is granted only when every gate it declares admits it under the
     reservation rule (counted grants and waiters ahead below the limit, D022-021), so a reset never takes a slot a
     buffered run waits for. A granted slot counts from its grant (`leased_at = granted_at` at run scope). The source's
     own slot still counts in its window: a reset of a source whose grant is younger than its window, on a key at its
     limit, is refused; the runbook says so. *After the reviews of 2026-11-21 (architecture Major 1, security minor):*
     cancelling a source (`CANCEL_SOURCE`) frees no slot, so on a key near its limit such a reset cancels its source and
     then fails; the runbook warns and advises `REQUIRE_TERMINAL`. Weighed: a best-effort refusal of a `CANCEL_SOURCE`
     request whose window is full when it arrives (advisory only: the window may empty or fill before the source ends;
     left open as task T022-114), and the reset run taking over its source's slot (it would contradict the approved
     "fresh slot" rule of `contracts/journal-events.md`, so it is the owner's to decide; not proposed).
  4. *The refusal, rate first (FR-022-052):* when the rate gate does not admit the reset, `409 CONFLICT` with
     `details.reason` `RATE_LIMIT_SLOT_UNAVAILABLE`; when only the permit does not, `CONCURRENCY_PERMIT_UNAVAILABLE`
     as before. `409` and not `429`: like the permit's, the refusal is a state of the key that a later retry may find
     changed, not a quota of the namespace (018's contract class for a reset). A pending reset (`CANCEL_SOURCE`) fails
     its entry with that reason: it joins `FAILURE_REASONS` and the `failure_reason` check of `run_resets` (migration
     `20270209000000`, added `NOT VALID` then validated; its down refuses while an entry carries the reason); a batch
     item answers `CONFLICT`. Each family the start declares counts the refusal `refused_reset` (a rate-only reset no
     longer counts a concurrency refusal it did not have).
  5. *The interim refusal is removed:* a reset of a rate-limited start is no longer `BINARY_CAPABILITY_MISSING`; that
     code stays for a process without the digest key ring, and for a definition with a `forEach`, `while` or
     `repeatUntil` node (G1197). Proved by `run_creators::{a_reset_takes_a_fresh_slot_right_after_run_reset,
     a_reset_without_a_free_slot_is_refused_rate_first_and_never_waits,
     a_pending_reset_takes_a_fresh_slot_or_fails_rate_limit_slot_unavailable,
     the_reset_rules_of_the_rate_limit_events_are_the_contracts}`, which replace
     `a_reset_of_a_rate_limited_start_is_refused_until_its_rules_are_built`.
  6. *The capability stays `FlowControlRateV1`* (architecture minor 3 of 2026-11-21): a build that predates this task
     cannot read a restored `rate_limit.*` event, and none was released (feature 022 is in no release), so the change
     needs no new capability version; the runbook asks for the rolling upgrade to finish before such a reset. *After
     the re-reviews of 2026-11-21 (architecture minor F, security minor):* this task ships in the same release as
     `FlowControlRateV1`, and no engine is rolled back past it once a rate-limited run was reset (the older build cannot
     decode the reset run's journal); the runbook says so in section 6.
  7. *The other run creators of task T022-064 with a rate gate* (correctness Major 3, architecture Major 2 of
     2026-11-21): every creator starts its run through `Engine::start`, which `scripts/check-architecture.py` enforces
     with the functions `run_start_errors` (one start point, D022-049) and `run_insert_errors` (one admission point),
     and `rate_creators.rs` proves an event, a cron tick, a schedule's action, a child and a delayed start; the rate
     twins of the start under a business identifier, the rerun, Signal- and Update-with-Start, the batch rerun and the
     ad hoc run are task T022-113, split from T022-086, whose text is amended to the reset. *After the re-reviews of
     2026-11-21 (correctness minor 1, nit 1; architecture minor D):* the function cited first, `run_creator_errors`, is
     design 043's rule and was the wrong one; and T022-113 joins the production gate of the rate capability (runbook
     section 6), because before the split those cases were part of T022-086, which gates it: the split moves work, it
     does not lower the gate.
  8. *A `CANCEL_SOURCE` reset on a full key* (re-review of 2026-11-21, architecture minors B and C): the trap of item 3
     is stated in the two contracts a caller reads (`resetRun` of 001's OpenAPI and the reset of 018's REST fragment),
     and its follow-up T022-114 is gap G022-019. T022-114 does **not** gate production: the strategy is off by default,
     the outcome is recorded (the entry ends `FAILED` with a closed reason, a `run.reset` audit record, the
     `workflow_reset_completions_total` failure count), the same class of outcome is accepted for the permit (ADR-0081),
     and the advisory refusal it builds cannot turn the outcome into a promise. Delegate (OD-2026-11-20-6).
- **D022-070 — The futures a gate adds to a run creator are boxed where they are created (2026-11-21; CI of #237 at
  `a443a04c`, a stack overflow of a feature 018 test).** Delegate (OD-2026-11-20-6, coordinator's instruction). A batch
  item's rerun overflowed the 2 MiB test thread of a debug build on x86-64: the gated start (`Engine::start`) and the
  admission it follows made every future on a creator's path larger, and a debug build keeps each future a frame creates in
  that frame. Measured with `size_of_val` and by bisecting `RUST_MIN_STACK` (plan, "Stack and future sizes of a run
  creator"): `BatchExecutor::step` held an item's whole use case inline (23,952 bytes), `RunService::start` the run's
  admission (11,136), `Engine::start` its gated part (6,256); the test needed 2,080 KiB. Boxed where each is created
  (`Box::pin` at the call of the item's use case and of `RunStore::create`; the gated part of `Engine::start` built inside
  a function that returns it boxed), the sizes are 2,040, 7,624 and 5,496 bytes and the test needs 1,418 KiB. Taken over a
  larger `RUST_MIN_STACK` or test-thread stack, which would hide the growth while production threads have the same edge,
  and over boxing every future (one allocation per command on the hot path for no measured need). The develop build was
  not measured beside it: develop has no `Engine::start`, and the second full build did not fit the disk; the sizes before
  the boxes are this branch's, measured just before the change. Pinned by `future_sizes::the_futures_of_a_run_creator_stay_below_their_bounds`. The one-start-point rule of
  `scripts/check-architecture.py` accepts `start_gated` beside `Engine::start`, since it is that function's gated part.
- **D022-071 — The grant from the buffer applies the namespace's current ceiling to every rate waiter (2026-11-21;
  task T022-080; D022-009; spec, edge cases of US07: "a lowered policy applies to waiters at once").** Delegate
  (OD-2026-11-20-6); it carries out D022-009, it does not change it. The fault test
  `rate_limits::a_lowered_policy_ceiling_slows_later_grants_and_revokes_none` found that the grant routine
  (`gate_queue::admits`) compared a waiter with the limit filed in its waiter row, the effective limit of its admission:
  with `limit: 3, per: second` filed and the namespace's `rate_limit_per_second` lowered to 1 while two starts waited, the
  two were granted 856 ms and 114 ms apart, two grants in one second against a ceiling of one. Now:
  1. *The waiter itself:* the routine reads the namespace's ceilings (`quotas::step_gate_ceilings`, one statement of the
     grant's transaction, only when the waiter waits for a slot) and admits it when its key's counted grants plus the
     eligible waiters ahead are below the lower of its filed limit and the current ceiling of its window length.
  2. *The waiters ahead* count with the same rule: a waiter ahead is eligible when the lower of its own filed limit and
     the ceiling of its own window length is above the key's counted grants (no head-of-line blocking, D1196, with the
     policy applied).
  3. *Nothing is revoked and nothing is rewritten:* the waiter row keeps its filed limit (a waiter row is never changed),
     a grant already made counts as it was; a raised ceiling never lifts a waiter above its filed limit, which is the lower
     of its declaration and the ceiling at its admission (review of 2026-11-21, minor 3: so a ceiling raised while it
     waits is not applied to it, an under-delivery; the grant's journal records the filed limit, not the one it was
     decided with; both open with the review's answers). *Answered 2026-11-21 (decision D022-075 item 7, minor 3):* the
     grant row and `rate_limit.granted` record the limit the grant was decided with; a raised ceiling stays unapplied to
     a waiter already filed. *Reversed 2026-11-24 (decisions D022-076 item 7 and D022-078 item 1):* a raised ceiling
     applies to a waiter already filed, up to its declaration.
  4. *Concurrency is unchanged:* the same question for a lowered `concurrency_limit` while runs are queued is not settled
     by the specification (its edge case speaks of a new version's lower limit only); left open as gap G022-020. *Decided 2026-11-21 (decision D022-074):* a lowered
     concurrency ceiling applies to the queued runs too.
- **D022-072 — The database clock set back is tested by moving the counted instants, not with `libfaketime`
  (2026-11-21; task T022-080; amends the test method of D022-027).** Delegate (OD-2026-11-20-6). Every decision of the
  rate gate compares the stored instants of a key's grants with `clock_timestamp()`, so a database clock set back by one
  window is, for those decisions, the same as the key's counted instants moved one window ahead; the fault test moves
  the grant rows' instants (as the owner, with the guard that forbids it switched off in that one transaction) and moves
  them back before the consistency check, which compares them with the journal. Weighed: `libfaketime` in the test
  container (D022-027's method) needs an image of our own beside the pinned `postgres:16-alpine` (rule 16 of `AGENTS.md`)
  and shifts the clock of every database of the container at once, which the suites share; a clock port would test the
  port (D022-027's own objection). What this does not exercise: a path that reads the clock for something other than
  counting grants (the rest hint and the queue deadline keep the true clock), recorded in `rate_limits.rs`'s module text.
- **D022-073 — Strict admission order: a new arrival queues behind an eligible waiter even when its key has room
  (2026-11-21; task T022-080, Major 5 of its review; SC-022-017, SC-022-004, FR-022-042; ADR-0089 decision 4).**
  Delegate (OD-2026-11-20-6); the coordinator's decision 1. The review found that admission granted a start at once
  whenever the key's counted grants plus all its waiters were below the start's limit: with `limit: 10, per: second`,
  starts 1 to 10 granted, 11 to 14 queued and a second gone by, start 15 saw `0 + 4 < 10` and was granted while 11 to
  14 waited for the pass, so the journals' grants did not follow admission order, which success criterion SC-022-017
  requires (and SC-022-004 for a permit). Options: (a) keep the reservation rule at admission and reword the success
  criterion (an approved text: the owner's); (b) strict admission order: a new arrival queues whenever its key has an
  eligible waiter, and the grant pass grants in order; (c) grant the waiters ahead inside the admission's own
  transaction (it would lock other runs inside a creator's transaction, against ADR-0089 decision 3's one routine).
  Chosen (b), as the approved criterion says:
  1. *The rule.* At admission (both gates, both levels, the one decision `decide_filed`), a gate admits only when its
     key has no eligible waiter and its counted holders or grants are below the acquirer's limit. Eligible is ADR-0089
     decision 4's word: not past its deadline, of a live run that is not paused, and admitted by its own limit, which is
     the lower of its filed limit and the namespace's current ceiling (decisions D022-071 and D022-074), counted over its
     own window (decision D022-075 item 4). A waiter refused by its own lower limit does not hold a new arrival back
     (D1196 stands: `queue::a_step_behind_only_an_ineligible_waiter_is_granted_at_once`, which under the counting of
     every waiter was queued and granted by the next pass); one refused only by its other gate does, since it keeps its
     room. This narrows ADR-0089 decision 4,
     it does not depart from it: with `p` eligible waiters ahead, a new arrival was granted only when `taken + p < L`;
     now also only when `p = 0`.
  2. *The grant pass keeps the reservation rule* of ADR-0089 decision 4 (accepted by the owner) and tries the waiters of
     a key in admission order; every decision of one pass counts the key's grants as of the instant the pass selected
     the key (`GateKey::selected_at`, at or before each decision), not at its own instant. Found by the flood under load:
     a pass refused a waiter, two counted grants aged out while it went on, and a younger waiter behind it found room for
     itself and the refused one (the reservation) and was granted first. Counted as of the selection, no grant ages out
     within a pass, so a waiter the rule refuses refuses every younger one of the same limit and window, and the next
     pass grants them in order. It never over-grants: an earlier instant counts every grant made since and keeps those
     that aged out meanwhile; the cost is that a grant aging out during a pass is used by the next one. The queue deadline
     is still checked at the decision's instant. Proved by
     `rate_limits::a_grant_that_ages_out_during_a_pass_never_puts_a_younger_waiter_first`. The exceptions are the
     accepted decision's own: a waiter skipped because another transaction holds its run (`Busy`) keeps its room and is
     granted by a later pass, after younger ones; and a waiter of two keys may be granted by its other key's pass
     (ADR-0089 decision 3), before older waiters of the first key, whose room it leaves.
  3. *Cost in statements.* The eligibility is read only when the key has waiters and the acquirer's semaphore or window
     has room: one statement, the key's waiters in queue order up to the first eligible one. An uncontended admission
     and one refused by a full key read nothing more, so no line of the hot-path budget moves (`plan.md`, "Hot-path
     budget of a gated definition").
  4. *The hint.* A start queued while its key has room lowers the key's next-eligible hint to the statement's instant
     (`least(hint, now)`), so the next pass visits the key; a start queued at a full key keeps the hint it found (early
     or the age-out instant, both never late). *Amended 2026-11-24 (decision D022-076 items 1 and 2; re-review Majors 1
     and 2):* a start or step queued while its key has room writes the key's hint to the statement's
     `clock_timestamp()` whatever the hint was (`least` kept a hint a pass in progress had selected, and the pass's rest
     then lost the new waiter's wake-up); one queued at a full window lowers the hint to the earliest instant a counted
     grant ages out of its own window (`least(hint, that instant)`), so a long window's hint never holds back a short
     one; a semaphore queued full keeps the hint it found, a hint absent becomes now.
  5. *The delay it costs a new start* (derived in `plan.md`, "The grant pass, by key"): at most `1.2 I + 2 D`, with `I`
     the scheduler's idle pause (`SAGAT_SCHEDULER_INTERVAL_MS`, 500 ms by default, jittered by up to 20 %) and `D` the
     duration of one tick; one tick more when more keys are due than one pass visits (the two passes of SC-022-017).
     Never a window: the pass grants the start under the counts that would have admitted it, unless a waiter that was
     not eligible at its admission became eligible meanwhile (a resumed run), which is then ahead of it by admission
     order. Tested by `run_admission::a_start_behind_an_eligible_waiter_queues_although_the_key_has_room_and_one_pass_grants_both_in_order`
     (a permit), `rate_gate::a_start_behind_an_eligible_waiter_queues_although_the_window_has_room_and_the_next_pass_grants_it`
     (a slot, with the window rested an hour ahead: the hint drops to the start's enqueue instant) and the flood of
     `rate_limits` (the fifteenth start), each red before the change.
- **D022-074 — A lowered namespace concurrency ceiling applies to the runs already queued (2026-11-21; gap G022-020;
  task T022-025; FR-022-021; ADR-0056 decision 7).** Delegate (OD-2026-11-20-6); the coordinator's decision 2, as
  decision D022-071 did for rate limits. A grant from the queue evaluates a permit waiter against the lower of the
  limit filed in its waiter row and the namespace's current `concurrency_limit`, and counts a waiter ahead with the same
  rule; admission already applied the current ceiling. Held permits are never revoked (ADR-0056 decision 7; task
  T022-025's `lifecycle::a_lowered_limit_revokes_nothing_and_applies_to_the_next_start`), and the holder row records the
  limit its grant was decided with. Weighed: keeping the filed limit (a namespace that lowers its ceiling to protect a
  dependency would wait until every run queued under the old limit is granted, up to the hard bound of 10,000 run
  waiters a key, before the new ceiling takes effect) and revoking held permits (an attempt in flight would lose its
  permit, which ADR-0056 forbids). Cost: the grant of a permit waiter reads the namespace's ceilings, one statement of
  the grant's transaction (it was read only for a slot waiter until now); no budget line covers the grant pass. Closes
  gap G022-020 with `lifecycle::a_lowered_concurrency_ceiling_applies_to_the_queued_starts_and_revokes_no_permit`.
- **D022-075 — The answers to the correctness review of task T022-080 (2026-11-21;
  `reviews/2026-11-21/rate-faults-correctness-reviewer.md`, 0 Blockers, 6 Majors).** Delegate (OD-2026-11-20-6). Each
  Major has a test seen failing before its fix, or a hand mutation where the test can only follow the code:
  1. *Major 1 (rounds 0 and 1 of the timeout race never reached the grant).* The race runs in four orders of five rounds
     each, every one reaching the decision it names: (0) the grant lists the start before its deadline (three seconds
     ahead, minor 6), holds its run and waits for the window; the deadline passes and the timeout waits for the run; the
     grant's check of the deadline under the window's lock refuses it. (1) The grant lists the start before its deadline
     and finds its run held, as by any transaction of the run: it reports the start skipped (`skipped == 1`) and the
     timeout decides after the deadline. (2) The grant commits before the deadline. (3) The grant has two starts of one
     key in hand and waits for the window on the first; the second's deadline passes and its timeout commits; the grant
     then finds the second timed out and grants the first only.
  2. *Major 2 (the set-back clock never decided while the moved grant was in the future).* The holder's grant is moved
     five windows ahead and a pass runs at once, with the database clock read below the moved instant before it; it
     grants nothing, asserted without a condition; the waiter's grant is no earlier than the moved instant plus one
     window.
  3. *Major 3 (the place estimate and the run view used the filed limit).* `RATE_PLACE_SQL` and the run view's `limit`
     use `L`, the lower of the waiter's filed limit and the namespace's current ceiling of its window length, read in
     the estimate's read-only transaction; `rate_rest::a_lowered_ceiling_moves_the_estimate_and_the_limit_a_waiting_run_shows`.
  4. *Major 4 (the waiters ahead were counted over the deciding waiter's window).* The grant routine and admission
     count the key's grants once for each window length up to the window row's longest period (`max_period_ms`, which
     is at least every waiter's period), and a waiter ahead is eligible when the lower of its own filed limit and the
     ceiling of its own window length is above the count of its own window. For a key with one period nothing changes;
     `rate_limits::a_waiter_of_a_longer_window_refused_by_its_own_limit_does_not_hold_back_a_shorter_one` mixes an hour
     and a second on one key.
  5. *Major 5 (the flood's admission order on a slow runner).* Decision D022-073. The flood is deterministic: the first
     ten starts are granted at admission, the window is aged on the database clock while four more wait, the fifteenth
     queues although the window has room, and every later start queues until the key's ceiling of 100 waiters, then is
     refused: exactly 10 granted at admission, 100 buffered, 890 refused, and the pass grants in admission order.
  6. *Major 6 (every crash came before the decision's first write).* Each of T20, T21 and T24 has a second crash point
     after its gate writes: the test holds an uncommitted journal row at the run's next sequence number, so the deciding
     transaction, its grant, waiter and window rows written, waits on the journal's primary key; it is killed there, and
     grant rows, waiter rows, the window's counters and the journal are as before.
  7. *Minors.* (1) The cancellation race asserts the run cancelled and the slot row each order leaves (a run's slot is
     leased at its grant, so a granted then cancelled start keeps its grant row until it ages out; a dequeued one has
     none). (2) The lease-recording crash uses `per: second` and asserts the waiter's grant no earlier than the recorded
     lease plus one second. (3) The grant from the queue records the limit it was decided with (the lower of the filed
     limit and the current ceiling) in its grant row and in `rate_limit.granted`; decision D022-071 item 3 is corrected:
     a ceiling raised while a waiter waits is not applied to it, its bound being its filed limit (its declaration
     clamped at its admission), an under-delivery and never an over-grant; the specification's "its own pinned `limit`"
     (US07, edge cases) is read as that filed limit, reported to the owner with this decision (*reversed 2026-11-24 by decisions D022-076
     item 7 and D022-078 item 1:* the bound is the lower of the declaration and the current ceiling, and the phrase is
     read as the declaration the version pins). (4) Done before this
     answer: `plan.md` and task T022-080's amendment line name the method of decision D022-072. (5) The second clause of
     SC-022-017 (a second saturated key, more keys and tenants than one pass visits) is new task T022-115. (6) Done in
     item 1. Nit 1 (the ceilings read once per waiter tried) is kept: reading them once a pass would carry a value read
     outside the grant's transaction into it, and the statement is one primary-key range of `quota_limits`.
- **D022-076 — The answers to the re-review of task T022-080 (2026-11-24;
  `reviews/2026-11-24/rate-faults-rereview-correctness-reviewer.md`, 0 Blockers, 3 Majors, 7 minors, 2 nits).**
  Delegate (OD-2026-11-24-4), which also settles review findings and ADR text. Each item below is built with a test
  seen failing first, or with a hand mutation where the test can only follow code that is already right.
  1. *Major 1 (a start queued during a pass loses its wake-up).* A start or step that queues while its gate has room
     (it queues only behind an eligible waiter) always writes its key's next-eligible hint to the statement's
     `clock_timestamp()`, whatever the hint was. The pass's rest compares the hint with the one it selected, so a hint
     written during the pass makes the rest fail and the key stays due; the next pass grants the new waiter. Weighed:
     re-listing the key at the end of each pass (a second read per key on every pass, and still a window between that
     read and the rest) and taking the key's lock in the rest (a lock in a path that today takes none, against
     ADR-0089 decision 3's one routine). The written hint costs nothing new: the statement exists. Decision D022-073
     item 4 is amended to this wording.
  2. *Major 2 (mixed windows keep a long hint).* A waiter that queues at a full window lowers its key's hint to the
     earliest instant at which one of the key's counted grants ages out of the new waiter's own window
     (`least(hint, that instant)`), so a short-window waiter is never held behind a hint computed for a longer window;
     a key with one period computes what it computes today. Weighed: "due now" for every new waiter (one wasted pass
     visit per new waiter of a full key, which is exactly the load a saturated key brings) against the age-out over the
     waiter's own period (one aggregate over rows the statement has already locked and counted).
  3. *Major 3 (a reset refused while its key has room).* A start cause that may not queue (a reset, decision D022-069;
     a pending reset; a reset of a permit, decision D022-053) is decided by the reservation rule of ADR-0089 decision
     4, not by strict order: it is granted when its key's counted holders or grants plus the eligible waiters ahead of
     it are below its limit, and refused otherwise. FR-022-041 and FR-022-018 refuse a reset only without a free slot
     or permit, and the reservation leaves room for every eligible waiter, so no waiter is overtaken in a way that
     costs it its grant. Weighed: narrowing the specification to "refused while any eligible waiter exists" (a reset of
     a run behind a dual-gated waiter blocked by its permit could then be refused indefinitely, the case the reviewer
     names) against the reservation rule (the accepted rule of the grant routine, already in the code).
  4. *minor 1 (frees between two decisions of one pass).* Within one pass, once a waiter of a key is refused for lack
     of room while it is eligible, the pass grants no younger waiter of that key whose limit and window are not looser
     than the refused one's (it skips to waiters only a looser limit admits, which skip-ahead, D1196, allows), and it
     leaves the key due. A free committed between the two decisions is then used by the next pass, in order. Weighed:
     naming the exception in decision D022-073 item 2 as the reviewer offered (a younger waiter granted first breaks
     FR-022-042's order, which this feature sells) against carrying the refused position forward (a comparison in
     memory, no statement).
  5. *minor 2 (the place and the run view).* `RATE_PLACE_SQL` counts ahead only the waiters the grant routine would
     count (eligible under the lower of their bound and the current ceiling of their own window); the run listing and
     `get`'s fallback report the effective limit, never the filed one.
  6. *minors 3, 4, 5 and 6 (weak tests).* A mixed-window case that asks for eligibility at admission; the permit
     holder's recorded limit asserted equal to the decided one; a hint check that fails when `key_due` does nothing
     (the releases no longer set the hint first); the lowered-ceiling fault case driven by the database clock (the
     window aged by moving the counted instants, decision D022-072), not by five starts within one real second.
  7. *minor 7 (a raised ceiling starves a filed waiter).* Decision D022-071 item 3 and decision D022-075 item 7 are
     reversed: a waiter's bound is the lower of its declared limit and the namespace's current ceiling, at every
     decision, so a raised ceiling applies to waiters already queued as a lowered one does (decisions D022-071 and
     D022-074). The waiter rows of both gates gain a nullable `declared_limit` (an additive migration; a row written
     before it uses its filed limit). Weighed: keeping the filed limit (a waiter filed under a low ceiling waits behind
     newer arrivals at the raised limit until its timeout, a starvation the specification's queue does not allow)
     against reading the declaration from the run's definition at each grant (a document read per waiter tried). The
     specification's "its own pinned `limit`" (US07 edge cases) is read as the declared limit, pinned by the version.
  8. *nit 1 (statement cost).* Kept: the count is one indexed range per window length present on the key, read under
     locks the grant already holds; its cost is measured by task T022-089's benchmark, not estimated.
  9. *nit 2 (ADR-0089's text).* ADR-0089 gains a dated amendment under decision 4: strict order at admission (decision
     D022-073), the reservation rule in the grant routine and for causes that may not queue (item 3). A narrowing,
     accepted under OD-2026-11-24-4.
- **D022-077 — The documents follow the code where the code carries a recorded decision, and the code follows the
  documents elsewhere (2026-11-24; the documentation of tasks T022-061 and T022-088).** Delegate (OD-2026-11-24-4).
  Writing the runbook found these disagreements; each is settled here:
  1. *Keys a grant pass visits.* The plan says 256; the code visits the scheduler's batch (200 by default). Decided: a
     pass visits the scheduler's batch of keys, never more than 256 (the hard ceiling of the plan's table). One knob
     bounds a tick's work for every pass, and the hard ceiling bounds the transactions one tick can start whatever an
     operator sets. The code gains the cap and a test; the plan's row reads "the scheduler's batch (200), at most 256".
  2. *The grant's lock order.* Plan row T26 names the fence first; decision D022-045 records run, fence, keys, as every
     pass (ADR-0050 decision 7). The plan's row is corrected to the decided order.
  3. *A step waiting for a permit.* `contracts/errors.md` says it stays `READY`; ADR-0089 decision 1 and FR-022-019 say
     it waits outside `READY` (`WAITING`). The contract is corrected.
  4. *The orphaned-waiter alert.* The plan lists `flow_control_waiter_orphaned`; the code logs and counts
     `outcome="orphan"` only (gap G022-005). Built by task T022-082 as alert `FLOW_CONTROL_WAITER_ORPHANED`.
  5. *FR-022-020's last sentence* (no new key version while old rows exist) is superseded by ADR-0093's per-key
     rotation, which the owner accepted (OD-2026-11-19-2); the specification gains a dated amendment line under it.
  6. *Task T022-082* builds `CONCURRENCY_QUEUE_STUCK` as well as the two rate alerts (the runbook documents three); an
     amendment line under the task says so.
  7. *The alerts' thresholds,* which the plan leaves open, are the runbook's: `CONCURRENCY_QUEUE_STUCK` when a key has
     a waiter older than `SAGAT_ALERT_UNCLAIMED_SECS` (300) and no grant in that time; `RATE_LIMIT_GRANT_LAGGING` when a
     key is due for longer than `SAGAT_ALERT_LEASE_GRACE_SECS` (60) and has had no grant in that time (a busy key the
     pass reads in parts is not lagging); `RATE_LIMIT_QUEUE_SATURATED` at 80 % of the effective
     `flow_control_key_waiters` of run starts. Reusing the existing alert settings keeps one operator vocabulary;
     new settings would be configuration nobody has a reason to tune apart.
  8. *Stale comments* (`apps/control-plane/src/flow_control.rs`, the build lists both capabilities;
     `crates/telemetry/src/flow_control.rs`, decisions are counted since decision D022-045) are corrected.
- **D022-078 — The declaration a waiter is bounded by is a column of its own, `pinned_limit`, the order bound of a
  pass starts at any refusal for lack of room, and the place counts the waiters that will be ahead (2026-11-24;
  decision D022-076 items 4, 5 and 7).** Delegate
  (OD-2026-11-24-4). Building decision D022-076 found three places where its text cannot be built as written:
  1. *The column of item 7.* The waiter rows of both gates already have a `declared_limit`: the filed limit (the
     declaration clamped by the namespace's ceiling at admission), which the `*.queued` event of the row records as its
     `limit`. A second column of that name cannot be added. Options: (a) rename the column to `filed_limit` and add
     `declared_limit` (not additive: a binary of the rollout reads the old name, constitution principle IV); (b) write
     the declaration into the existing `declared_limit` from now on (no migration, but one column with two meanings by
     the date of the row, and a row that no longer records what its `*.queued` event says, while the waiter rows are
     the step's journal in the consistency checks); (c) a new nullable column `pinned_limit`, the declaration pinned by
     the run's version before any ceiling (the specification's "its own pinned `limit`", US07 edge cases), `NULL` in a
     row written before it, which is then bounded by its filed limit as item 7 says. Chosen (c): additive, each column
     keeps one meaning, the row still records its event. Every decision bounds a waiter, deciding or ahead, by
     `least(coalesce(pinned_limit, declared_limit), the current ceiling)`; a CHECK holds `pinned_limit BETWEEN declared_limit AND
     10000` (the filed limit is never above the declaration, which the domain bounds at 10,000), added `NOT VALID` and validated by a second migration
     (`20270213000000` and `20270213000100`; first numbered `20270210000000` and `20270210000100`, which feature 041's
     step logs also took, and renumbered on 2026-11-25 after the latest migration of every branch; the queued-runs
     counters and the alert kinds then moved after them, to `20270213000200`..`20270213000400`, so the rollback order
     follows the order they were built in, review of 2026-11-25, correctness minor 8). A development database that
     applied the first numbers no longer starts against this build (its history names migrations the build does not
     have): the branch is unreleased, so such a database is recreated, never repaired by hand (batch 4 reviews,
     correctness minor 2; `quickstart.md`, Prerequisites). The
     migration's rollback refuses while a waiter row's declaration is above its filed limit: dropping the column would
     lose a bound no older binary can recover; a row whose two limits agree loses nothing. No grant changes (the table
     grants cover the column) and no storage class is added (a fixed-size column, outside the variable-size review).
  2. *"Eligible" in item 4.* The reviewer's own case (two frees committed between the refusal of `X` and the decision
     for a younger `Y` of the same limit) refuses `X` while the key's holders equal its limit, so `X` is not eligible
     in the sense of ADR-0089 decision 4 when it is refused. Read literally, item 4 would leave that case open. The
     bound therefore starts at any waiter that the pass's own key refuses for lack of room, its own limit included; a
     waiter refused by its other key, by its deadline, or skipped (`Busy`) sets none. A refused waiter holds back every
     younger waiter of the key whose limit is not higher and whose window is not shorter (the counts that refused it
     refuse those too unless a free came between); the grant routine reports the bound with its refusal and is given the
     pass's bounds with each waiter (no statement more). When a bound holds back a waiter that the counts admitted (a
     free came between), the pass leaves the key due, and the next pass grants in admission order. Weighed: the literal
     reading (a younger waiter granted first in the reviewer's case, against FR-022-042) and a count of frees since the
     selection (a statement more per waiter tried, for a case the bound already closes).
  3. *The place of item 5.* The grant routine decides eligibility at the decision's instant: at a full window every
     waiter is refused by its own limit, so counting ahead only the waiters eligible now would give every waiter of a
     full key the position 1 and the estimate of the first (found by `rate_rest::a_run_shows_its_rate_slot_and_a_waiting_run_its_place`).
     The place is an estimate of the instant this waiter has room, and at that instant every waiter ahead whose limit is
     not lower and whose window is not longer than this one's has room too, so the grant routine will count it ahead.
     Chosen: a waiter ahead counts when it is of no tighter limit and window than this one, or, tighter, when its own
     window has room for it now (the reviewer's case: a waiter refused by its own lower limit, or over a longer window,
     is not ahead). Weighed: the literal reading (a position that says nothing at a full key) and a projection of every
     waiter's window to the estimated instant (a second pass over the counted leases, for a read the plan bounds).

- **D022-079 — A `CANCEL_SOURCE` reset of a rate-limited source whose key's window is full is refused at its request,
  before the source is cancelled (2026-11-25; task T022-114; gap G022-019; decision D022-069 items 3 and 8; reviews of
  2026-11-21, architecture Major 1, security minor).** Delegate (OD-2026-11-24-4; T022-114 left the choice open).
  Options: (a) an advisory refusal at the request: `409 CONFLICT`, `details.reason` `RATE_LIMIT_SLOT_UNAVAILABLE`,
  nothing written and the source left running; (b) a warning in the reset preview (`previewReset`) that the key's
  window is full now; (c) both. Chosen **(a)**:
  1. *What it buys.* The trap of D022-069 item 3 destroys work: the source is cancelled and no run replaces it. (a)
     prevents that outcome for every caller in the case the request can see (the window full when it arrives); (b)
     informs only a caller that previews first and reads the warning (a batch job, a script or an SDK call does not),
     and leaves the destructive request accepted. (c) adds a second reading of the same fact to the preview's contract
     for a caller who is told at the request anyway. The refusal follows the precedent of the same request (design 043
     row 5: the input is checked before the source is cancelled, so a reset that could never be made is refused at its
     request).
  2. *The rule.* The request reads, in a read-only transaction, the source's own run-level slot row (its grant, or its
     waiter while it is buffered), which names the key, and counts the key's grants that count over the start's window
     now (run-scope grants only: a run's grant is leased at its grant, and a step's window is a row of its own; corrected
     2026-11-25, review of the batch, security nit 2); the window is full when they reach the lower of the version's declared
     `limit` and the namespace's current ceiling of that window length. Waiters are not counted: the request refuses
     only what the window itself makes impossible, and the completion, which counts eligible waiters by the reservation
     rule (D022-076 item 3), stays the authority. A source without such a row (its grant purged, a window and a minute
     after it ended counting) is not refused here.
  3. *Advisory, never a promise.* The window may empty or fill between the request and the source's end, so an
     accepted entry can still end `FAILED` `RATE_LIMIT_SLOT_UNAVAILABLE` (the outcome D022-069 item 8 accepts), and a
     refused request may have succeeded later; the runbook says to retry the request once a slot has stopped counting
     or to use `REQUIRE_TERMINAL`. Cost: one statement at the request of a `CANCEL_SOURCE` reset of a rate-limited
     version, none on any other path; no budget line covers a reset.
  4. *A permit is not checked:* cancelling the source gives its permit back at its end, so the window's trap has no
     equivalent for `concurrency`.
  Proved by `run_creators::a_cancel_source_reset_on_a_full_window_is_refused_at_its_request_and_cancels_nothing`
  (`limit: 1, per: hour`). Closes gap G022-019 when the task is ticked.
- **D022-080 — A child node whose child's start the gate refuses fails with the refusal's reason, through its failure
  route (2026-11-25; task T022-102; FR-022-055; decision D022-024; the follow-up of decision D022-062).** Delegate
  (OD-2026-11-24-4). The scheduler's pass that starts a reserved child passed every flow-control refusal up as an
  error: the pass failed on every tick and the parent's node neither completed nor failed. Now:
  1. *Every refusal of the gate fails the node,* with the refusal's own `details.reason` as the step's failure code:
     `RATE_LIMIT_QUEUE_FULL`, `CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT` (a full `queued_runs` or the live-keys
     ceiling), `CAPABILITY_DISABLED` (the deployment switched the capability off since the version was published),
     `FLOW_CONTROL_ACTIVATION_PENDING` (the capability is no longer activated for the fleet) and
     `BINARY_CAPABILITY_MISSING` (this process does not serve the gate, or the definition has a collection or a loop,
     G1197). The refusal is decided either in the child's creation (`PgRunStore::create`) or in its start
     (`Engine::start`); both roll the child back. The failure is applied only while the step's child is not started,
     under the step's row lock, as the input refusal of design 043 is (D043-038). Weighed: leaving the reservation in
     place for a later pass (the head of an oldest-first queue that one key could hold for every tenant, the reason the
     quota refusals of feature 010 fail too, G32) and failing only the three codes FR-022-055 names (the capability
     refusals would then stay errors on every tick). A process that does not serve the gate is a misconfiguration once
     the capability is activated (D022-057: activation waits for every live process to list it), so a visible, routable
     failure is preferred to a reservation that waits silently.
  2. *"Handled by the node's retry policy"* (FR-022-055, D022-024) cannot be built as written: a `child` node has no
     retry policy in IR v1 (`Node::retry_policy`: only an activity retries), and adding one is a change of the IR and of
     the six SDKs that no task of this feature carries. The codes are retryable as a class (each clears when the key
     drains or the deployment is put right), and the node's `onError` route is what handles them; an author who wants a
     retry routes the failure to a timer and back. The specification gains a dated amendment line under FR-022-055.
  3. *A `child` node inside a collection body* is refused at publication on this tree (the body subset of
     `crates/infra-ir/src/convert.rs`, `is_collection_body`), and the pass reads root children only, so the body's case
     has no path yet; `child_starts::a_child_node_inside_a_collection_body_is_refused_at_publication` holds the refusal
     and fails when a body child is enabled, to be replaced by the body's own case.
  Proved by `child_starts::{a_child_refused_by_a_full_buffer_or_a_ceiling_fails_its_node_through_the_route,
  a_child_refused_by_its_capability_fails_its_node_through_the_route}`.
  *Amended 2026-11-25 (review of the batch, correctness minor 3):* `BINARY_CAPABILITY_MISSING` is a fact of the replica
  that decides (it holds no digest key ring), not of the child: the reservation is left for a replica that serves the
  gate, as the grant pass leaves a key it cannot grant, and the pass counts a failure; `CAPABILITY_DISABLED` and
  `FLOW_CONTROL_ACTIVATION_PENDING` (the operator's switch and the fleet's activation, which the runbook says to drain
  before changing) still fail the node through its route. Proved by
  `child_starts::a_child_a_replica_cannot_serve_is_left_for_one_that_can` (red first: the node failed).
- **D022-081 — A key that reads the run's identity is refused, alone or beside other data (2026-11-25; task T022-103;
  FR-022-056; decision D022-025).** Delegate (OD-2026-11-24-4). FR-022-056 names "a key that reads only the run's
  identity (run id, business id, idempotency key)". On this tree an expression sees the run's identity through one
  variable, `run`, whose only field is the run's id (`crates/infra-ir/src/evaluator.rs`); the business identifier and
  the idempotency key are not visible to an expression, and a top-level key reads only `input`. Options: (a) refuse a key
  whose every variable is `run` (the literal reading); (b) refuse any key that reads `run`. Chosen (b): `input.accountId +
  run.id` makes one key per run exactly as `run.id` does, so the literal reading leaves the unbounded cardinality the
  rule exists for (the security review of 2026-11-13, Major 3) one concatenation away. The codec refuses it with the rule
  `FlowControlRule::UnboundedKey`, reported as `FLOW_CONTROL_KEY_UNBOUNDED` at the node; when `run` gains a field that is
  not the identity, the rule narrows to the identity's fields. *Amended 2026-11-25 (review of the batch, correctness minor
  1, security nit 1):* the rule is publication's only (`flow_control::check_publication_keys`, called by `parse` and not
  by `parse_stored`): a stored version is restored as it was published, so a version an earlier build published keeps
  its runs going. The specification gains a dated amendment line
  under FR-022-056.
- **D022-082 — The window decision has one pure model in the domain, and a differential test in the flow-control suite
  holds the SQL to it (2026-11-25; task T022-071; FR-022-040, FR-022-048; decisions D022-003, D022-005, D022-009,
  D022-014, D022-073, D022-078).** Delegate (OD-2026-11-24-4). The SQL of the gate (`readiness_gate.rs`,
  `gate_queue.rs`, `RATE_PLACE_SQL`) is the only implementation of the window's rules; nothing checked it against an
  independent statement of them except the cases each test chose.
  1. *Where the model lives.* Options: (a) the domain (`sagat_domain::execution::window`), pure functions of instants
     passed in; (b) a test-only model in `tests/support`; (c) no model, more example tests. Chosen (a): the task names the
     domain, the rules are business rules (constitution, layering: the domain owns them, the adapter executes them), and
     the estimate's arithmetic, which the application already held (`flow_control_keys::estimated_grant`), now calls the
     domain's `estimate_us`, so a waiter's estimate has one rule and the model is not a second copy that can drift. (b)
     would make the oracle test code that no production path uses; (c) never finds the case nobody thought of.
  2. *Where the differential test lives.* Domain property tests cannot reach PostgreSQL. Options: (a) the integration
     suite of the feature (`integration_flow_control_step_policies::window_model`), the real adapter and the real grant
     pass; (b) the fault suite; (c) a `proptest` run against PostgreSQL. Chosen (a): it decides through the same
     `PgRunStore` and `Scheduler` the other cases use, is selected with them by `scripts/select-test-targets.py`, and its
     sequences are not faults. Not (c): a shrinking property needs a fresh database per case, which costs minutes per
     failure on the shared machine; the sequences come from a fixed list of seeds through a small generator written in the
     test (SplitMix64), so a failure names its seed and replays exactly.
  3. *Time.* A sequence never depends on elapsed time (D022-072): before each operation every grant row of the key is
     written at an age the test chose, at least five seconds from the edge of every window, as the owner with the guard
     off in that transaction; an admission is decided by the model at the instant its row records; a pass counts as of
     its selection, which lies between two readings of the clock, and the model must decide the same at both ends (else the
     test's own setup is at fault and it says so). The run scope only: a step's lease instant is copied by the lease pass
     from a worker's lease, which the sequences would have to drive through a worker; the lease rules are held by the
     domain's properties (a lease never puts more than `limit` instants in a window of one bound) and by the existing
     cases of `rate_gate` and the fault suite.
  4. *What the properties assert.* For mixed acquirers (decision D022-009) the guarantee is per decision: after a grant
     decided by an acquirer of `limit` over `per` at instant `t`, at most `limit` grants count for `(t - per, t]` (an
     unleased grant counts, a grant ahead of a set-back clock counts); for run starts, invariant 10 over the decisions made
     up to each grant (D022-032). A whole-history window holds for one bound only: with mixed acquirers a later grant
     admitted by a looser limit may land in an earlier grant's window, which D022-009 allows.
- **D022-083 — What the flow-control telemetry measures, and how the four alerts read their conditions (2026-11-25; task
  T022-082; FR-022-049; decision D022-077 items 4, 6 and 7; gaps G022-001, G022-005).** Delegate (OD-2026-11-24-4). The
  plan names the series and the runbook the conditions; these choices were left open:
  1. *The wait histograms* (`concurrency_wait_seconds`, `rate_limit_wait_seconds`, by scope) observe each waiter the grant
     routine grants, from its queueing to its grant, after the commit. A wait that ends by a timeout or a dequeue is
     counted by its outcome in the decisions counters; observing it too would mix "how long until served" with "how long
     until given up".
  2. *The grant lag* (`rate_limit_grant_lag_seconds`) is measured, for a slot of the pass's own key, from the key's
     next-eligible hint as the pass selected it (or the waiter's queueing if later) to the grant. Options: (a) the hint;
     (b) the exact instant the waiter's slot freed, recomputed in the grant's transaction (one more statement over the key's
     grants on the hot path of every buffered grant). Chosen (a): it costs nothing, it is the quantity
     `RATE_LIMIT_GRANT_LAGGING` alerts on, and since the hint is early and never late it is an upper bound of the true lag,
     never an understatement.
  3. *The gauges* (`concurrency_waiters`, `rate_limit_waiters` by scope, `rate_limit_unleased_grants`) are set by the
     monitor every look, deployment-wide, from the key rows' counters (one statement), and zeroed with the monitor's other
     gauges when another process holds its lease.
  4. *The alerts* are four `AlertKind`s of the monitor, one alert per (namespace, group), the group the subject (cut to
     the table's 120 characters), never a key's value or label. `CONCURRENCY_QUEUE_STUCK` and `RATE_LIMIT_QUEUE_SATURATED`
     compare with the namespace's current ceilings (`concurrency_limit`; the effective `flow_control_key_waiters`), which
     the quota rules resolve: the monitor reads each namespace's ceilings once a look with the statement the gate decides
     with (`quotas::step_gate_ceilings`), over at most 5,000 candidate keys a look (beyond it the look is short and resolves
     nothing it did not see). *Revised 2026-11-25 (batch 3 reviews, correctness Major 2, security Major 1):* the cut came
     before each namespace's condition, deployment-wide, so one tenant's keys hid another's saturated key and a cut
     resolved nothing for anyone. The condition is now applied in the statement, joining each namespace's effective
     ceilings resolved in SQL by the same rules (`quotas::effective_sql`, the twin of `resolve_rows`), before any bound;
     the groups are ranked per namespace, per tenant and in all by severity (the longest wait for stuck; the fullest key's
     share of its ceiling for saturated), at most `MAX_PER_NAMESPACE`, `MAX_PER_TENANT` and `MAX_CANDIDATES` as for every
     kind, and a look that stops short records it for the namespace or the tenant beyond the bound, never for everyone. `RATE_LIMIT_GRANT_LAGGING` and `FLOW_CONTROL_WAITER_ORPHANED` are one aggregate statement
     each, as the earlier kinds. Severities: lagging and orphaned critical (the gate is not serving, or an invariant is
     broken), stuck and saturated warning.
  5. *Contracts.* The kind enumeration of `Alert` in 001's OpenAPI and of the alert events in the AsyncAPI grows by four
     values (additive: an enumeration may gain values, `contract_asyncapi_events_webhooks_messaging`); the Studio client is
     regenerated. Migrations `20270213000300` and `20270213000400` widen `operational_alerts_kind_valid` (`NOT VALID`, then
     validated); the down refuses while an alert of a new kind is kept. Feature 008's data model, an approved text, still
     lists the five earlier kinds: it is revised when it is next re-approved.
- **D022-084 — `flow_control_counts` holds the namespace's queued runs; the live keys stay counted from the key rows
  (2026-11-25; task T022-105; FR-022-058; decisions D022-026, D022-051).** Delegate (OD-2026-11-24-4). D022-026 put both
  `queued_runs` and the live keys in sixteen slot rows per namespace kept by `SECURITY DEFINER` triggers. Building it:
  1. *Queued runs: built.* `flow_control_counts (tenant_id, namespace_id, slot, queued_runs)`, the slot from the run's
     identifier (the slots of `run_active_counts`), kept by triggers on the run-scope waiter rows of both tables in the
     transaction that writes them; a run counts once (a row adds one only when the run has no run-scope waiter row in the
     other table, and takes one only when none is left there: both rows of a dual-gated run are written and removed in one
     transaction of the run, FR-022-044). `sagat_app` holds `SELECT` only. The check, the quota usage and the key listing
     read the sum of the slots (`QUEUED_RUNS_SQL`), exact under the admission lock as before (D022-051). Lock order: the
     slot row is locked when the run's own waiter row is written, after the run's active-run counter row (slot 6) and the
     start's keys (slot 7 and the rate rows of 8b); the only keys locked after it are the step keys of a buffered run's
     entry steps, and no transaction that holds a step key writes a run waiter after it. *Corrected 2026-11-25 (batch 3
     reviews, correctness Major 1):* the grant of a queued start deleted its waiter row (the slot row) before its settle
     ended the run (the active-run counter row); it now locks the active-run counter row before its keys.
  2. *Live keys: not kept in counters.* A key row is created in the middle of the sorted pass over a decision's keys and
     deleted at the commit by the deferred prune. A counter kept by an immediate trigger would be locked before the
     later keys of the same pass (two transactions sharing a slot can then wait on each other); one kept at the commit
     (a deferred trigger) would be locked in no fixed order beside the deferred quota flush of feature 009. Options: (a)
     counters anyway, with an advisory lock per namespace at the commit (serialises a namespace's key churn at its commits
     and still orders against the quota flush by event order); (b) insert-only delta rows folded by the housekeeping (no
     lock, but a read grows with the churn between folds); (c) keep counting the key rows of the versions in force, which
     only a decision that would create a key does, bounded by the ceiling. Chosen (c): exact, aware of the digest key
     versions in force (ADR-0093 decision 5, which a counter by namespace alone would lose), and without a new lock. The
     cost of the count is the benchmark's to measure (tasks T022-062 and T022-089); the data model's `live_keys` column
     is not built.
  3. *The rest of the task was built before:* foreign keys `(tenant_id, namespace_id, run_id)` → `runs` on the holder,
     waiter and grant rows (migrations `20270130000000`, `20270130000100`), `SECURITY DEFINER` count triggers on the key
     rows, and `UPDATE (next_eligible_at, updated_at)` as the runtime role's only update of semaphores and windows.

- **D022-085 — What a key reads, its level, and who sees its label (2026-11-25; task T022-104; FR-022-057; gaps G1185,
  G022-015; amends D022-060).** Delegate (OD-2026-11-24-4).
  1. *What a key reads.* Options: (a) only a plain reference (`input.a.b`, as the Iceberg write's fields are read,
     feature 035), anything else `pii`; (b) every place of the expression's syntax tree; (c) the places an evaluation
     touches (needs data). Chosen **(b)**: `Compiled::reads` (`crates/infra-expr/src/reads.rs`) lists each variable and
     literal path the expression names, each standing for the value there and everything under it (an expression may
     size, compare or convert a structure); a computed index ends the path where it stands and is read in turn; both arms
     of a condition count. It is conservative (more than an evaluation touches, never less) and keys are usually
     concatenations, which (a) would make `pii` for no reason.
  2. *The level of a place.* The carriers of feature 016 (FR-016-002) through the walk publication already runs, read
     once per document (`crates/infra-ir/src/classified_locations.rs`, shared with the Iceberg write's reading):
     `input.<path>` is the input carrier, `steps.<id>.output.<path>` (and `steps.<id>`) a root node's output carrier,
     `steps` whole every node's output; the level is the strictest at or under the place, at least `pii` when the schema
     does not say the value is a scalar, and `pii` for a place no schema declares, for `state` (no carrier: 016 refuses
     an annotation there) and for anything the reading cannot follow. A key's level is the strictest of its places;
     a key that reads nothing (the group's single key, a constant) is `public`.
  3. *Publication.* `secret` is refused `FLOW_CONTROL_KEY_SECRET` at the declaration (node, or the workflow) by
     publication only; `pii` is published with the warning `FLOW_CONTROL_KEY_PII` at each declaration. A stored version
     is restored with the level its key reads (an earlier build may have published a `secret` key), so its label is
     shown to nobody.
  4. *Where the level lives.* Options: (a) a column on the holder, waiter and grant rows, written by every decision;
     (b) a column on the key rows, raised by each decision; (c) derived from the run's pinned version, read by the codec
     (`ConcurrencyPolicy::key_level`, `RateLimitPolicy::key_level`, set by `WorkflowDefinition::with_flow_control_key_levels`
     for publication and restore alike, never canonical). Chosen **(c)**: no migration and no write on the gate's path
     (the hot-path budget is untouched), exact per run (the key of that run's definition, which (b) blurs across the
     definitions sharing a group), and one source with publication's check. A run read parses the version once (the
     definition cache) and checks the reveal grant once per namespace, only for runs with a permit or a slot; a version
     that cannot be read hides its labels (`secret`).
  5. *Who sees `Run.concurrency.keyLabel` and `Run.rateLimit.keyLabel`.* A key over `public` places: every reader; over
     `internal` places: a role that reads internal payloads (`payload:read-internal`: developer, operator, admin; not a
     viewer); over `pii` places: a caller who may reveal personal data in the namespace (D022-060's one rule,
     `reveal_grants::may_reveal_payload_unlocked`); `secret`: nobody. The reveal right needs no explicit reveal request
     here, as D022-060 decided: a label is a pseudonym, not an audited disclosure.
  6. *The key listings.* A key row does not say which definitions filed it (two definitions may share a group and a key
     value from places of different levels), so the per-key listings stay the view of a caller who may reveal personal
     data, which covers every level a published key may read; every other caller is given the group rows of T022-111
     (D022-086). A `secret` key of a version published before this rule could reach a revealer's listing by its label;
     none exists in production, since the runbook forbids activating the capability before T022-104 (G022-015).
     *Stated 2026-11-25 (batch 4 reviews, security nits):* a page of `listRuns` may parse up to its 200 runs' pinned
     versions to grade their labels (each version once a page, from the definition cache), a cost bounded by the page;
     and a continuation never attaches key levels: a continuation successor cannot declare a run-level gate
     (FR-022-059), and its node-level gates are graded when its own version is read.
  Evidence: `contract_flow_control_step_policies::ir::{a_key_has_the_strictest_level_of_the_locations_it_reads_over_the_016_carriers,
  a_key_over_personal_data_is_published_with_a_warning_and_one_over_internal_data_is_not,
  every_invalid_fixture_is_refused_with_its_code_before_a_version_exists}` and
  `integration_flow_control_step_policies::rest::a_key_label_is_shown_to_whoever_may_disclose_every_location_the_key_reads`.
  8. *Revised 2026-11-25 (batch 4 reviews, security Major 2 and the security suspicion).* A value carried through a node
     without an output carrier of its own (a `forEach` returning a body activity's `secret` output, a `while` or
     `repeatUntil` carry, a `database` node's rows, `state`) was graded `pii` and published with a warning: the level
     of "no schema says", not of what can reach it. Options: (a) follow each value's flow through expressions (a data
     flow analysis this build does not have); (b) refuse every key over such a location (refuses keys over `state`,
     which are legitimate); (c) the strictest level of every carrier of the document, never less than `pii`. Chosen
     (c): what reaches a carrierless location came from the run's input or from some node's output, so it is at most
     the strictest of them; it is exact for a document without a `secret` field and safe otherwise. `steps` read whole
     folds every carrier, the bodies' included. A carrier whose walk did not complete (`CLASSIFICATION_TOO_COMPLEX`,
     `CLASSIFICATION_UNRESOLVED_REF`, `CLASSIFICATION_UNSUPPORTED_KEYWORD`) cannot be proved not to hold a secret, so a
     key over it, and the floor of every carrierless location, is `secret`: publication refuses such a schema already,
     and a version restored with it hides the key's label from everyone. Evidence:
     `ir::{a_key_over_a_node_without_its_own_carrier_takes_the_strictest_level_of_the_document,
     a_key_over_a_carrier_whose_walk_did_not_complete_counts_as_secret}` and the revised cases of
     `a_key_has_the_strictest_level_of_the_locations_it_reads_over_the_016_carriers` (`state`, a missing node, `steps`).

- **D022-086 — The key listings' group view, read by a walk of the groups, not from counter rows (2026-11-25; task
  T022-111; FR-022-022, FR-022-057; decisions D022-061, D022-068 item 6; gap G022-014; departs from T022-111's "per-group
  counter rows").** Delegate (OD-2026-11-24-4).
  1. *Who reads which view.* Both listings answer `view: KEYS` (one row per key by label, whole instants) to a caller
     who may reveal personal data in the namespace, and `view: GROUPS` (one row per pool, scope and group: its keys,
     holders or counted and unleased grants, waiters, oldest wait, oldest holder or earliest next slot, whole instants)
     to every other caller, with `items` empty. D022-061 (b) allows whole instants for a group: a group is the
     definition's name or a declared slug, already on each run. The minute cut of D022-060 is gone with the per-key
     rows it protected (`to_the_minute` removed). A key's label stays the reveal right's in the listings (D022-085 item 6).
  2. *Where a group's sums come from.* Options: (a) per-group counter rows kept by the triggers that keep the key rows'
     counters, as the task says; (b) sharded counter rows (as `flow_control_counts`); (c) insert-only delta rows folded
     by the housekeeping; (d) a walk of the page's groups over the key rows, in one snapshot. (a) and (b) lock a group's
     counter row from the trigger of a holder or waiter row, between the key locks of one decision: a transaction that
     takes two keys of one group (two root steps of one run in one explicit group, a run's terminal release) holds the
     group's row while it waits for its second key, which another transaction holds while it waits for the group's row:
     a deadlock no lock order removes, since the trigger fires between the key locks (the reason D022-084 item 2 kept
     the live keys out of counters); one unsharded row would also serialise every decision of a group, the throughput
     the feature exists for. (c) adds a fold job, a write per holder and waiter change, and a first insert of a group's
     base row that waits on the inserting transaction (the same cycle, rarer). Chosen **(d)**: no write and no lock on
     the gate's path (the hot-path budget is untouched), no migration, exact in one `REPEATABLE READ` snapshot, and a
     page costs its own groups only: the groups are found by a recursive walk of the key table's primary key that skips
     from one group to the next (one probe a group, `groups_after_sql!`; for a named group, one probe of each of the four
     pools and scopes, `groups_named_sql!`), and each group is summed over its own key range and its holders' range of
     `concurrency_holders_key_age_idx` (a rate group over its windows, each read as the per-key listing reads it). A
     page's cost grows with its own groups' keys, never with the namespace's, and paging the whole listing reads each
     key once (the per-key listing's bound, D022-058); the 2 s statement timeout still bounds a page.
  3. *Order and cursor.* Groups by pool, scope and name (the primary key's order), not oldest wait first: an order by a
     group's oldest wait would need that minimum kept per group, which is the counter of item 2. The cursor is the
     group's place sealed as a key's (`KeyPosition::of_group`, its digest all zero), under listing tags of their own
     (`kg1`, `rg1`), so a cursor of one view is a `400` in the other (a caller whose reveal right changed between two
     pages starts again), and of one gate's view in the other gate's.
  4. *One opening for both handlers* (D022-068 item 6): `routes::concurrency::open_key_listing` reads the namespace and
     the group, opens the cursor of either view (`KeyPosition::open_either`), admits the caller and refuses a cursor of a
     version out of force (`key_cursor_in_force`), returning a `KeyListingQuery`; each handler then asks its service.
  Evidence: `integration_flow_control_step_policies::rest::{a_caller_without_the_reveal_right_reads_one_row_per_group_and_pages_them,
  a_page_of_groups_reads_only_its_own_groups, a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data}` and
  `rate_rest::the_namespace_lists_its_rate_keys_with_the_concurrency_listings_protections`.
  5. *Revised 2026-11-25 (batch 4 reviews, correctness Major 1, security Major 1 and minor 1, security minor 2).* (a) A
     page summed each of its groups whole, and the lookahead group too: one group of 100,000 keys made every page a
     `503` past the statement timeout, and no cursor got past it. A page now reads at most `GROUP_PAGE_KEY_ROWS` (200,
     the per-key page bound) key rows in one `REPEATABLE READ` snapshot: each group is read in chunks of its primary-key
     range after the last digest read, its sums accumulated; a group whose range ends within the budget is emitted, and
     one that does not is carried in the cursor (`KeyPosition::partial`: the group's place, the last digest read, the
     partial sums and oldest instants), sealed as before, so the next page resumes it. The lookahead is one probe of the
     primary key for the next group, never its sums. A group's row is therefore summed across the pages that read it,
     each page one snapshot (a rate group's counts each at its page's instant), and a page can hold no group and still
     have a cursor; the order and the tags are unchanged, and a cursor of the earlier build (a place without partial
     sums) still opens. Options weighed: a summary table per group (rejected in item 2: a lock on the gate's path); a
     larger statement timeout (no bound at all); sampling (inexact). (b) The group row's count of keys told a caller
     without the reveal right which runs share a key (a group of two runs and one key against two keys); it is removed
     from both group bodies, the OpenAPI and the Studio. (c) A rate group's `nextSlotAt` linked a waiting run to a granted
     one (the grant instant plus the window); it is omitted from the group view. Evidence:
     `rest::a_group_larger_than_a_page_is_summed_across_pages_and_the_next_group_is_reached`,
     `rest::a_page_of_groups_reads_at_most_its_budget_of_key_rows`,
     `rest::a_group_view_tells_nothing_that_changes_with_the_number_of_keys`.

- **D022-087 — A Signal-with-Start whose start queues holds its signal with the run, delivered by the grant (2026-11-25;
  task T022-109; FR-022-018, FR-018-024; decision D022-050 item 6; gap G022-010).** Delegate (OD-2026-11-24-4).
  1. *Where the signal waits.* Options: (a) a row of its own, `flow_control_held_signals`, one per run, its payload a
     `payloads` row named by digest (the content-addressed store every signal payload already lands in, charged to
     `tenant_content_bytes`); (b) the start's receipt carrying the payload (a receipt is a replay answer, not a work
     item, and it outlives the run); (c) a pending `step` row (a step that does not exist in the definition's graph
     breaks replay equals projection). Chosen (a): the row is the run's own (foreign key to `runs`, `ON DELETE
     CASCADE`), so it lives and dies with the run, and the grant finds it by the run's primary key, one probe.
  2. *What the request answers.* The run is created and queued, the signal is held, and the answer is `202` with
     `started: true`, `deliveredTo: []` and `held: true`. The receipts are written in the creation transaction: the
     signal's under its usual scope (`run.signal:<run>`, nothing delivered) and a marker (`run.signal:<run>.held`), so a
     retry of the request answers the same, `held: true`, whatever the run is now. A request whose definition declares
     no `waitEvent` for the signal's name keeps the refusal of FR-018-024 (`SIGNAL_NOT_ACCEPTED_AT_START`, nothing
     created): no grant could ever deliver it, and the answer does not depend on the key's load.
  3. *The delivery.* The grant routine, in the transaction that grants the run its start (T26, D022-073), after the
     grant's settle, takes the held row and delivers the signal exactly as `signalRun` would: to every step that waits
     for it, the payload the step's output, audited `run.signalled` by the original caller (the subject and the trace
     kept with the row), journaled `wait.satisfied` with `via: SIGNAL_WITH_START`. When no step waits for it at the
     grant (the waiting node is not on the entry's path), it is dropped with an audit record `run.signal_dropped`
     (actor the engine, reason `NOT_WAITING_AT_GRANT`), as `signalRun` would refuse it; when the run's retained output
     would pass its bound, dropped the same way (`OUTPUT_BOUND`).
  4. *A queued run that ends instead.* Its queue timeout, a cancellation, or any other end of a queued run drops the
     held signal in the commit that ends the run, audited `run.signal_dropped` (reason `RUN_ENDED`); the payload stays
     the tenant's content under its retention, as the payload of a delivered signal does. Only a commit that ends a run
     whose start declares a gate looks for the row, inside the flow-control step of `persist` (no statement on the hot
     path of a plain definition).
  5. *An open holder whose start is queued* is signalled exactly as `signalRun` (FR-018-024): nothing waits, `409`
     `NOT_WAITING`, nothing changed. Only the request that creates the run holds its signal; holding a second signal
     for an existing run would be a buffer, which FR-018-024 refuses.
  Migration `20270214000000_flow_control_held_signals` (`sagat_app`: `SELECT`, `INSERT`, `DELETE`; a down that refuses
  while a row is kept; the signal's name, customer content, charged to `tenant_content_bytes` by the ALWAYS accounting
  triggers, added 2026-11-25 after the merge with `develop` when
  `durable_quotas::manifest_covers_every_variable_size_column_and_every_accounted_table_has_always_triggers` failed on
  the table: the branch is unreleased, so the migration itself carries them, as D022-089 does for the quota lists). Evidence: `run_creators::{a_signal_with_start_whose_start_queues_holds_the_signal_and_the_grant_delivers_it,
  a_held_signal_is_dropped_with_an_audit_when_its_queued_run_times_out_or_is_cancelled,
  a_held_signal_no_step_waits_for_at_the_grant_is_dropped_with_an_audit,
  a_signal_with_start_of_a_name_no_node_waits_for_is_refused_when_the_start_would_queue}` and
  `rate_creators::a_signal_with_start_of_a_rate_limited_definition_takes_its_slot_or_holds_its_signal`.

- **D022-088 — A waiter the grant routine cannot decide now holds back the younger waiters of its key (2026-11-25; batch 3
  work item 2; decision D022-073; ADR-0089 decision 3).** Delegate (OD-2026-11-24-4). The fault test
  `rate_limits::a_flood_of_a_thousand_starts_on_one_key_never_holds_more_than_its_limit_in_a_window` failed twice on a
  loaded runner with "grants follow admission order". Cause, reproduced with a held lock
  (`run_admission::a_waiter_whose_run_is_busy_holds_back_the_younger_waiters_of_its_key`): the grant routine takes a
  waiter's run with `SKIP LOCKED`; a run another transaction holds (a pass that has not yet released it: a transaction
  that ends without a commit is rolled back by the pool when it takes the connection back, in a task of its own, so for
  a moment after a pass the run it refused is still locked; a statement in flight) was counted `skipped` and the pass
  went on, and on a window with room for both it granted the younger waiter behind it first. A failed grant did the
  same. Options: (a) wait for the run (`Lock::Wait`, a pass holding a key could then wait on a run, ADR-0089 decision 3
  forbids it); (b) hold back only the younger waiters of no looser limit and window (the waiter's limit is not known
  without its run); (c) end the key's pass at that waiter, the key keeping its hint, so the next pass starts from it.
  Chosen (c): exact admission order, one pass of delay at most, no new lock. The integration test
  `run_admission::a_cancellation_dequeues_a_queued_start_and_frees_a_held_permit` also failed twice in full owner runs
  ("the tick did not grant exactly one"); it was not reproduced (25 repetitions unloaded), and its tick has no other
  transaction on the waiter's run: the remaining causes are a grant that failed on the loaded runner (a statement or
  pool acquisition timeout) or the pool's late rollback above. Its assertion now prints the tick's report, which names
  which (`skipped`, `failed`, `fenced`).

- **D022-089 — The merge with `develop` names both features' quota resources in every restatement this feature owns
  (2026-11-25).** Delegate (OD-2026-11-24-4). Merging `develop` (feature 041, step logs) into this branch put feature
  041's `20270201000000_step_log_quotas` between this feature's `20270130000300_flow_control_quota_resources` and
  `20270206000000_flow_control_rate_quota_resources`. Each of the three restates the whole lists of the CHECKs
  `quota_ceilings_resource_valid`, `quota_limits_resource_valid` and `quota_limits_tenant_scoped`, and migrations run in
  version order: the last one, `20270206000000`, was written without the four step-log names, so on the merged tree a
  fresh database refused every step-log limit and ceiling. Seen failing on the merged tree:
  `migrations::every_quota_resource_is_accepted_by_the_checks_in_force_on_a_fresh_database` (inserts every value of
  `QuotaResource::ALL` at its default) and `develop`'s
  `migrations::the_last_restatement_of_each_quota_resource_list_names_every_resource` (reads the text), both in
  integration_platform_foundation. Options: (a) name the step-log resources in this feature's two restatements
  (`20270130000300`, `20270206000000`); (b) leave them and add a migration stamped after every migration of both that
  restates the union. (b) was built first and failed the upgrade of a database of `develop`: such a database has
  `20270201000000` and may store step-log limits before it takes this feature's migrations (sqlx applies a missing
  earlier version after later ones), and the validations `20270130000700` and `20270206000100` then fail on those rows
  (seen in `step_log_isolation::schema::the_migration_rolls_back_only_without_lines_and_restores_what_it_replaced`,
  integration_claim_transition_efficiency, which rolls back to `20270201000100` with a step-log limit stored and
  migrates again). Chosen (a): `20270206000000`, the last restatement, names all 51 resources, and a database of
  `develop` takes this feature's migrations with its step-log limits stored
  (`migrations::a_database_of_develop_with_step_log_limits_takes_the_flow_control_migrations`). It changes the checksum
  of two migrations of this unreleased branch: a database that applied them is recreated, as for the renumbering of
  D022-078. The reverse, a database of this branch that stores a flow-control limit and then takes `20270201000000`, is
  not supported for the same reason (that migration, already on `develop`, is not changed). The downs had the same
  hazard: the down of `20270201000000` restored the pre-041 lists, without this feature's six concurrency names, so
  `migrations::the_quota_and_declaration_migrations_roll_back_only_once_their_rows_are_drained`
  (integration_flow_control_step_policies), which rolls back to `20270130000300` with a `queued_runs` limit stored,
  failed on the restored CHECK before the clean refusal it expects. Each down now restores the lists in force before
  its migration on the merged tree: `20270201000000`'s down names the concurrency names of `20270130000300`, and
  `20270206000000`'s down the step-log names (a down is not checksummed). Value ranges are not restated: only
  `20270201000000` restated them, and this feature's resources take the general range (their bounds are in code). The
  same merge brought `develop`'s `migrations::a_constraint_added_not_valid_is_validated_by_a_later_migration`, which
  failed on `steps_waiting_kind_valid` (restated `NOT VALID` by `20270130000600` and left unvalidated, as every earlier
  restatement had left it): `20270215000000_flow_control_waiting_kind_validate` validates it under `SHARE UPDATE
  EXCLUSIVE` (every restatement only added kinds, so every stored row satisfies it).

- **D022-121 — Only a busy run and a failure that clears on its own end a key's grant pass (2026-11-25; batch 5
  correctness Major 1, security suspicion 2; amends D022-088) (numbered D022-090 until 2026-11-26, when the merge with the SDK-builders branch renumbered it).** Delegate (OD-2026-11-24-4). D022-088 ended a key's pass
  at a waiter whose grant failed, whatever the failure: a waiter whose grant always fails (a version this build cannot
  load, an execution error, a row a constraint refuses, a held signal whose delivery fails) then held back every younger
  waiter of its key until its own deadline. Options: (a) end the pass on every failure (as built; a poisoned waiter
  stalls its key); (b) skip every failure (a younger waiter overtakes an older one whenever the database hiccups,
  breaking admission order under load, the cause of D022-088); (c) end the pass only on what clears on its own, the
  store's `Unavailable` and `DatabaseUnavailable` (a pool or statement timeout, a serialization failure, a deadlock, a
  lost connection: SQLSTATE classes 08, 40, 53, 55, 57) and a busy run, and skip any other failure, counted `failed`
  in `scheduler_actions_total{kind="gate_grant"}` and logged with its run. Chosen (c). The skipped waiter stays queued
  until its deadline, which bounds it; on a concurrency key `CONCURRENCY_QUEUE_STUCK` names the key's group once younger
  waiters keep taking the permits, so no new alert kind is added. A held signal whose delivery fails (anything but a
  transient failure) no longer fails the grant: the grant's transaction is rolled back and the grant is tried once more
  in a new transaction that drops the signal with its audit record, reason `DELIVERY_FAILED`, and grants the start
  without it. Every drop's audit record (`NOT_WAITING_AT_GRANT`, `OUTPUT_BOUND`, `RUN_ENDED`, `DELIVERY_FAILED`) names
  the caller who sent the signal, the held row's actor, in its reason (batch 5 security minor 2): the record's actor
  stays the engine, which dropped it. Tests: `run_admission::a_waiter_whose_grant_always_fails_does_not_hold_back_the_younger_waiters` (a
  trigger refuses the older waiter's holder row; the younger is granted in the same pass) and
  `run_creators::a_held_signal_whose_delivery_fails_is_dropped_and_the_start_is_granted`.

- **D022-122 — Every waiter's transaction that does not commit is rolled back and the rollback awaited (2026-11-25; batch
  5 correctness Major 2) (numbered D022-091 until 2026-11-26, when the merge with the SDK-builders branch renumbered it).** Delegate (OD-2026-11-24-4). `grant_waiter` returned a refusal by dropping its transaction:
  the pool rolls a dropped transaction back later, in a task of its own, so for a moment the run stayed locked and a
  second waiter of the same run (two root nodes of one run on one key, or a start gated on two keys) found it busy,
  which since D022-088 ends the key's pass. `RunTx` gains `rollback`, and every return of `grant_waiter` that does not
  commit awaits it before the next waiter is tried (an adapter without a transaction drops it, as before). A failed
  rollback is logged; the connection is then closed by the pool, which releases the lock. Test:
  `run_admission::a_refused_waiter_frees_its_run_before_the_next_waiter_of_the_same_run_is_tried` (two waiters of one
  run adjacent on one key, the first refused by its other gate; a younger waiter of another run is granted in the first
  pass). The intermittent `run_admission::a_cancellation_dequeues_a_queued_start_and_frees_a_held_permit` was
  re-examined against this cause: its key has one waiter, so no earlier waiter of the same pass can leave the run
  locked; a transaction an earlier pass of the same tick drops on that run could, and is now named by the report
  (`skipped` counts a busy run). It is not closed by this change: when it fails again, the report says which cause.

- **D022-123 — The batch-5 minors and nits built on 2026-11-25 (batch 5 rulings) (numbered D022-092 until 2026-11-26, when the merge with the SDK-builders branch renumbered it).** Delegate (OD-2026-11-24-4).
  1. *A held row exists only while its run's start waits* (correctness minor 3, security suspicion 1). `verify_run`
     refuses a held signal kept for a run whose start does not wait
     (`run_creators::verify_run_refuses_a_held_signal_kept_for_a_run_whose_start_does_not_wait`). The drop of
     D022-087 item 4 no longer looks only at a commit with a dequeue or timeout event of the start: a commit of a run
     that has ended and whose start never ran (each gate of its start queued, timed out or dequeued, none ever held)
     drops it, whatever ended the run. A run whose start ran has a gate held or given back, so its end adds no statement
     (the budget of design 038 is unchanged).
  2. *Two doc comments said a signal is never buffered* (security minor 3): `routes/execution.rs` and `runs.rs` now name
     the one buffer, a new run whose start queues.
  3. *`effective_sql` guarded `iteration_concurrency` with a debug assertion only* (correctness nit): it returns a
     `Result` and refuses it in every build (`quotas::effective_sql_tests`).
  4. *A grant from the buffer is not re-authorized, for signals too* (security minor 1): FR-022-060 is amended in words.
  Correctness minor 4 is the dated amendment of ADR-0089 decision 4 (D022-121); security minor 2 is in D022-121.
  Batch 4, built the same day: the release-upgrade tests read the migration head from the migrator (correctness minor
  1, `support::migration_head`); the pinned-limit rollback test rolls the later migrations back first, so that its
  refused rollback is measured from its own schema (minor 4); the label test asserts the group view and its empty key
  list where its two checks of key rows would pass on an empty list (minor 6); a cursor of the other view of a key
  listing (what the caller may see changed between two pages) answers `400` with its own message, "the cursor was
  written for the other view of this listing", not "the cursor is not valid" (nit 1,
  `rest::a_cursor_of_the_other_view_is_refused_with_its_own_message`).
  Batch 3, built the same day: the counts migration `20270213000200` locks `rate_limit_waiters` before
  `concurrency_waiters`, the order a dual-gated admission writes them (correctness minor 6), and its counting trigger is
  two per table, `AFTER INSERT ... WHEN (NEW.node = '')` and `AFTER DELETE ... WHEN (OLD.node = '')`, so a step's waiter
  row fires none (correctness nit); both are checked in
  `migrations::the_queued_runs_counters_are_filled_from_the_waiters_and_roll_back_without_loss`. The migration is on no
  released tree; a database that applied it is recreated (D022-089).

- **D022-099 — A test that asserts what one grant pass decides about a waiter of two keys runs one key at a time
  (2026-11-25; CI runs 45637128106 at `f30d4ffd` and 45649612535 at `484c6047`, integration shard 2; decisions D022-088,
  D022-073 item 4).** Delegate (OD-2026-11-24-4). (The numbers D022-090 to D022-098 are taken by the SDK-builders branch,
  `feature/022-sdk-builders`, which wrote them first, on 2026-11-20; the three decisions of this branch that took D022-090
  to D022-092 on 2026-11-25 are renumbered D022-121 to D022-123.)
  `rate_gate::a_start_behind_an_eligible_waiter_queues_although_the_window_has_room_and_the_next_pass_grants_it` failed
  on both runs, and passed locally, with "the window rests until the holder's slot ages out": after the first pass, the
  window's hint of the dual-gated waiter `w2` was still the instant of its queueing. Cause: `w2` waits on the window and
  on a permit, so both keys are due, and the grant routine runs up to `SchedulerSettings::concurrency` keys' passes at
  once (eight by default); the permit's pass held `w2`'s run while the window's pass tried it, the window's pass found it
  busy (`SKIP LOCKED`) and, since D022-088, a busy waiter ends its key's pass and the key keeps its hint, so the window did
  not rest. It is the rule working, not a refusal mistaken for an undecided waiter: a busy waiter may be grantable the
  moment its run is free, and the next pass decides it (one pass more, as D022-088 accepted: the key that won the run
  decides the waiter in that pass, so the loser is alone in the next). The test depended on which key's pass took the run
  first. Reproduced red by holding `w2`'s run in a transaction of the test during the pass (the same assertion failed, the
  hint unchanged). Options: (a) the test runs that pass one key at a time, as
  `rate_gate::a_grant_through_another_keys_pass_wakes_the_waiters_of_its_window` already did for the same
  cause, and a test of its own pins the busy case down; (b) the routine awaits a waiter's run that another key's pass of
  the same process holds (an in-process registry of the runs in flight), which saves that one pass but adds coordination
  to every pass for a cost D022-088 already bounds; (c) wait for the run's lock (`Lock::Wait`), which ADR-0089 decision 3
  forbids. Chosen (a). Tests: the first pass of the failing test runs one key at a time; the new
  `rate_gate::a_dual_gated_waiter_found_busy_keeps_its_window_due_and_the_next_pass_rests_it` holds `w2`'s run during a
  pass of the default scheduler (nothing granted, `skipped` counted, the window's hint unchanged), then a pass one key at a
  time refuses `w2` for its permit and rests the window at the holder's grant plus one hour.

- **D022-100 — A key listing's cursor expires fifteen minutes after its page (2026-11-25; batch 5 security suspicion 3;
  decisions D022-059, D022-086 item 5).** Delegate (OD-2026-11-24-4). A cursor of a group view carries the sums of a
  group read in part; replayed later, it adds those old sums to rows read now, and a page boundary kept for long tells
  whether a group still crosses 200 keys. Options: (a) every cursor of both key listings, in both views, carries the
  instant its page sealed it, and is refused fifteen minutes later; (b) only a cursor that carries a group read in part
  expires (a key view's cursor replayed later only resumes a later snapshot, but two rules for one opaque value are
  harder to state and to test); (c) no partial sums, each page reading its groups whole (the 200-row bound of a page,
  D022-086 item 5, is lost). Chosen (a). The instant is the serving process's clock in whole seconds, inside the sealed
  text, so a caller can neither read nor move it; a cursor older than fifteen minutes is a `400` with a message of its
  own, "the cursor has expired: read the listing again without a cursor", decided with the other refusals of a cursor
  before admission (it tells nothing of the namespace: the cursor was sealed for it). A cursor whose instant is ahead of
  the reader's clock (replicas' clocks differ by far less than the lifetime) is read. The longest position sealed grows
  by the instant's at most 21 bytes and stays under `MAX_CURSOR_PLAIN_BYTES`. Tests:
  `flow_control_keys::tests::a_cursor_expires_fifteen_minutes_after_its_page` and
  `rest::an_expired_cursor_is_refused_with_its_own_message` (a real page's cursor sealed again sixteen minutes earlier
  with the deployment's ring is refused, and fourteen minutes earlier is read).

- **D022-101 — A group row says when its sums were read across pages (2026-11-25; batch 5 correctness minor 5; decision
  D022-086 item 5).** Delegate (OD-2026-11-24-4). A group with more keys than a page reads is summed across the pages
  that read it, each page one snapshot, so its sums mix instants: a group may show more holders than its keys' limits
  allow, or a waiter counted on one page and its holder row on the next. Options: (a) a required `spansPages` boolean
  on each group row of both listings, `true` when its sums were begun by an earlier page (the page resumed it from the
  cursor's partial sums), so a reader knows the row is not one snapshot; (b) words in the contract only (a reader
  cannot tell which row); (c) read a group whole on one page (the bound of a page is lost). Chosen (a): it tells nothing
  a viewer could not already infer from following the cursor (a page with no group and a `nextCursor`), and nothing
  about which runs share a key. Test:
  `rest::a_group_larger_than_a_page_is_summed_across_pages_and_the_next_group_is_reached` (the big group's row
  `spansPages: true`, the next group's `false`, in both listings).

- **D022-102 — The gates in the run state and a test's large scenario future are boxed where they are created
  (2026-11-25; CI runs 45637128106 and 45649612535, integration shard 1; D022-070).** Delegate (OD-2026-11-24-4).
  `scoped_event_storage::receiver::a_dispatcher_quota_refusal_rolls_back_receipts_and_retries_pending_lookback` (first
  run) and `..._with_held_inbox_keeps_lookback_pending` (second) aborted integration_workflow_primitives with a stack
  overflow of the 2 MiB test thread (debug build); both passed in the same job's isolated step and locally. Measured on
  the arm64 debug build, `RUST_MIN_STACK` bisected in 32 KiB steps, one test at a time: on `develop` (`b42eb7d5`) the
  held-inbox test needs 1,920 KiB (fails at 1,888), on this branch 2,048 KiB (fails at 2,016), and so does its sibling:
  the tests already sat at 94% of the stack on `develop`, and this feature's growth pushed them over on x86. What grew
  (`-Zprint-type-sizes`, both trees): `StepState` 128 to 288 bytes and `RunState` 336 to 520 (the 160-byte `Gates` of a
  step's attempts and of a run's start, inline), `Node` 560 to 704 (its 144-byte `FlowControl`), and the poll frames
  that copy them (`Engine::run`'s from 14,288 to 17,056 bytes); the futures on the dispatcher's path grew by at most
  about 500 bytes each. Options: (a) raise the test thread's stack (`RUST_MIN_STACK` in CI) — refused, it hides the
  growth from every other caller; (b) box the gates and the node's declaration (an ungated run, step or node allocates
  nothing) and pin the sizes; (c) also box the test's own scenario future where it is created, as D022-070 boxed a
  creator's futures. Chosen (b) and (c). `Gates` holds one pointer (`StepState` 136, `RunState` 368), a node's
  `flow_control` is `Option<Box<FlowControl>>` (`Node` 568); the gated tests need 1,984 KiB with (b) (the node's box
  moved nothing on this path, kept for the definition's own frames), and 1,376 KiB with (c), 544 KiB under `develop`.
  Tests: `crates/domain/tests/state_sizes.rs` (red at 288, 520, 160 and 704 bytes before the boxes) and
  `scoped_event_storage::receiver::the_quota_refusal_future_kept_on_the_test_thread_stays_small` (red without the box:
  78,272 bytes with the gates boxed, 80,592 before); the plan's table "Stack and future sizes of a run creator" holds the bounds.

- **D022-110 — The fragments and the canonical contracts say the same thing for the slice, and a test holds them
  together (2026-11-26; tasks T022-060 narrowed to the concurrency parts, T022-087 the rate-limit parts).** Delegate
  (OD-2026-11-24-4). The branch merged each part into the canonical documents with the code that serves it (T022-013,
  T022-026, T022-078, T022-111, D022-100, D022-101), so the task was to find what was still missing and where the two
  documents disagreed. Read on 2026-11-26 against the code: already merged were both key listings with their `group`,
  cursor and `Limit` parameters, the six key and group schemas (`spansPages` included), `Run.concurrency`
  (`RunConcurrency`), `Run.rateLimit` (`RunRateLimit`), the `waitingOn` values `rate_limit` and `concurrency`, the
  quota resources, the alert kinds, the refusal reasons but one, and both IR policies with the same shape as the
  fragment. Where they disagreed the code decided, and the fragment was wrong each time: the listings take the canonical
  `Limit` parameter (default 50, values outside 1 to 200 clamped, as `routes/concurrency.rs` reads it), not
  `PageLimit`; their `200` carries the trace and request headers like every canonical answer; `ConcurrencyKeyPage` is
  closed (`additionalProperties: false`), as `RateLimitKeyPage` already was; `Run.concurrency` and `Run.rateLimit` are
  absent, never `null`, on a run without a gate (`skip_serializing_if` in `dto_exec.rs`), and `Run.concurrency.since` is
  always sent. The fragment is corrected. Missing from the canonical documents, and merged: `RATE_LIMIT_QUEUE_FULL` in
  `Error.details` (the server gives it since T022-012); the publication codes of the two gates in
  `DefinitionProblem.code` (`CONCURRENCY_POLICY_INVALID`, `RATE_LIMIT_POLICY_INVALID`, `FLOW_CONTROL_KEY_UNBOUNDED`,
  `FLOW_CONTROL_KEY_SECRET`, `FLOW_CONTROL_COMBINATION` and the warnings `CONCURRENCY_SELF_CHILD`,
  `CONCURRENCY_LIMIT_MISMATCH`, `RATE_LIMIT_MISMATCH`, `FLOW_CONTROL_KEY_PII`); the step and child node failure codes in
  `RuntimeNode.failureCode`; and, in the IR schema's descriptions, the rules the fragment kept in `x-semantic-rules`
  (the two warnings, the declaration bounds, the strict window). No operation, parameter or response of the canonical
  OpenAPI changed, so its pinned baseline (`tests/contract/iceberg-writes/fixtures/openapi-operations.json`) does not
  move; the ten journal events are the journal vocabulary (`crates/domain/src/execution/event.rs`), not AsyncAPI
  messages, and the AsyncAPI fragment holds only `run.result_returned` (US3, not in the slice), so neither
  `events.asyncapi.yaml` nor `events.baseline.json` changes. Options for keeping the two documents together: (a) delete
  the fragment's merged parts (the fragment would no longer say what the feature adds); (b) words in the README only;
  (c) a contract test that compares the shapes, prose removed, and checks that every code the server gives is named in
  the canonical document. Chosen (c), as Rule 21 asks: `contract_flow_control_step_policies::contracts`, five tests,
  each seen red before the merge (the fragment's seven differences, `RATE_LIMIT_QUEUE_FULL`, nine publication codes,
  three IR rules, nine failure codes).

- **D022-103 — What the tenant battery holds of flow control, and where its raw-key canary looks (2026-11-26; tasks
  T022-058, T022-090; FR-022-022, FR-022-048, FR-022-057; SC-022-010, SC-022-023; NFR-022-002).** Delegate
  (OD-2026-11-24-4). The tasks name the battery and "raw-key canaries in logs, spans, metrics and listings"; they leave
  open which surfaces a canary reads and whether a key's label, not only its value, is a leak outside the listings.
  Options: (a) the raw value only, in response bodies; (b) the raw value in every body any caller reads, every log line,
  span and metric series, every audit record and every journal event of the gates, and a key's label in no log line,
  span or series and in no body a caller without the reveal right reads; (c) (b) and the grant pass of the scheduler
  driven by a real engine. Chosen (b). A label is the start of the keyed digest: the telemetry vocabulary
  (`crates/telemetry/src/flow_control.rs`) already keeps it out of spans and series, and in a log line read by an
  operator without the reveal right it would link runs as the group view forbids (D022-086). (c) adds nothing: the grant
  pass reads keys by their digests and never holds a raw value, which is evaluated at admission only, and every start of
  the test is admitted. The capture must show the gates' span names and series, or the canary would prove nothing, and
  the test collects every surface at fault before it fails, so that one hand mutation planting a key in several places
  turns each of them red at once. What the battery found: no leak. The listings' tenant boundary holds in three layers
  (the route's admission, the service's `authorize` and `require_namespace`, the store's tenant filter): removing the
  route's admission alone leaves the battery green, so the hand mutation that proves the cross-tenant clause removes
  the first two. The same raw value files one label per gate and namespace whatever its group or level (the digest
  reads the gate, the tenant, the namespace and the value: a step's slot shares the run's label), and unrelated labels
  in another namespace or tenant. Tests: `battery::flow_control_keys_stay_in_their_tenant_and_namespace_and_never_show_their_value`
  and `battery::every_implemented_operation_resists_an_administrator_of_another_tenant` (its victim now holds keys whose
  value is a canary), in `contract_rest_security_multitenancy`. The two tests hold one lock for their whole run: a
  `tracing` call site of the gates that one reaches while the other installs its capture can be cached as wanted by
  nobody (`tests/support/logs.rs`), and the first runtime-role run missed the `workflow.rate_limit.admit` span that
  way; the capture's own check of the gates' span names caught it.

### Decisions of the Studio slice (2026-11-24)

- **D022-150 — How the Studio authors, shows and lists the gates (tasks T022-054, T022-055, T022-056, T022-085;
  FR-022-022, FR-022-035, FR-022-048, FR-022-050, FR-022-057; FR-007-010). Delegate (OD-2026-11-24-4).** The tasks
  name the screens; these choices are left open by the specification and the contracts. Each option was weighed for
  the architecture and for the person using the Studio, not for the least code.
  1. *What turning a gate on writes.* Options: (a) a default policy (`limit` 1, `per` minute), so the draft validates
     at once; (b) an empty policy `{}` that the author completes, with each missing member said beside its field and
     the platform's own check (`CONCURRENCY_POLICY_INVALID`, `RATE_LIMIT_POLICY_INVALID`) in the checks panel; (c) no
     switch: the policy exists once a limit is typed and disappears when the limit is cleared. Chosen: **(b)**. A
     default is a value nobody chose: `limit` 1 silently serialises every run of a key, which an author who forgets to
     change it learns in production; the Studio's rule is that it never invents a field (`model/ir.ts`). (c) loses the
     key, group and timeout typed before the limit, and makes "no limit" and "limit being typed" one state. A draft is
     saved even when it does not validate, so (b) costs no work, and publication is refused until the policy is whole.
  2. *What the Studio checks itself.* Only what the server refuses, with the server's bounds
     (`crates/domain/src/workflow/flow_control.rs`, `crates/infra-ir/src/flow_control.rs`): `limit` a whole number from
     1 to 10,000 for a concurrency limit and to the window's ceiling for a rate limit (100 a second, 6,000 a minute,
     10,000 an hour); `per` one of `second`, `minute`, `hour`; `group` `^[a-z][a-z0-9.-]{0,63}$`; `queueTimeoutMs` a
     whole number from 1 to 86,400,000; `key` from 1 to 1,024 bytes. What a key may read (`input` at the top level;
     `input`, `steps`, `state` on a step; never `run`, decision D022-081) needs the platform's expression compiler, so it
     is said in the hint and checked by the platform's validation, not guessed by a second parser in the browser. A namespace ceiling lowers a
     limit at admission and never refuses a publication, so it is not a check either.
  3. *The order of the members.* The policy is written with its members in alphabetical order (`group`, `key`,
     `limit`, `per`, `queueTimeoutMs`), the canonical form the shared fixtures and the SDKs' canonical JSON use, so the
     Studio's policy is byte-identical to the fixtures' text and not only equal after sorting. Members the Studio does not
     know (a newer platform's) are kept after them, untouched, and said to be kept (FR-007-010); a policy that is not an
     object is shown unreadable, kept as it is, and only removable.
  4. *Where a run's gates are shown.* Options: a section of the selected step's panel, a column of the runs list, or
     the facts of the run's header. Chosen: **the header** (`features/runs/RunGates.tsx`), because `Run.concurrency` and
     `Run.rateLimit` are the run's own state and are what an operator opens a waiting run to see; the position and the
     estimate come from the single-run read only (FR-022-048), which the run page already makes, and the runs list does
     not compute them. A step waiting for a gate is said by its waiting words (`waitingOn` `rate_limit` or
     `concurrency`) wherever a step's waiting state is shown already (the step panel, the table, the card's name). The
     estimate is called an estimate and never a promise; an absent estimate is said absent, with the reasons the contract
     gives, and never replaced by a computed one.
  5. *A hidden key label.* `keyLabel` `null` is said in words ("hidden: you may not reveal personal data in this
     namespace") rather than left blank, so an operator knows the platform withheld it and does not read an empty cell
     as "no key"; in the listings the instants of such a key are said to be cut to the minute, as the contract says
     they are.
  6. *Unknown values.* A state, pool, scope or window this Studio does not know is shown as unsupported, with the value
     clipped, never mapped to a known word and never a crash (FR-007-010); a gate value that is not an object is shown
     as unsupported too.
  7. *Where the keys are listed.* Options: a page of its own, a tab of the Operations page, or two panels of it.
     Chosen: **one panel of the Operations page with the two listings** (`features/operations/FlowControlKeysPanel.tsx`):
     both need `ops:read`, which the page already checks, they answer one question (which queue is stuck, for how long),
     and a page of their own would add a route and a menu entry for two tables. Each listing is a table with a caption,
     column headers and the group as each row's header, pages through `nextCursor` with "Load more", and filters by
     group; the group filter is sent as typed and the platform's `422` is shown, so the Studio keeps no second copy of
     the derived-group rule. The namespace's `queuedRuns` is shown once per listing, as the platform reads it.

- **D022-111 — The Studio says which group rows were read across pages (2026-11-26; batch 5 correctness minor 5,
  decision D022-101; task T022-056).** Delegate (OD-2026-11-24-4). A group row with `spansPages: true` mixes the
  snapshots of several pages and may show more holders, or grants, than its keys allow; an operator who reads it as one
  snapshot could think a limit is broken. Options: (a) a column of its own (a whole column for a value that is
  `false` on nearly every row, and a cell read apart from the counts it qualifies); (b) a sign beside the counts with
  a tooltip (a tooltip is not reachable by touch and is easy to miss with a screen reader); (c) words under the group
  name in the row's header cell: "Totals read across pages; may be above the limit". Chosen (c): the row header is
  read by assistive technology with every cell of the row, so the warning reaches whoever reads a count, and it is
  text, not colour (WCAG 1.4.1). A row with `spansPages: false`, or from a platform that does not send it, shows the
  group name alone. Test: `FlowControlKeysPanel.test.tsx`, "says on a group row whose sums were read across pages
  that its totals may be above the limit" (red before the words existed).

### Decisions of the documentation slice (2026-11-26)

- **D022-117 — The runbook states the alert conditions the monitor reads, where they differ from the plan's wording
  or from decision D022-077 item 7 (2026-11-26; tasks T022-061 and T022-088; FR-022-049, NFR-022-005).** Delegate
  (OD-2026-11-24-4). Writing the runbook against the code found two disagreements:
  1. *`CONCURRENCY_QUEUE_STUCK`.* The plan ("Observability") says "oldest waiter older than its key's longest holder by
     a threshold while capacity is zero"; decision D022-077 item 7 worded it "a waiter older than the threshold and no
     grant in that time"; the monitor (`crates/infra-postgres/src/operations.rs`, `STUCK_ALERT_SQL`) fires when the
     key's oldest eligible waiter queued more than the threshold ago and more than the threshold before the key's oldest
     current holder took its permit, with the holders at the waiter's own limit. That is the plan's condition. The two
     wordings part when a holder was granted within the last threshold, long after the waiter queued: the monitor fires,
     item 7's wording would not. Options: (a) document the plan's condition, as built and as its test pins it (a holder
     older than the waiter is quiet, a younger one fires); (b) change the code to item 7's wording. Chosen (a): a grant
     made after the waiter queued went past it, however recent; a recent grant to someone else is the condition, not a
     sign of health. Test: `alerts::a_waiter_passed_over_by_every_holder_raises_concurrency_queue_stuck_until_it_leaves`.
  2. *`RATE_LIMIT_GRANT_LAGGING` and the clock.* The plan ("Observability (rate limits)") says the alert covers "the
     pass is not keeping up or the clock went back". As built it reads the key's next-eligible hint, which a database
     clock set back moves into the future with the grants, so it stays quiet then (gap G1183, accepted). Chosen: the
     runbook says plainly that it does not cover the clock and gives the query that does (sections 9 and 12); the plan's
     sentence is revised when the approved texts are next re-approved.

- **D022-118 — Each gate has its own deployment switch (2026-11-26; gap G022-036; FR-022-053, D022-057).** Delegate
  (OD-2026-11-24-4). The processes already read `SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED` and refuse to start with either
  switch on and no digest key file, but the chart and compose could set only the concurrency switch. Options: (a) one
  chart value `flowControl.enabled` for both gates; (b) one value per gate. Chosen (b): the gates are activated per
  capability (`flow-control-activate --capability`), an operator may want concurrency keys without rate limits, and
  one value per gate keeps the chart's switches the processes' switches one for one. The render fails when either is
  on without `flowControl.digestKeys.existingSecret`; `compose.yaml` passes the rate variable (default `false`); the
  local overlay turns both on. Test: `contract_scalability_ha::flow_control_chart::*`.

- **D022-119 — The upgrade tests name their current schema again, and the clock rule spares the shard fence
  (2026-11-26; the merged-tree gates of the slice; batch 4 correctness minor 1; FR-010-003).** Delegate
  (OD-2026-11-24-4). The gates run on the merged slice found two disagreements with rules that `develop` holds:
  1. *The current-schema oracle.* Batch 4 correctness minor 1 had the three release-upgrade tests read the migration
     head from the migrator (`migration_head()`), so that a new migration would not break them. `develop`'s
     `scripts/check-upgrade-boundary.py` requires exactly one written-out `assert_eq!(after, <version>)` in each, so
     that a new migration forces whoever adds it to look at the upgrade tests again. Options: (a) keep the migrator
     read and change the check; (b) write the version out again. Chosen (b): the check is the project's fitness rule,
     and the friction is its purpose; the ruling's concern (a hard-coded head that goes stale) is what the check turns
     into a failure with a message. The oracle is `20270215000000`, this branch's last migration.
  2. *The volatile clock in `maintenance.rs`.* `contract_scalability_ha::no_candidate_query_bounds_an_index_range_with_a_volatile_clock`
     refuses any comparison with `clock_timestamp()` in `maintenance.rs` and `operations.rs`, because the planner
     cannot use it as the bound of an index range (FR-010-003). The rest of a key's window (D022-064, commit
     `d08bb79b`) checks its shard's lease with `s.expires_at > clock_timestamp()` inside an `EXISTS` on
     `scheduler_shards` by its key: one row, no range. There the volatile clock is the correct one: `now()` is the
     transaction's start, and a lease that ran out during the transaction would still fence in. Options: (a) use
     `now()`; (b) narrow the rule to spare that fence. Chosen (b): the fence must be exact, and the rule's reason
     does not apply to a lookup by key. The test spares exactly the text `s.expires_at > clock_timestamp()` and says
     why; any other comparison with the volatile clock in those files is still refused.

## Gaps

| ID | Gap | Status |
|---|---|---|
| G1160 | Cache and step concurrency in bodies need per-instance evidence (014 exact-path transactions). | Open; `UNSUPPORTED_NODE` |
| G1161 | Blob outputs are not cached, mocked or pinned (ADR-0023 reference counting needed). | Open |
| G1162 | A parent `child` node cannot complete on the child's early result. | Open |
| G1163 | Rotating the flow-control digest key is refused while semaphore holders or waiters exist; no live re-keying. | Open; runbook |
| G1164 | Database query results are not cacheable (ADR-0035 decision 10). | Open |
| G1165 | Grants are FIFO by admission commit until 019; with 019 the order is D1602. | Dependency |
| G1166 | Standalone business-ID policies are 018. | Dependency |
| G1167 | Hot-key semaphore contention is unmeasured (NFR-022-004). | Open; benchmark |
| G1168 | FIFO follows admission commit order, not event arrival (G74 stands). | Accepted |
| G1169 | Approval nodes cannot be mocked. | Open |
| G1170 | No handler for `CANCELLED`. | Accepted (D1173) |
| G1171 | `step.cache_hit` names its source run: run readers learn two runs had equal keys. | Accepted (same audience) |
| G1172 | Whether erasure of a source run also removes the copies already delivered to consumer runs (hits, `fromRun` pins) is 020's erasure policy; this feature only exposes them by source. | Dependency (020) |
| G1173 | A lower-limit waiter can starve behind higher-limit grants until its queue timeout (D1196). | Accepted; bounded by timeout |
| G1174 | A buffered waiter is granted by a scheduler pass: up to one pass interval (500 ms by default) after its slot frees, plus a shard takeover; and a key gets a bounded number of grants per pass (D022-015), so a very high declared rate on one key is under-delivered. Neither figure is measured. | Open; benchmark T022-089 |
| G1175 | `rateLimit` on a body node (a step inside `forEach`, `while`, `repeatUntil`) is refused (`SCOPED_RATE_LIMIT`); the fan-out case is served only by a body `child` node starting a rate-limited workflow. | Open; deferred past 0.3.0 by the owner (O1167, OD-2026-11-13-16) |
| G1176 | A rate limit is a ceiling, not a throughput: with a busy key the grants arrive as `limit` at once and again as they age out; there is no smoothing and no `burst`. | Accepted by the owner (O1163, OD-2026-11-13-16) |
| G1177 | `rateLimit` with `continueAsNew` is refused, as `concurrency` is. | Open |
| G1178 | The cost of one hot key (one row lock per grant, the count of its live grants, one fenced transaction per waiter) with thousands of waiters is unmeasured. | Open; benchmark T022-089 |
| G1179 | The estimated grant time is an estimate and is absent in the cases of D022-014. | Accepted |
| G1180 | The limit bounds run starts and step leases, not the calls an activity makes during an attempt nor the moment worker code reaches the downstream system after its lease. | Accepted; documented |
| G1181 | Feature 018 lives on another branch: its reset-rule registry and its refusal reasons must gain the `rate_limit.*` rules and `RATE_LIMIT_SLOT_UNAVAILABLE` when both are on one tree; until then T022-086's 018 cases are pending, not passed. *Corrected 2026-11-13 (D022-031): schedule firing is already on `develop` and is tested now; reset, batch rerun, with-start and the instance registry are the pending part.* | Dependency (018) |
| G1182 | With feature 031, a rate-limit key over an encoded location must be refused by a code 031 registers (as `ENCODING_READ_BY_CONCURRENCY_KEY`). | Dependency (031) |
| G1184 | A grant's instant is read before its commit; work becomes visible up to one transaction later, so starts observed by commit time can be closer than `per` by that duration. Accepted; bounded by transaction duration, measured in T022-089. | Accepted |
| G1185 | A key label is an online oracle: a caller who may see labels and start runs can confirm a guessed key value by the label it gets (like G1171). FR-022-057 limits labels to callers who may disclose the key's source. | Accepted; documented |
| G1186 | Review findings not relayed to this author (architecture N2–N5, security three nits, correctness N2) are not addressed in this revision; the review records say so. | Open |
| G1187 | The namespace-level warnings `CONCURRENCY_LIMIT_MISMATCH` and `RATE_LIMIT_MISMATCH` need the declarations of the other published definitions of the namespace; the codec cannot see them and no store keeps them yet. *Closed 2026-11-16 (D022-041): the declarations table and the publication check; proved by `integration_flow_control_step_policies::publication`.* | Closed |
| G1188 | `RunState::restore` (a run rebuilt from its projection) does not restore gate state. No stored run has any in this build (admission refuses gated definitions; the projection writer refuses a gated step's wait). *Partly closed 2026-11-19 (D022-044): a step's gates are restored from `steps.flow_control_gates`; a run's start gates are not (T022-022).* *Closed 2026-11-20 (D022-049): a run's start gates are restored from `runs.flow_control_gates`.* | Closed |
| G1197 | The collection- and loop-finish paths (`crates/infra-postgres/src/primitives.rs`, `loop_progress.rs`, `crates/scheduler/src/passes.rs`) write their successor's step row directly: a gated root successor of a `forEach` or a loop is refused there (`RuntimeNotIntegrated`, fail closed; the finish is retried) instead of being decided. Corrected 2026-11-19: the first text said they refused an unexpected evaluation, which was true only for an `end` successor. | Open; T022-096 follow-up |
| G1198 | No deployment configuration carries the digest keys (`KeyRing`) to the stores of the control plane and the engine: a gated step is refused `BINARY_CAPABILITY_MISSING` without them. Lands with the capability listing (D022-037). *Extended 2026-11-19 (D022-045): a scheduler replica needs the ring too; without it the grant pass grants nothing (`unsupported`) and keeps its keys due; the timeout routine does not need it.* *Closed 2026-11-20 (D022-057): the ring is the file `SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE` in both binaries (a switch on without it does not start); both list `flow-control-concurrency-v1` with the switch and the ring; the admission point admits a gate a process serves. Open: the rate gate's listing (T022-072, T022-074).* | Closed for the concurrency gate; rate with T022-074 |
| G1199 | The readiness gate applies the declared `limit` and `queueTimeout`; the namespace's `concurrency_limit` and `flow_control_queue_timeout_ms` ceilings (`FlowControlCeilings`) are not read there yet, nor the live-keys ceiling (`FLOW_CONTROL_LIMIT` of FR-022-054) or a waiters-per-key bound (security review of 2026-11-19, Major): with distinct keys a caller can create key rows without bound in one namespace. No process may list the capabilities before these are applied. *Partly closed 2026-11-19 (D022-045): the namespace limit, queue timeout and live-keys ceilings apply at readiness (`FLOW_CONTROL_LIMIT`); step waiters are bounded by the plan per run and gated node, not per key. Open: a namespace ceiling of rate-limit keys and values (T022-077); run waiters per key (T022-022, T022-072).* *2026-11-20 (D022-057): the concurrency half is met: run waiters per key are bounded at admission (`CONCURRENCY_QUEUE_FULL` under the key's lock, `run_admission::*`) and `queued_runs` is exact (D022-051), so the concurrency capability is listed; the rate half (T022-077) stays open and the rate capability unlisted.* *2026-11-20 (D022-064): the rate half is met: `rate_limit_live_keys` and the per-window limits apply at readiness and admission, and the rate capability is listed.* | Closed (D022-057, D022-064) |
| G022-001 | The readiness gate records no metric or span yet (`concurrency_acquisitions_total`, `rate_limit_decisions_total`, `workflow.concurrency.acquire`, `workflow.rate_limit.admit`, rule 10; architecture review of 2026-11-19, M3). A grant counts as unleased until the pass records its lease (T022-074), so a rate-limited step key admits only `limit` steps in all until then. A key that yields `null` shares the group's single key. *Partly closed 2026-11-19 (D022-045): decisions, grants, timeouts, dequeues and returns of the step scope are counted after commit and spanned by group. Open: the wait histograms and waiter gauges, the grant-lag and unleased-grant series and their alerts (T022-082).* *Built 2026-11-25 (D022-083): the wait histograms and the grant lag at the grant from the queue, the waiter and unleased-grant gauges by the monitor, and the four alerts.* | Closed for the telemetry (T022-082, not ticked until its review); T022-074, T022-104 |
| G022-002 | The live-keys ceiling is checked without a namespace lock: concurrent transactions that each create a new key at the boundary can pass it by at most the new keys the others create (their number minus one when each creates one key). *Accepted 2026-11-19 by the owner (O1172 (a), OD-2026-11-19-4) for 0.3.0; the bound is documented in `docs/runbooks/flow-control.md` (D022-047). The exact bound is in keys when a round creates several (O1173), accepted 2026-11-20 (OD-2026-11-20-3).* *Extended 2026-11-20 by the owner to `rate_limit_live_keys` (OD-2026-11-20-11, "Yes, extend it"; D022-066 item 6), which closes the security reviews' suspicion 4.* | Accepted (OD-2026-11-19-4; in keys, OD-2026-11-20-3; rate keys, OD-2026-11-20-11) |
| G022-003 | A rate-limit grant's lease instant is recorded by a pass that is not built (transaction T23, T022-074): a leased grant counts for ever, so a rate key that reached its limit frees only by a returned slot or a dequeue, and its window never rests until an age-out instant. T022-097's rate half rests on it. *Closed 2026-11-20 (D022-064 items 4 and 5): the lease pass records the instant; the rest rule waits for the age-out of every waiter's window.* | Closed; T022-074 |
| G022-004 | `verify_run` does not check that a `wait_timed_out` comes at or after the deadline its `*.queued` journaled; the reservation counts an eligible waiter ahead without its own position (conservative). | Open |
| G022-005 | An orphaned waiter (of a run that ended) is logged and counted (`outcome="orphan"`), not alerted; the runbook does not list the `gate_grant` and `gate_timeout` action kinds. *2026-11-25 (D022-083): alert `FLOW_CONTROL_WAITER_ORPHANED` (runbook section 14).* | Alert built (T022-082); the action kinds with T022-061 |
| G022-006 | The digest-key commands (activation, force, listing) are SQL functions with no audited API, CLI, span or metric (NFR-022-005, constitution VI; security review of 2026-11-20, finding 4); the force's history is append-only only for roles that may not drop its trigger, and a deployment where operators share the schema owner's login records whatever name they type and can retire a version by a direct `UPDATE` with no record (finding 5). The runbook tells operators to use their own login and never to edit the table by hand. | Open; T022-061, T022-082; a dedicated operator role |
| G022-007 | FR-022-021 says "pausing a holder MUST NOT release its permit", and FR-022-019 counts a granted, unleased step as a holder, while FR-022-046 says a pause returns the permit and slot of a granted, unleased step: the approved texts disagree for that case; the build follows FR-022-046 (D022-048). T022-025's "pause keeps permit" holds for a leased step and a run holder. `data-model.md` ("Steps and their gates", invariant 11) does not say that a step a pause gave back waits with no waiter row. Revised when the approved texts are next re-approved. | Open; re-approval of the texts |
| G022-008 | A step queued when its run pauses keeps its waiter row and its deadline, which runs during the pause: a long pause can fail it `CONCURRENCY_WAIT_TIMEOUT` or `RATE_LIMIT_WAIT_TIMEOUT` (the timeout routine does not look at the pause), while a step granted before the pause waits with no deadline (correctness review 4 and architecture M2 of 2026-11-20). Not introduced by T022-100. | Open; O1175 (a) accepted without option (c) (OD-2026-11-20-2), so the deadline rule is a later decision |
| G022-009 | A start delay or a reset of a definition that gates its start is refused `BINARY_CAPABILITY_MISSING` (D022-049): the plan says the gate is decided at the delay's release, and a reset takes a fresh permit or is refused `CONCURRENCY_PERMIT_UNAVAILABLE` (never queues); neither is built. A child node whose run's start is refused (`CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`) is retried by the next pass instead of failing its node with the typed retryable code (T022-102). *Closed in part 2026-11-20 (T022-064): the delay is decided at its release (D022-052) and the reset takes a fresh permit or is refused (D022-053). Open: the child node's typed code (T022-102).* | Open; T022-102 |
| G022-010 | *Closed 2026-11-25 (D022-087, T022-109): the signal is held with the queued run and delivered by the grant.* A Signal- or Update-with-Start of a definition whose start queues is refused `SIGNAL_NOT_ACCEPTED_AT_START` (no step waits while the entry is held), so the outcome depends on the key's load (correctness review Major 2 of 2026-11-20). Chosen design (D022-050 item 6): hold the signal or update with the queued run and deliver it at the grant. *2026-11-20 (T022-064):* an Update-with-Start is not affected (it changes the declared state, which a queued run has: `run_creators::an_update_with_start_admits_through_the_key_and_applies_its_update`); a Signal-with-Start keeps the typed refusal with nothing created (`run_creators::a_signal_with_start_whose_start_would_queue_is_refused_and_creates_nothing`). The hold-and-deliver needs a durable held signal, its delivery in the grant's transaction and the replay of its receipt: follow-up T022-109. | Built (D022-087); closed when T022-109 is ticked |
| G022-011 | `queued_runs` was believed counted without a lock (D022-050 item 3). *Closed 2026-11-20 (D022-051):* the count is exact. Only an admission adds a run waiter, every admission holds the namespace's active-run quota lock from before its runs row to its commit, and the start's gate counts after it in a fresh READ COMMITTED statement; proved by `run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other` (the lock held by the test, both starts observed waiting on it, then exactly one queued and one refused), red under two hand mutations (the lock removed; the ceiling ignored). The merge blocker the owner set on it is met. T022-105's counter rows remain for the cost of the count (a scan of the namespace's run waiters, at most the ceiling) and its privileges. | Closed (exactness); T022-105 open for the counter rows |
| G022-012 | The start's decision (`decide_start_gates`) and the step's (`decide_gates`) are two code paths over the same rules; T022-072 builds one core for both levels and both gates (D022-050 item 5; architecture review Major 1 of 2026-11-20). A paused queued run's deadline runs during its pause, as G022-008 says of a step. *2026-11-20 (D022-054): the start's three causes share one validator (`check_start_decisions`) and one adapter function; the step's decision is still its own path.* *Closed 2026-11-20 (D022-064 item 2): `gates_of` and `decide_filed` serve both levels and both gates.* | Closed; T022-072 |
| G022-013 | Keys-review correctness m5 and m8 (2026-11-20): the takeover test races a raw `UPDATE` of the shard lease, not a keeper's takeover, and checks "nothing twice" afterwards; and SC-022-004 at run level lacks five same-key starts with `limit: 1` (at most one holder, grants in admission order) and a mixed-limit pair sharing a group. | Open; T022-110 |
| G022-014 | Without the label, a key's counts and the oldest-wait order still let a caller who may not reveal personal data partly link runs (one key of a group with waiters: every `WAITING` run of the group waits on it). The labels are hidden and the instants truncated to the minute (D022-060), which narrows and does not stop linking (D022-061). An aggregated listing per (pool, scope, group) for such callers closes it: T022-111 (D022-061). *2026-11-20 (D022-067 item 8):* `getRun`'s `rateLimit.position` is another surface of the same class (how many runs of the key are ahead); the estimate is withheld from such callers. *2026-11-21 (T022-086 security re-review, nit):* one bit more of the same class, a key's fullness: the synchronous `409` of a reset refused `RATE_LIMIT_SLOT_UNAVAILABLE` or `CONCURRENCY_PERMIT_UNAVAILABLE`, and the `failureReason` of the reset register (`run_resets.failure_reason`), which shows it afterwards to anyone who can read the entry; no code change. *Built 2026-11-25 (D022-086): a caller without the reveal right reads one row per group in both listings.* | Open until T022-111 is ticked; a release item with T022-104 |
| G022-015 | Publication does not yet refuse a concurrency key over a `secret` location (T022-104), while this build lists and can activate the concurrency capability (D022-057). The runbook forbids production activation before it; the release cannot close with it open. *Built 2026-11-25 (D022-085): `FLOW_CONTROL_KEY_SECRET` refused at publication, `FLOW_CONTROL_KEY_PII` warned, a run's labels per the level its key reads.* | Open until T022-104 is ticked; release gate |
| G022-016 | A reset of a run whose start declares `rateLimit` is refused `BINARY_CAPABILITY_MISSING` (D022-064 item 7): `RATE_LIMIT_SLOT_UNAVAILABLE` (018's code, G1181) and the reset rules of the `rate_limit.*` events land with T022-086. The refusal is retryable and no process of this build lifts it, so a pending reset of such a run is retried until it is cancelled: the runbook forbids production activation before it (D022-066 item 5), and `run_creators::a_reset_of_a_rate_limited_start_is_refused_until_its_rules_are_built` pins it. *Built 2026-11-21 (T022-086, D022-069):* a reset takes a fresh slot right after `run.reset` or is refused `RATE_LIMIT_SLOT_UNAVAILABLE`, rate first, and the five `rate_limit.*` events have the rules of `contracts/journal-events.md`; the interim refusal and its test are gone. *Closed 2026-11-21:* T022-086 ticked after its re-review (three reviewers, 0 Blockers, 0 Majors). | Closed (T022-086) |
| G022-017 | The purge keeps a grant for its window row's longest period seen (`max_period_ms`) plus a minute: an acquirer that later declares a longer window on the same key (a `RATE_LIMIT_MISMATCH` warned at publication) does not count the grants purged before it arrived, and may be admitted up to their number early in its first window. | Open; documented in the runbook |
| G022-018 | The grant purge visits keys in no order, without key-shard partitioning or an index on the lease instant (a full scan of `rate_limit_grants` a call): under steady per-second traffic on more keys than it visits per housekeeping its backlog grows (reviews of 2026-11-20, architecture Major 5). *2026-11-20 (D022-066 item 1):* the rows of a namespace are bounded by its `rate_limit_live_keys` again, and a decision that would be refused reclaims its own namespace's purgeable windows of at most 64 grants first, so only a window of more grants waits for this purge. Open: the order, the shards, the index, a time budget, an `outcome="failed"` and an oldest-purgeable-age gauge; a call stops at the first round that deleted nothing, which a round of keys all held by others also is (re-review correctness minor 3). | Open; production gate (runbook section 6); T022-062, T022-089 |
| G022-019 | A reset with the strategy `CANCEL_SOURCE` of a rate-limited source whose key's window is full cancels its source and then fails `RATE_LIMIT_SLOT_UNAVAILABLE`, with no run replacing it (D022-069 items 3 and 8; reviews of 2026-11-21, architecture Major 1, security minor; re-review architecture minor C). The runbooks warn and advise `REQUIRE_TERMINAL`; the two contracts say so. Task T022-114 builds the advisory refusal at the request, or a warning in the reset preview. *Built 2026-11-25 (decision D022-079):* the request is refused `RATE_LIMIT_SLOT_UNAVAILABLE` before the source is cancelled when the key's window is full as it arrives (advisory: the window can still fill before the source ends). | Open until T022-114 is ticked; not a production gate (D022-069 item 8) |
| G022-020 | A namespace's `concurrency_limit` lowered while runs are queued for a permit: the grant routine compares a queued waiter with the limit filed at its admission, as the rate gate did before D022-071; the specification's edge case speaks of a new version's lower limit only (`spec.md`, US02 edge cases). Whether a lowered concurrency ceiling applies to queued waiters at once is a product question for the coordinator (OD-2026-11-20-6) before it is built. *Closed 2026-11-21 (decision D022-074):* the grant from the queue evaluates a permit waiter, and a waiter ahead, against the lower of its filed limit and the current `concurrency_limit`; held permits are never revoked; `lifecycle::a_lowered_concurrency_ceiling_applies_to_the_queued_starts_and_revokes_no_permit`. | Closed; D022-074 |
| G1189 | Nothing in the database computes or checks `key_shard` and the waiter `shard`; two binaries that hashed differently would file one key under two shards. | Open; T022-097 |
| G1190 | `flow_control_key_activate` records the operator as typed and keeps no history row with the database role, unlike the registry's activations. *Partly closed 2026-11-16 (D022-042): the version row records `activated_role` (`session_user`); a history of earlier activations of the same version is not kept, since a version is activated once. 2026-11-19 (D022-046): every force has an append-only history row with the database role and a reason; the runbook part is `docs/runbooks/flow-control.md`.* | Open; T022-061 (the rest of the runbook) |
| G1191 | Untested: the down migrations' refusal when `run_events` holds a `concurrency.*` or `rate_limit.*` event; no contract test parses 018's reset-rule table against `RULES`. | Open |
| G1192 | An older replica's quota views read every stored limit and refuse a resource name its build does not know (`stored_limits`): a tenant that sets one of the new limits gets an error from an older replica's quota listing until it is upgraded. True of every resource added before. | Accepted; the next resource change could make the reader skip unknown names |
| G1193 | Activating a digest key version that a live process was not configured with makes that process refuse every gated decision (`UnknownVersion`) until it is; the registry does not check the key versions processes hold. | Open; ADR-0093, runbook (T022-061) |
| G1194 | The pinned operation baseline of `contract_iceberg_writes` digests each operation without following `$ref`, so a change to a shared parameter or schema (as the quota enum of D022-040) moves no digest; `contract_flow_control_step_policies::quotas` pins this enum to the code instead. | Accepted |
| G1195 | Approved texts still describe the drain-first rotation: FR-022-020's last sentence, `data-model.md` (key versions, "only by an audited operator command"), G1163; `data-model.md` does not list `flow_control_group_declarations` or `activated_role`; `contracts/errors.md` does not say that `*_POLICY_INVALID` covers the namespace's ceiling of node-level declarations. Changing them makes the docs drafts again (constitution, Approval). *O1170 answered 2026-11-19 (OD-2026-11-19-2). Also out of date since the force (D022-046): FR-022-056's "no key is ever split between two rows" and SC-022-028 (a forced rotation splits a key into at most two rows), and `data-model.md` lacks `flow_control_key_forced_retirements`. The texts, and the bound of D022-047 in `contracts/errors.md`, are revised when the approved texts are next re-approved (a change makes them drafts again).* | Open; re-approval of the texts |
| G1196 | A rotation's progress (rows left under the retiring version) has no metric and no audit record; the operator learns it by running the command again. *2026-11-19: likewise the rows left under a forced version (an over-admission in progress, D022-046); a force has an audit row, an activation does not.* | Open; T022-061, T022-082 |
| G022-036 | The deployment files cannot turn the rate gate on: the Helm chart has one switch, `flowControl.concurrencyKeys` (`infra/helm/values.yaml`, `templates/_helpers.tpl` `sagat.flowControl.env`), and no value for `SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED`; its comment still says "The rate gate is not served by this release"; the overlay `infra/docker/compose.flow-control.yaml` sets the concurrency switch only. The runbook (section 6) says so and gives the local workaround (found 2026-11-26, task T022-088). | Closed by D022-118 (2026-11-26) |
| G022-037 | An orphaned waiter (alert `FLOW_CONTROL_WAITER_ORPHANED`) is removed by a hand `DELETE` as the schema owner, with no API, command or audit record, while NFR-022-005 asks an audited API for every operator action; the same holds for the digest-key commands (G022-006). The runbook (section 14) tells the operator what to keep in the change record (found 2026-11-26, task T022-061). | Open; with G022-006's operator role |
| G1183 | A database clock that moves backwards under-delivers until it catches up (NFR-022-008); no alert tells the operator why grants paused beyond `rate_limit_grant_lagging`. | Accepted; runbook |

## Owner questions

### O1160 — May draft test runs execute in `prod` namespaces?
Unmocked steps are real executions. **Recommended default: no** (`TEST_RUN_PRODUCTION_NAMESPACE`);
production debugging uses 018's reset. An opt-in would need its own audit design.

### O1161 — May a failure handler mark the run recovered?
Windmill's `recover: true` turns a failure into success; after compensation that misstates what happened.
**Recommended default: no**; the handler never changes the outcome.

### O1162 — Which secrets may an unmocked step of a draft test run use?
FR-005-005 forbids production secrets in script tests; draft tests are refused in `prod`. **Recommended
default:** namespace secrets under the actor's current `secret:use` and access lists (017 precedent),
audited per resolution. Alternative: none, forcing secret-declaring steps to be mocked.

### O1163 — What does "N per window" mean?
**Answered 2026-11-13 (OD-2026-11-13-16, "Aceitar todas"): option (a), the strict sliding window, with no `burst` field.**
The owner asked for N executions per second or minute per key with the excess buffered. Options:
(a) **strict sliding window**: never more than N grants in any interval of the window; the first N start
at once, the next when those age out; (b) **token bucket with `burst`**: grants are spread evenly and an
idle key may start `burst` at once; a window can then hold up to N + `burst`, and because buffered work
is granted by a pass every 500 ms a bucket without burst cannot deliver more than two grants a second per
key; (c) **fixed window** aligned to the clock: cheapest, but up to 2N across a boundary.
**Recommended: (a)**, with no `burst` field in 0.3.0 (D022-003, ADR-0088). An author who wants a smoother
load declares a shorter window (`10 per second` rather than `600 per minute`).

### O1164 — What order does the buffer guarantee?
**Answered 2026-11-13 (OD-2026-11-13-16): as recommended — first in, first out within a key, with skip-ahead; no order between keys.**
**Recommended:** first in, first out within one key, by the commit that buffered the work; no order
between keys; a waiter that its own limit or its concurrency key refuses is skipped rather than blocking
the ones behind it (it can then wait until its queue timeout, G1173). Alternative: strict order with
head-of-line blocking (simpler to explain, but one blocked waiter stops its whole key).

### O1165 — What happens when the buffer is full?
**Answered 2026-11-13 (OD-2026-11-13-16): option (a) — a synchronous start is refused with the typed error; event and cron starts stay pending.**
The buffer is bounded per key (1,000 waiters by default, 10,000 at most) and per namespace (`queued_runs`).
Options: (a) **refuse** a synchronous start with a typed error (`QUOTA_EXCEEDED`/`RATE_LIMIT_QUEUE_FULL`)
before a run exists, and keep event and cron starts pending under their own retry; (b) make the caller
wait; (c) drop the oldest or the newest waiter. **Recommended: (a)**, the same behaviour as a full
concurrency queue; (b) holds a request open without bound and (c) loses admitted work.

### O1166 — Does a retry of a rate-limited step take a new slot?
**Answered 2026-11-13 (OD-2026-11-13-16): yes, every attempt takes a slot; the wait spends no attempt.**
**Recommended: yes.** The limit exists to protect what the step calls, and a retry calls it again; the
wait for the slot spends neither an attempt nor the retry budget. Alternative: only the first attempt
counts (higher throughput under failures, but a failing provider is then called above its limit).

### O1167 — Rate limits on steps inside collections in release 0.3.0?
**Answered 2026-11-13 (OD-2026-11-13-16): not in 0.3.0 — root graph and the body `child` shape; body nodes are the first follow-up (G1175 stays open).**
The classic case is a `forEach` whose body calls a limited provider. The slice refuses `rateLimit` on body
nodes (G1175) because body instances are scheduled by 014's exact-path transactions and need their own
fault evidence; the case is served by a body `child` node whose workflow declares a top-level `rateLimit`,
at the cost of one child run per item. **Recommended: ship 0.3.0 with the root graph and the child shape,
and take body nodes as the first follow-up.** Alternative: hold the slice until body nodes are covered
(larger; see the size estimate in the plan).

### O1168 — May a reset of a rate-limited run wait for a slot?
**Answered 2026-11-13 (OD-2026-11-13-16): no — a reset takes a slot or is refused (`RATE_LIMIT_SLOT_UNAVAILABLE`).**
018 says a reset never queues. **Recommended: the same for a rate slot** — the reset takes a slot when one
is free and is refused otherwise (`RATE_LIMIT_SLOT_UNAVAILABLE`), so the limit holds for every start.
Alternative: a reset bypasses the limit (an operator's action always proceeds, and the window can exceed
N by the number of resets).

### O1169 — If ADR-0089 is not accepted, does node-level `concurrency` ship in 0.3.0?
**Answered 2026-11-13 (OD-2026-11-13-19, "Aceitar o ADR-0089"): option (a) — ADR-0089 is accepted, so the question's condition does not hold and node-level `concurrency` ships under ADR-0089.**
ADR-0056 decision 3 (permit at claim) fills the claim's window with steps nobody can lease (D022-019).
Options: (a) **accept ADR-0089**: gated steps wait outside `READY` and are granted by the pass, as rate
limits already are; (b) ship 0.3.0 with run-level `concurrency` only and refuse node-level `concurrency`
(`UNSUPPORTED_NODE`) until (a); (c) keep decision 3 as accepted. **Recommended: (a)**; (b) is the fallback if
the owner wants more time on the ADR; (c) is not safe for a shared activity.

### O1170 — What ends a digest-key rotation that live rows keep open?

**Answered 2026-11-19 (OD-2026-11-19-2, "Sim, como recomendado"): (a) with (b) for incidents — ADR-0093 accepted; an operator-only, audited force retires the old version at once, a key may over-admit by at most its old row's holders until they drain (built under T022-108, D022-046).**
Asked 2026-11-16 (ADR-0093, proposed; security review S1, architecture review M2). As built, a rotation ends when the old
version's last semaphore or window row goes, and a third version is refused until then. A run-level permit lives as long
as its run, so one long-lived run of any tenant can keep a rotation open for weeks; an emergency rotation of a leaked key
(a third version) waits for it, and every process must keep the retiring key meanwhile. A key cannot be re-keyed without
its plaintext, which only a deciding transaction has.
Options: (a) **accept as built**, document in the runbook that an emergency rotation first waits for (or cancels) the
runs that hold old keys, and list them for the operator; (b) an operator-only **force** outcome that retires the old version
at once: its rows live on under their old digest until they drain, so a key may have two rows meanwhile and its limit can
be exceeded by at most the old row's holders, audited; (c) **several retiring versions** (at most 3), each decision
computing up to four digests and every process holding every retiring key; (d) **re-key on touch** besides (a): a decision
that finds a key under the retiring version moves the row to the active digest, which shortens rotations for keys in use
but not for permits held and never decided again. **Recommended: (a) with (b) for incidents** — the normal rotation stays
safe and bounded by the workload, and a leak can be answered at once with a stated, bounded over-admission. The owner
also accepts or changes ADR-0093 (it supersedes the last sentence of ADR-0056 decision 7).

### O1171 — May the readiness gate decide in an evaluation round before `persist` (ADR-0094)?
**Answered 2026-11-19 (OD-2026-11-19-3, "Sim, como recomendado"): (a) — ADR-0094 accepted.**
ADR-0089 decision 2 (accepted) puts the decision inside `persist`. The build (T022-096, D022-044) decides it in an
evaluation round the planner asks for, before `persist`, because `persist` locks usage rows before it could lock a key and
cannot route a key that fails to evaluate (ADR-0094, proposed). Options: (a) accept ADR-0094; (b) keep ADR-0089 decision 2
as written, which needs `persist` reordered (usage rows after the keys) and a key failure that does not route (a stuck
step instead of FR-022-054's failure route); (c) another placement. **Recommended: (a).** T022-096 is not ticked until the
owner answers.

### O1172 — May the live-keys ceiling be passed by concurrent key creations (G022-002)?
**Answered 2026-11-19 (OD-2026-11-19-4, "Sim, como recomendado"): (a) — the bounded overshoot is accepted for 0.3.0 and documented (`docs/runbooks/flow-control.md`, D022-047); no new lock.**
The readiness gate checks a step's new key against the namespace's live-keys ceiling in its own transaction, without a
namespace lock (D022-045): transactions that each create a new key at the boundary at the same moment can pass it by at
most their number. Options: (a) accept the bounded overshoot for 0.3.0 and document it (no new lock); (b) serialize key
creation per namespace with a transaction-scoped advisory lock taken before any key row (exact, one more statement per
new key, and a new lock to keep in the one order); (c) a counter row per namespace kept by the key rows' triggers.
**Recommended: (a)** for 0.3.0, the ceiling's purpose (no unbounded growth, the security review's Major) holding; (b)
if the owner wants the ceiling exact.

### O1173 — Is the live-keys overshoot bounded in keys rather than in transactions?
**Answered 2026-11-20 (OD-2026-11-20-3, "Aceitar as duas"): (a) — the bound is in keys: at most the new keys created by the other transactions of the same moment; documented in `docs/runbooks/flow-control.md` section 4, no lock added.**
Asked 2026-11-19 (D022-047), after OD-2026-11-19-4 accepted O1172 (a) as "at most the number of transactions creating
keys at the same moment". That holds when each transaction creates one key. A round that releases several gated steps
with distinct new keys in one commit (parallel branches) creates up to the room it saw, so concurrent rounds can pass the
ceiling by up to the new keys the others create. Options: (a) **accept the bound in keys** (at most the new keys created
by the other transactions of the same moment; each is bounded by the plan's gated steps per commit) and document it;
(b) a round creates at most one new key without a namespace lock and takes the advisory lock of O1172 (b) to create more;
(c) O1172 (b) for every new key. **Recommended: (a)**: the ceiling's purpose (no unbounded growth) holds, and the case
needs several gated nodes released together at the boundary. Nothing is built differently until the owner answers.

### O1174 — Does "at most its old row's holders" count the waiters the old row grants after the force?
**Answered 2026-11-20 (OD-2026-11-20-4, "Aceitar as duas"): (a) — the excess is the old row's holders at each moment, never more than its limit, until its holders and waiters drain; ADR-0093 decision 5 and the runbook say so.**
Asked 2026-11-20 (correctness review M1, D022-046). As built, a forced key's old row keeps its waiters and grants them
within its own limit, so the key is over-admitted at any moment by what the old row holds then, which can grow back to
its limit after the force while its waiters are granted (for instance a dual-gated head that waited on its other gate).
Options: (a) **accept this reading**: the excess is the old row's holders at each moment, never more than its limit,
until its holders and waiters drain; (b) the old row grants nothing after the force and its waiters are re-decided under
the active version (a new journal transition from a dequeue back to a decision, its replay rule and `verify_run` check):
the excess is then the holders at the force and falls as they finish; (c) the old row's waiters time out. **Recommended:
(a)**: it is bounded by the limit and by the drain, needs no change to the journal, and keeps each waiter's place in
its own row. (b) is the exact reading of the words, at the cost of a design change; nothing waits on the answer except
the wording of ADR-0093 decision 5.


### O1175 — May a pause give back a granted, unleased step and ask again on resume (ADR-0096)?
**Answered 2026-11-20 (OD-2026-11-20-2, "Aceitar ADR-0096"): (a) — ADR-0096 is Accepted and partly supersedes ADR-0094 decisions 2 and 3; T022-100 is ticked with its evidence. G022-008 (option (c)'s deadline rule) stays open.**
Asked 2026-11-20 (T022-100, D022-048; architecture review B1 of 2026-11-20). FR-022-046 (approved) says a pause returns
the permit and slot of a granted, unleased step, makes it wait again, and the resume makes it ask again. ADR-0094
(accepted 2026-11-19) says the planner asks every undecided gated step "running or paused" and `persist` refuses an
undecided wait. Options: (a) **accept ADR-0096**: the build as described in D022-048 (a step a pause gave back waits
undecided while paused; one made ready while paused is decided, and a grant is given back at once); (b) keep ADR-0094 as
written: a paused run keeps its grants (against FR-022-046 and invariant 13 while it is paused); (c) (a), and also stop
the deadline of a step queued before the pause while the run is paused (G022-008), which needs the timeout routine to
skip paused runs and the deadline to move by the pause's length. **Recommended: (a)** now, and (c)'s deadline rule as a
separate decision after measurement. T022-100 is not ticked until the owner answers.
## Owner decision of 2026-11-13: throughput control in release 0.3.0

To be recorded on `develop` as **OD-2026-11-13-11** in `docs/owner-decisions.md` (not edited by this
branch). The owner asked for "Controle de througput para fluxos que tenham limitações de execução,
buffering" in release 0.3.0 and chose "Concorrência + taxa": (1) concurrency limits by key with a durable
queue, which is US02 of this feature as specified (FR-022-017..022); and (2) a rate limit per time window
with buffering (N executions per second or minute per key, the excess waits durably instead of failing),
which this feature did not specify and which is added as US07. The other user stories (cache, early
return, failure handlers, standalone steps, test mode) are not in the 0.3.0 slice. The feature itself was
approved for implementation by OD-2026-11-11-1; O1163–O1168 were answered the same day, every
recommendation accepted (OD-2026-11-13-16, "Aceitar todas", recorded on `develop` by the coordinator). The owner then
accepted ADR-0056 and ADR-0088 ("Aceito tudo", OD-2026-11-13-17). After the reviews of 2026-11-13, ADR-0089
(proposed) changes ADR-0056 decisions 3 and 5 and refines ADR-0088 decisions 3 and 7; it needs the owner
(T022-107).

## Owner decisions of 2026-11-16

Recorded in `docs/owner-decisions.md` (section "2026-11-16 — Feature 022: the checklist deviations and the quota
resources of the OpenAPI").

- **OD-2026-11-16-1 — "Aceitar os 5".** The five unticked items of `checklists/requirements.md` are accepted as the
  deliberate deviations the checklist documents (the gate of OD-2026-11-03-33). Each item cites the row; they stay
  unticked because what they say is still true.
- **OD-2026-11-16-2 — "Autorizar, ler o diff".** The quota-resource enum of the canonical OpenAPI gains the
  concurrency resources of T022-013 (D022-040). The pinned operation baseline of `contract_iceberg_writes` is changed
  once, after reading its diff; T022-013's evidence block records what was read and which digests moved.

## Owner decisions of 2026-11-19

Recorded in `docs/owner-decisions.md` (section "2026-11-19 — Feature 022: the digest-key rotation, the readiness round and
the live-keys bound"). The assistant asked "Há seis decisões pendentes, todas com recomendação … Aceitas todas como
recomendado?"; the owner answered "Sim, como recomendado".

- **OD-2026-11-19-2 — O1170 (a) with (b) for incidents.** ADR-0093 is Accepted. The per-key rotation stands, and an
  operator-only, audited force retires the old version at once for an emergency; a key may over-admit by at most its old
  row's holders until they drain (T022-095; the force is T022-108, D022-046).
- **OD-2026-11-19-3 — O1171 (a).** ADR-0094 is Accepted: the readiness gate decides in an evaluation round before
  `persist` (T022-096).
- **OD-2026-11-19-4 — O1172 (a).** The live-keys ceiling may be overshot by at most the number of transactions creating
  keys at the same moment, in 0.3.0; the bound is documented (G022-002).

## Owner decisions of 2026-11-20

Recorded in `docs/owner-decisions.md` (section "2026-11-20 — Feature 022: the pause's returns, the live-keys bound in
keys and the force's bound"). The owner answered "Aceitar ADR-0096" (O1175) and "Aceitar as duas" (O1173, O1174).

- **OD-2026-11-20-2 — O1175 (a).** ADR-0096 is Accepted and partly supersedes ADR-0094 decisions 2 and 3: a pause gives
  back the permit and slot of a granted, unleased step and the resume asks again (T022-100, D022-048). G022-008 stays
  open.
- **OD-2026-11-20-3 — O1173 (a).** The live-keys overshoot is bounded in keys, not transactions, when one commit creates
  several keys (G022-002, D022-047).
- **OD-2026-11-20-4 — O1174 (a).** "Its old row's holders" in the force's bound is read at each moment, up to the old
  row's limit (ADR-0093 decision 5, D022-046).
- **OD-2026-11-20-11 — the rate live-keys count** ("Yes, extend it", section "2026-11-20 — Feature 022: the rate
  live-keys count" of `docs/owner-decisions.md`). The bound of OD-2026-11-19-4 and OD-2026-11-20-3 holds for
  `rate_limit_live_keys`; no namespace lock on rate-key creation (G022-002, D022-066 item 6).

## External semantics for rate limits (not re-read on 2026-11-13)

Stated from the author's knowledge of these products, without quotation; a reviewer should confirm them
before they are cited elsewhere. Windmill's concurrency limits count executions within a time window and
queue the excess (already listed above; D1181 took only the concurrent-holder part). Temporal limits the
rate at which a task queue dispatches activities, on the server, as a smoothed rate. Inngest separates a
lossy `rateLimit` from a queued `throttle` with `limit`, `period`, `burst` and `key`. The shape here keeps
the queued meaning and the `limit`/window/`key` vocabulary, and chooses the strict window for the reason
of D022-003.

