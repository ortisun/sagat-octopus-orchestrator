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
- **D022-037 — A binary announces a flow-control capability only once it runs the gate.** `flow-control-concurrency-v1`
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
| G1198 | No deployment configuration carries the digest keys (`KeyRing`) to the stores of the control plane and the engine: a gated step is refused `BINARY_CAPABILITY_MISSING` without them. Lands with the capability listing (D022-037). *Extended 2026-11-19 (D022-045): a scheduler replica needs the ring too; without it the grant pass grants nothing (`unsupported`) and keeps its keys due; the timeout routine does not need it.* | Open; capability listing |
| G1199 | The readiness gate applies the declared `limit` and `queueTimeout`; the namespace's `concurrency_limit` and `flow_control_queue_timeout_ms` ceilings (`FlowControlCeilings`) are not read there yet, nor the live-keys ceiling (`FLOW_CONTROL_LIMIT` of FR-022-054) or a waiters-per-key bound (security review of 2026-11-19, Major): with distinct keys a caller can create key rows without bound in one namespace. No process may list the capabilities before these are applied. *Partly closed 2026-11-19 (D022-045): the namespace limit, queue timeout and live-keys ceilings apply at readiness (`FLOW_CONTROL_LIMIT`); step waiters are bounded by the plan per run and gated node, not per key. Open: a namespace ceiling of rate-limit keys and values (T022-077); run waiters per key (T022-022, T022-072).* | Open; T022-077, T022-022 |
| G022-001 | The readiness gate records no metric or span yet (`concurrency_acquisitions_total`, `rate_limit_decisions_total`, `workflow.concurrency.acquire`, `workflow.rate_limit.admit`, rule 10; architecture review of 2026-11-19, M3). A grant counts as unleased until the pass records its lease (T022-074), so a rate-limited step key admits only `limit` steps in all until then. A key that yields `null` shares the group's single key. *Partly closed 2026-11-19 (D022-045): decisions, grants, timeouts, dequeues and returns of the step scope are counted after commit and spanned by group. Open: the wait histograms and waiter gauges, the grant-lag and unleased-grant series and their alerts (T022-082).* | Open; T022-082, T022-074, T022-104 |
| G022-002 | The live-keys ceiling is checked without a namespace lock: concurrent transactions that each create a new key at the boundary can pass it by at most the new keys the others create (their number minus one when each creates one key). *Accepted 2026-11-19 by the owner (O1172 (a), OD-2026-11-19-4) for 0.3.0; the bound is documented in `docs/runbooks/flow-control.md` (D022-047). The exact bound is in keys when a round creates several (O1173), accepted 2026-11-20 (OD-2026-11-20-3).* | Accepted (OD-2026-11-19-4; in keys, OD-2026-11-20-3) |
| G022-003 | A rate-limit grant's lease instant is recorded by a pass that is not built (T23, T022-074): a leased grant counts for ever, so a rate key that reached its limit frees only by a returned slot or a dequeue, and its window never rests until an age-out instant. T022-097's rate half rests on it. | Open; T022-074 |
| G022-004 | `verify_run` does not check that a `wait_timed_out` comes at or after the deadline its `*.queued` journaled; the reservation counts an eligible waiter ahead without its own position (conservative). | Open |
| G022-005 | An orphaned waiter (of a run that ended) is logged and counted (`outcome="orphan"`), not alerted; the runbook does not list the `gate_grant` and `gate_timeout` action kinds. | Open; T022-061, T022-082 |
| G022-006 | The digest-key commands (activation, force, listing) are SQL functions with no audited API, CLI, span or metric (NFR-022-005, constitution VI; security review of 2026-11-20, finding 4); the force's history is append-only only for roles that may not drop its trigger, and a deployment where operators share the schema owner's login records whatever name they type and can retire a version by a direct `UPDATE` with no record (finding 5). The runbook tells operators to use their own login and never to edit the table by hand. | Open; T022-061, T022-082; a dedicated operator role |
| G022-007 | FR-022-021 says "pausing a holder MUST NOT release its permit", and FR-022-019 counts a granted, unleased step as a holder, while FR-022-046 says a pause returns the permit and slot of a granted, unleased step: the approved texts disagree for that case; the build follows FR-022-046 (D022-048). T022-025's "pause keeps permit" holds for a leased step and a run holder. `data-model.md` ("Steps and their gates", invariant 11) does not say that a step a pause gave back waits with no waiter row. Revised when the approved texts are next re-approved. | Open; re-approval of the texts |
| G022-008 | A step queued when its run pauses keeps its waiter row and its deadline, which runs during the pause: a long pause can fail it `CONCURRENCY_WAIT_TIMEOUT` or `RATE_LIMIT_WAIT_TIMEOUT` (the timeout routine does not look at the pause), while a step granted before the pause waits with no deadline (correctness review 4 and architecture M2 of 2026-11-20). Not introduced by T022-100. | Open; O1175 (a) accepted without option (c) (OD-2026-11-20-2), so the deadline rule is a later decision |
| G022-009 | A start delay or a reset of a definition that gates its start is refused `BINARY_CAPABILITY_MISSING` (D022-049): the plan says the gate is decided at the delay's release, and a reset takes a fresh permit or is refused `CONCURRENCY_PERMIT_UNAVAILABLE` (never queues); neither is built. A child node whose run's start is refused (`CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`) is retried by the next pass instead of failing its node with the typed retryable code (T022-102). | Open; T022-064, T022-102 |
| G022-010 | A Signal- or Update-with-Start of a definition whose start queues is refused `SIGNAL_NOT_ACCEPTED_AT_START` (no step waits while the entry is held), so the outcome depends on the key's load (correctness review M2 of 2026-11-20). Chosen design (D022-050 item 6): hold the signal or update with the queued run and deliver it at the grant. | Open; T022-064 |
| G022-011 | `queued_runs` was believed counted without a lock (D022-050 item 3). *Closed 2026-11-20 (D022-051):* the count is exact. Only an admission adds a run waiter, every admission holds the namespace's active-run quota lock from before its runs row to its commit, and the start's gate counts after it in a fresh READ COMMITTED statement; proved by `run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other` (the lock held by the test, both starts observed waiting on it, then exactly one queued and one refused), red under two hand mutations (the lock removed; the ceiling ignored). The merge blocker the owner set on it is met. T022-105's counter rows remain for the cost of the count (a scan of the namespace's run waiters, at most the ceiling) and its privileges. | Closed (exactness); T022-105 open for the counter rows |
| G022-012 | The start's decision (`decide_start_gates`) and the step's (`decide_gates`) are two code paths over the same rules; T022-072 builds one core for both levels and both gates (D022-050 item 5; architecture review M1 of 2026-11-20). A paused queued run's deadline runs during its pause, as G022-008 says of a step. | Open; T022-072 |
| G1189 | Nothing in the database computes or checks `key_shard` and the waiter `shard`; two binaries that hashed differently would file one key under two shards. | Open; T022-097 |
| G1190 | `flow_control_key_activate` records the operator as typed and keeps no history row with the database role, unlike the registry's activations. *Partly closed 2026-11-16 (D022-042): the version row records `activated_role` (`session_user`); a history of earlier activations of the same version is not kept, since a version is activated once. 2026-11-19 (D022-046): every force has an append-only history row with the database role and a reason; the runbook part is `docs/runbooks/flow-control.md`.* | Open; T022-061 (the rest of the runbook) |
| G1191 | Untested: the down migrations' refusal when `run_events` holds a `concurrency.*` or `rate_limit.*` event; no contract test parses 018's reset-rule table against `RULES`. | Open |
| G1192 | An older replica's quota views read every stored limit and refuse a resource name its build does not know (`stored_limits`): a tenant that sets one of the new limits gets an error from an older replica's quota listing until it is upgraded. True of every resource added before. | Accepted; the next resource change could make the reader skip unknown names |
| G1193 | Activating a digest key version that a live process was not configured with makes that process refuse every gated decision (`UnknownVersion`) until it is; the registry does not check the key versions processes hold. | Open; ADR-0093, runbook (T022-061) |
| G1194 | The pinned operation baseline of `contract_iceberg_writes` digests each operation without following `$ref`, so a change to a shared parameter or schema (as the quota enum of D022-040) moves no digest; `contract_flow_control_step_policies::quotas` pins this enum to the code instead. | Accepted |
| G1195 | Approved texts still describe the drain-first rotation: FR-022-020's last sentence, `data-model.md` (key versions, "only by an audited operator command"), G1163; `data-model.md` does not list `flow_control_group_declarations` or `activated_role`; `contracts/errors.md` does not say that `*_POLICY_INVALID` covers the namespace's ceiling of node-level declarations. Changing them makes the docs drafts again (constitution, Approval). *O1170 answered 2026-11-19 (OD-2026-11-19-2). Also out of date since the force (D022-046): FR-022-056's "no key is ever split between two rows" and SC-022-028 (a forced rotation splits a key into at most two rows), and `data-model.md` lacks `flow_control_key_forced_retirements`. The texts, and the bound of D022-047 in `contracts/errors.md`, are revised when the approved texts are next re-approved (a change makes them drafts again).* | Open; re-approval of the texts |
| G1196 | A rotation's progress (rows left under the retiring version) has no metric and no audit record; the operator learns it by running the command again. *2026-11-19: likewise the rows left under a forced version (an over-admission in progress, D022-046); a force has an audit row, an activation does not.* | Open; T022-061, T022-082 |
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

## External semantics for rate limits (not re-read on 2026-11-13)

Stated from the author's knowledge of these products, without quotation; a reviewer should confirm them
before they are cited elsewhere. Windmill's concurrency limits count executions within a time window and
queue the excess (already listed above; D1181 took only the concurrent-holder part). Temporal limits the
rate at which a task queue dispatches activities, on the server, as a smoothed rate. Inngest separates a
lossy `rateLimit` from a queued `throttle` with `limit`, `period`, `burst` and `key`. The shape here keeps
the queued meaning and the `limit`/window/`key` vocabulary, and chooses the strict window for the reason
of D022-003.

